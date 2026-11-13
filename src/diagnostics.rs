use crate::{
    decode,
    iodd::{EventDefinition, EventType},
    profile::Profile,
    transport::{self, Status},
};
use anyhow::Result;
use serde::Serialize;
use std::time::Instant;

#[derive(Debug, Serialize)]
pub struct DeviceStatus {
    pub raw_hex: String,
    pub value: Option<u8>,
    pub meaning: Option<&'static str>,
    pub decode_error: Option<String>,
}

pub fn decode_device_status(bytes: &[u8]) -> DeviceStatus {
    let value = (bytes.len() == 1).then(|| bytes[0]);
    DeviceStatus {
        raw_hex: decode::hex(bytes),
        value,
        meaning: value.map(|value| match value {
            0 => "Operating properly",
            1 => "Maintenance required",
            2 => "Out of specification",
            3 => "Functional check",
            4 => "Failure",
            _ => "Reserved device-status value",
        }),
        decode_error: (bytes.len() != 1)
            .then(|| "Device Status (36:0) requires exactly one byte".into()),
    }
}

#[derive(Debug, Serialize)]
pub struct Event {
    pub slot: usize,
    pub raw_hex: String,
    pub qualifier: u8,
    pub code: u16,
    pub code_hex: String,
    pub instance: u8,
    pub instance_name: &'static str,
    pub source: u8,
    pub source_name: &'static str,
    pub event_type: u8,
    pub type_name: &'static str,
    pub mode: u8,
    pub mode_name: &'static str,
    pub pending_shape: bool,
    pub definition: Option<EventDefinition>,
    pub review: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct DetailedStatus {
    pub raw_hex: String,
    pub empty_slots: usize,
    pub events: Vec<Event>,
    pub decode_error: Option<String>,
}

/// Standard DetailedDeviceStatus, index 37:0. Each slot is qualifier + BE event code.
pub fn decode_detailed(profile: &Profile, bytes: &[u8]) -> DetailedStatus {
    let mut result = DetailedStatus {
        raw_hex: decode::hex(bytes),
        empty_slots: 0,
        events: Vec::new(),
        decode_error: None,
    };
    if bytes.is_empty() || bytes.len() > 192 || !bytes.len().is_multiple_of(3) {
        result.decode_error =
            Some("Detailed Device Status (37:0) requires 1–64 complete three-byte slots".into());
        return result;
    }
    for (index, entry) in bytes.as_chunks::<3>().0.iter().enumerate() {
        if *entry == [0, 0, 0] {
            result.empty_slots += 1;
            continue;
        }
        let qualifier = entry[0];
        let code = u16::from_be_bytes([entry[1], entry[2]]);
        let instance = qualifier & 7;
        let source = (qualifier >> 3) & 1;
        let event_type = (qualifier >> 4) & 3;
        let mode = qualifier >> 6;
        let application = source == 0 && instance == 4;
        let definition = application
            .then(|| {
                profile
                    .iodd
                    .as_ref()
                    .and_then(|iodd| iodd.events.iter().find(|event| event.code == code))
            })
            .flatten()
            .cloned();
        let pending_shape = application && mode == 3 && matches!(event_type, 2 | 3) && code != 0;
        let mut review = Vec::new();
        if !pending_shape {
            review.push(
                "Entry does not match a pending device-application warning/error with mode appears"
                    .into(),
            );
        }
        if !application {
            review.push("Device IODD event names are not applied to this source/instance".into());
        } else if definition.is_none() {
            let unresolved = profile
                .iodd
                .as_ref()
                .is_some_and(|iodd| iodd.standard_event_refs.contains(&code));
            review.push(
                if unresolved {
                    "Standard event reference needs supplied definitions"
                } else {
                    "Event code is absent from the saved IODD event catalog"
                }
                .into(),
            );
        }
        if let Some(definition) = &definition {
            let declared = match definition.event_type {
                EventType::Notification => 1,
                EventType::Warning => 2,
                EventType::Error => 3,
            };
            if declared != event_type {
                review.push("Observed event type differs from the IODD declaration".into());
            }
        }
        result.events.push(Event {
            slot: index + 1,
            raw_hex: decode::hex(entry),
            qualifier,
            code,
            code_hex: format!("0x{code:04X}"),
            instance,
            instance_name: match instance {
                0 => "Unknown",
                4 => "Application",
                5 => "System (revision-specific)",
                _ => "Reserved",
            },
            source,
            source_name: if source == 0 {
                "Device (remote)"
            } else {
                "Master/Port"
            },
            event_type,
            type_name: match event_type {
                1 => "Notification",
                2 => "Warning",
                3 => "Error",
                _ => "Reserved",
            },
            mode,
            mode_name: match mode {
                1 => "Single shot",
                2 => "Disappears",
                3 => "Appears",
                _ => "Reserved",
            },
            pending_shape,
            definition,
            review,
        });
    }
    result
}

impl DetailedStatus {
    pub fn text(&self) -> String {
        let mut text = format!(
            "37:0 raw {} · {} empty slots",
            self.raw_hex, self.empty_slots
        );
        if let Some(error) = &self.decode_error {
            text.push_str(&format!("\nDecode review · {error}"));
        }
        for event in &self.events {
            text.push_str(&format!(
                "\n\nSlot {} · {} · qualifier 0x{:02X} · {} · {} · {} · {}",
                event.slot,
                event.code_hex,
                event.qualifier,
                event.source_name,
                event.instance_name,
                event.type_name,
                event.mode_name
            ));
            if let Some(definition) = &event.definition {
                text.push_str(&format!("\n{}", definition.name));
                if let Some(description) = &definition.description {
                    text.push_str(&format!("\n{description}"));
                }
            }
            for note in &event.review {
                text.push_str(&format!("\nReview · {note}"));
            }
        }
        text
    }
}

#[derive(Debug, Serialize)]
pub struct Observation {
    pub index: u16,
    pub subindex: u8,
    pub host_read_utc: String,
    pub read_duration_us: u64,
    pub status: Option<Status>,
    pub raw_hex: Option<String>,
    pub read_error: Option<String>,
    #[serde(skip)]
    bytes: Option<Vec<u8>>,
}

fn observe(profile: &Profile, index: u16) -> Observation {
    let start = Instant::now();
    let response = transport::read_isdu_observation(profile, index, 0);
    let (status, bytes, read_error) = match response {
        Ok((status, bytes)) => (Some(status), Some(bytes), None),
        Err(error) => (None, None, Some(format!("{error:#}"))),
    };
    Observation {
        index,
        subindex: 0,
        host_read_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        read_duration_us: start.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
        status,
        raw_hex: bytes.as_ref().map(|bytes| decode::hex(bytes)),
        bytes,
        read_error,
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub observation_kind: &'static str,
    pub observed_identities_equal: Option<bool>,
    pub device_status: Option<DeviceStatus>,
    pub detailed_status: Option<DetailedStatus>,
    pub reads: Vec<Observation>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.reads.len() == 2
            && self.device_status.is_some()
            && self.detailed_status.is_some()
            && self.reads.iter().all(|read| read.read_error.is_none())
            && self.observed_identities_equal != Some(false)
            && self
                .device_status
                .as_ref()
                .is_none_or(|status| status.decode_error.is_none())
            && self
                .detailed_status
                .as_ref()
                .is_none_or(|status| status.decode_error.is_none())
    }

    pub fn text(&self) -> String {
        let mut text = "Explicit acyclic snapshots · source event history/loss unknown".to_owned();
        for read in &self.reads {
            text.push_str(&format!(
                "\n{}:0 · host {} · query {} µs · {}",
                read.index,
                read.host_read_utc,
                read.read_duration_us,
                read.status
                    .as_ref()
                    .map(|status| status.route)
                    .unwrap_or("read unavailable")
            ));
            if let Some(error) = &read.read_error {
                text.push_str(&format!("\nRead error · {error}"));
            }
        }
        if self.observed_identities_equal == Some(false) {
            text.push_str("\nReview · observed identity fields changed between the two reads");
        }
        if let Some(status) = &self.device_status {
            text.push_str(&format!(
                "\n36:0 raw {} · {}",
                status.raw_hex,
                status.meaning.unwrap_or("unavailable")
            ));
            if let Some(error) = &status.decode_error {
                text.push_str(&format!("\nDecode review · {error}"));
            }
        }
        if let Some(status) = &self.detailed_status {
            text.push_str(&format!("\n\n{}", status.text()));
        }
        text
    }
}

/// Explicit optional ISDU reads on independent clients. No cyclic-worker calls.
pub fn read(profile: &Profile) -> Result<Report> {
    profile.validate()?;
    let reads = vec![observe(profile, 36), observe(profile, 37)];
    let observed_identities_equal = reads[0]
        .status
        .as_ref()
        .zip(reads[1].status.as_ref())
        .map(|(a, b)| a.identity == b.identity);
    let device_status = reads[0].bytes.as_deref().map(decode_device_status);
    let detailed_status = reads[1]
        .bytes
        .as_deref()
        .map(|bytes| decode_detailed(profile, bytes));
    Ok(Report {
        observation_kind: "explicit acyclic snapshots; source event history/loss unknown",
        observed_identities_equal,
        device_status,
        detailed_status,
        reads,
    })
}

#[cfg(test)]
mod tests;
