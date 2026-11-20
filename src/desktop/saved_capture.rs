use super::{ACCENT, WARNING, Worker, background, completed, path_entry};
use eframe::egui::{self, Context, Ui};
use iolink_workbench::{archive, decode::Field, profile::Profile};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

mod compare;
mod identity;
mod interfaces;
mod review;

enum ExportOutcome {
    Combined(Box<archive::Summary>),
    Written(Box<archive::Redecode>),
    Verified(Box<archive::ExportVerification>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum Page {
    #[default]
    Verify,
    Signal,
    Exports,
    Compare,
}

// Each page keeps its results, while the source pages share a single job slot.
struct Task<T> {
    worker: Option<Worker<T>>,
    result: Option<anyhow::Result<T>>,
}

impl<T> Default for Task<T> {
    fn default() -> Self {
        Self {
            worker: None,
            result: None,
        }
    }
}

impl<T: Send + 'static> Task<T> {
    fn start(&mut self, name: &str, work: impl FnOnce() -> anyhow::Result<T> + Send + 'static) {
        self.result = None;
        match background(name, work) {
            Ok(worker) => self.worker = Some(worker),
            Err(error) => self.result = Some(Err(error.into())),
        }
    }

    fn busy(&self) -> bool {
        self.worker.is_some()
    }

    fn update(&mut self) -> bool {
        if let Some(result) = completed(&mut self.worker) {
            self.result = Some(result);
            true
        } else {
            false
        }
    }

    fn value(&self) -> Option<&T> {
        self.result.as_ref().and_then(|result| result.as_ref().ok())
    }

    fn show_error(&self, ui: &mut Ui) {
        if let Some(Err(error)) = &self.result {
            ui.colored_label(WARNING, format!("{error:#}"));
        }
    }
}

#[derive(Default)]
pub struct Verifier {
    pub(super) page: Page,
    comparer: compare::Comparer,
    directory: String,
    joined_csv: String,
    export_directory: String,
    bundle_directory: String,
    bundle_check_original: bool,
    field: String,
    available_fields: Option<Vec<Field>>,
    buckets: u16,
    verify: Task<archive::InterfaceReview>,
    signal: Task<archive::Review>,
    exports: Task<ExportOutcome>,
}

impl Verifier {
    pub(super) fn can_select_directory(&self) -> bool {
        !self.verify.busy() && !self.signal.busy() && !self.exports.busy()
    }

    fn set_directory(&mut self, directory: String) {
        if self.directory == directory {
            return;
        }
        self.directory = directory;
        self.joined_csv.clear();
        self.field.clear();
        self.available_fields = None;
        self.export_directory.clear();
        self.bundle_check_original = false;
        self.verify.result = None;
        self.signal.result = None;
        self.exports.result = None;
    }

    pub(super) fn select_directory(&mut self, directory: &Path) -> bool {
        let Some(path) = directory.to_str().filter(|_| self.can_select_directory()) else {
            return false;
        };
        self.page = Page::Verify;
        self.set_directory(path.into());
        true
    }

    pub(super) fn verify_directory(&mut self, directory: &Path) -> bool {
        if !self.select_directory(directory) {
            return false;
        }
        self.start_verification();
        true
    }

    fn start_verification(&mut self) {
        let directory = PathBuf::from(self.directory.trim());
        self.verify.start("verify-saved-capture", move || {
            archive::review_interfaces(&directory, None)
        });
    }

    pub(super) fn select_bundle(&mut self, directory: &Path) -> bool {
        let Some(path) = directory.to_str().filter(|_| self.can_select_directory()) else {
            return false;
        };
        self.page = Page::Exports;
        self.bundle_directory = path.into();
        self.bundle_check_original = false;
        self.exports.result = None;
        true
    }

    #[cfg(test)]
    pub(super) fn gates_for_test(&mut self) -> Vec<super::TestCompletion> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.exports.worker = Some(receiver);
        vec![
            Box::new(move || {
                let _ = sender.send(Err(anyhow::anyhow!("synthetic export completion")));
            }),
            self.comparer.gate_for_test(),
        ]
    }

    pub fn pending_jobs(&self) -> usize {
        usize::from(self.verify.busy())
            + usize::from(self.signal.busy())
            + usize::from(self.exports.busy())
            + usize::from(self.comparer.busy())
    }

