use super::{
    FileDigest, Manifest, Observer, Redecode, Row, Summary, check_manifest, check_values, inspect,
    open_part,
    redecode::{DigestReader, IDENTITY_VERIFICATION, digest},
    validate_row,
};
use crate::{
    capture::{CSV_METADATA, csv_header},
    profile::Profile,
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

const METADATA_LIMIT: u64 = 16 * 1024 * 1024;
const EXPORTED_FILES: [&str; 4] = [
    "mapping-profile.toml",
    "source-profile.toml",
    "source-capture.toml",
    "data.csv",
];

#[derive(Debug, Serialize)]
pub struct ExportVerification {
    pub export_directory: PathBuf,
    pub mapping_profile: String,
    pub rows_checked: u64,
    pub typed_rows: u64,
    pub blank_rows: u64,
    pub fields_checked: usize,
    pub local_file_hashes_checked: usize,
    pub original_source_reverified: bool,
    pub original_source_directory: Option<PathBuf>,
    pub original_csv_hashes_checked: u64,
    pub source_capture: Summary,
    pub scope: &'static str,
}

fn plain_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).with_context(|| path.display().to_string())?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "bundle member must be a regular file: {}",
        path.display()
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                == 0,
            "bundle member is a reparse point: {}",
            path.display()
        );
    }
    Ok(())
}

fn metadata(directory: &Path, name: &str) -> Result<Vec<u8>> {
    let path = directory.join(name);
    plain_file(&path).with_context(|| name.to_owned())?;
    let file = File::open(&path)?;
    ensure!(
        file.metadata()?.len() <= METADATA_LIMIT,
        "{name}: metadata exceeds 16 MiB"
    );
    let mut bytes = Vec::new();
    file.take(METADATA_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= METADATA_LIMIT,
        "{name}: metadata exceeds 16 MiB"
    );
    Ok(bytes)
}

fn inventory(files: &[FileDigest]) -> Result<BTreeMap<&str, &FileDigest>> {
    let mut result = BTreeMap::new();
    for file in files {
        ensure!(
            file.sha256.len() == 64 && file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "invalid SHA256 for {}",
            file.name
        );
        ensure!(
            result.insert(file.name.as_str(), file).is_none(),
            "duplicate file inventory entry {}",
            file.name
        );
    }
    Ok(result)
}

fn check_digest(actual: &FileDigest, inventory: &BTreeMap<&str, &FileDigest>) -> Result<()> {
    let expected = inventory
        .get(actual.name.as_str())
        .with_context(|| format!("missing receipt hash for {}", actual.name))?;
    ensure!(
        actual.bytes == expected.bytes && actual.sha256.eq_ignore_ascii_case(&expected.sha256),
        "{}: bytes or SHA256 differ from the receipt",
        actual.name
    );
    Ok(())
}

