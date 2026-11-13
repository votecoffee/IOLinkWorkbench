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
                    "Each element · {} · {} · ×{} + {} {}",
                    display.id,
                    display.format.label(),
                    display.field.scale,
                    display.field.offset,
                    display.field.unit
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
            "Display {} · {} · ×{} + {} {} · decimals {:?}",
            display.id,
            display.format.label(),
            field.scale,
            field.offset,
            field.unit,
            field.decimals
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

pub struct DisplaySelection {
    pub id: String,
    pub subindex: u8,
    pub display: Option<String>,
}

impl DisplaySelection {
    pub fn apply(self, profile: &mut Profile) -> anyhow::Result<()> {
        profile.set_parameter_display(&self.id, self.subindex, self.display.as_deref())
    }
}

fn preference(
    ui: &mut Ui,
    parent: &Parameter,
    target: &Parameter,
    subindex: u8,
    available: bool,
    salt: &str,
) -> Option<DisplaySelection> {
    let target = parent.array.as_ref().map_or(target, |array| &array.element);
    if target.displays.is_empty() {
        if parent.record.is_some() && subindex == 0 {
            ui.small("Select a member subindex to choose its preferred display.");
        }
        return None;
    }
    let mut chosen = target.preferred_display.clone();
    ui.add_enabled_ui(available, |ui| {
        egui::ComboBox::from_id_salt((salt, "preferred-display"))
            .selected_text(chosen.as_deref().unwrap_or("All displays · no preference"))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, None, "All displays · no preference");
                for display in &target.displays {
                    ui.selectable_value(
                        &mut chosen,
                        Some(display.id.clone()),
                        format!(
                            "{} · {} · {}",
                            display.id,
                            display.format.label(),
                            display.field.unit
                        ),
                    );
                }
            });
    });
    ui.small(if parent.array.is_some() {
        "Preferred display applies to every array element. Save the profile in Setup to retain it."
    } else {
        "Preferred display is a host setting. Save the profile in Setup to retain it."
    });
    (chosen != target.preferred_display).then(|| DisplaySelection {
        id: parent.id.clone(),
        subindex,
        display: chosen,
    })
}

#[derive(Default)]
pub struct Actions {
    pub read: Option<(String, u8)>,
    pub display: Option<DisplaySelection>,
    pub snapshot: Option<(String, u8, String)>,
}

pub fn show(
    ui: &mut Ui,
    profile: &Profile,
    selected: &mut Option<String>,
    subindex: &mut u8,
    available: bool,
    preferences_available: bool,
    snapshot_path: &mut String,
) -> Actions {
    let mut actions = Actions::default();
    ui.collapsing("IODD parameters", |ui| {
        ui.small("IODD defaults and permitted values describe the device. Reads use a separate acyclic client.");
        ui.add_enabled_ui(available, |ui| {
            if let Some((parent, target)) = picker(ui, profile, selected, subindex, "iodd-read-parameter") {
                actions.display = preference(ui, parent, target, *subindex, preferences_available, "iodd-read-parameter");
                let allowed = parent.access != Access::WriteOnly && target.access != Access::WriteOnly
                    && (*subindex == 0 || parent.subindex_access());
                if ui.add_enabled(allowed, egui::Button::new("Read named parameter")).clicked() {
                    actions.read = Some((parent.id.clone(), *subindex));
                }
                ui.add(egui::TextEdit::singleline(snapshot_path).hint_text("New snapshot JSON path"));
                if ui.add_enabled(allowed && !snapshot_path.trim().is_empty(), egui::Button::new("Read and save snapshot")).clicked() {
                    actions.snapshot = Some((parent.id.clone(), *subindex, snapshot_path.clone()));
                }
                ui.small("Saves one explicit read and its profile to a new file. Choose a new path for each read.");
            }
        });
        if let Some(imported) = &profile.iodd && !imported.standard_parameter_refs.is_empty() {
            ui.collapsing("Standard parameter references", |ui| {
                ui.small("These references require local standard definitions. The numeric ISDU read accepts a documented index and subindex.");
                for id in &imported.standard_parameter_refs { ui.monospace(id); }
            });
        }
    });
    actions
}

#[derive(Default)]
pub struct Offline {
    selected: Option<String>,
    subindex: u8,
    payload: String,
}

impl Offline {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        profile: &Profile,
        preferences_available: bool,
    ) -> Option<DisplaySelection> {
        let mut selection = None;
        ui.heading("Offline ISDU parameter decoder");
        ui.small("Paste a recorded 1–232 byte response. Select subindex 0 for a whole record, or the member addressed by the recorded query.");
        if let Some((parent, target)) = picker(
            ui,
            profile,
            &mut self.selected,
            &mut self.subindex,
            "iodd-offline-parameter",
        ) {
            selection = preference(
                ui,
                parent,
                target,
                self.subindex,
                preferences_available,
                "iodd-offline-parameter",
            );
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
        selection
    }
}

pub struct Saved {
    path: String,
    worker: Option<super::Worker<iolink_workbench::parameter_snapshot::Review>>,
    result: Option<iolink_workbench::parameter_snapshot::Review>,
    error: String,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            path: ".local/parameter.json".into(),
            worker: None,
            result: None,
            error: String::new(),
        }
    }
}

impl Saved {
    pub fn show(&mut self, ui: &mut Ui) {
        if let Some(result) = super::completed(&mut self.worker) {
            match result {
                Ok(review) => {
                    self.result = Some(review);
                    self.error.clear();
                }
                Err(error) => {
                    self.result = None;
                    self.error = error.to_string();
                }
            }
        }
        ui.heading("Saved parameter snapshot");
        ui.small("Reopen a JSON snapshot using its embedded profile. This checks local consistency; the file does not prove acquisition origin or physical bench acceptance.");
        if ui
            .add_enabled(
                self.worker.is_none(),
                egui::TextEdit::singleline(&mut self.path).hint_text("Snapshot JSON path"),
            )
            .changed()
        {
            self.result = None;
            self.error.clear();
        }
        if ui
            .add_enabled(
                self.worker.is_none() && !self.path.trim().is_empty(),
                egui::Button::new("Open parameter snapshot offline"),
            )
            .clicked()
        {
            let path = self.path.clone();
            match super::background("offline-parameter-snapshot", move || {
                iolink_workbench::parameter_snapshot::Snapshot::load(std::path::Path::new(&path))?
                    .review()
            }) {
                Ok(worker) => {
                    self.worker = Some(worker);
                    self.result = None;
                    self.error.clear();
                }
                Err(error) => self.error = error.to_string(),
            }
        }
        if self.worker.is_some() {
            ui.label("Opening snapshot…");
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        if !self.error.is_empty() {
            ui.colored_label(WARNING, &self.error);
        }
        if let Some(review) = &self.result {
            ui.monospace(review.text());
        }
    }
}
