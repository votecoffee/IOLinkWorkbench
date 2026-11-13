use super::*;
use crate::capture::{self, Options, Stop};
use std::time::Duration;

#[test]
fn logger_metrics_are_verified_and_absent_legacy_measurements_remain_unavailable() -> Result<()> {
    let (_scratch, path) = fixture(false, false)?;
    let report = path.join("capture.toml");
    let original: toml::Value = toml::from_str(&fs::read_to_string(&report)?)?;
    assert!(verify(&path, None).is_ok());
    assert!(
        review(&path, None, 1)?
            .signal
            .recorded_timing
            .logger_metrics
            .is_some()
    );
    for (keys, value) in [
        (vec!["queue_capacity"], toml::Value::Integer(0)),
        (
            vec!["queue_high_water_observed"],
            toml::Value::Integer(1_000_001),
        ),
        (vec!["rows_dequeued"], toml::Value::Integer(0)),
        (vec!["queue_wait", "count"], toml::Value::Integer(0)),
        (vec!["sync", "count"], toml::Value::Integer(1_000_000)),
        (
            vec!["row_service", "p50_us"],
            toml::Value::Integer(i64::MAX),
        ),
        (vec!["flush", "mean_us"], toml::Value::Float(f64::NAN)),
        (vec!["sync", "mean_us"], toml::Value::Float(-1.0)),
        (vec!["unknown_metric"], toml::Value::Integer(1)),
    ] {
        let mut changed = original.clone();
        let mut slot = &mut changed["logger_metrics"];
        for key in &keys[..keys.len() - 1] {
            slot = &mut slot[*key];
        }
        slot.as_table_mut()
            .unwrap()
            .insert(keys[keys.len() - 1].into(), value);
        fs::write(&report, toml::to_string_pretty(&changed)?)?;
        assert!(
            verify(&path, None).is_err(),
            "accepted inconsistent metric {keys:?}"
        );
    }
    let mut changed = original.clone();
    changed["logger_metrics"]
        .as_table_mut()
        .unwrap()
        .remove("queue_wait");
    fs::write(&report, toml::to_string_pretty(&changed)?)?;
    assert!(verify(&path, None).is_err());
    let mut legacy = original;
    legacy.as_table_mut().unwrap().remove("logger_metrics");
    fs::write(&report, toml::to_string_pretty(&legacy)?)?;
    assert!(verify(&path, None).is_ok());
    assert!(
        review(&path, None, 1)?
            .signal
            .recorded_timing
            .logger_metrics
            .is_none()
    );
    Ok(())
}

fn fixture(snapshots: bool, backpressure: bool) -> Result<(tempfile::TempDir, PathBuf)> {
    fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let path = scratch.path().join("capture");
    let mut profile = Profile::parse(include_str!("../../samples/simulator.toml"))?;
    profile.connection.simulate_snapshots = snapshots;
    profile.logging.fields.clear();
    profile.fields[0].name = "level, \"measured\"".into();
    profile.fields[0].unit = "µm, \"calibrated\"".into();
    profile.logging.rotate_rows = 11;
    if backpressure {
        profile.logging.queue_capacity = 1;
    }
    let report = capture::run(
        &profile,
        &Options {
            output: Some(path.clone()),
            // Allow startup IO under parallel Windows tests; timing behavior has separate capture tests.
            duration: Some(Duration::from_millis(500)),
            simulated_writer_delay: if backpressure {
                Duration::from_millis(6)
            } else {
                Duration::ZERO
            },
            ..Options::default()
        },
        &Stop::default(),
        None,
    )?;
    ensure!(report.success(), "fixture capture failed: {report:?}");
    Ok((scratch, path))
}

fn read_csv(path: &Path) -> Result<(csv::StringRecord, Vec<csv::StringRecord>)> {
    let mut reader = csv::Reader::from_path(path)?;
    let header = reader.headers()?.clone();
    let rows = reader.records().collect::<std::result::Result<_, _>>()?;
    Ok((header, rows))
}

fn write_csv(path: &Path, header: &csv::StringRecord, rows: &[csv::StringRecord]) -> Result<()> {
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record(header)?;
    for row in rows {
        writer.write_record(row)?;
    }
    writer.flush()?;
    Ok(())
}

