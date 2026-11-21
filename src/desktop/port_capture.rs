//! Review saved port profiles, read status, and control independent captures.
use super::*;
use anyhow::{Context as _, Result, ensure};
use iolink_workbench::profile::MAX_PORTS;
use iolink_workbench::transport::PortStatusReport;

struct Active {
    output: PathBuf,
    live: capture::PortLive,
    worker: Worker<capture::PortBatch>,
}

struct Finished {
    output: PathBuf,
    result: Result<capture::PortBatch>,
}

struct Reviewed {
    profiles: Vec<Profile>,
    capture_error: Option<String>,
}

pub(super) struct Panel {
    profile_paths: Vec<String>,
    output: String,
    seconds: u32,
    reviewed: Option<Reviewed>,
    review: Option<Worker<Reviewed>>,
    status_worker: Option<Worker<PortStatusReport>>,
    status_result: Option<Result<PortStatusReport>>,
    active: Option<Active>,
    finished: Option<Finished>,
    observations: [Live; MAX_PORTS],
    error: Option<String>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            profile_paths: vec![String::new(), String::new()],
            output: next_capture_path_with_prefix("", "port-set"),
            seconds: 10,
            reviewed: None,
            review: None,
            status_worker: None,
            status_result: None,
            active: None,
            finished: None,
            observations: std::array::from_fn(|_| Live::default()),
            error: None,
        }
    }
}

enum Action {
    Start,
    ReadStatus,
    Verify(PathBuf),
}

impl Panel {
    pub(super) fn stage_render_fixture(&mut self, mut first: Profile, finished: bool) {
        first.connection.port = 1;
        first.name = "Synthetic level input".into();
        let mut second = first.clone();
        second.connection.port = 2;
        second.name = "Synthetic pressure input".into();
        self.profile_paths = vec![".local/level.toml".into(), ".local/pressure.toml".into()];
        self.reviewed = Some(Reviewed {
            profiles: vec![first, second],
            capture_error: None,
        });
        self.output = ".local/port-set-example".into();
        if finished {
            self.finished = Some(Finished {
                output: self.output.clone().into(),
                result: Ok(capture::PortBatch {
                    format_version: 1,
                    gateway: Gateway::Simulator,
                    stopped_by_request: true,
                    observed_master_conflict: false,
                    ports: vec![
                        capture::PortResult {
                            port: 1,
                            profile_name: "Synthetic level input".into(),
                            directory: "port-1".into(),
                            report_saved: true,
                            success: true,
                            received: Some(204),
                            csv_rows_flushed: Some(203),
                            error: None,
                        },
                        capture::PortResult {
                            port: 2,
                            profile_name: "Synthetic pressure input".into(),
                            directory: "port-2".into(),
                            report_saved: true,
                            success: false,
                            received: Some(0),
                            csv_rows_flushed: Some(0),
                            error: Some("Synthetic device identity mismatch".into()),
                        },
                    ],
                }),
            });
        }
    }

    pub(super) fn stage_status_fixture(&mut self) {
        let status = transport::Status {
            route: "synthetic",
            model: "Simulator".into(),
            firmware: "1.0".into(),
            master_serial: Some("SIM-MASTER".into()),
            port: 1,
            port_mode: "IO-Link".into(),
            device_status: Some("Operating properly".into()),
            port_event: None,
            device_event: None,
            port_cycle_raw: None,
            port_cycle_us: Some(1_000),
            identity: Default::default(),
            freshness: Freshness::Counter32,
        };
        self.status_result = Some(Ok(PortStatusReport {
            gateway: Gateway::Simulator,
            observed_master_conflict: false,
            ports: vec![
                transport::PortStatus {
                    port: 1,
                    profile_name: "Synthetic level input".into(),
                    host_finished_utc: "2026-01-01T00:00:00Z".into(),
                    read_duration_us: 40,
                    identity_check: Some("identity matched"),
                    status: Some(status),
                    error: None,
                },
                transport::PortStatus {
                    port: 2,
                    profile_name: "Synthetic pressure input".into(),
                    host_finished_utc: "2026-01-01T00:00:00Z".into(),
                    read_duration_us: 120,
                    identity_check: None,
                    status: None,
                    error: Some("Synthetic status request timed out".into()),
                },
            ],
        }));
    }

