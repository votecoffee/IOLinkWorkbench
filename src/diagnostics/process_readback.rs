//! Explicit reads of the device's optional last-valid process data.

use super::{SavedRead, observe};
use crate::{
    decode::{self, Field, MAX_INPUT, Value},
    iodd::DecodedOutput,
    profile::Profile,
    snapshot_file, transport,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt::Write, path::Path};

pub const INPUT_READBACK_INDEX: u16 = 40;
pub const OUTPUT_READBACK_INDEX: u16 = 41;
const SNAPSHOT_KIND: &str = "process readback snapshot";
const BASIS: &str =
    "Device's last valid process data · sample time and current validity unavailable";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadbackDirection {
    #[default]
    Input,
    Output,
}

impl ReadbackDirection {
    pub const ALL: [Self; 2] = [Self::Input, Self::Output];

    pub fn index(self) -> u16 {
        match self {
            Self::Input => INPUT_READBACK_INDEX,
            Self::Output => OUTPUT_READBACK_INDEX,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Input => "Input (40:0)",
            Self::Output => "Output (41:0)",
        }
    }
}

impl TryFrom<u16> for ReadbackDirection {
    type Error = anyhow::Error;

    fn try_from(index: u16) -> Result<Self> {
        match index {
            INPUT_READBACK_INDEX => Ok(Self::Input),
            OUTPUT_READBACK_INDEX => Ok(Self::Output),
            _ => bail!("process readback requires index 40 (input) or 41 (output)"),
        }
    }
}

#[derive(Serialize)]
pub struct ReadbackInputField {
    pub origin: &'static str,
    pub field: Field,
    pub value: Value,
}

#[derive(Serialize)]
pub struct ProcessReadback {
    pub direction: ReadbackDirection,
    pub index: u16,
    pub observation_kind: &'static str,
    pub raw_hex: String,
    pub payload_bytes: usize,
    pub decode_error: Option<String>,
    pub mapping_note: Option<&'static str>,
    pub input_encoding_warning: Option<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub input_fields: Vec<ReadbackInputField>,
    pub iodd_output: Option<DecodedOutput>,
}

impl ProcessReadback {
    pub fn ok(&self) -> bool {
        self.decode_error.is_none()
            && self
                .input_fields
                .iter()
                .all(|item| item.value.encoding_error.is_none())
            && self.iodd_output.as_ref().is_none_or(|output| {
                output.decode_error.is_none()
                    && output
                        .fields
                        .iter()
                        .all(|item| item.value.encoding_error.is_none())
            })
    }

    pub fn text(&self) -> String {
        let raw = if self.raw_hex.is_empty() {
            "Empty"
        } else {
            &self.raw_hex
        };
        let mut text = format!(
            "Acyclic process readback · {}\n{}\nRaw reply · {raw}",
            self.direction.label(),
            self.observation_kind,
        );
        if let Some(error) = &self.decode_error {
            let _ = write!(text, "\nReview · {error}");
        }
        if let Some(note) = self.mapping_note {
            let _ = write!(text, "\n{note}");
        }
        if let Some(warning) = self.input_encoding_warning {
            let _ = write!(text, "\n{warning}");
        }
        for item in &self.input_fields {
            write_field(&mut text, &item.field, item.origin, &item.value);
        }
        if let Some(output) = &self.iodd_output {
            let _ = write!(
                text,
                "\nIODD output · {} · {} bits",
                output.layout_id, output.bit_length
            );
            if let Some(error) = &output.decode_error {
                let _ = write!(text, "\nReview · {error}");
            }
            if let Some(warning) = &output.encoding_warning {
                let _ = write!(text, "\n{warning}");
            }
            for item in &output.fields {
                write_field(&mut text, &item.field, "IODD", &item.value);
            }
        }
        text
    }
}

fn write_field(text: &mut String, field: &Field, origin: &str, value: &Value) {
    let display = format!("{} {}", value.display(), field.unit);
    let _ = write!(
        text,
        "\n{} [{origin}] · {} (raw {}, bits 0x{})",
        field.name,
        display.trim_end(),
        value.raw,
        value.bits_hex,
    );
    if let Some(error) = &value.encoding_error {
        let _ = write!(text, "\nReview · {error}");
    }
}

