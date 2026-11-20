use eframe::egui::{self, Color32, Context, RichText, Ui, ViewportCommand};
use iolink_workbench::{
    capture::{self, Live, LiveQueue, Report, Stop},
    decode, iodd,
    profile::{Gateway, LogMode, Profile},
    transport::{self, Freshness},
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

mod acyclic;
mod capture_destination;
mod diagnostics;
mod file_drop;
mod integration_review;
mod logger_metrics;
mod logging_fields;
mod observation;
mod parameters;
mod payload_inspector;
mod port_capture;
mod saved_capture;
pub mod self_test;
mod session_review;
mod setup;
mod shutdown;
mod workflows;
mod workspace;
mod writes;

#[cfg(test)]
static WORKER_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
type TestCompletion = Box<dyn FnOnce()>;

const ACCENT: Color32 = Color32::from_rgb(111, 219, 202);
const WARNING: Color32 = Color32::from_rgb(253, 196, 111);

pub fn run() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1130.0, 790.0])
            .with_drag_and_drop(true)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Tab {
    Device,
    Inputs,
    Logging,
    Mappings,
    Parameters,
    Integration,
    Diagnostics,
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
    let result = poll_worker(worker.as_ref()?)?;
    *worker = None;
    Some(result)
}

fn poll_worker<T>(worker: &Worker<T>) -> Option<anyhow::Result<T>> {
    Some(match worker.try_recv() {
        Ok(result) => result,
        Err(mpsc::TryRecvError::Empty) => return None,
        Err(mpsc::TryRecvError::Disconnected) => {
            Err(anyhow::anyhow!("Background worker stopped unexpectedly"))
        }
    })
}

struct App {
    tab: Tab,
    workspace: workspace::Workspace,
    active_recording: Option<PathBuf>,
    latest_recording: Option<Recording>,
    profile: Profile,
    setup: setup::Editor,
    capture_path: String,
    capture_folder_check: Option<capture_destination::FolderCheck>,
    port_capture: port_capture::Panel,
    duration: u32,
    simulate_faults: bool,
    api_enabled: bool,
    api_port: u16,
    mqtt_enabled: bool,
    mqtt_broker: String,
    mqtt_topic: String,
    mqtt_interval_ms: u64,
    shared_memory_enabled: bool,
    shared_memory_name: String,
    shared_memory_interval_ms: u64,
    worker: Option<Worker<Report>>,
    acyclic: acyclic::Reads,
    writes: writes::Panel,
    isdu_index: u16,
    isdu_subindex: u8,
    queue: LiveQueue,
    live: Live,
    report: Option<Report>,
    stop: Stop,
    closing: bool,
    message: String,
    payload_inspector: payload_inspector::Inspector,
    saved_capture: saved_capture::Verifier,
    parameter_picker: parameters::Picker,
    offline_parameter: parameters::Offline,
    parameter_snapshot_path: String,
    saved_parameter: parameters::Saved,
    diagnostic_snapshot_path: String,
    offline_diagnostics: diagnostics::Offline,
    saved_diagnostics: diagnostics::Saved,
}

struct Recording {
    directory: PathBuf,
    rows: u64,
    parts: u64,
    unconfirmed_rows: u64,
    has_error: bool,
}

impl App {
    fn new() -> Self {
        let profile =
            Profile::parse(include_str!("../samples/simulator.toml")).expect("bundled profile");
        let setup = setup::Editor::new(&profile);
        Self {
            tab: Tab::Device,
            workspace: workspace::Workspace::default(),
            active_recording: None,
            latest_recording: None,
            profile,
            setup,
            capture_path: default_capture_path(),
            capture_folder_check: None,
            port_capture: port_capture::Panel::default(),
            duration: 10,
            simulate_faults: false,
            api_enabled: false,
            api_port: 8765,
            mqtt_enabled: false,
            mqtt_broker: "127.0.0.1:1883".into(),
            mqtt_topic: "iolink-workbench/input".into(),
            mqtt_interval_ms: 100,
            shared_memory_enabled: false,
            shared_memory_name: "input".into(),
            shared_memory_interval_ms: 10,
            worker: None,
            acyclic: acyclic::Reads::default(),
            writes: writes::Panel::default(),
            isdu_index: 19,
            isdu_subindex: 0,
            queue: Arc::new(crossbeam_queue::ArrayQueue::new(1)),
            live: Live::default(),
            report: None,
            stop: Stop::default(),
            closing: false,
            message: String::new(),
            payload_inspector: payload_inspector::Inspector::default(),
            saved_capture: saved_capture::Verifier::default(),
            parameter_picker: parameters::Picker::default(),
            offline_parameter: parameters::Offline::default(),
            parameter_snapshot_path: ".local/parameter.json".into(),
            saved_parameter: parameters::Saved::default(),
            diagnostic_snapshot_path: ".local/diagnostics.json".into(),
            offline_diagnostics: diagnostics::Offline::default(),
            saved_diagnostics: diagnostics::Saved::default(),
        }
    }

    fn acquisition_active(&self) -> bool {
        self.worker.is_some() || self.port_capture.active()
    }

