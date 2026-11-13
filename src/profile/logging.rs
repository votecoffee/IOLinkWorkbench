//! Saved logging settings and an offline plan; no observed transport state is inferred.
use super::{Gateway, LogMode, Logging, Profile};
use crate::decode::Field;
use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Serialize)]
pub struct PlannedField<'a> {
    pub position: usize,
    pub origin: &'static str,
    pub mapping: &'a Field,
}

#[derive(Serialize)]
pub struct CapturePlan<'a> {
    pub basis: &'static str,
    pub configured_gateway: Gateway,
    pub configured_row_kind: &'static str,
    pub source_update_loss: &'static str,
    pub row_eligibility: &'static str,
    pub rotation_rule: &'static str,
    pub requested_host_poll_us: u64,
    pub selection: &'static str,
    pub settings: &'a Logging,
    pub selected_fields: Vec<PlannedField<'a>>,
    pub csv_columns: Vec<String>,
}

impl CapturePlan<'_> {
    pub fn text(&self) -> String {
        format!(
            "{}\nConfigured rows · {} · {:?}\n{}\nSource loss · {}\nRequested host poll {} µs · queue {} rows\nSelection {} · {} value columns · {} total CSV columns\nRotation · {} rows / {} seconds (0 disables each limit)\n{}",
            self.basis,
            self.configured_row_kind,
            self.settings.mode,
            self.row_eligibility,
            self.source_update_loss,
            self.requested_host_poll_us,
            self.settings.queue_capacity,
            self.selection,
            self.selected_fields.len(),
            self.csv_columns.len(),
            self.settings.rotate_rows,
            self.settings.rotate_seconds,
            self.rotation_rule
        )
    }
}

impl Profile {
    /// Build a validated logging candidate without mutating any source setting.
    pub fn with_logging(&self, logging: Logging) -> Result<Self> {
        ensure!(
            logging.rotate_rows <= i64::MAX as u64 && logging.rotate_seconds <= i64::MAX as u64,
            "logging rotation values must fit the TOML signed 64-bit integer range"
        );
        let mut candidate = self.clone();
        candidate.logging = logging;
        candidate.validate()?;
        Ok(candidate)
    }

    pub fn logging_plan(&self) -> Result<CapturePlan<'_>> {
        self.validate()?;
        let snapshots = match self.connection.gateway {
            Gateway::Simulator => self.connection.simulate_snapshots,
            Gateway::IfmAl1350 => true,
        };
        let fields = self.effective_fields();
        let selected_fields = self
            .log_fields()
            .into_iter()
            .enumerate()
            .map(|(index, field)| PlannedField {
                position: index + 1,
                origin: fields
                    .iter()
                    .find(|(candidate, _)| candidate.name == field.name)
                    .map(|(_, origin)| *origin)
                    .expect("validated field origin"),
                mapping: field,
            })
            .collect();
        Ok(CapturePlan {
            basis: "saved configuration only; no gateway accessed or runtime freshness/rate verified",
            configured_gateway: self.connection.gateway,
            configured_row_kind: if snapshots {
                "host_poll"
            } else {
                "source_update"
            },
            source_update_loss: if snapshots {
                "unknown; configured snapshot path exposes no verifiable source counter/timestamp"
            } else {
                "synthetic counter gaps measurable within a simulated connection generation; disconnect loss unknown; no physical rate/loss claim"
            },
            row_eligibility: match self.logging.mode {
                LogMode::EveryUpdate => {
                    "Every accepted, verifiably new source update is eligible, including equal consecutive payloads; snapshot paths instead record accepted host polls"
                }
                LogMode::OnChange => {
                    "First eligible row per connection generation and changes in the complete raw payload, validity, stale flag or diagnostic code; value selection/scaling does not redefine change"
                }
            },
            rotation_rule: "Thresholds are checked before writing the next row; time uses receive elapsed timestamps from the first row in each part; either enabled threshold can rotate; no timer-driven empty parts",
            requested_host_poll_us: self.connection.poll_us,
            selection: if self.logging.raw_only {
                "raw_only"
            } else if self.logging.fields.is_empty() {
                "all_mapped"
            } else {
                "explicit_order"
            },
            settings: &self.logging,
            selected_fields,
            csv_columns: crate::capture::csv_header(self),
        })
    }
}

#[cfg(test)]
mod tests;
