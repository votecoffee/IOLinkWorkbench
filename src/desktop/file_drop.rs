//! File drops select host paths only. Reading and applying remain explicit actions.
use super::{App, Tab, WARNING, workspace::OfflinePage};
use eframe::egui::{self, Context};
use std::path::Path;

#[derive(Clone, Copy)]
enum DropTarget {
    Profile,
    Iodd,
    Capture,
    Bundle,
}

impl DropTarget {
    fn saved(self) -> bool {
        matches!(self, Self::Capture | Self::Bundle)
    }
}

impl App {
    fn saved_drop_context(&self) -> bool {
        self.tab == Tab::Offline
            && self.workspace.offline == OfflinePage::Captures
            && !self.workspace.profile_dialog
            && !self.workspace.connect_dialog
    }

    fn drop_target(&self, path: &Path) -> Option<DropTarget> {
        let name = path.file_name()?.to_str()?;
        if self.saved_drop_context() {
            if name.eq_ignore_ascii_case("capture.toml") {
                return Some(DropTarget::Capture);
            }
            if name.eq_ignore_ascii_case("receipt.toml") {
                return Some(DropTarget::Bundle);
            }
        }
        let extension = path.extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("toml") {
            Some(DropTarget::Profile)
        } else if extension.eq_ignore_ascii_case("xml") || extension.eq_ignore_ascii_case("zip") {
            Some(DropTarget::Iodd)
        } else {
            None
        }
    }

    fn file_drop_block_reason(&self, target: DropTarget) -> Option<&'static str> {
        if target.saved() {
            if self.closing {
                return Some("Application is closing");
            }
            return (!self.saved_capture.can_select_directory()).then_some(
                "Wait for the active capture review or export before selecting a folder",
            );
        }
        self.start_block_reason().or_else(|| {
            self.acyclic
                .busy()
                .then_some("Wait for the active parameter read before selecting a file")
        })
    }

    pub(super) fn handle_file_drop(&mut self, ctx: &Context) {
        let files = ctx.input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
        if files.is_empty() {
            return;
        }
        if files.len() != 1 {
            self.message = "Drop one file at a time. File selection unchanged.".into();
            return;
        }
        let Some(path) = files[0].path.as_ref() else {
            self.message =
                "Drop a local file with a filesystem path. File selection unchanged.".into();
            return;
        };
        let Some(text) = path.to_str() else {
            self.message =
                "The dropped path cannot be represented in this editor. File selection unchanged."
                    .into();
            return;
        };
        let Some(target) = self.drop_target(path) else {
            self.message = "Drop a TOML profile or IODD XML/ZIP. On Saved captures, drop capture.toml or receipt.toml. File selection unchanged.".into();
            return;
        };
        if let Some(reason) = self.file_drop_block_reason(target) {
            self.message = format!("File selection unchanged · {reason}");
            return;
        }
        match target {
            DropTarget::Capture | DropTarget::Bundle => {
                let Some(folder) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                else {
                    self.message =
                        "Drop the file from its saved folder. File selection unchanged.".into();
                    return;
                };
                let (selected, message) = if matches!(target, DropTarget::Capture) {
                    (
                        self.saved_capture.select_directory(folder),
                        "Capture folder selected. Choose Verify capture when ready.",
                    )
                } else {
                    (
                        self.saved_capture.select_bundle(folder),
                        "Bundle folder selected. Choose Verify bundle when ready.",
                    )
                };
                if selected {
                    self.message = message.into();
                }
            }
            DropTarget::Profile => {
                self.setup.select_profile_path(text.into());
                self.workspace.connect_dialog = false;
                self.workspace.profile_dialog = true;
                self.message = "Profile path selected. Choose Load profile to validate and apply it; current settings are unchanged.".into();
            }
            DropTarget::Iodd => {
                self.setup.select_iodd_path(text.into());
                self.workspace.connect_dialog = false;
                self.workspace.profile_dialog = false;
                self.tab = Tab::Device;
                self.message = "IODD path selected. Review import options, then Inspect IODD; the applied profile is unchanged.".into();
            }
        }
    }

    pub(super) fn file_drop_preview(&self, ctx: &Context) {
        let Some(hovered) = ctx.input(|input| input.raw.hovered_files.first().cloned()) else {
            return;
        };
        let target = hovered
            .path
            .as_deref()
            .and_then(|path| self.drop_target(path))
            .unwrap_or(if self.saved_drop_context() {
                DropTarget::Capture
            } else {
                DropTarget::Profile
            });
        egui::Area::new(egui::Id::new("file-drop-preview"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(16.0).show(ui, |ui| {
                    ui.set_max_width(440.0);
                    if target.saved() {
                        ui.heading("Drop a saved capture or bundle");
                        ui.label("One capture.toml or receipt.toml file");
                    } else {
                        ui.heading("Drop a profile or IODD");
                        ui.label("One TOML profile or IODD XML/ZIP file");
                    }
                    if let Some(reason) = self.file_drop_block_reason(target) {
                        ui.colored_label(WARNING, reason);
                    } else if target.saved() {
                        ui.small("Selects the containing folder. Verify it when ready.");
                    } else {
                        ui.small("Selects a path for review. Loading, inspection and connection remain separate actions.");
                    }
                });
            });
    }
}
