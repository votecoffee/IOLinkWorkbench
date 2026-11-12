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

#[test]
fn frozen_counter_overrides_a_gateway_age_of_zero() {
    let now = Instant::now();
    let state = FreshnessState {
        last_new: Some(now - Duration::from_millis(20)),
        ..FreshnessState::default()
    };
    let mut sample = input(Some(1), None);
    sample.received = now;
    sample.source_age_us = Some(0);
    assert_eq!(state.stale(&sample, 5000), Some(true));
    sample.source_age_us = Some(50_000);
    assert_eq!(state.stale(&sample, 30_000), Some(true));
    assert_eq!(
        FreshnessState::default().stale(&sample, 60_000),
        Some(false)
    );
    sample.source_age_us = None;
    assert_eq!(FreshnessState::default().stale(&sample, 5000), None);
}

struct SnapshotFixture {
    simulator: Box<dyn InputTransport>,
    samples: u32,
    rotation_obstacle: Option<PathBuf>,
}

impl InputTransport for SnapshotFixture {
    fn connect(&mut self) -> Result<Status> {
        if let Some(path) = &self.rotation_obstacle {
            fs::write(path, "preserve this existing file")?;
        }
        let mut status = self.simulator.connect()?;
        status.freshness = Freshness::Snapshots;
        Ok(status)
    }

    fn poll(&mut self) -> Result<Input> {
        self.samples += 1;
        let mut sample = input(None, None);
        sample.payload = Payload::new(&[0, self.samples.min(10) as u8])?;
        Ok(sample)
    }
}

fn fixture(profile: &Profile, obstacle: Option<PathBuf>) -> Result<Box<dyn InputTransport>> {
    Ok(Box::new(SnapshotFixture {
        simulator: transport::open(profile, false)?,
        samples: 0,
        rotation_obstacle: obstacle,
    }))
}

#[test]
fn on_change_retries_a_dropped_value_until_it_is_queued() -> Result<()> {
    fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let mut profile = Profile::parse(include_str!("../../samples/simulator.toml"))?;
    profile.logging.mode = LogMode::OnChange;
    profile.logging.queue_capacity = 1;
    let output = scratch.path().join("capture");
    let report = run_with_transport(
        &profile,
        &Options {
            output: Some(output.clone()),
            duration: Some(Duration::from_millis(400)),
            simulated_writer_delay: Duration::from_millis(80),
            ..Options::default()
        },
        &Stop::default(),
        None,
        fixture(&profile, None)?,
    )?;
    assert!(report.success(), "{report:?}");
    assert!(report.counts.logger_overruns > 0);
    let mut csv = csv::Reader::from_path(output.join("capture-0001.csv"))?;
    let rows = csv.records().collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(rows.last().unwrap().get(11), Some("000A"));
    assert_eq!(report.csv_rows_unconfirmed, 0);
    assert_eq!(
        report.counts.selected,
        report.csv_rows_flushed + report.counts.logger_overruns
    );
    Ok(())
}

#[test]
fn rotation_failure_stops_capture_and_reports_unconfirmed_rows() -> Result<()> {
    fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let output = scratch.path().join("capture");
    let obstacle = output.join("capture-0002.csv");
    let mut profile = Profile::parse(include_str!("../../samples/simulator.toml"))?;
    profile.logging.rotate_rows = 1;
    let report = run_with_transport(
        &profile,
        &Options {
            output: Some(output.clone()),
            duration: Some(Duration::from_secs(3)),
            ..Options::default()
        },
        &Stop::default(),
        None,
        fixture(&profile, Some(obstacle.clone()))?,
    )?;
    assert!(!report.success());
    assert!(report.writer_error.is_some());
    assert!(report.elapsed_us < 2_000_000);
    assert_eq!(report.csv_rows_flushed, 1);
    assert!(report.csv_rows_unconfirmed > 0);
    assert_eq!(
        report.counts.selected,
        report.csv_rows_flushed + report.csv_rows_unconfirmed + report.counts.logger_overruns
    );
    assert_eq!(fs::read_to_string(obstacle)?, "preserve this existing file");
    assert!(output.join("capture.toml").is_file());
    Ok(())
}
