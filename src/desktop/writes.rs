use super::{ACCENT, App, Gateway, Profile, WARNING, Worker, background, completed};
use anyhow::{Context, Result, ensure};
use eframe::egui::{self, Ui};
use iolink_workbench::transport::write::{
    CurrentRead, CurrentValue, Operation, Outcome, Plan, ReadPoint,
};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Kind {
    #[default]
    Mode,
    Parameter,
    Command,
    Output,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Mode => "Port mode",
            Self::Parameter => "ISDU parameter",
            Self::Command => "Device command",
            Self::Output => "Process output",
        }
    }
}

#[derive(Default)]
struct Draft {
    kind: Kind,
    expected: String,
    desired: String,
    index: u16,
    subindex: u8,
    code: u8,
    meaning: String,
}

impl Draft {
    fn read_point(&self) -> Option<ReadPoint> {
        match self.kind {
            Kind::Mode => Some(ReadPoint::PortMode),
            Kind::Parameter => Some(ReadPoint::Isdu {
                index: self.index,
                subindex: self.subindex,
            }),
            Kind::Command => None,
            Kind::Output => Some(ReadPoint::ProcessOutput),
        }
    }

    fn operation(&self) -> Result<Operation> {
        Ok(match self.kind {
            Kind::Mode => Operation::PortMode {
                expected: self
                    .expected
                    .trim()
                    .parse()
                    .context("Enter the expected numeric mode in decimal")?,
                desired: self
                    .desired
                    .trim()
                    .parse()
                    .context("Enter the desired numeric mode in decimal")?,
            },
            Kind::Parameter => Operation::Isdu {
                index: self.index,
                subindex: self.subindex,
                expected: self.expected.clone(),
                desired: self.desired.clone(),
            },
            Kind::Command => Operation::DeviceCommand {
                code: self.code,
                meaning: self.meaning.clone(),
            },
            Kind::Output => Operation::ProcessOutput {
                expected: self.expected.clone(),
                desired: self.desired.clone(),
            },
        })
    }

    fn show(&mut self, ui: &mut Ui) {
        let before = self.kind;
        egui::ComboBox::from_id_salt("write-operation")
            .selected_text(self.kind.label())
            .show_ui(ui, |ui| {
                for kind in [Kind::Mode, Kind::Parameter, Kind::Command, Kind::Output] {
                    ui.selectable_value(&mut self.kind, kind, kind.label());
                }
            });
        if self.kind != before {
            self.expected.clear();
            self.desired.clear();
        }
        match self.kind {
            Kind::Mode => {
                ui.small("Use the numeric mode codes confirmed for this firmware.");
                self.values(ui, "Expected mode", "Desired mode");
            }
            Kind::Parameter => {
                ui.horizontal_wrapped(|ui| {
                    ui.add(egui::DragValue::new(&mut self.index).prefix("Index "));
                    ui.add(egui::DragValue::new(&mut self.subindex).prefix("Subindex "));
                });
                self.values(ui, "Expected parameter hex", "Desired parameter hex");
            }
            Kind::Command => {
                ui.add(
                    egui::DragValue::new(&mut self.code).prefix("SystemCommand code (decimal) "),
                );
                text_entry(ui, "Documented command meaning", &mut self.meaning);
            }
            Kind::Output => {
                ui.small("Enter the full process output image on both sides.");
                self.values(ui, "Expected output hex", "Desired output hex");
            }
        }
    }

    fn values(&mut self, ui: &mut Ui, before: &str, after: &str) {
        text_entry(ui, before, &mut self.expected);
        text_entry(ui, after, &mut self.desired);
    }
}

fn text_entry(ui: &mut Ui, label: &str, value: &mut String) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        ui.add(egui::TextEdit::singleline(value).desired_width(280.0));
    });
}

struct WriteResult {
    summary: &'static str,
    detail: Option<String>,
    receipt: Option<String>,
    unconfirmed: bool,
}

impl WriteResult {
    fn execute(plan: Plan) -> Result<Self> {
        let receipt = match plan.apply(plan.confirmation()) {
            Ok(receipt) => receipt,
            Err(error) => {
                return Ok(Self {
                    summary: "Write blocked before dispatch",
                    detail: Some(format!("{error:#}")),
                    receipt: None,
                    unconfirmed: false,
                });
            }
        };
        Ok(Self {
            summary: match receipt.outcome {
                Outcome::ReadbackMatched => "Gateway readback matched the requested value",
                Outcome::GatewayAcknowledged => "Gateway acknowledged the device command",
                Outcome::Unconfirmed => {
                    "Write outcome unconfirmed. Inspect the target before another action."
                }
            },
            detail: receipt.detail.clone(),
            receipt: Some(serde_json::to_string_pretty(&receipt)?),
            unconfirmed: receipt.outcome == Outcome::Unconfirmed,
        })
    }
}

