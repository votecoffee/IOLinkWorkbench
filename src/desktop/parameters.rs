use super::WARNING;
use eframe::egui::{self, Ui};
use iolink_workbench::{
    decode, diagnostics,
    iodd::{Access, Parameter},
    profile::Profile,
    transport::write::isdu_write_target,
};

#[derive(Default)]
pub(super) struct Picker {
    pub(super) selected: Option<String>,
    pub(super) subindex: u8,
    filter: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Browse {
    All,
    Writable,
}

impl Browse {
    fn allows(self, parent: &Parameter, subindex: u8) -> bool {
        self == Self::All || isdu_write_target(parent, subindex).is_ok()
    }

    fn includes(self, parent: &Parameter) -> bool {
        if self.allows(parent, 0) {
            return true;
        }
        if !parent.subindex_access() || parent.access != Access::ReadWrite {
            return false;
        }
        parent.record.as_ref().is_some_and(|record| {
            record
                .items
                .iter()
                .any(|item| self.allows(parent, item.subindex))
        }) || (parent.array.is_some() && self.allows(parent, 1))
    }

    fn subindex_label(self, parent: &Parameter, subindex: u8) -> String {
        if self.allows(parent, subindex) {
            format!("Subindex {subindex}")
        } else {
            "Select writable subindex".into()
        }
    }
}

impl Picker {
    fn show<'a>(
        &mut self,
        ui: &mut Ui,
        profile: &'a Profile,
        salt: &str,
    ) -> Option<(&'a Parameter, &'a Parameter)> {
        self.show_matching(ui, profile, salt, Browse::All)
    }

    pub(super) fn show_for_write(&mut self, ui: &mut Ui, profile: &Profile) -> Option<(u16, u8)> {
        let (parent, _) =
            self.show_matching(ui, profile, "write-iodd-parameter", Browse::Writable)?;
        Some((parent.index, self.subindex))
    }

