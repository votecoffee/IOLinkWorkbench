use super::*;
use crate::{
    capture::Counts,
    transport::{Freshness, Input, Payload, Validity},
};

fn summary() -> Summary {
    Summary {
        format_version: 1,
        name: "test".into(),
        session_id: "session".into(),
        interval_ms: 10,
        queue_capacity: 2,
        ..Default::default()
    }
}
fn profile() -> Profile {
    Profile::parse(include_str!("../../samples/simulator.toml")).unwrap()
}
fn sample(counter: u32) -> Sample {
    Sample {
        input: Input {
            received: Instant::now(),
            payload: Payload::new(&[1, 244]).unwrap(),
            validity: Validity::Valid,
            counter: Some(counter),
            source_time_us: Some(u64::from(counter) * 1000),
            source_age_us: Some(0),
            diagnostic_code: None,
        },
        freshness: Freshness::Counter32,
        stale: Some(false),
        classification: "forward",
        csv_sample_index: None,
        csv_enqueued: None,
        counts: Counts {
            received: u64::from(counter),
            ..Default::default()
        },
        source_gaps: Some(0),
    }
}
fn shared() -> Arc<Shared> {
    Arc::new(Shared {
        queue: ArrayQueue::new(2),
        offered: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
        stop: AtomicBool::new(false),
    })
}

#[test]
fn explicit_header_offsets_and_bounds_are_stable() -> Result<()> {
    let header = Header {
        abi_version: 1,
        payload_bytes: 100,
        publish_sequence: 42,
        publisher_pid: 17,
        flags: 3,
        published_qpc_ticks: 200,
        qpc_frequency: 10_000_000,
        process_creation_time: 1234,
    };
    let bytes = header.bytes();
    assert_eq!(&bytes[..8], layout::MAGIC);
    assert_eq!(&bytes[24..32], &42u64.to_le_bytes());
    assert_eq!(&bytes[48..56], &10_000_000u64.to_le_bytes());
    assert_eq!(Header::parse(&bytes)?.publish_sequence, 42);
    for (offset, value) in [
        (8, 2),
        (12, 65),
        (16, 128),
        (20, 0),
        (36, 4),
        (48, 0),
        (56, 0),
    ] {
        let mut bad = bytes;
        bad[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
        assert!(Header::parse(&bad).is_err(), "offset {offset}");
    }
    assert!(Header::parse(&bytes[..63]).is_err());
    Ok(())
}

#[test]
fn invalid_names_and_false_accounting_are_rejected() -> Result<()> {
    for name in ["", "Global\\unsafe", "bad/name", "µm", "space name"] {
        assert!(
            Config {
                name: name.into(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    let mut good = summary();
    good.events_offered = 10;
    good.events_dequeued = 8;
    good.queue_dropped = 2;
    good.coalesced = 5;
    good.publish_attempts = 3;
    good.final_refresh_attempts = 1;
    good.snapshots_written = 2;
    good.snapshots_replaced = 1;
    good.busy_skips = 2;
    good.final_snapshot_written = true;
    good.validate()?;
    for (key, value) in [
        ("queue_dropped", json!(3)),
        ("coalesced", json!(4)),
        ("snapshots_replaced", json!(2)),
        ("final_refresh_attempts", json!(2)),
        ("busy_skips", json!(3)),
        ("abandoned_recoveries", json!(3)),
    ] {
        let mut bad = serde_json::to_value(&good)?;
        bad[key] = value;
        assert!(
            serde_json::from_value::<Summary>(bad)?.validate().is_err(),
            "{key}"
        );
    }
    Ok(())
}

#[test]
fn latest_snapshot_preserves_raw_and_source_quality_without_claiming_history() -> Result<()> {
    let mut latest = Latest::default();
    let mut summary = summary();
    let shared = shared();
    for sequence in 1..=2 {
        latest.ingest(
            Event {
                sequence,
                elapsed_us: sequence * 1000,
                generation: 1,
                kind: Kind::Input(sample(sequence as u32)),
            },
            &mut summary,
        );
    }
    assert_eq!(summary.coalesced, 1);
    let bytes = latest.payload(&profile(), &summary, &shared, Instant::now(), "synthetic")?;
    let json: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(json["event_sequence"], 2);
    assert_eq!(json["input"]["raw_hex"], "01F4");
    assert_eq!(json["input"]["source_counter"], 2);
    let mut invalid = sample(3);
    invalid.input.validity = Validity::Invalid;
    latest.ingest(
        Event {
            sequence: 3,
            elapsed_us: 3000,
            generation: 1,
            kind: Kind::Input(invalid),
        },
        &mut summary,
    );
    let json: Value = serde_json::from_slice(&latest.payload(
        &profile(),
        &summary,
        &shared,
        Instant::now(),
        "synthetic",
    )?)?;
    assert!(json["input"]["values"].is_null());
    latest.ingest(
        Event {
            sequence: 4,
            elapsed_us: 4000,
            generation: 1,
            kind: Kind::Disconnected,
        },
        &mut summary,
    );
    assert!(latest.sample.is_none());
    Ok(())
}

#[test]
fn queue_saturation_counts_only_this_consumer() {
    let service = Service {
        shared: shared(),
        worker: None,
        initial: None,
        start: Instant::now(),
    };
    for counter in 1..=10000 {
        service.sample(1, sample(counter));
    }
    assert_eq!(service.shared.queue.len(), 2);
    assert_eq!(service.shared.dropped.load(Ordering::Relaxed), 9998);
}

#[cfg(windows)]
#[test]
fn native_snapshot_is_readable_and_collisions_preserve_committed_bytes() -> Result<()> {
    let config = Config {
        name: format!(
            "unit_{}_{}",
            std::process::id(),
            INSTANCE.fetch_add(1, Ordering::Relaxed)
        ),
        ..Default::default()
    };
    let mut mapping = native::Mapping::create(&config)?;
    let data=br#"{"format_version":1,"snapshot_sequence":1,"sampling_active":true,"gateway_connected_last_observed":false}"#;
    assert!(mapping.write(data, 1, 1)?.is_some());
    let first = read(&config.name, 50, 1000)?;
    assert_eq!(first.header.publish_sequence, 1);
    assert_eq!(first.publisher_process_alive, Some(true));
    assert!(!first.publish_age_exceeds_limit);
    assert!(native::Mapping::create(&config).is_err());
    let second = read(&config.name, 50, 1000)?;
    assert_eq!(first.payload, second.payload);
    drop(mapping);
    assert!(read(&config.name, 0, 1000).is_err());
    Ok(())
}
