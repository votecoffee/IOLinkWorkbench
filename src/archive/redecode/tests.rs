use super::*;
use crate::{
    archive,
    decode::{ByteOrder, Kind},
    transport::Validity,
};

fn fixture(width: usize) -> Result<tempfile::TempDir> {
    let directory = tempfile::tempdir()?;
    let profile = Profile {
        name: "Recorded source".into(),
        input_bytes: width,
        fields: vec![Field {
            name: "Original value".into(),
            bits: 16,
            kind: Kind::Signed,
            ..Default::default()
        }],
        ..Default::default()
    };
    profile.save(&directory.path().join("profile.toml"))?;
    fs::write(
        directory.path().join("capture.toml"),
        r#"format_version = 1
profile = "Recorded source"
utc_start = "2026-11-12T12:00:00.000Z"
elapsed_us = 1000
csv_parts = 3
csv_rows_flushed = 6
row_kind = "host_poll"
source_update_loss = "unknown"
stale_threshold_us = 5000
[counts]
selected = 8
logger_overruns = 2
reconnects = 1
[last_status]
freshness = "snapshots"
"#,
    )?;
    let indexes = ["0001", "2", "4", "5", "6", "8"];
    for part in 0..3 {
        let mut writer = csv::WriterBuilder::new()
            .terminator(csv::Terminator::Any(b'\n'))
            .from_path(
                directory
                    .path()
                    .join(format!("capture-{:04}.csv", part + 1)),
            )?;
        writer.write_record(csv_header(&profile))?;
        for (index, sample_index) in indexes.iter().enumerate().skip(part * 2).take(2) {
            let validity = match index {
                1 => Validity::Invalid,
                4 => Validity::Unknown,
                _ => Validity::Valid,
            };
            let stale = index == 2;
            let mut bytes = vec![0; width];
            bytes[width - 2..].copy_from_slice(&[0xFF, 0xF9 - index as u8]);
            if width == 8 && index == 5 {
                bytes.fill(255);
            }
            let mut cells = vec![
                "2026-11-12T12:00:00.000Z".into(),
                format!("{:06}", (index + 1) * 100),
                (*sample_index).into(),
                if index < 3 { "1" } else { "2" }.into(),
                "host_poll".into(),
                validity.label().into(),
                stale.to_string(),
                String::new(),
                String::new(),
                String::new(),
                if index == 1 { "530" } else { "" }.into(),
                crate::decode::hex(&bytes),
            ];
            cells.push(if validity == Validity::Valid && !stale {
                profile.fields[0].decode(&bytes)?.value
            } else {
                String::new()
            });
            writer.write_record(cells)?;
        }
        writer.flush()?;
    }
    Ok(directory)
}

