//! Task-oriented workspace, aligned with EthernetIP Workbench navigation.
use super::*;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct Workspace {
    pub profile_dialog: bool,
    pub connect_dialog: bool,
    pub parameter: ParameterPage,
    pub offline: OfflinePage,
    pub diagnostics: DiagnosticsPage,
    pub integration: IntegrationPage,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum ParameterPage {
    #[default]
    Iodd,
    Raw,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum OfflinePage {
    #[default]
    Captures,
    Process,
    Parameter,
    Snapshot,
    Events,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum DiagnosticsPage {
    #[default]
    Connection,
    Timing,
    Events,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum IntegrationPage {
    #[default]
    Api,
    Mqtt,
    SharedMemory,
}

impl Tab {
    pub(super) const ALL: [Self; 8] = [
        Self::Device,
        Self::Inputs,
        Self::Logging,
        Self::Mappings,
        Self::Parameters,
        Self::Integration,
        Self::Diagnostics,
        Self::Offline,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Device => "Device",
            Self::Inputs => "Inputs",
            Self::Logging => "Logging",
            Self::Mappings => "Mappings",
            Self::Parameters => "Parameters",
            Self::Integration => "Integration",
            Self::Diagnostics => "Diagnostics",
            Self::Offline => "Offline",
        }
    }
}

fn subtabs<T: Copy + PartialEq>(ui: &mut Ui, selected: &mut T, choices: &[(T, &str)]) {
    ui.horizontal_wrapped(|ui| {
        for &(page, label) in choices {
            ui.selectable_value(selected, page, label);
        }
    });
    ui.separator();
}

impl App {
    pub(super) fn render(&mut self, ctx: &Context) {
        // Completion polling must precede all views, including hidden tabs/dialogs.
        self.update_workers(ctx);
        self.handle_file_drop(ctx);
        egui::TopBottomPanel::top("navigation").show(ctx, |ui| {
            if self.closing {
                ui.disable();
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("IO-Link Workbench").size(22.0).strong());
                ui.colored_label(ACCENT, "READ ONLY");
                ui.label(if self.closing {
                    "CLOSING"
                } else if self.worker.is_some() && self.stop.requested() {
                    "STOPPING / FLUSHING"
                } else if self.worker.is_some() && !self.live.connected {
                    "CONNECTING / RECONNECTING"
                } else if self.active_recording.is_some() {
                    "RECORDING CSV"
                } else if self.worker.is_some() {
                    "MONITORING"
                } else {
                    "OFFLINE"
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.add_sized(
                    [180.0, 18.0],
                    egui::Label::new(RichText::new(&self.profile.name).strong()).truncate(),
                )
                .on_hover_text(&self.profile.name);
                ui.label(match self.profile.connection.gateway {
                    Gateway::Simulator => {
                        format!("Simulator · Port {}", self.profile.connection.port)
                    }
                    Gateway::IfmAl1350 => format!("AL1350 · Port {}", self.profile.connection.port),
                });
                if ui
                    .add_enabled(
                        self.start_block_reason().is_none(),
                        egui::Button::new("Connect…"),
                    )
                    .clicked()
                {
                    self.workspace.profile_dialog = false;
                    self.workspace.connect_dialog = true;
                }
                if ui.button("Record CSV…").clicked() {
                    self.tab = Tab::Logging;
                }
                if ui.button("Profiles…").clicked() {
                    self.workspace.connect_dialog = false;
                    self.workspace.profile_dialog = true;
                }
                if self.worker.is_some()
                    && ui
                        .add_enabled(!self.stop.requested(), egui::Button::new("Stop and flush"))
                        .clicked()
                {
                    self.stop.request();
                    self.message = "Stopping and flushing…".into();
                }
            });
            if self.setup.pending() {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(
                        WARNING,
                        "Unapplied profile edits · acquisition and profile save are blocked",
                    );
                    if ui.small_button("Review pending edits").clicked() {
                        self.tab = if self.setup.reveal_pending() {
                            Tab::Mappings
                        } else {
                            Tab::Device
                        };
                    }
                });
            } else if self.setup.busy() {
                ui.colored_label(
                    WARNING,
                    "Inspecting IODD · acquisition is blocked until inspection completes",
                );
            }
            let counts = self.live.counts;
            if counts.invalid != 0
                || counts.stale != 0
                || counts.logger_overruns != 0
                || counts.reconnects != 0
            {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(
                        WARNING,
                        format!(
                            "Invalid {} · Stale {} · CSV overruns {} · Reconnects {}",
                            counts.invalid, counts.stale, counts.logger_overruns, counts.reconnects
                        ),
                    );
                    if ui.small_button("Review diagnostics").clicked() {
                        self.tab = Tab::Diagnostics;
                    }
                });
            }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                for tab in Tab::ALL {
                    ui.selectable_value(&mut self.tab, tab, tab.label());
                }
            });
        });
        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let message = if self.message.is_empty() {
                    "Device → Inputs → Logging. Offline tools need no connection."
                } else {
                    &self.message
                };
                ui.add(egui::Label::new(message).truncate())
                    .on_hover_text(message);
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.closing { ui.disable(); }
            self.page_navigation(ui);
            egui::ScrollArea::vertical().id_salt(self.scroll_id()).auto_shrink([false, false]).show(ui, |ui| {
                ui.set_max_width(1000.0);
                let editable = self.worker.is_none() && !self.acyclic.busy() && !self.setup.busy();
                if matches!(self.tab, Tab::Device | Tab::Mappings) && !editable {
                    ui.colored_label(WARNING, "Wait for active acquisition and parameter reads to finish before editing setup.");
                }
                let changed = match self.tab {
                    Tab::Device => self.setup.show(ui, &mut self.profile, &mut self.message, editable, &mut self.tab),
                    Tab::Mappings => self.setup.show_mappings(ui, &mut self.profile, &mut self.message, editable),
                    Tab::Inputs => { self.inputs(ui); false }
                    Tab::Logging => { self.logging(ui); false }
                    Tab::Parameters => { self.parameter_page(ui); false }
                    Tab::Integration => { self.integration_page(ui); false }
                    Tab::Diagnostics => { self.diagnostics_page(ui); false }
                    Tab::Offline => { self.offline_page(ui); false }
                };
                if changed { self.profile_changed(); }
            });
        });
        self.dialogs(ctx);
        self.file_drop_preview(ctx);
    }

    fn page_navigation(&mut self, ui: &mut Ui) {
        match self.tab {
            Tab::Device => {
                let choices: Vec<_> = setup::Page::ALL
                    .into_iter()
                    .map(|page| (page, page.label()))
                    .collect();
                subtabs(ui, &mut self.setup.page, &choices);
            }
            Tab::Parameters => {
                subtabs(
                    ui,
                    &mut self.workspace.parameter,
                    &[
                        (ParameterPage::Iodd, "IODD reads"),
                        (ParameterPage::Raw, "Raw ISDU"),
                    ],
                );
            }
            Tab::Diagnostics => {
                subtabs(
                    ui,
                    &mut self.workspace.diagnostics,
                    &[
                        (DiagnosticsPage::Connection, "Connection & identity"),
                        (DiagnosticsPage::Timing, "Run timing"),
                        (DiagnosticsPage::Events, "Device events"),
                    ],
                );
            }
            Tab::Offline => {
                subtabs(
                    ui,
                    &mut self.workspace.offline,
                    &[
                        (OfflinePage::Captures, "Saved captures"),
                        (OfflinePage::Process, "Process bytes"),
                        (OfflinePage::Parameter, "Parameter bytes"),
                        (OfflinePage::Snapshot, "Parameter snapshot"),
                        (OfflinePage::Events, "Event codes"),
                    ],
                );
                if self.workspace.offline == OfflinePage::Captures {
                    use saved_capture::Page;
                    subtabs(
                        ui,
                        &mut self.saved_capture.page,
                        &[
                            (Page::Verify, "Verify"),
                            (Page::Signal, "Signal"),
                            (Page::Exports, "Exports"),
                            (Page::Compare, "Compare captures"),
                        ],
                    );
                }
            }
            Tab::Integration => {
                subtabs(
                    ui,
                    &mut self.workspace.integration,
                    &[
                        (IntegrationPage::Api, "Local HTTP API"),
                        (IntegrationPage::Mqtt, "MQTT"),
                        (IntegrationPage::SharedMemory, "Windows shared memory"),
                    ],
                );
            }
            _ => {}
        }
    }

    fn scroll_id(&self) -> egui::Id {
        // Each focused page keeps its own position; switching away must not strand
        // the next page below its heading or controls.
        egui::Id::new((
            self.tab,
            match self.tab {
                Tab::Device => self.setup.page as u8,
                Tab::Mappings => u8::from(self.setup.mapping_opened()),
                Tab::Parameters => self.workspace.parameter as u8,
                Tab::Integration => self.workspace.integration as u8,
                Tab::Diagnostics => self.workspace.diagnostics as u8,
                Tab::Offline => self.workspace.offline as u8,
                _ => 0,
            },
            (self.tab == Tab::Offline && self.workspace.offline == OfflinePage::Captures)
                .then_some(self.saved_capture.page),
            (self.tab == Tab::Offline && self.workspace.offline == OfflinePage::Process)
                .then_some(self.payload_inspector.page),
        ))
    }

    fn profile_changed(&mut self) {
        self.live = Live::default();
        self.report = None;
        self.parameter_picker = parameters::Picker::default();
        self.offline_parameter = parameters::Offline::default();
        self.offline_diagnostics = diagnostics::Offline::default();
        self.acyclic.clear_results();
    }

    fn parameter_page(&mut self, ui: &mut Ui) {
        match self.workspace.parameter {
            ParameterPage::Iodd => self.parameter_reads(ui),
            ParameterPage::Raw => self.raw_isdu(ui),
        }
    }

    fn diagnostics_page(&mut self, ui: &mut Ui) {
        match self.workspace.diagnostics {
            DiagnosticsPage::Connection => {
                if !self.message.is_empty() {
                    ui.label(&self.message);
                }
                self.connection_diagnostics(ui);
            }
            DiagnosticsPage::Timing => self.run_timing(ui),
            DiagnosticsPage::Events => self.device_diagnostics(ui),
        }
    }

    fn offline_page(&mut self, ui: &mut Ui) {
        match self.workspace.offline {
            OfflinePage::Captures => {
                if self.latest_recording.is_some()
                    && self.saved_capture.page != saved_capture::Page::Compare
                {
                    self.latest_recording(ui);
                    ui.separator();
                }
                self.saved_capture.show(
                    ui,
                    &self.profile,
                    !self.setup.pending() && !self.setup.busy(),
                    self.latest_recording
                        .as_ref()
                        .map(|recording| recording.directory.as_path()),
                );
            }
            OfflinePage::Process => self.payload_inspector.show(ui, &self.profile),
            OfflinePage::Parameter => {
                if let Some(selection) = self.offline_parameter.show(
                    ui,
                    &self.profile,
                    self.worker.is_none()
                        && !self.acyclic.busy()
                        && !self.setup.pending()
                        && !self.setup.busy(),
                ) {
                    self.select_parameter_display(selection);
                }
            }
            OfflinePage::Snapshot => self.saved_parameter.show(ui),
            OfflinePage::Events => self.offline_diagnostics.show(ui, &self.profile),
        }
    }

    pub(super) fn latest_recording(&mut self, ui: &mut Ui) {
        let Some(recording) = &self.latest_recording else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.strong("Latest recording");
            ui.label(format!(
                "{} rows flushed · {} CSV parts",
                recording.rows, recording.parts
            ));
            if recording.has_error {
                ui.colored_label(WARNING, "Run reported an error");
            }
            if recording.unconfirmed_rows != 0 {
                ui.colored_label(
                    WARNING,
                    format!("{} unconfirmed rows", recording.unconfirmed_rows),
                );
            }
            if ui
                .add_enabled(
                    self.saved_capture.can_select_directory(),
                    egui::Button::new("Verify latest recording"),
                )
                .on_disabled_hover_text("Wait for the active capture review to finish")
                .clicked()
                && self.saved_capture.verify_directory(&recording.directory)
            {
                self.tab = Tab::Offline;
                self.workspace.offline = OfflinePage::Captures;
            }
        });
        ui.label(recording.directory.display().to_string());
    }

    fn dialogs(&mut self, ctx: &Context) {
        if self.workspace.profile_dialog {
            let mut open = true;
            let mut applied = false;
            egui::Window::new("Device profiles")
                .open(&mut open)
                .default_width(600.0)
                .vscroll(true)
                .show(ctx, |ui| {
                    let editable = !self.closing
                        && self.worker.is_none()
                        && !self.acyclic.busy()
                        && !self.setup.busy();
                    if self
                        .setup
                        .show_profiles(ui, &mut self.profile, &mut self.message, editable)
                    {
                        self.profile_changed();
                        self.workspace.connect_dialog = false;
                        self.tab = Tab::Device;
                        applied = true;
                    }
                    if !self.message.is_empty() {
                        ui.label(&self.message);
                    }
                });
            self.workspace.profile_dialog = open && !applied;
        }
        if self.workspace.connect_dialog && self.worker.is_none() && !self.closing {
            let mut open = true;
            egui::Window::new("Review connection").open(&mut open).default_width(600.0).vscroll(true).show(ctx, |ui| {
                ui.heading(&self.profile.name);
                let route = self.check_session_route();
                ui.label(match self.profile.connection.gateway {
                    Gateway::Simulator => "Offline simulator · synthetic input".into(),
                    Gateway::IfmAl1350 if matches!(&route, Err(session_review::RouteIssue::Connection(_))) => "ifm AL1350 · review the configured endpoint".into(),
                    Gateway::IfmAl1350 => format!("ifm AL1350 · {}", self.profile.connection.endpoint),
                });
                ui.label(format!("Port {} · host poll {} µs · expected input {} bytes", self.profile.connection.port,
                    self.profile.connection.poll_us, self.profile.input_bytes));
                ui.label(format!("Expected vendor {} · device {} · Product ID {}",
                    optional_number(self.profile.expected_identity.vendor_id), optional_number(self.profile.expected_identity.device_id),
                    self.profile.expected_identity.product_id.as_deref().unwrap_or("unavailable")));
                ui.small("Monitoring reads the existing configured port. The connection will check reachability and observe device identity.");
                let feeds = self.review_feeds();
                if feeds.enabled() { ui.strong("Software feeds for this session"); }
                feeds.show_destinations(ui);
                self.show_feed_issues(ui, &feeds);
                let blocked = self.start_block_reason();
                if let Some(reason) = blocked { ui.colored_label(WARNING, reason); }
                if let Err(issue) = &route { self.show_route_issue(ui, issue); }
                if ui.add_enabled(blocked.is_none() && route.is_ok() && feeds.valid(), egui::Button::new("Start monitor")).clicked() {
                    match self.start(false) { Ok(()) => { self.tab = Tab::Inputs; }, Err(error) => self.message = format!("{error:#}") }
                }
                if !self.message.is_empty() { ui.label(&self.message); }
            });
            self.workspace.connect_dialog &= open && self.worker.is_none();
        } else {
            self.workspace.connect_dialog = false;
        }
    }

    fn integration_page(&mut self, ui: &mut Ui) {
        ui.heading("Software integration");
        ui.label("Optional feeds run with the next monitor or capture session. Stop acquisition to change their settings.");
        ui.add_enabled_ui(self.start_block_reason().is_none(), |ui| match self.workspace.integration {
            IntegrationPage::Api => {
                ui.checkbox(&mut self.api_enabled, "Read-only local API");
                ui.add_enabled(self.api_enabled, egui::DragValue::new(&mut self.api_port).range(1..=65535).prefix("127.0.0.1:"));
                ui.small("Loopback only · status, schema and bounded history · independent queue/loss counters");
            }
            IntegrationPage::Mqtt => {
                ui.checkbox(&mut self.mqtt_enabled, "MQTT latest-state publishing");
                ui.label("Broker IP:port"); ui.text_edit_singleline(&mut self.mqtt_broker);
                ui.label("Topic"); ui.text_edit_singleline(&mut self.mqtt_topic);
                ui.add(egui::DragValue::new(&mut self.mqtt_interval_ms).range(10..=60000).suffix(" ms"));
                ui.small("TCP · QoS 0 · not retained · no TLS/auth · broker delivery unknown");
            }
            IntegrationPage::SharedMemory => {
                ui.checkbox(&mut self.shared_memory_enabled, "Windows shared-memory snapshot");
                ui.label("Name"); ui.text_edit_singleline(&mut self.shared_memory_name);
                ui.add(egui::DragValue::new(&mut self.shared_memory_interval_ms).range(1..=1000).suffix(" ms"));
                ui.small("Local mapping · latest state · readers use the named mutex · independent loss accounting");
            }
        });
        if let Some(error) = self.review_feeds().issue(self.workspace.integration) {
            ui.colored_label(WARNING, format!("{error:#}"));
        }
        self.integration_status(ui);
    }
}
