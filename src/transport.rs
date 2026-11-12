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
    profile.validate()?;
    match profile.connection.gateway {
        Gateway::Simulator => simulator::read_isdu(index, subindex),
        Gateway::IfmAl1350 => {
            let mut client = ifm::Client::new(&profile.connection)?;
            let status = client.connect()?;
            profile.expected_identity.verify(&status.identity)?;
            client.read_isdu(index, subindex)
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