/// Check local bundle contents. Original source paths are read only when explicitly provided.
pub fn verify_export(directory: &Path, source: Option<&Path>) -> Result<ExportVerification> {
    let directory = directory.canonicalize()?;
    let receipt: Redecode =
        toml::from_str(std::str::from_utf8(&metadata(&directory, "receipt.toml")?)?)
            .context("receipt.toml")?;
    ensure!(
        receipt.format_version == 1 && receipt.kind == "offline_redecode",
        "unsupported re-decode receipt format or kind"
    );
    ensure!(
        !receipt.workbench_version.trim().is_empty(),
        "receipt has no application version"
    );
    chrono::DateTime::parse_from_rfc3339(&receipt.created_utc)
        .context("invalid export creation UTC")?;
    ensure!(
        receipt.identity_verification == IDENTITY_VERIFICATION,
        "receipt claims an unsupported identity verification scope"
    );
    let files = inventory(&receipt.exported_files)?;
    ensure!(
        files.len() == EXPORTED_FILES.len()
            && EXPORTED_FILES.iter().all(|name| files.contains_key(name)),
        "exported file inventory must contain exactly the four data/profile/report files"
    );
    let mapping_bytes = metadata(&directory, "mapping-profile.toml")?;
    let profile_bytes = metadata(&directory, "source-profile.toml")?;
    let manifest_bytes = metadata(&directory, "source-capture.toml")?;
    for (name, bytes) in [
        ("mapping-profile.toml", &mapping_bytes),
        ("source-profile.toml", &profile_bytes),
        ("source-capture.toml", &manifest_bytes),
    ] {
        check_digest(&digest(name, bytes), &files)?;
    }
    let mapping =
        Profile::parse(std::str::from_utf8(&mapping_bytes)?).context("mapping-profile.toml")?;
    let profile =
        Profile::parse(std::str::from_utf8(&profile_bytes)?).context("source-profile.toml")?;
    let manifest: Manifest =
        toml::from_str(std::str::from_utf8(&manifest_bytes)?).context("source-capture.toml")?;
    check_manifest(&manifest, &profile)?;
    ensure!(manifest.csv_parts != 0, "source report lists no CSV parts");
    ensure!(
        mapping.input_bytes == profile.input_bytes && receipt.input_bytes == mapping.input_bytes,
        "mapping/source/receipt input widths differ"
    );
    let fields = mapping.log_fields();
    ensure!(
        !fields.is_empty(),
        "export mapping has no selected typed fields"
    );
    ensure!(
        receipt.mapping_profile == mapping.name
            && receipt.exported_fields.iter().eq(fields.iter().copied()),
        "receipt mapping name or selected field definitions differ from the saved mapping"
    );
    let source_files = inventory(&receipt.source_files)?;
    ensure!(
        source_files.len() as u128 == u128::from(manifest.csv_parts) + 2,
        "source file inventory count differs from its report"
    );
    check_digest(&digest("profile.toml", &profile_bytes), &source_files)?;
    check_digest(&digest("capture.toml", &manifest_bytes), &source_files)?;
    for part in 1..=manifest.csv_parts {
        ensure!(
            source_files.contains_key(format!("capture-{part:04}.csv").as_str()),
            "missing source CSV inventory entry {part}"
        );
    }
    plain_file(&directory.join("data.csv"))?;
    let mut reader = open_part(&directory.join("data.csv"), true)?;
    let header = csv::StringRecord::from(csv_header(&mapping));
    ensure!(
        reader.headers()? == &header,
        "data.csv: header differs from the saved mapping"
    );
    let mut summary = Summary::new(&manifest, None);
    let mut previous_record = None::<csv::StringRecord>;
    let mut typed_rows = 0;
    for (index, record) in reader.records().enumerate() {
        let check = || -> Result<csv::StringRecord> {
            let record = record?;
            let row: Row<'_> = record.deserialize(Some(&header))?;
            let previous: Option<Row<'_>> = previous_record
                .as_ref()
                .map(|record| record.deserialize(Some(&header)))
                .transpose()?;
            validate_row(&row, previous.as_ref(), &manifest)?;
            let (_, typed) = check_values(&mapping, &fields, &row, &record)?;
            typed_rows += u64::from(typed);
            summary.observe(&row, previous.as_ref());
            Ok(record)
        };
        previous_record =
            Some(check().with_context(|| format!("data.csv: data row {}", index + 1))?);
    }
    summary.finish(&manifest)?;
    check_digest(&reader.get_ref().receipt("data.csv")?, &files)?;
    ensure!(
        summary == receipt.source_capture,
        "receipt source summary differs from its saved report and derived CSV metadata"
    );
    ensure!(
        receipt.rows_written == summary.rows_checked
            && receipt.typed_rows == typed_rows
            && receipt.blank_rows == summary.rows_checked - typed_rows,
        "receipt export row accounting differs from data.csv"
    );
    let source_directory = if let Some(source) = source {
        let source = source
            .canonicalize()
            .context("explicit original capture directory")?;
        let mut comparison = Comparison::new(&directory, &mapping, &source_files, &files)?;
        let (original_summary, _) = inspect(&source, None, None, Some(&mut comparison))?;
        ensure!(
            original_summary == summary,
            "original source verification summary differs from the bundle"
        );
        comparison.finish()?;
        Some(source)
    } else {
        None
    };
    Ok(ExportVerification {
        export_directory: directory,
        mapping_profile: mapping.name.clone(),
        rows_checked: summary.rows_checked,
        typed_rows,
        blank_rows: summary.rows_checked - typed_rows,
        fields_checked: fields.len(),
        local_file_hashes_checked: files.len(),
        original_source_reverified: source_directory.is_some(),
        original_csv_hashes_checked: if source_directory.is_some() {
            manifest.csv_parts
        } else {
            0
        },
        original_source_directory: source_directory,
        source_capture: summary,
        scope: "file hashes and data consistency; receipt authorship, device identity, calibration, and physical acceptance are not established",
    })
}

struct Comparison<'a> {
    reader: csv::Reader<DigestReader>,
    fields: Vec<&'a crate::decode::Field>,
    source_files: &'a BTreeMap<&'a str, &'a FileDigest>,
    files: &'a BTreeMap<&'a str, &'a FileDigest>,
    rows: u64,
}

impl<'a> Comparison<'a> {
    fn new(
        directory: &Path,
        mapping: &'a Profile,
        source_files: &'a BTreeMap<&'a str, &'a FileDigest>,
        files: &'a BTreeMap<&'a str, &'a FileDigest>,
    ) -> Result<Self> {
        plain_file(&directory.join("data.csv"))?;
        let mut reader = open_part(&directory.join("data.csv"), true)?;
        ensure!(
            reader.headers()? == &csv::StringRecord::from(csv_header(mapping)),
            "data.csv header changed before original-source comparison"
        );
        Ok(Self {
            reader,
            fields: mapping.log_fields(),
            source_files,
            files,
            rows: 0,
        })
    }

    fn finish(mut self) -> Result<()> {
        ensure!(
            self.reader.records().next().is_none(),
            "data.csv has rows absent from the original capture"
        );
        check_digest(&self.reader.get_ref().receipt("data.csv")?, self.files)
    }
}

impl Observer for Comparison<'_> {
    fn source(
        &mut self,
        _profile: &Profile,
        profile_bytes: &[u8],
        manifest_bytes: &[u8],
    ) -> Result<()> {
        check_digest(&digest("profile.toml", profile_bytes), self.source_files)?;
        check_digest(&digest("capture.toml", manifest_bytes), self.source_files)
    }

    fn row(&mut self, original: &csv::StringRecord, payload: &[u8], typed: bool) -> Result<()> {
        self.rows += 1;
        let record = self
            .reader
            .records()
            .next()
            .context("data.csv ended before the original capture")??;
        ensure!(
            record
                .iter()
                .take(CSV_METADATA.len())
                .eq(original.iter().take(CSV_METADATA.len())),
            "data.csv row {}: metadata/raw cells differ from the original capture",
            self.rows
        );
        for (column, field) in self.fields.iter().enumerate() {
            let expected = if typed {
                field.decode(payload)?.value
            } else {
                String::new()
            };
            ensure!(
                record[CSV_METADATA.len() + column] == expected,
                "data.csv row {}: revised value differs from the original payload",
                self.rows
            );
        }
        Ok(())
    }

    fn source_part(&mut self, file: FileDigest) -> Result<()> {
        check_digest(&file, self.source_files)
    }
}

#[cfg(test)]
mod tests;
