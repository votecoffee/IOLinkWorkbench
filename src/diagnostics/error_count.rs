//! Read and review the optional device application ErrorCount at 32:0.

use super::{SavedRead, observe};
use crate::{decode, profile::Profile, snapshot_file, transport};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt::Write, path::Path};

pub const ERROR_COUNT_INDEX: u16 = 32;
const SNAPSHOT_KIND: &str = "device error count snapshot";
const BASIS: &str = "Device application errors since power-on or reset, as defined by the vendor";

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
