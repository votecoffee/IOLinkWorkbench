use super::*;
use iolink_workbench::archive;
use std::{fs, time::Instant};

fn pump_until(app: &mut App, predicate: impl Fn(&App) -> bool) -> Result<()> {
    let ctx = Context::default();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !predicate(app) {
        ensure!(
            Instant::now() < deadline,
            "port capture controller timed out: {}",
            app.message
        );
        app.update_workers(&ctx);
        thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}

fn set_profiles(app: &mut App, folder: &Path, mismatch: bool) -> Result<Vec<PathBuf>> {
    let paths = [folder.join("port-one.toml"), folder.join("port-two.toml")];
    for (index, path) in paths.iter().enumerate() {
        let mut profile = app.profile.clone();
        profile.name = format!("Synthetic port {}", index + 1);
        profile.connection.port = (index + 1) as u8;
        profile.logging.rotate_rows = 4;
        if index == 1 && mismatch {
            profile.expected_identity.device_id = Some(99);
        }
        profile.save(path)?;
    }
    app.port_capture.profile_paths = paths
        .iter()
        .map(|path| path.to_str().unwrap().into())
        .collect();
    app.port_capture.output = folder.join("set").to_str().unwrap().into();
    app.port_capture.seconds = 30;
    Ok(Vec::from(paths))
}

#[test]
fn reviewed_profiles_run_independently_and_flush_when_close_is_requested_off_page() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let scratch = tempfile::tempdir()?;
    let mut app = App::new();
    let applied = app.profile.clone();
    let sources = set_profiles(&mut app, scratch.path(), false)?;
    assert!(
        app.start_port_capture()
            .unwrap_err()
            .to_string()
            .contains("Review")
    );
    let output = PathBuf::from(&app.port_capture.output);
    app.port_capture.review_profiles()?;
    assert_eq!(app.pending_jobs(), 1);
    app.tab = Tab::Parameters;
    pump_until(&mut app, |app| app.pending_jobs() == 0)?;
    assert!(!output.exists());
    assert_eq!(
        app.port_capture.reviewed.as_ref().unwrap().profiles.len(),
        2
    );
    let mut changed = Profile::load(&sources[1])?;
    changed.name = "Later file edit".into();
    changed.save(&sources[1])?;
    let pinned = sources
        .iter()
        .map(fs::read)
        .collect::<std::io::Result<Vec<_>>>()?;
    app.start_port_capture()?;
    assert!(app.acquisition_active() && app.worker.is_none());
    let current_stop = app.stop.clone();
    assert!(app.start(false).is_err() && app.start_port_capture().is_err());
    assert!(!current_stop.requested());
    pump_until(&mut app, |app| {
        app.port_capture.observations[..2]
            .iter()
            .all(|live| live.counts.received > 10 && live.connected)
    })?;
    assert_eq!(app.advance_close(true), shutdown::CloseAction::Wait);
    assert!(current_stop.requested());
    pump_until(&mut app, |app| app.pending_jobs() == 0)?;
    assert!(app.closing);
    let finished = app
        .port_capture
        .finished
        .as_ref()
        .context("missing port capture outcome")?;
    let batch = finished.result.as_ref().unwrap();
    assert!(batch.all_successful() && batch.stopped_by_request);
    assert_eq!(batch.ports[1].profile_name, "Synthetic port 2");
    assert_eq!(app.profile, applied);
    for port in &batch.ports {
        let checked = archive::verify(&output.join(&port.directory), None)?;
        assert_eq!(Some(checked.rows_checked), port.csv_rows_flushed);
        assert!(checked.rows_checked > 4 && checked.csv_parts > 1);
    }
    assert_eq!(
        pinned,
        sources
            .iter()
            .map(fs::read)
            .collect::<std::io::Result<Vec<_>>>()?
    );
    assert!(app.start_port_capture().is_err());
    Ok(())
}