#[test]
fn joined_parts_preserve_every_cell_and_quoted_header() -> Result<()> {
    let (scratch, path) = fixture(false, false)?;
    let destination = scratch.path().join("joined.csv");
    let summary = verify(&path, Some(&destination))?;
    assert!(summary.csv_parts > 1);
    assert_eq!(summary.row_kind.as_deref(), Some("source_update"));
    assert_eq!(summary.missing_sample_indexes, 0);
    let (header, rows) = read_csv(&destination)?;
    assert!(header.get(12).unwrap().contains("level, \"measured\""));
    assert_eq!(rows.len() as u64, summary.rows_checked);
    let mut expected = Vec::new();
    for part in 1..=summary.csv_parts {
        let (part_header, part_rows) = read_csv(&path.join(format!("capture-{part:04}.csv")))?;
        assert_eq!(part_header, header);
        expected.extend(part_rows);
    }
    assert_eq!(expected, rows);
    assert!(verify(&path, Some(&destination)).is_err());
    assert_eq!(read_csv(&destination)?.1, rows);
    Ok(())
}

#[test]
fn snapshot_loss_and_source_gaps_keep_their_recorded_meaning() -> Result<()> {
    let (_scratch, path) = fixture(true, true)?;
    let summary = verify(&path, None)?;
    assert_eq!(summary.row_kind.as_deref(), Some("host_poll"));
    assert_eq!(summary.recorded_source_update_loss, "unknown");
    assert_eq!(summary.recorded_source_gaps, None);
    assert!(summary.missing_sample_indexes > 0);
    assert!(summary.valid_rows > 0);
    assert!(!summary.capture_had_error);
    Ok(())
}

#[test]
fn corrupted_typed_values_fail_before_join_publication() -> Result<()> {
    let (scratch, path) = fixture(false, false)?;
    let part = path.join("capture-0001.csv");
    let (header, mut rows) = read_csv(&part)?;
    let mut cells = rows[0].iter().map(str::to_owned).collect::<Vec<_>>();
    cells[12] = "changed after capture".into();
    rows[0] = csv::StringRecord::from(cells);
    write_csv(&part, &header, &rows)?;
    let destination = scratch.path().join("joined.csv");
    let error = verify(&path, Some(&destination)).unwrap_err();
    assert!(format!("{error:#}").contains("typed value"));
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn part_set_and_final_record_are_checked() -> Result<()> {
    let (_scratch, path) = fixture(false, false)?;
    let first = path.join("capture-0001.csv");
    let saved = path.join("saved-part.csv");
    fs::rename(&first, &saved)?;
    assert!(
        verify(&path, None)
            .unwrap_err()
            .to_string()
            .contains("CSV parts")
    );
    fs::rename(&saved, &first)?;
    let extra = path.join("capture-999999999.csv");
    fs::copy(&first, &extra)?;
    assert!(
        verify(&path, None)
            .unwrap_err()
            .to_string()
            .contains("unexpected CSV part")
    );
    fs::rename(extra, saved)?;
    let file = fs::OpenOptions::new().write(true).open(&first)?;
    file.set_len(file.metadata()?.len() - 1)?;
    assert!(format!("{:#}", verify(&path, None).unwrap_err()).contains("incomplete final record"));
    Ok(())
}

#[test]
fn ordering_is_checked_across_rotated_parts() -> Result<()> {
    let (_scratch, path) = fixture(false, false)?;
    let second = path.join("capture-0002.csv");
    let (header, mut rows) = read_csv(&second)?;
    let mut cells = rows[0].iter().map(str::to_owned).collect::<Vec<_>>();
    cells[2] = "1".into();
    rows[0] = csv::StringRecord::from(cells);
    write_csv(&second, &header, &rows)?;
    assert!(format!("{:#}", verify(&path, None).unwrap_err()).contains("sample indexes"));
    Ok(())
}

#[test]
fn pre_accounting_reports_are_supported_and_unconfirmed_rows_are_rejected() -> Result<()> {
    let (_scratch, path) = fixture(false, false)?;
    let report = path.join("capture.toml");
    let text = fs::read_to_string(&report)?;
    let legacy = text
        .replace("csv_rows_unconfirmed = 0\n", "")
        .replace("format_version = 1\n", "");
    fs::write(&report, &legacy)?;
    assert!(verify(&path, None).is_ok());
    fs::write(
        &report,
        text.replace("csv_rows_unconfirmed = 0", "csv_rows_unconfirmed = 1"),
    )?;
    assert!(
        verify(&path, None)
            .unwrap_err()
            .to_string()
            .contains("unconfirmed CSV rows")
    );
    Ok(())
}
