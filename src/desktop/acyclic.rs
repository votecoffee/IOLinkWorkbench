//! One explicit acyclic operation at a time, with results routed by request.
use super::{App, WARNING, Worker, background, poll_worker};
use eframe::egui::{self, Ui};
use iolink_workbench::transport::{inspect_master, layout_condition};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Named,
    Raw,
    ProductId,
    Events,
    LayoutCondition,
    MasterInspection,
}

pub(super) enum Query {
    Named {
        id: String,
        index: u16,
        subindex: u8,
    },
    Raw(u16, u8),
    ProductId(String),
    Events,
    LayoutCondition(layout_condition::Plan),
    MasterInspection,
}

impl Query {
    fn kind(&self) -> Kind {
        match self {
            Self::Named { .. } => Kind::Named,
            Self::Raw(..) => Kind::Raw,
            Self::ProductId(_) => Kind::ProductId,
            Self::Events => Kind::Events,
            Self::LayoutCondition(_) => Kind::LayoutCondition,
            Self::MasterInspection => Kind::MasterInspection,
        }
    }

    fn label(&self) -> String {
        match self {
            Self::Named {
                id,
                index,
                subindex,
            } => format!("{id} · index {index}:{subindex}"),
            Self::Raw(index, subindex) => format!("Raw ISDU · index {index}:{subindex}"),
            Self::ProductId(expected) => {
                format!("Product ID check · index 19:0 · expected {expected}")
            }
            Self::Events => "Device diagnostics · indexes 36:0 and 37:0".into(),
            Self::LayoutCondition(plan) => format!(
                "Layout {} · {} · index {}:{}",
                plan.layout, plan.parameter_id, plan.index, plan.read_subindex
            ),
            Self::MasterInspection => "Master and port inspection".into(),
        }
    }
}

#[derive(Debug)]
pub(super) struct Output {
    pub text: String,
    pub port_configuration: Option<String>,
}

impl From<String> for Output {
    fn from(text: String) -> Self {
        Self {
            text,
            port_configuration: None,
        }
    }
}

impl From<&str> for Output {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

pub(super) struct Reading {
    pub query: Query,
    pub output: anyhow::Result<Output>,
}

#[derive(Default)]
pub(super) struct Reads {
    pending: Option<(Query, Worker<Output>)>,
    latest: [Option<Reading>; 6],
}

impl Reads {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    pub fn update(&mut self) {
        if let Some((_, worker)) = &self.pending
            && let Some(output) = poll_worker(worker)
        {
            let (query, _) = self.pending.take().expect("pending query");
            self.store_result(query, output);
        }
    }

    pub(super) fn store_result(&mut self, query: Query, output: anyhow::Result<Output>) {
        let slot = query.kind() as usize;
        self.latest[slot] = Some(Reading { query, output });
    }

    pub fn latest(&self, kind: Kind) -> Option<&Reading> {
        self.latest[kind as usize].as_ref()
    }

    pub fn clear_results(&mut self) {
        self.latest = Default::default();
    }

    pub fn clear_named(&mut self, id: &str) {
        if matches!(self.latest(Kind::Named).map(|reading| &reading.query),
            Some(Query::Named { id: read_id, .. }) if read_id == id)
        {
            self.latest[Kind::Named as usize] = None;
        }
    }

    #[cfg(test)]
    pub fn hold_for_test(&mut self, worker: Worker<Output>) {
        assert!(!self.busy());
        self.pending = Some((Query::Raw(19, 0), worker));
    }
}

impl App {
    pub(super) fn master_inspection_panel(&mut self, ui: &mut Ui) {
        if self.profile.connection.gateway.ifm_model().is_none() {
            return;
        }
        egui::CollapsingHeader::new("Master and ports")
            .default_open(self.live.status.is_none())
            .show(ui, |ui| {
                ui.small("Read master identity, supply diagnostics, and metadata for every port.");
                let blocked = self.acyclic_block_reason();
                let button = ui.add_enabled(blocked.is_none(), egui::Button::new("Inspect master"));
                if let Some(reason) = blocked {
                    button.on_disabled_hover_text(reason);
                } else if button.clicked() {
                    let profile = self.profile.clone();
                    self.start_acyclic_output(Query::MasterInspection, move || {
                        let reading = inspect_master(&profile)?;
                        Ok(Output {
                            text: reading.text(),
                            port_configuration: reading.port_configuration_text(),
                        })
                    });
                }
                self.acyclic_result(ui, Kind::MasterInspection);
            });
    }

