use super::*;
use iolink_workbench::transport::{Input, Payload, Status};

fn live(at: Instant, freshness: Freshness) -> Live {
    Live {
        connected: true,
        input: Some(Input {
            received: at,
            payload: Payload::new(&[0x03, 0xe9]).unwrap(),
            validity: Validity::Valid,
            counter: Some(1),
            source_time_us: Some(1000),
            source_age_us: Some(0),
            diagnostic_code: None,
        }),
        last_new_at: Some(at),
        stale: Some(false),
        status: Some(Status {
            route: "synthetic GUI observation",
            model: "synthetic".into(),
            firmware: "test".into(),
            master_serial: None,
            port: 1,
            port_mode: "IO-Link".into(),
            device_status: None,
            port_event: None,
            device_event: None,
            port_cycle_raw: None,
            port_cycle_us: None,
            identity: Default::default(),
            freshness,
        }),
        ..Default::default()
    }
}

#[test]
fn held_ages_advance_without_rewriting_receipt_metadata() {
    let at = Instant::now();
    let mut live = live(at, Freshness::Counter32);
    live.last_new_at = Some(at - Duration::from_millis(40));
    live.input.as_mut().unwrap().source_age_us = Some(1200);
    let settings = Connection::default();
    let before = Observation::at(&live, true, at, &settings).unwrap();
    let later = Observation::at(&live, true, at + Duration::from_secs(2), &settings).unwrap();
    assert_eq!(
        later.receive_age - before.receive_age,
        Duration::from_secs(2)
    );
    assert_eq!(
        later.observed_update_age.unwrap() - before.observed_update_age.unwrap(),
        Duration::from_secs(2)
    );
    assert_eq!(later.source_age_at_receipt_us, Some(1200));
    assert_eq!(later.state, State::HostSilence);
    assert!(!later.show_typed());
    assert_eq!(live.input.unwrap().received, at);
    assert_eq!(live.input.unwrap().source_age_us, Some(1200));
    assert_eq!(live.stale, Some(false));
    assert_eq!(live.counts.stale, 0);
}

#[test]
fn gui_handoff_budget_prevents_false_silence_and_has_a_strict_boundary() {
    let at = Instant::now();
    let live = live(at, Freshness::Counter32);
    let settings = Connection::default();
    let initial = Observation::at(&live, true, at, &settings).unwrap();
    assert_eq!(initial.display_limit, Duration::from_millis(165));
    for delay in [Duration::from_millis(80), initial.display_limit] {
        assert!(
            Observation::at(&live, true, at + delay, &settings)
                .unwrap()
                .show_typed()
        );
    }
    assert_eq!(
        Observation::at(
            &live,
            true,
            at + initial.display_limit + Duration::from_nanos(1),
            &settings
        )
        .unwrap()
        .state,
        State::HostSilence
    );
    let slower = Connection {
        poll_us: 1_000_000,
        ..settings
    };
    assert_eq!(
        Observation::at(&live, true, at, &slower)
            .unwrap()
            .display_limit,
        Duration::from_millis(1160)
    );
}

#[test]
fn receipt_quality_and_session_state_gate_typed_values_immediately() {
    let at = Instant::now();
    let mut live = live(at, Freshness::Timestamp);
    let settings = Connection::default();
    for (validity, stale, expected) in [
        (Validity::Valid, Some(false), State::Recent),
        (Validity::Valid, None, State::Recent),
        (Validity::Valid, Some(true), State::StaleAtReceipt),
        (Validity::Invalid, Some(false), State::InvalidAtReceipt),
        (Validity::Unknown, None, State::UnknownValidity),
    ] {
        live.input.as_mut().unwrap().validity = validity;
        live.stale = stale;
        let view = Observation::at(&live, true, at, &settings).unwrap();
        assert_eq!(view.state, expected);
        assert_eq!(view.show_typed(), expected == State::Recent);
    }
    assert_eq!(
        Observation::at(&live, false, at, &settings).unwrap().state,
        State::Stopped
    );
    live.connected = false;
    assert_eq!(
        Observation::at(&live, true, at, &settings).unwrap().state,
        State::Disconnected
    );
}

#[test]
fn recent_receipts_do_not_reset_the_last_observed_source_update_age() {
    let now = Instant::now();
    for freshness in [Freshness::Counter32, Freshness::Timestamp] {
        // At receipt the update was only 2 ms old (below stale_us=5 ms).
        // The GUI now holds it past the 165 ms display budget while its
        // receive age remains just inside that budget.
        let mut live = live(now - Duration::from_millis(164), freshness);
        live.last_new_at = Some(now - Duration::from_millis(166));
        let view = Observation::at(&live, true, now, &Connection::default()).unwrap();
        assert_eq!(view.receive_age, Duration::from_millis(164));
        assert_eq!(view.observed_update_age, Some(Duration::from_millis(166)));
        assert_eq!(view.state, State::UpdateOld);
        assert!(!view.show_typed());
        // The displayed hold does not retrospectively reclassify a CSV row.
        assert_eq!(live.stale, Some(false));
        assert_eq!(live.input.unwrap().source_age_us, Some(0));
    }
}

#[test]
fn snapshot_source_freshness_stays_unknown_and_missing_input_stays_absent() {
    let at = Instant::now();
    let mut live = live(at, Freshness::Snapshots);
    live.input.as_mut().unwrap().source_age_us = None;
    live.stale = None;
    // Even a stray host update timer cannot establish snapshot source cadence.
    live.last_new_at = Some(at - Duration::from_secs(10));
    let settings = Connection::default();
    let view = Observation::at(&live, true, at, &settings).unwrap();
    assert!(view.show_typed());
    assert_eq!(view.observed_update_age, None);
    assert_eq!(view.source_age_at_receipt_us, None);
    assert_eq!(
        Observation::at(&live, true, at + Duration::from_secs(1), &settings)
            .unwrap()
            .state,
        State::HostSilence
    );
    assert_eq!(live.stale, None);
    live.input = None;
    assert_eq!(Observation::at(&live, true, at, &settings), None);
}
