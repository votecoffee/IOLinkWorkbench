//! Local evidence from one explicit ifm master inspection.
use super::MasterInspection;
use crate::{
    profile::{Connection, CycleUnit, Profile},
    snapshot_file,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;

const KIND: &str = "master inspection snapshot";
const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MasterInspectionSnapshot {
    pub format_version: u32,
    pub workbench_version: String,
    pub profile_name: String,
    pub endpoint: String,
    pub firmware_pin: String,
    pub cycle_unit: Option<CycleUnit>,
    pub inspection: MasterInspection,
}

impl MasterInspectionSnapshot {
    pub fn new(profile: &Profile, inspection: MasterInspection) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            workbench_version: env!("CARGO_PKG_VERSION").into(),
            profile_name: profile.name.clone(),
            endpoint: profile.connection.endpoint.clone(),
            firmware_pin: profile.connection.ifm_firmware.clone(),
            cycle_unit: profile.connection.ifm_cycle_unit,
            inspection,
        }
    }

    pub fn check_destination(path: &Path) -> Result<()> {
        snapshot_file::check_destination(path)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        snapshot_file::save(self, path, KIND)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let snapshot: Self = snapshot_file::load(path, KIND)?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.format_version == FORMAT_VERSION,
            "unsupported master inspection snapshot version"
        );
        ensure!(
            !self.workbench_version.trim().is_empty(),
            "missing producing Workbench version"
        );
        let report = &self.inspection;
        let model = report
            .gateway
            .ifm_model()
            .context("master inspection requires an ifm gateway")?;
        Connection {
            gateway: report.gateway,
            endpoint: self.endpoint.clone(),
            ..Connection::default()
        }
        .origin()?;
        snapshot_file::check_utc(&report.host_finished_utc)?;
        if let Some(error) = &report.error {
            ensure!(
                !error.trim().is_empty()
                    && report.ports.is_empty()
                    && report.master_diagnostics.is_none(),
                "failed master inspection must retain only initial master evidence"
            );
            return Ok(());
        }
        ensure!(
            report.master.model.as_deref() == Some(model),
            "observed model differs from the configured gateway"
        );
        let firmware = report
            .master
            .firmware
            .as_deref()
            .filter(|version| !version.trim().is_empty())
            .context("completed master inspection requires an observed firmware revision")?;
        ensure!(
            self.firmware_pin.is_empty() || self.firmware_pin == firmware,
            "observed firmware differs from the saved pin"
        );
        ensure!(
            report.master_diagnostics.is_some(),
            "completed master inspection requires the diagnostics section"
        );
        ensure!(
            report
                .ports
                .iter()
                .map(|port| port.port)
                .eq(1..=report.gateway.max_port()),
            "master inspection has missing, duplicate or out-of-order ports"
        );
        for port in &report.ports {
            ensure!(
                port.identity.device_id.is_none_or(|id| id <= 0xFF_FFFF)
                    && port
                        .configuration
                        .validation_device_id
                        .is_none_or(|id| id <= 0xFF_FFFF),
                "port {} has a Device ID outside the 24-bit range",
                port.port
            );
            ensure!(
                self.cycle_unit.is_some()
                    || (port.port_cycle_us.is_none()
                        && port.configuration.preset_cycle_us.is_none()),
                "port {} reports converted cycles without a saved cycle unit",
                port.port
            );
        }
        Ok(())
    }
}
