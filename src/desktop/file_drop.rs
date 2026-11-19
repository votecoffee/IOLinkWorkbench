//! File drops select host paths only. Reading and applying remain explicit actions.
use super::{App, Tab, WARNING};
use eframe::egui::{self, Context};

impl App {
    fn file_drop_block_reason(&self) -> Option<&'static str> {
        self.start_block_reason().or_else(|| {
            self.isdu_worker
                .is_some()
                .then_some("Wait for the active parameter read before selecting a file")
        })
    }

    pub(super) fn handle_file_drop(&mut self, ctx: &Context) {
        let files = ctx.input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
        if files.is_empty() {
            return;
        }
        if let Some(reason) = self.file_drop_block_reason() {
            self.message = format!("File selection unchanged · {reason}");
            return;
        }
        if files.len() != 1 {
            self.message = "Drop one file at a time: a TOML profile or IODD XML/ZIP. File selection unchanged.".into();
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
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if extension.eq_ignore_ascii_case("toml") {
            self.setup.select_profile_path(text.into());
            self.workspace.connect_dialog = false;
            self.workspace.profile_dialog = true;
            self.message = "Profile path selected. Choose Load profile to validate and apply it; current settings are unchanged.".into();
        } else if extension.eq_ignore_ascii_case("xml") || extension.eq_ignore_ascii_case("zip") {
            self.setup.select_iodd_path(text.into());
            self.workspace.connect_dialog = false;
            self.workspace.profile_dialog = false;
            self.tab = Tab::Device;
            self.message = "IODD path selected. Review import options, then Inspect IODD; the applied profile is unchanged.".into();
        } else {
            self.message = "Drop a TOML profile or IODD XML/ZIP. Use Offline for saved captures. File selection unchanged.".into();
        }
    }

    pub(super) fn file_drop_preview(&self, ctx: &Context) {
        if ctx.input(|input| input.raw.hovered_files.is_empty()) {
            return;
        }
        egui::Area::new(egui::Id::new("file-drop-preview"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(16.0).show(ui, |ui| {
                    ui.set_max_width(440.0);
                    ui.heading("Drop a profile or IODD");
                    ui.label("One TOML profile or IODD XML/ZIP file");
                    if let Some(reason) = self.file_drop_block_reason() {
                        ui.colored_label(WARNING, reason);
                    } else {
                        ui.small("Selects a path for review. Loading, inspection and connection remain separate actions.");
                    }
                });
            });
    }
}
