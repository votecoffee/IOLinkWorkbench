//! Isolated egui input only: these tests never send desktop input.
use super::*;
use anyhow::Result;
use std::{fs, time::Instant};

fn draw(app: &mut App, ctx: &Context, size: egui::Vec2) -> Vec<(String, egui::Rect)> {
    draw_input(app, ctx, size, egui::RawInput::default())
}

fn draw_input(
    app: &mut App,
    ctx: &Context,
    size: egui::Vec2,
    mut input: egui::RawInput,
) -> Vec<(String, egui::Rect)> {
    input.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
    let output = ctx.run(input, |ctx| app.render(ctx));
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
        .unwrap_or_else(|| {
            let visible: Vec<_> = found.iter().map(|(text, _)| text.as_str()).collect();
            panic!("Missing control {label}. Drawn text: {visible:?}")
        })
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
fn copied_input_keeps_receipt_quality_bytes_and_mapping_when_the_profile_changes() -> Result<()> {
    use iolink_workbench::transport::{Input, Payload, Validity};
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let mut app = App::new();
    app.profile.connection.simulate_snapshots = true;
    let (status, _) = capture::probe(&app.profile)?;
    assert_eq!(status.freshness, Freshness::Snapshots);
    app.live = Live {
        status: Some(status),
        generation: 3,
        input: Some(Input {
            received: Instant::now() - Duration::from_secs(1),
            payload: Payload::new(&[0x03, 0xe9])?,
            validity: Validity::Invalid,
            counter: None,
            source_time_us: None,
            source_age_us: None,
            diagnostic_code: None,
        }),
        ..Default::default()
    };
    app.tab = Tab::Inputs;
    let has = |app: &mut App, snippet: &str| {
        draw(app, &ctx, size)
            .iter()
            .any(|(text, _)| text.contains(snippet))
    };
    assert!(has(&mut app, "RAW  03E9"));
    assert!(!has(&mut app, "50.0"));
    click(&mut app, &ctx, size, "Inspect bytes offline");
    assert_eq!(app.tab, Tab::Offline);
    assert_eq!(app.workspace.offline, OfflinePage::Process);
    assert_eq!(app.pending_jobs(), 0);
    assert!(has(
        &mut app,
        "Validity at receipt invalid · staleness at receipt unknown"
    ));
    assert!(has(
        &mut app,
        "Host poll · source age and update count unknown"
    ));
    assert!(has(&mut app, "generation 3"));
    assert!(has(&mut app, "50.0"));
    assert_eq!(app.live.input.unwrap().validity, Validity::Invalid);

    app.profile.name = "Different mapping".into();
    app.profile.fields[0].scale = 1.0;
    app.setup.sync(&app.profile);
    app.profile_changed();
    assert!(has(&mut app, "Copied mapping · Synthetic level sensor"));
    assert!(has(&mut app, "RAW  03E9"));
    assert!(has(&mut app, "50.0"));
    click(&mut app, &ctx, size, "Input bytes");
    assert!(has(&mut app, "RAW  FFF9"));
    assert!(has(&mut app, "Different mapping"));
    click(&mut app, &ctx, size, "From Inputs");
    assert!(has(&mut app, "RAW  03E9"));
    assert_eq!(app.pending_jobs(), 0);
    Ok(())
}

#[test]
fn capture_folder_check_is_read_only_and_start_rechecks_its_destination() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let scratch = tempfile::tempdir()?;
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let mut app = App::new();
    app.tab = Tab::Logging;
    let has = |app: &mut App, snippet: &str| {
        draw(app, &ctx, size)
            .iter()
            .any(|(text, _)| text.contains(snippet))
    };

    let absent_parent = scratch.path().join("missing");
    app.capture_path = absent_parent.join("new-capture").display().to_string();
    click(&mut app, &ctx, size, "Check folder");
    assert!(has(&mut app, "Capture parent folder does not exist"));
    assert!(!absent_parent.exists());
    assert_eq!(app.pending_jobs(), 0);

    let regular_file = scratch.path().join("file-not-parent");
    fs::write(&regular_file, b"original bytes")?;
    app.capture_path = regular_file.join("new-capture").display().to_string();
    assert!(!has(&mut app, "Capture parent folder does not exist"));
    click(&mut app, &ctx, size, "Check folder");
    assert!(has(&mut app, "Capture parent is not a folder"));
    assert_eq!(fs::read(&regular_file)?, b"original bytes");
    app.capture_path = regular_file.display().to_string();
    click(&mut app, &ctx, size, "Check folder");
    assert!(has(&mut app, "Capture folder already exists"));

    let destination = scratch.path().join("検査 capture");
    app.capture_path = destination.display().to_string();
    click(&mut app, &ctx, size, "Check folder");
    assert!(has(&mut app, "Folder name available at check time"));
    assert!(has(&mut app, "検査 capture"));
    assert!(!destination.exists());
    assert_eq!(app.pending_jobs(), 0);
    fs::create_dir(&destination)?;
    let prior_data = destination.join("existing.txt");
    fs::write(&prior_data, b"keep me")?;
    app.live.counts.received = 41;
    app.stop.request();
    let prior_report = capture::run(&app.profile, &capture::Options::default(), &app.stop, None)?;
    let prior_report_text = toml::to_string(&prior_report)?;
    app.report = Some(prior_report);
    click(&mut app, &ctx, size, "Start CSV capture");
    assert!(
        app.current_folder_check()
            .is_some_and(|check| !check.available)
    );
    assert!(has(&mut app, "Capture folder already exists"));
    assert_eq!(fs::read(&prior_data)?, b"keep me");
    assert!(app.stop.requested() && app.live.counts.received == 41 && app.pending_jobs() == 0);
    assert_eq!(
        toml::to_string(app.report.as_ref().unwrap())?,
        prior_report_text
    );
    Ok(())
}