    pub fn active(&self) -> bool {
        self.active.is_some()
    }

    pub fn pending_jobs(&self) -> usize {
        usize::from(self.active())
            + usize::from(self.review.is_some())
            + usize::from(self.status_worker.is_some())
    }

    /// Return whether a newly completed capture needs review before closing.
    pub fn update(&mut self) -> Option<bool> {
        if let Some(result) = completed(&mut self.review) {
            match result {
                Ok(reviewed) => self.reviewed = Some(reviewed),
                Err(error) => self.error = Some(format!("{error:#}")),
            }
        }
        if let Some(result) = completed(&mut self.status_worker) {
            self.status_result = Some(result);
        }
        let active = self.active.as_ref()?;
        let result = poll_worker(&active.worker);
        for (index, observation) in self.observations.iter_mut().enumerate() {
            if let Some(latest) = active.live.take((index + 1) as u8) {
                *observation = latest;
            }
        }
        let result = result?;
        let output = self.active.take().expect("active port capture").output;
        let failed = !result
            .as_ref()
            .is_ok_and(capture::PortBatch::all_successful);
        self.finished = Some(Finished { output, result });
        Some(failed)
    }

    fn review_profiles(&mut self) -> Result<()> {
        ensure!(
            self.pending_jobs() == 0,
            "Wait for the active port operation"
        );
        ensure!(
            self.profile_paths
                .iter()
                .all(|path| !path.trim().is_empty()),
            "Enter a path for each selected port profile"
        );
        let paths = self.profile_paths.clone();
        let review = background("review-port-profiles", move || {
            let profiles = paths
                .iter()
                .map(|path| {
                    Profile::load(Path::new(path.trim()))
                        .with_context(|| format!("open port profile {path}"))
                })
                .collect::<Result<Vec<_>>>()?;
            transport::check_port_selection(&profiles)?;
            let capture_error = capture::check_ports(&profiles)
                .err()
                .map(|error| format!("{error:#}"));
            Ok(Reviewed {
                profiles,
                capture_error,
            })
        })?;
        self.clear_review();
        self.review = Some(review);
        Ok(())
    }

    fn clear_review(&mut self) {
        self.reviewed = None;
        self.status_result = None;
        self.error = None;
    }

    fn read_status(&mut self) -> Result<()> {
        ensure!(
            self.pending_jobs() == 0,
            "Wait for the active port operation"
        );
        let profiles = self
            .reviewed
            .as_ref()
            .context("Review the port profiles first")?
            .profiles
            .clone();
        let worker = background("read-port-status", move || {
            transport::read_port_statuses(&profiles)
        })?;
        self.status_worker = Some(worker);
        self.status_result = None;
        self.error = None;
        Ok(())
    }

    fn start(&mut self) -> Result<Stop> {
        ensure!(
            self.pending_jobs() == 0,
            "Wait for the active port operation"
        );
        let reviewed = self
            .reviewed
            .as_ref()
            .context("Review the port profiles first")?;
        if let Some(error) = &reviewed.capture_error {
            anyhow::bail!("{error}");
        }
        let profiles = reviewed.profiles.clone();
        ensure!(
            (1..=86_400).contains(&self.seconds),
            "Enter a duration of 1–86400 seconds per port"
        );
        let destination = capture_destination::Destination::check(&self.output)?;
        let output = destination.path.clone();
        let options = capture::PortOptions {
            output: output.clone(),
            duration: Duration::from_secs(u64::from(self.seconds)),
            simulate_faults: false,
            simulated_writer_delay: Duration::ZERO,
        };
        let stop = Stop::default();
        let worker_stop = stop.clone();
        let live = capture::PortLive::default();
        let worker_live = live.clone();
        let worker = background("cyclic-port-set", move || {
            destination.prepare_parent()?;
            capture::run_ports_observed(&profiles, &options, &worker_stop, &worker_live)
        })?;
        self.active = Some(Active {
            output,
            live,
            worker,
        });
        self.observations = std::array::from_fn(|_| Live::default());
        self.error = None;
        Ok(stop)
    }

