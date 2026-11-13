use super::*;
use crate::archive;
use std::{fs, path::Path};

fn data(path: &Path) -> Result<toml::Value> {
    Ok(toml::from_str(&fs::read_to_string(
        path.join("capture.toml"),
    )?)?)
}
fn save(path: &Path, data: &toml::Value) -> Result<()> {
    fs::write(path.join("capture.toml"), toml::to_string_pretty(data)?)?;
    Ok(())
}
fn copy(source: &Path, parent: &Path) -> Result<std::path::PathBuf> {
    let target = parent.join("copy");
    fs::create_dir(&target)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        fs::copy(entry.path(), target.join(entry.file_name()))?;
    }
    Ok(target)
}

#[test]
fn full_saved_identity_is_shared_by_field_and_interface_review() -> Result<()> {
    let (_scratch, path) = archive::tests::fixture(false, false)?;
    let original = data(&path)?;
    let reviewed = archive::review_interfaces(&path, None)?;
    let identity = reviewed.identity.as_ref().unwrap();
    assert_eq!(
        identity
            .last_accepted
            .as_ref()
            .unwrap()
            .identity
            .as_ref()
            .unwrap()
            .vendor_id,
        Some(65535)
    );
    assert_eq!(
        identity.accepted_recheck,
        "matched all configured identity fields"
    );
    assert_eq!(
        identity.last_accepted.as_ref().unwrap().firmware.as_deref(),
        Some("synthetic-1")
    );
    assert!(identity.rejected.is_none());
    assert_eq!(
        serde_json::to_value(&reviewed.identity)?,
        serde_json::to_value(archive::review(&path, None, 2)?.identity)?
    );
    assert_eq!(data(&path)?, original);
    Ok(())
}

#[test]
fn partial_and_missing_identity_do_not_turn_producer_claims_into_live_verification() -> Result<()> {
    let (_scratch, path) = archive::tests::fixture(false, false)?;
    let mut profile = Profile::parse(&fs::read_to_string(path.join("profile.toml"))?)?;
    profile.expected_identity.vendor_id = Some(65535);
    profile.expected_identity.device_id = Some(1);
    profile.save(&path.join("profile.toml"))?;
    let mut report = data(&path)?;
    report["last_status"]["identity"]
        .as_table_mut()
        .unwrap()
        .remove("device_id");
    save(&path, &report)?;
    let reviewed = archive::review_interfaces(&path, None)?.identity.unwrap();
    assert_eq!(reviewed.accepted_recheck, "partial identity match");
    report["last_status"]
        .as_table_mut()
        .unwrap()
        .remove("identity");
    report["identity_check"] = "matched all configured identity fields".into();
    save(&path, &report)?;
    let reviewed = archive::review_interfaces(&path, None)?.identity.unwrap();
    assert_eq!(
        reviewed.accepted_recheck,
        "recorded device identity unavailable"
    );
    assert_eq!(
        reviewed.producer_check.as_deref(),
        Some("matched all configured identity fields")
    );
    let compared = archive::compare_captures(&path, &path)?;
    assert_eq!(compared.timing_context, "settings_unavailable");
    assert!(compared.metrics.iter().all(|v| v.magnitude.is_none()));
    Ok(())
}

#[test]
fn accepted_pin_contradictions_reject_all_capture_paths_without_outputs() -> Result<()> {
    let (scratch, path) = archive::tests::fixture(false, false)?;
    let mut profile = Profile::parse(&fs::read_to_string(path.join("profile.toml"))?)?;
    profile.expected_identity.serial = Some("wrong serial".into());
    profile.save(&path.join("profile.toml"))?;
    let joined = scratch.path().join("joined.csv");
    let exported = scratch.path().join("export");
    assert!(
        format!("{:#}", archive::verify(&path, Some(&joined)).unwrap_err())
            .contains("last accepted identity contradicts")
    );
    assert!(archive::review_interfaces(&path, None).is_err());
    assert!(archive::review(&path, None, 2).is_err());
    assert!(archive::compare_captures(&path, &path).is_err());
    assert!(archive::redecode(&path, &profile, &exported).is_err());
    assert!(!joined.exists() && !exported.exists());
    Ok(())
}