#[test]
fn feed_review_routes_errors_and_shows_the_prepared_session_destinations() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let mut app = App::new();
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let has = |app: &mut App, snippet: &str| {
        draw(app, &ctx, size)
            .iter()
            .any(|(text, _)| text.contains(snippet))
    };
    let scratch = tempfile::tempdir()?;
    let destination = scratch.path().join("no-capture");
    app.capture_path = destination.display().to_string();
    app.api_enabled = true;
    app.api_port = 18765;
    app.mqtt_enabled = true;
    app.mqtt_broker = "127.0.0.1:1".into();
    app.mqtt_topic = "line/+/inputs".into();
    app.shared_memory_enabled = true;
    app.shared_memory_name = "invalid\\name".into();
    app.stop.request();
    app.live.counts.received = 24;

    click(&mut app, &ctx, size, "Connect…");
    assert!(has(
        &mut app,
        "Local HTTP API · http://127.0.0.1:18765/v1/status"
    ));
    assert!(has(&mut app, "MQTT topic must be"));
    assert!(has(&mut app, "shared-memory name must be"));
    click(&mut app, &ctx, size, "Start monitor");
    assert_eq!(app.pending_jobs(), 0);
    click(&mut app, &ctx, size, "Edit MQTT settings");
    assert_eq!(app.tab, Tab::Integration);
    assert_eq!(app.workspace.integration, IntegrationPage::Mqtt);
    assert!(!app.workspace.connect_dialog);
    assert!(has(&mut app, "MQTT topic must be"));
    assert!(
        app.start(true)
            .unwrap_err()
            .to_string()
            .contains("MQTT settings")
    );
    assert!(app.stop.requested() && app.live.counts.received == 24);
    assert!(!destination.exists());

    app.mqtt_topic = "line/1/inputs".into();
    click(&mut app, &ctx, size, "Record CSV…");
    assert!(has(&mut app, "shared-memory name must be"));
    click(&mut app, &ctx, size, "Start CSV capture");
    assert_eq!(app.pending_jobs(), 0);
    assert!(!destination.exists());
    click(&mut app, &ctx, size, "Edit shared memory settings");
    assert_eq!(app.workspace.integration, IntegrationPage::SharedMemory);
    assert!(has(&mut app, "shared-memory name must be"));
    app.shared_memory_name = "line.1".into();
    app.mqtt_interval_ms = 250;
    app.shared_memory_interval_ms = 30;
    click(&mut app, &ctx, size, "Connect…");
    assert!(has(
        &mut app,
        "MQTT · 127.0.0.1:1 · topic \"line/1/inputs\" · 250 ms"
    ));
    assert!(has(
        &mut app,
        "Shared memory · Local\\IOLinkWorkbench.Snapshot.line.1 · 30 ms"
    ));
    let feeds = app.review_feeds().into_options()?;
    assert_eq!(feeds.local_api.unwrap().listen.port(), 18765);
    let mqtt = feeds.mqtt.unwrap();
    assert_eq!(mqtt.broker.to_string(), "127.0.0.1:1");
    assert_eq!(mqtt.topic, "line/1/inputs");
    assert_eq!(mqtt.interval_ms, 250);
    assert_eq!(
        feeds.shared_memory.unwrap().mapping_name(),
        "Local\\IOLinkWorkbench.Snapshot.line.1"
    );
    app.mqtt_broker = "ignored while disabled".into();
    app.mqtt_enabled = false;
    assert!(app.review_feeds().into_options()?.mqtt.is_none());
    assert_eq!(app.pending_jobs(), 0);
    Ok(())
}