    fn show(
        &mut self,
        ui: &mut Ui,
        blocked: Option<&str>,
        stopping: bool,
        can_verify: bool,
    ) -> Option<Action> {
        ui.heading("Multiple ports");
        if let Some(active) = &self.active {
            ui.label(if stopping {
                "Stopping and flushing every port…"
            } else {
                "Capturing configured ports"
            });
            ui.label(active.output.display().to_string());
            if let Some(reviewed) = &self.reviewed {
                for profile in &reviewed.profiles {
                    self.show_observation(ui, profile);
                }
            }
            return None;
        }

        let action = self
            .finished
            .as_ref()
            .and_then(|finished| finished.show(ui, can_verify));
        if self.finished.is_some() {
            ui.separator();
        }
        ui.small("Choose one saved TOML profile for each configured port on the same master.");
        if let Some(reason) = blocked {
            ui.colored_label(WARNING, reason);
        }
        let available = blocked.is_none() && self.pending_jobs() == 0;
        ui.add_enabled_ui(available, |ui| self.show_profile_paths(ui));
        if self.review.is_some() {
            ui.label("Checking profiles offline…");
        }
        if let Some(error) = &self.error {
            ui.colored_label(WARNING, error);
        }
        let mut read = false;
        if let Some(reviewed) = &self.reviewed {
            show_profiles(ui, &reviewed.profiles);
            read = ui
                .add_enabled(available, egui::Button::new("Read port status"))
                .clicked();
        }
        if self.status_worker.is_some() {
            ui.label("Reading port status…");
        }
        if let Some(result) = &self.status_result {
            show_status(ui, result);
        }
        ui.separator();
        let capture_error = self
            .reviewed
            .as_ref()
            .and_then(|reviewed| reviewed.capture_error.as_deref());
        if let Some(error) = capture_error {
            ui.colored_label(WARNING, format!("Capture unavailable · {error}"));
        }
        let capture_ready = self.reviewed.is_some() && capture_error.is_none();
        let start = ui
            .add_enabled_ui(available && capture_error.is_none(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Output folder");
                    path_entry(ui, &mut self.output, "New capture-set folder");
                    if ui.small_button("New folder name").clicked() {
                        self.output = next_capture_path_with_prefix(&self.output, "port-set");
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Duration per port");
                    ui.add(
                        egui::DragValue::new(&mut self.seconds)
                            .range(1..=86_400)
                            .suffix(" s"),
                    );
                });
                ui.add_enabled(
                    capture_ready && !self.output.trim().is_empty(),
                    egui::Button::new(RichText::new("Start capture set").color(ACCENT)),
                )
                .on_disabled_hover_text("Review the port profiles and enter a new output folder")
                .clicked()
            })
            .inner;
        if read {
            Some(Action::ReadStatus)
        } else if start {
            Some(Action::Start)
        } else {
            action
        }
    }

    fn show_profile_paths(&mut self, ui: &mut Ui) {
        let mut changed = false;
        let mut remove = None;
        let can_remove = self.profile_paths.len() > 1;
        for (index, path) in self.profile_paths.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Profile {}", index + 1));
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(path)
                                .hint_text("Saved TOML profile path")
                                .desired_width(340.0),
                        )
                        .changed();
                    if can_remove && ui.small_button("Remove").clicked() {
                        remove = Some(index);
                    }
                });
            });
        }
        if let Some(index) = remove {
            self.profile_paths.remove(index);
            changed = true;
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.profile_paths.len() < MAX_PORTS,
                    egui::Button::new("Add profile"),
                )
                .clicked()
            {
                self.profile_paths.push(String::new());
                changed = true;
            }
            if changed {
                self.clear_review();
            }
            let complete = self
                .profile_paths
                .iter()
                .all(|path| !path.trim().is_empty());
            if ui
                .add_enabled(complete, egui::Button::new("Review profiles"))
                .clicked()
                && let Err(error) = self.review_profiles()
            {
                self.error = Some(format!("{error:#}"));
            }
        });
    }

    fn show_observation(&self, ui: &mut Ui, profile: &Profile) {
        let port = profile.connection.port;
        let observation = &self.observations[usize::from(port - 1)];
        ui.group(|ui| {
            ui.strong(format!("Port {port} · {}", profile.name));
            let message = if observation.message.is_empty() {
                "Waiting for the first observation"
            } else {
                &observation.message
            };
            ui.label(message);
            if let Some(status) = &observation.status {
                ui.small(format!(
                    "{} · firmware {} · {} · cycle {} µs · {}",
                    status.model,
                    status.firmware,
                    status.port_mode,
                    optional_number(status.port_cycle_us),
                    observation.identity_check.unwrap_or("identity unchecked"),
                ));
                if status.freshness == Freshness::Snapshots {
                    ui.small("Host polls · source-update loss unknown");
                }
                ui.collapsing("Device identity and input quality", |ui| {
                    ui.label(format!(
                        "Vendor ID {} · Device ID {} · S/N {}",
                        optional_number(status.identity.vendor_id),
                        optional_number(status.identity.device_id),
                        status.identity.serial.as_deref().unwrap_or("unavailable"),
                    ));
                    if let Some(input) = observation.input {
                        ui.label(format!(
                            "Validity at receipt {} · received {} ms ago",
                            input.validity.label(),
                            input.received.elapsed().as_millis(),
                        ));
                    }
                });
            }
            let counts = observation.counts;
            ui.label(format!(
                "{} received · {} invalid · {} stale · {} reconnects · {} CSV overruns",
                counts.received,
                counts.invalid,
                counts.stale,
                counts.reconnects,
                counts.logger_overruns
            ));
        });
    }
}

