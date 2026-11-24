//! Offline decoding of the wired IO-Link V1.1 Direct Parameter Page 1.

use crate::{decode, profile::Identity};
use anyhow::Result;
use serde::Serialize;
use std::fmt::Write;

const PAGE_BYTES: usize = 16;
const V11_REVISION: u8 = 0x11;

#[derive(Serialize)]
pub struct DirectPage {
    pub raw_hex: String,
    pub revision_id: Option<u8>,
    pub protocol_revision: Option<String>,
    pub issues: Vec<&'static str>,
    pub link: Option<Link>,
}

#[derive(Serialize)]
pub struct Link {
    pub vendor_id: u16,
    pub device_id: u32,
    pub expected_vendor_id: Option<u16>,
    pub expected_device_id: Option<u32>,
    pub master_cycle_us: Option<u32>,
    pub device_min_cycle_us: Option<u32>,
    pub m_sequence_capability: u8,
    pub isdu_supported: bool,
    pub operate_code: u8,
    pub preoperate_code: u8,
    pub sio_supported: bool,
    pub input: ProcessLength,
    pub output: ProcessLength,
}

#[derive(Serialize)]
pub struct ProcessLength {
    pub raw: u8,
    pub octet_structured: bool,
    pub bits: Option<u16>,
}

impl DirectPage {
    pub fn ok(&self) -> bool {
        self.issues.is_empty()
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "Direct Parameter Page 1 · 0:0\nRaw reply · {}",
            self.raw_hex
        );
        if let Some(revision) = &self.protocol_revision {
            let _ = write!(text, "\nProtocol revision · {revision}");
        }
        if let Some(link) = &self.link {
            let _ = write!(
                text,
                "\nVendor ID · 0x{:04X}{}\nDevice ID · 0x{:06X}{}",
                link.vendor_id,
                link.expected_vendor_id
                    .map_or_else(String::new, |id| format!(" · Profile expects 0x{id:04X}")),
                link.device_id,
                link.expected_device_id
                    .map_or_else(String::new, |id| format!(" · Profile expects 0x{id:06X}")),
            );
            let _ = write!(
                text,
                "\nRecorded master cycle · {}\nRecorded device minimum cycle · {}",
                link.master_cycle_us
                    .map_or_else(|| "Unavailable".into(), |us| format!("{us} µs")),
                link.device_min_cycle_us
                    .map_or_else(|| "Unavailable".into(), |us| format!("{us} µs")),
            );
            let _ = write!(
                text,
                "\nISDU · {} · SIO · {}\nM-sequence codes · PREOPERATE {} · OPERATE {}",
                if link.isdu_supported {
                    "Supported"
                } else {
                    "Unsupported"
                },
                if link.sio_supported {
                    "Supported"
                } else {
                    "Unsupported"
                },
                link.preoperate_code,
                link.operate_code,
            );
            let _ = write!(
                text,
                "\nProcess input · {}\nProcess output · {}",
                link.input.text(),
                link.output.text(),
            );
        }
        for issue in &self.issues {
            let _ = write!(text, "\nReview · {issue}");
        }
        text
    }
}

impl ProcessLength {
    fn decode(raw: u8) -> Self {
        let octet_structured = raw & 0x80 != 0;
        let code = u16::from(raw & 0x1f);
        let bits = match (octet_structured, code) {
            (false, 0..=16) => Some(code),
            (true, 2..=31) => Some((code + 1) * 8),
            _ => None,
        };
        Self {
            raw,
            octet_structured,
            bits,
        }
    }

    fn text(&self) -> String {
        match self.bits {
            Some(0) => "None".into(),
            Some(bits) if self.octet_structured => format!("{} octets ({bits} bits)", bits / 8),
            Some(bits) => format!("{bits} bits"),
            None => format!("Reserved length code (0x{:02X})", self.raw),
        }
    }
}

/// Decode bytes in ascending direct-parameter address order (0x00–0x0F).
pub fn decode_direct_page(expected: Option<&Identity>, hex: &str) -> Result<DirectPage> {
    let bytes = if hex.trim().is_empty() {
        Vec::new()
    } else {
        decode::parse_hex_bounded(hex, 232)?
    };
    let mut report = DirectPage {
        raw_hex: decode::hex(&bytes),
        revision_id: None,
        protocol_revision: None,
        issues: Vec::new(),
        link: None,
    };
    let Ok(page) = <[u8; PAGE_BYTES]>::try_from(bytes) else {
        report.issues.push("Page 1 requires exactly 16 bytes");
        return Ok(report);
    };
    let revision_id = page[4];
    report.revision_id = Some(revision_id);
    report.protocol_revision = Some(format!("{}.{}", revision_id >> 4, revision_id & 0x0F));
    if revision_id != V11_REVISION {
        report
            .issues
            .push("Link decoding requires wired IO-Link protocol revision 1.1");
        return Ok(report);
    }

    let master_cycle_us = cycle_us(page[1]);
    let device_min_cycle_us = cycle_us(page[2]);
    let m_sequence_capability = page[3];
    let input = ProcessLength::decode(page[5]);
    let output = ProcessLength::decode(page[6]);
    let vendor_id = u16::from_be_bytes([page[7], page[8]]);
    let device_id = u32::from_be_bytes([0, page[9], page[10], page[11]]);
    let expected_vendor_id = expected.and_then(|identity| identity.vendor_id);
    let expected_device_id = expected.and_then(|identity| identity.device_id);
    for (needs_review, issue) in [
        (
            master_cycle_us.is_none(),
            "MasterCycleTime has no defined cycle duration",
        ),
        (
            device_min_cycle_us.is_none() && page[2] != 0,
            "MinCycleTime uses a reserved encoding",
        ),
        (
            m_sequence_capability & 0xc0 != 0,
            "Reserved M-sequence capability bits are set",
        ),
        (
            input.bits.is_none(),
            "ProcessDataIn uses a reserved length encoding",
        ),
        (
            output.bits.is_none(),
            "ProcessDataOut uses a reserved length encoding",
        ),
        (input.raw & 0x20 != 0, "A reserved ProcessDataIn bit is set"),
        (
            output.raw & 0x60 != 0,
            "Reserved ProcessDataOut bits are set",
        ),
        (device_id == 0, "DeviceID 0 is reserved"),
        (
            expected_vendor_id.is_some_and(|id| id != vendor_id),
            "Vendor ID differs from the profile pin",
        ),
        (
            expected_device_id.is_some_and(|id| id != device_id),
            "Device ID differs from the profile pin",
        ),
    ] {
        if needs_review {
            report.issues.push(issue);
        }
    }
    report.link = Some(Link {
        vendor_id,
        device_id,
        expected_vendor_id,
        expected_device_id,
        master_cycle_us,
        device_min_cycle_us,
        m_sequence_capability,
        isdu_supported: m_sequence_capability & 1 != 0,
        operate_code: (m_sequence_capability >> 1) & 7,
        preoperate_code: (m_sequence_capability >> 4) & 3,
        sio_supported: input.raw & 0x40 != 0,
        input,
        output,
    });
    Ok(report)
}

fn cycle_us(raw: u8) -> Option<u32> {
    let multiplier = u32::from(raw & 0x3f);
    match raw >> 6 {
        0 if multiplier >= 4 => Some(multiplier * 100),
        1 => Some(6400 + multiplier * 400),
        2 => Some(32000 + multiplier * 1600),
        _ => None,
    }
}
