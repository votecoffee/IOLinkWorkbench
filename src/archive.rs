use crate::{
    capture::{CSV_METADATA, csv_header},
    decode,
    profile::Profile,
    transport::{Freshness, Validity},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

mod redecode;
pub use redecode::{FileDigest, Redecode};
mod verify_export;
pub use verify_export::{ExportVerification, verify_export};
mod review;
pub use review::{Bucket, NumericSummary, RecordedTiming, Review, SignalReview};

#[derive(Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub profile: String,
    pub utc_start: String,
    pub csv_parts: u64,
    pub rows_checked: u64,
    pub valid_rows: u64,
    pub invalid_rows: u64,
    pub unknown_validity_rows: u64,
    pub stale_rows: u64,
    pub row_kind: Option<String>,
    pub first_sample_index: Option<u64>,
    pub last_sample_index: Option<u64>,
    pub missing_sample_indexes: u64,
    pub generations_in_csv: u64,
    pub recorded_source_gaps: Option<u64>,
    pub recorded_source_update_loss: String,
    pub capture_had_error: bool,
    pub joined_csv: Option<PathBuf>,
}

// This subset also accepts reports written before unconfirmed-row accounting was added.
#[derive(Deserialize)]
struct Manifest {
    #[serde(default = "legacy_format")]
    format_version: u32,
    profile: String,
    utc_start: String,
    elapsed_us: u64,
    csv_parts: u64,
    csv_rows_flushed: u64,
    #[serde(default)]
    csv_rows_unconfirmed: u64,
    row_kind: Option<String>,
    last_status: Option<ArchivedStatus>,
    counts: ArchivedCounts,
    source_gaps_observed: Option<u64>,
    source_update_loss: String,
    stale_threshold_us: u64,
    terminal_error: Option<String>,
    writer_error: Option<String>,
    requested_poll_us: Option<u64>,
    iodd_communication: Option<crate::iodd::Communication>,
    simulation_faults: Option<bool>,
    simulated_writer_delay_ms: Option<u64>,
    host_poll_start_spacing: Option<crate::capture::Spacing>,
    host_receive_spacing: Option<crate::capture::Spacing>,
    host_new_update_spacing: Option<crate::capture::Spacing>,
    gateway_update_spacing: Option<crate::capture::Spacing>,
    logger_metrics: Option<crate::capture::LoggerMetrics>,
}

fn legacy_format() -> u32 {
    1
}

#[derive(Deserialize)]
struct ArchivedStatus {
    freshness: Freshness,
    route: Option<String>,
    port_cycle_raw: Option<String>,
    port_cycle_us: Option<u64>,
}

#[derive(Deserialize)]
struct ArchivedCounts {
    selected: u64,
    logger_overruns: u64,
    reconnects: u64,
}

#[derive(Deserialize)]
struct Row<'a> {
    capture_utc_start: &'a str,
    host_elapsed_us: u64,
    sample_index: u64,
    generation: u64,
    row_kind: &'a str,
    validity: Validity,
    stale: Option<bool>,
    source_counter: Option<u32>,
    source_time_us: Option<u64>,
    source_age_us: Option<u64>,
    // Validate the diagnostic column's representation even when its code is vendor-specific.
    #[serde(rename = "diagnostic_code")]
    _diagnostic_code: Option<u16>,
    raw_hex: &'a str,
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn parts(directory: &Path, expected: u64) -> Result<BTreeSet<u64>> {
    ensure!(expected != 0, "capture report lists no CSV parts");
    let mut found = BTreeSet::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(number) = name
            .strip_prefix("capture-")
            .and_then(|s| s.strip_suffix(".csv"))
        {
            let part: u64 = number
                .parse()
                .with_context(|| format!("invalid CSV part name {name}"))?;
            ensure!(
                (1..=expected).contains(&part) && name == format!("capture-{part:04}.csv"),
                "unexpected CSV part {name}"
            );
            ensure!(entry.path().is_file(), "CSV part {name} is not a file");
            found.insert(part);
        }
    }
    ensure!(
        found.len() as u64 == expected,
        "capture report expects {expected} CSV parts, found {}",
        found.len()
    );
    Ok(found)
}