fn show_profiles(ui: &mut Ui, profiles: &[Profile]) {
    let Some(first) = profiles.first() else {
        return;
    };
    match first.connection.gateway {
        Gateway::Simulator => ui.strong("Simulator · synthetic input"),
        gateway => ui.strong(format!(
            "{} · {}",
            gateway.label(),
            first.connection.endpoint
        )),
    };
    for profile in profiles {
        ui.label(format!(
            "Port {} · {} · {} input bytes · host poll {} µs",
            profile.connection.port, profile.name, profile.input_bytes, profile.connection.poll_us
        ));
        ui.small(format!(
            "Expected Vendor ID {} · Device ID {} · {}",
            optional_number(profile.expected_identity.vendor_id),
            optional_number(profile.expected_identity.device_id),
            match profile.logging.mode {
                LogMode::EveryUpdate => "Every update / poll",
                LogMode::OnChange => "On change",
            }
        ));
    }
}

fn show_status(ui: &mut Ui, result: &Result<PortStatusReport>) {
    egui::CollapsingHeader::new("Last port status check")
        .default_open(true)
        .show(ui, |ui| {
            let report = match result {
                Ok(report) => report,
                Err(error) => {
                    ui.colored_label(WARNING, format!("{error:#}"));
                    return;
                }
            };
            if report.observed_master_conflict {
                ui.colored_label(
                    WARNING,
                    "Observed master identifiers disagree across the set",
                );
            }
            for port in &report.ports {
                ui.push_id(port.port, |ui| {
                    ui.strong(format!("Port {} · {}", port.port, port.profile_name));
                    if let Some(error) = &port.error {
                        ui.colored_label(WARNING, error);
                    }
                    if let Some(status) = &port.status {
                        ui.label(format!(
                            "{} · {}",
                            status.port_mode,
                            port.identity_check.unwrap_or("identity needs review")
                        ));
                        ui.collapsing("Master, identity and diagnostics", |ui| {
                            ui.label(format!(
                                "{} · firmware {} · master S/N {}",
                                status.model,
                                status.firmware,
                                status.master_serial.as_deref().unwrap_or("unavailable"),
                            ));
                            ui.label(format!(
                                "{} · Vendor ID {} · Device ID {} · device S/N {}",
                                status
                                    .identity
                                    .product_name
                                    .as_deref()
                                    .unwrap_or("Device name unavailable"),
                                optional_number(status.identity.vendor_id),
                                optional_number(status.identity.device_id),
                                status.identity.serial.as_deref().unwrap_or("unavailable"),
                            ));
                            let cycle = status.port_cycle_us.map_or_else(
                                || {
                                    status.port_cycle_raw.as_ref().map_or_else(
                                        || "unavailable".into(),
                                        |raw| format!("{raw} (unit unverified)"),
                                    )
                                },
                                |us| format!("{us} µs"),
                            );
                            ui.label(format!("Port cycle {cycle}"));
                            for (label, value) in [
                                ("Device status", &status.device_status),
                                ("Port event", &status.port_event),
                                ("Device event", &status.device_event),
                            ] {
                                ui.label(format!(
                                    "{label} · {}",
                                    value.as_deref().unwrap_or("unavailable")
                                ));
                            }
                        });
                    }
                    ui.small(format!(
                        "{} · read duration {} µs",
                        port.host_finished_utc, port.read_duration_us
                    ));
                });
            }
        });
}

