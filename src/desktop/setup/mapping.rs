use eframe::egui::{self, Ui};
use iolink_workbench::{
    decode::{self, ByteOrder, Field, Kind, Value},
    profile::Profile,
};

#[derive(Default)]
pub struct Form {
    baseline: Option<Field>,
    draft: Option<Field>,
    create_override: bool,
    preview: String,
}

impl Form {
    pub fn edit(&mut self, field: Field, manual: bool) {
        self.baseline = Some(field.clone());
        self.draft = Some(field);
        self.create_override = !manual;
    }

    pub fn new_field(&mut self, profile: &Profile) {
        let fields = profile.effective_fields();
        let name = (1..)
            .map(|index| format!("field_{index}"))
            .find(|name| fields.iter().all(|(field, _)| field.name != *name))
            .expect("unused field name");
        self.baseline = None;
        self.draft = Some(Field {
            name,
            ..Default::default()
        });
        self.create_override = false;
    }

    pub fn pending(&self) -> bool {
        self.draft.is_some() && (self.create_override || self.draft != self.baseline)
    }

    pub fn sync(&mut self, profile: &Profile) {
        let name = self.draft.as_ref().map(|field| field.name.clone());
        self.baseline = name.and_then(|name| {
            profile
                .effective_fields()
                .into_iter()
                .find(|(field, _)| field.name == name)
                .map(|(field, _)| field.clone())
        });
        self.draft = self.baseline.clone();
        self.create_override = false;
    }

    pub fn show(&mut self, ui: &mut Ui, profile: &mut Profile, message: &mut String) -> bool {
        let Some(field) = &mut self.draft else {
            return false;
        };
        ui.heading("Manual field / IODD override");
        ui.small("This edits host decoding only. Matching names override IODD fields; imported definitions remain saved. Check the complete payload, bit layout, scaling and units against the device documentation.");
        egui::Grid::new("manual-mapping-editor")
            .num_columns(2)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.label("Field name");
                ui.add_enabled(
                    self.baseline.is_none(),
                    egui::TextEdit::singleline(&mut field.name),
                );
                ui.end_row();
                ui.label("Bit offset");
                ui.add(
                    egui::DragValue::new(&mut field.bit_offset)
                        .range(0..=profile.input_bytes * 8 - 1),
                );
                ui.end_row();
                ui.label("Bit width");
                ui.add(egui::DragValue::new(&mut field.bits).range(1..=64));
                ui.end_row();
                ui.label("Type");
                egui::ComboBox::from_id_salt("manual-field-kind")
                    .selected_text(format!("{:?}", field.kind))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut field.kind, Kind::Unsigned, "Unsigned");
                        ui.selectable_value(&mut field.kind, Kind::Signed, "Signed");
                        ui.selectable_value(
                            &mut field.kind,
                            Kind::Bool,
                            "Boolean (1 bit, unscaled)",
                        );
                        ui.selectable_value(&mut field.kind, Kind::Float32, "Float32 (32 bits)");
                    });
                ui.end_row();
                ui.label("Byte order");
                egui::ComboBox::from_id_salt("manual-field-order")
                    .selected_text(format!("{:?}", field.byte_order))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut field.byte_order,
                            ByteOrder::BigEndian,
                            "Big endian · bit 0 in last byte",
                        );
                        ui.selectable_value(
                            &mut field.byte_order,
                            ByteOrder::LittleEndian,
                            "Little endian · bit 0 in first byte",
                        );
                    });
                ui.end_row();
                ui.label("Scale");
                ui.add(egui::DragValue::new(&mut field.scale).speed(0.001));
                ui.end_row();
                ui.label("Offset");
                ui.add(egui::DragValue::new(&mut field.offset).speed(0.001));
                ui.end_row();
                ui.label("Unit");
                ui.text_edit_singleline(&mut field.unit);
                ui.end_row();
                ui.label("Decimal places");
                ui.horizontal(|ui| {
                    let mut enabled = field.decimals.is_some();
                    if ui.checkbox(&mut enabled, "Fixed precision").changed() {
                        field.decimals = enabled.then_some(0);
                    }
                    if let Some(places) = &mut field.decimals {
                        ui.add(egui::DragValue::new(places).range(0..=12));
                    }
                });
                ui.end_row();
            });
        if let Err(error) = field.validate(profile.input_bytes) {
            ui.colored_label(super::WARNING, format!("Needs input · {error:#}"));
        }
        ui.label("Offline preview · complete input payload (hex)");
        ui.text_edit_singleline(&mut self.preview);
        if !self.preview.trim().is_empty() {
            match preview(field, profile.input_bytes, &self.preview) {
                Ok(value) => {
                    ui.monospace(format!(
                        "Raw {} · bits 0x{} · value {} {}",
                        value.raw, value.bits_hex, value.value, field.unit
                    ));
                }
                Err(error) => {
                    ui.colored_label(super::WARNING, format!("Preview · {error:#}"));
                }
            }
        }
        let field = field.clone();
        let mut changed = false;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(self.pending(), egui::Button::new("Apply mapping"))
                .clicked()
            {
                match profile.with_field(field.clone()) {
                    Ok(candidate) => {
                        *profile = candidate;
                        self.sync(profile);
                        *message = "Host decoding mapping applied locally".into();
                        changed = true;
                    }
                    Err(error) => *message = format!("{error:#}"),
                }
            }
            if ui
                .add_enabled(self.pending(), egui::Button::new("Discard mapping edits"))
                .clicked()
            {
                self.draft = self.baseline.clone();
                self.create_override = false;
                message.clear();
            }
            let removable = profile
                .fields
                .iter()
                .any(|candidate| candidate.name == field.name);
            if ui
                .add_enabled(
                    removable && !self.pending(),
                    egui::Button::new("Remove manual mapping / restore IODD"),
                )
                .clicked()
            {
                match profile.without_manual_field(&field.name) {
                    Ok(candidate) => {
                        *profile = candidate;
                        self.sync(profile);
                        *message =
                            "Manual mapping removed; any imported IODD field is restored".into();
                        changed = true;
                    }
                    Err(error) => *message = format!("{error:#}"),
                }
            }
            if ui
                .add_enabled(!self.pending(), egui::Button::new("Close editor"))
                .clicked()
            {
                self.draft = None;
                self.baseline = None;
            }
            if self.pending() {
                ui.colored_label(super::WARNING, "Pending mapping edits");
            }
        });
        changed
    }
}

fn preview(field: &Field, input_bytes: usize, text: &str) -> anyhow::Result<Value> {
    let payload = decode::parse_hex(text)?;
    anyhow::ensure!(
        payload.len() == input_bytes,
        "expected the complete {input_bytes}-byte input"
    );
    field.decode(&payload)
}

#[cfg(test)]
mod tests;
