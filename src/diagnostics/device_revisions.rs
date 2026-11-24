//! Device HardwareRevision and FirmwareRevision, IO-Link V1.1.5 Table B.8.

use super::{
    SavedRead,
    comparison::{ContextIssue, match_context},
    observe,
};
use crate::{decode, profile::Profile, snapshot_file, transport};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt::Write, path::Path};

pub const HARDWARE_REVISION_INDEX: u16 = 22;
pub const FIRMWARE_REVISION_INDEX: u16 = 23;
const MAX_REVISION_BYTES: usize = 64;
const SNAPSHOT_KIND: &str = "device revisions snapshot";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevisionKind {
    Hardware,
    Firmware,
}

impl RevisionKind {
    pub const ALL: [Self; 2] = [Self::Hardware, Self::Firmware];

    pub fn index(self) -> u16 {
        match self {
            Self::Hardware => HARDWARE_REVISION_INDEX,
            Self::Firmware => FIRMWARE_REVISION_INDEX,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hardware => "Device hardware revision",
            Self::Firmware => "Device firmware revision",
        }
    }
}

impl TryFrom<u16> for RevisionKind {
    type Error = anyhow::Error;

    fn try_from(index: u16) -> Result<Self> {
        match index {
            HARDWARE_REVISION_INDEX => Ok(Self::Hardware),
            FIRMWARE_REVISION_INDEX => Ok(Self::Firmware),
            _ => bail!("device revision requires index 22 or 23"),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RevisionValue {
    pub index: u16,
    pub raw_hex: String,
    pub value: Option<String>,
    pub decode_error: Option<String>,
}

impl RevisionValue {
    pub fn ok(&self) -> bool {
        self.decode_error.is_none()
    }

    pub fn text(&self) -> String {
        let raw = if self.raw_hex.is_empty() {
            "Empty"
        } else {
            &self.raw_hex
        };
        let mut text = format!("Raw reply · {raw}");
        if let Some(value) = &self.value {
            let _ = write!(text, "\nRevision · {value:?}");
        }
        if let Some(error) = &self.decode_error {
            let _ = write!(text, "\nReview · {error}");
        }
        text
    }
}

fn decode_bytes(kind: RevisionKind, bytes: &[u8]) -> RevisionValue {
    let decoded = if bytes.len() > MAX_REVISION_BYTES {
        Err(anyhow::anyhow!("revision reply exceeds 64 bytes"))
    } else {
        decode::decode_padded_text(bytes, false)
    };
    let (value, decode_error) = match decoded {
        Ok(text) => (
            Some(text.to_owned()),
            text.trim()
                .is_empty()
                .then_some("Device revision is blank".to_owned()),
        ),
        Err(error) => (None, Some(error.to_string())),
    };
    RevisionValue {
        index: kind.index(),
        raw_hex: decode::hex(bytes),
        value,
        decode_error,
    }
}

pub fn decode_device_revision(kind: RevisionKind, hex: &str) -> Result<RevisionValue> {
    let bytes = if hex.trim().is_empty() {
        Vec::new()
    } else {
        decode::parse_hex_bounded(hex, 232)?
    };
    Ok(decode_bytes(kind, &bytes))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceRevisionsSnapshot {
    pub format_version: u32,
    pub workbench_version: String,
    pub profile: Profile,
    pub reads: [SavedRead; 2],
}

#[derive(Debug, Serialize)]
pub struct DeviceRevisionsReview<'a> {
    pub workbench_version: &'a str,
    pub decoder_version: &'static str,
    pub profile_name: &'a str,
    pub observed_context_matches: Option<bool>,
    pub context_issue: Option<&'static str>,
    pub hardware: Option<RevisionValue>,
    pub firmware: Option<RevisionValue>,
    pub reads: &'a [SavedRead; 2],
}

impl DeviceRevisionsReview<'_> {
    pub fn ok(&self) -> bool {
        self.observed_context_matches != Some(false)
            && self.hardware.as_ref().is_some_and(RevisionValue::ok)
            && self.firmware.as_ref().is_some_and(RevisionValue::ok)
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "Device revisions · {}\nProducer {} · decoder {}",
            self.profile_name, self.workbench_version, self.decoder_version
        );
        if let Some(issue) = self.context_issue {
            let _ = write!(text, "\nReview · {issue}");
        }
        let values = [self.hardware.as_ref(), self.firmware.as_ref()];
        for ((read, kind), value) in self.reads.iter().zip(RevisionKind::ALL).zip(values) {
            let _ = write!(
                text,
                "\n\n{} · {}:0\nHost {} · query {} µs",
                kind.label(),
                read.index,
                read.host_read_utc,
                read.read_duration_us
            );
            if let Some(observed) = &read.observation {
                let _ = write!(
                    text,
                    "\n{} · {} master firmware {} · port {} · {}\nMaster serial {:?} · Vendor {:?} · Device {:?} · Sensor serial {:?}",
                    observed.route,
                    observed.model,
                    observed.firmware,
                    observed.port,
                    observed.port_mode,
                    observed.master_serial,
                    observed.identity.vendor_id,
                    observed.identity.device_id,
                    observed.identity.serial
                );
            }
            if let Some(error) = &read.read_error {
                let _ = write!(text, "\nRead error · {error}");
            }
            if let Some(value) = value {
                let _ = write!(text, "\n{}", value.text());
            }
        }
        text
    }
}

impl DeviceRevisionsSnapshot {
    pub fn read(profile: &Profile) -> Result<Self> {
        profile.validate()?;
        transport::check_route(profile)?;
        Ok(Self {
            format_version: 1,
            workbench_version: env!("CARGO_PKG_VERSION").into(),
            profile: profile.clone(),
            reads: RevisionKind::ALL.map(|kind| {
                SavedRead::new(profile.connection.gateway, &observe(profile, kind.index()))
            }),
        })
    }

