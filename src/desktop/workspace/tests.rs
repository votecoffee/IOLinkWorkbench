//! Isolated egui input only: these tests never send desktop input.
use super::*;
use anyhow::Result;
use std::time::Instant;

fn draw(app: &mut App, ctx: &Context, size: egui::Vec2) -> Vec<(String, egui::Rect)> {
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        },
        |ctx| app.render(ctx),
    );
    fn texts(shape: egui::Shape, found: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(text) => found.push((
                text.galley.job.text.clone(),
                egui::Rect::from_min_size(text.pos, text.galley.size()),
            )),
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    texts(shape, found);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for shape in output.shapes {
        texts(shape.shape, &mut found);
    }
    found
}

fn click(app: &mut App, ctx: &Context, size: egui::Vec2, label: &str) {
    draw(app, ctx, size);
    let found = draw(app, ctx, size);
    let position = found
        .iter()
        .find(|(text, _)| text == label)
        .unwrap_or_else(|| panic!("Missing control {label}"))
        .1
        .center();
    for pressed in [true, false] {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| app.render(ctx),
        );
    }
}

fn paste(app: &mut App, ctx: &Context, size: egui::Vec2, hint: &str, value: &str) {
    click(app, ctx, size, hint);
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events: vec![egui::Event::Paste(value.into())],
            ..Default::default()
        },
        |ctx| app.render(ctx),
    );
}

#[test]
fn overview_routes_to_setup_tasks_without_mutating_or_starting() {
    for size in [egui::vec2(790.0, 530.0), egui::vec2(1130.0, 790.0)] {
        let ctx = Context::default();
        let mut app = App::new();
        let original = app.profile.clone();
        assert_eq!(app.setup.page, setup::Page::Overview);
        click(&mut app, &ctx, size, "Edit connection");
        assert_eq!(app.setup.page, setup::Page::Connection);
        click(&mut app, &ctx, size, "Overview");
        click(&mut app, &ctx, size, "Import IODD");
        assert_eq!(app.setup.page, setup::Page::Iodd);
        click(&mut app, &ctx, size, "Overview");
        click(&mut app, &ctx, size, "Review mappings");
        assert_eq!(app.tab, Tab::Mappings);
        assert_eq!(app.profile, original);
        assert_eq!(app.pending_jobs(), 0);
    }
}

#[test]
fn starter_selection_is_offline_and_protects_the_loaded_profile_file() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("original.toml");
    let mut original = Profile::parse(include_str!("../../../samples/simulator.toml"))?;
    original.name = "Previously saved profile".into();
    original.save_new(&path)?;
    let original_bytes = fs::read(&path)?;
    let size = egui::vec2(790.0, 530.0);
    let ctx = Context::default();
    let mut app = App::new();
    click(&mut app, &ctx, size, "Profiles…");
    click(&mut app, &ctx, size, "Open & save");
    paste(
        &mut app,
        &ctx,
        size,
        "Enter or paste a full profile path",
        &path.display().to_string(),
    );
    click(&mut app, &ctx, size, "Load profile");
    assert_eq!(app.profile, original);
    assert!(!app.workspace.profile_dialog);
    assert_eq!(app.setup.page, setup::Page::Overview);
    click(&mut app, &ctx, size, "Profiles…");
    click(&mut app, &ctx, size, "Start a profile");
    click(&mut app, &ctx, size, "Simulator with raw-only CSV");
    assert_eq!(app.profile, original, "Selection alone must not apply");
    click(&mut app, &ctx, size, "Use starting profile");
    assert!(app.profile.logging.raw_only);
    assert_eq!(app.profile.effective_fields().len(), 2);
    assert!(!app.workspace.profile_dialog);
    assert_eq!(app.tab, Tab::Device);
    assert_eq!(app.pending_jobs(), 0);
    click(&mut app, &ctx, size, "Profiles…");
    click(&mut app, &ctx, size, "Open & save");
    assert!(
        draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == "Enter or paste a full profile path")
    );
    click(&mut app, &ctx, size, "Save / replace profile");
    assert_eq!(fs::read(&path)?, original_bytes);
    // A malformed load must also preserve the applied settings and source bytes.
    let malformed = directory.path().join("malformed.toml");
    fs::write(&malformed, "invalid TOML = [")?;
    let applied = app.profile.clone();
    paste(
        &mut app,
        &ctx,
        size,
        "Enter or paste a full profile path",
        &malformed.display().to_string(),
    );
    click(&mut app, &ctx, size, "Load profile");
    assert_eq!(app.profile, applied);
    assert!(app.workspace.profile_dialog);
    assert_eq!(fs::read(&path)?, original_bytes);
    assert_eq!(app.pending_jobs(), 0);
    Ok(())
}

#[test]
fn profile_replacement_is_fenced_by_drafts_and_all_setup_owners() {
    for fence in ["draft", "cyclic", "isdu", "inspection", "closing"] {
        let size = egui::vec2(790.0, 530.0);
        let ctx = Context::default();
        let mut app = App::new();
        click(&mut app, &ctx, size, "Profiles…");
        click(&mut app, &ctx, size, "ifm AL1350 setup template");
        let original = app.profile.clone();
        let mut held_cyclic = None;
        let mut held_isdu = None;
        let mut held_inspection = None;
        match fence {
            "draft" => app.setup.mark_pending_for_test(),
            "cyclic" => {
                let (sender, receiver) = mpsc::channel();
                held_cyclic = Some(sender);
                app.worker = Some(receiver);
            }
            "isdu" => {
                let (sender, receiver) = mpsc::channel();
                held_isdu = Some(sender);
                app.isdu_worker = Some(receiver);
            }
            "inspection" => held_inspection = Some(app.setup.gate_for_test()),
            "closing" => app.closing = true,
            _ => unreachable!(),
        }
        click(&mut app, &ctx, size, "Use starting profile");
        assert_eq!(app.profile, original, "Replacement bypassed {fence}");
        drop((held_cyclic, held_isdu, held_inspection));
    }
}

