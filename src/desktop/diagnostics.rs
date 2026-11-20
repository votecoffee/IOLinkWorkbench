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

pub(super) struct Saved {
    path: String,
    worker: Option<super::Worker<(bool, String)>>,
    result: Option<anyhow::Result<(bool, String)>>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            path: ".local/diagnostics.json".into(),
            worker: None,
            result: None,
        }
    }
}

impl Saved {
    pub(super) fn stage_render_fixture(&mut self) {
        self.path = ".local/synthetic-diagnostics.json".into();
        self.result = Some(Ok((
            false,
            "37:0 · synthetic fixture reply unavailable".into(),
        )));
    }

    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }

    pub fn update(&mut self) {
        if let Some(result) = super::completed(&mut self.worker) {
            self.result = Some(result);
        }
    }

    pub fn stage_path(&mut self, path: &str) {
        if !self.busy() && self.path != path {
            self.path = path.into();
            self.result = None;
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        ui.heading("Saved diagnostic snapshot");
        ui.small(
            "Open a saved read to review its raw ISDU replies, device identity and events offline.",
        );
        let busy = self.busy();
        if ui
            .add_enabled(
                !busy,
                egui::TextEdit::singleline(&mut self.path)
                    .hint_text("Diagnostic snapshot JSON path"),
            )
            .changed()
        {
            self.result = None;
        }
        if ui
            .add_enabled(
                !busy && !self.path.trim().is_empty(),
                egui::Button::new("Open diagnostic snapshot"),
            )
            .clicked()
        {
            let path = std::path::PathBuf::from(self.path.trim());
            match super::background("offline-diagnostic-snapshot", move || {
                let review = diagnostics::Snapshot::load(&path)?.review()?;
                Ok((review.ok(), review.text()))
            }) {
                Ok(worker) => {
                    self.worker = Some(worker);
                    self.result = None;
                }
                Err(error) => self.result = Some(Err(error.into())),
            }
        }
        if self.busy() {
            ui.label("Reading snapshot…");
        }
        if let Some(result) = &self.result {
            match result {
                Ok((ok, text)) => {
                    if !ok {
                        ui.colored_label(WARNING, "Snapshot needs review");
                    }
                    ui.monospace(text);
                }
                Err(error) => {
                    ui.colored_label(WARNING, format!("{error:#}"));
                }
            }
        }
    }

    #[cfg(test)]
    pub(super) fn gate_for_test(&mut self) -> super::TestCompletion {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.worker = Some(receiver);
        Box::new(move || {
            let _ = sender.send(Ok((true, "Synthetic saved diagnostic review".into())));
        })
    }
}
