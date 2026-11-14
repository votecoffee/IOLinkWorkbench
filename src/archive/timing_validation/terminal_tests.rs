use super::*;
use crate::{
    archive,
    capture::{self, Options, Stop},
    profile::{LogMode, Profile},
};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

fn read(path: &Path) -> Result<toml::Value> {
    Ok(toml::from_str(&fs::read_to_string(
        path.join("capture.toml"),
    )?)?)
}
fn save(path: &Path, value: &toml::Value) -> Result<()> {
    fs::write(path.join("capture.toml"), toml::to_string_pretty(value)?)?;
    Ok(())
}
fn last_host(path: &Path, data: &toml::Value) -> Result<u64> {
    let mut last = None;
    for part in 1..=data["csv_parts"].as_integer().unwrap() {
        for row in csv::Reader::from_path(path.join(format!("capture-{part:04}.csv")))?.records() {
            last = Some(row?[1].parse()?);
        }
    }
    last.context("fixture has no CSV rows")
}
fn copy(path: &Path, target: &Path) -> Result<()> {
    fs::create_dir(target)?;
    for file in fs::read_dir(path)? {
        let file = file?;
        fs::copy(file.path(), target.join(file.file_name()))?;
    }
    Ok(())
}

#[test]
fn terminal_age_contradictions_reject_all_capture_entry_points_without_publication() -> Result<()> {
    let (scratch, path) = archive::tests::fixture(true, false)?;
    let original = read(&path)?;
    let end = original["elapsed_us"].as_integer().unwrap();
    let last = last_host(&path, &original)? as i64;
    assert!(last > 0);
    let mapping = Profile::load(&path.join("profile.toml"))?;
    let joined = scratch.path().join("joined.csv");
    let exported = scratch.path().join("exported");
    for kind in 0..3 {
        let mut data = original.clone();
        if kind == 0 {
            data["time_since_last_receive_us"] = (end + 1).into();
        } else if kind == 1 {
            data["time_since_last_receive_us"] = (end - last + 1).into();
        } else {
            data["elapsed_us"] = (end + 1000).into();
        }
        save(&path, &data)?;
        let error = archive::verify(&path, Some(&joined)).unwrap_err();
        assert!(format!("{error:#}").contains("time_since_last_receive_us"));
        assert!(archive::review_interfaces(&path, None).is_err());
        assert!(archive::review(&path, None, 2).is_err());
        assert!(archive::compare_captures(&path, &path).is_err());
        assert!(archive::redecode(&path, &mapping, &exported).is_err());
        assert!(!joined.exists() && !exported.exists());
    }
    save(&path, &original)?;
    archive::verify(&path, None)?;
    Ok(())
}

#[test]
fn complete_receive_coverage_allows_independent_microsecond_rounding_and_safe_extremes()
-> Result<()> {
    let (_scratch, path) = archive::tests::fixture(true, false)?;
    let mut data = read(&path)?;
    let last = last_host(&path, &data)?;
    let end = data["elapsed_us"].as_integer().unwrap() as u64 + 2;
    data["elapsed_us"] = (end as i64).into();
    for age in [end - last, end - last - 1] {
        data["time_since_last_receive_us"] = (age as i64).into();
        save(&path, &data)?;
        let result = archive::review_interfaces(&path, None)?;
        assert_eq!(
            result
                .acquisition
                .recorded_timing
                .time_since_last_receive_us,
            Some(age)
        );
    }
    let mut m: Manifest = toml::from_str(&toml::to_string(&data)?)?;
    m.elapsed_us = u64::MAX;
    m.time_since_last_receive_us = Some(0);
    finish_csv(&m, Some(u64::MAX))?;
    m.time_since_last_receive_us = Some(u64::MAX);
    assert!(finish_csv(&m, Some(u64::MAX)).is_err());
    finish_csv(&m, Some(0))?;
    Ok(())
}