fn open_part(path: &Path, hash: bool) -> Result<csv::Reader<redecode::DigestReader>> {
    let mut file = File::open(path)?;
    ensure!(file.metadata()?.len() != 0, "CSV part is empty");
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0];
    file.read_exact(&mut last)?;
    ensure!(last[0] == b'\n', "CSV part has an incomplete final record");
    file.rewind()?;
    Ok(csv::ReaderBuilder::new().from_reader(redecode::DigestReader::new(file, hash)))
}

fn validate_row(row: &Row<'_>, previous: Option<&Row<'_>>, manifest: &Manifest) -> Result<()> {
    ensure!(
        row.capture_utc_start == manifest.utc_start,
        "UTC capture origin differs from the report"
    );
    ensure!(
        row.sample_index != 0 && row.sample_index <= manifest.counts.selected,
        "sample index {} is outside the report's selected range",
        row.sample_index
    );
    ensure!(
        row.host_elapsed_us <= manifest.elapsed_us,
        "receive timestamp is beyond capture end"
    );
    ensure!(
        row.generation != 0
            && u128::from(row.generation) <= u128::from(manifest.counts.reconnects) + 1,
        "connection generation is outside the report's reconnect range"
    );
    let freshness = manifest
        .last_status
        .as_ref()
        .context("capture report has no gateway status")?
        .freshness;
    ensure!(
        Some(row.row_kind) == manifest.row_kind.as_deref() && row.row_kind == freshness.row_kind(),
        "row kind differs from the report's freshness contract"
    );
    if row
        .source_age_us
        .is_some_and(|age| age > manifest.stale_threshold_us)
    {
        ensure!(
            row.stale == Some(true),
            "source age exceeds the stale threshold without a stale flag"
        );
    }
    if freshness == Freshness::Counter32 {
        ensure!(
            row.source_counter.is_some(),
            "source-update row is missing its counter"
        );
    }
    if freshness == Freshness::Timestamp {
        ensure!(
            row.source_time_us.is_some(),
            "source-update row is missing its timestamp"
        );
    }
    if let Some(previous) = previous {
        ensure!(
            row.sample_index > previous.sample_index,
            "sample indexes repeat or go backward"
        );
        ensure!(
            row.host_elapsed_us >= previous.host_elapsed_us,
            "receive timestamps go backward"
        );
        ensure!(
            row.generation >= previous.generation,
            "connection generations go backward"
        );
        if row.generation == previous.generation {
            match freshness {
                Freshness::Counter32 => {
                    let step = row
                        .source_counter
                        .unwrap()
                        .wrapping_sub(previous.source_counter.unwrap());
                    ensure!(
                        step != 0 && step < (1 << 31),
                        "source counter repeats or goes backward"
                    );
                }
                Freshness::Timestamp => ensure!(
                    row.source_time_us > previous.source_time_us,
                    "source timestamp repeats or goes backward"
                ),
                Freshness::Snapshots => {}
            }
        }
    }
    Ok(())
}

fn check_values(
    profile: &Profile,
    fields: &[&crate::decode::Field],
    row: &Row<'_>,
    record: &csv::StringRecord,
) -> Result<(Vec<u8>, bool)> {
    let payload = if row.raw_hex.is_empty() {
        Vec::new()
    } else {
        decode::parse_hex(row.raw_hex)?
    };
    ensure!(
        decode::hex(&payload) == row.raw_hex,
        "raw payload is not canonical hexadecimal"
    );
    let typed = row.validity == Validity::Valid && row.stale != Some(true);
    if row.validity == Validity::Valid {
        ensure!(
            payload.len() == profile.input_bytes,
            "valid payload width differs from the saved profile"
        );
    }
    for (column, field) in fields.iter().enumerate() {
        let expected = if typed {
            field.decode(&payload)?.value
        } else {
            String::new()
        };
        ensure!(
            record[CSV_METADATA.len() + column] == expected,
            "typed value for {} differs from the raw payload or validity",
            field.name
        );
    }
    Ok((payload, typed))
}

