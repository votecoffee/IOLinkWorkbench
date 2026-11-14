use super::*;
use crate::desktop::{self, Tab, TestCompletion};
use eframe::egui::{self, Context, ViewportCommand};
use iolink_workbench::archive;
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn frame(app: &mut App, ctx: &Context) -> egui::FullOutput {
    let input = egui::RawInput::default();
    assert!(input.events.is_empty());
    ctx.run(input, |ctx| app.render(ctx))
}

fn command(output: &egui::FullOutput, close: bool) -> bool {
    output.viewport_output.values().any(|v| {
        v.commands.iter().any(|c| {
            if close {
                matches!(c, ViewportCommand::Close)
            } else {
                matches!(c, ViewportCommand::CancelClose)
            }
        })
    })
}

#[test]
fn idle_close_and_pending_close_are_distinct_without_input_injection() {
    let mut app = App::new();
    let before = app.profile.clone();
    assert_eq!(app.advance_close(false), CloseAction::Idle);
    assert!(!app.stop.requested());
    assert_eq!(app.advance_close(true), CloseAction::Close);
    assert!(app.closing && app.stop.requested());
    assert!(command(&frame(&mut app, &Context::default()), true));
    assert_eq!(app.profile, before);
}

#[test]
fn every_worker_domain_delays_close_and_completes_with_its_tab_hidden() {
    let mut app = App::new();
    app.tab = Tab::Monitor;
    let before = app.profile.clone();
    let (capture_sender, capture_receiver) = mpsc::channel();
    app.worker = Some(capture_receiver);
    let (isdu_sender, isdu_receiver) = mpsc::channel();
    app.isdu_worker = Some(isdu_receiver);
    let stop = iolink_workbench::capture::Stop::default();
    stop.request();
    let finished = iolink_workbench::capture::run(
        &app.profile,
        &iolink_workbench::capture::Options::default(),
        &stop,
        None,
    )
    .unwrap();
    assert!(finished.success());
    let mut release: Vec<TestCompletion> = vec![
        Box::new(move || {
            let _ = capture_sender.send(Ok(finished));
        }),
        Box::new(move || {
            let _ = isdu_sender.send(Ok("synthetic acyclic completion".into()));
        }),
        app.setup.gate_for_test(),
    ];
    release.extend(app.saved_capture.gates_for_test());
    release.push(app.saved_parameter.gate_for_test());
    assert_eq!(app.pending_jobs(), 6);
    assert_eq!(app.advance_close(false), CloseAction::Idle);
    assert_eq!(app.advance_close(true), CloseAction::Wait);
    assert!(app.stop.requested());
    let ctx = Context::default();
    let output = frame(&mut app, &ctx);
    assert!(command(&output, false) && !command(&output, true));
    for (index, complete) in release.into_iter().enumerate() {
        complete();
        let output = frame(&mut app, &ctx);
        let remaining = 5 - index;
        assert_eq!(app.pending_jobs(), remaining);
        if remaining != 0 {
            assert!(command(&output, false) && !command(&output, true));
        } else {
            assert!(command(&output, true));
        }
    }
    assert_eq!(app.isdu_result, "synthetic acyclic completion");
    assert_eq!(app.profile, before);
}

#[test]
fn disconnected_worker_does_not_leave_close_waiting_forever() {
    let mut app = App::new();
    let (sender, receiver) = mpsc::channel();
    app.isdu_worker = Some(receiver);
    drop(sender);
    assert_eq!(app.advance_close(true), CloseAction::Wait);
    let output = frame(&mut app, &Context::default());
    assert_eq!(app.pending_jobs(), 0);
    assert!(command(&output, true));
    assert!(app.isdu_result.contains("stopped unexpectedly"));
}