fn decode_bytes(
    direction: ReadbackDirection,
    profile: Option<&Profile>,
    bytes: &[u8],
) -> ProcessReadback {
    let mut report = ProcessReadback {
        direction,
        index: direction.index(),
        observation_kind: BASIS,
        raw_hex: decode::hex(bytes),
        payload_bytes: bytes.len(),
        decode_error: None,
        mapping_note: None,
        input_encoding_warning: None,
        input_fields: Vec::new(),
        iodd_output: None,
    };
    if !(1..=MAX_INPUT).contains(&bytes.len()) {
        report.decode_error = Some(format!("Process readback requires 1–{MAX_INPUT} bytes"));
        return report;
    }
    let Some(profile) = profile else {
        report.mapping_note = Some("Raw reply · select a saved mapping to interpret the payload");
        return report;
    };
    if profile
        .iodd
        .as_ref()
        .is_some_and(|iodd| iodd.condition.is_some())
        && !profile.confirm_iodd_condition
    {
        report.mapping_note = Some("Raw reply · selected IODD layout condition is unconfirmed");
        return report;
    }
    match direction {
        ReadbackDirection::Input if profile.has_process_input() => {
            report.input_encoding_warning = profile.input_encoding_warning(bytes);
            match profile.decode(bytes) {
                Ok(fields) if fields.is_empty() => {
                    report.mapping_note =
                        Some("Raw reply · the profile has no mapped input fields");
                }
                Ok(fields) => {
                    report.input_fields = fields
                        .into_iter()
                        .map(|(field, origin, value)| ReadbackInputField {
                            origin,
                            field: field.clone(),
                            value,
                        })
                        .collect();
                }
                Err(error) => report.decode_error = Some(error.to_string()),
            }
        }
        ReadbackDirection::Input => {
            report.mapping_note = Some("Raw reply · the profile has no input layout");
        }
        ReadbackDirection::Output => {
            if let Some(layout) = profile.output_layout() {
                report.iodd_output =
                    Some(layout.decode_output(bytes, Some(profile.confirm_iodd_condition)));
            } else {
                report.mapping_note = Some("Raw reply · the profile has no IODD output layout");
            }
        }
    }
    report
}

pub fn decode_process_readback(
    direction: ReadbackDirection,
    profile: Option<&Profile>,
    hex: &str,
) -> Result<ProcessReadback> {
    let bytes = if hex.trim().is_empty() {
        Vec::new()
    } else {
        decode::parse_hex_bounded(hex, 232)?
    };
    Ok(decode_bytes(direction, profile, &bytes))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessReadbackSnapshot {
    pub format_version: u32,
    pub workbench_version: String,
    pub profile: Profile,
    pub direction: ReadbackDirection,
    pub read: SavedRead,
}

#[derive(Serialize)]
pub struct ProcessReadbackReview<'a> {
    pub direction: ReadbackDirection,
    pub observation_kind: &'static str,
    pub workbench_version: &'a str,
    pub decoder_version: &'static str,
    pub profile_name: &'a str,
    pub read: &'a SavedRead,
    pub decoded: Option<ProcessReadback>,
}

impl ProcessReadbackReview<'_> {
    pub fn ok(&self) -> bool {
        self.decoded.as_ref().is_some_and(ProcessReadback::ok)
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "Process readback · {} · {}\nProducer {} · decoder {}\nHost read completed {} · query {} µs",
            self.direction.label(),
            self.profile_name,
            self.workbench_version,
            self.decoder_version,
            self.read.host_read_utc,
            self.read.read_duration_us,
        );
        if let Some(observed) = &self.read.observation {
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

impl ProcessReadbackSnapshot {
    pub fn read(profile: &Profile, direction: ReadbackDirection) -> Result<Self> {
        profile.validate()?;
        transport::check_route(profile)?;
        Ok(Self {
            format_version: 1,
            workbench_version: env!("CARGO_PKG_VERSION").into(),
            profile: profile.clone(),
            direction,
            read: SavedRead::new(
                profile.connection.gateway,
                &observe(profile, direction.index()),
            ),
        })
    }

    fn checked_bytes(&self) -> Result<Option<Vec<u8>>> {
        ensure!(
            self.format_version == 1,
            "unsupported process readback snapshot version"
        );
        ensure!(
            !self.workbench_version.trim().is_empty(),
            "snapshot producer version is empty"
        );
        self.profile.validate()?;
        self.read
            .checked_bytes(&self.profile, self.direction.index())
            .context("process readback snapshot")
    }

    pub fn review(&self) -> Result<ProcessReadbackReview<'_>> {
        self.review_with_bytes().map(|(review, _)| review)
    }

    pub(super) fn review_with_bytes(&self) -> Result<(ProcessReadbackReview<'_>, Option<Vec<u8>>)> {
        let bytes = self.checked_bytes()?;
        let review = ProcessReadbackReview {
            direction: self.direction,
            observation_kind: BASIS,
            workbench_version: &self.workbench_version,
            decoder_version: env!("CARGO_PKG_VERSION"),
            profile_name: &self.profile.name,
            read: &self.read,
            decoded: bytes
                .as_deref()
                .map(|bytes| decode_bytes(self.direction, Some(&self.profile), bytes)),
        };
        Ok((review, bytes))
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
