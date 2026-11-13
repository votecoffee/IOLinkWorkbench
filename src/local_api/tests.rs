use super::*;
use crate::{
    capture::Counts,
    transport::{Freshness, Input, Payload, Validity},
};
use std::{
    io::{Read, Write},
    net::TcpStream,
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
        requests: AtomicU64::new(0),
        failed_requests: AtomicU64::new(0),
    })
}
fn store(capacity: usize) -> Arc<Mutex<Store>> {
    Arc::new(Mutex::new(Store {
        history: VecDeque::with_capacity(capacity),
        capacity,
        ingested: 0,
        evicted: 0,
        master: None,
    }))
}
fn server(capacity: usize) -> http::Server {
    http::Server {
        shared: shared(2),
        store: store(capacity),
        schema: json!({"test":true}),
        profile: profile(),
        session_id: "test-session".into(),
        utc_start: "2026-11-13T00:00:00Z".into(),
        start: Instant::now(),
        authority: "127.0.0.1:8765".into(),
        queue_capacity: 2,
        history_capacity: capacity,
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
        body: Body::Sample(sample(sequence as u32)),
    }
}
fn get(server: &http::Server, target: &str) -> (u16, Value) {
    server.route(&format!(
        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1:8765\r\n\r\n"
    ))
}

#[test]
fn loopback_configuration_and_schema_are_bounded() {
    for config in [
        Config {
            listen: "0.0.0.0:0".parse().unwrap(),
            ..Default::default()
        },
        Config {
            listen: "[::1]:0".parse().unwrap(),
            ..Default::default()
        },
        Config {
            queue_capacity: 0,
            ..Default::default()
        },
        Config {
            history_capacity: 16385,
            ..Default::default()
        },
    ] {
        assert!(config.validate().is_err());
    }
    let mut config = Config::default();
    config.listen.set_port(0);
    let mut profile = profile();
    profile.fields[0].name = "x".repeat(257);
    assert!(Service::start(&config, &profile, Instant::now(), "test").is_err());
}

#[test]
fn saved_api_counters_reject_false_drain_and_history_claims() -> Result<()> {
    let valid = Summary {
        format_version: 1,
        url: "http://127.0.0.1:8765".into(),
        session_id: "session".into(),
        queue_capacity: 2,
        history_capacity: 8,
        events_offered: 12,
        events_ingested: 10,
        queue_dropped: 2,
        history_evicted: 2,
        requests: 2,
        failed_requests: 1,
        worker_failed: false,
    };
    valid.validate()?;
    for (key, value) in [
        ("queue_dropped", json!(3)),
        ("events_ingested", json!(9)),
        ("history_evicted", json!(1)),
        ("queue_capacity", json!(0)),
        ("history_capacity", json!(16385)),
        ("failed_requests", json!(3)),
        ("format_version", json!(2)),
        ("url", json!("http://0.0.0.0:8765")),
        ("session_id", json!("")),
    ] {
        let mut changed = serde_json::to_value(&valid)?;
        changed[key] = value;
        assert!(
            serde_json::from_value::<Summary>(changed)?
                .validate()
                .is_err(),
            "{key}"
        );
    }
    Ok(())
}

#[test]
fn requests_are_read_only_and_session_cursors_are_explicit() {
    let server = server(2);
    assert_eq!(get(&server, "/v1/schema").0, 200);
    assert_eq!(get(&server, "/v1/status").0, 200);
    assert_eq!(get(&server, "/unknown").0, 404);
    for target in [
        "/v1/history?after=0",
        "/v1/history?session=test-session",
        "/v1/history?after=1&session=test-session",
        "/v1/history?limit=0",
        "/v1/history?limit=129",
        "/v1/history?limit=1&limit=2",
        "/v1/history?after=-1&session=test-session",
        "/v1/history?write=1",
        "/v1/schema?query=1",
    ] {
        assert_eq!(get(&server, target).0, 400, "{target}");
    }
    assert_eq!(
        get(&server, "/v1/history?after=0&session=old-session").0,
        409
    );
    for (headers, code) in [
        (
            "POST /v1/status HTTP/1.1\r\nHost: 127.0.0.1:8765\r\n\r\n",
            405,
        ),
        (
            "GET /v1/status HTTP/1.1\r\nHost: attacker.example\r\n\r\n",
            403,
        ),
        (
            "GET /v1/status HTTP/1.1\r\nHost: 127.0.0.1:8765\r\nOrigin: http://example.com\r\n\r\n",
            403,
        ),
        (
            "GET /v1/status HTTP/1.1\r\nHost: 127.0.0.1:8765\r\nTransfer-Encoding: chunked\r\n\r\n",
            400,
        ),
        (
            "GET /v1/status HTTP/1.1\r\nHost: 127.0.0.1:8765\r\nHost: 127.0.0.1:8765\r\n\r\n",
            400,
        ),
    ] {
        assert_eq!(server.route(headers).0, code);
    }
}

