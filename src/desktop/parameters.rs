use super::WARNING;
use eframe::egui::{self, Ui};
use iolink_workbench::{
    decode,
    iodd::{Access, Parameter},
    profile::Profile,
};

fn picker<'a>(
    ui: &mut Ui,
    profile: &'a Profile,
    selected: &mut Option<String>,
    subindex: &mut u8,
    salt: &str,
) -> Option<(&'a Parameter, &'a Parameter)> {
    let Some(imported) = &profile.iodd else {
        ui.label("Import an IODD in Setup to browse named parameters.");
        return None;
    };
    let before = selected.clone();
    egui::ComboBox::from_id_salt(salt)
        .selected_text(selected.as_deref().unwrap_or("Select parameter"))
        .show_ui(ui, |ui| {
            for parameter in &imported.parameters {
                ui.selectable_value(
                    selected,
                    Some(parameter.id.clone()),
                    format!(
                        "{} · {} · index {}",
                        parameter.name,
                        parameter.access.label(),
                        parameter.index
                    ),
                );
            }
        });
    if *selected != before {
        *subindex = 0;
    }
    let parent = imported
        .parameters
        .iter()
        .find(|p| Some(&p.id) == selected.as_ref())?;
    if let Some(record) = &parent.record {
        ui.label(format!(
            "Individual subindex access · {}",
            record.subindex_access
        ));
        egui::ComboBox::from_id_salt((salt, "subindex"))
            .selected_text(format!("Subindex {subindex}"))
            .show_ui(ui, |ui| {
                ui.selectable_value(subindex, 0, "0 · whole record");
                for item in &record.items {
                    ui.selectable_value(
                        subindex,
                        item.subindex,
                        format!("{} · {}", item.subindex, item.parameter.name),
                    );
                }
            });
        ui.collapsing("Record layout", |ui| {
            for item in &record.items {
                ui.label(format!(
                    "Subindex {} · bit {} · width {:?} · {} · {}",
                    item.subindex,
                    item.bit_offset,
                    item.width(),
                    item.parameter.datatype,
                    item.parameter.access.label()
                ));
                for note in &item.parameter.needs_input {
                    ui.colored_label(WARNING, note);
                }
            }
        });
    } else if let Some(array) = &parent.array {
        ui.label(format!(
            "Array · {} elements · {:?} bits each · {} · individual subindex access {}",
            array.count,
            array.element.width(),
            array.element.datatype,
            array.subindex_access
        ));
        egui::ComboBox::from_id_salt((salt, "subindex"))
            .selected_text(format!("Subindex {subindex}"))
            .show_ui(ui, |ui| {
                ui.selectable_value(subindex, 0, "0 · whole array");
                for element in 1..=array.count {
                    ui.selectable_value(subindex, element, format!("Element {element}"));
                }
            });
        for note in &array.element.needs_input {
            ui.colored_label(WARNING, note);
        }
        if *subindex == 0 {
            for display in &array.element.displays {
                ui.label(format!(
                    "Each element · ×{} + {} {}",
                    display.field.scale, display.field.offset, display.field.unit
                ));
            }
        }
    } else {
        *subindex = 0;
    }
    let target = parent.target(*subindex).ok()?;
    ui.label(format!(
        "{} · {} · index {}:{} · access {}",
        target.name,
        target.datatype,
        parent.index,
        subindex,
        target.access.label()
    ));
    if let Some(bits) = target.bits {
        ui.label(format!("Declared width · {bits} bits"));
    }
    if let Some(length) = target.fixed_length {
        ui.label(format!("Declared length · {length} bytes"));
    }
    if let Some(encoding) = &target.encoding {
        ui.label(format!("Encoding · {encoding}"));
    }
    if let Some(default) = &target.default_value {
        ui.label(format!("IODD default · {default}"));
    }
    if let Some(origin) = &target.origin {
        ui.small(format!("Standard definition · {origin}"));
    }
    for display in &target.displays {
        let field = &display.field;
        ui.label(format!(
            "Display {} · ×{} + {} {} · decimals {:?}",
            display.id, field.scale, field.offset, field.unit, field.decimals
        ));
    }
    for range in &target.ranges {
        ui.label(format!(
            "IODD range · {} through {}",
            range.lower, range.upper
        ));
    }
    for choice in &target.choices {
        ui.label(format!("{} · {}", choice.value, choice.name));
    }
    for note in &target.needs_input {
        ui.colored_label(WARNING, note);
    }
    Some((parent, target))
}

pub fn show(
    ui: &mut Ui,
    profile: &Profile,
    selected: &mut Option<String>,
    subindex: &mut u8,
    available: bool,
) -> Option<(String, u8)> {
    let mut requested = None;
    ui.collapsing("IODD parameters", |ui| {
        ui.small("IODD defaults and permitted values describe the device. Reads use a separate acyclic client.");
        ui.add_enabled_ui(available, |ui| {
            if let Some((parent, target)) = picker(ui, profile, selected, subindex, "iodd-read-parameter") {
                let allowed = parent.access != Access::WriteOnly && target.access != Access::WriteOnly
                    && (*subindex == 0 || parent.subindex_access());
                if ui.add_enabled(allowed, egui::Button::new("Read named parameter")).clicked() {
                    requested = Some((parent.id.clone(), *subindex));
                }
            }
        });
        if let Some(imported) = &profile.iodd && !imported.standard_parameter_refs.is_empty() {
            ui.collapsing("Standard parameter references", |ui| {
                ui.small("These references require local standard definitions. The numeric ISDU read accepts a documented index and subindex.");
                for id in &imported.standard_parameter_refs { ui.monospace(id); }
            });
        }
    });
    requested
}

#[derive(Default)]
pub struct Offline {
    selected: Option<String>,
    subindex: u8,
    payload: String,
}

impl Offline {
    pub fn show(&mut self, ui: &mut Ui, profile: &Profile) {
        ui.heading("Offline ISDU parameter decoder");
        ui.small("Paste a recorded 1–232 byte response. Select subindex 0 for a whole record, or the member addressed by the recorded query.");
        if let Some((_, target)) = picker(
            ui,
            profile,
            &mut self.selected,
            &mut self.subindex,
            "iodd-offline-parameter",
        ) {
            ui.add(
                egui::TextEdit::multiline(&mut self.payload)
                    .code_editor()
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            if !self.payload.trim().is_empty() {
                match decode::parse_hex_bounded(&self.payload, 232) {
                    Ok(bytes) => {
                        ui.monospace(target.decode(&bytes).text());
                    }
                    Err(error) => {
                        ui.colored_label(WARNING, error.to_string());
                    }
                }
            }
        }
    }
}