#[test]
fn output_only_profile_blocks_input_controls_and_opens_output_workflows() -> Result<()> {
    let mut app = App::new();
    let xml = include_str!("../../../samples/sensor.xml")
        .replace("ProcessDataIn id=\"IN\"", "ProcessDataOut id=\"OUT\"")
        .replace("</ProcessDataIn>", "</ProcessDataOut>");
    let catalog = iolink_workbench::iodd::parse(&xml, "synthetic output".into())?;
    let (pins, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    app.profile.apply_iodd(pins, imported);
    app.profile.validate()?;
    app.setup.sync(&app.profile);
    assert!(
        app.start(false)
            .unwrap_err()
            .to_string()
            .contains("output-only layout")
    );
    assert!(app.start(true).is_err() && app.worker.is_none());

    let context = Context::default();
    let size = egui::vec2(1060.0, 830.0);
    app.tab = Tab::Device;
    click(&mut app, &context, size, "Open reviewed writes");
    assert!(app.setup.page == setup::Page::Writes);
    app.tab = Tab::Mappings;
    let labels = draw(&mut app, &context, size);
    assert!(
        labels
            .iter()
            .any(|(label, _)| label == "IODD output layout")
    );
    assert!(labels.iter().all(|(label, _)| label != "Add manual field"));
    app.tab = Tab::Offline;
    app.workspace.offline = OfflinePage::Process;
    draw(&mut app, &context, size);
    assert_eq!(
        app.payload_inspector.page,
        super::super::payload_inspector::Page::Output
    );
    Ok(())
}

#[test]
fn session_route_reviews_block_start_and_point_to_the_relevant_settings() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let mut app = App::new();
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    app.profile = Profile::parse(include_str!("../../../samples/ifm-al1350.toml"))?;
    app.profile.connection.endpoint = "https://synthetic-secret@127.0.0.1:1/".into();
    app.setup.sync(&app.profile);
    let path = tempfile::tempdir()?;
    let destination = path.path().join("no-capture");
    app.capture_path = destination.display().to_string();
    app.live.counts.received = 12;
    let prior_stop = app.stop.clone();
    prior_stop.request();
    click(&mut app, &ctx, size, "Connect…");
    let found = draw(&mut app, &ctx, size);
    assert!(
        found
            .iter()
            .any(|(text, _)| text.contains("origin without path or credentials"))
    );
    assert!(
        !found
            .iter()
            .any(|(text, _)| text.contains("synthetic-secret"))
    );
    click(&mut app, &ctx, size, "Start monitor");
    assert_eq!(app.pending_jobs(), 0);
    click(&mut app, &ctx, size, "Go to connection settings");
    assert_eq!(app.tab, Tab::Device);
    assert_eq!(app.setup.page, setup::Page::Connection);
    assert!(!app.workspace.connect_dialog);
    assert!(
        app.start(true)
            .unwrap_err()
            .to_string()
            .contains("origin without")
    );
    assert!(!destination.exists());
    assert!(app.stop.requested());
    assert_eq!(app.live.counts.received, 12);

    let catalog = iodd::parse(
        include_str!("../../../samples/sensor.xml"),
        "synthetic condition receipt".into(),
    )?;
    let (identity, mut imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    imported.condition = Some("Demo mode 1".into());
    app.profile.connection.endpoint = "https://127.0.0.1:1".into();
    app.profile.input_bytes = imported.input_bytes;
    app.profile.expected_identity = identity;
    app.profile.iodd = Some(imported);
    app.setup.sync(&app.profile);
    click(&mut app, &ctx, size, "Record CSV…");
    assert_eq!(app.tab, Tab::Logging);
    let found = draw(&mut app, &ctx, size);
    assert!(found.iter().any(|(text, _)| text.contains("Demo mode 1")));
    click(&mut app, &ctx, size, "Start CSV capture");
    assert_eq!(app.pending_jobs(), 0);
    assert!(
        app.start(true)
            .unwrap_err()
            .to_string()
            .contains("Demo mode 1")
    );
    assert!(!destination.exists());
    click(&mut app, &ctx, size, "Review IODD layout");
    assert_eq!(app.tab, Tab::Mappings);
    assert!(
        draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text.contains("Confirmed active condition on the device"))
    );
    assert!(!app.profile.confirm_iodd_condition);
    assert_eq!(app.pending_jobs(), 0);
    app.profile.confirm_iodd_condition = true;
    assert!(app.check_session_route().is_ok());
    assert!(!destination.exists());
    Ok(())
}

#[test]
fn master_port_configuration_opens_without_another_read() -> Result<()> {
    use crate::desktop::acyclic::{Output, Query};
    use crate::desktop::workspace::DiagnosticsPage;

    let mut app = App::new();
    app.profile = Profile::parse(include_str!("../../../samples/ifm-al1352.toml"))?;
    app.setup.sync(&app.profile);
    app.tab = Tab::Diagnostics;
    app.workspace.diagnostics = DiagnosticsPage::Connection;
    let details = "Port 1 · Cycle preset automatic · Configured Vendor ID 310";
    app.acyclic.store_result(
        Query::MasterInspection,
        Ok(Output {
            text: "Port 1 · connected Vendor ID 65535".into(),
            port_configuration: Some(details.into()),
            comparison_sources: None,
        }),
    );
    let ctx = Context::default();
    let size = egui::vec2(1130.0, 790.0);
    let visible =
        |app: &mut App, label: &str| draw(app, &ctx, size).iter().any(|(text, _)| text == label);
    assert!(visible(&mut app, "Port 1 · connected Vendor ID 65535"));
    assert!(!visible(&mut app, details));
    click(&mut app, &ctx, size, "Port configuration");
    assert!(visible(&mut app, details));
    assert!(!app.acyclic.busy());
    Ok(())
}