#[derive(Default)]
pub(super) struct Panel {
    master_serial: String,
    draft: Draft,
    review: Option<Plan>,
    approved: bool,
    error: Option<String>,
    worker: Option<Worker<WriteResult>>,
    read_worker: Option<Worker<CurrentValue>>,
    current: Option<CurrentValue>,
    result: Option<WriteResult>,
    dispatched_target: String,
}

enum Action {
    ReadCurrent,
    Apply,
}

impl Panel {
    pub(super) fn busy(&self) -> bool {
        self.writing() || self.read_worker.is_some()
    }

    pub(super) fn writing(&self) -> bool {
        self.worker.is_some()
    }

    pub(super) fn clear_draft(&mut self) {
        self.master_serial.clear();
        self.draft = Draft::default();
        self.current = None;
        self.clear_review();
    }

    fn clear_review(&mut self) {
        self.review = None;
        self.approved = false;
        self.error = None;
    }

    pub(super) fn stage(&mut self, plan: Plan) {
        self.clear_review();
        self.review = Some(plan);
    }

    pub(super) fn update(&mut self, message: &mut String) -> bool {
        if let Some(result) = completed(&mut self.read_worker) {
            match result {
                Ok(current) => {
                    self.current = Some(current);
                    *message = "Current value read · review it in Device → Writes".into();
                }
                Err(error) => {
                    let detail = format!("Current-value read failed · {error:#}");
                    self.error = Some(detail.clone());
                    *message = detail;
                }
            }
        }
        let Some(result) = completed(&mut self.worker) else {
            return false;
        };
        let result = result.unwrap_or_else(|error| WriteResult {
            summary: "Write outcome unconfirmed. Inspect the target before another action.",
            detail: Some(format!("{error:#}")),
            receipt: None,
            unconfirmed: true,
        });
        *message = result.summary.into();
        let unconfirmed = result.unconfirmed;
        self.result = Some(result);
        unconfirmed
    }

    fn current_for_draft(&self) -> Option<&CurrentValue> {
        self.current.as_ref().filter(|value| {
            self.draft.read_point() == Some(value.point)
                && self.master_serial == value.master_serial
        })
    }

    fn show(
        &mut self,
        ui: &mut Ui,
        profile: &Profile,
        blocked: Option<&str>,
        read_blocked: Option<&str>,
    ) -> Option<Action> {
        ui.heading("Reviewed AL1350 writes");
        if let Some(reason) = blocked {
            ui.colored_label(WARNING, reason);
        }
        let mut action = None;
        if !self.busy() && profile.connection.gateway == Gateway::IfmAl1350 {
            if let Some(plan) = &self.review {
                show_review(ui, plan);
                ui.add_enabled_ui(blocked.is_none(), |ui| {
                    ui.checkbox(&mut self.approved, "I approve this exact write");
                    if ui
                        .add_enabled(self.approved, egui::Button::new("Apply reviewed write"))
                        .clicked()
                    {
                        action = Some(Action::Apply);
                    }
                });
                if ui.button("Edit write").clicked() {
                    self.clear_review();
                }
            } else {
                ui.label(format!(
                    "{} · Port {}",
                    profile.connection.endpoint, profile.connection.port
                ));
                text_entry(ui, "Expected master serial", &mut self.master_serial);
                self.draft.show(ui);
                if self.draft.read_point().is_some() {
                    if ui
                        .add_enabled(
                            read_blocked.is_none(),
                            egui::Button::new("Read current value"),
                        )
                        .on_disabled_hover_text(read_blocked.unwrap_or_default())
                        .clicked()
                    {
                        action = Some(Action::ReadCurrent);
                    }
                    self.show_current_value(ui);
                }
                if ui
                    .add_enabled(
                        blocked.is_none(),
                        egui::Button::new("Preview write offline"),
                    )
                    .clicked()
                {
                    match self
                        .draft
                        .operation()
                        .and_then(|operation| Plan::new(profile, &self.master_serial, operation))
                    {
                        Ok(plan) => self.stage(plan),
                        Err(error) => self.error = Some(format!("{error:#}")),
                    }
                }
            }
        }
        if let Some(error) = &self.error {
            ui.colored_label(WARNING, error);
        }
        if let Some(result) = &self.result {
            ui.separator();
            ui.strong("Last write result");
            ui.label(&self.dispatched_target);
            ui.colored_label(
                if result.unconfirmed || result.receipt.is_none() {
                    WARNING
                } else {
                    ACCENT
                },
                result.summary,
            );
            if let Some(detail) = &result.detail {
                ui.label(detail);
            }
            if let Some(receipt) = &result.receipt {
                ui.collapsing("Exact write receipt", |ui| {
                    ui.monospace(receipt);
                });
            }
        }
        action
    }

