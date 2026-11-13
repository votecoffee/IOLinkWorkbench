use eframe::egui::{self, Ui};
use iolink_workbench::profile::{LogSelection, Profile};

fn apply(profile: &mut Profile, selection: LogSelection, message: &mut String) {
    if let Err(error) = profile.set_log_selection(selection) {
        *message = format!("{error:#}");
    }
}

pub fn show(ui: &mut Ui, profile: &mut Profile, message: &mut String) -> bool {
    let before = profile.logging.clone();
    ui.collapsing("CSV values and order", |ui| {
        ui.small("Raw inputs and metadata are always included. Value selection controls CSV columns; On change compares the whole raw input. Save the profile in Setup to keep this selection.");
        ui.horizontal(|ui| {
            if ui.button("All mapped values").clicked() { apply(profile, LogSelection::All, message); }
            if ui.button("Raw only").clicked() { apply(profile, LogSelection::RawOnly, message); }
            ui.label(if profile.logging.raw_only { "Raw inputs and metadata only" } else if profile.logging.fields.is_empty() { "All mapped values in mapping order" } else { "Selected values in saved order" });
        });
        let fields: Vec<_> = profile.effective_fields().into_iter().map(|(field, origin)| (field.clone(), origin)).collect();
        if fields.is_empty() { ui.label("No mapped values are available. Add a mapping or import an IODD in Setup."); }
        egui::Grid::new("csv-field-selection").striped(true).show(ui, |ui| {
            for (field, origin) in &fields {
                let mut selected = profile.log_fields().iter().any(|candidate| candidate.name == field.name);
                if ui.checkbox(&mut selected, &field.name).on_hover_text(format!("{origin} · bit {} · {} bits · {:?} · {:?} · ×{} + {}", field.bit_offset, field.bits, field.kind, field.byte_order, field.scale, field.offset)).changed() {
                    let mut names: Vec<_> = profile.log_fields().into_iter().map(|field| field.name.clone()).collect();
                    if selected { names.push(field.name.clone()); } else { names.retain(|name| name != &field.name); }
                    let selection = if names.is_empty() { LogSelection::RawOnly } else { LogSelection::Fields(names) };
                    apply(profile, selection, message);
                }
                ui.label(&field.unit);
                ui.small(*origin);
                ui.end_row();
            }
        });
        let names: Vec<_> = profile.log_fields().into_iter().map(|field| field.name.clone()).collect();
        if !names.is_empty() {
            ui.label("CSV value column order");
            for (index, name) in names.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("{}. {name}", index + 1));
                    if ui.add_enabled(index > 0, egui::Button::new("Up")).clicked() {
                        let mut reordered = names.clone();
                        reordered.swap(index, index - 1);
                        apply(profile, LogSelection::Fields(reordered), message);
                    }
                    if ui.add_enabled(index + 1 < names.len(), egui::Button::new("Down")).clicked() {
                        let mut reordered = names.clone();
                        reordered.swap(index, index + 1);
                        apply(profile, LogSelection::Fields(reordered), message);
                    }
                });
            }
        }
    });
    before != profile.logging
}