    pub fn update(&mut self, ctx: &Context) {
        self.comparer.update(ctx);
        if self.verify.update() {
            let fields = self
                .verify
                .value()
                .map(|result| result.logged_fields().to_vec());
            if let Some(fields) = fields {
                self.set_logged_fields(fields);
            } else {
                self.available_fields = None;
                self.signal.result = None;
            }
        }
        if self.signal.update()
            && let Some(review) = self.signal.value()
        {
            self.set_logged_fields(review.signal.available_fields.clone());
        }
        if self.exports.update()
            && let Some(ExportOutcome::Written(receipt)) = self.exports.value()
        {
            self.bundle_directory = receipt.output_directory.to_string_lossy().into_owned();
        }
        if !self.can_select_directory() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    pub(super) fn set_logged_fields(&mut self, fields: Vec<Field>) {
        let selected = if self.field.is_empty() {
            fields.first()
        } else {
            fields.iter().find(|field| field.name == self.field)
        };
        if selected.is_none()
            || self
                .signal
                .value()
                .is_some_and(|review| selected != Some(&review.signal.field))
        {
            self.signal.result = None;
        }
        if selected.is_none() {
            self.field.clear();
        }
        self.available_fields = Some(fields);
    }

    pub fn show(&mut self, ui: &mut Ui, mapping: &Profile, mapping_available: bool) {
        ui.heading("Review saved capture");
        let editable = self.can_select_directory();
        if self.verify.busy() {
            ui.label("Checking capture…");
        } else if self.signal.busy() {
            ui.label("Verifying and reviewing capture…");
        } else if self.exports.busy() {
            ui.label("Checking saved files for export…");
        }
        if self.page == Page::Compare {
            self.comparer.show(ui);
            return;
        }

        ui.add_enabled_ui(editable, |ui| {
            let mut directory = self.directory.clone();
            ui.horizontal_wrapped(|ui| {
                ui.label("Capture folder");
                path_entry(ui, &mut directory, "Capture folder");
            });
            self.set_directory(directory);
        });
        match self.page {
            Page::Verify => self.show_verify(ui, editable),
            Page::Signal => self.show_signal(ui, editable),
            Page::Exports => self.show_exports(ui, mapping, editable, mapping_available),
            Page::Compare => unreachable!(),
        }
    }

    fn show_verify(&mut self, ui: &mut Ui, editable: bool) {
        ui.small("Drop capture.toml from Explorer to select its folder.");
        if ui
            .add_enabled(
                editable && !self.directory.trim().is_empty(),
                egui::Button::new("Verify capture"),
            )
            .clicked()
        {
            self.start_verification();
        }
        self.verify.show_error(ui);
        if let Some(verified) = self.verify.value() {
            ui.colored_label(ACCENT, "CSV data matches the saved report");
            if verified.logged_fields().is_empty() {
                ui.label("No typed CSV fields were recorded.");
            } else {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{} logged fields", verified.logged_fields().len()));
                    if ui.button("Choose a signal").clicked() {
                        self.page = Page::Signal;
                    }
                });
            }
            show_summary(ui, &verified.capture);
            interfaces::show_review(ui, verified);
        }
    }

    fn show_signal(&mut self, ui: &mut Ui, editable: bool) {
        if self.available_fields.as_ref().is_some_and(Vec::is_empty) {
            ui.label("This capture contains no typed CSV fields.");
            if ui.button("Open re-decode options").clicked() {
                self.page = Page::Exports;
            }
            return;
        }
        ui.add_enabled_ui(editable, |ui| {
            ui.horizontal_wrapped(|ui| {
                let before = (self.field.clone(), self.buckets);
                ui.label("Logged field");
                if let Some(fields) = &self.available_fields {
                    select_field(ui, &mut self.field, fields);
                } else {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.field)
                            .hint_text("Empty = first logged field")
                            .desired_width(240.0),
                    );
                }
                if self.buckets == 0 {
                    self.buckets = 128;
                }
                ui.add(
                    egui::DragValue::new(&mut self.buckets)
                        .range(1..=4096)
                        .prefix("Bins "),
                );
                if before != (self.field.clone(), self.buckets) {
                    self.signal.result = None;
                }
                if ui
                    .add_enabled(
                        !self.directory.trim().is_empty(),
                        egui::Button::new("Review field"),
                    )
                    .clicked()
                {
                    let directory = PathBuf::from(self.directory.trim());
                    let field = (!self.field.is_empty()).then(|| self.field.clone());
                    let buckets = usize::from(self.buckets);
                    self.signal.start("review-saved-capture", move || {
                        archive::review(&directory, field.as_deref(), buckets)
                    });
                }
            });
        });
        if self.available_fields.is_none() {
            ui.small("Verify the capture to list its logged fields.");
        }
        self.signal.show_error(ui);
        if let Some(review) = self.signal.value() {
            show_summary(ui, &review.capture);
            review::show(ui, review);
        }
    }

    fn show_exports(
        &mut self,
        ui: &mut Ui,
        mapping: &Profile,
        editable: bool,
        mapping_available: bool,
    ) {
        ui.add_enabled_ui(editable, |ui| {
            ui.strong("Combine CSV parts");
            ui.horizontal_wrapped(|ui| {
                path_entry(ui, &mut self.joined_csv, "New combined CSV filename");
                let ready = !self.directory.trim().is_empty() && !self.joined_csv.trim().is_empty();
                if ui
                    .add_enabled(ready, egui::Button::new("Verify and save combined CSV"))
                    .clicked()
                {
                    let directory = PathBuf::from(self.directory.trim());
                    let destination = PathBuf::from(self.joined_csv.trim());
                    self.exports.start("combine-saved-capture", move || {
                        let verified = archive::review_interfaces(&directory, Some(&destination))?;
                        Ok(ExportOutcome::Combined(Box::new(verified.capture)))
                    });
                }
            });
            ui.separator();
            ui.strong("Re-decode with applied profile mapping");
            ui.label(format!(
                "Mapping · {} · {} bytes · {} selected fields",
                mapping.name,
                mapping.input_bytes,
                mapping.log_fields().len()
            ));
            ui.horizontal_wrapped(|ui| {
                path_entry(
                    ui,
                    &mut self.export_directory,
                    "New folder outside the source capture",
                );
                let ready = mapping_available
                    && !self.directory.trim().is_empty()
                    && !self.export_directory.trim().is_empty();
                if ui
                    .add_enabled(ready, egui::Button::new("Export re-decoded CSV"))
                    .clicked()
                {
                    let directory = PathBuf::from(self.directory.trim());
                    let destination = PathBuf::from(self.export_directory.trim());
                    let mapping = mapping.clone();
                    self.exports.start("redecode-saved-capture", move || {
                        Ok(ExportOutcome::Written(Box::new(archive::redecode(
                            &directory,
                            &mapping,
                            &destination,
                        )?)))
                    });
                }
            });
            if !mapping_available {
                ui.colored_label(
                    WARNING,
                    "Finish IODD inspection and apply pending profile edits before exporting.",
                );
            }
            ui.separator();
            ui.strong("Verify re-decoded bundle");
            ui.small("Drop receipt.toml from Explorer to select its bundle.");
            ui.checkbox(
                &mut self.bundle_check_original,
                "Also reverify the original Capture folder above",
            );
            ui.horizontal_wrapped(|ui| {
                path_entry(
                    ui,
                    &mut self.bundle_directory,
                    "Folder containing receipt.toml and data.csv",
                );
                let ready = !self.exports.busy()
                    && !self.bundle_directory.trim().is_empty()
                    && (!self.bundle_check_original || !self.directory.trim().is_empty());
                if ui
                    .add_enabled(ready, egui::Button::new("Verify bundle"))
                    .clicked()
                {
                    let bundle = PathBuf::from(self.bundle_directory.trim());
                    let source = self
                        .bundle_check_original
                        .then(|| PathBuf::from(self.directory.trim()));
                    self.exports.start("verify-redecoded-bundle", move || {
                        Ok(ExportOutcome::Verified(Box::new(archive::verify_export(
                            &bundle,
                            source.as_deref(),
                        )?)))
                    });
                }
            });
        });
        self.exports.show_error(ui);
        match self.exports.value() {
            Some(ExportOutcome::Combined(summary)) => show_summary(ui, summary),
            Some(ExportOutcome::Written(receipt)) => show_export(ui, receipt),
            Some(ExportOutcome::Verified(check)) => show_export_check(ui, check),
            None => {}
        }
    }
}

