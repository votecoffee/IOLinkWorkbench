//! Bundled starters are explicit offline replacements, never auto-connections.
use super::{Editor, WARNING, path_entry};
use eframe::egui::{self, Ui};
use iolink_workbench::profile::Profile;
use std::path::Path;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(in crate::desktop) enum ProfilePage {
    #[default]
    Start,
    Files,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Starter {
    #[default]
    Simulator,
    RawSimulator,
    Al1350,
    Al1351,
    Al1352,
    Al1353,
}

impl Starter {
    fn profile(self) -> Profile {
        Profile::parse(match self {
            Self::Simulator => include_str!("../../../samples/simulator.toml"),
            Self::RawSimulator => include_str!("../../../samples/raw-only.toml"),
            Self::Al1350 => include_str!("../../../samples/ifm-al1350.toml"),
            Self::Al1351 => include_str!("../../../samples/ifm-al1351.toml"),
            Self::Al1352 => include_str!("../../../samples/ifm-al1352.toml"),
            Self::Al1353 => include_str!("../../../samples/ifm-al1353.toml"),
        })
        .expect("validated bundled profile")
    }
}

impl Editor {
    pub fn show_profiles(
        &mut self,
        ui: &mut Ui,
        profile: &mut Profile,
        message: &mut String,
        editable: bool,
    ) -> bool {
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.profiles_page,
                ProfilePage::Start,
                "Start a profile",
            );
            ui.selectable_value(&mut self.profiles_page, ProfilePage::Files, "Open & save");
        });
        ui.separator();
        ui.small("Profiles are host settings. Loading or starting a profile does not connect.");
        if !editable {
            ui.colored_label(WARNING, "Finish active acquisition, parameter reads or IODD inspection before changing profiles.");
        } else if self.pending() {
            ui.colored_label(
                WARNING,
                "Apply or discard pending edits before replacing or saving a profile.",
            );
        }
        let mut changed = false;
        ui.add_enabled_ui(editable && !self.pending(), |ui| match self.profiles_page {
            ProfilePage::Start => {
                ui.label("Choose a starting point");
                ui.radio_value(&mut self.starter, Starter::Simulator, "Simulator with typed values");
                ui.radio_value(&mut self.starter, Starter::RawSimulator, "Simulator with raw-only CSV");
                ui.radio_value(&mut self.starter, Starter::Al1350, "ifm AL1350 setup template");
                ui.horizontal_wrapped(|ui| {
                    ui.radio_value(&mut self.starter, Starter::Al1351, "ifm AL1351 read-only template");
                    ui.radio_value(&mut self.starter, Starter::Al1352, "ifm AL1352 read-only template");
                    ui.radio_value(&mut self.starter, Starter::Al1353, "ifm AL1353 read-only template");
                });
                ui.separator();
                match self.starter {
                    Starter::Simulator => {
                        ui.label("Synthetic level (mm) and switch input on port 1, with a 1 ms requested host poll. Try monitoring, CSV logging and offline decoding without hardware.");
                    }
                    Starter::RawSimulator => {
                        ui.label("The same synthetic input with raw-only CSV. Mappings stay available for preview and later re-decoding.");
                    }
                    Starter::Al1350 | Starter::Al1351 | Starter::Al1352 | Starter::Al1353 => {
                        ui.label("An incomplete HTTP snapshot template, with no endpoint or identity pins. Set the actual host route, input width and sensor mapping before reading.");
                        ui.small("Confirm the installed model and firmware before bench use. Snapshot rows are host polls with unknown source freshness and loss.");
                    }
                }
                ui.small("Use replaces the applied settings in memory. Save separately to keep them; existing files are untouched.");
                if ui.button("Use starting profile").clicked() {
                    *profile = self.starter.profile();
                    // A new starter must not inherit a previously loaded file destination.
                    *self = Self::new(profile);
                    *message = "Starting profile applied in memory. Review Device overview, then save to a new profile path.".into();
                    changed = true;
                }
            }
            ProfilePage::Files => {
                ui.small("Drop one TOML profile onto the workbench to select its path, then choose Load profile.");
                ui.strong("Open profile");
                ui.horizontal_wrapped(|ui| {
                    path_entry(ui, &mut self.open_path, "Enter or paste a full profile path");
                    if ui.add_enabled(!self.open_path.trim().is_empty(), egui::Button::new("Load profile")).clicked() {
                        match Profile::load(Path::new(self.open_path.trim())) {
                            Ok(loaded) => {
                                let path = self.open_path.clone();
                                *profile = loaded;
                                *self = Self::new(profile);
                                self.open_path = path.clone();
                                self.save_path = path;
                                self.profiles_page = ProfilePage::Files;
                                changed = true;
                                *message = "Profile loaded. Review Device overview before connecting.".into();
                            }
                            Err(error) => *message = format!("{error:#}"),
                        }
                    }
                });
                ui.separator();
                ui.strong(format!("Save applied profile · {}", profile.name));
                ui.horizontal_wrapped(|ui| {
                    path_entry(ui, &mut self.save_path, "Save destination (.toml)");
                    if ui.add_enabled(!self.save_path.trim().is_empty(), egui::Button::new("Save / replace profile")).clicked() {
                        *message = match profile.save(Path::new(self.save_path.trim())) {
                            Ok(()) => "Profile saved".into(),
                            Err(error) => format!("{error:#}"),
                        };
                    }
                });
                ui.small("Save / replace writes applied settings to the save destination, replacing an existing file. Selecting a file to open does not change this destination; a successful load sets it to the loaded profile.");
            }
        });
        changed
    }
}
