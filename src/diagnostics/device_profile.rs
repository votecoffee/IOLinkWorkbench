//! One explicit set of Common Profile reads with per-query connection evidence.
use super::{
    DescriptorDirection, DescriptorStatus, ProcessDescriptor, ProfileCharacteristic,
    ProfileCharacteristicStatus, SavedRead, decode_process_descriptor,
    decode_profile_characteristic, observe,
};
use crate::{profile::Profile, snapshot_file, transport};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt::Write, path::Path};

const INDEXES: [u16; 3] = [13, 14, 15];
const SNAPSHOT_KIND: &str = "device profile snapshot";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceProfileSnapshot {
    pub format_version: u32,
    pub workbench_version: String,
    pub profile: Profile,
    pub reads: [SavedRead; 3],
}

#[derive(Debug, Serialize)]
pub struct DeviceProfileReview {
    pub workbench_version: String,
    pub decoder_version: &'static str,
    pub profile_name: String,
    pub observed_identity_conflict: bool,
    pub observed_master_conflict: bool,
    pub profile_characteristic: Option<ProfileCharacteristic>,
    pub input_descriptor: Option<ProcessDescriptor>,
    pub output_descriptor: Option<ProcessDescriptor>,
    pub reads: [SavedRead; 3],
}

impl DeviceProfileReview {
    pub fn ok(&self) -> bool {
        !self.observed_identity_conflict
            && !self.observed_master_conflict
            && self.profile_characteristic.as_ref().is_some_and(|report| {
                matches!(
                    report.status,
                    ProfileCharacteristicStatus::Match | ProfileCharacteristicStatus::Undeclared
                )
            })
            && [&self.input_descriptor, &self.output_descriptor]
                .into_iter()
                .all(|report| {
                    report.as_ref().is_some_and(|report| {
                        matches!(
                            report.status,
                            DescriptorStatus::Match | DescriptorStatus::NoIodd
                        )
                    })
                })
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "Device profile and layouts · {}\nProducer {} · decoder {}",
            self.profile_name, self.workbench_version, self.decoder_version,
        );
        if self.observed_identity_conflict {
            text.push_str(
                "\nReview · device identity fields differ between reads. IODD comparison withheld.",
            );
        }
        if self.observed_master_conflict {
            text.push_str("\nReview · master or port context differs between reads. IODD comparison withheld.");
        }
        let decoded = [
            self.profile_characteristic
                .as_ref()
                .map(ProfileCharacteristic::text),
            self.input_descriptor.as_ref().map(ProcessDescriptor::text),
            self.output_descriptor.as_ref().map(ProcessDescriptor::text),
        ];
        for (read, decoded) in self.reads.iter().zip(decoded) {
            let _ = write!(
                text,
                "\n\n{}:{} · host {} · query {} µs",
                read.index, read.subindex, read.host_read_utc, read.read_duration_us
            );
            if let Some(observed) = &read.observation {
                let _ = write!(
                    text,
                    "\n{} · {} firmware {} · master serial {:?} · port {} · {}\nVendor {:?} · Device {:?} · sensor serial {:?}",
                    observed.route,
                    observed.model,
                    observed.firmware,
                    observed.master_serial,
                    observed.port,
                    observed.port_mode,
                    observed.identity.vendor_id,
                    observed.identity.device_id,
                    observed.identity.serial,
                );
            }
            if let Some(error) = &read.read_error {
                let _ = write!(text, "\nRead error · {error}");
            }
            if let Some(decoded) = decoded {
                let _ = write!(text, "\n{decoded}");
            }
        }
        text
    }
}

impl DeviceProfileSnapshot {
    pub fn read(profile: &Profile) -> Result<Self> {
        profile.validate()?;
        transport::check_route(profile)?;
        Ok(Self {
            format_version: 1,
            workbench_version: env!("CARGO_PKG_VERSION").into(),
            profile: profile.clone(),
            reads: INDEXES
                .map(|index| SavedRead::new(profile.connection.gateway, &observe(profile, index))),
        })
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.format_version == 1,
            "unsupported device profile snapshot version"
        );
        ensure!(
            !self.workbench_version.trim().is_empty(),
            "snapshot producer version is empty"
        );
        self.profile.validate()?;
        for (read, index) in self.reads.iter().zip(INDEXES) {
            read.checked_bytes(&self.profile, index)
                .with_context(|| format!("device profile snapshot {index}:0"))?;
        }
        Ok(())
    }

    pub fn review(&self) -> Result<DeviceProfileReview> {
        self.validate()?;
        let observations: Vec<_> = self
            .reads
            .iter()
            .filter_map(|read| read.observation.as_ref())
            .collect();
        let mut observed_identity_conflict = false;
        let mut observed_master_conflict = false;
        for (left_index, left) in observations.iter().enumerate() {
            for right in &observations[left_index + 1..] {
                observed_identity_conflict |= left.identity != right.identity;
                observed_master_conflict |= left.route != right.route
                    || left.model != right.model
                    || left.firmware != right.firmware
                    || left.port_mode != right.port_mode
                    || left
                        .master_serial
                        .as_ref()
                        .zip(right.master_serial.as_ref())
                        .is_some_and(|(left, right)| left != right);
            }
        }
        let compare =
            (!observed_identity_conflict && !observed_master_conflict).then_some(&self.profile);
        let [characteristic, input, output] =
            self.reads.each_ref().map(|read| read.raw_hex.as_deref());
        Ok(DeviceProfileReview {
            workbench_version: self.workbench_version.clone(),
            decoder_version: env!("CARGO_PKG_VERSION"),
            profile_name: self.profile.name.clone(),
            observed_identity_conflict,
            observed_master_conflict,
            profile_characteristic: characteristic
                .map(|hex| decode_profile_characteristic(compare, hex))
                .transpose()?,
            input_descriptor: input
                .map(|hex| decode_process_descriptor(DescriptorDirection::Input, compare, hex))
                .transpose()?,
            output_descriptor: output
                .map(|hex| decode_process_descriptor(DescriptorDirection::Output, compare, hex))
                .transpose()?,
            reads: self.reads.clone(),
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