#[test]
fn hardware_template_clears_the_synthetic_sensor_and_reports_missing_route() {
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let mut app = App::new();
    click(&mut app, &ctx, size, "Profiles…");
    click(&mut app, &ctx, size, "ifm AL1350 setup template");
    let found = draw(&mut app, &ctx, size);
    let button = found
        .iter()
        .find(|(text, _)| text == "Use starting profile")
        .unwrap()
        .1;
    assert!(
        button.min.y >= 0.0 && button.max.y < size.y,
        "Starter action must fit the minimum window"
    );
    assert_eq!(app.profile.connection.gateway, Gateway::Simulator);
    click(&mut app, &ctx, size, "Use starting profile");
    assert_eq!(app.profile.connection.gateway, Gateway::IfmAl1350);
    assert!(app.profile.connection.endpoint.is_empty());
    assert!(app.profile.effective_fields().is_empty());
    assert_eq!(app.profile.expected_identity, Default::default());
    assert!(app.profile.iodd.is_none());
    assert_eq!(app.setup.page, setup::Page::Overview);
    assert!(
        draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == "Needs input · master endpoint is empty")
    );
    assert_eq!(app.pending_jobs(), 0);
}

#[test]
fn workflows_are_isolated_and_toolbar_fits_supported_windows() {
    for size in [egui::vec2(790.0, 530.0), egui::vec2(1130.0, 790.0)] {
        let ctx = Context::default();
        let mut app = App::new();
        let before = app.profile.clone();
        for tab in Tab::ALL {
            app.tab = tab;
            draw(&mut app, &ctx, size);
            let found = draw(&mut app, &ctx, size);
            for label in ["Connect…", "Record CSV…", "Profiles…"]
                .into_iter()
                .chain(Tab::ALL.map(Tab::label))
            {
                let bounds = found
                    .iter()
                    .find(|(text, _)| text == label)
                    .unwrap_or_else(|| panic!("{label} absent on {tab:?}"))
                    .1;
                assert!(
                    bounds.min.x >= 0.0
                        && bounds.max.x <= size.x
                        && bounds.min.y >= 0.0
                        && bounds.max.y <= size.y,
                    "Toolbar {label} clipped on {tab:?}: {bounds:?}"
                );
            }
            for (page, heading) in [
                (Tab::Device, "Device setup"),
                (Tab::Inputs, "Live inputs"),
                (Tab::Logging, "CSV logging"),
                (Tab::Mappings, "Process input mappings"),
                (Tab::Parameters, "IODD parameter reads"),
                (Tab::Integration, "Software integration"),
                (Tab::Diagnostics, "Connection diagnostics"),
                (Tab::Offline, "Review saved capture"),
            ] {
                assert_eq!(
                    found.iter().any(|(text, _)| text == heading),
                    tab == page,
                    "{tab:?}: {heading}"
                );
            }
            assert_eq!(app.profile, before);
            assert_eq!(app.pending_jobs(), 0);
        }
    }
}

#[test]
fn persistent_actions_review_connections_and_reveal_hidden_drafts() {
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let mut app = App::new();
    click(&mut app, &ctx, size, "Record CSV…");
    assert_eq!(app.tab, Tab::Logging);
    assert!(app.worker.is_none());
    click(&mut app, &ctx, size, "Profiles…");
    assert!(app.workspace.profile_dialog);
    assert!(
        draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == "Device profiles")
    );
    app.workspace.profile_dialog = false;
    click(&mut app, &ctx, size, "Connect…");
    assert!(app.workspace.connect_dialog);
    assert!(app.worker.is_none());
    assert!(
        draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == "Offline simulator · synthetic input")
    );
    app.workspace.connect_dialog = false;
    app.setup.mark_pending_for_test();
    app.tab = Tab::Offline;
    click(&mut app, &ctx, size, "Review pending edits");
    assert_eq!(app.tab, Tab::Device);
    assert_eq!(app.setup.page, setup::Page::Advanced);
    assert!(app.start_block_reason().is_some());
    assert!(app.worker.is_none());
}

#[test]
fn stop_and_flush_remains_available_on_another_tab() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let destination = tempfile::tempdir()?;
    let path = destination.path().join("capture");
    let mut app = App::new();
    app.capture_path = path.display().to_string();
    app.duration = 0;
    app.start(true)?;
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.live.counts.received == 0 && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.live.counts.received > 0);
    app.tab = Tab::Integration;
    click(&mut app, &ctx, size, "Stop and flush");
    assert!(app.stop.requested());
    while app.worker.is_some() && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.worker.is_none());
    let report = app.report.as_ref().expect("completed capture");
    assert!(report.success() && report.csv_rows_flushed > 0 && report.csv_rows_unconfirmed == 0);
    let verified = iolink_workbench::archive::verify(&path, None)?;
    assert_eq!(verified.rows_checked, report.csv_rows_flushed);
    Ok(())
}