    fn start_block_reason(&self) -> Option<&'static str> {
        if self.closing {
            Some("Application is closing; new sessions are disabled")
        } else if self.acquisition_active() {
            Some(
                "An acquisition session is already active; wait for its completion before starting another",
            )
        } else if self.writes.writing() {
            Some("A reviewed write is in progress")
        } else if self.setup.pending() {
            Some("Apply or discard pending profile edits before starting a session")
        } else if self.setup.busy() {
            Some("Wait for IODD inspection to complete before starting a session")
        } else {
            None
        }
    }

    fn start(&mut self, record: bool) -> anyhow::Result<()> {
        if let Some(reason) = self.start_block_reason() {
            anyhow::bail!(reason);
        }
        self.profile.validate()?;
        self.check_session_route()
            .map_err(session_review::RouteIssue::into_error)?;
        let mut options = self.review_feeds().into_options()?;
        let destination = record.then(|| self.check_capture_folder()).transpose()?;
        options.output = destination.as_ref().map(|folder| folder.path.clone());
        let profile = self.profile.clone();
        let active_recording = options.output.clone();
        options.duration =
            (record && self.duration != 0).then(|| Duration::from_secs(u64::from(self.duration)));
        options.simulate_faults =
            profile.connection.gateway == Gateway::Simulator && self.simulate_faults;
        let stop = Stop::default();
        let queue = Arc::new(crossbeam_queue::ArrayQueue::new(1));
        let worker_stop = stop.clone();
        let worker_queue = queue.clone();
        let worker = background("cyclic-input", move || {
            if let Some(folder) = destination {
                folder.prepare_parent()?;
            }
            capture::run(&profile, &options, &worker_stop, Some(&worker_queue))
        })?;
        // A failed dispatch leaves the previous evidence and control token intact.
        self.stop = stop;
        self.queue = queue;
        self.report = None;
        self.live = Live::default();
        self.worker = Some(worker);
        self.active_recording = active_recording;
        if record {
            self.capture_folder_check = None;
        }
        self.message = if record {
            "Starting capture…"
        } else {
            "Connecting…"
        }
        .into();
        Ok(())
    }

    fn update_workers(&mut self, ctx: &Context) {
        let mut capture_failed = false;
        self.saved_capture.update(ctx);
        self.saved_parameter.update();
        self.saved_diagnostics.update();
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
            let recording = self.active_recording.take();
            if let Some(live) = self.queue.pop() {
                self.live = live;
            }
            match result {
                Ok(report) => {
                    capture_failed = !report.success();
                    // The recording worker returns a report only after publishing capture.toml.
                    if let Some(directory) = recording {
                        self.latest_recording = Some(Recording {
                            directory,
                            rows: report.csv_rows_flushed,
                            parts: report.csv_parts,
                            unconfirmed_rows: report.csv_rows_unconfirmed,
                            has_error: capture_failed,
                        });
                    }
                    self.message = report
                        .terminal_error
                        .as_ref()
                        .or(report.writer_error.as_ref())
                        .cloned()
                        .unwrap_or_else(|| {
                            if report.success() && report.csv_parts > 0 {
                                format!("Stopped · {} CSV rows flushed", report.csv_rows_flushed)
                            } else if report.success() {
                                "Session stopped".into()
                            } else {
                                "Capture worker failed; review the final report".into()
                            }
                        });
                    self.report = Some(report);
                }
                Err(error) => {
                    capture_failed = true;
                    self.message = format!("{error:#}");
                }
            }
        }
        let batch_outcome = self.port_capture.update();
        if let Some(failed) = batch_outcome {
            capture_failed |= failed;
            self.message = if failed {
                "Port capture set needs review · see Logging → Multiple ports"
            } else {
                "Port capture set saved · see Logging → Multiple ports"
            }
            .into();
        }
        self.acyclic.update();
        let write_unconfirmed = self.writes.update(&mut self.message);
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if batch_outcome == Some(true) && (close_requested || self.closing) {
            self.tab = Tab::Logging;
            self.workspace.logging = workspace::LoggingPage::Ports;
            self.workspace.connect_dialog = false;
            self.workspace.profile_dialog = false;
        }
        if write_unconfirmed && (close_requested || self.closing) {
            self.tab = Tab::Device;
            self.setup.page = setup::Page::Writes;
            self.workspace.connect_dialog = false;
            self.workspace.profile_dialog = false;
        }
        match self.settle_close(close_requested, capture_failed || write_unconfirmed) {
            shutdown::CloseAction::Wait | shutdown::CloseAction::Cancel => {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
            shutdown::CloseAction::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
            shutdown::CloseAction::Idle => {}
        }
        if self.pending_jobs() != 0 {
            ctx.request_repaint_after(capture::LIVE_REFRESH_INTERVAL);
        }
    }

    fn select_parameter_display(&mut self, selection: parameters::DisplaySelection) {
        let id = selection.id.clone();
        match selection.apply(&mut self.profile) {
            Ok(()) => {
                self.setup.sync(&self.profile);
                self.acyclic.clear_named(&id);
                self.message =
                    "Display preference updated; save the profile with Profiles… to retain it."
                        .into();
            }
            Err(error) => self.message = error.to_string(),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _: &mut eframe::Frame) {
        self.render(ctx);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop.request();
    }
}

fn default_capture_path() -> String {
    next_capture_path("")
}

fn next_capture_path(current: &str) -> String {
    next_capture_path_with_prefix(current, "capture")
}

fn next_capture_path_with_prefix(current: &str, prefix: &str) -> String {
    let parent = Path::new(current.trim())
        .parent()
        .unwrap_or_else(|| Path::new(".local"));
    let name = format!(
        "{prefix}-{}",
        chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f")
    );
    parent.join(name).to_string_lossy().into_owned()
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

fn show_input_wire_warning(ui: &mut Ui, profile: &Profile, payload: &[u8]) {
    if let Some(warning) = profile.input_encoding_warning(payload) {
        ui.colored_label(WARNING, warning);
    }
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
