use super::*;
use crate::archive;

fn manifest() -> Result<(tempfile::TempDir, std::path::PathBuf, toml::Value)> {
    let (scratch, path) = archive::tests::fixture(false, false)?;
    let data = toml::from_str(&std::fs::read_to_string(path.join("capture.toml"))?)?;
    Ok((scratch, path, data))
}
fn save(path: &std::path::Path, data: &toml::Value) -> Result<()> {
    std::fs::write(path.join("capture.toml"), toml::to_string_pretty(data)?)?;
    Ok(())
}
fn api(offered: u64) -> crate::local_api::Summary {
    crate::local_api::Summary {
        format_version: 1,
        url: "http://127.0.0.1:9".into(),
        session_id: "offline".into(),
        queue_capacity: 2,
        history_capacity: 2,
        events_offered: offered,
        events_ingested: offered - 1,
        queue_dropped: 1,
        history_evicted: offered.saturating_sub(3),
        requests: 1,
        failed_requests: 1,
        worker_failed: false,
    }
}

#[test]
fn absent_metrics_stay_unrecorded_and_metadata_hashes_identify_reviewed_bytes() -> Result<()> {
    let (_scratch, path, _) = manifest()?;
    let report = std::fs::read(path.join("capture.toml"))?;
    let profile = std::fs::read(path.join("profile.toml"))?;
    let result = archive::review_interfaces(&path, None)?;
    assert_eq!(
        result.capture_report_sha256,
        format!("{:x}", Sha256::digest(&report))
    );
    assert_eq!(
        result.saved_profile_sha256,
        format!("{:x}", Sha256::digest(&profile))
    );
    assert_eq!(result.interfaces.local_api_status, "not_recorded");
    assert!(!result.interfaces.any_recorded());
    assert!(result.interfaces.findings.is_empty());
    assert!(archive::review(&path, None, 1)?.interfaces.is_none());
    assert_eq!(std::fs::read(path.join("capture.toml"))?, report);
    assert_eq!(std::fs::read(path.join("profile.toml"))?, profile);
    Ok(())
}

#[test]
fn recorded_interface_counters_are_separate_from_source_and_csv_loss() -> Result<()> {
    let (_scratch, path, mut data) = manifest()?;
    let parsed: super::super::Manifest = toml::from_str(&toml::to_string_pretty(&data)?)?;
    let offered = super::super::expected_interface_offers(&parsed).unwrap() as u64;
    data.as_table_mut()
        .unwrap()
        .insert("local_api".into(), toml::Value::try_from(api(offered))?);
    save(&path, &data)?;
    let result = archive::review_interfaces(&path, None)?;
    assert_eq!(result.interfaces.local_api_status, "recorded");
    assert!(result.interfaces.offer_counts_checked_against_acquisition);
    assert!(
        result
            .interfaces
            .findings
            .iter()
            .any(|f| f.code == "api_queue_drops" && f.count == Some(1))
    );
    assert!(
        result
            .interfaces
            .findings
            .iter()
            .any(|f| f.code == "api_history_evictions" && f.category == "retention")
    );
    assert!(
        result
            .interfaces
            .findings
            .iter()
            .any(|f| f.code == "api_client_delivery_unknown")
    );
    assert_eq!(result.acquisition.recorded_logger_overruns, 0);
    assert!(archive::review(&path, None, 1)?.interfaces.is_some());
    Ok(())
}

#[test]
fn internally_balanced_offer_counts_must_match_acquisition_when_available() -> Result<()> {
    let (_scratch, path, mut data) = manifest()?;
    data.as_table_mut()
        .unwrap()
        .insert("local_api".into(), toml::Value::try_from(api(10))?);
    save(&path, &data)?;
    assert!(archive::review_interfaces(&path, None).is_err());
    assert!(archive::verify(&path, None).is_err());
    data["counts"].as_table_mut().unwrap().remove("received");
    save(&path, &data)?;
    let result = archive::review_interfaces(&path, None)?;
    assert!(!result.interfaces.offer_counts_checked_against_acquisition);
    assert_eq!(result.acquisition.recorded_received, None);
    Ok(())
}

#[test]
fn raw_only_review_needs_no_typed_column_and_rejects_corrupt_csv() -> Result<()> {
    let (_scratch, path, mut data) = manifest()?;
    // Use the existing capture engine to produce a valid raw-only bundle.
    let profile = crate::profile::Profile::parse(include_str!("../../../samples/raw-only.toml"))?;
    let raw = path.parent().unwrap().join("raw");
    crate::capture::run(
        &profile,
        &crate::capture::Options {
            output: Some(raw.clone()),
            duration: Some(std::time::Duration::from_millis(500)),
            ..Default::default()
        },
        &crate::capture::Stop::default(),
        None,
    )?;
    assert!(archive::review_interfaces(&raw, None)?.capture.rows_checked > 0);
    assert!(archive::review(&raw, None, 1).is_err());
    let first = raw.join("capture-0001.csv");
    std::fs::write(first, b"corrupt\n")?;
    assert!(archive::review_interfaces(&raw, None).is_err());
    data.as_table_mut()
        .unwrap()
        .insert("local_api".into(), toml::Value::try_from(api(10))?);
    data["local_api"]["events_ingested"] = toml::Value::Integer(1000);
    save(&path, &data)?;
    assert!(archive::review_interfaces(&path, None).is_err());
    Ok(())
}