impl Finished {
    fn show(&self, ui: &mut Ui, can_verify: bool) -> Option<Action> {
        ui.strong("Last capture set");
        ui.label(self.output.display().to_string());
        let report = match &self.result {
            Ok(report) => report,
            Err(error) => {
                ui.colored_label(WARNING, format!("{error:#}"));
                return None;
            }
        };
        if report.observed_master_conflict {
            ui.colored_label(
                WARNING,
                "Observed master identifiers disagree across the set",
            );
        }
        let mut action = None;
        for port in &report.ports {
            ui.push_id(port.port, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(format!("Port {} · {}", port.port, port.profile_name));
                    if port.success {
                        ui.label(format!(
                            "{} rows flushed",
                            optional_number(port.csv_rows_flushed)
                        ));
                    } else {
                        ui.colored_label(WARNING, "Needs review");
                    }
                    if port.report_saved
                        && ui
                            .add_enabled(can_verify, egui::Button::new("Verify capture"))
                            .clicked()
                    {
                        action = Some(Action::Verify(self.output.join(&port.directory)));
                    }
                });
                if let Some(error) = &port.error {
                    ui.colored_label(WARNING, error);
                }
            });
        }
        action
    }
}

#[cfg(test)]
mod tests;

impl App {
    fn start_port_capture(&mut self) -> Result<()> {
        if let Some(reason) = self.start_block_reason() {
            anyhow::bail!(reason);
        }
        self.stop = self.port_capture.start()?;
        self.message = "Starting port capture set…".into();
        Ok(())
    }

    pub(super) fn port_capture_page(&mut self, ui: &mut Ui) {
        let blocked = self.start_block_reason();
        let action = self.port_capture.show(
            ui,
            blocked,
            self.stop.requested(),
            self.saved_capture.can_select_directory(),
        );
        match action {
            Some(Action::Start) => {
                if let Err(error) = self.start_port_capture() {
                    self.port_capture.error = Some(format!("{error:#}"));
                }
            }
            Some(Action::Verify(directory)) if self.saved_capture.verify_directory(&directory) => {
                self.tab = Tab::Offline;
                self.workspace.offline = workspace::OfflinePage::Captures;
            }
            Some(Action::ReadStatus) => {
                if let Err(error) = self.read_port_status() {
                    self.port_capture.error = Some(format!("{error:#}"));
                }
            }
            _ => {}
        }
    }

    fn read_port_status(&mut self) -> Result<()> {
        if let Some(reason) = self.start_block_reason() {
            anyhow::bail!(reason);
        }
        self.port_capture.read_status()
    }
}
