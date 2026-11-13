use eframe::egui::{self, Color32, Context, RichText, Ui, ViewportCommand};
use iolink_workbench::{
    capture::{self, Live, LiveQueue, Options, Report, Stop},
    decode, iodd,
    profile::{Gateway, LogMode, Profile},
    transport::{self, Freshness, Validity},
};
use std::{
    fs, io,
    path::Path,
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

mod diagnostics;
mod logger_metrics;
mod logging_fields;
mod parameters;
mod saved_capture;
pub mod self_test;
mod setup;

const ACCENT: Color32 = Color32::from_rgb(111, 219, 202);
const WARNING: Color32 = Color32::from_rgb(253, 196, 111);

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1130.0, 790.0])
            .with_min_inner_size([790.0, 530.0]),
        ..Default::default()
    };
    eframe::run_native(
        "IO-Link Workbench",
        options,
        Box::new(|cc| {
            configure_context(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}

fn configure_context(ctx: &Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = Color32::from_rgb(22, 28, 35);
    style.visuals.selection.bg_fill = Color32::from_rgb(43, 89, 88);
    ctx.set_style_of(egui::Theme::Dark, style);
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Setup,
    Monitor,
    Offline,
}

type Worker<T> = mpsc::Receiver<anyhow::Result<T>>;

fn background<T: Send + 'static>(
    name: &str,
    job: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> io::Result<Worker<T>> {
    if self_test::block_background() {
        return Err(io::Error::other(
            "background workers are disabled during the GUI self-test",
        ));
    }
    let (sender, receiver) = mpsc::channel();
    thread::Builder::new().name(name.into()).spawn(move || {
        let _ = sender.send(job());
    })?;
    Ok(receiver)
}

fn completed<T>(worker: &mut Option<Worker<T>>) -> Option<anyhow::Result<T>> {
    let result = match worker.as_ref()?.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return None,
        Err(mpsc::TryRecvError::Disconnected) => {
            Err(anyhow::anyhow!("Background worker stopped unexpectedly"))
        }
    };
    *worker = None;
    Some(result)
}

struct App {
    tab: Tab,
    profile: Profile,
    setup: setup::Editor,
    capture_path: String,
    duration: u32,
    simulate_faults: bool,
    api_enabled: bool,
    api_port: u16,
    mqtt_enabled: bool,
    mqtt_broker: String,
    mqtt_topic: String,
    mqtt_interval_ms: u64,
    worker: Option<Worker<Report>>,
    isdu_worker: Option<Worker<String>>,
    isdu_index: u16,
    isdu_subindex: u8,
    isdu_result: String,
    queue: LiveQueue,
    live: Live,
    report: Option<Report>,
    stop: Stop,
    closing: bool,
    message: String,
    offline_hex: String,
    saved_capture: saved_capture::Verifier,
    selected_parameter: Option<String>,
    selected_subindex: u8,
    offline_parameter: parameters::Offline,
    parameter_snapshot_path: String,
    saved_parameter: parameters::Saved,
    offline_diagnostics: diagnostics::Offline,
}

impl App {
    fn new() -> Self {
        let profile =
            Profile::parse(include_str!("../samples/simulator.toml")).expect("bundled profile");
        let setup = setup::Editor::new(&profile);
        Self {
            tab: Tab::Setup,
            profile,
            setup,
            capture_path: default_capture_path(),
            duration: 10,
            simulate_faults: false,
            api_enabled: false,
            api_port: 8765,
            mqtt_enabled: false,
            mqtt_broker: "127.0.0.1:1883".into(),
            mqtt_topic: "iolink-workbench/input".into(),
            mqtt_interval_ms: 100,
            worker: None,
            isdu_worker: None,
            isdu_index: 19,
            isdu_subindex: 0,
            isdu_result: String::new(),
            queue: Arc::new(crossbeam_queue::ArrayQueue::new(1)),
            live: Live::default(),
            report: None,
            stop: Stop::default(),
            closing: false,
            message: String::new(),
            offline_hex: "FFF9".into(),
            saved_capture: saved_capture::Verifier::default(),
            selected_parameter: None,
            selected_subindex: 0,
            offline_parameter: parameters::Offline::default(),
            parameter_snapshot_path: ".local/parameter.json".into(),
            saved_parameter: parameters::Saved::default(),
            offline_diagnostics: diagnostics::Offline::default(),
        }
    }

    fn start(&mut self, record: bool) -> anyhow::Result<()> {
        self.profile.validate()?;
        let output = if record {
            anyhow::ensure!(
                !self.capture_path.trim().is_empty(),
                "Enter a new capture folder"
            );
            let path = Path::new(self.capture_path.trim());
            if path.parent() == Some(Path::new(".local")) {
                fs::create_dir_all(".local")?;
            }
            anyhow::ensure!(
                !path.exists(),
                "Capture folder already exists. Enter a new folder."
            );
            Some(path.to_path_buf())
        } else {
            None
        };
        let profile = self.profile.clone();
        let mqtt = if self.mqtt_enabled {
            Some(iolink_workbench::mqtt::Config {
                broker: self
                    .mqtt_broker
                    .trim()
                    .parse()
                    .map_err(|_| anyhow::anyhow!("Enter a numeric MQTT broker IP:PORT"))?,
                topic: self.mqtt_topic.clone(),
                interval_ms: self.mqtt_interval_ms,
                ..Default::default()
            })
        } else {
            None
        };
        let options = Options {
            mqtt,
            output,
            duration: (record && self.duration != 0)
                .then(|| Duration::from_secs(u64::from(self.duration))),
            simulate_faults: profile.connection.gateway == Gateway::Simulator
                && self.simulate_faults,
            local_api: self
                .api_enabled
                .then(|| iolink_workbench::local_api::Config {
                    listen: std::net::SocketAddrV4::new(
                        std::net::Ipv4Addr::LOCALHOST,
                        self.api_port,
                    )
                    .into(),
                    ..Default::default()
                }),
            ..Options::default()
        };
        self.stop = Stop::default();
        self.report = None;
        self.live = Live::default();
        let stop = self.stop.clone();
        let queue = self.queue.clone();
        self.worker = Some(background("cyclic-input", move || {
            capture::run(&profile, &options, &stop, Some(&queue))
        })?);
        self.message = if record {
            "Starting capture…"
        } else {
            "Connecting…"
        }
        .into();
        Ok(())
    }

    fn update_workers(&mut self, ctx: &Context) {
        self.saved_capture.update(ctx);
        self.setup.update(ctx, &mut self.message);
        if let Some(live) = self.queue.pop() {
            self.live = live;
            if self.live.connected {
                match self.message.as_str() {
                    "Connecting…" => self.message = "Monitoring".into(),
                    "Starting capture…" => self.message = "Capturing".into(),
                    _ => {}
                }
            }
        }
        if let Some(result) = completed(&mut self.worker) {
            if let Some(live) = self.queue.pop() {
                self.live = live;
            }
            match result {
                Ok(report) => {
                    self.message = report
                        .terminal_error
                        .as_ref()
                        .or(report.writer_error.as_ref())
                        .cloned()
                        .unwrap_or_else(|| {
                            format!("Stopped · {} CSV rows flushed", report.csv_rows_flushed)
                        });
                    self.report = Some(report);
                }
                Err(error) => self.message = format!("{error:#}"),
            }
        }
        if let Some(result) = completed(&mut self.isdu_worker) {
            self.isdu_result = match result {
                Ok(value) => value,
                Err(error) => format!("{error:#}"),
            };
        }
        if self.worker.is_some() {
            if ctx.input(|input| input.viewport().close_requested()) {
                self.closing = true;
                self.stop.request();
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
        } else if self.closing {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if self.worker.is_some() || self.isdu_worker.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    fn monitor(&mut self, ui: &mut Ui) {
        ui.heading("Process input and capture");
        let running = self.worker.is_some();
        let available = !running && !self.closing && !self.setup.pending() && !self.setup.busy();
        ui.add_enabled_ui(available, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.mqtt_enabled, "MQTT latest-state publishing");
                ui.label("Broker IP:port");
                ui.text_edit_singleline(&mut self.mqtt_broker);
            });
            if self.mqtt_enabled {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Topic");
                    ui.text_edit_singleline(&mut self.mqtt_topic);
                    ui.add(
                        egui::DragValue::new(&mut self.mqtt_interval_ms)
                            .range(10..=60000)
                            .suffix(" ms"),
                    );
                    ui.small("TCP · QoS 0 · not retained · no TLS/auth");
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.api_enabled, "Read-only local API");
                ui.add_enabled(
                    self.api_enabled,
                    egui::DragValue::new(&mut self.api_port)
                        .range(1..=65535)
                        .prefix("127.0.0.1:"),
                );
                ui.small("Opt-in per session · status, schema, bounded history");
            });
            ui.horizontal_wrapped(|ui| {
                path_entry(ui, &mut self.capture_path, "New capture folder");
                if ui.small_button("New folder name").clicked() {
                    self.capture_path = default_capture_path();
                }
                ui.add(
                    egui::DragValue::new(&mut self.duration)
                        .range(0..=86_400)
                        .suffix(" s"),
                );
                ui.small("0 = manual stop");
            });
            ui.horizontal_wrapped(|ui| {
                let mut logging = self.profile.logging.clone();
                ui.selectable_value(
                    &mut logging.mode,
                    LogMode::EveryUpdate,
                    "Every update / poll",
                );
                ui.selectable_value(&mut logging.mode, LogMode::OnChange, "On change");
                ui.add(
                    egui::DragValue::new(&mut logging.queue_capacity)
                        .range(1..=1_000_000)
                        .prefix("Queue "),
                );
                ui.add(egui::DragValue::new(&mut logging.rotate_rows).prefix("Rotate rows "));
                ui.add(egui::DragValue::new(&mut logging.rotate_seconds).prefix("Rotate seconds "));
                if logging != self.profile.logging {
                    match self.profile.with_logging(logging) {
                        Ok(candidate) => {
                            self.profile = candidate;
                            self.setup.sync(&self.profile);
                        }
                        Err(error) => self.message = format!("{error:#}"),
                    }
                }
            });
            if logging_fields::show(ui, &mut self.profile, &mut self.message) {
                self.setup.sync(&self.profile);
            }
            if available {
                ui.collapsing("Configured capture plan (offline)", |ui| {
                    match self.profile.logging_plan() {
                        Ok(plan) => {
                            ui.label(plan.text());
                        }
                        Err(error) => {
                            ui.colored_label(WARNING, format!("{error:#}"));
                        }
                    }
                });
            }
            if self.profile.connection.gateway == Gateway::Simulator {
                ui.checkbox(
                    &mut self.simulate_faults,
                    "Simulate stale input, invalid input, and a disconnect",
                );
            }
        });
        if self.setup.pending() {
            ui.colored_label(WARNING, "Apply or discard pending profile edits in Setup.");
        }
        if running && self.api_enabled {
            ui.label(format!("Local API requested at http://127.0.0.1:{}/v1/status · API drops are separate from CSV/source loss", self.api_port));
        }
        if running && self.mqtt_enabled {
            ui.label(format!(
                "MQTT requested at {} · {} · latest-state; broker delivery unknown",
                self.mqtt_broker, self.mqtt_topic
            ));
        }
        if let Some(mqtt) = self.report.as_ref().and_then(|report| report.mqtt.as_ref()) {
            ui.label(format!("MQTT stopped · {} socket writes · {} coalesced · {} queue drops · {} write failures · worker failed: {}",mqtt.socket_writes_completed,mqtt.coalesced,mqtt.queue_dropped,mqtt.publish_write_failures,mqtt.worker_failed));
        }
        if let Some(api) = self
            .report
            .as_ref()
            .and_then(|report| report.local_api.as_ref())
        {
            ui.label(format!("Local API stopped · {} events ingested · {} queue drops · {} history evictions · worker failed: {}", api.events_ingested, api.queue_dropped, api.history_evicted, api.worker_failed));
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(available, egui::Button::new("Start monitor"))
                .clicked()
                && let Err(error) = self.start(false)
            {
                self.message = format!("{error:#}");
            }
            if ui
                .add_enabled(
                    available,
                    egui::Button::new(RichText::new("Start CSV capture").color(ACCENT)),
                )
                .clicked()
                && let Err(error) = self.start(true)
            {
                self.message = format!("{error:#}");
            }
            if ui
                .add_enabled(
                    running && !self.stop.requested(),
                    egui::Button::new("Stop and flush"),
                )
                .clicked()
            {
                self.stop.request();
                self.message = "Stopping and flushing…".into();
            }
        });
        ui.separator();
        if let Some(status) = &self.live.status {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(&status.model).strong());
                ui.label(format!(
                    "Firmware {} · Port {} · mode {}",
                    status.firmware, status.port, status.port_mode
                ));
                if let Some(serial) = &status.master_serial {
                    ui.small(format!("Master S/N {serial}"));
                }
                ui.colored_label(ACCENT, status.route);
            });
            ui.label(format!(
                "{} · Generation {} · {}",
                if running && self.live.connected {
                    "Connected"
                } else {
                    "Disconnected"
                },
                self.live.generation,
                self.live.identity_check.unwrap_or("identity unchecked")
            ));
            let identity = &status.identity;
            ui.label(format!(
                "Vendor {} · Device {} · Product ID {} · {} · Device S/N {}",
                optional_number(identity.vendor_id),
                optional_number(identity.device_id),
                identity.product_id.as_deref().unwrap_or("unavailable"),
                identity
                    .product_name
                    .as_deref()
                    .unwrap_or("product name unavailable"),
                identity.serial.as_deref().unwrap_or("unavailable")
            ));
            ui.label(format!(
                "Connection-time gateway snapshots · status {} · port event {} · device event {}",
                status.device_status.as_deref().unwrap_or("unavailable"),
                status.port_event.as_deref().unwrap_or("unavailable"),
                status.device_event.as_deref().unwrap_or("unavailable")
            ));
            ui.label(
                match (status.port_cycle_us, status.port_cycle_raw.as_deref()) {
                    (Some(us), _) => format!("Master-reported IO-Link port cycle · {us} µs"),
                    (None, Some(raw)) => {
                        format!("Master-reported IO-Link port cycle · raw {raw}, confirm unit")
                    }
                    _ => "Master-reported IO-Link port cycle · unavailable".into(),
                },
            );
            if status.freshness == Freshness::Snapshots {
                ui.colored_label(
                    WARNING,
                    "Host polls · source-update loss and input age are unknown",
                );
            }
            if let Ok(context) = iolink_workbench::timing::assess(
                &self.profile,
                Some(self.profile.connection.poll_us),
                status.into(),
                1000,
            ) {
                ui.collapsing("Cycle target and declared limits", |ui| {
                    ui.label(context.text());
                });
            }
        }
        if running && !self.live.message.is_empty() {
            ui.label(&self.live.message);
        }
        if let Some(input) = self.live.input {
            let host_age = input.received.elapsed();
            let source_age = input.source_age_us;
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "Validity {} · last received {} ms ago",
                    input.validity.label(),
                    host_age.as_millis()
                ));
                if let Some(age) = source_age {
                    ui.label(format!(
                        "Input age at receipt {:.2} ms",
                        age as f64 / 1000.0
                    ));
                } else if let Some(last) = self.live.last_new_at {
                    ui.label(format!(
                        "Time since last observed update {} ms",
                        input.received.saturating_duration_since(last).as_millis()
                    ));
                } else {
                    ui.label("Input age unknown");
                }
                if let Some(code) = input.diagnostic_code {
                    ui.label(format!("Gateway code {code}"));
                }
            });
            let stale = self.live.stale == Some(true);
            if stale {
                ui.colored_label(WARNING, "Stale input");
            }
            payload_view(
                ui,
                &self.profile,
                input.payload.bytes(),
                input.validity == Validity::Valid && !stale,
            );
        }
        let counts = self.live.counts;
        ui.label(format!(
            "{} responses · {} new · {} stale · {} invalid · {} overruns · {} reconnects",
            counts.received,
            counts.new_updates_observed,
            counts.stale,
            counts.invalid,
            counts.logger_overruns,
            counts.reconnects
        ));
        if let Some(report) = &self.report {
            ui.separator();
            ui.heading("Run timing");
            for (label, stats) in [
                (
                    "Host poll start spacing",
                    report.host_poll_start_spacing.as_ref(),
                ),
                ("Host receive spacing", report.host_receive_spacing.as_ref()),
                (
                    "Host observed new update spacing",
                    report.host_new_update_spacing.as_ref(),
                ),
                (
                    "Gateway update spacing",
                    report.gateway_update_spacing.as_ref(),
                ),
            ] {
                ui.label(match stats {
                    Some(stats) => format!(
                        "{label} · p50 {} / p95 {} / p99 {} / longest {} µs · {:.1} Hz",
                        stats.p50_us, stats.p95_us, stats.p99_us, stats.longest_us, stats.mean_hz
                    ),
                    None => format!("{label} · unavailable"),
                });
            }
            ui.label(format!(
                "Source-update loss · {}",
                report.source_update_loss
            ));
            if let Some(gaps) = report.source_gaps_observed {
                ui.label(format!("Observed missing source updates · {gaps}"));
            }
            ui.label(format!(
                "{} rows flushed across {} CSV parts",
                report.csv_rows_flushed, report.csv_parts
            ));
            logger_metrics::show(ui, report.logger_metrics.as_ref());
            if report.csv_rows_unconfirmed != 0 {
                ui.colored_label(
                    WARNING,
                    format!(
                        "{} queued rows have an unconfirmed CSV write",
                        report.csv_rows_unconfirmed
                    ),
                );
            }
        }
        ui.separator();
        ui.collapsing("Device diagnostics (explicit ISDU reads)", |ui| {
            ui.small("Reads optional 36:0 and 37:0 on independent acyclic clients. These snapshots provide no continuous event history or source-loss count.");
            let available = self.isdu_worker.is_none() && !self.setup.pending() && !self.setup.busy();
            if ui.add_enabled(available, egui::Button::new("Read device diagnostics")).clicked() {
                let profile = self.profile.clone();
                match background("acyclic-device-diagnostics", move || Ok(iolink_workbench::diagnostics::read(&profile)?.text())) {
                    Ok(worker) => { self.isdu_worker = Some(worker); self.isdu_result = "Reading device diagnostics…".into(); }
                    Err(error) => self.isdu_result = error.to_string(),
                }
            }
            if let Some(imported) = &self.profile.iodd {
                ui.small(format!("{} saved event definitions · {} unresolved standard event codes", imported.events.len(), imported.standard_event_refs.len()));
                for event in &imported.events { ui.label(format!("0x{:04X} · {:?} · {}", event.code, event.event_type, event.name)); }
            }
        });
        let parameter_actions = parameters::show(
            ui,
            &self.profile,
            &mut self.selected_parameter,
            &mut self.selected_subindex,
            self.isdu_worker.is_none() && !self.setup.pending() && !self.setup.busy(),
            self.worker.is_none(),
            &mut self.parameter_snapshot_path,
        );
        if let Some(selection) = parameter_actions.display {
            self.select_parameter_display(selection);
        }
        let request = parameter_actions
            .read
            .map(|(id, subindex)| (id, subindex, None))
            .or_else(|| {
                parameter_actions
                    .snapshot
                    .map(|(id, subindex, path)| (id, subindex, Some(path)))
            });
        if let Some((id, subindex, snapshot_path)) = request {
            let profile = self.profile.clone();
            match background("acyclic-iodd-parameter", move || {
                let (reading, save_message) = if let Some(path) = snapshot_path {
                    let path = Path::new(&path);
                    iolink_workbench::parameter_snapshot::check_destination(path)?;
                    let (snapshot, reading) =
                        iolink_workbench::parameter_snapshot::read(&profile, &id, subindex)?;
                    let message = match snapshot.save(path) {
                        Ok(()) => format!("\nSaved snapshot {}", path.display()),
                        Err(error) => format!("\nSnapshot save failed: {error}"),
                    };
                    (reading, message)
                } else {
                    (
                        transport::read_parameter_at(&profile, &id, subindex)?,
                        String::new(),
                    )
                };
                let mut text = format!(
                    "{} · index {}:{}\nRaw bytes {}",
                    reading.name, reading.index, reading.subindex, reading.raw_hex
                );
                if let Some(value) = &reading.raw_value {
                    text.push_str(&format!("\nRaw value {value}"));
                }
                if let Some(name) = &reading.choice_name {
                    text.push_str(&format!(" · {name}"));
                }
                if let Some(time) = &reading.time_value {
                    text.push_str(&format!("\n{}", time.text()));
                }
                if let Some(assessment) = &reading.value_assessment {
                    text.push_str(&format!("\n{}", assessment.text()));
                }
                if let Some(error) = &reading.decode_error {
                    text.push_str(&format!("\nDecode review · {error}"));
                }
                for display in &reading.displays {
                    text.push_str(&format!(
                        "\nDisplay {}{} · {} {}",
                        display.id,
                        if reading
                            .preferred_display
                            .as_ref()
                            .is_some_and(|preferred| preferred.id == display.id)
                        {
                            " (preferred)"
                        } else {
                            ""
                        },
                        display.value,
                        display.unit
                    ));
                }
                for item in reading.record_items.iter().chain(&reading.array_items) {
                    text.push_str(&format!(
                        "\n\n{} · subindex {} · bit {}\n{}",
                        item.name,
                        item.subindex,
                        item.bit_offset,
                        item.value.text()
                    ));
                }
                text.push_str(&format!(
                    "\nHost read {} · query {} µs",
                    reading.host_read_utc, reading.read_duration_us
                ));
                text.push_str(&save_message);
                Ok(text)
            }) {
                Ok(worker) => {
                    self.isdu_worker = Some(worker);
                    self.isdu_result = "Reading parameter…".into();
                }
                Err(error) => self.isdu_result = error.to_string(),
            }
        }
        ui.collapsing("Acyclic ISDU read", |ui| {
            ui.small("One parameter read on a separate client. Use an index and subindex from the device documentation.");
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::DragValue::new(&mut self.isdu_index).prefix("Index "));
                ui.add(egui::DragValue::new(&mut self.isdu_subindex).prefix("Subindex "));
                if ui.add_enabled(self.isdu_worker.is_none() && !self.setup.pending() && !self.setup.busy(), egui::Button::new("Read parameter")).clicked() {
                    let (profile, index, subindex) = (self.profile.clone(), self.isdu_index, self.isdu_subindex);
                    match background("acyclic-isdu", move || { let bytes = transport::read_isdu(&profile, index, subindex)?;
                        let hex = decode::hex(&bytes);
                        Ok(match std::str::from_utf8(&bytes) {
                            Ok(text) => format!("{hex}  ·  {text:?}"),
                            Err(_) => hex,
                        })
                    }) {
                        Ok(worker) => { self.isdu_worker = Some(worker); self.isdu_result = "Reading…".into(); }
                        Err(error) => self.isdu_result = error.to_string(),
                    }
                }
                let can_verify = self.isdu_worker.is_none() && !self.setup.pending() && !self.setup.busy()
                    && self.profile.expected_identity.product_id.is_some();
                if ui.add_enabled(can_verify, egui::Button::new("Verify Product ID")).clicked() {
                    let profile = self.profile.clone();
                    match background("acyclic-product-id", move || {
                        let product_id = transport::verify_product_id(&profile)?;
                        Ok(format!("Matched Product ID {product_id}"))
                    }) {
                        Ok(worker) => { self.isdu_worker = Some(worker); self.isdu_result = "Reading Product ID…".into(); }
                        Err(error) => self.isdu_result = error.to_string(),
                    }
                }
            });
        });
        if !self.isdu_result.is_empty() {
            ui.monospace(&self.isdu_result);
        }
    }

    fn select_parameter_display(&mut self, selection: parameters::DisplaySelection) {
        match selection.apply(&mut self.profile) {
            Ok(()) => {
                self.setup.sync(&self.profile);
                self.isdu_result =
                    "Display preference updated. Read again to refresh the result.".into();
                self.message =
                    "Display preference updated; save the profile in Setup to retain it.".into();
            }
            Err(error) => self.message = error.to_string(),
        }
    }

    fn offline(&mut self, ui: &mut Ui) {
        ui.heading("Offline payload decoder");
        ui.label(format!(
            "{} · {} expected bytes",
            self.profile.name, self.profile.input_bytes
        ));
        ui.add(
            egui::TextEdit::multiline(&mut self.offline_hex)
                .code_editor()
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        match decode::parse_hex(&self.offline_hex) {
            Ok(payload) => payload_view(ui, &self.profile, &payload, true),
            Err(error) => {
                ui.colored_label(WARNING, error.to_string());
            }
        }
        ui.separator();
        if let Some(selection) = self.offline_parameter.show(
            ui,
            &self.profile,
            self.worker.is_none()
                && self.isdu_worker.is_none()
                && !self.setup.pending()
                && !self.setup.busy(),
        ) {
            self.select_parameter_display(selection);
        }
        ui.separator();
        self.saved_parameter.show(ui);
        ui.separator();
        self.offline_diagnostics.show(ui, &self.profile);
        ui.separator();
        self.saved_capture.show(
            ui,
            &self.profile,
            !self.setup.pending() && !self.setup.busy(),
        );
    }
}