#[test]
fn acyclic_results_follow_the_read_action_and_keep_the_original_address() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let catalog = iodd::parse(
        include_str!("../../../samples/sensor-formats.xml"),
        "synthetic GUI result routing".into(),
    )?;
    let (identity, imported) = catalog.select(None, None)?;
    let mut app = App::new();
    app.profile.input_bytes = imported.input_bytes;
    app.profile.expected_identity = identity;
    app.profile.iodd = Some(imported);
    app.setup.sync(&app.profile);
    app.parameter_picker.selected = Some("V_SIM_Record".into());
    app.parameter_picker.subindex = 1;
    app.tab = Tab::Parameters;
    let ctx = Context::default();
    let size = egui::vec2(1130.0, 790.0);
    let finish = |app: &mut App| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.acyclic.busy() && Instant::now() < deadline {
            draw(app, &ctx, size);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!app.acyclic.busy());
    };
    let titles = |app: &mut App| {
        draw(app, &ctx, size)
            .into_iter()
            .map(|(text, _)| text)
            .filter(|text| text.starts_with("Last result"))
            .collect::<Vec<_>>()
    };

    click(&mut app, &ctx, size, "Read named parameter");
    click(&mut app, &ctx, size, "Raw ISDU");
    finish(&mut app);
    assert!(titles(&mut app).is_empty());
    click(&mut app, &ctx, size, "Read parameter");
    finish(&mut app);
    assert_eq!(titles(&mut app), ["Last result · Raw ISDU · index 19:0"]);
    app.isdu_index = 21;
    assert!(
        draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == "The current address differs from this read.")
    );
    click(&mut app, &ctx, size, "Verify Product ID");
    finish(&mut app);
    assert_eq!(
        titles(&mut app),
        [
            "Last result · Raw ISDU · index 19:0",
            "Last result · Product ID check · index 19:0 · expected SIM-SENSOR",
        ]
    );

    click(&mut app, &ctx, size, "Diagnostics");
    click(&mut app, &ctx, size, "Device events");
    assert!(titles(&mut app).is_empty());
    click(&mut app, &ctx, size, "Read device diagnostics");
    finish(&mut app);
    assert_eq!(
        titles(&mut app),
        ["Last result · Device diagnostics · indexes 36:0 and 37:0"]
    );
    click(&mut app, &ctx, size, "Parameters");
    click(&mut app, &ctx, size, "IODD reads");
    app.parameter_picker.subindex = 2;
    assert_eq!(
        titles(&mut app),
        ["Last result · V_SIM_Record · index 66:1"]
    );
    assert!(
        draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == "The current IODD selection differs from this read.")
    );
    assert!(
        app.acyclic
            .latest(acyclic::Kind::Named)
            .unwrap()
            .output
            .is_ok()
    );
    assert!(
        app.acyclic
            .latest(acyclic::Kind::Raw)
            .unwrap()
            .output
            .is_ok()
    );
    assert!(
        app.acyclic
            .latest(acyclic::Kind::ProductId)
            .unwrap()
            .output
            .is_ok()
    );
    assert!(
        app.acyclic
            .latest(acyclic::Kind::Events)
            .unwrap()
            .output
            .is_ok()
    );
    app.profile_changed();
    assert!(titles(&mut app).is_empty());
    click(&mut app, &ctx, size, "Raw ISDU");
    assert!(titles(&mut app).is_empty());
    assert_eq!(app.pending_jobs(), 0);
    Ok(())
}

#[test]
fn parameter_search_keeps_selection_explicit_and_read_controls_compact() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let catalog = iodd::parse(
        include_str!("../../../samples/sensor-formats.xml"),
        "synthetic GUI search fixture".into(),
    )?;
    let (identity, mut imported) = catalog.select(None, None)?;
    imported
        .parameters
        .iter_mut()
        .find(|p| p.index == 64)
        .unwrap()
        .name = "Étage threshold".into();
    let mut app = App::new();
    app.profile.input_bytes = imported.input_bytes;
    app.profile.expected_identity = identity;
    app.profile.iodd = Some(imported);
    app.setup.sync(&app.profile);
    app.tab = Tab::Parameters;
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let search = "Search name, IODD ID, or index";
    let has =
        |app: &mut App, label: &str| draw(app, &ctx, size).iter().any(|(text, _)| text == label);
    for query in ["étAGE", "0X0040", "00064", "v_sIm_thre"] {
        paste(&mut app, &ctx, size, search, query);
        assert!(has(&mut app, "1 of 3 parameters"), "query {query}");
        assert!(app.parameter_picker.selected.is_none());
        click(&mut app, &ctx, size, "Clear search");
    }
    paste(&mut app, &ctx, size, search, "66");
    click(&mut app, &ctx, size, "Select parameter");
    let access = app.profile.parameter("V_SIM_Record")?.access.label();
    click(
        &mut app,
        &ctx,
        size,
        &format!("Synthetic settings record · index 66 · {access}"),
    );
    assert_eq!(
        app.parameter_picker.selected.as_deref(),
        Some("V_SIM_Record")
    );
    click(&mut app, &ctx, size, "Subindex 0");
    click(&mut app, &ctx, size, "1 · Level");
    assert_eq!(app.parameter_picker.subindex, 1);
    assert_eq!(app.pending_jobs(), 0);
    assert!(has(&mut app, "Read named parameter"));
    assert!(!has(&mut app, "Read and save snapshot"));
    assert!(!has(&mut app, "Inspect declared default (offline)"));
    click(&mut app, &ctx, size, "IODD details and display preference");
    assert!(has(&mut app, "Inspect declared default (offline)"));
    click(&mut app, &ctx, size, "IODD details and display preference");
    click(&mut app, &ctx, size, "Read named parameter");
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.acyclic.busy() && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    let named = app
        .acyclic
        .latest(acyclic::Kind::Named)
        .unwrap()
        .output
        .as_ref()
        .map(|output| output.text.as_str())
        .unwrap();
    assert!(named.contains("index 66:1"), "{named}");
    click(&mut app, &ctx, size, "Clear search");
    paste(&mut app, &ctx, size, search, "no such parameter");
    assert!(has(&mut app, "No matching parameters"));
    assert!(has(
        &mut app,
        "Current selection is outside the search results."
    ));
    assert_eq!(app.parameter_picker.subindex, 1);
    click(&mut app, &ctx, size, "Clear search");
    paste(&mut app, &ctx, size, search, "67");
    click(&mut app, &ctx, size, "Synthetic settings record · index 66");
    let access = app.profile.parameter("V_SIM_History")?.access.label();
    click(
        &mut app,
        &ctx,
        size,
        &format!("Synthetic history · index 67 · {access}"),
    );
    assert_eq!(
        app.parameter_picker.selected.as_deref(),
        Some("V_SIM_History")
    );
    assert_eq!(app.parameter_picker.subindex, 0);
    app.tab = Tab::Offline;
    app.workspace.offline = OfflinePage::Parameter;
    assert!(has(&mut app, "3 of 3 parameters"));
    assert!(has(&mut app, "Select parameter"));
    paste(&mut app, &ctx, size, search, "0x40");
    assert!(has(&mut app, "1 of 3 parameters"));
    assert_eq!(app.pending_jobs(), 0);
    Ok(())
}

