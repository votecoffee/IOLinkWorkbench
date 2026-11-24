//! Read and review the optional device application ErrorCount at 32:0.

use super::{ReplyComparison, SavedRead, observe};
use crate::{decode, profile::Profile, snapshot_file, transport};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt::Write, path::Path};

pub const ERROR_COUNT_INDEX: u16 = 32;
const SNAPSHOT_KIND: &str = "device error count snapshot";
const BASIS: &str = "Device application errors since power-on or reset, as defined by the vendor";
const INTERVAL_BASIS: &str = "The number of errors between reads is unknown";

#[derive(Debug, Serialize)]
pub struct DeviceErrorCount {
    pub raw_hex: String,
    pub value: Option<u16>,
    pub decode_error: Option<&'static str>,
    pub meaning: &'static str,
}

impl DeviceErrorCount {
    pub fn ok(&self) -> bool {
        self.value.is_some()
    }

    pub fn text(&self) -> String {
        let raw = if self.raw_hex.is_empty() {
            "Empty"
        } else {
            &self.raw_hex
        };
        let mut text = format!("Device error count · 32:0\nRaw reply · {raw}");
        if let Some(value) = self.value {
            let _ = write!(text, "\nCount · {value}\n{}", self.meaning);
        }
        if let Some(error) = self.decode_error {
            let _ = write!(text, "\nReview · {error}");
        }
        text
    }
}

fn decode_bytes(bytes: &[u8]) -> DeviceErrorCount {
    let value = <[u8; 2]>::try_from(bytes).ok().map(u16::from_be_bytes);
    DeviceErrorCount {
        raw_hex: decode::hex(bytes),
        value,
        decode_error: value
            .is_none()
            .then_some("ErrorCount (32:0) requires exactly two bytes"),
        meaning: BASIS,
    }
}

pub fn decode_error_count(hex: &str) -> Result<DeviceErrorCount> {
    let bytes = if hex.trim().is_empty() {
        Vec::new()
    } else {
        decode::parse_hex_bounded(hex, 232)?
    };
    Ok(decode_bytes(&bytes))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorCountSnapshot {
    pub format_version: u32,
    pub workbench_version: String,
    pub profile: Profile,
    pub read: SavedRead,
}

#[derive(Debug, Serialize)]
pub struct ErrorCountReview<'a> {
    pub workbench_version: &'a str,
    pub decoder_version: &'static str,
    pub profile_name: &'a str,
    pub read: &'a SavedRead,
    pub decoded: Option<DeviceErrorCount>,
}

impl ErrorCountReview<'_> {
    pub fn ok(&self) -> bool {
        self.decoded.as_ref().is_some_and(DeviceErrorCount::ok)
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "Device error count read · {}\nProducer {} · decoder {}\nHost {} · query {} µs",
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

impl ErrorCountSnapshot {
    pub fn read(profile: &Profile) -> Result<Self> {
        profile.validate()?;
        transport::check_route(profile)?;
        Ok(Self {
            format_version: 1,
            workbench_version: env!("CARGO_PKG_VERSION").into(),
            profile: profile.clone(),
            read: SavedRead::new(
                profile.connection.gateway,
                &observe(profile, ERROR_COUNT_INDEX),
            ),
        })
    }

    fn checked_bytes(&self) -> Result<Option<Vec<u8>>> {
        ensure!(
            self.format_version == 1,
            "unsupported error count snapshot version"
        );
        ensure!(
            !self.workbench_version.trim().is_empty(),
            "snapshot producer version is empty"
        );
        self.profile.validate()?;
        self.read
            .checked_bytes(&self.profile, ERROR_COUNT_INDEX)
            .context("error count snapshot 32:0")
    }

    pub fn review(&self) -> Result<ErrorCountReview<'_>> {
        Ok(ErrorCountReview {
            workbench_version: &self.workbench_version,
            decoder_version: env!("CARGO_PKG_VERSION"),
            profile_name: &self.profile.name,
            read: &self.read,
            decoded: self.checked_bytes()?.as_deref().map(decode_bytes),
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

#[derive(Debug, Serialize)]
pub struct ErrorCountComparison<'a> {
    pub left: ErrorCountReview<'a>,
    pub right: ErrorCountReview<'a>,
    pub reply: ReplyComparison,
    pub recorded_counts: Option<RecordedErrorCounts>,
    pub interval_basis: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RecordedErrorCounts {
    pub left: u16,
    pub right: u16,
    pub numeric_difference: i32,
}

impl ErrorCountComparison<'_> {
    pub fn ok(&self) -> bool {
        self.recorded_counts.is_some()
    }

    pub fn summary_text(&self) -> String {
        let mut text = format!(
            "Saved device error count comparison · left → right\nProfiles · {} → {}",
            self.left.profile_name, self.right.profile_name
        );
        self.reply.write_summary(&mut text, ERROR_COUNT_INDEX);
        if let Some(counts) = &self.recorded_counts {
            let _ = write!(
                text,
                "\nRecorded count · {} → {}\nRight minus left · {}",
                counts.left, counts.right, counts.numeric_difference
            );
        }
        let _ = write!(text, "\n{}", self.interval_basis);
        text
    }
}

pub fn compare_error_counts<'a>(
    left: &'a ErrorCountSnapshot,
    right: &'a ErrorCountSnapshot,
) -> Result<ErrorCountComparison<'a>> {
    let endpoint_issue = (left.profile.connection.endpoint != right.profile.connection.endpoint)
        .then_some("Saved gateway endpoints differ");
    let left = left.review()?;
    let right = right.review()?;
    let values = left
        .decoded
        .as_ref()
        .and_then(|decoded| decoded.value)
        .zip(right.decoded.as_ref().and_then(|decoded| decoded.value));
    let reply = ReplyComparison::new(
        left.read,
        right.read,
        endpoint_issue,
        values
            .is_none()
            .then_some("One or both saved replies need length review"),
    );
    let recorded_counts =
        values
            .filter(|_| reply.interpretation_issue.is_none())
            .map(|(left, right)| RecordedErrorCounts {
                left,
                right,
                numeric_difference: i32::from(right) - i32::from(left),
            });
    Ok(ErrorCountComparison {
        left,
        right,
        reply,
        recorded_counts,
        interval_basis: INTERVAL_BASIS,
    })
}
