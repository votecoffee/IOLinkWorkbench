use super::*;
use crate::{
    archive::{
        self,
        redecode::tests::{fixture, mapping},
    },
    decode::{Field, Kind},
};

fn exported(width: usize) -> Result<(tempfile::TempDir, tempfile::TempDir, PathBuf)> {
    let source = fixture(width)?;
    let root = tempfile::tempdir()?;
    let output = root.path().join("export");
    archive::redecode(source.path(), &mapping(width), &output)?;
    Ok((source, root, output))
}

fn load(directory: &Path) -> Result<Redecode> {
    Ok(toml::from_str(&fs::read_to_string(
        directory.join("receipt.toml"),
    )?)?)
}

fn save_receipt(directory: &Path, receipt: &Redecode) -> Result<()> {
    fs::write(
        directory.join("receipt.toml"),
        toml::to_string_pretty(receipt)?,
    )?;
    Ok(())
}

fn rehash(directory: &Path, name: &str) -> Result<()> {
    let mut receipt = load(directory)?;
    *receipt
        .exported_files
        .iter_mut()
        .find(|file| file.name == name)
        .unwrap() = digest(name, &fs::read(directory.join(name))?);
    save_receipt(directory, &receipt)
}

fn copy_files(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir(destination)?;
    for file in fs::read_dir(source)? {
        let file = file?;
        fs::copy(file.path(), destination.join(file.file_name()))?;
    }
    Ok(())
}

#[test]
fn local_and_original_source_scopes_are_distinct_and_read_only() -> Result<()> {
    let (source, _root, output) = exported(2)?;
    let original = fs::read(output.join("receipt.toml"))?;
    let local = verify_export(&output, None)?;
    assert_eq!(
        (local.rows_checked, local.typed_rows, local.blank_rows),
        (6, 3, 3)
    );
    assert_eq!(local.local_file_hashes_checked, 4);
    assert!(!local.original_source_reverified);
    assert_eq!(local.original_csv_hashes_checked, 0);
    assert!(local.original_source_directory.is_none());
    assert_eq!(local.source_capture.recorded_source_update_loss, "unknown");
    assert_eq!(local.source_capture.missing_sample_indexes, 2);
    let full = verify_export(&output, Some(source.path()))?;
    assert!(full.original_source_reverified);
    assert_eq!(full.original_csv_hashes_checked, 3);
    assert_eq!(full.source_capture, local.source_capture);
    assert_eq!(fs::read(output.join("receipt.toml"))?, original);
    let mut uppercase = load(&output)?;
    for file in uppercase
        .exported_files
        .iter_mut()
        .chain(&mut uppercase.source_files)
    {
        file.sha256.make_ascii_uppercase();
    }
    save_receipt(&output, &uppercase)?;
    assert!(verify_export(&output, Some(source.path()))?.original_source_reverified);
    assert!(toml::to_string_pretty(&full)?.contains("original_source_reverified = true"));
    Ok(())
}

#[test]
fn copied_bundles_and_relocated_sources_do_not_follow_recorded_paths() -> Result<()> {
    let (source, root, output) = exported(2)?;
    let copied = root.path().join("copied");
    copy_files(&output, &copied)?;
    let mut receipt = load(&copied)?;
    receipt.output_directory = root.path().join("missing-output");
    receipt.source_directory = root.path().join("missing-source");
    save_receipt(&copied, &receipt)?;
    assert!(!verify_export(&copied, None)?.original_source_reverified);
    let moved_source = root.path().join("relocated-source");
    copy_files(source.path(), &moved_source)?;
    let full = verify_export(&copied, Some(&moved_source))?;
    assert_eq!(
        full.original_source_directory,
        Some(moved_source.canonicalize()?)
    );
    assert!(verify_export(&copied, Some(&receipt.source_directory)).is_err());
    Ok(())
}

#[test]
fn every_pinned_local_file_is_checked() -> Result<()> {
    let (_source, root, output) = exported(2)?;
    for (index, name) in EXPORTED_FILES.iter().enumerate() {
        let copied = root.path().join(format!("tampered-{index}"));
        copy_files(&output, &copied)?;
        let path = copied.join(name);
        let mut bytes = fs::read(&path)?;
        bytes.push(b'\n');
        fs::write(&path, bytes)?;
        let error = verify_export(&copied, None).unwrap_err();
        assert!(format!("{error:#}").contains(name));
    }
    Ok(())
}