#[test]
fn bounded_history_pages_expose_eviction_and_equal_payload_updates() {
    let server = server(2);
    server.shared.offered.store(4, Ordering::Relaxed);
    for sequence in 1..=4 {
        server.store.lock().unwrap().ingest(event(sequence));
    }
    let (code, page) = get(&server, "/v1/history?limit=1");
    assert_eq!(code, 200);
    assert_eq!(page["body"]["metrics"]["history_evicted"], 2);
    assert_eq!(page["body"]["missed_before_oldest"], 2);
    assert_eq!(page["body"]["next_after"], 3);
    assert_eq!(page["body"]["more_available"], true);
    let (_, next) = get(&server, "/v1/history?after=3&session=test-session");
    assert_eq!(next["body"]["events"][0]["sequence"], 4);
    let first = &page["body"]["events"][0]["data"];
    let second = &next["body"]["events"][0]["data"];
    assert_eq!(first["raw_hex"], second["raw_hex"]);
    assert_ne!(first["source_counter"], second["source_counter"]);
    assert!(first["csv_sample_index"].is_null());
    assert!(first["values"].is_array());
}

#[test]
fn invalid_stale_and_snapshot_meanings_survive_api_rendering() {
    for invalid in [true, false] {
        let mut event = event(1);
        if let Body::Sample(sample) = &mut event.body {
            if invalid {
                sample.input.validity = Validity::Invalid;
            } else {
                sample.stale = Some(true);
            }
        }
        let rendered = event_json(&event, &profile());
        assert_eq!(rendered["data"]["raw_hex"], "01F4");
        assert!(rendered["data"]["values"].is_null());
    }
    let mut event = event(1);
    if let Body::Sample(sample) = &mut event.body {
        sample.freshness = Freshness::Snapshots;
        sample.source_gaps = None;
        sample.input.counter = None;
        sample.input.source_time_us = None;
        sample.input.source_age_us = None;
        sample.stale = None;
        sample.classification = "host_poll";
    }
    let rendered = event_json(&event, &profile());
    assert_eq!(rendered["data"]["row_kind"], "host_poll");
    assert_eq!(rendered["data"]["source_update_loss"], "unknown");
    assert!(rendered["data"]["source_gaps_observed"].is_null());
}

#[test]
fn saturated_handoff_counts_drops_without_waiting_for_a_consumer() {
    let service = Service {
        shared: shared(2),
        store: store(2),
        ingest: None,
        http: None,
        config: Config::default(),
        url: String::new(),
        session_id: String::new(),
        start: Instant::now(),
    };
    for count in 1..=10000 {
        service.sample(1, sample(count));
    }
    assert_eq!(service.shared.offered.load(Ordering::Relaxed), 10000);
    assert_eq!(service.shared.dropped.load(Ordering::Relaxed), 9998);
    assert_eq!(service.shared.queue.len(), 2);
}

#[test]
fn response_size_pagination_keeps_the_next_event_and_utf8_status_bounds() {
    let mut server = server(256);
    server.profile.logging.fields.clear();
    server.profile.fields = (0..128)
        .map(|index| crate::decode::Field {
            name: format!("{index:03}{}", "n".repeat(250)),
            unit: "u".repeat(256),
            bits: 1,
            ..Default::default()
        })
        .collect();
    server.shared.offered.store(128, Ordering::Relaxed);
    for sequence in 1..=128 {
        server.store.lock().unwrap().ingest(event(sequence));
    }
    let (code, page) = get(&server, "/v1/history?limit=128");
    assert_eq!(code, 200);
    assert_eq!(page["body"]["size_limited"], true);
    assert!(serde_json::to_vec(&page).unwrap().len() <= MAX_REPLY);
    let cursor = page["body"]["next_after"].as_u64().unwrap();
    assert!((1..128).contains(&cursor));
    let (_, next) = get(
        &server,
        &format!("/v1/history?after={cursor}&session=test-session&limit=1"),
    );
    assert_eq!(next["body"]["events"][0]["sequence"], cursor + 1);
    let mut text = "µ".repeat(200);
    assert!(truncate(&mut text));
    assert_eq!(text.len(), 256);
    assert_eq!(text.chars().count(), 128);
}

#[test]
fn slow_http_headers_do_not_block_ingestion_and_shutdown_drains() -> Result<()> {
    let mut config = Config::default();
    config.listen.set_port(0);
    let service = Service::start(
        &config,
        &profile(),
        Instant::now(),
        "2026-11-13T00:00:00.000Z",
    )?;
    assert!(
        service
            .session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    );
    let address = service.url.trim_start_matches("http://").to_owned();
    let mut slow = TcpStream::connect(&address)?;
    thread::sleep(Duration::from_millis(20));
    slow.write_all(b"G")?;
    for count in 1..=100 {
        service.sample(1, sample(count));
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while service.store.lock().unwrap().ingested < 100 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(service.store.lock().unwrap().ingested, 100);
    let mut client = TcpStream::connect(&address)?;
    client.set_read_timeout(Some(Duration::from_secs(2)))?;
    client.write_all(format!("GET /v1/status HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes())?;
    let mut response = String::new();
    client.read_to_string(&mut response)?;
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("no-store"));
    service.ended(1);
    let summary = service.finish();
    assert_eq!(
        summary.events_ingested + summary.queue_dropped,
        summary.events_offered
    );
    assert!(!summary.worker_failed);
    assert!(TcpStream::connect(&address).is_err());
    Ok(())
}
