use crate::{
    decode::MAX_INPUT,
    profile::{Gateway, Identity, Profile},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::time::Instant;

mod ifm;
mod simulator;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Snapshots,
    Counter32,
    Timestamp,
}

impl Freshness {
    pub fn row_kind(self) -> &'static str {
        match self {
            Self::Snapshots => "host_poll",
            Self::Counter32 | Self::Timestamp => "source_update",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Validity {
    Valid,
    Invalid,
    Unknown,
}

impl Validity {
    pub fn label(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Payload {
    bytes: [u8; MAX_INPUT],
    len: u8,
}

impl Payload {
    pub fn new(input: &[u8]) -> Result<Self> {
        ensure!(input.len() <= MAX_INPUT, "input exceeds {MAX_INPUT} bytes");
        let mut bytes = [0; MAX_INPUT];
        bytes[..input.len()].copy_from_slice(input);
        Ok(Self {
            bytes,
            len: input.len() as u8,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Input {
    pub received: Instant,
    pub payload: Payload,
    pub validity: Validity,
    /// Counter increments once per input update and wraps at 2^32 in Counter32 mode.
    pub counter: Option<u32>,
    /// Monotonic gateway time of this input update, in microseconds, within this generation.
    pub source_time_us: Option<u64>,
    /// Gateway-provided input age at receipt, if available.
    pub source_age_us: Option<u64>,
    pub diagnostic_code: Option<u16>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub route: &'static str,
    pub model: String,
    pub firmware: String,
    pub master_serial: Option<String>,
    pub port: u8,
    pub port_mode: String,
    pub device_status: Option<String>,
    pub port_event: Option<String>,
    pub device_event: Option<String>,
    pub port_cycle_raw: Option<String>,
    pub port_cycle_us: Option<u64>,
    pub identity: Identity,
    pub freshness: Freshness,
}

pub trait InputTransport: Send {
    fn connect(&mut self) -> Result<Status>;
    fn poll(&mut self) -> Result<Input>;
}

pub fn open(profile: &Profile, simulate_faults: bool) -> Result<Box<dyn InputTransport>> {
    profile.validate()?;
    ensure!(
        profile.connection.gateway == Gateway::Simulator || !simulate_faults,
        "fault injection requires the simulator gateway"
    );
    match profile.connection.gateway {
        Gateway::Simulator => Ok(Box::new(simulator::Simulator::new(
            profile,
            simulate_faults,
        ))),
        Gateway::IfmAl1350 => Ok(Box::new(ifm::Client::new(&profile.connection)?)),
    }
}

/// A separate client and call path, independent of the cyclic poll worker and CSV queue.
pub fn read_isdu(profile: &Profile, index: u16, subindex: u8) -> Result<Vec<u8>> {
    Ok(read_isdu_observation(profile, index, subindex)?.1)
}

pub(crate) fn read_isdu_observation(
    profile: &Profile,
    index: u16,
    subindex: u8,
) -> Result<(Status, Vec<u8>)> {
    profile.validate()?;
    match profile.connection.gateway {
        Gateway::Simulator => {
            let status = simulator::Simulator::new(profile, false).connect()?;
            profile.expected_identity.verify(&status.identity)?;
            Ok((status, simulator::read_isdu(index, subindex)?))
        }
        Gateway::IfmAl1350 => {
            let mut client = ifm::Client::new(&profile.connection)?;
            let status = client.connect()?;
            profile.expected_identity.verify(&status.identity)?;
            Ok((status, client.read_isdu(index, subindex)?))
        }
    }
}

/// Verify the selected IODD product variant using the standard Product ID ISDU.
pub fn verify_product_id(profile: &Profile) -> Result<String> {
    let expected = profile
        .expected_identity
        .product_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("set the expected Product ID in the profile first"))?;
    let bytes = read_isdu(profile, 19, 0)?;
    let actual = std::str::from_utf8(&bytes)?.trim_end_matches('\0');
    ensure!(
        actual == expected,
        "Product ID mismatch (expected {expected:?}, observed {actual:?})"
    );
    Ok(actual.to_owned())
}

#[derive(Debug, Serialize)]
pub struct ParameterReading {
    pub parameter_id: String,
    pub name: String,
    pub index: u16,
    pub subindex: u8,
    pub host_read_utc: String,
    pub read_duration_us: u64,
    pub raw_hex: String,
    pub raw_value: Option<String>,
    pub displays: Vec<crate::iodd::DisplayValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_display: Option<crate::iodd::DisplayValue>,
    pub choice_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_assessment: Option<crate::iodd::ValueAssessment>,
    pub decode_error: Option<String>,
    pub record_items: Vec<crate::iodd::RecordValue>,
    pub array_items: Vec<crate::iodd::RecordValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_value: Option<crate::iodd::TimeValue>,
}

/// Read one explicit IODD variable at subindex zero on the independent acyclic path.
pub fn read_parameter(profile: &Profile, id: &str) -> Result<ParameterReading> {
    read_parameter_at(profile, id, 0)
}

pub fn read_parameter_at(profile: &Profile, id: &str, subindex: u8) -> Result<ParameterReading> {
    Ok(read_parameter_observed(profile, id, subindex)?.0)
}

pub(crate) fn parameter_target<'a>(
    profile: &'a Profile,
    id: &str,
    subindex: u8,
) -> Result<&'a crate::iodd::Parameter> {
    let parent = profile.parameter(id)?;
    ensure!(
        subindex == 0 || parent.subindex_access(),
        "IODD does not permit individual subindex reads for {id}"
    );
    let parameter = parent.target(subindex)?;
    ensure!(
        parent.access != crate::iodd::Access::WriteOnly
            && parameter.access != crate::iodd::Access::WriteOnly,
        "parameter {id} is write-only in the IODD"
    );
    Ok(parameter)
}

pub(crate) fn read_parameter_observed(
    profile: &Profile,
    id: &str,
    subindex: u8,
) -> Result<(ParameterReading, Status)> {
    let parameter = parameter_target(profile, id, subindex)?;
    let parent = profile.parameter(id)?;
    let start = Instant::now();
    let (status, bytes) = read_isdu_observation(profile, parameter.index, subindex)?;
    let host_read_utc = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let read_duration_us = start.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
    let decoded = parameter.decode(&bytes);
    Ok((
        ParameterReading {
            parameter_id: parent.id.clone(),
            name: if parent.array.is_some() && subindex > 0 {
                format!("{} [{subindex}]", parent.name)
            } else {
                parameter.name.clone()
            },
            index: parameter.index,
            subindex,
            host_read_utc,
            read_duration_us,
            raw_hex: decoded.raw_hex,
            raw_value: decoded.raw_value,
            displays: decoded.displays,
            preferred_display: decoded.preferred_display,
            choice_name: decoded.choice_name,
            value_assessment: decoded.value_assessment,
            decode_error: decoded.decode_error,
            record_items: decoded.record_items,
            array_items: decoded.array_items,
            time_value: decoded.time_value,
        },
        status,
    ))
}
