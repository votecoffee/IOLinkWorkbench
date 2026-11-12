use super::WARNING;
use eframe::egui::{self, Ui};
use iolink_workbench::{iodd::Access, profile::Profile};

pub fn show(
    ui: &mut Ui,
    profile: &Profile,
    selected: &mut Option<String>,
    available: bool,
) -> Option<String> {
    let mut requested = None;
    ui.collapsing("IODD parameters", |ui| {
        let Some(imported) = &profile.iodd else {
            ui.label("Import an IODD in Setup to browse named parameters.");
            return;
        };
        ui.small("IODD defaults and permitted values describe the device. Read a selected parameter to obtain its current raw value.");
        ui.add_enabled_ui(available, |ui| {
            egui::ComboBox::from_id_salt("iodd-parameter").selected_text(selected.as_deref().unwrap_or("Select parameter"))
                .show_ui(ui, |ui| {
                    for parameter in &imported.parameters {
                        ui.selectable_value(selected, Some(parameter.id.clone()),
                            format!("{} · {} · index {}", parameter.name, parameter.access.label(), parameter.index));
                    }
                });
            if let Some(parameter) = imported.parameters.iter().find(|parameter| Some(&parameter.id) == selected.as_ref()) {
                ui.label(format!("{} · {} · index {}:0 · access {}",
                    parameter.id, parameter.datatype, parameter.index, parameter.access.label()));
                if let Some(bits) = parameter.bits { ui.label(format!("Declared width · {bits} bits")); }
                if let Some(length) = parameter.fixed_length { ui.label(format!("Declared length · {length} bytes")); }
                if let Some(encoding) = &parameter.encoding { ui.label(format!("Encoding · {encoding}")); }
                if let Some(default) = &parameter.default_value { ui.label(format!("IODD default · {default}")); }
                if let Some(origin) = &parameter.origin { ui.small(format!("Standard definition · {origin}")); }
                for display in &parameter.displays {
                    let field = &display.field;
                    ui.label(format!("Display {} · ×{} + {} {} · decimals {:?}", display.id, field.scale, field.offset, field.unit, field.decimals));
                }
                for range in &parameter.ranges { ui.label(format!("IODD range · {} through {}", range.lower, range.upper)); }
                for choice in &parameter.choices { ui.label(format!("{} · {}", choice.value, choice.name)); }
                for item in &parameter.needs_input { ui.colored_label(WARNING, item); }
                if ui.add_enabled(parameter.access != Access::WriteOnly, egui::Button::new("Read named parameter")).clicked() {
                    requested = Some(parameter.id.clone());
                }
            }
        });
        if !imported.standard_parameter_refs.is_empty() {
            ui.collapsing("Standard parameter references", |ui| {
                ui.small("These references require standard definitions. The numeric ISDU read accepts a documented index and subindex.");
                for id in &imported.standard_parameter_refs { ui.monospace(id); }
            });
        }
    });
    requested
}