fn mapping(width: usize) -> Profile {
    let mut profile = Profile {
        name: "Revised interpretation".into(),
        input_bytes: width,
        fields: vec![
            Field {
                name: "Revised, \"µm\"".into(),
                bits: 16,
                kind: Kind::Signed,
                byte_order: ByteOrder::LittleEndian,
                scale: 0.01,
                offset: 2.,
                decimals: Some(2),
                unit: "µm".into(),
                ..Default::default()
            },
            Field {
                name: "Flag".into(),
                bits: 1,
                kind: Kind::Bool,
                ..Default::default()
            },
            Field {
                name: "Not selected".into(),
                bits: 8,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    profile.logging.fields = vec!["Flag".into(), "Revised, \"µm\"".into()];
    profile
}

fn records(path: &Path) -> Result<(csv::StringRecord, Vec<csv::StringRecord>)> {
    let mut reader = csv::Reader::from_path(path)?;
    let header = reader.headers()?.clone();
    Ok((
        header,
        reader.records().collect::<std::result::Result<_, _>>()?,
    ))
}

fn fingerprint(path: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = fs::read_dir(path)?
        .map(|entry| {
            let entry = entry?;
            Ok((
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path())?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

#[test]
fn revised_mapping_preserves_metadata_and_raw_cells_and_publishes_complete_receipts() -> Result<()>
{
    let source = fixture(2)?;
    let destination = tempfile::tempdir()?;
    let output = destination.path().join("derived");
    let before = fingerprint(source.path())?;
    let mapping = mapping(2);
    let report = archive::redecode(source.path(), &mapping, &output)?;
    assert_eq!(
        (report.rows_written, report.typed_rows, report.blank_rows),
        (6, 3, 3)
    );
    assert_eq!(report.source_capture.missing_sample_indexes, 2);
    assert_eq!(report.source_capture.generations_in_csv, 2);
    assert_eq!(report.source_capture.recorded_source_update_loss, "unknown");
    assert_eq!(report.kind, "offline_redecode");
    assert!(report.identity_verification.contains("not performed"));
    let (header, exported) = records(&output.join("data.csv"))?;
    assert_eq!(header, csv::StringRecord::from(csv_header(&mapping)));
    let fields = mapping.log_fields();
    let mut all = Vec::new();
    for part in 1..=3 {
        all.extend(records(&source.path().join(format!("capture-{part:04}.csv")))?.1);
    }
    for (original, derived) in all.iter().zip(&exported) {
        assert!(
            original
                .iter()
                .take(CSV_METADATA.len())
                .eq(derived.iter().take(CSV_METADATA.len()))
        );
        let payload = crate::decode::parse_hex(&original[11])?;
        for (column, field) in fields.iter().enumerate() {
            let expected = if original[5] == *"valid" && original[6] != *"true" {
                field.decode(&payload)?.value
            } else {
                String::new()
            };
            assert_eq!(&derived[CSV_METADATA.len() + column], expected);
        }
    }
    assert_eq!(&exported[0][2], "0001");
    assert_eq!(&exported[0][1], "000100");
    assert_eq!(
        fs::read(output.join("source-profile.toml"))?,
        fs::read(source.path().join("profile.toml"))?
    );
    assert_eq!(
        Profile::load(&output.join("mapping-profile.toml"))?,
        mapping
    );
    assert_eq!(report.source_files.len(), 5);
    assert_eq!(report.exported_files.len(), 4);
    for file in &report.source_files {
        assert_eq!(
            file.sha256,
            digest(&file.name, &fs::read(source.path().join(&file.name))?).sha256
        );
    }
    for file in &report.exported_files {
        assert_eq!(
            file.sha256,
            digest(&file.name, &fs::read(output.join(&file.name))?).sha256
        );
    }
    let receipt: toml::Value = toml::from_str(&fs::read_to_string(output.join("receipt.toml"))?)?;
    assert_eq!(receipt["rows_written"].as_integer(), Some(6));
    assert!(!output.join("capture.toml").exists());
    assert_eq!(fingerprint(source.path())?, before);
    Ok(())
}

#[test]
fn raw_only_sources_can_gain_fields_and_full_width_values_stay_exact() -> Result<()> {
    let source = fixture(8)?;
    let mut profile = Profile::load(&source.path().join("profile.toml"))?;
    profile.fields.clear();
    profile.save(&source.path().join("profile.toml"))?;
    for part in 1..=3 {
        let path = source.path().join(format!("capture-{part:04}.csv"));
        let (_, rows) = records(&path)?;
        let mut writer = csv::Writer::from_path(path)?;
        writer.write_record(csv_header(&profile))?;
        for row in rows {
            writer.write_record(row.iter().take(CSV_METADATA.len()))?;
        }
        writer.flush()?;
    }
    let destination = tempfile::tempdir()?;
    let profile = Profile {
        input_bytes: 8,
        fields: vec![Field {
            name: "Counter".into(),
            bits: 64,
            ..Default::default()
        }],
        ..Default::default()
    };
    archive::redecode(source.path(), &profile, &destination.path().join("wide"))?;
    assert_eq!(
        &records(&destination.path().join("wide/data.csv"))?
            .1
            .last()
            .unwrap()[12],
        u64::MAX.to_string()
    );
    Ok(())
}

#[test]
fn corrupted_original_values_prevent_publication_even_when_mapping_replaces_them() -> Result<()> {
    let source = fixture(2)?;
    let part = source.path().join("capture-0003.csv");
    let (header, rows) = records(&part)?;
    let mut writer = csv::Writer::from_path(&part)?;
    writer.write_record(&header)?;
    for (i, row) in rows.iter().enumerate() {
        let mut cells = row.iter().map(str::to_owned).collect::<Vec<_>>();
        if i == 1 {
            cells[12] = "corrupted".into();
        }
        writer.write_record(cells)?;
    }
    writer.flush()?;
    drop(writer);
    let before = fingerprint(source.path())?;
    let destination = tempfile::tempdir()?;
    assert!(
        archive::redecode(
            source.path(),
            &mapping(2),
            &destination.path().join("derived")
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(destination.path())?.count(), 0);
    assert_eq!(fingerprint(source.path())?, before);
    Ok(())
}

#[test]
fn existing_and_nested_destinations_and_width_mismatches_are_rejected() -> Result<()> {
    let source = fixture(2)?;
    let destination = tempfile::tempdir()?;
    let existing = destination.path().join("existing");
    fs::create_dir(&existing)?;
    fs::write(existing.join("keep.txt"), "keep")?;
    assert!(archive::redecode(source.path(), &mapping(2), &existing).is_err());
    assert_eq!(fs::read_to_string(existing.join("keep.txt"))?, "keep");
    assert!(archive::redecode(source.path(), &mapping(2), &source.path().join("nested")).is_err());
    assert!(
        archive::redecode(
            source.path(),
            &mapping(3),
            &destination.path().join("wrong-width")
        )
        .is_err()
    );
    assert_eq!(fs::read_dir(destination.path())?.count(), 1);
    let mut empty = mapping(2);
    empty.fields.clear();
    empty.logging.fields.clear();
    assert!(archive::redecode(source.path(), &empty, &destination.path().join("empty")).is_err());
    Ok(())
}

#[test]
fn original_capture_errors_are_retained_and_unconfirmed_rows_block_export() -> Result<()> {
    let source = fixture(2)?;
    let path = source.path().join("capture.toml");
    let text = fs::read_to_string(&path)?;
    fs::write(
        &path,
        format!("terminal_error = \"synthetic terminal error\"\n{text}"),
    )?;
    let destination = tempfile::tempdir()?;
    let receipt = archive::redecode(
        source.path(),
        &mapping(2),
        &destination.path().join("partial-acquisition"),
    )?;
    assert!(receipt.source_capture.capture_had_error);
    fs::write(path, format!("csv_rows_unconfirmed = 1\n{text}"))?;
    assert!(
        archive::redecode(
            source.path(),
            &mapping(2),
            &destination.path().join("unconfirmed")
        )
        .is_err()
    );
    assert!(!destination.path().join("unconfirmed").exists());
    Ok(())
}

#[test]
fn final_flush_failure_does_not_publish_a_bundle() -> Result<()> {
    let source = fixture(2)?;
    let destination = tempfile::tempdir()?;
    let output = destination.path().join("failure");
    let mut export = Exporter::new(source.path(), &mapping(2), &output)?;
    // A real read-only file handle makes the final buffered write fail.
    let mut writer = csv::Writer::from_writer(File::open(source.path().join("profile.toml"))?);
    writer.write_record(["buffered output"])?;
    export.writer = Some(writer);
    let summary = archive::verify(source.path(), None)?;
    export.rows = summary.rows_checked;
    assert!(export.finish(summary).is_err());
    assert_eq!(fs::read_dir(destination.path())?.count(), 0);
    Ok(())
}

#[test]
fn publication_refuses_a_destination_created_after_staging() -> Result<()> {
    let root = tempfile::tempdir()?;
    let staging = tempfile::tempdir_in(root.path())?;
    fs::write(staging.path().join("data"), "new")?;
    let target = root.path().join("collision");
    fs::create_dir(&target)?;
    assert!(publish(staging.path(), &target).is_err());
    assert!(target.is_dir() && fs::read_dir(&target)?.next().is_none());
    assert_eq!(fs::read_to_string(staging.path().join("data"))?, "new");
    Ok(())
}

#[test]
fn streamed_sha256_matches_a_known_vector() -> Result<()> {
    let root = tempfile::tempdir()?;
    let path = root.path().join("bytes");
    fs::write(&path, b"hello world")?;
    let mut reader = DigestReader::new(File::open(path)?, true);
    let mut buffer = [0; 3];
    while reader.read(&mut buffer)? != 0 {}
    let receipt = reader.receipt("bytes")?;
    assert_eq!(receipt.bytes, 11);
    assert_eq!(
        receipt.sha256,
        "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
    );
    Ok(())
}