impl App {
    fn render(&mut self, ctx: &Context) {
        self.update_workers(ctx);
        egui::TopBottomPanel::top("navigation").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("IO-Link Workbench").size(24.0).strong());
                ui.colored_label(ACCENT, "READ ONLY");
                ui.separator();
                ui.selectable_value(&mut self.tab, Tab::Setup, "Setup");
                ui.selectable_value(&mut self.tab, Tab::Monitor, "Monitor & CSV");
                ui.selectable_value(&mut self.tab, Tab::Offline, "Offline decode");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(if self.worker.is_some() {
                        "SESSION ACTIVE"
                    } else {
                        "OFFLINE"
                    });
                });
            });
            ui.add_space(5.0);
        });
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ACCENT, &self.profile.name);
                if !self.message.is_empty() {
                    ui.separator();
                    ui.label(&self.message);
                }
            });
            ui.add_space(4.0);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(self.tab as u8)
                .show(ui, |ui| {
                    ui.add_space(5.0);
                    match self.tab {
                        Tab::Setup => {
                            let editable = self.worker.is_none()
                                && self.isdu_worker.is_none()
                                && !self.setup.busy();
                            if self
                                .setup
                                .show(ui, &mut self.profile, &mut self.message, editable)
                            {
                                self.live = Live::default();
                                self.report = None;
                                self.selected_parameter = None;
                                self.selected_subindex = 0;
                                self.offline_parameter = parameters::Offline::default();
                                self.offline_diagnostics = diagnostics::Offline::default();
                                self.isdu_result.clear();
                            }
                        }
                        Tab::Monitor => self.monitor(ui),
                        Tab::Offline => self.offline(ui),
                    }
                });
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        self.render(ctx);
    }
}