#[test]
fn rehashed_bad_typed_values_still_fail_semantic_verification() -> Result<()> {
    let (_source, _root, output) = exported(2)?;
    alter_first(&output, false)?;
    rehash(&output, "data.csv")?;
    let error = verify_export(&output, None).unwrap_err();
    assert!(format!("{error:#}").contains("typed value"));
    Ok(())
}

fn alter_first(output: &Path, valid_alternate_raw: bool) -> Result<()> {
    let path = output.join("data.csv");
    let mut reader = csv::Reader::from_path(&path)?;
    let header = reader.headers()?.clone();
    let mut rows = reader
        .records()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(reader);
    let mut cells = rows[0].iter().map(str::to_owned).collect::<Vec<_>>();
    if valid_alternate_raw {
        cells[11] = "0002".into();
        let profile = Profile::load(&output.join("mapping-profile.toml"))?;
        for (index, field) in profile.log_fields().iter().enumerate() {
            cells[CSV_METADATA.len() + index] = field.decode(&[0, 2])?.value;
        }
    } else {
        cells[CSV_METADATA.len() + 1] = "wrong".into();
    }
    rows[0] = csv::StringRecord::from(cells);
    let mut writer = csv::Writer::from_path(&path)?;
    writer.write_record(&header)?;
    for row in rows {
        writer.write_record(&row)?;
    }
    writer.flush()?;
    Ok(())
}

#[test]
fn locally_consistent_rehashed_payload_changes_need_original_source_comparison() -> Result<()> {
    let (source, _root, output) = exported(2)?;
    alter_first(&output, true)?;
    rehash(&output, "data.csv")?;
    let local = verify_export(&output, None)?;
    assert!(!local.original_source_reverified);
    let error = verify_export(&output, Some(source.path())).unwrap_err();
    assert!(format!("{error:#}").contains("metadata/raw cells differ"));
    Ok(())
}

#[test]
fn source_byte_changes_fail_even_when_saved_values_still_verify() -> Result<()> {
    let (source, _root, output) = exported(2)?;
    let path = source.path().join("capture-0002.csv");
    let mut bytes = fs::read(&path)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    assert!(archive::verify(source.path(), None).is_ok());
    assert!(verify_export(&output, None).is_ok());
    let error = verify_export(&output, Some(source.path())).unwrap_err();
    assert!(format!("{error:#}").contains("capture-0002.csv"));
    Ok(())
}

#[test]
fn receipt_kinds_inventories_and_accounting_are_checked() -> Result<()> {
    let (_source, root, output) = exported(2)?;
    for variant in 0..8 {
        let copied = root.path().join(format!("bad-receipt-{variant}"));
        copy_files(&output, &copied)?;
        let mut receipt = load(&copied)?;
        match variant {
            0 => receipt.format_version = 2,
            1 => receipt.kind = "physical_capture".into(),
            2 => receipt.exported_files[0].name = "../../outside".into(),
            3 => receipt.source_files[2].name = "missing.csv".into(),
            4 => receipt.source_files[2].name = receipt.source_files[3].name.clone(),
            5 => receipt.typed_rows += 1,
            6 => receipt.source_capture.generations_in_csv += 1,
            _ => receipt.identity_verification = "verified device".into(),
        }
        save_receipt(&copied, &receipt)?;
        assert!(verify_export(&copied, None).is_err(), "variant {variant}");
    }
    Ok(())
}

#[test]
fn unknown_receipt_fields_and_oversized_metadata_are_rejected() -> Result<()> {
    let (_source, _root, output) = exported(2)?;
    let path = output.join("receipt.toml");
    let text = fs::read_to_string(&path)?;
    fs::write(&path, format!("unexpected_claim = true\n{text}"))?;
    assert!(verify_export(&output, None).is_err());
    fs::OpenOptions::new()
        .write(true)
        .open(&path)?
        .set_len(METADATA_LIMIT + 1)?;
    let error = verify_export(&output, None).unwrap_err();
    assert!(error.to_string().contains("16 MiB"));
    Ok(())
}

