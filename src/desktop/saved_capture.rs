use super::{ACCENT, WARNING, Worker, background, completed, path_entry};
use eframe::egui::{Context, Ui};
use iolink_workbench::archive;
use std::{path::PathBuf, time::Duration};

#[derive(Default)]
pub struct Verifier {
    directory: String,
    joined_csv: String,
    worker: Option<Worker<archive::Summary>>,
    summary: Option<archive::Summary>,
    message: String,
}

impl Verifier {
    pub fn update(&mut self, ctx: &Context) {
        if let Some(result) = completed(&mut self.worker) {
            match result {
                Ok(summary) => {
                    self.summary = Some(summary);
                    self.message = "CSV data matches the saved report".into();
                }
                Err(error) => {
                    self.summary = None;
                    self.message = format!("{error:#}");
                }
            }
        }
        if self.worker.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        ui.heading("Verify saved capture");
        ui.label("Choose a finalized capture folder containing capture.toml, profile.toml, and its CSV parts.");
        ui.add_enabled_ui(self.worker.is_none(), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Capture folder");
                path_entry(ui, &mut self.directory, "Capture folder");
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Combined CSV");
                path_entry(ui, &mut self.joined_csv, "Optional new CSV filename");
                if ui.button("Verify capture").clicked() {
                    let directory = PathBuf::from(self.directory.trim());
                    let output = self.joined_csv.trim();
                    let joined_csv = (!output.is_empty()).then(|| PathBuf::from(output));
                    self.summary = None;
                    match background("verify-saved-capture", move || {
                        archive::verify(&directory, joined_csv.as_deref())
                    }) {
                        Ok(worker) => {
                            self.worker = Some(worker);
                            self.message = "Checking capture…".into();
                        }
                        Err(error) => self.message = error.to_string(),
                    }
                }
            });
        });
        if !self.message.is_empty() {
            ui.colored_label(
                if self.summary.is_some() {
                    ACCENT
                } else {
                    WARNING
                },
                &self.message,
            );
        }
        if let Some(summary) = &self.summary {
            ui.label(format!("{} · {}", summary.profile, summary.utc_start));
            ui.label(format!(
                "{} rows checked across {} parts · {} connection generations",
                summary.rows_checked, summary.csv_parts, summary.generations_in_csv
            ));
            ui.label(format!(
                "{} valid · {} invalid · {} with unknown validity · {} stale",
                summary.valid_rows,
                summary.invalid_rows,
                summary.unknown_validity_rows,
                summary.stale_rows
            ));
            ui.label(format!(
                "{} selected sample indexes absent from CSV",
                summary.missing_sample_indexes
            ));
            ui.label(format!(
                "Recorded source-update loss · {}",
                summary.recorded_source_update_loss
            ));
            if let Some(gaps) = summary.recorded_source_gaps {
                ui.label(format!("Recorded missing source updates · {gaps}"));
            }
            if summary.capture_had_error {
                ui.colored_label(
                    WARNING,
                    "The capture report contains a terminal or writer error.",
                );
            }
            if let Some(path) = &summary.joined_csv {
                ui.label(format!("Combined CSV saved to {}", path.display()));
            }
        }
    }
}
