use super::*;
use crate::{
    archive,
    capture::{self, Options, Stop, csv_header},
    decode::Field,
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn saved() -> Result<(tempfile::TempDir, PathBuf)> {
    fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let path = scratch.path().join("capture");
    fs::create_dir(&path)?;
    let mut profile = Profile::default();
    profile.logging.raw_only = true;
    profile.save(&path.join("profile.toml"))?;
    fs::write(
        path.join("capture.toml"),
        r#"format_version = 1
profile = "New profile"
utc_start = "2026-11-12T12:00:00.000Z"
elapsed_us = 200000
csv_parts = 1
csv_rows_flushed = 5
row_kind = "source_update"
source_update_loss = "counter gaps within generations; disconnect loss unknown"
source_gaps_observed = 1
stale_threshold_us = 5000
ended_connected = true
[counts]
poll_attempts = 6
received = 5
new_updates_observed = 5
selected = 5
unchanged = 0
logger_overruns = 0
disconnects = 1
reconnects = 1
[last_status]
freshness = "counter32"
route = "original synthetic saved fixture"
"#,
    )?;
    let mut writer = csv::Writer::from_path(path.join("capture-0001.csv"))?;
    writer.write_record(csv_header(&profile))?;
    for (index, (generation, counter)) in [
        (1, u32::MAX - 1),
        (1, u32::MAX),
        (1, 0),
        (2, 123456789),
        (2, 123456791),
    ]
    .into_iter()
    .enumerate()
    {
        writer.write_record([
            "2026-11-12T12:00:00.000Z".into(),
            [1000, 2000, 3000, 60000, 61000][index].to_string(),
            (index + 1).to_string(),
            generation.to_string(),
            "source_update".into(),
            "valid".into(),
            "false".into(),
            counter.to_string(),
            String::new(),
            String::new(),
            String::new(),
            "0000".into(),
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
fn mapping() -> Profile {
    let mut p = Profile::default();
    p.fields.push(Field {
        name: "value".into(),
        bits: 16,
        ..Default::default()
    });
    p
}

#[test]
fn counter_wrap_equal_values_and_unrepresented_empty_generations_are_preserved() -> Result<()> {
    let (_scratch, path) = saved()?;
    let checked = archive::verify(&path, None)?;
    assert_eq!(checked.rows_checked, 5);
    assert_eq!(checked.recorded_source_gaps, Some(1));
    assert_eq!(checked.generations_in_csv, 2);
    let mut data = read(&path)?;
    data["counts"]["reconnects"] = 2.into();
    data["counts"]["disconnects"] = 2.into();
    data["counts"]["poll_attempts"] = 7.into();
    save(&path, &data)?;
    assert_eq!(archive::verify(&path, None)?.generations_in_csv, 2);
    Ok(())
}

#[test]
fn impossible_metadata_is_rejected_without_publishing_a_join_or_export() -> Result<()> {
    let (scratch, path) = saved()?;
    let original = read(&path)?;
    let joined = scratch.path().join("joined.csv");
    let exported = scratch.path().join("exported");
    for (scope, key, value) in [
        ("root", "ended_connected", false.into()),
        ("counts", "unchanged", 1.into()),
        ("counts", "new_updates_observed", 6.into()),
        ("counts", "reconnects", 2.into()),
        ("counts", "received", 4.into()),
    ] {
        let mut data = original.clone();
        if scope == "root" {
            data[key] = value;
        } else {
            data[scope][key] = value;
        }
        save(&path, &data)?;
        assert!(archive::verify(&path, Some(&joined)).is_err());
        assert!(archive::review_interfaces(&path, None).is_err());
        assert!(archive::compare_captures(&path, &path).is_err());
        assert!(archive::redecode(&path, &mapping(), &exported).is_err());
        assert!(!joined.exists() && !exported.exists());
    }
    Ok(())
}

#[test]
fn complete_csv_counter_gaps_must_match_recorded_source_gaps() -> Result<()> {
    let (scratch, path) = saved()?;
    let mut data = read(&path)?;
    let original = fs::read(path.join("capture-0001.csv"))?;
    data["source_gaps_observed"] = 2.into();
    save(&path, &data)?;
    let joined = scratch.path().join("joined.csv");
    let exported = scratch.path().join("exported");
    assert!(
        format!("{:#}", archive::verify(&path, Some(&joined)).unwrap_err())
            .contains("source counter gaps in complete CSV")
    );
    assert!(archive::review_interfaces(&path, None).is_err());
    assert!(archive::compare_captures(&path, &path).is_err());
    assert!(archive::redecode(&path, &mapping(), &exported).is_err());
    assert!(!joined.exists() && !exported.exists());
    assert_eq!(fs::read(path.join("capture-0001.csv"))?, original);
    Ok(())
}

fn selected_counter_gaps(path: &Path) -> Result<u128> {
    let mut parts = fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    parts.sort_by_key(|p| p.file_name());
    let mut last = None;
    let mut gaps = 0;
    for entry in parts {
        if !entry.file_name().to_string_lossy().starts_with("capture-") {
            continue;
        }
        let mut reader = csv::Reader::from_path(entry.path())?;
        for record in reader.records() {
            let r = record?;
            let generation: u64 = r[3].parse()?;
            let counter: u32 = r[7].parse()?;
            if let Some((prior_generation, prior_counter)) = last
                && generation == prior_generation
            {
                gaps += u128::from(counter.wrapping_sub(prior_counter).saturating_sub(1));
            }
            last = Some((generation, counter));
        }
    }
    Ok(gaps)
}

#[test]
fn on_change_filtering_and_writer_drops_are_not_reclassified_as_source_gaps() -> Result<()> {
    fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let path = scratch.path().join("on-change");
    let mut profile = Profile::parse(include_str!("../../../samples/simulator.toml"))?;
    profile.logging.mode = LogMode::OnChange;
    let report = capture::run(
        &profile,
        &Options {
            output: Some(path.clone()),
            duration: Some(Duration::from_millis(500)),
            ..Default::default()
        },
        &Stop::default(),
        None,
    )?;
    assert!(report.success() && report.counts.unchanged > 0);
    let checked = archive::verify(&path, None)?;
    assert!(selected_counter_gaps(&path)? > u128::from(checked.recorded_source_gaps.unwrap()));
    let (_scratch, path) = archive::tests::fixture(false, true)?;
    let data = read(&path)?;
    assert!(data["counts"]["logger_overruns"].as_integer().unwrap() > 0);
    let checked = archive::verify(&path, None)?;
    assert!(selected_counter_gaps(&path)? > u128::from(checked.recorded_source_gaps.unwrap()));
    Ok(())
}

#[test]
fn missing_legacy_populations_do_not_imply_complete_source_coverage() -> Result<()> {
    let (_scratch, path) = saved()?;
    let mut data = read(&path)?;
    for key in [
        "new_updates_observed",
        "received",
        "poll_attempts",
        "unchanged",
        "disconnects",
    ] {
        data["counts"].as_table_mut().unwrap().remove(key);
    }
    data.as_table_mut().unwrap().remove("ended_connected");
    data["source_gaps_observed"] = 2.into();
    save(&path, &data)?;
    assert_eq!(archive::verify(&path, None)?.recorded_source_gaps, Some(2));
    Ok(())
}

#[test]
fn empty_csv_cannot_hide_freshness_or_snapshot_gap_contradictions() -> Result<()> {
    let (_scratch, path) = saved()?;
    let mut data = read(&path)?;
    data["csv_rows_flushed"] = 0.into();
    for key in [
        "poll_attempts",
        "received",
        "new_updates_observed",
        "selected",
        "disconnects",
        "reconnects",
    ] {
        data["counts"][key] = 0.into();
    }
    data["source_gaps_observed"] = 0.into();
    data["row_kind"] = "host_poll".into();
    save(&path, &data)?;
    let profile = Profile::parse(&fs::read_to_string(path.join("profile.toml"))?)?;
    let mut writer = csv::Writer::from_path(path.join("capture-0001.csv"))?;
    writer.write_record(csv_header(&profile))?;
    writer.flush()?;
    drop(writer);
    assert!(
        format!("{:#}", archive::verify(&path, None).unwrap_err()).contains("row kind contradicts")
    );
    data["last_status"]["freshness"] = "snapshots".into();
    save(&path, &data)?;
    assert!(
        format!("{:#}", archive::verify(&path, None).unwrap_err())
            .contains("source-gap total requires")
    );
    data.as_table_mut().unwrap().remove("source_gaps_observed");
    save(&path, &data)?;
    assert!(archive::verify(&path, None).is_ok());
    Ok(())
}

#[test]
fn rehashed_export_cannot_relabel_complete_csv_source_gap_totals() -> Result<()> {
    let (scratch, path) = saved()?;
    let output = scratch.path().join("exported");
    let mut receipt = archive::redecode(&path, &mapping(), &output)?;
    assert!(archive::verify_export(&output, None).is_ok());
    let report = output.join("source-capture.toml");
    let mut data: toml::Value = toml::from_str(&fs::read_to_string(&report)?)?;
    data["source_gaps_observed"] = 2.into();
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
    receipt.source_capture.recorded_source_gaps = Some(2);
    fs::write(
        output.join("receipt.toml"),
        toml::to_string_pretty(&receipt)?,
    )?;
    assert!(
        format!("{:#}", archive::verify_export(&output, None).unwrap_err())
            .contains("source counter gaps in complete CSV")
    );
    assert!(archive::verify(&path, None).is_ok());
    Ok(())
}

#[test]
fn counter_arithmetic_does_not_overflow_or_invent_a_missing_end_state() -> Result<()> {
    let (_scratch, path) = saved()?;
    let mut m: Manifest = toml::from_str(&fs::read_to_string(path.join("capture.toml"))?)?;
    m.counts.reconnects = u64::MAX;
    m.counts.disconnects = Some(u64::MAX);
    m.counts.poll_attempts = None;
    m.ended_connected = Some(false);
    assert!(
        format!("{:#}", validate(&m, &Profile::default()).unwrap_err())
            .contains("end connection state")
    );
    m.ended_connected = None;
    validate(&m, &Profile::default())?;
    Ok(())
}

#[test]
fn missing_filter_counts_do_not_override_known_mode_or_freshness_contracts() -> Result<()> {
    let (_scratch, path) = saved()?;
    let mut m: Manifest = toml::from_str(&fs::read_to_string(path.join("capture.toml"))?)?;
    m.counts.unchanged = None;
    m.counts.new_updates_observed = Some(6);
    m.counts.received = Some(6);
    m.counts.poll_attempts = Some(7);
    assert!(
        format!("{:#}", validate(&m, &Profile::default()).unwrap_err())
            .contains("every-update selection")
    );
    m.counts.new_updates_observed = Some(1);
    m.row_kind = Some("host_poll".into());
    m.last_status.as_mut().unwrap().freshness = Freshness::Snapshots;
    m.source_gaps_observed = None;
    assert!(
        format!("{:#}", validate(&m, &Profile::default()).unwrap_err())
            .contains("snapshot freshness")
    );
    Ok(())
}
