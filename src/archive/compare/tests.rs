use super::*;
use std::{fs, path::PathBuf};

fn copy(source: &Path, parent: &Path) -> Result<PathBuf> {
    let target = parent.join("copy");
    fs::create_dir(&target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        fs::copy(entry.path(), target.join(entry.file_name()))?;
    }
    Ok(target)
}
fn edit(path: &Path, edit: impl FnOnce(&mut toml::Value)) -> Result<()> {
    let mut data: toml::Value = toml::from_str(&fs::read_to_string(path)?)?;
    edit(&mut data);
    fs::write(path, toml::to_string_pretty(&data)?)?;
    Ok(())
}

#[test]
fn matching_capture_preserves_zero_unknown_and_exact_reviews() -> Result<()> {
    let (_scratch, path) = super::super::tests::fixture(false, false)?;
    let result = compare_captures(&path, &path)?;
    assert_eq!(result.timing_context, "matching_recorded_context");
    assert_eq!(result.count_context, "matching_recorded_context");
    let zero = result
        .metrics
        .iter()
        .find(|v| v.name == "logger_overruns")
        .unwrap();
    assert_eq!(
        (zero.left, zero.right, zero.magnitude),
        (Some(0), Some(0), Some(0))
    );
    let absent = result.metrics.iter().find(|v| v.domain == "mqtt").unwrap();
    assert_eq!(
        (absent.left, absent.right, absent.magnitude),
        (None, None, None)
    );
    assert_eq!(absent.assessment, "metric_unavailable");
    assert_eq!(
        serde_json::to_value(&result.left)?,
        serde_json::to_value(super::super::review_interfaces(&path, None)?)?
    );
    assert_eq!(
        result.left.saved_profile_sha256,
        result.right.saved_profile_sha256
    );
    let encoded = toml::to_string_pretty(&result)?;
    assert!(toml::from_str::<toml::Value>(&encoded).is_ok());
    Ok(())
}

#[test]
fn differing_exposure_blocks_counts_but_not_spacing() -> Result<()> {
    let (scratch, path) = super::super::tests::fixture(false, false)?;
    let other = copy(&path, scratch.path())?;
    edit(&other.join("capture.toml"), |v| {
        let elapsed = v["elapsed_us"].as_integer().unwrap();
        v["elapsed_us"] = (elapsed + 1).into();
    })?;
    let result = compare_captures(&path, &other)?;
    assert_eq!(result.timing_context, "matching_recorded_context");
    assert_eq!(result.count_context, "settings_differ");
    assert!(
        result
            .metrics
            .iter()
            .filter(|v| v.unit == "count")
            .all(|v| v.magnitude.is_none())
    );
    assert!(
        result
            .metrics
            .iter()
            .any(|v| v.unit == "us" && v.magnitude == Some(0))
    );
    Ok(())
}

#[test]
fn selection_changes_and_legacy_missing_context_suppress_deltas() -> Result<()> {
    let (scratch, path) = super::super::tests::fixture(false, false)?;
    let other = copy(&path, scratch.path())?;
    edit(&other.join("profile.toml"), |v| {
        v["logging"]["mode"] = "on_change".into()
    })?;
    let result = compare_captures(&path, &other)?;
    assert_eq!(result.timing_context, "settings_differ");
    assert!(result.metrics.iter().all(|v| v.magnitude.is_none()));
    fs::copy(path.join("profile.toml"), other.join("profile.toml"))?;
    edit(&other.join("capture.toml"), |v| {
        v.as_table_mut().unwrap().remove("requested_poll_us");
        v.as_table_mut().unwrap().remove("host_receive_spacing");
        v["counts"].as_table_mut().unwrap().remove("received");
    })?;
    let result = compare_captures(&path, &other)?;
    assert_eq!(result.timing_context, "settings_unavailable");
    let received = result
        .metrics
        .iter()
        .find(|v| v.name == "received")
        .unwrap();
    assert!(received.left.is_some() && received.right.is_none());
    assert_eq!(received.assessment, "metric_unavailable");
    assert!(
        result
            .metrics
            .iter()
            .filter(|v| v.domain == "host_receive")
            .all(|v| v.right.is_none())
    );
    Ok(())
}

#[test]
fn snapshot_counter_and_corrupt_capture_are_never_treated_as_equivalent() -> Result<()> {
    let (_scratch, counter) = super::super::tests::fixture(false, false)?;
    let (_other, snapshot) = super::super::tests::fixture(true, true)?;
    let result = compare_captures(&counter, &snapshot)?;
    assert_eq!(result.timing_context, "settings_differ");
    assert!(result.metrics.iter().all(|v| v.magnitude.is_none()));
    assert!(result.right.capture.recorded_source_gaps.is_none());
    let original = fs::read(counter.join("capture.toml"))?;
    fs::write(snapshot.join("capture-0001.csv"), b"corrupt\n")?;
    let error = compare_captures(&counter, &snapshot).unwrap_err();
    assert!(format!("{error:#}").contains("right capture verification failed"));
    assert_eq!(fs::read(counter.join("capture.toml"))?, original);
    Ok(())
}

#[test]
fn arithmetic_is_unsigned_safe_and_no_missing_value_becomes_zero() {
    let up = metric(
        "test",
        "test".into(),
        "count",
        Some(0),
        Some(u64::MAX),
        "matching_recorded_context",
    );
    assert_eq!((up.change, up.magnitude), ("increase", Some(u64::MAX)));
    let down = metric(
        "test",
        "test".into(),
        "count",
        Some(u64::MAX),
        Some(0),
        "matching_recorded_context",
    );
    assert_eq!((down.change, down.magnitude), ("decrease", Some(u64::MAX)));
    let absent = metric(
        "test",
        "test".into(),
        "count",
        None,
        Some(0),
        "matching_recorded_context",
    );
    assert_eq!((absent.change, absent.magnitude), ("not_compared", None));
}

#[test]
fn invalid_spacing_and_recorded_failure_do_not_produce_a_timing_delta() -> Result<()> {
    let (scratch, path) = super::super::tests::fixture(false, false)?;
    let other = copy(&path, scratch.path())?;
    edit(&other.join("capture.toml"), |v| {
        v["host_receive_spacing"]["count"] = 0.into();
    })?;
    assert!(
        format!("{:#}", compare_captures(&path, &other).unwrap_err())
            .contains("right capture verification failed")
    );
    fs::copy(path.join("capture.toml"), other.join("capture.toml"))?;
    edit(&other.join("capture.toml"), |v| {
        v.as_table_mut()
            .unwrap()
            .insert("terminal_error".into(), "synthetic saved error".into());
    })?;
    let result = compare_captures(&path, &other)?;
    assert_eq!(result.timing_context, "capture_or_worker_error");
    assert!(result.metrics.iter().all(|v| v.magnitude.is_none()));
    assert!(result.right.capture.capture_had_error);
    Ok(())
}