fn drop_paths(app: &mut App, ctx: &Context, size: egui::Vec2, paths: &[&Path]) {
    draw_input(
        app,
        ctx,
        size,
        egui::RawInput {
            dropped_files: paths
                .iter()
                .map(|path| egui::DroppedFile {
                    path: Some(path.to_path_buf()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        },
    );
}

#[test]
fn saved_marker_drops_stage_paths_and_respect_the_review_owner() -> Result<()> {
    let scratch = tempfile::tempdir()?;
    let directory = scratch.path().join("Earlier 録画");
    let marker = directory.join("CAPTURE.TOML");
    let bundle = scratch.path().join("redecoded").join("receipt.toml");
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let mut app = App::new();
    let before = app.profile.clone();
    let (_sender, receiver) = mpsc::channel();
    app.worker = Some(receiver);
    app.setup.mark_pending_for_test();
    app.tab = Tab::Offline;
    drop_paths(&mut app, &ctx, size, &[&marker]);
    assert_eq!(app.saved_capture.page, saved_capture::Page::Verify);
    assert!(app.message.starts_with("Capture folder selected."));
    assert!(!app.workspace.profile_dialog);
    assert_eq!(
        app.pending_jobs(),
        1,
        "A saved-file drop must only stage its path"
    );
    assert!(!directory.exists());
    assert_eq!(app.profile, before);
    let releases = app.saved_capture.gates_for_test();
    drop_paths(&mut app, &ctx, size, &[&bundle]);
    assert_eq!(app.saved_capture.page, saved_capture::Page::Verify);
    assert!(app.message.contains("active capture review or export"));
    assert_eq!(app.pending_jobs(), 3);
    app.saved_capture.page = saved_capture::Page::Compare;
    drop_paths(&mut app, &ctx, size, &[&marker]);
    assert_eq!(app.saved_capture.page, saved_capture::Page::Compare);
    assert!(app.message.contains("active comparison"));
    assert!(
        !draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == "Assign dropped capture")
    );
    for release in releases {
        release();
    }
    drop_paths(&mut app, &ctx, size, &[&bundle]);
    assert_eq!(app.saved_capture.page, saved_capture::Page::Exports);
    assert!(app.message.starts_with("Bundle folder selected."));
    assert_eq!(app.pending_jobs(), 1);

    let mut other = App::new();
    drop_paths(&mut other, &ctx, size, &[&marker]);
    assert!(
        other.workspace.profile_dialog,
        "A TOML file outside Saved captures remains a profile choice"
    );
    assert_eq!(other.pending_jobs(), 0);
    Ok(())
}

#[test]
fn profile_drop_selects_a_unicode_path_without_loading_until_requested() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("入力 device.TOML");
    let mut saved = App::new().profile.clone();
    saved.name = "Dropped profile".into();
    saved.save_new(&path)?;
    let original_bytes = fs::read(&path)?;
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let mut app = App::new();
    let before = app.profile.clone();
    app.tab = Tab::Inputs;
    app.workspace.connect_dialog = true;
    drop_paths(&mut app, &ctx, size, &[&path]);
    assert_eq!(app.profile, before);
    assert_eq!(app.pending_jobs(), 0);
    assert!(app.workspace.profile_dialog && !app.workspace.connect_dialog);
    assert!(app.message.starts_with("Profile path selected."));
    let found = draw(&mut app, &ctx, size);
    assert!(
        found
            .iter()
            .any(|(text, _)| text == &path.display().to_string())
    );
    assert!(
        found
            .iter()
            .any(|(text, _)| text == "Save destination (.toml)")
    );
    click(&mut app, &ctx, size, "Save / replace profile");
    assert_eq!(
        fs::read(&path)?,
        original_bytes,
        "Selecting a file must not redirect saving to it"
    );
    click(&mut app, &ctx, size, "Load profile");
    assert_eq!(app.profile, saved);
    assert_eq!(app.pending_jobs(), 0);
    assert_eq!(fs::read(&path)?, original_bytes);
    let other = directory.path().join("not-a-profile.toml");
    let other_bytes = b"keep this unvalidated source unchanged";
    fs::write(&other, other_bytes)?;
    drop_paths(&mut app, &ctx, size, &[&other]);
    assert_eq!(app.profile, saved);
    click(&mut app, &ctx, size, "Save / replace profile");
    assert_eq!(app.message, "Profile saved");
    assert_eq!(fs::read(&path)?, original_bytes);
    assert_eq!(
        fs::read(&other)?,
        other_bytes,
        "Drop must not redirect an existing save destination"
    );
    Ok(())
}

#[test]
fn iodd_drop_invalidates_the_old_preview_and_requires_explicit_inspection_and_apply() -> Result<()>
{
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let old_path = root.join("samples/sensor.xml");
    let new_path = root.join("samples/sensor-array.xml");
    let original_bytes = fs::read(&new_path)?;
    let ctx = Context::default();
    let size = egui::vec2(1130.0, 790.0);
    let mut app = App::new();
    let before = app.profile.clone();
    app.setup.select_iodd_path(old_path.display().to_string());
    click(&mut app, &ctx, size, "Inspect IODD");
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.setup.busy() && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!app.setup.busy());
    assert!(
        draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == "Apply IODD layout")
    );
    app.tab = Tab::Logging;
    drop_paths(&mut app, &ctx, size, &[&new_path]);
    assert_eq!(app.tab, Tab::Device);
    assert_eq!(app.setup.page, setup::Page::Iodd);
    assert_eq!(app.profile, before);
    assert_eq!(app.pending_jobs(), 0);
    assert!(
        !draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == "Apply IODD layout")
    );
    click(&mut app, &ctx, size, "Inspect IODD");
    while app.setup.busy() && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!app.setup.busy());
    assert_eq!(
        app.profile, before,
        "Inspection must not apply the description"
    );
    click(&mut app, &ctx, size, "Apply IODD layout");
    assert_eq!(
        app.profile.iodd.as_ref().expect("explicit import").source,
        "sensor-array.xml"
    );
    assert_eq!(app.pending_jobs(), 0);
    assert_eq!(fs::read(&new_path)?, original_bytes);
    Ok(())
}

