use super::*;
use crate::{archive, capture::Spacing, profile::Profile};
use std::{fs, path::Path};

fn read(path: &Path) -> Result<toml::Value> {
    Ok(toml::from_str(&fs::read_to_string(
        path.join("capture.toml"),
    )?)?)
}
fn save(path: &Path, data: &toml::Value) -> Result<()> {
    fs::write(path.join("capture.toml"), toml::to_string_pretty(data)?)?;
    Ok(())
}

#[test]
fn malformed_spacing_is_rejected_by_all_archive_entry_points_without_publication() -> Result<()> {
    let (scratch, path) = archive::tests::fixture(false, false)?;
    let original = read(&path)?;
    let mapping = Profile::parse(&fs::read_to_string(path.join("profile.toml"))?)?;
    let raw = fs::read(path.join("capture-0001.csv"))?;
    let joined = scratch.path().join("joined.csv");
    let exported = scratch.path().join("exported");
    for section in [
        "host_poll_start_spacing",
        "host_receive_spacing",
        "host_new_update_spacing",
        "gateway_update_spacing",
    ] {
        for (key, value) in [
            ("count", toml::Value::Integer(0)),
            ("p50_us", toml::Value::Integer(i64::MAX)),
            ("mean_hz", toml::Value::Float(f64::NAN)),
            ("mean_hz", toml::Value::Float(f64::INFINITY)),
            ("mean_hz", toml::Value::Float(-1.0)),
            ("mean_hz", toml::Value::Float(0.0)),
        ] {
            let mut data = original.clone();
            data[section][key] = value;
            save(&path, &data)?;
            let error = archive::verify(&path, Some(&joined)).unwrap_err();
            assert!(format!("{error:#}").contains(section));
            assert!(archive::review_interfaces(&path, None).is_err());
            assert!(archive::review(&path, None, 2).is_err());
            assert!(archive::compare_captures(&path, &path).is_err());
            assert!(archive::redecode(&path, &mapping, &exported).is_err());
            assert!(!joined.exists() && !exported.exists());
        }
    }
    assert_eq!(fs::read(path.join("capture-0001.csv"))?, raw);
    Ok(())
}

#[test]
fn available_observation_counts_are_crosschecked_without_using_selected_csv_rows() -> Result<()> {
    let (_scratch, path) = archive::tests::fixture(false, true)?;
    let original = read(&path)?;
    assert!(original["counts"]["logger_overruns"].as_integer().unwrap() > 0);
    assert!(archive::verify(&path, None).is_ok());
    for section in ["host_poll_start_spacing", "host_receive_spacing"] {
        let mut data = original.clone();
        data[section]["count"] = (data[section]["count"].as_integer().unwrap() + 1).into();
        save(&path, &data)?;
        assert!(
            format!("{:#}", archive::verify(&path, None).unwrap_err())
                .contains("observation count")
        );
    }
    let mut data = original.clone();
    data["counts"]["new_updates_observed"] = 0.into();
    save(&path, &data)?;
    assert!(
        format!("{:#}", archive::verify(&path, None).unwrap_err()).contains("new source updates")
    );
    let mut data = original.clone();
    data["host_new_update_spacing"]["count"] = (data["gateway_update_spacing"]["count"]
        .as_integer()
        .unwrap()
        - 1)
    .into();
    save(&path, &data)?;
    assert!(
        format!("{:#}", archive::verify(&path, None).unwrap_err()).contains("new-update receives")
    );
    Ok(())
}

#[test]
fn missing_legacy_measurements_and_counts_remain_unavailable() -> Result<()> {
    let (_scratch, path) = archive::tests::fixture(false, false)?;
    let mut data = read(&path)?;
    for key in [
        "poll_attempts",
        "received",
        "new_updates_observed",
        "disconnects",
    ] {
        data["counts"].as_table_mut().unwrap().remove(key);
    }
    save(&path, &data)?;
    assert!(archive::verify(&path, None).is_ok());
    for key in [
        "host_poll_start_spacing",
        "host_receive_spacing",
        "host_new_update_spacing",
        "gateway_update_spacing",
    ] {
        data.as_table_mut().unwrap().remove(key);
    }
    save(&path, &data)?;
    let reviewed = archive::review_interfaces(&path, None)?;
    let timing = reviewed.acquisition.recorded_timing;
    assert!(
        timing.host_poll_start_spacing.is_none()
            && timing.host_receive_spacing.is_none()
            && timing.host_new_update_spacing.is_none()
            && timing.gateway_update_spacing.is_none()
    );
    assert!(reviewed.acquisition.recorded_received.is_none());
    Ok(())
}