    fn show_current_value(&mut self, ui: &mut Ui) {
        let Some(current) = self.current_for_draft() else {
            return;
        };
        let value = current.value_text();
        let use_value = ui
            .horizontal_wrapped(|ui| {
                ui.label(format!("Observed value · {value}"))
                    .on_hover_text(format!(
                        "{} · Port {} · {}",
                        current.endpoint, current.port, current.observed_utc
                    ));
                ui.button("Use as expected value").clicked()
            })
            .inner;
        if use_value {
            self.draft.expected = value;
        }
    }
}

fn show_review(ui: &mut Ui, plan: &Plan) {
    let review = plan.review();
    ui.strong(format!(
        "{} · {} · Port {}",
        review.model, review.endpoint, review.port
    ));
    ui.label(format!(
        "Firmware {} · master serial {}",
        review.firmware, review.master_serial
    ));
    if let Some(identity) = &review.expected_device {
        ui.label(format!(
            "Expected Vendor ID {} · Device ID {}",
            super::optional_number(identity.vendor_id),
            super::optional_number(identity.device_id)
        ));
        for (pin, value) in [
            ("Product ID", &identity.product_id),
            ("Product name", &identity.product_name),
            ("Device serial", &identity.serial),
        ] {
            if let Some(value) = value {
                ui.label(format!("{pin} · {value}"));
            }
        }
    }
    if let Some(mode) = review.numeric_io_link_mode {
        ui.label(format!("Expected numeric IO-Link mode · {mode}"));
    }
    if let Some(expected) = &review.expected_value {
        ui.label(format!("Expected current value · {expected}"));
    }
    if let Some(meaning) = &review.command_meaning {
        ui.label(format!("Command meaning · {meaning}"));
    }
    ui.strong("Proposed request");
    ui.monospace(&review.address);
    ui.monospace(review.request_data.to_string());
    ui.small("Apply checks the target and sends one write. A lost response leaves the outcome unconfirmed.");
}

impl App {
    fn write_block_reason(&self) -> Option<&'static str> {
        self.start_block_reason()
            .or_else(|| self.current_read_block_reason())
    }

    fn current_read_block_reason(&self) -> Option<&'static str> {
        if self.closing {
            Some("Application is closing")
        } else if self.writes.writing() {
            Some("A reviewed write is in progress")
        } else if self.writes.read_worker.is_some() {
            Some("Reading the current value…")
        } else if self.acyclic.busy() {
            Some("Wait for the active parameter read")
        } else if self.setup.pending() || self.setup.busy() {
            Some("Finish inspection and apply or discard profile edits")
        } else if self.profile.connection.gateway != Gateway::IfmAl1350 {
            Some("Apply an AL1350 profile to prepare a write")
        } else {
            None
        }
    }

    pub(super) fn write_page(&mut self, ui: &mut Ui) {
        let blocked = self.write_block_reason();
        let read_blocked = self.current_read_block_reason();
        let result = match self.writes.show(ui, &self.profile, blocked, read_blocked) {
            Some(Action::ReadCurrent) => self.start_current_read(),
            Some(Action::Apply) => self.start_reviewed_write(),
            None => Ok(()),
        };
        if let Err(error) = result {
            self.writes.error = Some(format!("{error:#}"));
        }
    }

    fn start_current_read(&mut self) -> Result<()> {
        if let Some(reason) = self.current_read_block_reason() {
            anyhow::bail!(reason);
        }
        let point = self
            .writes
            .draft
            .read_point()
            .context("Choose a readable write target")?;
        let read = CurrentRead::new(&self.profile, &self.writes.master_serial, point)?;
        let worker = background("al1350-current-value", move || read.read())
            .context("Could not start the current-value read")?;
        self.writes.clear_review();
        self.writes.current = None;
        self.writes.read_worker = Some(worker);
        self.message = "Reading the current write target value…".into();
        Ok(())
    }

    fn start_reviewed_write(&mut self) -> Result<()> {
        if let Some(reason) = self.write_block_reason() {
            anyhow::bail!(reason);
        }
        ensure!(self.writes.approved, "Approve the reviewed write first");
        let plan = self
            .writes
            .review
            .take()
            .context("Preview the write first")?;
        self.writes.approved = false;
        let review = plan.review();
        let target = format!(
            "{} · {} · Port {} · master serial {}",
            review.model, review.endpoint, review.port, review.master_serial
        );
        let worker = background("reviewed-al1350-write", move || WriteResult::execute(plan))
            .context("Could not start write worker")?;
        self.writes.worker = Some(worker);
        self.writes.error = None;
        self.writes.result = None;
        self.writes.current = None;
        self.writes.dispatched_target = target;
        self.live = Default::default();
        self.message = "Checking the target and applying the reviewed write…".into();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
