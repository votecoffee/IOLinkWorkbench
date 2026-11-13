use anyhow::{Context, Result};
use eframe::egui::{self, Ui};
use iolink_workbench::profile::{Connection, CycleUnit, Gateway, Profile};

pub struct Form {
    baseline: Connection,
    draft: Connection,
    mode: String,
}

impl Form {
    pub fn new(connection: &Connection) -> Self {
        Self {
            baseline: connection.clone(),
            draft: connection.clone(),
            mode: connection
                .ifm_mode
                .map(|value| value.to_string())
                .unwrap_or_default(),
        }
    }

    pub fn sync(&mut self, connection: &Connection) {
        self.baseline = connection.clone();
        self.draft = connection.clone();
        self.mode = connection
            .ifm_mode
            .map(|value| value.to_string())
            .unwrap_or_default();
    }

    pub fn pending(&self) -> bool {
        self.draft != self.baseline
            || self.mode
                != self
                    .baseline
                    .ifm_mode
                    .map(|value| value.to_string())
                    .unwrap_or_default()
    }

    fn candidate(&self, profile: &Profile) -> Result<Profile> {
        let mut connection = self.draft.clone();
        connection.ifm_mode =
            if self.mode.trim().is_empty() {
                None
            } else {
                Some(self.mode.trim().parse::<u8>().context(
                    "Expected numeric IO-Link mode must be a code from 0 to 255, or blank",
                )?)
            };
        profile.with_connection(connection)
    }

