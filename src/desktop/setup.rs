use super::{WARNING, iodd, path_entry};
use eframe::egui::{self, Ui};
use iolink_workbench::{decode::Field, iodd::Catalog, profile::Profile};
use std::path::Path;

pub struct Editor {
    path: String,
    toml: String,
    pub pending: bool,
    iodd_path: String,
    entry: String,
    catalog: Option<Catalog>,
    variant: Option<String>,
    layout: Option<String>,
}

impl Editor {
    pub fn new(profile: &Profile) -> Self {
        Self {
            path: String::new(),
            toml: serialize(profile),
            pending: false,
            iodd_path: String::new(),
            entry: String::new(),
            catalog: None,
            variant: None,
            layout: None,
        }
    }

    pub fn sync(&mut self, profile: &Profile) {
        self.toml = serialize(profile);
        self.pending = false;
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        profile: &mut Profile,
        message: &mut String,
        editable: bool,
    ) -> bool {
        let mut changed = false;
        ui.heading("Profiles and device mapping");
        ui.label("The applied profile controls Monitor & CSV and Offline decode. Start with the built-in simulator or load a TOML profile.");
        ui.add_enabled_ui(editable, |ui| {
            ui.horizontal_wrapped(|ui| {
                path_entry(ui, &mut self.path, "Profile path (.toml)");
                if ui.add_enabled(!self.pending, egui::Button::new("Load profile")).clicked() {
                    match Profile::load(Path::new(self.path.trim())) {
                        Ok(loaded) => {
                            *profile = loaded;
                            self.sync(profile);
                            self.catalog = None;
                            changed = true;
                            *message = "Profile loaded".into();
                        }
                        Err(error) => *message = format!("{error:#}"),
                    }
                }
                if ui.add_enabled(!self.pending, egui::Button::new("Save / replace profile")).clicked() {
                    *message = match profile.save(Path::new(self.path.trim())) {
                        Ok(()) => "Profile saved".into(),
                        Err(error) => format!("{error:#}"),
                    };
                }
            });
            ui.separator();
            changed |= self.iodd(ui, profile, message);
            ui.separator();
            ui.heading("Effective process input");
            ui.small("Bit 0 is the least significant bit. Big endian counts from the last byte, little endian from the first.");
            if let Some(import) = &profile.iodd {
                ui.label(format!("{} · product {} · layout {} · {} imported bytes",
                    import.source, import.variant, import.layout, import.input_bytes));
                for item in &import.needs_input { ui.colored_label(WARNING, item); }
                if let Some(condition) = &import.condition {
                    let checkbox = egui::Checkbox::new(&mut profile.confirm_iodd_condition,
                        format!("Confirmed active condition on the device · {condition}"));
                    if ui.add_enabled(!self.pending, checkbox).changed() {
                        self.sync(profile);
                        changed = true;
                    }
                }
            }
            let mut override_field = None;
            for (field, origin) in profile.effective_fields() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{} · {} · bit {} / {} · {:?} {:?} · ×{} + {} {}",
                        field.name, origin, field.bit_offset, field.bits, field.kind, field.byte_order,
                        field.scale, field.offset, field.unit));
                    if origin == "IODD"
                        && ui.add_enabled(!self.pending, egui::Button::new("Edit as override")).clicked()
                    { override_field = Some(field.clone()); }
                });
            }
            if let Some(field) = override_field { self.append_field(field); }
            if ui.add_enabled(!self.pending, egui::Button::new("Add manual field")).clicked() {
                let existing = profile.effective_fields();
                let name = (1..).map(|n| format!("field_{n}"))
                    .find(|name| existing.iter().all(|(field, _)| field.name != *name)).expect("unused name");
                self.append_field(Field { name, ..Field::default() });
            }
            ui.separator();
            ui.heading("Profile settings");
            ui.small("Edit the host connection, expected identity, and named fields here. A matching field name overrides an imported field. Set logging.fields to the names to record.");
            ui.horizontal(|ui| {
                if ui.add_enabled(self.pending, egui::Button::new("Apply edits")).clicked() {
                    match Profile::parse(&self.toml) {
                        Ok(edited) => {
                            *profile = edited;
                            self.sync(profile);
                            changed = true;
                            *message = "Profile changes applied".into();
                        }
                        Err(error) => *message = format!("{error:#}"),
                    }
                }
                if ui.add_enabled(self.pending, egui::Button::new("Discard edits")).clicked() {
                    self.sync(profile);
                    message.clear();
                }
                if self.pending { ui.colored_label(WARNING, "Pending edits"); }
            });
            if ui.add(egui::TextEdit::multiline(&mut self.toml).code_editor()
                .desired_rows(16).desired_width(f32::INFINITY)).changed()
            { self.pending = true; }
        });
        changed
    }

    fn append_field(&mut self, field: Field) {
        #[derive(serde::Serialize)]
        struct FieldList {
            fields: Vec<Field>,
        }
        let table = toml::to_string_pretty(&FieldList {
            fields: vec![field],
        })
        .expect("serializable field");
        self.toml.push('\n');
        self.toml.push_str(&table);
        self.pending = true;
    }

    fn iodd(&mut self, ui: &mut Ui, profile: &mut Profile, message: &mut String) -> bool {
        let mut changed = false;
        ui.heading("IODD import");
        ui.horizontal_wrapped(|ui| {
            path_entry(ui, &mut self.iodd_path, "IODD XML or ZIP");
            ui.add(
                egui::TextEdit::singleline(&mut self.entry)
                    .hint_text("ZIP member if ambiguous")
                    .desired_width(170.0),
            );
            if ui.button("Inspect IODD").clicked() {
                let entry = self.entry.trim();
                match iodd::load(
                    Path::new(self.iodd_path.trim()),
                    (!entry.is_empty()).then_some(entry),
                ) {
                    Ok(catalog) => {
                        self.variant = (catalog.variants.len() == 1)
                            .then(|| catalog.variants[0].product_id.clone());
                        self.layout = (catalog.layouts.len() == 1
                            && catalog.layouts[0].condition.is_none())
                        .then(|| catalog.layouts[0].id.clone());
                        self.catalog = Some(catalog);
                        message.clear();
                    }
                    Err(error) => {
                        self.catalog = None;
                        *message = format!("{error:#}");
                    }
                }
            }
        });
        if let Some(catalog) = &self.catalog {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "Vendor {} · Device {}",
                    catalog.vendor_id, catalog.device_id
                ));
                egui::ComboBox::from_id_salt("variant")
                    .selected_text(self.variant.as_deref().unwrap_or("Select product ID"))
                    .show_ui(ui, |ui| {
                        for variant in &catalog.variants {
                            ui.selectable_value(
                                &mut self.variant,
                                Some(variant.product_id.clone()),
                                format!("{} · {}", variant.product_id, variant.name),
                            );
                        }
                    });
                egui::ComboBox::from_id_salt("input-layout")
                    .selected_text(self.layout.as_deref().unwrap_or("Select input layout"))
                    .show_ui(ui, |ui| {
                        for layout in &catalog.layouts {
                            ui.selectable_value(
                                &mut self.layout,
                                Some(layout.id.clone()),
                                &layout.id,
                            );
                        }
                    });
                if ui
                    .add_enabled(!self.pending, egui::Button::new("Apply IODD layout"))
                    .clicked()
                {
                    match catalog.select(self.variant.as_deref(), self.layout.as_deref()) {
                        Ok((identity, imported)) => {
                            profile.input_bytes = imported.input_bytes;
                            profile.expected_identity = identity;
                            profile.iodd = Some(imported);
                            profile.confirm_iodd_condition = false;
                            profile.logging.fields.clear();
                            changed = true;
                            *message =
                                "IODD applied. Review retained manual fields and overrides.".into();
                        }
                        Err(error) => *message = format!("{error:#}"),
                    }
                }
            });
            if let Some(layout) = catalog
                .layouts
                .iter()
                .find(|layout| Some(&layout.id) == self.layout.as_ref())
            {
                if let Some(condition) = &layout.condition {
                    ui.colored_label(WARNING, condition);
                }
                ui.small(format!(
                    "{} bytes · {} mapped fields",
                    layout.input_bytes,
                    layout.fields.len()
                ));
            }
        }
        if changed {
            self.sync(profile);
        }
        changed
    }
}

fn serialize(profile: &Profile) -> String {
    toml::to_string_pretty(profile).expect("serializable profile")
}