#[test]
fn file_drops_are_fenced_by_drafts_and_setup_owners() {
    for fence in ["draft", "cyclic", "isdu", "inspection", "closing"] {
        let ctx = Context::default();
        let size = egui::vec2(790.0, 530.0);
        let mut app = App::new();
        let before = app.profile.clone();
        app.tab = Tab::Inputs;
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
                app.acyclic.hold_for_test(receiver);
            }
            "inspection" => held_inspection = Some(app.setup.gate_for_test()),
            "closing" => app.closing = true,
            _ => unreachable!(),
        }
        for name in ["not-read.toml", "not-inspected.zip"] {
            drop_paths(&mut app, &ctx, size, &[Path::new(name)]);
            assert_eq!(app.profile, before, "Drop bypassed {fence}");
            assert_eq!(app.tab, Tab::Inputs);
            assert_eq!(app.setup.page, setup::Page::Overview);
            assert!(!app.workspace.profile_dialog && !app.workspace.connect_dialog);
            assert!(
                app.message.starts_with("File selection unchanged"),
                "{fence}: {}",
                app.message
            );
        }
        drop((held_cyclic, held_isdu, held_inspection));
    }
}

#[test]
fn unsupported_and_multiple_drops_preserve_selection_and_hover_hint_fits() {
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let mut app = App::new();
    app.tab = Tab::Inputs;
    for paths in [
        vec![Path::new("unknown.bin")],
        vec![Path::new("a.toml"), Path::new("b.xml")],
    ] {
        drop_paths(&mut app, &ctx, size, &paths);
        assert_eq!(app.tab, Tab::Inputs);
        assert!(!app.workspace.profile_dialog);
        assert!(app.message.ends_with("File selection unchanged."));
        assert_eq!(app.pending_jobs(), 0);
    }
    draw_input(
        &mut app,
        &ctx,
        size,
        egui::RawInput {
            dropped_files: vec![egui::DroppedFile {
                name: "without-local-path.toml".into(),
                ..Default::default()
            }],
            ..Default::default()
        },
    );
    assert!(app.message.starts_with("Drop a local file"));
    assert!(!app.workspace.profile_dialog);
    let hover = egui::RawInput {
        hovered_files: vec![egui::HoveredFile::default()],
        ..Default::default()
    };
    draw_input(&mut app, &ctx, size, hover.clone()); // egui sizes a new Area before painting it.
    {
        let found = draw_input(&mut app, &ctx, size, hover);
        let bounds = found
            .iter()
            .find(|(text, _)| text == "Drop a profile or IODD")
            .expect("hover hint")
            .1;
        assert!(
            bounds.min.x >= 0.0
                && bounds.max.x <= size.x
                && bounds.min.y >= 0.0
                && bounds.max.y <= size.y
        );
    }
    assert_eq!(app.pending_jobs(), 0);
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
                app.acyclic.hold_for_test(receiver);
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
fn stop_and_flush_from_another_tab_can_review_the_recording() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let destination = tempfile::tempdir()?;
    let path = destination.path().join("capture");
    let mut app = App::new();
    app.profile.logging.rotate_rows = 2;
    app.profile
        .set_log_selection(iolink_workbench::profile::LogSelection::Fields(vec![
            "switch".into(),
            "level".into(),
        ]))?;
    app.capture_path = path.display().to_string();
    app.duration = 0;
    app.start(true)?;
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.live.counts.received < 3 && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.live.counts.received >= 3);
    app.tab = Tab::Inputs;
    click(&mut app, &ctx, size, "Inspect bytes offline");
    assert_eq!(app.tab, Tab::Offline);
    let received = app.live.counts.received;
    while app.live.counts.received <= received && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.live.counts.received > received);
    assert!(app.active_recording.is_some() && !app.stop.requested());
    app.tab = Tab::Logging;
    let found = draw(&mut app, &ctx, size);
    assert!(
        found
            .iter()
            .any(|(text, _)| text.starts_with("Recording to "))
    );
    assert!(
        !found
            .iter()
            .any(|(text, _)| text == "Verify latest recording")
    );
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
    let rows = report.csv_rows_flushed;
    let parts = report.csv_parts;
    assert!(parts > 1);
    let timing_window = format!(
        "Local setup · {} µs · Acquisition window · {} µs",
        report.local_setup_us,
        report.elapsed_us - report.local_setup_us
    );
    app.tab = Tab::Diagnostics;
    app.workspace.diagnostics = DiagnosticsPage::Timing;
    assert!(
        draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == &timing_window)
    );
    // The operator can review the stored selection while a different selection is applied.
    app.profile
        .set_log_selection(iolink_workbench::profile::LogSelection::All)?;
    assert_eq!(app.profile.log_fields()[0].name, "level");
    app.setup.sync(&app.profile);
    let joined = destination.path().join("combined.csv");
    app.tab = Tab::Offline;
    click(&mut app, &ctx, size, "Saved captures");
    drop_paths(&mut app, &ctx, size, &[&path.join("capture.toml")]);
    assert_eq!(app.saved_capture.page, saved_capture::Page::Verify);
    assert_eq!(app.pending_jobs(), 0);
    click(&mut app, &ctx, size, "Exports");
    paste(
        &mut app,
        &ctx,
        size,
        "New combined CSV filename",
        &joined.display().to_string(),
    );
    // A later monitor session supersedes the timing report, but not the last saved recording.
    app.start(false)?;
    app.stop.request();
    while app.worker.is_some() && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.report.as_ref().expect("monitor report").csv_parts, 0);
    click(&mut app, &ctx, size, "Logging");
    click(&mut app, &ctx, size, "New folder name");
    let next = Path::new(&app.capture_path);
    assert_eq!(next.parent(), path.parent());
    assert_ne!(next, path);
    assert!(!next.exists());
    click(&mut app, &ctx, size, "Verify latest recording");
    assert_eq!(app.tab, Tab::Offline);
    assert_eq!(app.workspace.offline, OfflinePage::Captures);
    assert_eq!(app.saved_capture.page, saved_capture::Page::Verify);
    assert!(!joined.exists());
    click(&mut app, &ctx, size, "Signal");
    while app.pending_jobs() > 0 && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    let found = draw(&mut app, &ctx, size);
    assert!(found.iter().any(|(text, _)| text == "Review field"));
    assert!(
        found
            .iter()
            .any(|(text, _)| text == "First logged field · switch")
    );
    assert!(
        !found
            .iter()
            .any(|(text, _)| text == "Verify capture" || text == "Verify bundle")
    );
    assert!(
        !found
            .iter()
            .any(|(text, _)| text.starts_with("CSV data matches"))
    );
    click(&mut app, &ctx, size, "First logged field · switch");
    click(&mut app, &ctx, size, "level · mm");
    click(&mut app, &ctx, size, "Review field");
    while app.pending_jobs() > 0 && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    let found = draw(&mut app, &ctx, size);
    assert!(found.iter().any(|(text, _)| text.contains("finite values")));
    assert!(found.iter().any(|(text, _)| text == "level · mm"));
    click(&mut app, &ctx, size, "Verify");
    let found = draw(&mut app, &ctx, size);
    assert!(found.iter().any(|(text, _)| text.starts_with(&format!("{rows} rows checked across {parts} parts"))));
    assert!(
        found
            .iter()
            .any(|(text, _)| text == "CSV data matches the saved report")
    );
    assert!(!found.iter().any(|(text, _)| text == "Review field"));
    click(&mut app, &ctx, size, "Choose a signal");
    assert_eq!(app.saved_capture.page, saved_capture::Page::Signal);
    assert_eq!(app.pending_jobs(), 0);
    assert!(
        !joined.exists(),
        "Verification must not publish a pending export"
    );
    click(&mut app, &ctx, size, "Exports");
    let manifest_before = fs::read(path.join("capture.toml"))?;
    click(&mut app, &ctx, size, "Verify and save combined CSV");
    while app.pending_jobs() > 0 && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    let mut csv = csv::Reader::from_path(&joined)?;
    let fields = csv.headers()?;
    assert_eq!(fields.get(fields.len() - 2), Some("value.switch"));
    assert_eq!(fields.get(fields.len() - 1), Some("value.level [mm]"));
    assert_eq!(
        csv.records().collect::<csv::Result<Vec<_>>>()?.len() as u64,
        rows
    );
    assert_eq!(fs::read(path.join("capture.toml"))?, manifest_before);
    let exported = destination.path().join("redecoded");
    paste(
        &mut app,
        &ctx,
        size,
        "New folder outside the source capture",
        &exported.display().to_string(),
    );
    click(&mut app, &ctx, size, "Export re-decoded CSV");
    click(&mut app, &ctx, size, "Compare captures");
    while app.pending_jobs() > 0 && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(exported.join("receipt.toml").is_file());
    click(&mut app, &ctx, size, "Exports");
    click(
        &mut app,
        &ctx,
        size,
        "Also reverify the original Capture folder above",
    );
    drop_paths(&mut app, &ctx, size, &[&exported.join("receipt.toml")]);
    assert_eq!(app.saved_capture.page, saved_capture::Page::Exports);
    assert_eq!(app.pending_jobs(), 0);
    click(&mut app, &ctx, size, "Verify bundle");
    while app.pending_jobs() > 0 && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    let found = draw(&mut app, &ctx, size);
    assert!(
        found
            .iter()
            .any(|(text, _)| text.contains("Original capture not reverified"))
    );
    assert!(
        found
            .iter()
            .any(|(text, _)| text.contains("local file hashes checked"))
    );
    click(&mut app, &ctx, size, "Signal");
    let found = draw(&mut app, &ctx, size);
    assert!(found.iter().any(|(text, _)| text.contains("finite values")));
    assert_eq!(app.pending_jobs(), 0);
    drop_paths(
        &mut app,
        &ctx,
        size,
        &[&destination.path().join("different-capture/capture.toml")],
    );
    assert_eq!(app.saved_capture.page, saved_capture::Page::Verify);
    let found = draw(&mut app, &ctx, size);
    assert!(
        !found
            .iter()
            .any(|(text, _)| text == "CSV data matches the saved report")
    );
    click(&mut app, &ctx, size, "Signal");
    assert!(
        !draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text.contains("finite values"))
    );
    assert!(
        draw(&mut app, &ctx, size)
            .iter()
            .any(|(text, _)| text == "Empty = first logged field")
    );
    assert_eq!(app.pending_jobs(), 0);
    Ok(())
}

