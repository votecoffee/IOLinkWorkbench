use super::WARNING;
use eframe::egui::{self, Ui};
use iolink_workbench::{decode, diagnostics, profile::Profile};

#[derive(Default)]
pub struct Offline {
    device_status: String,
    detailed: String,
    error_type: String,
    pub(super) error_count: String,
}

impl Offline {
    pub(super) fn stage_error_type_fixture(&mut self) {
        self.error_type = "80 11".into();
    }

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
        egui::CollapsingHeader::new("ISDU ErrorType")
            .default_open(!self.error_type.is_empty())
            .show(ui, |ui| {
                ui.small("Paste the two ErrorType bytes from a recorded negative ISDU reply.");
                ui.add(
                    egui::TextEdit::singleline(&mut self.error_type)
                        .hint_text("ErrorType bytes, e.g. 80 11"),
                );
                if !self.error_type.trim().is_empty() {
                    match decode::parse_hex_bounded(&self.error_type, 2)
                        .and_then(|bytes| diagnostics::decode_error_type(Some(profile), &bytes))
                    {
                        Ok(report) => {
                            ui.monospace(report.text());
                        }
                        Err(error) => {
                            ui.colored_label(WARNING, error.to_string());
                        }
                    }
                }
            });
        ui.collapsing("Device error count (32:0)", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.error_count)
                    .hint_text("Paste the two recorded bytes in hex"),
            );
            if !self.error_count.trim().is_empty() {
                match diagnostics::decode_error_count(&self.error_count) {
                    Ok(report) => {
                        ui.monospace(report.text());
                    }
                    Err(error) => {
                        ui.colored_label(WARNING, error.to_string());
                    }
                }
            }
        });
    }
}

struct SavedResult {
    ok: bool,
    text: String,
    slot_details: Option<String>,
    comparison_reviews: Option<[String; 2]>,
}

pub(super) struct Saved {
    path: String,
    compare_path: String,
    worker: Option<super::Worker<SavedResult>>,
    result: Option<anyhow::Result<SavedResult>>,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            path: ".local/diagnostics.json".into(),
            compare_path: String::new(),
            worker: None,
            result: None,
        }
    }
}

impl Saved {
    pub(super) fn stage_render_fixture(&mut self, compare: bool) {
        self.path = ".local/synthetic-diagnostics.json".into();
        if compare {
            self.compare_path = ".local/synthetic-diagnostics-right.json".into();
        }
        self.result = Some(Ok(if compare {
            SavedResult {
                ok: true,
                text: "Recorded pending-event counts · left → right\nE48DFE · 1 → 0 · Warning · Synthetic range warning".into(),
                slot_details: Some("Slot 1 · E48DFE → 000000".into()),
                comparison_reviews: Some([
                    "Synthetic left read".into(),
                    "Synthetic right read".into(),
                ]),
            }
        } else {
            SavedResult {
                ok: false,
                text: "37:0 · synthetic fixture reply unavailable".into(),
                slot_details: None,
                comparison_reviews: None,
            }
        }));
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
            self.open(false);
        }
        ui.collapsing("Compare two snapshots", |ui| {
            ui.small("Compare raw replies. Matching recorded context enables Device Status, pending-event counts and slot details.");
            if ui
                .add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut self.compare_path).hint_text("Second diagnostic snapshot JSON path"),
                )
                .changed()
            {
                self.result = None;
            }
            if ui
                .add_enabled(
                    !busy && !self.path.trim().is_empty() && !self.compare_path.trim().is_empty(),
                    egui::Button::new("Compare diagnostic snapshots"),
                )
                .clicked()
            {
                self.open(true);
            }
        });
        if self.busy() {
            ui.label("Reading snapshot…");
        }
        if let Some(result) = &self.result {
            match result {
                Ok(result) => {
                    if !result.ok {
                        ui.colored_label(WARNING, "Snapshot needs review");
                    }
                    ui.monospace(&result.text);
                    if let Some(slots) = &result.slot_details {
                        ui.collapsing("Wire slot positions", |ui| ui.monospace(slots));
                    }
                    if let Some([left, right]) = &result.comparison_reviews {
                        ui.collapsing("Left snapshot", |ui| ui.monospace(left));
                        ui.collapsing("Right snapshot", |ui| ui.monospace(right));
                    }
                }
                Err(error) => {
                    ui.colored_label(WARNING, format!("{error:#}"));
                }
            }
        }
    }

    fn open(&mut self, compare: bool) {
        if self.busy() {
            return;
        }
        let left = std::path::PathBuf::from(self.path.trim());
        let right = compare.then(|| std::path::PathBuf::from(self.compare_path.trim()));
        match super::background("offline-diagnostic-snapshot", move || {
            let snapshot = diagnostics::Snapshot::load(&left)?;
            if let Some(right) = right {
                let comparison =
                    diagnostics::compare(&snapshot, &diagnostics::Snapshot::load(&right)?)?;
                return Ok(SavedResult {
                    ok: comparison.ok(),
                    text: comparison.summary_text(),
                    slot_details: comparison.slot_changes_text(),
                    comparison_reviews: Some([comparison.left.text(), comparison.right.text()]),
                });
            }
            let review = snapshot.review()?;
            Ok(SavedResult {
                ok: review.ok(),
                text: review.text(),
                slot_details: None,
                comparison_reviews: None,
            })
        }) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.result = None;
            }
            Err(error) => self.result = Some(Err(error.into())),
        }
    }

    #[cfg(test)]
    pub(super) fn gate_for_test(&mut self) -> super::TestCompletion {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.worker = Some(receiver);
        Box::new(move || {
            let _ = sender.send(Ok(SavedResult {
                ok: true,
                text: "Synthetic saved diagnostic review".into(),
                slot_details: None,
                comparison_reviews: None,
            }));
        })
    }
}