    fn checked_bytes(&self) -> Result<[Option<Vec<u8>>; 2]> {
        ensure!(
            self.format_version == 1,
            "unsupported device revisions snapshot version"
        );
        ensure!(
            !self.workbench_version.trim().is_empty(),
            "snapshot producer version is empty"
        );
        self.profile.validate()?;
        let [hardware, firmware] = &self.reads;
        Ok([
            hardware
                .checked_bytes(&self.profile, HARDWARE_REVISION_INDEX)
                .context("device revisions snapshot 22:0")?,
            firmware
                .checked_bytes(&self.profile, FIRMWARE_REVISION_INDEX)
                .context("device revisions snapshot 23:0")?,
        ])
    }

    pub fn review(&self) -> Result<DeviceRevisionsReview<'_>> {
        let [hardware, firmware] = self.checked_bytes()?;
        let context = match_context(&self.reads[0], &self.reads[1]);
        let observed_context_matches = match &context {
            Ok(()) => Some(true),
            Err(ContextIssue::Different(_)) => Some(false),
            Err(ContextIssue::Unavailable(_)) => None,
        };
        Ok(DeviceRevisionsReview {
            workbench_version: &self.workbench_version,
            decoder_version: env!("CARGO_PKG_VERSION"),
            profile_name: &self.profile.name,
            observed_context_matches,
            context_issue: context.err().map(ContextIssue::message),
            hardware: hardware
                .as_deref()
                .map(|bytes| decode_bytes(RevisionKind::Hardware, bytes)),
            firmware: firmware
                .as_deref()
                .map(|bytes| decode_bytes(RevisionKind::Firmware, bytes)),
            reads: &self.reads,
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let snapshot: Self = snapshot_file::load(path, SNAPSHOT_KIND)?;
        snapshot.checked_bytes()?;
        Ok(snapshot)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.checked_bytes()?;
        snapshot_file::save(self, path, SNAPSHOT_KIND)
    }
}
