use super::*;
use crate::desktop::{App, completed};
use iolink_workbench::capture::{self, Live, Options, Stop};
use std::sync::mpsc;

#[test]
fn failed_dispatch_preserves_previous_observations_report_queue_and_stop() -> Result<()> {
    let _serial = crate::desktop::WORKER_TEST_LOCK.lock().unwrap();
    let mut app = App::new();
    let stopped = Stop::default();
    stopped.request();
    app.report = Some(capture::run(
        &app.profile,
        &Options::default(),
        &stopped,
        None,
    )?);
    let before_report = toml::to_string(app.report.as_ref().unwrap())?;
    let before_profile = app.profile.clone();
    app.live.counts.received = 42;
    app.live.message = "retained observation".into();
    app.message = "previous run".into();
    app.stop.request();
    let old_stop = app.stop.clone();
    let old_queue = app.queue.clone();
    old_queue.force_push(Live {
        message: "queued observation".into(),
        ..Default::default()
    });
    std::fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let path = scratch.path().join("must-not-create");
    app.capture_path = path.to_string_lossy().into_owned();
    let _guard = Guard::enter()?;
    let error = app.start(true).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("background workers are disabled")
    );
    assert!(app.stop.requested() && old_stop.requested());
    assert!(Arc::ptr_eq(&old_queue, &app.queue));
    assert_eq!(app.live.counts.received, 42);
    assert_eq!(app.live.message, "retained observation");
    assert_eq!(app.message, "previous run");
    assert_eq!(
        toml::to_string(app.report.as_ref().unwrap())?,
        before_report
    );
    assert_eq!(app.profile, before_profile);
    assert!(app.worker.is_none() && !path.exists());
    assert_eq!(app.queue.pop().unwrap().message, "queued observation");
    assert_eq!(BACKGROUND_ATTEMPTS.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn second_start_cannot_replace_a_pending_or_completed_unconsumed_owner() -> Result<()> {
    let _serial = crate::desktop::WORKER_TEST_LOCK.lock().unwrap();
    let _guard = Guard::enter()?;
    for ready in [false, true] {
        let mut app = App::new();
        let (sender, receiver) = mpsc::channel();
        app.worker = Some(receiver);
        app.stop.request();
        app.live.counts.received = 17;
        let old_stop = app.stop.clone();
        let old_queue = app.queue.clone();
        if ready {
            sender.send(Err(anyhow::anyhow!("held completion")))?;
        }
        for record in [false, true] {
            let error = app.start(record).unwrap_err();
            assert!(error.to_string().contains("already active"));
            assert!(app.worker.is_some() && app.stop.requested() && old_stop.requested());
            assert!(Arc::ptr_eq(&old_queue, &app.queue));
            assert_eq!(app.live.counts.received, 17);
        }
        if !ready {
            sender.send(Err(anyhow::anyhow!("held completion")))?;
        }
        assert!(
            completed(&mut app.worker)
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("held completion")
        );
    }
    assert_eq!(BACKGROUND_ATTEMPTS.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn pending_profile_edits_and_inspection_block_start_before_dispatch_or_state_reset() -> Result<()> {
    let _serial = crate::desktop::WORKER_TEST_LOCK.lock().unwrap();
    let _guard = Guard::enter()?;
    let mut app = App::new();
    app.stop.request();
    app.live.counts.received = 29;
    app.setup.mark_pending_for_test();
    assert!(
        app.start_block_reason()
            .unwrap()
            .contains("pending profile edits")
    );
    for record in [false, true] {
        assert!(
            app.start(record)
                .unwrap_err()
                .to_string()
                .contains("pending profile edits")
        );
        assert!(app.stop.requested() && app.setup.pending());
        assert_eq!(app.live.counts.received, 29);
    }
    app.setup.sync(&app.profile);
    let release = app.setup.gate_for_test();
    assert!(
        app.start_block_reason()
            .unwrap()
            .contains("IODD inspection")
    );
    for record in [false, true] {
        assert!(
            app.start(record)
                .unwrap_err()
                .to_string()
                .contains("IODD inspection")
        );
        assert!(app.stop.requested() && app.setup.busy());
        assert_eq!(app.live.counts.received, 29);
    }
    release();
    app.setup
        .update(&eframe::egui::Context::default(), &mut app.message);
    assert!(app.start_block_reason().is_none());
    assert_eq!(BACKGROUND_ATTEMPTS.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn consecutive_simulator_captures_keep_independent_owners_while_other_jobs_are_pending()
-> Result<()> {
    use std::{fs, thread, time::Duration};
    let _serial = crate::desktop::WORKER_TEST_LOCK.lock().unwrap();
    fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let mut app = App::new();
    app.duration = 0;
    app.profile.logging.queue_capacity = 8;
    app.profile.logging.rotate_rows = 4;
    let (acyclic_sender, acyclic_receiver) = mpsc::channel();
    app.isdu_worker = Some(acyclic_receiver);
    let mut releases = app.saved_capture.gates_for_test();
    releases.push(app.saved_parameter.gate_for_test());
    assert_eq!(app.pending_jobs(), 4);
    let ctx = eframe::egui::Context::default();
    let mut first_files = Vec::new();
    for index in 0..2 {
        assert!(app.start_block_reason().is_none());
        let old_stop = app.stop.clone();
        old_stop.request();
        let old_queue = app.queue.clone();
        let path = scratch.path().join(format!("capture-{index}"));
        app.capture_path = path.to_string_lossy().into_owned();
        app.start(true)?;
        assert!(!app.stop.requested() && old_stop.requested());
        assert!(!Arc::ptr_eq(&old_queue, &app.queue));
        assert!(app.report.is_none() && app.live.counts.received == 0);
        old_queue.force_push(Live {
            message: "previous session only".into(),
            ..Default::default()
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.live.counts.received == 0 && app.worker.is_some() && Instant::now() < deadline {
            app.update_workers(&ctx);
            thread::sleep(Duration::from_millis(2));
        }
        ensure!(
            app.live.counts.received > 0,
            "simulator did not produce an observation"
        );
        assert_ne!(app.live.message, "previous session only");
        app.stop.request();
        while app.worker.is_some() && Instant::now() < deadline {
            app.update_workers(&ctx);
            thread::sleep(Duration::from_millis(2));
        }
        ensure!(app.worker.is_none(), "simulator did not stop and flush");
        let report = app.report.as_ref().context("capture result missing")?;
        assert!(report.success() && report.stopped_by_request);
        let checked = iolink_workbench::archive::verify(&path, None)?;
        assert_eq!(checked.first_sample_index, Some(1));
        assert_eq!(checked.rows_checked, report.csv_rows_flushed);
        assert!(checked.rows_checked > 0);
        assert_eq!(app.pending_jobs(), 4);
        let before_report = toml::to_string(report)?;
        let before_live = app.live.counts.received;
        assert!(
            app.start(true)
                .unwrap_err()
                .to_string()
                .contains("already exists")
        );
        assert_eq!(
            toml::to_string(app.report.as_ref().unwrap())?,
            before_report
        );
        assert_eq!(app.live.counts.received, before_live);
        if index == 0 {
            first_files = fs::read_dir(&path)?
                .map(|entry| {
                    let path = entry?.path();
                    Ok((path.clone(), fs::read(&path)?))
                })
                .collect::<std::io::Result<Vec<_>>>()?;
        }
    }
    for (path, before) in first_files {
        assert_eq!(fs::read(path)?, before);
    }
    for release in releases {
        release();
    }
    acyclic_sender.send(Ok("held acyclic result".into()))?;
    app.update_workers(&ctx);
    assert_eq!(app.pending_jobs(), 0);
    assert_eq!(app.isdu_result, "held acyclic result");
    Ok(())
}
