use super::*;
use crate::{
    archive,
    capture::csv_header,
    decode::{self, Field},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn saved() -> Result<(tempfile::TempDir, PathBuf)> {
    fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let path = scratch.path().join("capture");
    fs::create_dir(&path)?;
    let mut profile = Profile::default();
    profile.connection.simulate_snapshots = true;
    profile.fields.push(Field {
        name: "value".into(),
        bits: 16,
        ..Default::default()
    });
    profile.save(&path.join("profile.toml"))?;
    fs::write(
        path.join("capture.toml"),
        r#"format_version = 1
profile = "New profile"
utc_start = "2026-11-12T12:00:00.000Z"
elapsed_us = 10000
csv_parts = 1
csv_rows_flushed = 5
row_kind = "host_poll"
source_update_loss = "unknown"
stale_threshold_us = 5000
ended_connected = true
[counts]
poll_attempts = 5
received = 5
new_updates_observed = 0
duplicate = 0
out_of_order = 0
invalid = 1
validity_unknown = 1
stale = 2
freshness_unknown = 2
selected = 5
unchanged = 0
logger_overruns = 0
disconnects = 0
reconnects = 0
[last_status]
freshness = "snapshots"
route = "original synthetic quality fixture"
"#,
    )?;
    let mut writer = csv::Writer::from_path(path.join("capture-0001.csv"))?;
    writer.write_record(csv_header(&profile))?;
    for (index, (validity, stale, age, payload)) in [
        (Validity::Valid, Some(false), Some(0), vec![0, 1]),
        (Validity::Invalid, None, None, vec![]),
        (Validity::Unknown, Some(true), Some(6000), vec![0, 2]),
        (Validity::Valid, Some(true), Some(6000), vec![0, 3]),
        (Validity::Valid, None, None, vec![0, 4]),
    ]
    .into_iter()
    .enumerate()
    {
        let value = if validity == Validity::Valid && stale != Some(true) {
            profile.fields[0].decode(&payload)?.value
        } else {
            String::new()
        };
        writer.write_record([
            "2026-11-12T12:00:00.000Z".into(),
            ((index + 1) * 1000).to_string(),
            (index + 1).to_string(),
            "1".into(),
            "host_poll".into(),
            validity.label().into(),
            stale.map(|v| v.to_string()).unwrap_or_default(),
            String::new(),
            String::new(),
            age.map(|v| v.to_string()).unwrap_or_default(),
            String::new(),
            decode::hex(&payload),
            value,
        ])?;
    }
    writer.flush()?;
    Ok((scratch, path))
}
fn read(path: &Path) -> Result<toml::Value> {
    Ok(toml::from_str(&fs::read_to_string(
        path.join("capture.toml"),
    )?)?)
}
fn save(path: &Path, data: &toml::Value) -> Result<()> {
    fs::write(path.join("capture.toml"), toml::to_string_pretty(data)?)?;
    Ok(())
}
fn rows(path: &Path) -> Result<(csv::StringRecord, Vec<csv::StringRecord>)> {
    let mut r = csv::Reader::from_path(path.join("capture-0001.csv"))?;
    let h = r.headers()?.clone();
    Ok((h, r.records().collect::<std::result::Result<_, _>>()?))
}
fn write_rows(
    path: &Path,
    header: &csv::StringRecord,
    records: &[csv::StringRecord],
) -> Result<()> {
    let mut w = csv::Writer::from_path(path.join("capture-0001.csv"))?;
    w.write_record(header)?;
    for row in records {
        w.write_record(row)?;
    }
    w.flush()?;
    Ok(())
}
fn replace(row: &csv::StringRecord, column: usize, value: &str) -> csv::StringRecord {
    row.iter()
        .enumerate()
        .map(|(i, v)| if i == column { value } else { v })
        .collect()
}

#[test]
fn valid_unknown_invalid_and_stale_rows_keep_separate_meanings() -> Result<()> {
    let (_scratch, path) = saved()?;
    let checked = archive::verify(&path, None)?;
    assert_eq!(
        (
            checked.valid_rows,
            checked.invalid_rows,
            checked.unknown_validity_rows,
            checked.stale_rows
        ),
        (3, 1, 1, 2)
    );
    let review = archive::review(&path, None, 2)?;
    assert_eq!(
        (
            review.signal.numeric.finite_values,
            review.signal.numeric.blank_values
        ),
        (2, 3)
    );
    Ok(())
}

#[test]
fn contradictory_quality_metadata_rejects_all_paths_without_publication() -> Result<()> {
    let (scratch, path) = saved()?;
    let original = read(&path)?;
    let profile = Profile::parse(&fs::read_to_string(path.join("profile.toml"))?)?;
    let joined = scratch.path().join("joined.csv");
    let exported = scratch.path().join("exported");
    for (scope, key, value) in [
        ("root", "stale_threshold_us", 5001),
        ("counts", "duplicate", 1),
        ("counts", "out_of_order", 1),
        ("counts", "invalid", 6),
        ("counts", "validity_unknown", 5),
        ("counts", "stale", 5),
    ] {
        let mut data = original.clone();
        if scope == "root" {
            data[key] = value.into();
        } else {
            data[scope][key] = value.into();
        }
        save(&path, &data)?;
        assert!(archive::verify(&path, Some(&joined)).is_err());
        assert!(archive::review_interfaces(&path, None).is_err());
        assert!(archive::review(&path, None, 2).is_err());
        assert!(archive::compare_captures(&path, &path).is_err());
        assert!(archive::redecode(&path, &profile, &exported).is_err());
        assert!(!joined.exists() && !exported.exists());
    }
    Ok(())
}

#[test]
fn explicit_age_and_stale_flags_agree_at_boundaries_and_unsigned_extremes() -> Result<()> {
    let (_scratch, path) = saved()?;
    let original = read(&path)?;
    let (header, original_rows) = rows(&path)?;
    let mut records = original_rows.clone();
    records[0] = replace(&records[0], 9, "5000");
    write_rows(&path, &header, &records)?;
    assert!(archive::verify(&path, None).is_ok());
    records[0] = replace(&records[0], 6, "true");
    records[0] = replace(&records[0], 12, "");
    let mut data = original.clone();
    data["counts"]["stale"] = 3.into();
    save(&path, &data)?;
    write_rows(&path, &header, &records)?;
    assert!(
        format!("{:#}", archive::verify(&path, None).unwrap_err())
            .contains("stale flag contradicts recorded source age")
    );
    records[0] = replace(&records[0], 9, &u64::MAX.to_string());
    write_rows(&path, &header, &records)?;
    assert!(archive::verify(&path, None).is_ok());
    records = original_rows;
    records[2] = replace(&records[2], 6, "");
    data = original;
    data["counts"]["stale"] = 1.into();
    data["counts"]["freshness_unknown"] = 3.into();
    save(&path, &data)?;
    write_rows(&path, &header, &records)?;
    assert!(
        format!("{:#}", archive::verify(&path, None).unwrap_err()).contains("without a stale flag")
    );
    Ok(())
}

#[test]
fn category_bounds_and_complete_population_equality_detect_streamed_contradictions() -> Result<()> {
    let (_scratch, path) = saved()?;
    let original = read(&path)?;
    for (key, value, expected) in [
        ("invalid", 0, "CSV invalid rows exceed"),
        ("validity_unknown", 0, "CSV unknown validity rows exceed"),
        ("freshness_unknown", 1, "CSV unknown staleness rows exceed"),
        ("invalid", 2, "complete CSV invalid population differs"),
    ] {
        let mut data = original.clone();
        data["counts"][key] = value.into();
        save(&path, &data)?;
        assert!(format!("{:#}", archive::verify(&path, None).unwrap_err()).contains(expected));
    }
    Ok(())
}

#[test]
fn filtered_and_dropped_rows_only_support_quality_subset_checks() -> Result<()> {
    for on_change in [false, true] {
        let (_scratch, path) = saved()?;
        let (header, mut records) = rows(&path)?;
        records.pop();
        write_rows(&path, &header, &records)?;
        let mut data = read(&path)?;
        data["csv_rows_flushed"] = 4.into();
        data["counts"]["invalid"] = 2.into();
        if on_change {
            let mut profile = Profile::parse(&fs::read_to_string(path.join("profile.toml"))?)?;
            profile.logging.mode = crate::profile::LogMode::OnChange;
            profile.save(&path.join("profile.toml"))?;
            data["counts"]["selected"] = 4.into();
            data["counts"]["unchanged"] = 1.into();
        } else {
            data["counts"]["logger_overruns"] = 1.into();
        }
        save(&path, &data)?;
        assert!(archive::verify(&path, None).is_ok());
        data["counts"]["validity_unknown"] = 0.into();
        save(&path, &data)?;
        assert!(archive::verify(&path, None).is_err());
    }
    Ok(())
}

#[test]
fn overlapping_validity_and_staleness_domains_are_not_summed_together() -> Result<()> {
    let (_scratch, path) = saved()?;
    let (header, mut records) = rows(&path)?;
    for row in &mut records {
        *row = replace(row, 5, "invalid");
        *row = replace(row, 6, "true");
        *row = replace(row, 9, "6000");
        *row = replace(row, 11, "");
        *row = replace(row, 12, "");
    }
    write_rows(&path, &header, &records)?;
    let mut data = read(&path)?;
    for (key, value) in [
        ("invalid", 5),
        ("validity_unknown", 0),
        ("stale", 5),
        ("freshness_unknown", 0),
    ] {
        data["counts"][key] = value.into();
    }
    save(&path, &data)?;
    let checked = archive::verify(&path, None)?;
    assert_eq!((checked.invalid_rows, checked.stale_rows), (5, 5));
    Ok(())
}

#[test]
fn missing_legacy_quality_metrics_remain_unknown_without_filling_flags() -> Result<()> {
    let (_scratch, path) = saved()?;
    let mut data = read(&path)?;
    let before = fs::read(path.join("capture-0001.csv"))?;
    for key in [
        "invalid",
        "validity_unknown",
        "stale",
        "freshness_unknown",
        "duplicate",
        "out_of_order",
        "received",
    ] {
        data["counts"].as_table_mut().unwrap().remove(key);
    }
    save(&path, &data)?;
    let checked = archive::verify(&path, None)?;
    assert_eq!(
        (
            checked.invalid_rows,
            checked.unknown_validity_rows,
            checked.stale_rows
        ),
        (1, 1, 2)
    );
    assert_eq!(fs::read(path.join("capture-0001.csv"))?, before);
    Ok(())
}

#[test]
fn local_export_uses_source_threshold_and_rejects_rehashed_quality_lies() -> Result<()> {
    let (scratch, path) = saved()?;
    let mut mapping = Profile::parse(&fs::read_to_string(path.join("profile.toml"))?)?;
    mapping.connection.stale_us = 20000;
    let output = scratch.path().join("exported");
    let mut receipt = archive::redecode(&path, &mapping, &output)?;
    assert!(archive::verify_export(&output, None).is_ok());
    let report = output.join("source-capture.toml");
    let mut data: toml::Value = toml::from_str(&fs::read_to_string(&report)?)?;
    data["counts"]["invalid"] = 0.into();
    let bytes = toml::to_string_pretty(&data)?.into_bytes();
    fs::write(&report, &bytes)?;
    for file in &mut receipt.source_files {
        if file.name == "capture.toml" {
            *file = archive::redecode::digest("capture.toml", &bytes);
        }
    }
    for file in &mut receipt.exported_files {
        if file.name == "source-capture.toml" {
            *file = archive::redecode::digest("source-capture.toml", &bytes);
        }
    }
    fs::write(
        output.join("receipt.toml"),
        toml::to_string_pretty(&receipt)?,
    )?;
    assert!(
        format!("{:#}", archive::verify_export(&output, None).unwrap_err())
            .contains("CSV invalid rows exceed")
    );
    assert!(archive::verify(&path, None).is_ok());
    Ok(())
}

#[test]
fn classification_partitions_and_quality_bounds_use_wide_arithmetic() -> Result<()> {
    let (_scratch, path) = saved()?;
    let mut m: Manifest = toml::from_str(&fs::read_to_string(path.join("capture.toml"))?)?;
    m.last_status.as_mut().unwrap().freshness = Freshness::Counter32;
    m.row_kind = Some("source_update".into());
    assert!(
        format!("{:#}", validate(&m, &Profile::default()).unwrap_err())
            .contains("classifications differ")
    );
    m.counts.new_updates_observed = Some(3);
    m.counts.duplicate = Some(1);
    m.counts.out_of_order = Some(1);
    validate(&m, &Profile::default())?;
    m.counts.new_updates_observed = Some(4);
    m.counts.duplicate = Some(2);
    m.counts.out_of_order = None;
    assert!(
        format!("{:#}", validate(&m, &Profile::default()).unwrap_err())
            .contains("known source classifications exceed")
    );
    m.last_status.as_mut().unwrap().freshness = Freshness::Snapshots;
    m.counts.new_updates_observed = Some(0);
    m.counts.duplicate = None;
    m.counts.out_of_order = None;
    m.counts.received = Some(u64::MAX);
    m.counts.invalid = Some(u64::MAX);
    m.counts.validity_unknown = Some(u64::MAX);
    assert!(
        format!("{:#}", validate(&m, &Profile::default()).unwrap_err())
            .contains("validity category")
    );
    assert!(Rows::default().finish(&m).is_err());
    m.counts.validity_unknown = Some(0);
    m.counts.stale = Some(u64::MAX);
    m.counts.freshness_unknown = Some(0);
    validate(&m, &Profile::default())?;
    Ok(())
}
