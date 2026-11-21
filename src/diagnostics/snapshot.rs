//! Saved evidence from one explicit pair of diagnostic reads.
use super::{
    DetailedStatus, DeviceStatus, Observation, Report, SNAPSHOT_KIND, append_results,
    decode_detailed, decode_device_status, diagnostic_result_ok,
};
use crate::{
    decode,
    parameter_snapshot::Observation as ConnectionObservation,
    profile::{Gateway, Profile},
    snapshot_file,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SavedRead {
    pub index: u16,
    pub subindex: u8,
    pub host_read_utc: String,
    pub read_duration_us: u64,
    pub observation: Option<ConnectionObservation>,
    pub raw_hex: Option<String>,
    pub read_error: Option<String>,
}

impl SavedRead {
    fn new(gateway: Gateway, read: &Observation) -> Self {
        Self {
            index: read.index,
            subindex: read.subindex,
            host_read_utc: read.host_read_utc.clone(),
            read_duration_us: read.read_duration_us,
            observation: read
                .status
                .clone()
                .map(|status| ConnectionObservation::new(gateway, status)),
            raw_hex: read.raw_hex.clone(),
            read_error: read.read_error.clone(),
        }
    }

    fn checked_bytes(&self, profile: &Profile, index: u16) -> Result<Option<Vec<u8>>> {
        ensure!(
            (self.index, self.subindex) == (index, 0),
            "expected diagnostic query {index}:0"
        );
        snapshot_file::check_utc(&self.host_read_utc)?;
        if let Some(observation) = &self.observation {
            observation.validate_for(profile)?;
            let connection = &profile.connection;
            if let Some(model) = connection.gateway.ifm_model() {
                ensure!(
                    observation.model == model,
                    "recorded model differs from the {model} route"
                );
                ensure!(
                    connection.ifm_firmware.is_empty()
                        || observation.firmware == connection.ifm_firmware,
                    "recorded firmware differs from the saved pin"
                );
            }
        }
        match (&self.raw_hex, &self.read_error) {
            (Some(raw_hex), None) => {
                ensure!(
                    self.observation.is_some(),
                    "diagnostic bytes require a connection observation"
                );
                // A successful query can return an empty value. Retain it for length review.
                let bytes = if raw_hex.is_empty() {
                    Vec::new()
                } else {
                    decode::parse_hex_bounded(raw_hex, 232)?
                };
                ensure!(
                    decode::hex(&bytes) == *raw_hex,
                    "diagnostic bytes must be canonical uppercase hex"
                );
                Ok(Some(bytes))
            }
            (None, Some(error)) if !error.trim().is_empty() => Ok(None),
            _ => bail!("each diagnostic query requires either raw bytes or a read error"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub format_version: u32,
    pub workbench_version: String,
    pub profile: Profile,
    pub reads: [SavedRead; 2],
}

#[derive(Debug, Serialize)]
pub struct Review {
    pub observation_kind: &'static str,
    pub workbench_version: String,
    pub decoder_version: &'static str,
    pub profile_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iodd_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iodd_stamp: Option<crate::iodd::stamp::Report>,
    pub observed_identities_equal: Option<bool>,
    pub observed_master_conflict: bool,
    pub device_status: Option<DeviceStatus>,
    pub detailed_status: Option<DetailedStatus>,
    pub reads: [SavedRead; 2],
}

impl Review {
    pub fn ok(&self) -> bool {
        diagnostic_result_ok(
            self.device_status.as_ref(),
            self.detailed_status.as_ref(),
            self.observed_identities_equal,
            self.observed_master_conflict,
        )
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "Saved diagnostic snapshot · {}\nProducer {} · decoder {}\nExplicit acyclic snapshots · source event history/loss unknown",
            self.profile_name, self.workbench_version, self.decoder_version
        );
        if let Some(source) = &self.iodd_source {
            text.push_str(&format!("\nSaved IODD {source}"));
        }
        if let Some(stamp) = &self.iodd_stamp {
            text.push_str(&format!("\n{}", stamp.summary()));
        }
        for read in &self.reads {
            text.push_str(&format!(
                "\n\n{}:{} · host {} · query {} µs",
                read.index, read.subindex, read.host_read_utc, read.read_duration_us,
            ));
            if let Some(observation) = &read.observation {
                text.push_str(&format!(
                    "\n{} · master {} · firmware {} · serial {:?} · port {} · {}\nVendor {:?} · Device {:?} · Device serial {:?}\nConnection status {:?} · port event {:?} · device event {:?}",
                    observation.route, observation.model, observation.firmware, observation.master_serial,
                    observation.port, observation.port_mode, observation.identity.vendor_id, observation.identity.device_id,
                    observation.identity.serial, observation.device_status, observation.port_event, observation.device_event,
                ));
            }
            if let Some(error) = &read.read_error {
                text.push_str(&format!("\nRead error · {error}"));
            }
        }
        append_results(
            &mut text,
            self.device_status.as_ref(),
            self.detailed_status.as_ref(),
            self.observed_identities_equal,
            self.observed_master_conflict,
        );
        text
    }
}

impl Snapshot {
    fn checked_reads(&self) -> Result<[Option<Vec<u8>>; 2]> {
        ensure!(
            self.format_version == 1,
            "unsupported diagnostic snapshot version"
        );
        ensure!(
            !self.workbench_version.trim().is_empty(),
            "snapshot producer version is empty"
        );
        self.profile.validate()?;
        Ok([
            self.reads[0]
                .checked_bytes(&self.profile, 36)
                .context("diagnostic snapshot 36:0")?,
            self.reads[1]
                .checked_bytes(&self.profile, 37)
                .context("diagnostic snapshot 37:0")?,
        ])
    }

    pub fn review(&self) -> Result<Review> {
        let [device, detailed] = self.checked_reads()?;
        let context = self.reads[0]
            .observation
            .as_ref()
            .zip(self.reads[1].observation.as_ref());
        Ok(Review {
            observation_kind: SNAPSHOT_KIND,
            workbench_version: self.workbench_version.clone(),
            decoder_version: env!("CARGO_PKG_VERSION"),
            profile_name: self.profile.name.clone(),
            iodd_source: self.profile.iodd.as_ref().map(|iodd| iodd.source.clone()),
            iodd_stamp: self
                .profile
                .iodd
                .as_ref()
                .and_then(|iodd| iodd.stamp_validation.clone()),
            observed_identities_equal: context.map(|(a, b)| a.identity == b.identity),
            observed_master_conflict: context.is_some_and(|(a, b)| {
                a.model != b.model
                    || a.firmware != b.firmware
                    || a.master_serial
                        .as_ref()
                        .zip(b.master_serial.as_ref())
                        .is_some_and(|(a, b)| a != b)
            }),
            device_status: device.as_deref().map(decode_device_status),
            detailed_status: detailed
                .as_deref()
                .map(|bytes| decode_detailed(&self.profile, bytes)),
            reads: self.reads.clone(),
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let snapshot: Self = snapshot_file::load(path, "diagnostic snapshot")?;
        snapshot.checked_reads()?;
        Ok(snapshot)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.checked_reads()?;
        snapshot_file::save(self, path, "diagnostic snapshot")
    }
}

/// Read the two diagnostics once, retaining partial responses and their connection context.
pub fn read_snapshot(profile: &Profile) -> Result<(Snapshot, Report)> {
    let report = super::read(profile)?;
    let [device, detailed] = report.reads.as_slice() else {
        bail!("expected two diagnostic reads");
    };
    let gateway = profile.connection.gateway;
    Ok((
        Snapshot {
            format_version: 1,
            workbench_version: env!("CARGO_PKG_VERSION").into(),
            profile: profile.clone(),
            reads: [
                SavedRead::new(gateway, device),
                SavedRead::new(gateway, detailed),
            ],
        },
        report,
    ))
}
