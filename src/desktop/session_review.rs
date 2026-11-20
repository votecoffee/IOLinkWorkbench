//! Local checks for the read route selected in a GUI session.
use super::{App, Tab, WARNING, capture, setup, transport};
use eframe::egui::Ui;

pub(super) enum RouteIssue {
    Connection(anyhow::Error),
    Layout(anyhow::Error),
}

impl RouteIssue {
    pub(super) fn error(&self) -> &anyhow::Error {
        match self {
            Self::Connection(error) | Self::Layout(error) => error,
        }
    }

    pub(super) fn into_error(self) -> anyhow::Error {
        match self {
            Self::Connection(error) | Self::Layout(error) => error,
        }
    }
}

impl App {
    pub(super) fn check_session_route(&self) -> Result<(), RouteIssue> {
        self.profile
            .require_process_input()
            .map_err(RouteIssue::Layout)?;
        transport::check_route(&self.profile).map_err(RouteIssue::Connection)?;
        capture::check_layout(&self.profile).map_err(RouteIssue::Layout)
    }

    pub(super) fn show_route_issue(&mut self, ui: &mut Ui, issue: &RouteIssue) {
        ui.colored_label(WARNING, format!("{:#}", issue.error()));
        let (button, tab) = match issue {
            RouteIssue::Connection(_) => ("Go to connection settings", Tab::Device),
            RouteIssue::Layout(_) => ("Review IODD layout", Tab::Mappings),
        };
        if ui.small_button(button).clicked() {
            self.tab = tab;
            if tab == Tab::Device {
                self.setup.page = setup::Page::Connection;
            } else {
                self.setup.show_mapping_list();
            }
            self.workspace.connect_dialog = false;
        }
    }
}