#[test]
fn compare_shortcuts_stage_two_recordings_and_invalidate_previous_results() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let scratch = tempfile::tempdir()?;
    let record = |folder, name: &str| -> Result<Recording> {
        let directory = scratch.path().join(folder);
        let mut profile = Profile::parse(include_str!("../../../samples/simulator.toml"))?;
        profile.name = name.into();
        let report = capture::run(
            &profile,
            &capture::Options {
                output: Some(directory.clone()),
                duration: Some(Duration::from_millis(50)),
                ..Default::default()
            },
            &Stop::default(),
            None,
        )?;
        assert!(report.success() && report.csv_rows_flushed > 0);
        Ok(Recording {
            directory,
            rows: report.csv_rows_flushed,
            parts: report.csv_parts,
            unconfirmed_rows: report.csv_rows_unconfirmed,
            has_error: false,
        })
    };
    let baseline = record("Review 基準", "Baseline sensor run")?;
    let latest = record("Latest run", "Latest sensor run")?;
    let baseline_file = baseline.directory.join("capture.toml");
    let latest_file = latest.directory.join("capture.toml");
    let mut app = App::new();
    app.tab = Tab::Offline;
    app.latest_recording = Some(latest);
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    let finish = |app: &mut App| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.pending_jobs() > 0 && Instant::now() < deadline {
            draw(app, &ctx, size);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.pending_jobs(), 0);
    };
    let shows = |app: &mut App, start: &str| {
        draw(app, &ctx, size)
            .iter()
            .any(|(text, _)| text.starts_with(start))
    };

    click(&mut app, &ctx, size, "Compare captures");
    paste(
        &mut app,
        &ctx,
        size,
        "Comparison capture folder",
        baseline.directory.to_str().unwrap(),
    );
    click(&mut app, &ctx, size, "Verify");
    drop_paths(&mut app, &ctx, size, &[&baseline_file]);
    click(&mut app, &ctx, size, "Verify capture");
    finish(&mut app);
    click(&mut app, &ctx, size, "Compare as baseline");
    assert_eq!(app.saved_capture.page, saved_capture::Page::Compare);
    assert_eq!(app.pending_jobs(), 0);
    assert!(!shows(&mut app, "Verify latest recording"));
    click(&mut app, &ctx, size, "Verify and compare");
    finish(&mut app);
    assert!(shows(&mut app, "Left · Baseline sensor run"));
    assert!(shows(&mut app, "Right · Baseline sensor run"));

    click(&mut app, &ctx, size, "Use latest recording");
    assert_eq!(app.pending_jobs(), 0);
    assert!(!shows(&mut app, "Right ·"));
    click(&mut app, &ctx, size, "Verify and compare");
    click(&mut app, &ctx, size, "Verify");
    finish(&mut app);
    click(&mut app, &ctx, size, "Compare captures");
    assert!(shows(&mut app, "Left · Baseline sensor run"));
    assert!(shows(&mut app, "Right · Latest sensor run"));

    drop_paths(&mut app, &ctx, size, &[&baseline_file]);
    assert_eq!(app.saved_capture.page, saved_capture::Page::Compare);
    assert!(shows(&mut app, "Assign dropped capture"));
    assert!(shows(&mut app, "Right · Latest sensor run"));
    click(&mut app, &ctx, size, "Verify and compare");
    assert_eq!(app.pending_jobs(), 0);
    click(&mut app, &ctx, size, "Cancel drop");
    assert!(!shows(&mut app, "Assign dropped capture"));
    assert!(shows(&mut app, "Right · Latest sensor run"));

    drop_paths(&mut app, &ctx, size, &[&baseline_file]);
    click(&mut app, &ctx, size, "Use on the right");
    assert!(!shows(&mut app, "Right ·"));
    drop_paths(&mut app, &ctx, size, &[&latest_file]);
    click(&mut app, &ctx, size, "Use as baseline");
    assert_eq!(app.pending_jobs(), 0);
    click(&mut app, &ctx, size, "Verify and compare");
    finish(&mut app);
    assert!(shows(&mut app, "Left · Latest sensor run"));
    assert!(shows(&mut app, "Right · Baseline sensor run"));
    Ok(())
}

