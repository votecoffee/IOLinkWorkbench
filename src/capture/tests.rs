use super::*;
use crate::transport::Payload;

fn input(counter: Option<u32>, time: Option<u64>) -> Input {
    Input {
        received: Instant::now(),
        payload: Payload::new(&[0]).unwrap(),
        validity: Validity::Valid,
        counter,
        source_time_us: time,
        source_age_us: None,
        diagnostic_code: None,
    }
}

#[test]
fn counter_wrap_repeats_and_late_packets() -> Result<()> {
    let state = FreshnessState {
        counter: Some(u32::MAX - 1),
        ..FreshnessState::default()
    };
    assert_eq!(
        state.classify(Freshness::Counter32, &input(Some(u32::MAX - 1), None))?,
        Update::Same
    );
    assert_eq!(
        state.classify(Freshness::Counter32, &input(Some(1), None))?,
        Update::Forward(3)
    );
    assert_eq!(
        state.classify(Freshness::Counter32, &input(Some(u32::MAX - 2), None))?,
        Update::Backward
    );
    assert!(
        state
            .classify(Freshness::Counter32, &input(None, None))
            .is_err()
    );
    Ok(())
}

#[test]
fn timestamp_freshness_is_independent_of_payload() -> Result<()> {
    let state = FreshnessState {
        source_time_us: Some(500),
        ..FreshnessState::default()
    };
    assert_eq!(
        state.classify(Freshness::Timestamp, &input(None, Some(500)))?,
        Update::Same
    );
    assert_eq!(
        state.classify(Freshness::Timestamp, &input(None, Some(600)))?,
        Update::Forward(1)
    );
    assert_eq!(
        state.classify(Freshness::Timestamp, &input(None, Some(200)))?,
        Update::Backward
    );
    assert_eq!(
        state.classify(Freshness::Snapshots, &input(None, None))?,
        Update::Snapshot
    );
    Ok(())
}
