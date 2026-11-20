//! Focused acquisition views. Worker ownership and polling remain in App.
use super::*;

impl App {
    pub(super) fn inputs(&mut self, ui: &mut Ui) {
        ui.heading("Live inputs");
        if let Err(error) = self.profile.require_process_input() {
            ui.label(error.to_string());
            return;
        }
        ui.label("Read raw and typed process input. Use Connect… to start monitoring or Record CSV… to configure logging.");
        let running = self.worker.is_some();
        if let Some(status) = &self.live.status {
            ui.label(format!(
                "{} · Port {} · {}",
                status.model,
                status.port,
                if running && self.live.connected {
                    "Connected"
                } else {
                    "Last observation"
                }
            ));
            if status.freshness == Freshness::Snapshots {
                ui.colored_label(WARNING, "Host polls · source-update loss and age unknown");
            }
        } else {
            ui.label(
                "No input received. Use Connect… to review the applied route before starting.",
            );
        }
        if running && !self.live.message.is_empty() {
            ui.label(&self.live.message);
        }
        if let (Some(input), Some(observation)) = (
            self.live.input,
            observation::Observation::at(
                &self.live,
                running,
                std::time::Instant::now(),
                &self.profile.connection,
            ),
        ) {
            // Held observations keep aging even after acquisition has stopped.
            ui.ctx().request_repaint_after(Duration::from_millis(100));
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "Validity at receipt {} · last received {} ms ago",
                    input.validity.label(),
                    observation.receive_age.as_millis()
                ));
                if let Some(age) = observation.source_age_at_receipt_us {
                    ui.label(format!(
                        "Gateway input age at receipt {:.2} ms",
                        age as f64 / 1000.0
                    ));
                } else {
                    ui.label("Gateway input age unknown");
                }
                if let Some(age) = observation.observed_update_age {
                    ui.label(format!(
                        "Time since last observed update {} ms",
                        age.as_millis()
                    ));
                }
                if let Some(code) = input.diagnostic_code {
                    ui.label(format!("Gateway code {code}"));
                }
            });
            if observation.show_typed() {
                ui.label(observation.state.label());
            } else {
                ui.colored_label(WARNING, observation.state.label());
            }
            ui.small(format!(
                "Host display age limit {} ms · includes GUI update allowance; source quality above is from receipt",
                observation.display_limit.as_millis()
            ));
            let inspect = ui
                .horizontal_wrapped(|ui| {
                    ui.monospace(format!("RAW  {}", decode::hex(input.payload.bytes())));
                    ui.add_enabled(
                        !input.payload.bytes().is_empty(),
                        egui::Button::new("Inspect bytes offline"),
                    )
                    .clicked()
                })
                .inner;
            show_input_wire_warning(ui, &self.profile, input.payload.bytes());
            if observation.show_typed() {
                match self.profile.decode(input.payload.bytes()) {
                    Ok(rows) => {
                        egui::Grid::new("live-values").striped(true).show(ui, |ui| {
                            ui.strong("Field");
                            ui.strong("Value");
                            ui.strong("Unit");
                            ui.end_row();
                            for (field, _, value) in rows {
                                ui.label(&field.name);
                                ui.colored_label(ACCENT, value.value);
                                ui.label(&field.unit);
                                ui.end_row();
                            }
                        });
                    }
                    Err(error) => {
                        ui.colored_label(WARNING, error.to_string());
                    }
                }
                ui.collapsing("Bit layout, scaling and raw values", |ui| {
                    payload_view(ui, &self.profile, input.payload.bytes(), true);
                });
            }
            if inspect {
                self.payload_inspector
                    .copy(input, observation, &self.live, &self.profile);
                self.workspace.offline = workspace::OfflinePage::Process;
                self.tab = Tab::Offline;
            }
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
    }

    pub(super) fn logging(&mut self, ui: &mut Ui) {
        ui.heading("CSV logging");
        if let Err(error) = self.profile.require_process_input() {
            ui.label(error.to_string());
            self.latest_recording(ui);
            return;
        }
        ui.label("Choose a new folder, the values to log and when to stop. Raw input and metadata are always retained.");
        let available = self.start_block_reason().is_none();
        let route = self.check_session_route();
        let feeds = self.review_feeds();
        let mut start_requested = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    available
                        && route.is_ok()
                        && feeds.valid()
                        && !self.capture_path.trim().is_empty(),
                    egui::Button::new(RichText::new("Start CSV capture").color(ACCENT)),
                )
                .clicked()
            {
                start_requested = true;
            }
            if self.acquisition_active() {
                ui.label(
                    "Session active. Use Stop and flush in the header before starting another.",
                );
            }
        });
        if available && let Err(issue) = &route {
            self.show_route_issue(ui, issue);
        }
        if available {
            self.show_feed_issues(ui, &feeds);
            if feeds.enabled() {
                ui.collapsing("Software feeds for this recording", |ui| {
                    feeds.show_destinations(ui)
                });
            }
        }
        if let Some(directory) = &self.active_recording {
            ui.label(format!("Recording to {}", directory.display()));
        }
        if self.latest_recording.is_some() {
            self.latest_recording(ui);
            ui.separator();
        }
        ui.add_enabled_ui(available, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Capture folder");
                path_entry(ui, &mut self.capture_path, "New capture folder");
                if ui.small_button("New folder name").clicked() {
                    self.capture_path = next_capture_path(&self.capture_path);
                }
                if ui.add_enabled(!self.capture_path.trim().is_empty(), egui::Button::new("Check folder")).clicked() {
                    let _ = self.check_capture_folder();
                }
            });
            if self.capture_path.trim().is_empty() {
                ui.colored_label(WARNING, "Enter a new capture folder.");
            } else if let Some(check) = self.current_folder_check() {
                ui.colored_label(if check.available { ACCENT } else { WARNING }, &check.message);
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Duration");
                ui.add(
                    egui::DragValue::new(&mut self.duration)
                        .range(0..=86_400)
                        .suffix(" s"),
                );
                ui.small("0 = manual stop");
            });
            let mut logging = self.profile.logging.clone();
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut logging.mode, LogMode::EveryUpdate, "Every update / poll");
                ui.selectable_value(&mut logging.mode, LogMode::OnChange, "On change");
            });
            ui.collapsing("Buffer sizing and rotation", |ui| {
                ui.add(egui::DragValue::new(&mut logging.queue_capacity).range(1..=1_000_000).prefix("Queue rows "));
                ui.add(egui::DragValue::new(&mut logging.rotate_rows).prefix("Rotate rows "));
                ui.add(egui::DragValue::new(&mut logging.rotate_seconds).prefix("Rotate seconds "));
                ui.small("0 disables a rotation limit. Queue saturation is counted separately from source loss.");
            });
            if logging != self.profile.logging {
                match self.profile.with_logging(logging) {
                    Ok(candidate) => { self.profile = candidate; self.setup.sync(&self.profile); }
                    Err(error) => self.message = format!("{error:#}"),
                }
            }
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
        if start_requested && let Err(error) = self.start(true) {
            self.message = format!("{error:#}");
        }
    }

    pub(super) fn connection_diagnostics(&mut self, ui: &mut Ui) {
        ui.heading("Connection diagnostics");
        let running = self.worker.is_some();
        if self.live.status.is_none() {
            ui.label("No observed master or port status. Connect explicitly to collect a read-only observation.");
        }
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
                if !running {
                    if self.live.connected {
                        "Session stopped · last observation connected"
                    } else {
                        "Session stopped · last observation disconnected"
                    }
                } else if self.live.connected {
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
                    "Host polls · source-update loss unknown; value changes do not establish freshness",
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
    }

    pub(super) fn run_timing(&self, ui: &mut Ui) {
        ui.label("Finalized timing is available after Stop and flush. Host polling, source updates and writer performance remain separate.");
        if let Some(report) = &self.report {
            ui.separator();
            ui.heading("Run timing");
            super::logger_metrics::acquisition_window(
                ui,
                report.elapsed_us,
                Some(report.local_setup_us),
            );
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
            logger_metrics::receive_tail(ui, report.time_since_last_receive_us);
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
        if self.report.is_none() {
            ui.heading("Run timing");
            ui.label("No completed session yet.");
        }
    }

    pub(super) fn device_diagnostics(&mut self, ui: &mut Ui) {
        ui.heading("Device events");
        ui.small("Read Device Status and Detailed Device Status as one-time snapshots.");
        let available = self.acyclic_controls(ui);
        let read = ui
            .add_enabled(available, egui::Button::new("Read device diagnostics"))
            .clicked();
        let save = ui
            .collapsing("Read and save a snapshot", |ui| {
                ui.add_enabled_ui(available, |ui| {
                    path_entry(
                        ui,
                        &mut self.diagnostic_snapshot_path,
                        "New diagnostic snapshot JSON path",
                    );
                });
                ui.add_enabled(
                    available && !self.diagnostic_snapshot_path.trim().is_empty(),
                    egui::Button::new("Read and save diagnostic snapshot"),
                )
                .clicked()
            })
            .body_returned
            .unwrap_or(false);
        if read || save {
            let output = save.then(|| PathBuf::from(self.diagnostic_snapshot_path.trim()));
            let profile = self.profile.clone();
            self.start_acyclic(acyclic::Query::Events, move || {
                use iolink_workbench::diagnostics;
                let Some(path) = output else {
                    return Ok(diagnostics::read(&profile)?.text());
                };
                diagnostics::check_destination(&path)?;
                let (snapshot, report) = diagnostics::read_snapshot(&profile)?;
                let mut text = report.text();
                match snapshot.save(&path) {
                    Ok(()) => {
                        text.push_str(&format!("\n\nSaved diagnostic snapshot {}", path.display()))
                    }
                    Err(error) => {
                        text.push_str(&format!("\n\nDiagnostic snapshot save failed · {error:#}"))
                    }
                }
                Ok(text)
            });
        }
        if ui.button("Review saved diagnostics…").clicked() {
            self.saved_diagnostics
                .stage_path(&self.diagnostic_snapshot_path);
            self.workspace.offline = workspace::OfflinePage::DiagnosticSnapshots;
            self.tab = Tab::Offline;
        }
        self.acyclic_result(ui, acyclic::Kind::Events);
        if let Some(imported) = &self.profile.iodd {
            ui.collapsing("IODD event definitions", |ui| {
                ui.small(format!(
                    "{} saved event definitions · {} unresolved standard event codes",
                    imported.events.len(),
                    imported.standard_event_refs.len()
                ));
                for event in &imported.events {
                    ui.label(format!(
                        "0x{:04X} · {:?} · {}",
                        event.code, event.event_type, event.name
                    ));
                }
            });
        }
    }

    pub(super) fn parameter_reads(&mut self, ui: &mut Ui) {
        ui.heading("IODD parameter reads");
        ui.label("Choose a declared parameter to read on an independent acyclic client. Reading does not write device settings.");
        let available = self.acyclic_controls(ui);
        let editable = !self.acquisition_active() && !self.writes.busy();
        let parameter_actions = parameters::show(
            ui,
            &self.profile,
            &mut self.parameter_picker,
            available,
            editable,
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
            let index = match profile.parameter(&id) {
                Ok(parameter) => parameter.index,
                Err(error) => {
                    self.message = format!("{error:#}");
                    return;
                }
            };
            let query = acyclic::Query::Named {
                id: id.clone(),
                index,
                subindex,
            };
            self.start_acyclic(query, move || {
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
            });
        }
        self.acyclic_result(ui, acyclic::Kind::Named);
    }

    pub(super) fn raw_isdu(&mut self, ui: &mut Ui) {
        ui.heading("Raw ISDU read");
        ui.small("Use an index and subindex from the device documentation.");
        let available = self.acyclic_controls(ui);
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::DragValue::new(&mut self.isdu_index).prefix("Index "));
            ui.add(egui::DragValue::new(&mut self.isdu_subindex).prefix("Subindex "));
            if ui
                .add_enabled(available, egui::Button::new("Read parameter"))
                .clicked()
            {
                let (profile, index, subindex) =
                    (self.profile.clone(), self.isdu_index, self.isdu_subindex);
                self.start_acyclic(acyclic::Query::Raw(index, subindex), move || {
                    let bytes = transport::read_isdu(&profile, index, subindex)?;
                    let hex = decode::hex(&bytes);
                    Ok(match std::str::from_utf8(&bytes) {
                        Ok(text) => format!("{hex}  ·  {text:?}"),
                        Err(_) => hex,
                    })
                });
            }
        });
        self.acyclic_result(ui, acyclic::Kind::Raw);
        ui.separator();
        let expected = self.profile.expected_identity.product_id.as_ref();
        if ui
            .add_enabled(
                available && expected.is_some(),
                egui::Button::new("Verify Product ID"),
            )
            .on_disabled_hover_text(if expected.is_none() {
                "Set an expected Product ID in Device → Identity expectations"
            } else {
                "Wait for the current operation"
            })
            .clicked()
            && let Some(expected) = expected
        {
            let profile = self.profile.clone();
            let query = acyclic::Query::ProductId(expected.clone());
            self.start_acyclic(query, move || {
                let product_id = transport::verify_product_id(&profile)?;
                Ok(format!("Matched Product ID {product_id}"))
            });
        }
        self.acyclic_result(ui, acyclic::Kind::ProductId);
    }

    fn acyclic_controls(&self, ui: &mut Ui) -> bool {
        if let Some(reason) = self.acyclic_block_reason() {
            ui.colored_label(WARNING, reason);
            false
        } else {
            true
        }
    }

    pub(super) fn integration_status(&self, ui: &mut Ui) {
        let running = self.worker.is_some();
        if running && self.api_enabled {
            ui.label(format!("Local API requested at http://127.0.0.1:{}/v1/status · API drops are separate from CSV/source loss", self.api_port));
        }
        if running && self.shared_memory_enabled {
            ui.label(format!(
                "Shared memory requested · Local\\IOLinkWorkbench.Snapshot.{} · latest state",
                self.shared_memory_name
            ));
        }
        if let Some(shm) = self
            .report
            .as_ref()
            .and_then(|report| report.shared_memory.as_ref())
        {
            ui.label(format!("Shared memory stopped · {} commits · {} queue drops · {} coalesced · {} busy skips · final snapshot: {}",shm.snapshots_written,shm.queue_dropped,shm.coalesced,shm.busy_skips,shm.final_snapshot_written));
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
    }
}