#[test]
fn verified_raw_only_capture_offers_redecode_instead_of_an_empty_field_picker() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let path = tempfile::tempdir()?;
    let source = path.path().join("raw-only");
    let profile = Profile::parse(include_str!("../../../samples/raw-only.toml"))?;
    let stop = Stop::default();
    stop.request();
    let report = capture::run(
        &profile,
        &capture::Options {
            output: Some(source.clone()),
            ..Default::default()
        },
        &stop,
        None,
    )?;
    assert!(report.success());
    let mut app = App::new();
    app.tab = Tab::Offline;
    assert!(app.saved_capture.select_directory(&source));
    let ctx = Context::default();
    let size = egui::vec2(790.0, 530.0);
    click(&mut app, &ctx, size, "Verify capture");
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.pending_jobs() > 0 && Instant::now() < deadline {
        draw(&mut app, &ctx, size);
        std::thread::sleep(Duration::from_millis(5));
    }
    let found = draw(&mut app, &ctx, size);
    assert!(
        found
            .iter()
            .any(|(text, _)| text == "No typed CSV fields were recorded.")
    );
    click(&mut app, &ctx, size, "Signal");
    let found = draw(&mut app, &ctx, size);
    assert!(
        found
            .iter()
            .any(|(text, _)| text == "This capture contains no typed CSV fields.")
    );
    assert!(!found.iter().any(|(text, _)| text == "Review field"));
    click(&mut app, &ctx, size, "Open re-decode options");
    assert_eq!(app.saved_capture.page, saved_capture::Page::Exports);
    assert_eq!(app.pending_jobs(), 0);
    Ok(())
}