    fn show_matching<'a>(
        &mut self,
        ui: &mut Ui,
        profile: &'a Profile,
        salt: &str,
        browse: Browse,
    ) -> Option<(&'a Parameter, &'a Parameter)> {
        let Some(imported) = &profile.iodd else {
            ui.label("Import an IODD from Device → IODD import to browse named parameters.");
            return None;
        };
        if imported.parameters.is_empty() {
            ui.label("This profile has no named IODD parameters.");
            return None;
        }

        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .id_salt((salt, "filter"))
                    .hint_text("Search name, IODD ID, or index")
                    .desired_width(250.0),
            );
            if !self.filter.is_empty() && ui.small_button("Clear search").clicked() {
                self.filter.clear();
            }
        });
        let query = self.filter.trim().to_lowercase();
        let index = if let Some(hex) = query.strip_prefix("0x") {
            u16::from_str_radix(hex, 16).ok()
        } else {
            query.parse::<u16>().ok()
        };
        let matches: Vec<_> = imported
            .parameters
            .iter()
            .filter(|parameter| browse.includes(parameter))
            .filter(|parameter| {
                query.is_empty()
                    || index == Some(parameter.index)
                    || parameter.name.to_lowercase().contains(&query)
                    || parameter.id.to_lowercase().contains(&query)
            })
            .collect();
        let before = self.selected.clone();
        ui.horizontal_wrapped(|ui| {
            let selection = self.selected.as_deref().and_then(|id| {
                imported
                    .parameters
                    .iter()
                    .find(|parameter| parameter.id == id && browse.includes(parameter))
            });
            let label = selection.map_or_else(
                || "Select parameter".into(),
                |parameter| format!("{} · index {}", parameter.name, parameter.index),
            );
            ui.add_enabled_ui(!matches.is_empty(), |ui| {
                egui::ComboBox::from_id_salt(salt)
                    .selected_text(label)
                    .width(300.0)
                    .height(240.0)
                    .truncate()
                    .show_ui(ui, |ui| {
                        for parameter in &matches {
                            let chosen = self.selected.as_ref() == Some(&parameter.id);
                            if ui
                                .selectable_label(
                                    chosen,
                                    format!(
                                        "{} · index {} · {}",
                                        parameter.name,
                                        parameter.index,
                                        parameter.access.label()
                                    ),
                                )
                                .on_hover_text(&parameter.id)
                                .clicked()
                            {
                                self.selected = Some(parameter.id.clone());
                            }
                        }
                    });
            });
            if matches.is_empty() {
                ui.small(if browse == Browse::Writable {
                    "No matching writable parameters"
                } else {
                    "No matching parameters"
                });
            } else if browse == Browse::Writable {
                ui.small(format!("{} matching writable parameters", matches.len()));
            } else {
                ui.small(format!(
                    "{} of {} parameters",
                    matches.len(),
                    imported.parameters.len()
                ));
            }
            if selection.is_some_and(|selected| {
                !matches.iter().any(|parameter| parameter.id == selected.id)
            }) {
                ui.small("Current selection is outside the search results.");
            }
        });
        if self.selected != before {
            self.subindex = 0;
        }
        let parent = imported.parameters.iter().find(|parameter| {
            Some(&parameter.id) == self.selected.as_ref() && browse.includes(parameter)
        })?;
        let subindex = &mut self.subindex;
        if let Some(record) = &parent.record {
            ui.label(format!(
                "Individual subindex access · {}",
                record.subindex_access
            ));
            egui::ComboBox::from_id_salt((salt, "subindex"))
                .selected_text(browse.subindex_label(parent, *subindex))
                .show_ui(ui, |ui| {
                    if browse.allows(parent, 0) {
                        ui.selectable_value(subindex, 0, "0 · whole record");
                    }
                    for item in record
                        .items
                        .iter()
                        .filter(|item| browse.allows(parent, item.subindex))
                    {
                        ui.selectable_value(
                            subindex,
                            item.subindex,
                            format!("{} · {}", item.subindex, item.parameter.name),
                        );
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
                .selected_text(browse.subindex_label(parent, *subindex))
                .show_ui(ui, |ui| {
                    if browse.allows(parent, 0) {
                        ui.selectable_value(subindex, 0, "0 · whole array");
                    }
                    for element in
                        (1..=array.count).filter(|&element| browse.allows(parent, element))
                    {
                        ui.selectable_value(subindex, element, format!("Element {element}"));
                    }
                });
        } else {
            *subindex = 0;
        }
        if !browse.allows(parent, *subindex) {
            return None;
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
        Some((parent, target))
    }
}

fn definition(ui: &mut Ui, parent: &Parameter, target: &Parameter, subindex: u8, salt: &str) {
    ui.small(format!("IODD ID · {}", parent.id));
    if let Some(record) = &parent.record {
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
    }
    if let Some(array) = &parent.array {
        for note in &array.element.needs_input {
            ui.colored_label(WARNING, note);
        }
        if subindex == 0 {
            for display in &array.element.displays {
                ui.label(format!(
                    "Each element · {} · {} · ×{} + {} {}",
                    display.id,
                    display.format_label(),
                    display.field.scale,
                    display.field.offset,
                    display.field.unit
                ));
            }
        }
    }
    if let Some(bits) = target.bits {
        ui.label(format!("Declared width · {bits} bits"));
    }
    if matches!(target.datatype.as_str(), "TimeT" | "TimeSpanT") {
        ui.small("Time layout · 64 bits · big endian · 1/2^32-second ticks · clock validity is separate from decoding");
    }
    if let Some(length) = target.fixed_length {
        ui.label(format!("Declared length · {length} bytes"));
    }
    if target.datatype == "OctetStringT" {
        ui.small(
            "Ordered octets · exact declared length · zeros are data · default 0xNN sequence view",
        );
    }
    if target.datatype == "StringT" {
        ui.small("Text length counts octets · condensed responses or NUL-only padding · raw bytes retained");
    }
    if let Some(encoding) = &target.encoding {
        ui.label(format!("Encoding · {encoding}"));
    }
    if let Some(default) = &target.default_value {
        ui.label(format!("IODD default · {default}"));
    }
    ui.push_id((salt, "declared-default"), |ui| {
        ui.collapsing("Inspect declared default (offline)", |ui| {
            match parent.inspect_default(subindex) {
                Ok(value) => {
                    if value.needs_review() {
                        ui.colored_label(WARNING, "Declared default needs review");
                    }
                    ui.monospace(value.text());
                }
                Err(error) => {
                    ui.colored_label(WARNING, error.to_string());
                }
            }
        });
    });
    if let Some(origin) = &target.origin {
        ui.small(format!("Standard definition · {origin}"));
    }
    for display in &target.displays {
        let field = &display.field;
        ui.label(format!(
            "Display {} · {} · ×{} + {} {} · decimals {:?}",
            display.id,
            display.format_label(),
            field.scale,
            field.offset,
            field.unit,
            field.decimals
        ));
    }
    for range in &target.ranges {
        ui.label(format!(
            "IODD range · {} through {}{}",
            range.lower,
            range.upper,
            range
                .name
                .as_ref()
                .map(|name| format!(" · {name}"))
                .unwrap_or_default()
        ));
    }
    for choice in &target.choices {
        ui.label(format!("{} · {}", choice.value, choice.name));
    }
    for note in &target.needs_input {
        ui.colored_label(WARNING, note);
    }
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
                            display.format_label(),
                            display.field.unit
                        ),
                    );
                }
            });
    });
    ui.small(if parent.array.is_some() {
        "Preferred display applies to every array element. Save the profile through Profiles… to retain it."
    } else {
        "Preferred display is a host setting. Save the profile through Profiles… to retain it."
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

pub(super) fn show(
    ui: &mut Ui,
    profile: &Profile,
    picker: &mut Picker,
    available: bool,
    preferences_available: bool,
    snapshot_path: &mut String,
) -> Actions {
    let mut actions = Actions::default();
    ui.add_enabled_ui(available, |ui| {
        let salt = "iodd-read-parameter";
        if let Some((parent, target)) = picker.show(ui, profile, salt) {
            let subindex = picker.subindex;
            let denied = if parent.access == Access::WriteOnly || target.access == Access::WriteOnly {
                Some("The IODD declares this selection write-only.")
            } else if subindex != 0 && !parent.subindex_access() {
                Some("Select subindex 0 for a whole-value read. The IODD disallows individual subindex reads.")
            } else {
                None
            };
            if ui.add_enabled(denied.is_none(), egui::Button::new("Read named parameter")).clicked() {
                actions.read = Some((parent.id.clone(), subindex));
            }
            if let Some(reason) = denied {
                ui.colored_label(WARNING, reason);
            }
            ui.collapsing("Read and save a snapshot", |ui| {
                ui.horizontal_wrapped(|ui| {
                    super::path_entry(ui, snapshot_path, "New snapshot JSON path");
                    if ui.add_enabled(denied.is_none() && !snapshot_path.trim().is_empty(), egui::Button::new("Read and save snapshot")).clicked() {
                        actions.snapshot = Some((parent.id.clone(), subindex, snapshot_path.clone()));
                    }
                });
            });
            ui.collapsing("IODD details and display preference", |ui| {
                actions.display = preference(ui, parent, target, subindex, preferences_available, salt);
                definition(ui, parent, target, subindex, salt);
            });
        }
    });
    if let Some(imported) = &profile.iodd
        && !imported.standard_parameter_refs.is_empty()
    {
        ui.collapsing("Standard parameter references", |ui| {
            ui.small("Load local standard definitions to resolve these references. Use Raw ISDU for a documented index and subindex.");
            for id in &imported.standard_parameter_refs {
                ui.monospace(id);
            }
        });
    }
    actions
}

pub(super) enum OfflineEdit {
    DisplayPreference(DisplaySelection),
    AddInputFields(diagnostics::InputDescriptorPlan),
}

#[derive(Default)]
pub struct Offline {
    pub(super) picker: Picker,
    payload: String,
    pub(super) profile_characteristic_payload: String,
    pub(super) access_locks_payload: String,
    data_storage_part: diagnostics::DataStoragePart,
    pub(super) data_storage_payload: String,
    pub(super) direct_page_payload: String,
    pub(super) process_descriptor_payload: String,
    descriptor_direction: diagnostics::DescriptorDirection,
    empty_descriptor: bool,
}

impl Offline {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        profile: &Profile,
        editing_available: bool,
    ) -> Option<OfflineEdit> {
        let mut edit = None;
        ui.heading("Offline ISDU parameter decoder");
        ui.small("Paste a recorded 1–232 byte response. Select subindex 0 for a whole record, or the member addressed by the recorded query.");
        ui.collapsing("Device access locks (12:0)", |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.access_locks_payload)
                    .hint_text("Paste the complete two-byte reply in hex"),
            );
            if !self.access_locks_payload.trim().is_empty() {
                match diagnostics::decode_device_access_locks(
                    Some(profile),
                    &self.access_locks_payload,
                ) {
                    Ok(report) => {
                        ui.label(report.text());
                    }
                    Err(error) => {
                        ui.colored_label(WARNING, error.to_string());
                    }
                }
            }
        });
        ui.collapsing("Device profile IDs (13:0)", |ui| {
            ui.small("Paste the complete ProfileCharacteristic reply in hex.");
            ui.add(
                egui::TextEdit::multiline(&mut self.profile_characteristic_payload)
                    .code_editor()
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            if !self.profile_characteristic_payload.trim().is_empty() {
                match diagnostics::decode_profile_characteristic(
                    Some(profile),
                    &self.profile_characteristic_payload,
                ) {
                    Ok(report) => {
                        ui.label(report.text());
                    }
                    Err(error) => {
                        ui.colored_label(WARNING, error.to_string());
                    }
                }
            }
        });
        ui.collapsing("Process data descriptors (14:0 / 15:0)", |ui| {
            use diagnostics::DescriptorDirection;
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(
                    &mut self.descriptor_direction,
                    DescriptorDirection::Input,
                    "Input (14:0)",
                );
                ui.selectable_value(
                    &mut self.descriptor_direction,
                    DescriptorDirection::Output,
                    "Output (15:0)",
                );
                ui.checkbox(&mut self.empty_descriptor, "Empty reply");
            });
            ui.add_enabled(
                !self.empty_descriptor,
                egui::TextEdit::multiline(&mut self.process_descriptor_payload)
                    .hint_text("Paste the complete descriptor reply in hex")
                    .code_editor()
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            let hex = if self.empty_descriptor {
                ""
            } else {
                &self.process_descriptor_payload
            };
            if self.empty_descriptor || !hex.trim().is_empty() {
                match diagnostics::decode_process_descriptor(
                    self.descriptor_direction,
                    Some(profile),
                    hex,
                ) {
                    Ok(report) => {
                        ui.label(report.text());
                        if self.descriptor_direction == DescriptorDirection::Input
                            && profile.has_process_input()
                            && report.status != diagnostics::DescriptorStatus::InvalidReply
                        {
                            ui.collapsing("Suggest manual input fields", |ui| {
                                match diagnostics::plan_input_descriptor(profile, hex) {
                                    Ok(plan) => {
                                        ui.label(plan.text());
                                        if ui
                                            .add_enabled(
                                                editing_available && !plan.additions.is_empty(),
                                                egui::Button::new("Add manual fields"),
                                            )
                                            .clicked()
                                        {
                                            edit = Some(OfflineEdit::AddInputFields(plan));
                                        }
                                    }
                                    Err(error) => {
                                        ui.colored_label(WARNING, error.to_string());
                                    }
                                }
                            });
                        }
                    }
                    Err(error) => {
                        ui.colored_label(WARNING, error.to_string());
                    }
                }
            }
        });
        let salt = "iodd-offline-parameter";
        if let Some((parent, target)) = self.picker.show(ui, profile, salt) {
            let subindex = self.picker.subindex;
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
            ui.collapsing("IODD details and display preference", |ui| {
                if let Some(selection) =
                    preference(ui, parent, target, subindex, editing_available, salt)
                {
                    edit = Some(OfflineEdit::DisplayPreference(selection));
                }
                definition(ui, parent, target, subindex, salt);
            });
        }
        egui::CollapsingHeader::new("Data Storage metadata (3:2–5)").show(ui, |ui| {
            ui.small("Paste recorded metadata from a master/device exchange.");
            egui::ComboBox::from_id_salt("offline-data-storage-part")
                .selected_text(self.data_storage_part.label())
                .show_ui(ui, |ui| {
                    for part in diagnostics::DataStoragePart::ALL {
                        ui.selectable_value(&mut self.data_storage_part, part, part.label());
                    }
                });
            ui.add(
                egui::TextEdit::multiline(&mut self.data_storage_payload)
                    .hint_text("Paste the recorded reply in hex")
                    .code_editor()
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            if !self.data_storage_payload.trim().is_empty() {
                match diagnostics::decode_data_storage(
                    self.data_storage_part,
                    &self.data_storage_payload,
                ) {
                    Ok(report) => {
                        ui.label(report.text());
                    }
                    Err(error) => {
                        ui.colored_label(WARNING, error.to_string());
                    }
                }
            }
        });
        ui.collapsing("Direct Parameter Page 1 (0:0)", |ui| {
            ui.small("Paste 16 recorded bytes in address order, from 0x00 to 0x0F.");
            ui.add(
                egui::TextEdit::singleline(&mut self.direct_page_payload)
                    .hint_text("Paste the recorded page in hex"),
            );
            if !self.direct_page_payload.trim().is_empty() {
                match diagnostics::decode_direct_page(
                    Some(&profile.expected_identity),
                    &self.direct_page_payload,
                ) {
                    Ok(report) => {
                        ui.label(report.text());
                    }
                    Err(error) => {
                        ui.colored_label(WARNING, error.to_string());
                    }
                }
            }
        });
        edit
    }
}

