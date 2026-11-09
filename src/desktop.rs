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
            cc.egui_ctx.set_theme(egui::Theme::Dark);
            let mut style = (*cc.egui_ctx.style_of(egui::Theme::Dark)).clone();
            style.spacing.item_spacing = egui::vec2(10.0, 8.0);
            style.visuals = egui::Visuals::dark();
            style.visuals.panel_fill = Color32::from_rgb(22, 28, 35);
            style.visuals.selection.bg_fill = Color32::from_rgb(43, 89, 88);
            cc.egui_ctx.set_style_of(egui::Theme::Dark, style);
            Ok(Box::new(App::new()))
        }),
    )
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
        let options = Options {
            output,
            duration: (record && self.duration != 0)
                .then(|| Duration::from_secs(u64::from(self.duration))),
            simulate_faults: profile.connection.gateway == Gateway::Simulator
                && self.simulate_faults,
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
        let available = !running && !self.closing && !self.setup.pending;
        ui.add_enabled_ui(available, |ui| {
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
                let old = self.profile.logging.clone();
                ui.selectable_value(
                    &mut self.profile.logging.mode,
                    LogMode::EveryUpdate,
                    "Every update / poll",
                );
                ui.selectable_value(
                    &mut self.profile.logging.mode,
                    LogMode::OnChange,
                    "On change",
                );
                ui.add(
                    egui::DragValue::new(&mut self.profile.logging.queue_capacity)
                        .range(1..=1_000_000)
                        .prefix("Queue "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.profile.logging.rotate_rows)
                        .prefix("Rotate rows "),
                );
                ui.add(
                    egui::DragValue::new(&mut self.profile.logging.rotate_seconds)
                        .prefix("Rotate seconds "),
                );
                if old != self.profile.logging {
                    self.setup.sync(&self.profile);
                }
            });
            if self.profile.connection.gateway == Gateway::Simulator {
                ui.checkbox(
                    &mut self.simulate_faults,
                    "Simulate stale input, invalid input, and a disconnect",
                );
            }
        });
        if self.setup.pending {
            ui.colored_label(WARNING, "Apply or discard pending profile edits in Setup.");
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
                "Device status {} · Port event {} · Device event {}",
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
        }
        ui.separator();
        ui.collapsing("Acyclic ISDU read", |ui| {
            ui.small("One parameter read on a separate client. Use an index and subindex from the device documentation.");
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::DragValue::new(&mut self.isdu_index).prefix("Index "));
                ui.add(egui::DragValue::new(&mut self.isdu_subindex).prefix("Subindex "));
                if ui.add_enabled(self.isdu_worker.is_none() && !self.setup.pending, egui::Button::new("Read parameter")).clicked() {
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
                let can_verify = self.isdu_worker.is_none() && !self.setup.pending
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
                ui.monospace(&self.isdu_result);
            });
        });
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
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _: &mut eframe::Frame) {
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
                            let editable = self.worker.is_none() && self.isdu_worker.is_none();
                            if self
                                .setup
                                .show(ui, &mut self.profile, &mut self.message, editable)
                            {
                                self.live = Live::default();
                                self.report = None;
                            }
                        }
                        Tab::Monitor => self.monitor(ui),
                        Tab::Offline => self.offline(ui),
                    }
                });
        });
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
