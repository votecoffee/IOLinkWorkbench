use super::WARNING;
use eframe::egui::{self, Ui};
use iolink_workbench::{decode, diagnostics, profile::Profile};

#[derive(Default)]
pub struct Offline {
    device_status: String,
    detailed: String,
}

impl Offline {
    pub fn show(&mut self, ui: &mut Ui, profile: &Profile) {
        ui.heading("Offline device diagnostics");
        ui.small("Paste a recorded 37:0 response as three-byte qualifier/code slots. An optional 36:0 byte gives Device Status. These are read snapshots.");
        ui.add(
            egui::TextEdit::singleline(&mut self.device_status)
                .hint_text("Optional Device Status byte, e.g. 02"),
        );
        if !self.device_status.trim().is_empty() {
            match decode::parse_hex_bounded(&self.device_status, 232) {
                Ok(bytes) => {
                    let decoded = diagnostics::decode_device_status(&bytes);
                    ui.monospace(format!(
                        "36:0 raw {} · {}",
                        decoded.raw_hex,
                        decoded.meaning.unwrap_or("unavailable")
                    ));
                    if let Some(error) = decoded.decode_error {
                        ui.colored_label(WARNING, error);
                    }
                }
                Err(error) => {
                    ui.colored_label(WARNING, error.to_string());
                }
            }
        }
        ui.add(
            egui::TextEdit::multiline(&mut self.detailed)
                .code_editor()
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text("Detailed Device Status, e.g. E48DFE000000"),
        );
        if !self.detailed.trim().is_empty() {
            match decode::parse_hex_bounded(&self.detailed, 232) {
                Ok(bytes) => {
                    ui.monospace(diagnostics::decode_detailed(profile, &bytes).text());
                }
                Err(error) => {
                    ui.colored_label(WARNING, error.to_string());
                }
            }
        }
    }
}