#[test]
fn rejected_status_remains_separate_and_does_not_replace_accepted_identity() -> Result<()> {
    let (_scratch, path) = archive::tests::fixture(false, false)?;
    let mut report = data(&path)?;
    let mut rejected = report["last_status"].clone();
    rejected["identity"]["serial"] = "rejected replacement".into();
    report
        .as_table_mut()
        .unwrap()
        .insert("rejected_status".into(), rejected);
    report.as_table_mut().unwrap().insert(
        "terminal_error".into(),
        "synthetic rejected reconnect".into(),
    );
    save(&path, &report)?;
    let reviewed = archive::review_interfaces(&path, None)?;
    let identity = reviewed.identity.unwrap();
    assert_eq!(
        identity
            .last_accepted
            .unwrap()
            .identity
            .unwrap()
            .serial
            .as_deref(),
        Some("SIM-0001")
    );
    assert_eq!(
        identity
            .rejected
            .unwrap()
            .identity
            .unwrap()
            .serial
            .as_deref(),
        Some("rejected replacement")
    );
    assert!(reviewed.capture.capture_had_error);
    Ok(())
}

#[test]
fn observed_device_and_master_changes_suppress_deltas_without_serial_pins() -> Result<()> {
    let (scratch, path) = archive::tests::fixture(false, false)?;
    let other = copy(&path, scratch.path())?;
    let original = data(&other)?;
    for change in ["serial", "firmware", "port"] {
        let mut report = original.clone();
        match change {
            "serial" => report["last_status"]["identity"]["serial"] = "another sensor".into(),
            "firmware" => report["last_status"]["firmware"] = "other firmware".into(),
            _ => report["last_status"]["port"] = 2.into(),
        }
        save(&other, &report)?;
        let compared = archive::compare_captures(&path, &other)?;
        assert_eq!(compared.timing_context, "settings_differ");
        assert!(compared.metrics.iter().all(|v| v.magnitude.is_none()));
    }
    let mut report = original;
    report["last_status"]["identity"]["device_id"] = 0x100_0000.into();
    save(&other, &report)?;
    assert!(format!("{:#}", archive::verify(&other, None).unwrap_err()).contains("24-bit"));
    Ok(())
}

#[test]
fn legacy_minimal_status_has_no_fabricated_identity_and_no_comparison_delta() -> Result<()> {
    let (_scratch, path) = archive::tests::fixture(false, false)?;
    let mut report = data(&path)?;
    for key in [
        "model",
        "firmware",
        "master_serial",
        "port",
        "port_mode",
        "device_status",
        "port_event",
        "device_event",
        "identity",
    ] {
        report["last_status"].as_table_mut().unwrap().remove(key);
    }
    report.as_table_mut().unwrap().remove("identity_check");
    save(&path, &report)?;
    let with_pins = archive::review_interfaces(&path, None)?.identity.unwrap();
    assert_eq!(
        with_pins.accepted_recheck,
        "recorded device identity unavailable"
    );
    assert!(with_pins.last_accepted.unwrap().identity.is_none());
    let mut profile = Profile::parse(&fs::read_to_string(path.join("profile.toml"))?)?;
    profile.expected_identity = Identity::default();
    profile.save(&path.join("profile.toml"))?;
    assert!(archive::review_interfaces(&path, None)?.identity.is_none());
    assert!(archive::review(&path, None, 1)?.identity.is_none());
    assert_eq!(
        archive::compare_captures(&path, &path)?.timing_context,
        "settings_unavailable"
    );
    Ok(())
}

#[test]
fn first_connection_rejection_preserves_diagnostics_without_an_accepted_device() -> Result<()> {
    use crate::capture::{self, Options, Stop};
    use std::time::Duration;
    fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let path = scratch.path().join("rejected-first");
    let mut profile = Profile::parse(include_str!("../../../samples/simulator.toml"))?;
    profile.expected_identity.serial = Some("different expected unit".into());
    let report = capture::run(
        &profile,
        &Options {
            output: Some(path.clone()),
            duration: Some(Duration::from_millis(250)),
            ..Default::default()
        },
        &Stop::default(),
        None,
    )?;
    assert!(!report.success() && report.last_status.is_none() && report.rejected_status.is_some());
    let reviewed = archive::review_interfaces(&path, None)?;
    assert_eq!(reviewed.capture.rows_checked, 0);
    assert!(reviewed.capture.capture_had_error);
    let identity = reviewed.identity.unwrap();
    assert!(identity.last_accepted.is_none());
    assert_eq!(
        identity.accepted_recheck,
        "no accepted connection status recorded"
    );
    assert_eq!(
        identity
            .rejected
            .unwrap()
            .identity
            .unwrap()
            .serial
            .as_deref(),
        Some("SIM-0001")
    );
    Ok(())
}
