use super::{ACCENT, WARNING, Worker, background, completed, path_entry};
use eframe::egui::{Context, Ui};
use iolink_workbench::archive;
use std::{path::PathBuf, time::Duration};

mod compare;
mod identity;
mod interfaces;
mod review;

enum Outcome {
    Verified(Box<archive::InterfaceReview>),
    Reviewed(Box<archive::Review>),
    Exported(Box<archive::Redecode>),
    ExportVerified(Box<archive::ExportVerification>),
}

#[derive(Default)]
pub struct Verifier {
    comparer: compare::Comparer,
    directory: String,
    joined_csv: String,
    export_directory: String,
    export: Option<archive::Redecode>,
    bundle_directory: String,
    bundle_check_original: bool,
    checked_export: Option<archive::ExportVerification>,
    worker: Option<Worker<Outcome>>,
    review: Option<archive::Review>,
    field: String,
    available_fields: Vec<String>,
    buckets: u16,
    summary: Option<archive::InterfaceReview>,
    message: String,
}

impl Verifier {
    pub fn update(&mut self, ctx: &Context) {
        self.comparer.update(ctx);
        if let Some(result) = completed(&mut self.worker) {
            self.export = None;
            self.checked_export = None;
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
                Ok(Outcome::ExportVerified(summary)) => {
                    self.summary = None;
                    self.review = None;
                    self.checked_export = Some(*summary);
                    self.message = "Re-decoded bundle checks passed".into();
                }
                Ok(Outcome::Exported(receipt)) => {
                    self.bundle_directory = receipt.output_directory.to_string_lossy().into_owned();
                    self.summary = None;
                    self.review = None;
                    self.export = Some(*receipt);
                    self.message = "Re-decoded CSV bundle published".into();
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

    pub fn show(
        &mut self,
        ui: &mut Ui,
        mapping: &iolink_workbench::profile::Profile,
        mapping_available: bool,
    ) {
        ui.heading("Review saved capture");
        self.comparer.show(ui);
        let before = (
            self.directory.clone(),
            self.joined_csv.clone(),
            self.field.clone(),
            self.buckets,
            self.export_directory.clone(),
            self.bundle_directory.clone(),
            self.bundle_check_original,
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
                    self.export = None;
                    self.checked_export = None;
                    self.summary = None;
                    self.review = None;
                    match background("verify-saved-capture", move || {
                        Ok(Outcome::Verified(Box::new(archive::review_interfaces(&directory, joined_csv.as_deref())?)))
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
                    self.export = None;
                    self.checked_export = None;
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
            ui.collapsing("Re-decode with applied Setup mapping", |ui| {
                ui.label(format!("Mapping · {} · {} bytes · {} selected fields", mapping.name, mapping.input_bytes, mapping.log_fields().len()));
                ui.small("Export a new interpretation of saved raw bytes. No gateway access or device identity verification is performed.");
                ui.horizontal_wrapped(|ui| {
                    ui.label("New export folder");
                    path_entry(ui, &mut self.export_directory, "New folder outside the source capture");
                    if ui.add_enabled(mapping_available, eframe::egui::Button::new("Export re-decoded CSV")).clicked() {
                        let directory = PathBuf::from(self.directory.trim());
                        let output = PathBuf::from(self.export_directory.trim());
                        let mapping = mapping.clone();
                        self.summary = None;
                        self.review = None;
                        self.export = None;
                        self.checked_export = None;
                        match background("redecode-saved-capture", move || {
                            Ok(Outcome::Exported(Box::new(archive::redecode(&directory, &mapping, &output)?)))
                        }) {
                            Ok(worker) => { self.worker = Some(worker); self.message = "Verifying source and exporting revised mapping…".into(); }
                            Err(error) => self.message = error.to_string(),
                        }
                    }
                });
                if !mapping_available { ui.colored_label(WARNING, "Finish IODD inspection and apply pending Setup edits before exporting."); }
            });
            ui.collapsing("Verify re-decoded bundle", |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Existing bundle folder");
                    path_entry(ui, &mut self.bundle_directory, "Folder containing receipt.toml and data.csv");
                });
                ui.checkbox(&mut self.bundle_check_original, "Also reverify the original Capture folder above");
                ui.small("Stored paths in the receipt are not followed. Local checks use only this bundle; optional original-source checks use the Capture folder you entered.");
                if ui.button("Verify bundle").clicked() {
                    let directory = PathBuf::from(self.bundle_directory.trim());
                    let source = self.bundle_check_original.then(|| PathBuf::from(self.directory.trim()));
                    self.summary = None;
                    self.review = None;
                    self.export = None;
                    self.checked_export = None;
                    match background("verify-redecoded-bundle", move || {
                        Ok(Outcome::ExportVerified(Box::new(archive::verify_export(&directory, source.as_deref())?)))
                    }) {
                        Ok(worker) => { self.worker = Some(worker); self.message = "Checking bundle hashes and saved values…".into(); }
                        Err(error) => self.message = error.to_string(),
                    }
                }
            });
        });
        if before
            != (
                self.directory.clone(),
                self.joined_csv.clone(),
                self.field.clone(),
                self.buckets,
                self.export_directory.clone(),
                self.bundle_directory.clone(),
                self.bundle_check_original,
            )
        {
            if before.0 != self.directory {
                self.available_fields.clear();
            }
            self.summary = None;
            self.review = None;
            self.export = None;
            self.checked_export = None;
            if self.worker.is_none() {
                self.message.clear();
            }
        }
        if !self.message.is_empty() {
            ui.colored_label(
                if self.summary.is_some()
                    || self.review.is_some()
                    || self.export.is_some()
                    || self.checked_export.is_some()
                {
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
            .or(self.summary.as_ref().map(|review| &review.capture))
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
        if let Some(review) = &self.summary {
            interfaces::show_review(ui, review);
        }
        if let Some(check) = &self.checked_export {
            ui.label(format!(
                "{} · {} rows · {} mapped fields · {} local file hashes checked",
                check.mapping_profile,
                check.rows_checked,
                check.fields_checked,
                check.local_file_hashes_checked
            ));
            ui.label(format!("Bundle · {}", check.export_directory.display()));
            if let Some(source) = &check.original_source_directory {
                ui.label(format!(
                    "Original capture reverified · {} · {} CSV hashes checked",
                    source.display(),
                    check.original_csv_hashes_checked
                ));
            } else {
                ui.label("Original capture not reverified; checks apply to this bundle only.");
            }
            ui.label(format!(
                "Recorded source-update loss · {} · {} selected sample indexes absent from CSV",
                check.source_capture.recorded_source_update_loss,
                check.source_capture.missing_sample_indexes
            ));
            ui.small(check.scope);
            if check.source_capture.capture_had_error {
                ui.colored_label(WARNING, "The saved acquisition reported a terminal or writer error; that flag is retained.");
            }
        }
        if let Some(receipt) = &self.export {
            ui.label(format!("Export mapping · {}", receipt.mapping_profile));
            ui.label(format!(
                "{} rows exported · {} with typed values · {} with blank typed cells",
                receipt.rows_written, receipt.typed_rows, receipt.blank_rows
            ));
            ui.label(format!("Saved to {}", receipt.output_directory.display()));
            ui.small("Bundle: data.csv, mapping-profile.toml, source-profile.toml, source-capture.toml, and receipt.toml with SHA256 file hashes.");
            ui.small(&receipt.identity_verification);
            if receipt.source_capture.capture_had_error {
                ui.colored_label(WARNING, "The original capture reported a terminal or writer error; that flag is retained in the export receipt.");
            }
        }
    }
}