fn default_capture_path() -> String {
    format!(
        ".local/capture-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f")
    )
}

fn path_entry(ui: &mut Ui, value: &mut String, hint: &str) {
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(350.0),
    );
}

fn optional_number(value: Option<impl ToString>) -> String {
    value
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unavailable".into())
}

fn payload_view(ui: &mut Ui, profile: &Profile, payload: &[u8], decode_values: bool) {
    if !payload.is_empty() {
        ui.monospace(format!("RAW  {}", decode::hex(payload)));
    }
    if !decode_values {
        return;
    }
    match profile.decode(payload) {
        Ok(rows) => {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                egui::Grid::new("typed-input").striped(true).show(ui, |ui| {
                    for heading in [
                        "Field",
                        "Origin",
                        "Bit / width",
                        "Type",
                        "Order",
                        "Scaling",
                        "Raw",
                        "Value",
                        "Unit",
                    ] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for (field, origin, value) in rows {
                        ui.label(&field.name);
                        ui.label(origin);
                        ui.monospace(format!("{} / {}", field.bit_offset, field.bits));
                        ui.label(format!("{:?}", field.kind));
                        ui.label(match field.byte_order {
                            decode::ByteOrder::BigEndian => "BE",
                            decode::ByteOrder::LittleEndian => "LE",
                        });
                        ui.monospace(format!("×{} + {}", field.scale, field.offset));
                        ui.monospace(format!("{} · 0x{}", value.raw, value.bits_hex));
                        ui.colored_label(ACCENT, value.value);
                        ui.label(&field.unit);
                        ui.end_row();
                    }
                });
            });
        }
        Err(error) => {
            ui.colored_label(WARNING, error.to_string());
        }
    }
}