    pub(super) fn layout_condition_page(&mut self, ui: &mut Ui) {
        if self
            .profile
            .iodd
            .as_ref()
            .and_then(|import| import.condition.as_ref())
            .is_none()
        {
            return;
        }
        egui::CollapsingHeader::new("Layout condition")
            .default_open(true)
            .show(ui, |ui| {
                let plan = match layout_condition::Plan::from_profile(&self.profile) {
                    Ok(plan) => plan,
                    Err(error) => {
                        ui.colored_label(WARNING, format!("{error:#}"));
                        return;
                    }
                };
                ui.label(format!(
                    "{} · {} · ISDU {}:{}",
                    plan.layout, plan.parameter_name, plan.index, plan.read_subindex
                ));
                ui.monospace(&plan.declaration);
                if plan.read_subindex != plan.subindex {
                    ui.small(format!(
                        "Reads the whole value for selector subindex {}",
                        plan.subindex
                    ));
                }
                let blocked = self.acyclic_block_reason();
                let button =
                    ui.add_enabled(blocked.is_none(), egui::Button::new("Read layout selector"));
                if let Some(reason) = blocked {
                    button.on_disabled_hover_text(reason);
                } else if button.clicked() {
                    let profile = self.profile.clone();
                    self.start_acyclic(Query::LayoutCondition(plan), move || {
                        Ok(layout_condition::read(&profile)?.text())
                    });
                }
                self.acyclic_result(ui, Kind::LayoutCondition);
                if !self.profile.acyclic_only() {
                    ui.small(
                        "Review the reading before confirming the active condition in the mapping.",
                    );
                }
            });
    }

    pub(super) fn acyclic_block_reason(&self) -> Option<&'static str> {
        if self.closing {
            Some("Application is closing")
        } else if self.acyclic.busy() {
            Some("An acyclic read is in progress")
        } else if self.writes.busy() {
            Some("Wait for the write workflow's current operation before reading parameters")
        } else if self.setup.pending() || self.setup.busy() {
            Some("Finish IODD inspection and apply or discard profile edits before reading")
        } else {
            None
        }
    }

    pub(super) fn start_acyclic(
        &mut self,
        query: Query,
        work: impl FnOnce() -> anyhow::Result<String> + Send + 'static,
    ) {
        self.start_acyclic_output(query, move || work().map(Output::from));
    }

    fn start_acyclic_output(
        &mut self,
        query: Query,
        work: impl FnOnce() -> anyhow::Result<Output> + Send + 'static,
    ) {
        if let Some(reason) = self.acyclic_block_reason() {
            self.message = reason.into();
            return;
        }
        match background("acyclic-read", work) {
            Ok(worker) => self.acyclic.pending = Some((query, worker)),
            Err(error) => self.message = format!("Could not start {} · {error}", query.label()),
        }
    }

    pub(super) fn acyclic_result(&self, ui: &mut Ui, kind: Kind) {
        if let Some((query, _)) = &self.acyclic.pending
            && kind == query.kind()
        {
            ui.label(format!("Reading {}…", query.label()));
        }
        if let Some(reading) = self.acyclic.latest(kind) {
            ui.separator();
            ui.strong(format!("Last result · {}", reading.query.label()));
            match &reading.query {
                Query::Named { id, subindex, .. }
                    if self.parameter_picker.selected.as_ref() != Some(id)
                        || self.parameter_picker.subindex != *subindex =>
                {
                    ui.small("The current IODD selection differs from this read.");
                }
                Query::Raw(index, subindex)
                    if (self.isdu_index, self.isdu_subindex) != (*index, *subindex) =>
                {
                    ui.small("The current address differs from this read.");
                }
                _ => {}
            }
            match &reading.output {
                Ok(output) => {
                    if let Some(configuration) = &output.port_configuration {
                        ui.collapsing("Port configuration", |ui| {
                            ui.monospace(configuration);
                        });
                    }
                    ui.monospace(&output.text);
                }
                Err(error) => {
                    ui.colored_label(WARNING, format!("{error:#}"));
                }
            }
        }
    }
}