#[test]
fn truncated_rehashed_csv_and_missing_files_are_rejected() -> Result<()> {
    let (_source, root, output) = exported(2)?;
    let truncated = root.path().join("truncated");
    copy_files(&output, &truncated)?;
    let path = truncated.join("data.csv");
    let mut bytes = fs::read(&path)?;
    assert_eq!(bytes.pop(), Some(b'\n'));
    fs::write(path, bytes)?;
    rehash(&truncated, "data.csv")?;
    assert!(
        format!("{:#}", verify_export(&truncated, None).unwrap_err())
            .contains("incomplete final record")
    );
    fs::remove_file(truncated.join("mapping-profile.toml"))?;
    assert!(verify_export(&truncated, None).is_err());
    Ok(())
}

#[test]
fn mapping_claims_and_source_part_counts_are_validated_before_use() -> Result<()> {
    let (_source, root, output) = exported(2)?;
    for variant in 0..4 {
        let copied = root.path().join(format!("bad-mapping-{variant}"));
        copy_files(&output, &copied)?;
        let mut receipt = load(&copied)?;
        match variant {
            0 => receipt.exported_fields[0].bit_offset += 1,
            1 => receipt.input_bytes += 1,
            2 => receipt.mapping_profile = "different mapping".into(),
            _ => {
                let path = copied.join("source-capture.toml");
                let text = fs::read_to_string(&path)?
                    .replace("csv_parts = 3", "csv_parts = 9223372036854775807");
                fs::write(path, text)?;
                rehash(&copied, "source-capture.toml")?;
                receipt = load(&copied)?;
                receipt.source_files[1] = digest(
                    "capture.toml",
                    &fs::read(copied.join("source-capture.toml"))?,
                );
            }
        }
        save_receipt(&copied, &receipt)?;
        assert!(verify_export(&copied, None).is_err());
    }
    Ok(())
}

#[test]
fn full_width_integer_exports_remain_exact() -> Result<()> {
    let source = fixture(8)?;
    let root = tempfile::tempdir()?;
    let output = root.path().join("wide");
    let mut mapping = mapping(8);
    mapping.fields = vec![Field {
        name: "Wide".into(),
        bits: 64,
        kind: Kind::Unsigned,
        ..Default::default()
    }];
    mapping.logging.fields.clear();
    archive::redecode(source.path(), &mapping, &output)?;
    assert_eq!(verify_export(&output, Some(source.path()))?.rows_checked, 6);
    assert!(fs::read_to_string(output.join("data.csv"))?.contains(&u64::MAX.to_string()));
    Ok(())
}

#[test]
fn empty_capture_error_flags_remain_visible_after_verification() -> Result<()> {
    let source = fixture(2)?;
    let path = source.path().join("capture.toml");
    let text = fs::read_to_string(&path)?;
    let text = text
        .split("[last_status]")
        .next()
        .unwrap()
        .replace("csv_rows_flushed = 6", "csv_rows_flushed = 0")
        .replace("selected = 8", "selected = 0")
        .replace("logger_overruns = 2", "logger_overruns = 0")
        .replace("row_kind = \"host_poll\"\n", "");
    fs::write(
        path,
        format!("terminal_error = \"no synthetic input arrived\"\n{text}"),
    )?;
    let profile = Profile::load(&source.path().join("profile.toml"))?;
    for part in 1..=3 {
        let mut writer =
            csv::Writer::from_path(source.path().join(format!("capture-{part:04}.csv")))?;
        writer.write_record(csv_header(&profile))?;
        writer.flush()?;
    }
    let root = tempfile::tempdir()?;
    let output = root.path().join("empty-export");
    archive::redecode(source.path(), &mapping(2), &output)?;
    let check = verify_export(&output, Some(source.path()))?;
    assert_eq!(check.rows_checked, 0);
    assert_eq!(check.typed_rows, 0);
    assert!(check.original_source_reverified && check.source_capture.capture_had_error);
    assert!(check.source_capture.row_kind.is_none());
    Ok(())
}