#[test]
fn partial_capture_keeps_healthy_rows_and_cancels_close_on_the_batch_page() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let scratch = tempfile::tempdir()?;
    let mut app = App::new();
    set_profiles(&mut app, scratch.path(), true)?;
    let output = PathBuf::from(&app.port_capture.output);
    app.port_capture.review_profiles()?;
    pump_until(&mut app, |app| app.pending_jobs() == 0)?;
    app.start_port_capture()?;
    app.tab = Tab::Inputs;
    pump_until(&mut app, |app| {
        app.port_capture.observations[0].counts.received > 10
            && app.port_capture.observations[1]
                .message
                .contains("identity mismatch")
    })?;
    assert_eq!(app.advance_close(true), shutdown::CloseAction::Wait);
    pump_until(&mut app, |app| app.pending_jobs() == 0)?;
    assert!(!app.closing);
    assert_eq!(app.tab, Tab::Logging);
    assert_eq!(app.workspace.logging, workspace::LoggingPage::Ports);
    let batch = app
        .port_capture
        .finished
        .as_ref()
        .unwrap()
        .result
        .as_ref()
        .unwrap();
    assert!(!batch.all_successful() && batch.stopped_by_request);
    assert!(batch.ports[0].success && !batch.ports[1].success);
    assert!(
        batch.ports[1]
            .error
            .as_ref()
            .unwrap()
            .contains("identity mismatch")
    );
    assert_eq!(
        Some(archive::verify(&output.join("port-1"), None)?.rows_checked),
        batch.ports[0].csv_rows_flushed
    );
    assert!(output.join("port-2/capture.toml").is_file());
    Ok(())
}

#[test]
fn read_status_keeps_partial_results_and_accepts_an_acyclic_profile() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let scratch = tempfile::tempdir()?;
    let mut app = App::new();
    let applied = app.profile.clone();
    let sources = set_profiles(&mut app, scratch.path(), true)?;
    let mut acyclic = Profile::load(&sources[0])?;
    let xml = include_str!("../../../samples/sensor.xml").replace(
        "<ProcessDataIn id=\"IN\" bitLength=\"16\"><DatatypeRef datatypeId=\"PD\"/></ProcessDataIn>",
        "",
    );
    let catalog = iodd::parse(&xml, "synthetic acyclic".into())?;
    let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    acyclic.apply_iodd(identity, imported);
    acyclic.save(&sources[0])?;
    let pinned = fs::read(&sources[0])?;
    let output = PathBuf::from(&app.port_capture.output);
    assert!(app.read_port_status().is_err());
    app.port_capture.review_profiles()?;
    pump_until(&mut app, |app| app.pending_jobs() == 0)?;
    assert!(
        app.start_port_capture()
            .unwrap_err()
            .to_string()
            .contains("acyclic-only")
    );
    assert!(app.port_capture.status_result.is_none());
    app.read_port_status()?;
    assert!(app.read_port_status().is_err());
    assert!(!app.acquisition_active());
    app.tab = Tab::Offline;
    assert_eq!(app.advance_close(true), shutdown::CloseAction::Wait);
    pump_until(&mut app, |app| app.pending_jobs() == 0)?;
    let report = app
        .port_capture
        .status_result
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert!(!report.all_successful() && !report.observed_master_conflict);
    assert_eq!(
        report
            .ports
            .iter()
            .map(|port| port.port)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert!(report.ports[0].status.is_some() && report.ports[0].error.is_none());
    assert!(report.ports[1].status.is_some());
    assert!(
        report.ports[1]
            .error
            .as_deref()
            .unwrap()
            .contains("identity mismatch")
    );
    assert!(app.read_port_status().is_err());
    assert_eq!(app.tab, Tab::Offline);
    assert_eq!(app.profile, applied);
    assert_eq!(fs::read(&sources[0])?, pinned);
    assert!(!output.exists());
    app.port_capture.clear_review();
    assert!(app.port_capture.status_result.is_none());
    Ok(())
}
