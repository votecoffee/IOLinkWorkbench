use crate::desktop::{WARNING, Worker, background, completed, path_entry};
use eframe::egui::{self, Context, Ui};
use iolink_workbench::archive::{self, CaptureComparison, ComparisonMetric};
use std::{path::PathBuf, time::Duration};

#[derive(Default)]
pub(super) struct Comparer {
    left: String,
    right: String,
    worker: Option<Worker<CaptureComparison>>,
    result: Option<CaptureComparison>,
    message: String,
    show_unavailable: bool,
}

impl Comparer {
    #[cfg(test)]
    pub(super) fn gate_for_test(&mut self) -> crate::desktop::TestCompletion {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.worker = Some(receiver);
        Box::new(move || {
            let _ = sender.send(Err(anyhow::anyhow!("synthetic comparison completion")));
        })
    }
    pub(super) fn busy(&self) -> bool {
        self.worker.is_some()
    }
    pub fn update(&mut self, ctx: &Context) {
        if let Some(result) = completed(&mut self.worker) {
            match result {
                Ok(result) => {
                    self.result = Some(result);
                    self.message.clear();
                }
                Err(error) => {
                    self.result = None;
                    self.message = format!("{error:#}");
                }
            }
        }
        if self.worker.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }

    pub fn show(&mut self, ui: &mut Ui) {
        ui.collapsing("Compare two saved captures offline", |ui| {
            let before = (self.left.clone(), self.right.clone());
            ui.add_enabled_ui(self.worker.is_none(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Left capture");
                    path_entry(ui, &mut self.left, "Finalized capture folder");
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("Right capture");
                    path_entry(ui, &mut self.right, "Finalized capture folder");
                });
                if ui.button("Verify and compare").clicked() {
                    self.result = None;
                    let left = PathBuf::from(self.left.trim());
                    let right = PathBuf::from(self.right.trim());
                    match background("compare-saved-captures", move || {
                        archive::compare_captures(&left, &right)
                    }) {
                        Ok(worker) => {
                            self.worker = Some(worker);
                            self.message = "Verifying both captures…".into();
                        }
                        Err(error) => self.message = error.to_string(),
                    }
                }
            });
            if before != (self.left.clone(), self.right.clone()) {
                self.result = None;
                if self.worker.is_none() {
                    self.message.clear();
                }
            }
            if !self.message.is_empty() {
                ui.colored_label(WARNING, &self.message);
            }
            if let Some(data) = &self.result {
                ui.small(data.basis);
                ui.small(data.delta_policy);
                ui.label(format!(
                    "Left · {} · {} · {} rows",
                    data.left.capture.profile,
                    data.left.capture.utc_start,
                    data.left.capture.rows_checked
                ));
                ui.label(format!(
                    "Right · {} · {} · {} rows",
                    data.right.capture.profile,
                    data.right.capture.utc_start,
                    data.right.capture.rows_checked
                ));
                ui.label(format!(
                    "Timing · {} · counts · {}",
                    data.timing_context.replace('_', " "),
                    data.count_context.replace('_', " ")
                ));
                ui.collapsing("Recorded settings and differences", |ui| {
                    for setting in &data.settings {
                        ui.label(format!("{} · {}", setting.name, setting.relation));
                        if setting.relation != "matching" {
                            ui.small(format!(
                                "Left: {}\nRight: {}",
                                setting.left.as_deref().unwrap_or("unavailable"),
                                setting.right.as_deref().unwrap_or("unavailable")
                            ));
                        }
                    }
                });
                ui.checkbox(&mut self.show_unavailable, "Include unavailable metrics");
                metrics(ui, &data.metrics, self.show_unavailable);
                ui.collapsing("Left verified context and diagnostics", |ui| {
                    super::interfaces::show_review(ui, &data.left)
                });
                ui.collapsing("Right verified context and diagnostics", |ui| {
                    super::interfaces::show_review(ui, &data.right)
                });
            }
        });
    }
}

fn metrics(ui: &mut Ui, metrics: &[ComparisonMetric], unavailable: bool) {
    egui::ScrollArea::both()
        .id_salt("capture-comparison-metrics")
        .max_height(360.0)
        .show(ui, |ui| {
            egui::Grid::new("comparison-values")
                .striped(true)
                .show(ui, |ui| {
                    for heading in ["Domain / metric", "Left", "Right", "Right vs left", "Basis"] {
                        ui.strong(heading);
                    }
                    ui.end_row();
                    for value in metrics
                        .iter()
                        .filter(|v| unavailable || v.left.is_some() || v.right.is_some())
                    {
                        ui.label(format!(
                            "{} / {} ({})",
                            value.domain, value.name, value.unit
                        ));
                        ui.label(
                            value
                                .left
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "unavailable".into()),
                        );
                        ui.label(
                            value
                                .right
                                .map(|v| v.to_string())
                                .unwrap_or_else(|| "unavailable".into()),
                        );
                        ui.label(
                            value
                                .magnitude
                                .map(|v| format!("{} {v}", value.change))
                                .unwrap_or_else(|| "not compared".into()),
                        );
                        ui.label(value.assessment.replace('_', " "));
                        ui.end_row();
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_availability_and_context_render_without_input_or_io() {
        let context = Context::default();
        let values = [ComparisonMetric {
            domain: "csv",
            name: "stale_rows".into(),
            unit: "count",
            left: Some(0),
            right: None,
            assessment: "metric_unavailable",
            change: "not_compared",
            magnitude: None,
        }];
        for show_unavailable in [false, true] {
            let input = egui::RawInput::default();
            assert!(input.events.is_empty());
            let output = context.run(input, |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| metrics(ui, &values, show_unavailable));
            });
            assert!(!output.shapes.is_empty());
        }
    }
}
