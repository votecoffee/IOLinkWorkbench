//! One explicit acyclic operation at a time, with results routed by request.
use super::{App, WARNING, Worker, background, poll_worker};
use eframe::egui::{self, Ui};
use iolink_workbench::transport::{
    MasterInspection, MasterInspectionComparison, MasterInspectionSnapshot, inspect_master,
    layout_condition,
};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Named,
    Raw,
    ProductId,
    Events,
    LayoutCondition,
    MasterInspection,
    SavedMasterInspection,
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
    SavedMasterInspection(PathBuf),
    CompareMasterSnapshots(PathBuf, PathBuf),
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
            Self::SavedMasterInspection(_) | Self::CompareMasterSnapshots(..) => {
                Kind::SavedMasterInspection
            }
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
            Self::SavedMasterInspection(path) => {
                format!("Saved master inspection · {}", path.display())
            }
            Self::CompareMasterSnapshots(left, right) => {
                format!(
                    "Master snapshots · {} → {}",
                    left.display(),
                    right.display()
                )
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct Output {
    pub text: String,
    pub port_configuration: Option<String>,
    pub comparison_sources: Option<[String; 2]>,
}

impl From<&MasterInspection> for Output {
    fn from(inspection: &MasterInspection) -> Self {
        Self {
            text: inspection.text(),
            port_configuration: inspection.port_configuration_text(),
            comparison_sources: None,
        }
    }
}

impl From<String> for Output {
    fn from(text: String) -> Self {
        Self {
            text,
            port_configuration: None,
            comparison_sources: None,
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
    latest: [Option<Reading>; 7],
    master_save_path: String,
    master_review_path: String,
    master_compare_path: String,
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
        let saved_master = self.latest[Kind::SavedMasterInspection as usize].take();
        self.latest = Default::default();
        self.latest[Kind::SavedMasterInspection as usize] = saved_master;
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

fn saved_master_output(snapshot: &MasterInspectionSnapshot) -> Output {
    let mut output = Output::from(&snapshot.inspection);
    output.text = format!(
        "Saved profile {} · Endpoint {}\nProducer {}\n{}",
        snapshot.profile_name, snapshot.endpoint, snapshot.workbench_version, output.text
    );
    output
}

fn saved_master_text(snapshot: &MasterInspectionSnapshot) -> String {
    let mut output = saved_master_output(snapshot);
    if let Some(configuration) = output.port_configuration {
        output.text.push_str("\n\nPort configuration\n");
        output.text.push_str(&configuration);
    }
    output.text
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
                    self.run_master_inspection(None);
                }
                ui.collapsing("Read and save master snapshot", |ui| {
                    ui.add_enabled(
                        blocked.is_none(),
                        egui::TextEdit::singleline(&mut self.acyclic.master_save_path)
                            .hint_text("New master snapshot JSON path"),
                    );
                    let path = self.acyclic.master_save_path.trim();
                    if ui
                        .add_enabled(
                            blocked.is_none() && !path.is_empty(),
                            egui::Button::new("Inspect and save master snapshot"),
                        )
                        .clicked()
                    {
                        self.run_master_inspection(Some(PathBuf::from(path)));
                    }
                });
                self.acyclic_result(ui, Kind::MasterInspection);
            });
    }

    fn run_master_inspection(&mut self, destination: Option<PathBuf>) {
        let profile = self.profile.clone();
        self.start_acyclic_output(Query::MasterInspection, move || {
            if let Some(path) = &destination {
                MasterInspectionSnapshot::check_destination(path)?;
            }
            let reading = inspect_master(&profile)?;
            let mut output = Output::from(&reading);
            if let Some(path) = destination {
                let snapshot = MasterInspectionSnapshot::new(&profile, reading);
                match snapshot.save(&path) {
                    Ok(()) => output
                        .text
                        .push_str(&format!("\n\nSaved master snapshot {}", path.display())),
                    Err(error) => output
                        .text
                        .push_str(&format!("\n\nMaster snapshot save failed · {error:#}")),
                }
            }
            Ok(output)
        });
    }

    pub(super) fn saved_master_panel(&mut self, ui: &mut Ui) {
        egui::CollapsingHeader::new("Master inspection snapshots")
            .default_open(self.acyclic.latest(Kind::SavedMasterInspection).is_some())
            .show(ui, |ui| {
                ui.small("Review a saved master and port inventory offline.");
                let available = self.acyclic_job_block_reason().is_none();
                ui.add_enabled(
                    available,
                    egui::TextEdit::singleline(&mut self.acyclic.master_review_path)
                        .hint_text("Saved master snapshot JSON path"),
                );
                let path = self.acyclic.master_review_path.trim();
                if ui
                    .add_enabled(
                        available && !path.is_empty(),
                        egui::Button::new("Open master snapshot"),
                    )
                    .clicked()
                {
                    let path = PathBuf::from(path);
                    self.start_acyclic_output(
                        Query::SavedMasterInspection(path.clone()),
                        move || {
                            let snapshot = MasterInspectionSnapshot::load(&path)?;
                            Ok(saved_master_output(&snapshot))
                        },
                    );
                }
                ui.collapsing("Compare two master snapshots", |ui| {
                    ui.add_enabled(
                        available,
                        egui::TextEdit::singleline(&mut self.acyclic.master_compare_path)
                            .hint_text("Second master snapshot JSON path"),
                    );
                    let left = self.acyclic.master_review_path.trim();
                    let right = self.acyclic.master_compare_path.trim();
                    if ui
                        .add_enabled(
                            available && !left.is_empty() && !right.is_empty(),
                            egui::Button::new("Compare master snapshots"),
                        )
                        .clicked()
                    {
                        let (left, right) = (PathBuf::from(left), PathBuf::from(right));
                        self.start_acyclic_output(
                            Query::CompareMasterSnapshots(left.clone(), right.clone()),
                            move || {
                                let comparison = MasterInspectionComparison::load(&left, &right)?;
                                Ok(Output {
                                    text: comparison.text(),
                                    comparison_sources: Some([
                                        saved_master_text(&comparison.left),
                                        saved_master_text(&comparison.right),
                                    ]),
                                    port_configuration: None,
                                })
                            },
                        );
                    }
                });
                self.acyclic_result(ui, Kind::SavedMasterInspection);
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

    fn acyclic_job_block_reason(&self) -> Option<&'static str> {
        if self.closing {
            Some("Application is closing")
        } else if self.acyclic.busy() {
            Some("An acyclic read is in progress")
        } else {
            None
        }
    }

    pub(super) fn acyclic_block_reason(&self) -> Option<&'static str> {
        if let Some(reason) = self.acyclic_job_block_reason() {
            Some(reason)
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
        let blocked = match &query {
            Query::SavedMasterInspection(_) | Query::CompareMasterSnapshots(..) => {
                self.acyclic_job_block_reason()
            }
            _ => self.acyclic_block_reason(),
        };
        if let Some(reason) = blocked {
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
                    if let Some([left, right]) = &output.comparison_sources {
                        ui.collapsing("Left master snapshot", |ui| ui.monospace(left));
                        ui.collapsing("Right master snapshot", |ui| ui.monospace(right));
                    }
                }
                Err(error) => {
                    ui.colored_label(WARNING, format!("{error:#}"));
                }
            }
        }
    }
}