pub struct Saved {
    path: String,
    compare_path: String,
    worker: Option<super::Worker<String>>,
    result: Option<String>,
    error: String,
}

impl Default for Saved {
    fn default() -> Self {
        Self {
            path: ".local/parameter.json".into(),
            compare_path: String::new(),
            worker: None,
            result: None,
            error: String::new(),
        }
    }
}

impl Saved {
    #[cfg(test)]
    pub(super) fn gate_for_test(&mut self) -> super::TestCompletion {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.worker = Some(receiver);
        Box::new(move || {
            let _ = sender.send(Err(anyhow::anyhow!("synthetic snapshot completion")));
        })
    }

    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }

    pub fn update(&mut self) {
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
    }

    pub fn show(&mut self, ui: &mut Ui) {
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
            self.open(false);
        }
        ui.collapsing("Compare two snapshots", |ui| {
            ui.small("Compare two separate reads at the same index and subindex. Each snapshot keeps its own identity and connection diagnostics.");
            if ui.add_enabled(
                self.worker.is_none(),
                egui::TextEdit::singleline(&mut self.compare_path).hint_text("Second snapshot JSON path"),
            ).changed() {
                self.result = None;
                self.error.clear();
            }
            if ui.add_enabled(
                self.worker.is_none() && !self.path.trim().is_empty() && !self.compare_path.trim().is_empty(),
                egui::Button::new("Compare snapshots offline"),
            ).clicked() {
                self.open(true);
            }
        });
        if self.worker.is_some() {
            ui.label("Reading snapshot files…");
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        if !self.error.is_empty() {
            ui.colored_label(WARNING, &self.error);
        }
        if let Some(review) = &self.result {
            ui.monospace(review);
        }
    }

    fn open(&mut self, compare: bool) {
        use iolink_workbench::parameter_snapshot::{self, Snapshot};
        let path = std::path::PathBuf::from(&self.path);
        let other = compare.then(|| std::path::PathBuf::from(&self.compare_path));
        match super::background("offline-parameter-snapshot", move || {
            let snapshot = Snapshot::load(&path)?;
            if let Some(other) = other {
                return Ok(
                    parameter_snapshot::compare(&snapshot, &Snapshot::load(&other)?)?.text(),
                );
            }
            Ok(snapshot.review()?.text())
        }) {
            Ok(worker) => {
                self.worker = Some(worker);
                self.result = None;
                self.error.clear();
            }
            Err(error) => self.error = error.to_string(),
        }
    }
}
