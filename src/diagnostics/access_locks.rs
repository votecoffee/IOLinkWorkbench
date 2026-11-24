//! Decode and preserve one explicit DeviceAccessLocks read (12:0).
use super::{ReplyComparison, SavedRead, observe};
use crate::{decode, profile::Profile, snapshot_file, transport};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt::Write, path::Path};

const INDEX: u16 = 12;
const SNAPSHOT_KIND: &str = "device access locks snapshot";
const NAMES: [&str; 4] = [
    "Parameter write access",
    "Data Storage",
    "Local parameterization",
    "Local user interface",
];

#[derive(Debug, Serialize)]
pub struct AccessLockFlag {
    pub bit: usize,
    pub name: &'static str,
    pub locked: bool,
    pub iodd_supported: Option<bool>,
    pub legacy: bool,
}

#[derive(Debug, Serialize)]
pub struct DeviceAccessLocks {
    pub raw_hex: String,
    pub reserved_mask: Option<u16>,
    pub flags: Vec<AccessLockFlag>,
    pub decode_error: Option<&'static str>,
}

impl DeviceAccessLocks {
    pub fn ok(&self) -> bool {
        self.reserved_mask == Some(0)
            && !self
                .flags
                .iter()
                .any(|flag| flag.locked && flag.iodd_supported == Some(false))
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "Device Access Locks · 12:0\nRaw reply · {}",
            if self.raw_hex.is_empty() {
                "Empty"
            } else {
                &self.raw_hex
            },
        );
        if let Some(error) = self.decode_error {
            let _ = write!(text, "\n{error}");
        }
        for flag in &self.flags {
            let observed = if flag.locked { "Set" } else { "Clear" };
            let declared = match flag.iodd_supported {
                Some(true) => "Yes",
                Some(false) => "No",
                None => "Undeclared",
            };
            let legacy = if flag.legacy { " · Legacy" } else { "" };
            let _ = write!(
                text,
                "\nBit {} · {} · {observed} · IODD support {declared}{legacy}",
                flag.bit, flag.name,
            );
            if flag.locked && flag.iodd_supported == Some(false) {
                text.push_str(
                    "\nReview · lock flag is set for a capability the IODD marks unsupported",
                );
            }
        }
        if let Some(mask) = self.reserved_mask.filter(|mask| *mask != 0) {
            let _ = write!(text, "\nReview · reserved bits are set (0x{mask:04X})");
        }
        text
    }
}

pub fn decode_device_access_locks(
    profile: Option<&Profile>,
    hex: &str,
) -> Result<DeviceAccessLocks> {
    let bytes = if hex.trim().is_empty() {
        Vec::new()
    } else {
        decode::parse_hex_bounded(hex, 232)?
    };
    let mut report = DeviceAccessLocks {
        raw_hex: decode::hex(&bytes),
        reserved_mask: None,
        flags: Vec::new(),
        decode_error: None,
    };
    let Ok(bytes) = <[u8; 2]>::try_from(bytes.as_slice()) else {
        report.decode_error = Some("DeviceAccessLocks (12:0) requires exactly two bytes");
        return Ok(report);
    };
    let word = u16::from_be_bytes(bytes);
    let supported = profile
        .and_then(|profile| profile.iodd.as_ref())
        .and_then(|iodd| iodd.features.as_ref())
        .and_then(|features| features.access_locks.as_ref())
        .map(|locks| {
            [
                locks.parameter,
                locks.data_storage,
                locks.local_parameterization,
                locks.local_user_interface,
            ]
        });
    report.reserved_mask = Some(word & 0xFFF0);
    report.flags = NAMES
        .into_iter()
        .enumerate()
        .map(|(bit, name)| AccessLockFlag {
            bit,
            name,
            locked: word & (1 << bit) != 0,
            iodd_supported: supported.map(|declared| declared[bit]),
            legacy: bit < 2,
        })
        .collect();
    Ok(report)
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccessLocksSnapshot {
    pub format_version: u32,
    pub workbench_version: String,
    pub profile: Profile,
    pub read: SavedRead,
}

#[derive(Debug, Serialize)]
pub struct AccessLocksReview {
    pub workbench_version: String,
    pub decoder_version: &'static str,
    pub profile_name: String,
    pub read: SavedRead,
    pub decoded: Option<DeviceAccessLocks>,
}

impl AccessLocksReview {
    pub fn ok(&self) -> bool {
        self.decoded.as_ref().is_some_and(DeviceAccessLocks::ok)
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "Device access lock read · {}\nProducer {} · decoder {}\nHost {} · query {} µs",
            self.profile_name,
            self.workbench_version,
            self.decoder_version,
            self.read.host_read_utc,
            self.read.read_duration_us,
        );
        if let Some(observed) = &self.read.observation {
            let _ = write!(
                text,
                "\n{} · {} firmware {} · port {} · {}\nMaster serial {:?} · Vendor {:?} · Device {:?} · Sensor serial {:?}",
                observed.route,
                observed.model,
                observed.firmware,
                observed.port,
                observed.port_mode,
                observed.master_serial,
                observed.identity.vendor_id,
                observed.identity.device_id,
                observed.identity.serial,
            );
        }
        if let Some(error) = &self.read.read_error {
            let _ = write!(text, "\nRead error · {error}");
        }
        if let Some(decoded) = &self.decoded {
            let _ = write!(text, "\n\n{}", decoded.text());
        }
        text
    }
}