#[test]
fn close_state_rejects_a_new_session_before_resetting_stop_or_publishing_files()
-> anyhow::Result<()> {
    let _serial = desktop::WORKER_TEST_LOCK.lock().unwrap();
    std::fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let path = scratch.path().join("must-not-exist");
    let mut app = App::new();
    app.capture_path = path.to_string_lossy().into_owned();
    let stop = app.stop.clone();
    assert_eq!(app.advance_close(true), CloseAction::Close);
    assert!(format!("{:#}", app.start(true).unwrap_err()).contains("Application is closing"));
    assert!(stop.requested() && app.stop.requested());
    assert!(app.worker.is_none() && !path.exists());
    Ok(())
}

#[test]
fn normal_close_waits_for_simulator_csv_flush_and_preserves_a_verified_bundle() -> anyhow::Result<()>
{
    let _serial = desktop::WORKER_TEST_LOCK.lock().unwrap();
    std::fs::create_dir_all(".local")?;
    let scratch = tempfile::tempdir_in(".local")?;
    let path = scratch.path().join("capture");
    let mut app = App::new();
    app.capture_path = path.to_string_lossy().into_owned();
    app.duration = 0;
    app.profile.logging.rotate_rows = 2;
    app.profile.logging.queue_capacity = 2;
    app.start(true)?;
    let ctx = Context::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.live.counts.received == 0 && app.worker.is_some() && Instant::now() < deadline {
        app.update_workers(&ctx);
        thread::sleep(Duration::from_millis(1));
    }
    anyhow::ensure!(
        app.live.counts.received > 0,
        "simulator produced no GUI observations"
    );
    assert_eq!(app.advance_close(true), CloseAction::Wait);
    while app.pending_jobs() != 0 && Instant::now() < deadline {
        app.update_workers(&ctx);
        thread::sleep(Duration::from_millis(1));
    }
    anyhow::ensure!(
        app.pending_jobs() == 0,
        "GUI capture did not complete after stop"
    );
    let report = app
        .report
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("capture report missing: {}", app.message))?;
    anyhow::ensure!(
        report.success() && report.stopped_by_request,
        "GUI close capture failed: {report:?}"
    );
    let checked = archive::verify(&path, None)?;
    assert_eq!(checked.rows_checked, report.csv_rows_flushed);
    assert!(checked.rows_checked > 0 && checked.csv_parts > 1);
    assert_eq!(app.advance_close(false), CloseAction::Close);
    Ok(())
}

#[test]
fn disposing_app_requests_stop_without_launching_an_operation() {
    let app = App::new();
    let stop = app.stop.clone();
    assert!(!stop.requested());
    drop(app);
    assert!(stop.requested());
}

#[test]
fn capture_failure_during_close_cancels_exit_and_preserves_the_error() {
    let mut app = App::new();
    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    assert_eq!(app.advance_close(true), CloseAction::Wait);
    sender
        .send(Err(anyhow::anyhow!("synthetic CSV flush failure")))
        .unwrap();
    let output = frame(&mut app, &Context::default());
    assert!(command(&output, false) && !command(&output, true));
    assert!(
        !app.closing
            && app.message.contains("Close canceled")
            && app.message.contains("CSV flush failure")
    );
    // A later explicit close request can leave after the failure has been presented.
    assert_eq!(app.advance_close(true), CloseAction::Close);

    let mut app = App::new();
    let stop = iolink_workbench::capture::Stop::default();
    stop.request();
    let mut report = iolink_workbench::capture::run(
        &app.profile,
        &iolink_workbench::capture::Options::default(),
        &stop,
        None,
    )
    .unwrap();
    report.writer_error = Some("synthetic final drain error".into());
    let (sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    assert_eq!(app.advance_close(true), CloseAction::Wait);
    sender.send(Ok(report)).unwrap();
    let output = frame(&mut app, &Context::default());
    assert!(command(&output, false) && !command(&output, true));
    assert!(!app.closing && app.message.contains("final drain error"));
    assert!(app.report.as_ref().unwrap().writer_error.is_some());
}
