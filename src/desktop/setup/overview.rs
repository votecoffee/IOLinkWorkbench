//! A short route through host setup; no transport or background work belongs here.
use super::{Editor, Page, WARNING};
use crate::desktop::Tab;
use eframe::egui::{self, Ui};
use iolink_workbench::profile::{Gateway, LogMode, Profile};

impl Editor {
    pub(super) fn overview(&mut self, ui: &mut Ui, profile: &Profile, tab: &mut Tab) {
        ui.label("Review these host settings, then use Connect… to review and start monitoring. This overview performs no device reads.");
        section(ui, "1. Host connection", |ui| {
            match profile.connection.gateway {
                Gateway::Simulator => {
                    ui.label("Offline simulator · synthetic input, no hardware");
                }
                Gateway::IfmAl1350 => {
                    if profile.connection.endpoint.trim().is_empty() {
                        ui.colored_label(WARNING, "Needs input · master endpoint is empty");
                    } else {
                        ui.label(format!("ifm AL1350 · {}", profile.connection.endpoint));
                    }
                    ui.small("Documented adapter target; confirm the selected master, firmware and existing port setup before reading.");
                }
            }
            ui.label(format!(
                "Port {} · {} input bytes · requested host poll {} µs",
                profile.connection.port, profile.input_bytes, profile.connection.poll_us
            ));
            if ui.small_button("Edit connection").clicked() {
                self.page = Page::Connection;
            }
        });
        section(ui, "2. Description and process mapping", |ui| {
            if let Some(import) = &profile.iodd {
                ui.label(format!(
                    "IODD product {} · layout {}",
                    import.variant, import.layout
                ));
                if !import.needs_input.is_empty() {
                    ui.colored_label(
                        WARNING,
                        format!(
                            "{} IODD notes need review in Mappings",
                            import.needs_input.len()
                        ),
                    );
                }
                if import.condition.is_some() && !profile.confirm_iodd_condition {
                    ui.colored_label(
                        WARNING,
                        "Needs review · confirm the active IODD condition in Mappings",
                    );
                }
            } else {
                ui.label("No IODD imported · manual mapping and raw capture are available");
            }
            let fields = profile.effective_fields();
            let overrides = fields
                .iter()
                .filter(|(_, origin)| *origin == "override")
                .count();
            ui.label(format!(
                "{} effective fields · {} manual overrides",
                fields.len(),
                overrides
            ));
            ui.horizontal_wrapped(|ui| {
                if ui.small_button("Import IODD").clicked() {
                    self.page = Page::Iodd;
                }
                if ui.small_button("Review mappings").clicked() {
                    *tab = Tab::Mappings;
                }
            });
        });
        section(ui, "3. Identity expectations", |ui| {
            let identity = &profile.expected_identity;
            ui.label(format!(
                "Vendor {} · Device {} · Product ID {}",
                identity
                    .vendor_id
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unconfigured".into()),
                identity
                    .device_id
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "unconfigured".into()),
                identity.product_id.as_deref().unwrap_or("unconfigured")
            ));
            ui.small("Configured pins and IODD selection do not verify the connected device. Observed identity appears in Diagnostics after a read.");
            if ui.small_button("Review identity expectations").clicked() {
                self.page = Page::Identity;
            }
        });
        section(ui, "4. CSV values", |ui| {
            let mode = match profile.logging.mode {
                LogMode::EveryUpdate => "Every verified update / host poll",
                LogMode::OnChange => "On raw input or quality change",
            };
            ui.label(format!(
                "{mode} · {} typed CSV fields plus raw bytes",
                profile.log_fields().len()
            ));
            if profile.logging.raw_only {
                ui.small("Raw-only CSV is selected; typed previews remain available.");
            }
            if ui.small_button("Choose CSV fields").clicked() {
                *tab = Tab::Logging;
            }
        });
    }
}

fn section(ui: &mut Ui, title: &str, content: impl FnOnce(&mut Ui)) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.strong(title);
        content(ui);
    });
}
