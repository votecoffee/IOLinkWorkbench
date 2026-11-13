use super::*;
use crate::{
    capture::Counts,
    transport::{Freshness, Input, Payload, Validity},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
};

fn profile() -> Profile {
    Profile::parse(include_str!("../../samples/simulator.toml")).unwrap()
}
fn shared(capacity: usize) -> Arc<Shared> {
    Arc::new(Shared {
        queue: ArrayQueue::new(capacity),
        offered: AtomicU64::new(0),
        dropped: AtomicU64::new(0),
        stop: AtomicBool::new(false),
    })
}
fn summary() -> Summary {
    Summary {
        format_version: 1,
        broker: "127.0.0.1:1883".into(),
        topic: "test/input".into(),
        session_id: "session".into(),
        client_id: "IOLWB0123456789".into(),
        interval_ms: 100,
        queue_capacity: 2,
        ..Default::default()
    }
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
fn event(sequence: u64) -> Event {
    Event {
        sequence,
        elapsed_us: sequence * 1000,
        generation: 1,
        kind: Kind::Input(sample(sequence as u32)),
    }
}

#[test]
fn mqtt_framing_has_clean_session_qos_zero_and_no_retain_or_identifier() -> Result<()> {
    assert_eq!(
        wire::connect_packet("abc"),
        [
            0x10, 15, 0, 4, b'M', b'Q', b'T', b'T', 4, 2, 0, 10, 0, 3, b'a', b'b', b'c'
        ]
    );
    assert_eq!(
        wire::publish_packet("a", b"b")?,
        [0x30, 4, 0, 1, b'a', b'b']
    );
    let encoded = wire::publish_packet("x", &[0; 125])?;
    assert_eq!(&encoded[..3], &[0x30, 128, 1]);
    assert!(wire::publish_packet("x", &vec![0; wire::MAX_PAYLOAD + 1]).is_err());
    Ok(())
}

#[test]
fn invalid_mqtt_settings_are_rejected_without_network_use() {
    for config in [
        Config {
            broker: "0.0.0.0:1883".parse().unwrap(),
            ..Default::default()
        },
        Config {
            broker: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        },
        Config {
            queue_capacity: 0,
            ..Default::default()
        },
        Config {
            queue_capacity: 16385,
            ..Default::default()
        },
        Config {
            interval_ms: 9,
            ..Default::default()
        },
        Config {
            interval_ms: 60001,
            ..Default::default()
        },
        Config {
            topic: "test/+".into(),
            ..Default::default()
        },
        Config {
            topic: "test/#".into(),
            ..Default::default()
        },
        Config {
            topic: "test\0value".into(),
            ..Default::default()
        },
        Config {
            topic: String::new(),
            ..Default::default()
        },
        Config {
            topic: "x".repeat(513),
            ..Default::default()
        },
    ] {
        assert!(config.validate().is_err());
    }
    Config {
        topic: "测量/µm".into(),
        ..Default::default()
    }
    .validate()
    .unwrap();
}

#[test]
fn latest_state_counts_intentional_coalescing_and_preserves_source_meaning() -> Result<()> {
    let mut latest = Latest::default();
    let mut summary = summary();
    let shared = shared(2);
    latest.ingest(event(1), &mut summary);
    latest.ingest(event(2), &mut summary);
    assert_eq!(summary.coalesced, 1);
    assert_eq!(summary.events_dequeued, 2);
    let json: serde_json::Value = serde_json::from_slice(&latest.payload(
        &profile(),
        &summary,
        &shared,
        "synthetic",
        Instant::now(),
    )?)?;
    assert_eq!(json["event_sequence"], 2);
    assert_eq!(json["input"]["raw_hex"], "01F4");
    assert_eq!(json["input"]["source_counter"], 2);
    let mut snapshot = sample(3);
    snapshot.freshness = Freshness::Snapshots;
    snapshot.input.counter = None;
    snapshot.input.source_time_us = None;
    snapshot.input.source_age_us = None;
    snapshot.source_gaps = None;
    snapshot.stale = None;
    snapshot.classification = "host_poll";
    latest.ingest(
        Event {
            sequence: 3,
            elapsed_us: 3000,
            generation: 1,
            kind: Kind::Input(snapshot),
        },
        &mut summary,
    );
    let json: serde_json::Value = serde_json::from_slice(&latest.payload(
        &profile(),
        &summary,
        &shared,
        "synthetic",
        Instant::now(),
    )?)?;
    assert_eq!(json["input"]["row_kind"], "host_poll");
    assert_eq!(json["input"]["source_update_loss"], "unknown");
    assert!(json["input"]["source_gaps_observed"].is_null());
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
    assert!(!latest.connected);
    Ok(())
}

#[test]
fn saturated_mqtt_queue_never_requires_a_consumer() {
    let service = Service {
        shared: shared(2),
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

#[test]
fn saved_mqtt_summary_rejects_mixed_loss_claims() -> Result<()> {
    let mut original = summary();
    original.events_offered = 10;
    original.events_dequeued = 8;
    original.queue_dropped = 2;
    original.coalesced = 5;
    original.publish_attempts = 2;
    original.shutdown_discarded = 1;
    original.socket_writes_completed = 1;
    original.publish_write_failures = 1;
    original.broker_connect_attempts = 3;
    original.broker_connections = 2;
    original.connect_failures = 1;
    original.connection_losses = 1;
    original.validate()?;
    for (key, value) in [
        ("queue_dropped", json!(3)),
        ("coalesced", json!(4)),
        ("shutdown_discarded", json!(2)),
        ("publish_write_failures", json!(2)),
        ("broker_connections", json!(4)),
        ("format_version", json!(2)),
        ("client_id", json!("bad-id")),
    ] {
        let mut bad = serde_json::to_value(&original)?;
        bad[key] = value;
        assert!(
            serde_json::from_value::<Summary>(bad)?.validate().is_err(),
            "{key}"
        );
    }
    Ok(())
}

#[test]
fn fragmented_connack_ping_and_clean_disconnect_work() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let broker = thread::spawn(move || -> Result<()> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let expected = wire::connect_packet("abc");
        let mut connect = vec![0; expected.len()];
        stream.read_exact(&mut connect)?;
        assert_eq!(connect, expected);
        stream.write_all(&[0x20, 2])?;
        thread::sleep(Duration::from_millis(10));
        stream.write_all(&[0, 0])?;
        let mut request = [0; 2];
        stream.read_exact(&mut request)?;
        assert_eq!(request, [0xC0, 0]);
        stream.write_all(&[0xD0])?;
        thread::sleep(Duration::from_millis(10));
        stream.write_all(&[0])?;
        stream.read_exact(&mut request)?;
        assert_eq!(request, [0xE0, 0]);
        Ok(())
    });
    let mut connection = wire::Connection::connect(address, "abc")?;
    wire::expire_keepalive(&mut connection);
    assert!(connection.maintain()?);
    thread::sleep(Duration::from_millis(30));
    assert!(!connection.maintain()?);
    assert!(connection.disconnect());
    broker.join().unwrap()?;
    Ok(())
}

#[test]
fn refused_or_malformed_connack_is_not_accepted() -> Result<()> {
    for reply in [
        [0x20, 2, 0, 5],
        [0x20, 2, 1, 0],
        [0x20, 3, 0, 0],
        [0x21, 2, 0, 0],
    ] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let broker = thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut data = vec![0; wire::connect_packet("abc").len()];
            stream.read_exact(&mut data)?;
            stream.write_all(&reply)?;
            Ok(())
        });
        assert!(wire::Connection::connect(address, "abc").is_err());
        broker.join().unwrap()?;
    }
    Ok(())
}

#[test]
fn ping_timeout_is_detected_even_while_publishing() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let broker = thread::spawn(move || -> Result<()> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut data = vec![0; wire::connect_packet("abc").len()];
        stream.read_exact(&mut data)?;
        stream.write_all(&[0x20, 2, 0, 0])?;
        let mut pending = [0; 2];
        stream.read_exact(&mut pending)?;
        assert_eq!(pending, [0xC0, 0]);
        let mut byte = [0; 1];
        let _ = stream.read(&mut byte);
        Ok(())
    });
    let mut connection = wire::Connection::connect(address, "abc")?;
    wire::expire_keepalive(&mut connection);
    assert!(connection.maintain()?);
    wire::expire_ping(&mut connection);
    assert!(connection.maintain().is_err());
    drop(connection);
    broker.join().unwrap()?;
    Ok(())
}