impl Summary {
    fn new(manifest: &Manifest, joined_csv: Option<&Path>) -> Self {
        Self {
            profile: manifest.profile.clone(),
            utc_start: manifest.utc_start.clone(),
            csv_parts: manifest.csv_parts,
            rows_checked: 0,
            valid_rows: 0,
            invalid_rows: 0,
            unknown_validity_rows: 0,
            stale_rows: 0,
            row_kind: manifest.row_kind.clone(),
            first_sample_index: None,
            last_sample_index: None,
            missing_sample_indexes: 0,
            generations_in_csv: 0,
            recorded_source_gaps: manifest.source_gaps_observed,
            recorded_source_update_loss: manifest.source_update_loss.clone(),
            capture_had_error: manifest.terminal_error.is_some() || manifest.writer_error.is_some(),
            joined_csv: joined_csv.map(Path::to_owned),
        }
    }

    fn observe(&mut self, row: &Row<'_>, previous: Option<&Row<'_>>) {
        self.rows_checked += 1;
        match row.validity {
            Validity::Valid => self.valid_rows += 1,
            Validity::Invalid => self.invalid_rows += 1,
            Validity::Unknown => self.unknown_validity_rows += 1,
        }
        self.stale_rows += u64::from(row.stale == Some(true));
        if previous.is_none_or(|previous| previous.generation != row.generation) {
            self.generations_in_csv += 1;
        }
        self.first_sample_index.get_or_insert(row.sample_index);
        self.last_sample_index = Some(row.sample_index);
    }

    fn finish(&mut self, manifest: &Manifest) -> Result<()> {
        ensure!(
            self.rows_checked == manifest.csv_rows_flushed,
            "report confirms {} CSV rows, found {}",
            manifest.csv_rows_flushed,
            self.rows_checked
        );
        self.missing_sample_indexes = manifest.counts.selected - self.rows_checked;
        Ok(())
    }
}

fn check_manifest(manifest: &Manifest, profile: &Profile) -> Result<()> {
    ensure!(
        manifest.iodd_communication.as_ref() == profile.communication(),
        "capture report and profile have different IODD communication declarations"
    );
    if let Some(requested) = manifest.requested_poll_us {
        ensure!(
            requested == profile.connection.poll_us,
            "capture report and profile have different requested host poll intervals"
        );
    }
    ensure!(
        manifest.format_version == 1,
        "unsupported capture format {}",
        manifest.format_version
    );
    ensure!(
        profile.name == manifest.profile,
        "saved profile name differs from the report"
    );
    chrono::DateTime::parse_from_rfc3339(&manifest.utc_start)
        .context("invalid UTC capture origin")?;
    ensure!(
        manifest.csv_rows_unconfirmed == 0,
        "report has {} unconfirmed CSV rows; this capture cannot be verified as complete",
        manifest.csv_rows_unconfirmed
    );
    ensure!(
        u128::from(manifest.counts.selected)
            == u128::from(manifest.csv_rows_flushed) + u128::from(manifest.counts.logger_overruns),
        "report's CSV row accounting is inconsistent"
    );
    if let Some(metrics) = &manifest.logger_metrics {
        metrics.validate(
            profile.logging.queue_capacity,
            manifest.counts.selected - manifest.counts.logger_overruns,
            manifest.csv_rows_flushed,
            manifest.csv_parts,
            manifest.writer_error.is_some(),
        )?;
    }
    Ok(())
}

/// Check a finalized capture using its saved profile and optionally publish a joined CSV.
/// All rows are checked while streaming. The joined file becomes visible only after success.
pub fn verify(directory: &Path, joined_csv: Option<&Path>) -> Result<Summary> {
    Ok(inspect(directory, joined_csv, None, None)?.0)
}

/// Review one saved logged field while verifying all capture rows in the same pass.
/// Omitting the field selects the first field in the saved logging order.
pub fn review(directory: &Path, field: Option<&str>, buckets: usize) -> Result<Review> {
    ensure!(
        (1..=4096).contains(&buckets),
        "trend buckets must be 1–4096"
    );
    let (summary, collector) = inspect(directory, None, Some((field, buckets)), None)?;
    Ok(collector
        .expect("requested review collector")
        .finish(summary))
}

/// Export a new interpretation of verified saved payloads, preserving source files.
pub fn redecode(directory: &Path, mapping: &Profile, output: &Path) -> Result<Redecode> {
    let mut export = redecode::Exporter::new(directory, mapping, output)?;
    let (summary, _) = inspect(directory, None, None, Some(&mut export))?;
    export.finish(summary)
}