impl AccessLocksSnapshot {
    pub fn read(profile: &Profile) -> Result<Self> {
        profile.validate()?;
        transport::check_route(profile)?;
        Ok(Self {
            format_version: 1,
            workbench_version: env!("CARGO_PKG_VERSION").into(),
            profile: profile.clone(),
            read: SavedRead::new(profile.connection.gateway, &observe(profile, INDEX)),
        })
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.format_version == 1,
            "unsupported access locks snapshot version"
        );
        ensure!(
            !self.workbench_version.trim().is_empty(),
            "snapshot producer version is empty"
        );
        self.profile.validate()?;
        self.read
            .checked_bytes(&self.profile, INDEX)
            .context("access locks snapshot 12:0")?;
        Ok(())
    }

    pub fn review(&self) -> Result<AccessLocksReview> {
        self.validate()?;
        let decoded = self
            .read
            .raw_hex
            .as_deref()
            .map(|hex| decode_device_access_locks(Some(&self.profile), hex))
            .transpose()?;
        Ok(AccessLocksReview {
            workbench_version: self.workbench_version.clone(),
            decoder_version: env!("CARGO_PKG_VERSION"),
            profile_name: self.profile.name.clone(),
            read: self.read.clone(),
            decoded,
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let snapshot: Self = snapshot_file::load(path, SNAPSHOT_KIND)?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        snapshot_file::save(self, path, SNAPSHOT_KIND)
    }
}

#[derive(Debug, Serialize)]
pub struct AccessLocksComparison {
    pub left: AccessLocksReview,
    pub right: AccessLocksReview,
    pub reply: ReplyComparison,
    pub xor_mask: Option<u16>,
    pub flag_changes: Option<Vec<AccessLockChange>>,
}

#[derive(Debug, Serialize)]
pub struct AccessLockChange {
    pub bit: usize,
    pub name: &'static str,
    pub left_locked: bool,
    pub right_locked: bool,
    pub legacy: bool,
}

impl AccessLocksComparison {
    pub fn ok(&self) -> bool {
        self.flag_changes.is_some()
    }

    pub fn summary_text(&self) -> String {
        let mut text = format!(
            "Saved access locks comparison · left → right\nProfiles · {} → {}",
            self.left.profile_name, self.right.profile_name,
        );
        self.reply.write_summary(&mut text, INDEX);
        if let Some(mask) = self.xor_mask {
            let _ = write!(text, "\nWire XOR · 0x{mask:04X}");
        }
        if let Some(changes) = &self.flag_changes {
            if changes.is_empty() {
                text.push_str("\nRecorded lock flags match");
            }
            for change in changes {
                let state = |locked| if locked { "Set" } else { "Clear" };
                let legacy = if change.legacy { " · Legacy" } else { "" };
                let _ = write!(
                    text,
                    "\nBit {} · {} · {} → {}{legacy}",
                    change.bit,
                    change.name,
                    state(change.left_locked),
                    state(change.right_locked),
                );
            }
        }
        text
    }
}

pub fn compare_access_locks(
    left: &AccessLocksSnapshot,
    right: &AccessLocksSnapshot,
) -> Result<AccessLocksComparison> {
    let endpoint_issue = (left.profile.connection.endpoint != right.profile.connection.endpoint)
        .then_some("Saved gateway endpoints differ");
    let left = left.review()?;
    let right = right.review()?;
    let mut reply = ReplyComparison::new(&left.read, &right.read, endpoint_issue, None);
    let decoded = left.decoded.as_ref().zip(right.decoded.as_ref());
    let xor_mask = decoded
        .filter(|(a, b)| a.decode_error.is_none() && b.decode_error.is_none())
        .and_then(|(a, b)| {
            let a = u16::from_str_radix(&a.raw_hex, 16).ok()?;
            let b = u16::from_str_radix(&b.raw_hex, 16).ok()?;
            Some(a ^ b)
        });
    let flags = decoded
        .filter(|(a, b)| a.ok() && b.ok())
        .map(|(a, b)| a.flags.iter().zip(&b.flags));
    let flag_issue = match &flags {
        None => Some("One or both saved lock replies need decoding review"),
        Some(flags)
            if flags
                .clone()
                .any(|(a, b)| a.iodd_supported != b.iodd_supported) =>
        {
            Some("Saved IODD lock support declarations differ")
        }
        Some(_) => None,
    };
    reply.interpretation_issue = reply.interpretation_issue.or(flag_issue);
    let flag_changes = flags
        .filter(|_| reply.interpretation_issue.is_none())
        .map(|flags| {
            flags
                .filter(|(a, b)| a.locked != b.locked)
                .map(|(a, b)| AccessLockChange {
                    bit: a.bit,
                    name: a.name,
                    left_locked: a.locked,
                    right_locked: b.locked,
                    legacy: a.legacy,
                })
                .collect()
        });
    Ok(AccessLocksComparison {
        left,
        right,
        reply,
        xor_mask,
        flag_changes,
    })
}
