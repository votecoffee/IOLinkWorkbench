use super::{WARNING, Worker, background, completed, iodd, path_entry};
use eframe::egui::{self, Ui};
use iolink_workbench::{decode::Field, iodd::Catalog, profile::Profile};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

mod connection;
mod identity;
mod mapping;

pub struct Editor {
    path: String,
    toml: String,
    pending: bool,
    connection: connection::Form,
    identity: identity::Form,
    mapping: mapping::Form,
    iodd_path: String,
    standard_definitions: String,
    standard_language_file: String,
    xsd: String,
    require_stamp: bool,
    language: String,
    language_file: String,
    unit_definitions: String,
    inspect_worker: Option<Worker<Catalog>>,
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
            connection: connection::Form::new(&profile.connection),
            identity: identity::Form::new(&profile.expected_identity),
            mapping: mapping::Form::default(),
            iodd_path: String::new(),
            standard_definitions: String::new(),
            standard_language_file: String::new(),
            xsd: String::new(),
            require_stamp: false,
            language: String::new(),
            language_file: String::new(),
            unit_definitions: String::new(),
            inspect_worker: None,
            entry: String::new(),
            catalog: None,
            variant: None,
            layout: None,
        }
    }

    pub fn sync(&mut self, profile: &Profile) {
        self.toml = serialize(profile);
        self.pending = false;
        self.connection.sync(&profile.connection);
        self.identity.sync(&profile.expected_identity);
        self.mapping.sync(profile);
    }

    pub fn pending(&self) -> bool {
        self.pending
            || self.connection.pending()
            || self.identity.pending()
            || self.mapping.pending()
    }

    pub fn edit_field(&mut self, field: Field, manual: bool) {
        self.mapping.edit(field, manual);
    }

    pub fn busy(&self) -> bool {
        self.inspect_worker.is_some()
    }

    pub fn update(&mut self, ctx: &egui::Context, message: &mut String) {
        if let Some(result) = completed(&mut self.inspect_worker) {
            match result {
                Ok(catalog) => {
                    self.variant = (catalog.variants.len() == 1)
                        .then(|| catalog.variants[0].product_id.clone());
                    self.layout = (catalog.layouts.len() == 1
                        && catalog.layouts[0].condition.is_none())
                    .then(|| catalog.layouts[0].id.clone());
                    *message = catalog
                        .schema_validation
                        .as_ref()
                        .map(|report| report.summary())
                        .unwrap_or_else(|| {
                            "Parser checks only; XSD validation not performed".into()
                        });
                    self.catalog = Some(catalog);
                }
                Err(error) => {
                    self.catalog = None;
                    *message = format!("{error:#}");
                }
            }
        }
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }
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
                if ui.add_enabled(!self.pending(), egui::Button::new("Load profile")).clicked() {
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
                if ui.add_enabled(!self.pending(), egui::Button::new("Save / replace profile")).clicked() {
                    *message = match profile.save(Path::new(self.path.trim())) {
                        Ok(()) => "Profile saved".into(),
                        Err(error) => format!("{error:#}"),
                    };
                }
            });
            ui.separator();
            if ui.add_enabled_ui(!self.pending && !self.identity.pending() && !self.mapping.pending(), |ui| self.connection.show(ui, profile, message)).inner {
                self.sync(profile);
                changed = true;
            }
            ui.separator();
            if ui.add_enabled_ui(!self.pending && !self.connection.pending() && !self.mapping.pending(), |ui| self.identity.show(ui, profile, message)).inner {
                self.sync(profile);
                changed = true;
            }
            ui.separator();
            changed |= self.iodd(ui, profile, message);
            ui.separator();
            ui.heading("Effective process input");
            if let Ok(context) = iolink_workbench::timing::assess(profile, Some(profile.connection.poll_us), Default::default(), 1000) {
                ui.collapsing("Cycle target from the applied profile", |ui| { ui.label(context.text()); });
            }
            ui.small("Bit 0 is the least significant bit. Big endian counts from the last byte, little endian from the first.");
            if let Some(import) = &profile.iodd {
                ui.small(import.communication.as_ref().map(|value| value.summary()).unwrap_or_else(|| "IODD wired communication declaration unavailable".into()));
                ui.small(import.stamp_validation.as_ref().map(|report| report.summary()).unwrap_or_else(|| "IODD checksum not checked in this saved profile".into()));
                if let Some(localization) = &import.localization { ui.small(localization.summary()); }
                ui.small(import.schema_validation.as_ref().map(|report| report.summary()).unwrap_or_else(|| "Parser checks only; XSD validation not performed".into()));
                ui.label(format!("{} · product {} · layout {} · {} imported bytes",
                    import.source, import.variant, import.layout, import.input_bytes));
                for item in &import.needs_input { ui.colored_label(WARNING, item); }
                if let Some(condition) = &import.condition {
                    let checkbox = egui::Checkbox::new(&mut profile.confirm_iodd_condition,
                        format!("Confirmed active condition on the device · {condition}"));
                    if ui.add_enabled(!self.pending(), checkbox).changed() {
                        self.sync(profile);
                        changed = true;
                    }
                }
            }
            let mut edit_field = None;
            for (field, origin) in profile.effective_fields() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("{} · {} · bit {} / {} · {:?} {:?} · ×{} + {} {}",
                        field.name, origin, field.bit_offset, field.bits, field.kind, field.byte_order,
                        field.scale, field.offset, field.unit));
                    if ui.add_enabled(!self.pending(), egui::Button::new("Edit mapping")).clicked()
                    { edit_field = Some((field.clone(), origin != "IODD")); }
                });
            }
            if let Some((field, manual)) = edit_field { self.edit_field(field, manual); }
            if ui.add_enabled(!self.pending(), egui::Button::new("Add manual field")).clicked() {
                self.mapping.new_field(profile);
            }
            if ui.add_enabled_ui(!self.pending && !self.connection.pending() && !self.identity.pending(), |ui| self.mapping.show(ui, profile, message)).inner {
                self.sync(profile);
                changed = true;
            }
            ui.separator();
            ui.heading("Advanced profile settings");
            ui.small("Edit the host connection, expected identity, and named fields here. A matching field name overrides an imported field. Choose CSV values in Monitor & CSV, or set logging.fields / logging.raw_only here.");
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
            if ui.add_enabled(!self.connection.pending() && !self.identity.pending() && !self.mapping.pending(), egui::TextEdit::multiline(&mut self.toml).code_editor()
                .desired_rows(16).desired_width(f32::INFINITY)).changed()
            { self.pending = true; }
        });
        changed
    }

    fn iodd(&mut self, ui: &mut Ui, profile: &mut Profile, message: &mut String) -> bool {
        let mut changed = false;
        let previous = (
            self.iodd_path.clone(),
            self.entry.clone(),
            self.standard_definitions.clone(),
            self.standard_language_file.clone(),
            self.xsd.clone(),
            self.require_stamp,
            self.language.clone(),
            self.language_file.clone(),
            self.unit_definitions.clone(),
        );
        ui.heading("IODD import");
        ui.checkbox(
            &mut self.require_stamp,
            "Require matching main and selected-language checksums",
        );
        ui.small("Present mismatched or malformed stamps reject import. Missing stamps remain visible unless matching checksums are required.");
        ui.add(
            egui::TextEdit::singleline(&mut self.language)
                .hint_text("Language tag (blank uses primary language)"),
        );
        path_entry(
            ui,
            &mut self.language_file,
            "Optional explicit language XML",
        );
        path_entry(
            ui,
            &mut self.unit_definitions,
            "Optional standard unit definitions XML",
        );
        path_entry(
            ui,
            &mut self.xsd,
            "Optional local root XSD (require validation)",
        );
        path_entry(
            ui,
            &mut self.standard_definitions,
            "Optional standard definitions XML",
        );
        path_entry(
            ui,
            &mut self.standard_language_file,
            "Optional standard language XML (requires language tag and definitions)",
        );
        ui.horizontal_wrapped(|ui| {
            path_entry(ui, &mut self.iodd_path, "IODD XML or ZIP");
            ui.add(
                egui::TextEdit::singleline(&mut self.entry)
                    .hint_text("ZIP member if ambiguous")
                    .desired_width(170.0),
            );
            if previous
                != (
                    self.iodd_path.clone(),
                    self.entry.clone(),
                    self.standard_definitions.clone(),
                    self.standard_language_file.clone(),
                    self.xsd.clone(),
                    self.require_stamp,
                    self.language.clone(),
                    self.language_file.clone(),
                    self.unit_definitions.clone(),
                )
            {
                self.catalog = None;
                self.variant = None;
                self.layout = None;
            }
            if ui.button("Inspect IODD").clicked() {
                let path = PathBuf::from(self.iodd_path.trim());
                let entry = self.entry.trim();
                let entry = (!entry.is_empty()).then(|| entry.to_owned());
                let definitions = self.standard_definitions.trim();
                let definitions = (!definitions.is_empty()).then(|| PathBuf::from(definitions));
                let standard_language_file = self.standard_language_file.trim();
                let standard_language_file = (!standard_language_file.is_empty())
                    .then(|| PathBuf::from(standard_language_file));
                let xsd = self.xsd.trim();
                let xsd = (!xsd.is_empty()).then(|| PathBuf::from(xsd));
                let language = self.language.trim();
                let language = (!language.is_empty()).then(|| language.to_owned());
                let language_file = self.language_file.trim();
                let language_file =
                    (!language_file.is_empty()).then(|| PathBuf::from(language_file));
                let unit_definitions = self.unit_definitions.trim();
                let unit_definitions =
                    (!unit_definitions.is_empty()).then(|| PathBuf::from(unit_definitions));
                let require_stamp = self.require_stamp;
                self.catalog = None;
                match background("iodd-inspection", move || {
                    iodd::load_with_options(
                        &path,
                        &iodd::LoadOptions {
                            require_stamp,
                            entry: entry.as_deref(),
                            definitions: definitions.as_deref(),
                            standard_language_file: standard_language_file.as_deref(),
                            xsd: xsd.as_deref(),
                            language: language.as_deref(),
                            language_file: language_file.as_deref(),
                            unit_definitions: unit_definitions.as_deref(),
                        },
                    )
                }) {
                    Ok(worker) => {
                        self.inspect_worker = Some(worker);
                        *message = "Inspecting IODD…".into();
                    }
                    Err(error) => *message = error.to_string(),
                }
            }
        });
        let can_apply = !self.pending();
        if let Some(catalog) = &self.catalog {
            if let Some(communication) = &catalog.communication {
                ui.small(communication.summary());
                ui.small("IODD declarations do not establish the actual port cycle or sensor/gateway update rate.");
            }
            if let Some(report) = &catalog.stamp_validation {
                ui.small(report.summary());
                ui.small("Checksum consistency does not authenticate the vendor or rerun the IODD Checker.");
            }
            if let Some(localization) = &catalog.localization {
                ui.small(localization.summary());
                ui.small(format!(
                    "Available text languages · {}",
                    localization.available_languages.join(", ")
                ));
                for note in &localization.review {
                    ui.colored_label(WARNING, note);
                }
            }
            ui.small(
                catalog
                    .schema_validation
                    .as_ref()
                    .map(|report| report.summary())
                    .unwrap_or_else(|| "Parser checks only; XSD validation not performed".into()),
            );
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
                    .add_enabled(can_apply, egui::Button::new("Apply IODD layout"))
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