fn select_field(ui: &mut Ui, selected: &mut String, fields: &[Field]) {
    let first = format!("First logged field · {}", fields[0].name);
    egui::ComboBox::from_id_salt("capture-review-fields")
        .selected_text(if selected.is_empty() {
            first.as_str()
        } else {
            selected.as_str()
        })
        .width(280.0)
        .height(240.0)
        .truncate()
        .show_ui(ui, |ui| {
            for field in fields {
                let label = if field.unit.is_empty() {
                    field.name.clone()
                } else {
                    format!("{} · {}", field.name, field.unit)
                };
                ui.selectable_value(selected, field.name.clone(), label);
            }
        });
}

fn show_summary(ui: &mut Ui, summary: &archive::Summary) {
    ui.label(format!("{} · {}", summary.profile, summary.utc_start));
    ui.label(format!(
        "{} rows checked across {} parts · {} connection generations",
        summary.rows_checked, summary.csv_parts, summary.generations_in_csv
    ));
    ui.label(format!(
        "{} valid · {} invalid · {} with unknown validity · {} stale",
        summary.valid_rows, summary.invalid_rows, summary.unknown_validity_rows, summary.stale_rows
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

fn show_export_check(ui: &mut Ui, check: &archive::ExportVerification) {
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
        ui.colored_label(
            WARNING,
            "The saved acquisition reported a terminal or writer error; that flag is retained.",
        );
    }
}

fn show_export(ui: &mut Ui, receipt: &archive::Redecode) {
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
