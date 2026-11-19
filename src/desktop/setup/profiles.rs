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
}

impl Starter {
    fn profile(self) -> Profile {
        Profile::parse(match self {
            Self::Simulator => include_str!("../../../samples/simulator.toml"),
            Self::RawSimulator => include_str!("../../../samples/raw-only.toml"),
            Self::Al1350 => include_str!("../../../samples/ifm-al1350.toml"),
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
                ui.separator();
                match self.starter {
                    Starter::Simulator => {
                        ui.label("Synthetic level (mm) and switch input on port 1, with a 1 ms requested host poll. Try monitoring, CSV logging and offline decoding without hardware.");
                    }
                    Starter::RawSimulator => {
                        ui.label("The same synthetic input with raw-only CSV. Mappings stay available for preview and later re-decoding.");
                    }
                    Starter::Al1350 => {
                        ui.label("An incomplete HTTP snapshot template, with no endpoint or identity pins. Set the actual host route, input width and sensor mapping before reading.");
                        ui.small("This adapter target is not selected or physically accepted bench hardware. Snapshot rows are host polls; source freshness and source-update loss remain unknown.");
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
                ui.label("Profile file (.toml)");
                path_entry(ui, &mut self.path, "Enter or paste a full profile path");
                let has_path = !self.path.trim().is_empty();
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(has_path, egui::Button::new("Load profile")).clicked() {
                        match Profile::load(Path::new(self.path.trim())) {
                            Ok(loaded) => {
                                let path = self.path.clone();
                                *profile = loaded;
                                *self = Self::new(profile);
                                self.path = path;
                                self.profiles_page = ProfilePage::Files;
                                changed = true;
                                *message = "Profile loaded. Review Device overview before connecting.".into();
                            }
                            Err(error) => *message = format!("{error:#}"),
                        }
                    }
                    if ui.add_enabled(has_path, egui::Button::new("Save / replace profile")).clicked() {
                        *message = match profile.save(Path::new(self.path.trim())) {
                            Ok(()) => "Profile saved".into(),
                            Err(error) => format!("{error:#}"),
                        };
                    }
                });
                ui.small("Save / replace writes the applied profile to this path, replacing an existing file. It does not apply pending drafts.");
            }
        });
        changed
    }
}
