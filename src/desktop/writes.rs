use super::parameters::Picker;
use super::{ACCENT, App, Gateway, Profile, WARNING, Worker, background, completed};
use anyhow::{Context, Result, ensure};
use eframe::egui::{self, Ui};
use iolink_workbench::{
    iodd::{AssessmentStatus, DecodedParameter},
    transport::write::{
        CurrentRead, CurrentValue, IsduComparison, IsduValueReview, Operation, Outcome, Plan,
        ReadPoint,
    },
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
    typed_desired: bool,
    member_subindex: u8,
    output_field: Option<String>,
    empty_text: bool,
    index: u16,
    subindex: u8,
    iodd_picker: Picker,
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

    fn operation(&self, profile: &Profile) -> Result<Operation> {
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
            Kind::Parameter if self.typed_desired => {
                ensure!(
                    !self.desired.is_empty() || self.empty_text,
                    "Enter a desired IODD value"
                );
                if self.member_subindex > 0 {
                    ensure!(
                        self.subindex == 0,
                        "member editing requires a whole-parameter write at subindex 0"
                    );
                    Operation::isdu_member(
                        profile,
                        self.index,
                        self.member_subindex,
                        self.expected.clone(),
                        &self.desired,
                    )?
                } else {
                    Operation::isdu_typed(
                        profile,
                        self.index,
                        self.subindex,
                        self.expected.clone(),
                        &self.desired,
                    )?
                }
            }
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
            Kind::Output if self.typed_desired => Operation::process_output_field(
                profile,
                self.expected.clone(),
                self.output_field
                    .as_deref()
                    .context("Choose an IODD output field")?,
                &self.desired,
            )?,
            Kind::Output => Operation::ProcessOutput {
                expected: self.expected.clone(),
                desired: self.desired.clone(),
            },
        })
    }

    fn reset_values(&mut self) {
        self.expected.clear();
        self.desired.clear();
        self.typed_desired = false;
        self.member_subindex = 0;
        self.output_field = None;
        self.empty_text = false;
    }

    fn set_parameter_address(&mut self, address: (u16, u8)) {
        if (self.index, self.subindex) != address {
            (self.index, self.subindex) = address;
            self.reset_values();
        }
    }

    fn show(&mut self, ui: &mut Ui, profile: &Profile) {
        let before = self.kind;
        egui::ComboBox::from_id_salt("write-operation")
            .selected_text(self.kind.label())
            .show_ui(ui, |ui| {
                for kind in [Kind::Mode, Kind::Parameter, Kind::Command, Kind::Output] {
                    if kind != Kind::Output || !profile.acyclic_only() {
                        ui.selectable_value(&mut self.kind, kind, kind.label());
                    }
                }
            });
        if self.kind != before {
            self.reset_values();
        }
        match self.kind {
            Kind::Mode => {
                ui.small("Use the numeric mode codes confirmed for this firmware.");
                self.values(ui, "Expected mode", "Desired mode");
            }
            Kind::Parameter => self.show_parameter(ui, profile),
            Kind::Command => {
                ui.add(
                    egui::DragValue::new(&mut self.code).prefix("SystemCommand code (decimal) "),
                );
                text_entry(ui, "Documented command meaning", &mut self.meaning);
            }
            Kind::Output => self.show_output(ui, profile),
        }
    }

    fn show_output(&mut self, ui: &mut Ui, profile: &Profile) {
        use iolink_workbench::decode::Kind as FieldKind;
        let fields: Vec<_> = profile
            .output_layout()
            .into_iter()
            .flat_map(|layout| layout.editable_output_fields())
            .collect();
        let was_typed = self.typed_desired;
        if fields.is_empty() {
            self.typed_desired = false;
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.typed_desired, false, "Raw hex");
                ui.selectable_value(&mut self.typed_desired, true, "Edit one field");
            });
        }
        if was_typed != self.typed_desired {
            self.output_field = None;
            self.desired.clear();
        }
        text_entry(ui, "Expected output hex", &mut self.expected);
        if !self.typed_desired {
            text_entry(ui, "Desired output hex", &mut self.desired);
            return;
        }
        let previous = self.output_field.clone();
        egui::ComboBox::from_id_salt("write-output-field")
            .selected_text(previous.as_deref().unwrap_or("Select a field"))
            .show_ui(ui, |ui| {
                for field in &fields {
                    ui.selectable_value(
                        &mut self.output_field,
                        Some(field.name.clone()),
                        &field.name,
                    );
                }
            });
        if previous != self.output_field {
            self.desired.clear();
        }
        if let Some(field) = fields
            .iter()
            .find(|field| Some(field.name.as_str()) == self.output_field.as_deref())
        {
            let label = match field.kind {
                FieldKind::Bool => "Desired Boolean (true or false)",
                FieldKind::Float32 => "Desired Float32 (unscaled)",
                FieldKind::Signed | FieldKind::Unsigned => "Desired integer (unscaled decimal)",
                FieldKind::OctetString | FieldKind::Utf8String | FieldKind::AsciiString => {
                    "Edit string fields with a complete output image"
                }
                FieldKind::Time | FieldKind::TimeSpan => {
                    "Edit temporal fields with a complete output image"
                }
            };
            text_entry(ui, label, &mut self.desired);
            if let Some(layout) = profile.output_layout() {
                if layout.standalone_boolean {
                    ui.small("Standalone Boolean · 00 false · FF true");
                } else if layout.padded_integer {
                    ui.small("Preview extends the integer to its full wire width.");
                } else {
                    ui.small("Preview builds the full output image and preserves all other bits.");
                }
            }
        }
    }

    fn show_parameter(&mut self, ui: &mut Ui, profile: &Profile) {
        let mut address = (self.index, self.subindex);
        if profile
            .iodd
            .as_ref()
            .is_some_and(|iodd| !iodd.parameters.is_empty())
        {
            ui.collapsing("Choose from IODD", |ui| {
                if let Some(selected) = self.iodd_picker.show_for_write(ui, profile)
                    && ui.button("Use selected address").clicked()
                {
                    address = selected;
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::DragValue::new(&mut address.0).prefix("Index "));
            ui.add(egui::DragValue::new(&mut address.1).prefix("Subindex "));
        });
        self.set_parameter_address(address);
        text_entry(ui, "Expected parameter hex", &mut self.expected);
        let point = ReadPoint::Isdu {
            index: self.index,
            subindex: self.subindex,
        };
        let previous = self.typed_desired;
        let parameter = point.typed_isdu_parameter(profile);
        let members = if parameter.is_none() {
            point.typed_isdu_members(profile)
        } else {
            Vec::new()
        };
        if let Some(parameter) = parameter {
            ui.small(format!("{} · {}", parameter.name, parameter.datatype));
        }
        if parameter.is_some() || !members.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.typed_desired, false, "Raw hex");
                let label = if members.is_empty() {
                    "IODD value"
                } else {
                    "Edit one member"
                };
                ui.selectable_value(&mut self.typed_desired, true, label);
            });
        } else {
            self.typed_desired = false;
        }
        if previous != self.typed_desired {
            self.desired.clear();
            self.empty_text = false;
            self.member_subindex = 0;
        }
        let typed = if self.typed_desired && !members.is_empty() {
            let before = self.member_subindex;
            let member_label = |subindex: u8, parameter: &iolink_workbench::iodd::Parameter| {
                format!("{} · {} · {}", subindex, parameter.name, parameter.datatype)
            };
            let current = members
                .iter()
                .find(|(subindex, _)| *subindex == before)
                .map_or_else(
                    || "Select a member".into(),
                    |&(subindex, parameter)| member_label(subindex, parameter),
                );
            egui::ComboBox::from_id_salt("write-image-member")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for &(subindex, parameter) in &members {
                        ui.selectable_value(
                            &mut self.member_subindex,
                            subindex,
                            member_label(subindex, parameter),
                        );
                    }
                });
            ui.small("The request writes the complete image at subindex 0.");
            if before != self.member_subindex {
                self.desired.clear();
                self.empty_text = false;
            }
            let Some((_, member)) = members
                .into_iter()
                .find(|(subindex, _)| *subindex == self.member_subindex)
            else {
                return;
            };
            Some(member)
        } else {
            parameter.filter(|_| self.typed_desired)
        };
        let datatype = typed.map(|parameter| parameter.datatype.as_str());
        let label = match datatype {
            Some("BooleanT") => "Desired Boolean (true or false)",
            Some("Float32T") => "Desired Float32 (unscaled)",
            Some("TimeT") => "Desired UTC time",
            Some("TimeSpanT") => "Desired duration",
            Some("StringT") => "Desired text",
            Some("OctetStringT") => "Desired bytes (0xNN,0xNN)",
            Some(_) => "Desired integer (unscaled decimal)",
            None => "Desired parameter hex",
        };
        text_entry(ui, label, &mut self.desired);
        match datatype {
            Some("TimeT") => {
                ui.small("yyyy-mm-ddThh:mm:ss[.fff]Z · up to millisecond precision");
            }
            Some("TimeSpanT") => {
                ui.small("[+-]PTseconds[.fff]S · up to millisecond precision");
            }
            _ => {}
        }
        if let Some(parameter) = typed
            && let Some(length) = parameter
                .fixed_length
                .filter(|_| matches!(datatype, Some("StringT" | "OctetStringT")))
        {
            if datatype == Some("StringT") {
                let encoding = parameter.encoding.as_deref().unwrap_or_default();
                ui.small(format!("{encoding} · {length} bytes including padding"));
            } else {
                ui.small(format!("Enter exactly {length} bytes"));
            }
        }
        if datatype == Some("StringT") && self.desired.is_empty() {
            ui.checkbox(&mut self.empty_text, "Write empty text");
        } else {
            self.empty_text = false;
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
                Outcome::TextReadbackMatched => "Gateway readback matched the IODD text",
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
                let previous_point = self.draft.read_point();
                self.draft.show(ui, profile);
                if self.draft.read_point() != previous_point {
                    self.error = None;
                }
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
                        .operation(profile)
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
        if let Some(iodd) = &current.iodd_value {
            let decoded = &iodd.interpretation;
            let observed = iodd_value_summary(decoded, &iodd.datatype);
            let label = match observed {
                Some(value) => format!("Observed IODD · {} · {value}", iodd.name),
                None => format!("Observed IODD · {} · {}", iodd.name, iodd.datatype),
            };
            ui.label(label);
            if decoded.needs_review() {
                ui.colored_label(WARNING, "Review the observed IODD decoding details.");
            } else if decoded
                .value_assessment
                .as_ref()
                .is_some_and(|value| value.status == AssessmentStatus::OutsideDefinition)
            {
                ui.colored_label(
                    WARNING,
                    "Observed value is outside the IODD choices or ranges.",
                );
            }
            ui.collapsing("Observed IODD details", |ui| ui.monospace(decoded.text()));
        }
        if let Some(output) = &current.iodd_output {
            super::payload_inspector::show_output_fields(ui, output);
        }
        if use_value {
            self.draft.expected = value;
        }
    }
}