#[test]
fn gateway_and_host_clocks_have_separate_duration_bounds() -> Result<()> {
    let (_scratch, path) = archive::tests::fixture(false, false)?;
    let mut data = read(&path)?;
    let gap = data["elapsed_us"].as_integer().unwrap() as u64 + 1_000_000;
    let spacing = Spacing {
        count: 1,
        p50_us: gap,
        p95_us: gap,
        p99_us: gap,
        longest_us: gap,
        mean_hz: 1_000_000.0 / gap as f64,
    };
    data["gateway_update_spacing"] = toml::Value::try_from(&spacing)?;
    save(&path, &data)?;
    let reviewed = archive::review_interfaces(&path, None)?;
    assert_eq!(
        reviewed
            .acquisition
            .recorded_timing
            .gateway_update_spacing
            .unwrap()
            .longest_us,
        gap
    );
    let count = data["host_receive_spacing"]["count"].as_integer().unwrap() as u64;
    data["host_receive_spacing"] = toml::Value::try_from(Spacing { count, ..spacing })?;
    save(&path, &data)?;
    assert!(
        format!("{:#}", archive::verify(&path, None).unwrap_err())
            .contains("host acquisition window")
    );
    Ok(())
}

#[test]
fn snapshots_cannot_acquire_source_cadence_from_edited_metadata() -> Result<()> {
    let (_scratch, path) = archive::tests::fixture(true, false)?;
    let original = read(&path)?;
    assert!(archive::verify(&path, None).is_ok());
    for section in ["host_new_update_spacing", "gateway_update_spacing"] {
        let mut data = original.clone();
        data.as_table_mut().unwrap().insert(
            section.into(),
            toml::Value::try_from(Spacing {
                count: 1,
                p50_us: 1000,
                p95_us: 1000,
                p99_us: 1000,
                longest_us: 1000,
                mean_hz: 1000.0,
            })?,
        );
        save(&path, &data)?;
        assert!(
            format!("{:#}", archive::verify(&path, None).unwrap_err())
                .contains("snapshot acquisition")
        );
    }
    Ok(())
}

#[test]
fn rehashed_export_with_invalid_source_timing_is_rejected_locally() -> Result<()> {
    let (scratch, path) = archive::tests::fixture(false, false)?;
    let mapping = Profile::parse(&fs::read_to_string(path.join("profile.toml"))?)?;
    let output = scratch.path().join("exported");
    let mut receipt = archive::redecode(&path, &mapping, &output)?;
    assert!(archive::verify_export(&output, None).is_ok());
    let report = output.join("source-capture.toml");
    let mut data: toml::Value = toml::from_str(&fs::read_to_string(&report)?)?;
    data["host_receive_spacing"]["p99_us"] = 0.into();
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
            .contains("host_receive_spacing")
    );
    assert!(archive::verify(&path, None).is_ok());
    Ok(())
}

#[test]
fn zero_duration_and_extreme_spacing_statistics_are_checked_without_overflow() -> Result<()> {
    let mut value = Spacing {
        count: u64::MAX,
        p50_us: 0,
        p95_us: 0,
        p99_us: 0,
        longest_us: 0,
        mean_hz: 0.0,
    };
    value.validate()?;
    value.mean_hz = 1.0;
    assert!(value.validate().is_err());
    value.longest_us = u64::MAX;
    value.mean_hz = 1_000_000.0 / u64::MAX as f64;
    value.validate()?;
    value.mean_hz = 1_000_000.0;
    value.validate()?;
    value.mean_hz = 1_000_001.0;
    assert!(value.validate().is_err());
    value.count = 1;
    value.longest_us = 1000;
    value.p50_us = 1000;
    value.p95_us = 1000;
    value.p99_us = 1000;
    value.mean_hz = 1000.0;
    value.validate()?;
    value.mean_hz = 999.0;
    assert!(value.validate().is_err());
    value.mean_hz = 1001.0;
    assert!(value.validate().is_err());
    Ok(())
}