#[test]
fn coalescing_retention_and_unknown_delivery_do_not_become_source_loss() -> Result<()> {
    let (_scratch, path) = archive::tests::fixture(true, false)?;
    let mut data: toml::Value =
        toml::from_str(&std::fs::read_to_string(path.join("capture.toml"))?)?;
    let parsed: super::super::Manifest = toml::from_str(&toml::to_string_pretty(&data)?)?;
    let n = super::super::expected_interface_offers(&parsed).unwrap() as u64;
    let mqtt = crate::mqtt::Summary {
        format_version: 1,
        broker: "127.0.0.1:9".into(),
        topic: "offline".into(),
        session_id: "offline".into(),
        client_id: "offline".into(),
        interval_ms: 10,
        queue_capacity: 2,
        events_offered: n,
        events_dequeued: n - 1,
        queue_dropped: 1,
        coalesced: n - 4,
        publish_attempts: 2,
        shutdown_discarded: 1,
        socket_writes_completed: 1,
        publish_write_failures: 1,
        broker_connect_attempts: 2,
        broker_connections: 1,
        connect_failures: 1,
        connection_losses: 1,
        ..Default::default()
    };
    let shm = crate::shared_memory::Summary {
        format_version: 1,
        name: "nonexistent_offline_object".into(),
        session_id: "offline".into(),
        interval_ms: 10,
        queue_capacity: 2,
        events_offered: n,
        events_dequeued: n - 1,
        queue_dropped: 1,
        coalesced: n - 3,
        publish_attempts: 2,
        final_refresh_attempts: 1,
        snapshots_written: 1,
        busy_skips: 2,
        ..Default::default()
    };
    data.as_table_mut()
        .unwrap()
        .insert("mqtt".into(), toml::Value::try_from(mqtt)?);
    data.as_table_mut()
        .unwrap()
        .insert("shared_memory".into(), toml::Value::try_from(shm)?);
    save(&path, &data)?;
    let review = archive::review_interfaces(&path, None)?;
    assert_eq!(review.capture.recorded_source_update_loss, "unknown");
    assert_eq!(review.capture.recorded_source_gaps, None);
    assert_eq!(review.acquisition.recorded_logger_overruns, 0);
    let finding = |code| {
        review
            .interfaces
            .findings
            .iter()
            .find(|f| f.code == code)
            .unwrap()
    };
    assert_eq!(finding("mqtt_coalesced").category, "intentional_selection");
    assert_eq!(finding("mqtt_write_uncertain").category, "unconfirmed");
    assert_eq!(finding("shm_busy_skips").count, Some(2));
    assert_eq!(
        finding("shm_final_state_unconfirmed").category,
        "unconfirmed"
    );
    assert_eq!(review.interfaces.mqtt.unwrap().socket_writes_completed, 1);
    Ok(())
}

#[test]
fn present_zero_counters_and_failed_worker_data_keep_their_distinct_states() -> Result<()> {
    let (_scratch, path, mut data) = manifest()?;
    let parsed: super::super::Manifest = toml::from_str(&toml::to_string_pretty(&data)?)?;
    let n = super::super::expected_interface_offers(&parsed).unwrap() as u64;
    let mut recorded = api(n);
    recorded.queue_dropped = 0;
    recorded.events_ingested = n;
    recorded.history_capacity = n as usize;
    recorded.history_evicted = 0;
    recorded.failed_requests = 0;
    data.as_table_mut()
        .unwrap()
        .insert("local_api".into(), toml::Value::try_from(&recorded)?);
    save(&path, &data)?;
    let review = archive::review_interfaces(&path, None)?;
    assert_eq!(review.interfaces.local_api_status, "recorded");
    assert_eq!(
        review.interfaces.local_api.as_ref().unwrap().queue_dropped,
        0
    );
    assert!(
        !review
            .interfaces
            .findings
            .iter()
            .any(|f| f.code == "api_queue_drops")
    );
    recorded.worker_failed = true;
    recorded.events_ingested = 0;
    data.as_table_mut()
        .unwrap()
        .insert("local_api".into(), toml::Value::try_from(&recorded)?);
    save(&path, &data)?;
    let review = archive::review_interfaces(&path, None)?;
    assert!(
        review
            .interfaces
            .findings
            .iter()
            .any(|f| f.code == "api_worker_failure")
    );
    assert!(!review.capture.capture_had_error);
    assert!(review.capture.rows_checked > 0);
    Ok(())
}

#[test]
fn joined_output_is_published_only_after_verification_and_never_overwritten() -> Result<()> {
    let (_scratch, path, _) = manifest()?;
    let joined = path.parent().unwrap().join("joined.csv");
    let result = archive::review_interfaces(&path, Some(&joined))?;
    assert_eq!(result.capture.joined_csv, Some(joined.clone()));
    let original = std::fs::read(&joined)?;
    assert!(archive::review_interfaces(&path, Some(&joined)).is_err());
    assert_eq!(std::fs::read(&joined)?, original);
    let target = path.parent().unwrap().join("must-not-publish.csv");
    let part = path.join("capture-0001.csv");
    std::fs::write(part, b"corrupt\n")?;
    assert!(archive::review_interfaces(&path, Some(&target)).is_err());
    assert!(!target.exists());
    Ok(())
}