#[test]
fn filtered_and_dropped_populations_do_not_reconstruct_terminal_age_from_csv() -> Result<()> {
    let (scratch, dropped) = archive::tests::fixture(true, true)?;
    let filtered = scratch.path().join("filtered");
    let mut profile = Profile::parse(include_str!("../../../samples/simulator.toml"))?;
    profile.connection.simulate_snapshots = true;
    profile.logging.mode = LogMode::OnChange;
    let report = capture::run(
        &profile,
        &Options {
            output: Some(filtered.clone()),
            duration: Some(Duration::from_millis(500)),
            ..Default::default()
        },
        &Stop::default(),
        None,
    )?;
    assert!(report.success() && report.counts.selected < report.counts.received);
    for path in [&filtered, &dropped] {
        let mut data = read(path)?;
        assert!(
            data["counts"]["logger_overruns"].as_integer().unwrap() > 0
                || data["counts"]["selected"] != data["counts"]["received"]
        );
        // A longer recorded acquisition may contain unlogged final receives.
        data["elapsed_us"] = (data["elapsed_us"].as_integer().unwrap() + 1_000_000).into();
        let age = data["time_since_last_receive_us"].as_integer().unwrap() as u64;
        save(path, &data)?;
        let reviewed = archive::review_interfaces(path, None)?;
        assert_eq!(
            reviewed
                .acquisition
                .recorded_timing
                .time_since_last_receive_us,
            Some(age)
        );
        assert_eq!(reviewed.capture.recorded_source_update_loss, "unknown");
    }
    Ok(())
}

#[test]
fn local_exports_reject_rehashed_terminal_age_contradictions() -> Result<()> {
    let (scratch, path) = archive::tests::fixture(true, false)?;
    let profile = Profile::load(&path.join("profile.toml"))?;
    let output = scratch.path().join("export");
    let mut receipt = archive::redecode(&path, &profile, &output)?;
    let report = output.join("source-capture.toml");
    let original = fs::read(path.join("capture.toml"))?;
    let mut data: toml::Value = toml::from_str(std::str::from_utf8(&original)?)?;
    data["elapsed_us"] = (data["elapsed_us"].as_integer().unwrap() + 1000).into();
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
            .contains("time_since_last_receive_us")
    );
    assert_eq!(fs::read(path.join("capture.toml"))?, original);
    archive::verify(&path, None)?;
    Ok(())
}

#[test]
fn missing_age_stays_unavailable_and_no_receive_cannot_claim_even_zero_age() -> Result<()> {
    let (scratch, path) = archive::tests::fixture(true, false)?;
    let mut data = read(&path)?;
    data.as_table_mut()
        .unwrap()
        .remove("time_since_last_receive_us");
    save(&path, &data)?;
    let review = archive::review_interfaces(&path, None)?;
    assert_eq!(
        review
            .acquisition
            .recorded_timing
            .time_since_last_receive_us,
        None
    );
    assert!(
        serde_json::to_value(review.acquisition.recorded_timing)?
            .get("time_since_last_receive_us")
            .is_none()
    );
    let empty = scratch.path().join("empty");
    let stop = Stop::default();
    stop.request();
    let profile = Profile::default();
    let report = capture::run(
        &profile,
        &Options {
            output: Some(empty.clone()),
            ..Default::default()
        },
        &stop,
        None,
    )?;
    assert!(report.success() && report.counts.received == 0);
    archive::verify(&empty, None)?;
    let mut data = read(&empty)?;
    data.as_table_mut()
        .unwrap()
        .insert("time_since_last_receive_us".into(), 0.into());
    save(&empty, &data)?;
    assert!(
        format!("{:#}", archive::verify(&empty, None).unwrap_err())
            .contains("without received input")
    );
    Ok(())
}

#[test]
fn terminal_age_comparison_requires_equal_duration_and_available_values() -> Result<()> {
    let (scratch, path) = archive::tests::fixture(true, true)?;
    let other: PathBuf = scratch.path().join("other");
    copy(&path, &other)?;
    let original = read(&path)?;
    let age = original["time_since_last_receive_us"].as_integer().unwrap() as u64;
    let mut data = original;
    data["time_since_last_receive_us"] = 0.into();
    save(&other, &data)?;
    let compared = archive::compare_captures(&path, &other)?;
    let metric = compared
        .metrics
        .iter()
        .find(|m| m.name == "terminal_receive_age")
        .unwrap();
    assert_eq!(metric.magnitude, Some(age));
    data["elapsed_us"] = (data["elapsed_us"].as_integer().unwrap() + 1).into();
    save(&other, &data)?;
    let compared = archive::compare_captures(&path, &other)?;
    assert!(
        compared
            .metrics
            .iter()
            .find(|m| m.name == "terminal_receive_age")
            .unwrap()
            .magnitude
            .is_none()
    );
    data.as_table_mut()
        .unwrap()
        .remove("time_since_last_receive_us");
    save(&other, &data)?;
    let compared = archive::compare_captures(&path, &other)?;
    let metric = compared
        .metrics
        .iter()
        .find(|m| m.name == "terminal_receive_age")
        .unwrap();
    assert_eq!(metric.right, None);
    assert_eq!(metric.magnitude, None);
    Ok(())
}
