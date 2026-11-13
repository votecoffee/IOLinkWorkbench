//! One explicit acyclic observation, saved with its decoding context. No cyclic data or outputs.
use crate::{
    decode,
    iodd::DecodedParameter,
    profile::{Gateway, Identity, Profile},
    transport::{self, ParameterReading, Status},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub route: String,
    pub gateway: Gateway,
    pub model: String,
    pub firmware: String,
    pub master_serial: Option<String>,
    pub port: u8,
    pub port_mode: String,
    pub identity: Identity,
    // Connection-time status precedes the explicit parameter query.
    pub device_status: Option<String>,
    pub port_event: Option<String>,
    pub device_event: Option<String>,
    pub port_cycle_raw: Option<String>,
    pub port_cycle_us: Option<u64>,
}

impl Observation {
    fn new(gateway: Gateway, status: Status) -> Self {
        Self {
            gateway,
            route: status.route.into(),
            model: status.model,
            firmware: status.firmware,
            master_serial: status.master_serial,
            port: status.port,
            port_mode: status.port_mode,
            identity: status.identity,
            device_status: status.device_status,
            port_event: status.port_event,
            device_event: status.device_event,
            port_cycle_raw: status.port_cycle_raw,
            port_cycle_us: status.port_cycle_us,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub format_version: u32,
    pub workbench_version: String,
    pub profile: Profile,
    pub parameter_id: String,
    pub index: u16,
    pub subindex: u8,
    /// Host UTC at the end of the read; no device timestamp is implied.
    pub host_read_utc: String,
    /// Monotonic duration including connect, identity checks and the acyclic query.
    pub read_duration_us: u64,
    pub observation: Observation,
    pub raw_hex: String,
}

#[derive(Debug, Serialize)]
pub struct Review {
    pub workbench_version: String,
    pub parameter_id: String,
    pub name: String,
    pub index: u16,
    pub subindex: u8,
    pub host_read_utc: String,
    pub read_duration_us: u64,
    pub identity_check: String,
    pub observation: Observation,
    pub decoded: DecodedParameter,
}

impl Review {
    pub fn text(&self) -> String {
        format!(
            "Saved parameter snapshot · {}\n{} · index {}:{}\nMaster {} · firmware {} · port {} · {}\nIdentity {} · vendor {:?} · device {:?} · serial {:?}\nHost read {} · total operation {} µs\n{}",
            self.observation.route,
            self.name,
            self.index,
            self.subindex,
            self.observation.model,
            self.observation.firmware,
            self.observation.port,
            self.observation.port_mode,
            self.identity_check,
            self.observation.identity.vendor_id,
            self.observation.identity.device_id,
            self.observation.identity.serial,
            self.host_read_utc,
            self.read_duration_us,
            self.decoded.text()
        )
    }
}

/// Exactly one named parameter read on the existing independent acyclic client.
pub fn read(profile: &Profile, id: &str, subindex: u8) -> Result<(Snapshot, ParameterReading)> {
    let (reading, status) = transport::read_parameter_observed(profile, id, subindex)?;
    let snapshot = Snapshot {
        format_version: 1,
        workbench_version: env!("CARGO_PKG_VERSION").into(),
        profile: profile.clone(),
        parameter_id: id.into(),
        index: reading.index,
        subindex,
        host_read_utc: reading.host_read_utc.clone(),
        read_duration_us: reading.read_duration_us,
        observation: Observation::new(profile.connection.gateway, status),
        raw_hex: reading.raw_hex.clone(),
    };
    snapshot.validate()?;
    Ok((snapshot, reading))
}

/// Preflight without any network access. Final creation still uses an atomic no-clobber save.
pub fn check_destination(path: &Path) -> Result<()> {
    ensure!(
        !path.try_exists()?,
        "snapshot destination already exists: {}",
        path.display()
    );
    ensure!(
        path.file_name().is_some(),
        "snapshot destination needs a file name"
    );
    let parent = parent(path);
    ensure!(
        parent.is_dir(),
        "snapshot parent folder does not exist: {}",
        parent.display()
    );
    Ok(())
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

impl Snapshot {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.format_version == 1,
            "unsupported parameter snapshot version"
        );
        ensure!(
            !self.workbench_version.trim().is_empty(),
            "snapshot producer version is empty"
        );
        self.profile.validate()?;
        let parameter =
            transport::parameter_target(&self.profile, &self.parameter_id, self.subindex)?;
        ensure!(
            parameter.index == self.index,
            "snapshot query index differs from the saved IODD"
        );
        ensure!(
            self.observation.gateway == self.profile.connection.gateway
                && self.observation.port == self.profile.connection.port,
            "snapshot gateway or port differs from the saved profile"
        );
        ensure!(
            !self.observation.route.trim().is_empty()
                && !self.observation.model.trim().is_empty()
                && !self.observation.firmware.trim().is_empty(),
            "snapshot master context is incomplete"
        );
        self.profile
            .expected_identity
            .verify(&self.observation.identity)?;
        let utc = chrono::DateTime::parse_from_rfc3339(&self.host_read_utc)
            .context("invalid snapshot host UTC")?;
        ensure!(
            utc.offset().local_minus_utc() == 0,
            "snapshot host read time must be UTC"
        );
        let bytes = decode::parse_hex_bounded(&self.raw_hex, 232)?;
        ensure!(
            !bytes.is_empty() && decode::hex(&bytes) == self.raw_hex,
            "snapshot raw bytes must be canonical uppercase hex"
        );
        Ok(())
    }

    /// Decode only with the embedded profile. Stored file paths and endpoint are never opened.
    pub fn review(&self) -> Result<Review> {
        self.validate()?;
        let parent = self.profile.parameter(&self.parameter_id)?;
        let target = parent.target(self.subindex)?;
        Ok(Review {
            workbench_version: self.workbench_version.clone(),
            parameter_id: self.parameter_id.clone(),
            name: if parent.array.is_some() && self.subindex > 0 {
                format!("{} [{}]", parent.name, self.subindex)
            } else {
                target.name.clone()
            },
            index: self.index,
            subindex: self.subindex,
            host_read_utc: self.host_read_utc.clone(),
            read_duration_us: self.read_duration_us,
            identity_check: self
                .profile
                .expected_identity
                .verify(&self.observation.identity)?
                .into(),
            observation: self.observation.clone(),
            decoded: target.decode(&decode::parse_hex_bounded(&self.raw_hex, 232)?),
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_BYTES,
            "parameter snapshot exceeds 16 MiB"
        );
        let snapshot: Self =
            serde_json::from_slice(&bytes).context("invalid parameter snapshot JSON")?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    /// Publish a complete file atomically. Existing destinations are never replaced.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        check_destination(path)?;
        let mut bytes = serde_json::to_vec_pretty(self)?;
        bytes.push(b'\n');
        ensure!(
            bytes.len() <= MAX_BYTES,
            "parameter snapshot exceeds 16 MiB"
        );
        let mut temporary = tempfile::NamedTempFile::new_in(parent(path))?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(path)
            .with_context(|| format!("save snapshot {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
