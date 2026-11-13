use super::{ACCENT, WARNING, Worker, background, completed, path_entry};
use eframe::egui::{Context, Ui};
use iolink_workbench::archive;
use std::{path::PathBuf, time::Duration};

mod review;

enum Outcome {
    Verified(Box<archive::Summary>),
    Reviewed(Box<archive::Review>),
}

#[derive(Default)]
pub struct Verifier {
    directory: String,
    joined_csv: String,
    worker: Option<Worker<Outcome>>,
    review: Option<archive::Review>,
    field: String,
    available_fields: Vec<String>,
    buckets: u16,
    summary: Option<archive::Summary>,
    message: String,
}

impl Verifier {
    pub fn update(&mut self, ctx: &Context) {
        if let Some(result) = completed(&mut self.worker) {
            match result {
                Ok(Outcome::Verified(summary)) => {
                    self.review = None;
                    self.summary = Some(*summary);
                    self.message = "CSV data matches the saved report".into();
                }
                Ok(Outcome::Reviewed(review)) => {
                    self.available_fields = review
                        .signal
                        .available_fields
                        .iter()
                        .map(|field| field.name.clone())
                        .collect();
                    self.summary = None;
                    self.review = Some(*review);
                    self.message = "Capture verified and field reviewed".into();
                }
                Err(error) => {
                    self.review = None;
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
        ui.heading("Review saved capture");
        let before = (
            self.directory.clone(),
            self.joined_csv.clone(),
            self.field.clone(),
            self.buckets,
        );
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
                    self.review = None;
                    match background("verify-saved-capture", move || {
                        Ok(Outcome::Verified(Box::new(archive::verify(&directory, joined_csv.as_deref())?)))
                    }) {
                        Ok(worker) => {
                            self.worker = Some(worker);
                            self.message = "Checking capture…".into();
                        }
                        Err(error) => self.message = error.to_string(),
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("Logged field");
                ui.add(eframe::egui::TextEdit::singleline(&mut self.field).hint_text("Empty = first logged field").desired_width(240.0));
                eframe::egui::ComboBox::from_id_salt("capture-review-fields")
                    .selected_text("Saved fields").show_ui(ui, |ui| {
                        for field in &self.available_fields { ui.selectable_value(&mut self.field, field.clone(), field); }
                    });
                if self.buckets == 0 { self.buckets = 128; }
                ui.add(eframe::egui::DragValue::new(&mut self.buckets).range(1..=4096).prefix("Bins "));
                if ui.button("Review field").clicked() {
                    let directory = PathBuf::from(self.directory.trim());
                    let field = (!self.field.is_empty()).then(|| self.field.clone());
                    let buckets = usize::from(self.buckets);
                    self.summary = None;
                    self.review = None;
                    match background("review-saved-capture", move || {
                        Ok(Outcome::Reviewed(Box::new(archive::review(&directory, field.as_deref(), buckets)?)))
                    }) {
                        Ok(worker) => { self.worker = Some(worker); self.message = "Verifying and reviewing capture…".into(); }
                        Err(error) => self.message = error.to_string(),
                    }
                }
            });
            ui.small("Review uses the saved profile and CSV only. Combined CSV output applies to Verify capture.");
        });
        if before
            != (
                self.directory.clone(),
                self.joined_csv.clone(),
                self.field.clone(),
                self.buckets,
            )
        {
            if before.0 != self.directory {
                self.available_fields.clear();
            }
            self.summary = None;
            self.review = None;
            if self.worker.is_none() {
                self.message.clear();
            }
        }
        if !self.message.is_empty() {
            ui.colored_label(
                if self.summary.is_some() || self.review.is_some() {
                    ACCENT
                } else {
                    WARNING
                },
                &self.message,
            );
        }
        if let Some(summary) = self
            .review
            .as_ref()
            .map(|review| &review.capture)
            .or(self.summary.as_ref())
        {
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
        if let Some(review) = &self.review {
            review::show(ui, review);
        }
    }
}