    pub fn show(&mut self, ui: &mut Ui, profile: &mut Profile, message: &mut String) -> bool {
        ui.heading("Host connection settings");
        ui.label("Edit the host route and sampling settings. Apply updates the local profile; Save writes the profile file.");
        ui.small("Expected firmware, numeric mode and cycle units describe the existing master setup. Applying these settings never connects, changes a port, or writes a device parameter/output.");
        egui::Grid::new("host-connection-settings")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Gateway");
                egui::ComboBox::from_id_salt("host-gateway")
                    .selected_text(match self.draft.gateway {
                        Gateway::Simulator => "Offline simulator",
                        Gateway::IfmAl1350 => "ifm AL1350",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.draft.gateway,
                            Gateway::Simulator,
                            "Offline simulator",
                        );
                        ui.selectable_value(
                            &mut self.draft.gateway,
                            Gateway::IfmAl1350,
                            "ifm AL1350",
                        );
                    });
                ui.end_row();
                ui.label("Port");
                ui.add(egui::DragValue::new(&mut self.draft.port).range(1..=4));
                ui.end_row();
                let ifm = self.draft.gateway == Gateway::IfmAl1350;
                ui.label("Master endpoint");
                ui.add_enabled(
                    ifm,
                    egui::TextEdit::singleline(&mut self.draft.endpoint)
                        .hint_text("http(s)://master-address"),
                );
                ui.end_row();
                ui.label("Expected firmware (optional)");
                ui.add_enabled(
                    ifm,
                    egui::TextEdit::singleline(&mut self.draft.ifm_firmware),
                );
                ui.end_row();
                ui.label("Expected numeric IO-Link mode");
                ui.add_enabled_ui(ifm, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.mode)
                                .hint_text("Confirmed code, or blank"),
                        );
                    });
                });
                ui.end_row();
                ui.label("Confirmed port cycle unit");
                ui.add_enabled_ui(ifm, |ui| {
                    egui::ComboBox::from_id_salt("host-cycle-unit")
                        .selected_text(match self.draft.ifm_cycle_unit {
                            None => "Unknown",
                            Some(CycleUnit::Microseconds) => "Microseconds",
                            Some(CycleUnit::Milliseconds) => "Milliseconds",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.draft.ifm_cycle_unit, None, "Unknown");
                            ui.selectable_value(
                                &mut self.draft.ifm_cycle_unit,
                                Some(CycleUnit::Microseconds),
                                "Microseconds",
                            );
                            ui.selectable_value(
                                &mut self.draft.ifm_cycle_unit,
                                Some(CycleUnit::Milliseconds),
                                "Milliseconds",
                            );
                        });
                });
                ui.end_row();
                ui.label("Requested host poll interval");
                ui.add(
                    egui::DragValue::new(&mut self.draft.poll_us)
                        .range(100..=60_000_000)
                        .suffix(" µs"),
                );
                ui.end_row();
                ui.label("Request timeout");
                ui.add(
                    egui::DragValue::new(&mut self.draft.timeout_ms)
                        .range(1..=30_000)
                        .suffix(" ms"),
                );
                ui.end_row();
                ui.label("Stale threshold");
                ui.add(egui::DragValue::new(&mut self.draft.stale_us).suffix(" µs"));
                ui.end_row();
                ui.label("Reconnect delay");
                ui.add(
                    egui::DragValue::new(&mut self.draft.reconnect_ms)
                        .range(1..=60_000)
                        .suffix(" ms"),
                );
                ui.end_row();
                ui.label("Simulator freshness");
                ui.add_enabled(
                    !ifm,
                    egui::Checkbox::new(
                        &mut self.draft.simulate_snapshots,
                        "Expose snapshots instead of synthetic counters",
                    ),
                );
                ui.end_row();
            });
        if self.draft.gateway == Gateway::IfmAl1350 && self.draft.endpoint.trim().is_empty() {
            ui.small("Unfinished bench template: enter the master endpoint before connecting. Numeric mode and cycle units must come from the installed master documentation/configuration.");
        }
        let mut applied = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.pending(), egui::Button::new("Apply host settings"))
                .clicked()
            {
                match self.candidate(profile) {
                    Ok(candidate) => {
                        *profile = candidate;
                        self.sync(&profile.connection);
                        *message =
                            "Host connection settings applied locally; no gateway access performed"
                                .into();
                        applied = true;
                    }
                    Err(error) => *message = format!("{error:#}"),
                }
            }
            if ui
                .add_enabled(self.pending(), egui::Button::new("Discard host settings"))
                .clicked()
            {
                let baseline = self.baseline.clone();
                self.sync(&baseline);
                message.clear();
            }
            if self.pending() {
                ui.colored_label(super::WARNING, "Pending host settings");
            }
        });
        applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_host_drafts_are_isolated_and_profile_sync_resets_them() {
        let profile = Profile::default();
        let mut form = Form::new(&profile.connection);
        form.draft.poll_us = 20_000;
        assert!(form.pending());
        assert_eq!(profile.connection.poll_us, 1000);
        let candidate = profile.with_connection(form.draft.clone()).unwrap();
        form.sync(&candidate.connection);
        assert!(!form.pending());
        form.draft.endpoint = "unfinished edit".into();
        assert!(form.pending());
        form.sync(&profile.connection);
        assert!(!form.pending());
    }

    #[test]
    fn shared_pending_guard_includes_both_editors_and_failed_candidates() {
        let profile = Profile::default();
        let mut editor = super::super::Editor::new(&profile);
        editor.connection.draft.gateway = Gateway::IfmAl1350;
        editor.connection.draft.endpoint = "invalid endpoint".into();
        assert!(editor.pending());
        assert!(
            profile
                .with_connection(editor.connection.draft.clone())
                .is_err()
        );
        assert!(editor.pending());
        editor.connection.draft.endpoint = "http://127.0.0.1:9".into();
        editor.connection.mode = "300".into();
        assert!(editor.connection.candidate(&profile).is_err());
        assert!(editor.pending());
        editor.connection.mode = "3".into();
        assert_eq!(
            editor
                .connection
                .candidate(&profile)
                .unwrap()
                .connection
                .ifm_mode,
            Some(3)
        );
        editor.sync(&profile);
        assert!(!editor.pending());
        editor.pending = true;
        assert!(editor.pending());
        editor.sync(&profile);
        assert!(!editor.pending());
    }
}
