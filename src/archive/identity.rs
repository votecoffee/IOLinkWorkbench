//! Saved connection observations, distinct from configured pins and live verification.
use super::Manifest;
use crate::{
    profile::{Identity, Profile},
    transport::Freshness,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RecordedStatus {
    pub freshness: Freshness,
    pub route: Option<String>,
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub master_serial: Option<String>,
    pub port: Option<u8>,
    pub port_mode: Option<String>,
    pub device_status: Option<String>,
    pub port_event: Option<String>,
    pub device_event: Option<String>,
    pub port_cycle_raw: Option<String>,
    pub port_cycle_us: Option<u64>,
    pub identity: Option<Identity>,
}

#[derive(Debug, Serialize)]
pub struct IdentityReview {
    pub basis: &'static str,
    pub expected: Identity,
    pub accepted_recheck: String,
    pub producer_check: Option<String>,
    pub last_accepted: Option<RecordedStatus>,
    pub rejected: Option<RecordedStatus>,
    pub reconnects: u64,
    pub notes: Vec<&'static str>,
}

pub(super) fn validate(manifest: &Manifest, profile: &Profile) -> Result<()> {
    if let Some(actual) = manifest
        .last_status
        .as_ref()
        .and_then(|s| s.identity.as_ref())
    {
        ensure!(
            actual.device_id.is_none_or(|id| id <= 0xff_ffff),
            "last accepted device ID exceeds the IO-Link 24-bit range"
        );
        profile
            .expected_identity
            .verify(actual)
            .context("last accepted identity contradicts the saved profile")?;
    }
    // A rejected connection can disagree with pins; it never supplied accepted capture input.
    Ok(())
}

impl IdentityReview {
    pub(super) fn new(manifest: &Manifest, profile: &Profile) -> Option<Self> {
        let accepted = manifest.last_status.as_ref();
        let details = accepted.is_some_and(|s| {
            s.identity.is_some()
                || s.model.is_some()
                || s.firmware.is_some()
                || s.port.is_some()
                || s.port_mode.is_some()
                || s.master_serial.is_some()
        });
        if !details
            && manifest.rejected_status.is_none()
            && manifest.identity_check.is_none()
            && profile.expected_identity == Identity::default()
        {
            return None;
        }
        let recheck = match accepted {
            None => "no accepted connection status recorded",
            Some(status) => match &status.identity {
                None => "recorded device identity unavailable",
                // Common manifest validation already rejects known accepted-pin contradictions.
                Some(actual) => profile
                    .expected_identity
                    .verify(actual)
                    .unwrap_or("identity mismatch"),
            },
        };
        let mut notes = vec![
            "Offline comparison against the final accepted saved identity only; no master, ISDU read or live identity query performed",
            "Accepted and rejected statuses are separate recorded observations; rejected status is not the source of accepted CSV input",
            "No per-generation connection-status journal is stored; reconnect counts and CSV generations do not reconstruct identity history",
            "Matching recorded fields do not authenticate a device, establish physical origin or prove the same unit when serial numbers are unavailable",
        ];
        if accepted.is_some_and(|s| s.master_serial.is_none()) {
            notes.push("Accepted master serial is unavailable; no unique master-unit match can be established");
        }
        if accepted.is_some_and(|s| s.identity.as_ref().is_none_or(|id| id.serial.is_none())) {
            notes.push("Accepted device serial is unavailable; matching model IDs do not establish the same sensor unit");
        }
        Some(Self {
            basis: "saved connection observations and configured expectations; source metadata is a recorded claim, not a live verification or authentication",
            expected: profile.expected_identity.clone(),
            accepted_recheck: recheck.into(),
            producer_check: manifest.identity_check.clone(),
            last_accepted: accepted.cloned(),
            rejected: manifest.rejected_status.clone(),
            reconnects: manifest.counts.reconnects,
            notes,
        })
    }
}

#[cfg(test)]
mod tests;