fn iodd_value_summary(decoded: &DecodedParameter, datatype: &str) -> Option<String> {
    if let Some(time) = &decoded.time_value {
        return Some(time.value.clone());
    }
    if let Some(display) = decoded
        .preferred_display
        .as_ref()
        .or_else(|| decoded.displays.first())
    {
        return Some(if display.unit.is_empty() {
            display.value.clone()
        } else {
            format!("{} {}", display.value, display.unit)
        });
    }
    decoded.raw_value.as_ref().map(|value| {
        if datatype == "StringT" {
            format!("{value:?}")
        } else {
            value.clone()
        }
    })
}

fn show_isdu_comparison(
    ui: &mut Ui,
    comparison: &IsduComparison,
    proposed: &DecodedParameter,
    datatype: &str,
) {
    let expected_value = iodd_value_summary(&comparison.expected, datatype);
    let proposed_value = iodd_value_summary(proposed, datatype);
    if expected_value.is_some() || proposed_value.is_some() {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "Expected IODD · {}",
                expected_value.as_deref().unwrap_or("see raw bytes")
            ));
            ui.label(format!(
                "Proposed IODD · {}",
                proposed_value.as_deref().unwrap_or("see raw bytes")
            ));
        });
    }
    if comparison.expected.needs_review() {
        ui.colored_label(WARNING, "Review the expected IODD decoding details.");
    }
    if !comparison.changed_members.is_empty() {
        ui.collapsing(
            format!(
                "Changed IODD members ({})",
                comparison.changed_members.len()
            ),
            |ui| {
                for member in &comparison.changed_members {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!(
                            "Subindex {} · {} · bit {}",
                            member.subindex, member.name, member.bit_offset
                        ));
                        ui.monospace(format!("{} → {}", member.expected_hex, member.proposed_hex));
                    });
                }
            },
        );
    }
    ui.collapsing("IODD value details", |ui| {
        ui.strong("Expected");
        ui.monospace(comparison.expected.text());
        ui.separator();
        ui.strong("Proposed");
        ui.monospace(proposed.text());
    });
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
    if let Some(ReadPoint::Isdu { index, subindex }) = plan.read_point() {
        ui.label(format!("ISDU index {index} · subindex {subindex}"));
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
    match &review.isdu_value_review {
        Some(IsduValueReview::NoIoddParameter) => {
            ui.colored_label(
                WARNING,
                "No IODD definition for this address. Review the raw bytes manually.",
            );
        }
        Some(IsduValueReview::Iodd {
            name,
            datatype,
            interpretation,
            manual_review,
            preserved_read_only,
            ..
        }) => {
            ui.label(format!("IODD · {name} · {datatype}"));
            if interpretation.time_value.is_some() {
                ui.small("Typed input uses milliseconds. Fractions round by less than one tick. See exact ticks in the details.");
            }
            if !preserved_read_only.is_empty() {
                let subindexes = preserved_read_only
                    .iter()
                    .map(u8::to_string)
                    .collect::<Vec<_>>();
                ui.label(format!(
                    "Read-only subindexes preserved · {}",
                    subindexes.join(", ")
                ));
            }
            if let Some(note) = manual_review {
                ui.colored_label(WARNING, note);
            }
            if let Some(comparison) = plan.isdu_comparison() {
                show_isdu_comparison(ui, comparison, interpretation, datatype);
            }
        }
        None => {}
    }
    if let Some(output) = &review.process_output_review {
        ui.label(format!(
            "IODD output · {} · {} bits",
            output.layout_id, output.bit_length
        ));
        if let Some(condition) = &output.condition {
            ui.small(format!("Confirmed layout condition · {condition}"));
        }
        if output.standalone_boolean {
            ui.small("Standalone Boolean · 00 false · FF true");
        }
        if output.padded_integer {
            ui.small("Standalone integer · signed values extend the sign bit · unsigned values extend with zeros");
        }
        if let Some(warning) = &output.expected_encoding_warning {
            ui.colored_label(WARNING, warning);
        }
        if !output.changed_unmapped_bits.is_empty() {
            let bits = output
                .changed_unmapped_bits
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>();
            ui.colored_label(
                WARNING,
                format!(
                    "Review changes to unmapped output bits · {}",
                    bits.join(", ")
                ),
            );
        }
        let changes = output
            .fields
            .iter()
            .filter(|entry| entry.expected.bits_hex != entry.proposed.bits_hex)
            .count();
        ui.collapsing(format!("IODD output fields ({changes} changed)"), |ui| {
            for entry in &output.fields {
                let field = &entry.field;
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{} · bit {} · {} bits · {:?} · {:?}",
                        field.name, field.bit_offset, field.bits, field.kind, field.byte_order
                    ));
                    ui.monospace(format!(
                        "{} → {} {} (raw {} → {})",
                        entry.expected.value,
                        entry.proposed.value,
                        field.unit,
                        entry.expected.raw,
                        entry.proposed.raw
                    ));
                });
            }
            for note in &output.needs_input {
                ui.colored_label(WARNING, note);
            }
        });
    } else if review.operation == "process_output" {
        ui.colored_label(
            WARNING,
            "No imported output layout. Review the full wire image manually.",
        );
    }
    if review.text_readback.is_some() {
        ui.small("Readback accepts the same IODD text with different zero padding.");
    }
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