trait Observer {
    fn source(
        &mut self,
        profile: &Profile,
        profile_bytes: &[u8],
        manifest_bytes: &[u8],
    ) -> Result<()>;
    fn row(&mut self, original: &csv::StringRecord, payload: &[u8], typed: bool) -> Result<()>;
    fn source_part(&mut self, file: FileDigest) -> Result<()>;
}

fn inspect(
    directory: &Path,
    joined_csv: Option<&Path>,
    request: Option<(Option<&str>, usize)>,
    mut export: Option<&mut dyn Observer>,
) -> Result<(Summary, Option<review::Collector>)> {
    let directory = directory
        .canonicalize()
        .with_context(|| directory.display().to_string())?;
    let manifest_bytes = fs::read(directory.join("capture.toml"))?;
    let manifest: Manifest = toml::from_str(std::str::from_utf8(&manifest_bytes)?)?;
    let profile_bytes = fs::read(directory.join("profile.toml"))?;
    let profile =
        Profile::parse(std::str::from_utf8(&profile_bytes)?).context("saved capture profile")?;
    check_manifest(&manifest, &profile)?;
    let parts = parts(&directory, manifest.csv_parts)?;
    if let Some(export) = &mut export {
        export.source(&profile, &profile_bytes, &manifest_bytes)?;
    }
    let header = csv_header(&profile);
    let header_record = csv::StringRecord::from(header.clone());
    let fields = profile.log_fields();
    let mut collector = request
        .map(|(field, buckets)| review::Collector::new(&profile, &manifest, field, buckets))
        .transpose()?;
    let mut temporary = if let Some(path) = joined_csv {
        ensure!(
            !path.exists(),
            "joined CSV already exists: {}",
            path.display()
        );
        let name = path
            .file_name()
            .context("joined CSV needs a filename")?
            .to_string_lossy();
        ensure!(
            parent(path).canonicalize()? != directory
                || !(name.starts_with("capture-") && name.ends_with(".csv")),
            "joined CSV must not use a capture part filename in the source directory"
        );
        Some(tempfile::NamedTempFile::new_in(parent(path))?)
    } else {
        None
    };
    let mut writer = temporary.as_mut().map(|file| {
        csv::WriterBuilder::new()
            .terminator(csv::Terminator::Any(b'\n'))
            .from_writer(file.as_file_mut())
    });
    if let Some(writer) = &mut writer {
        writer.write_record(&header)?;
    }
    let mut summary = Summary::new(&manifest, joined_csv);
    let mut previous_record = None::<csv::StringRecord>;
    for part in parts {
        let path = directory.join(format!("capture-{part:04}.csv"));
        let mut reader =
            open_part(&path, export.is_some()).with_context(|| path.display().to_string())?;
        ensure!(
            reader
                .headers()?
                .iter()
                .eq(header.iter().map(String::as_str)),
            "{}: CSV header differs from the saved profile",
            path.display()
        );
        for (index, record) in reader.records().enumerate() {
            let check = || -> Result<csv::StringRecord> {
                let record = record?;
                let row: Row<'_> = record.deserialize(Some(&header_record))?;
                let previous: Option<Row<'_>> = previous_record
                    .as_ref()
                    .map(|record| record.deserialize(Some(&header_record)))
                    .transpose()?;
                validate_row(&row, previous.as_ref(), &manifest)?;
                let (payload, typed) = check_values(&profile, &fields, &row, &record)?;
                if let Some(export) = &mut export {
                    export.row(&record, &payload, typed)?;
                }
                if let Some(collector) = &mut collector {
                    collector.add(&row, &record);
                }
                if let Some(writer) = &mut writer {
                    writer.write_record(&record)?;
                }
                summary.observe(&row, previous.as_ref());
                Ok(record)
            };
            previous_record = Some(
                check().with_context(|| format!("{}: data row {}", path.display(), index + 1))?,
            );
        }
        if let Some(export) = &mut export {
            export.source_part(
                reader
                    .get_ref()
                    .receipt(&format!("capture-{part:04}.csv"))?,
            )?;
        }
    }
    summary.finish(&manifest)?;
    if let Some(writer) = &mut writer {
        writer.flush()?;
    }
    drop(writer);
    if let Some(file) = temporary {
        file.as_file().sync_all()?;
        file.persist_noclobber(joined_csv.expect("joined output path"))?;
    }
    Ok((summary, collector))
}

#[cfg(test)]
mod tests;
