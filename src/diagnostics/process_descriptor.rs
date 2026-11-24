//! Offline Common Profile B.5 descriptor review.
use crate::{
    decode::{self, Field, Kind, MAX_INPUT},
    profile::{Imported, Profile},
};
use anyhow::Result;
use serde::Serialize;
use std::fmt::Write;

const MAX_BITS: u16 = MAX_INPUT as u16 * 8;
const MAX_ISDU_BYTES: usize = 232;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DescriptorDirection {
    #[default]
    Input,
    Output,
}

impl DescriptorDirection {
    pub fn index(self) -> u16 {
        match self {
            Self::Input => 14,
            Self::Output => 15,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DescriptorStatus {
    Match,
    Different,
    Partial,
    NoIodd,
    InvalidReply,
}

#[derive(Debug, Serialize)]
pub struct ProcessDescriptor {
    pub direction: DescriptorDirection,
    pub index: u16,
    pub subindex: u8,
    pub raw_hex: String,
    pub status: DescriptorStatus,
    pub entries: Vec<DescriptorEntry>,
    pub comparison: Option<DescriptorComparison>,
    pub review: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DescriptorEntry {
    pub datatype_code: u8,
    pub datatype: &'static str,
    pub bits: u16,
    pub bit_offset: u16,
}

impl DescriptorEntry {
    fn end(&self) -> u16 {
        self.bit_offset + self.bits
    }
}

#[derive(Debug, Serialize)]
pub struct DescriptorComparison {
    pub layout_id: Option<String>,
    pub condition: Option<String>,
    pub payload_bytes: usize,
    pub fields: Vec<DescriptorField>,
    pub partly_or_wholly_unmapped_entries: Vec<usize>,
    pub outside_payload_entries: Vec<usize>,
}

#[derive(Debug, Serialize)]
pub struct DescriptorField {
    pub name: String,
    pub datatype: &'static str,
    pub bits: u16,
    pub bit_offset: u16,
    pub status: &'static str,
}

impl ProcessDescriptor {
    pub fn text(&self) -> String {
        let status = match self.status {
            DescriptorStatus::Match => "Descriptor matches mapped IODD fields",
            DescriptorStatus::Different => "Descriptor differs from the selected IODD mapping",
            DescriptorStatus::Partial => "Descriptor comparison needs more mapping information",
            DescriptorStatus::NoIodd => "Decoded descriptor · IODD mapping unavailable",
            DescriptorStatus::InvalidReply => "Descriptor reply needs review",
        };
        let raw = if self.raw_hex.is_empty() {
            "Empty"
        } else {
            &self.raw_hex
        };
        let mut text = format!("{status}\nRaw reply · {raw}");
        for (index, entry) in self.entries.iter().enumerate() {
            let _ = write!(
                text,
                "\n{} · {} (0x{:02X}) · {} bits at bit {}",
                index + 1,
                entry.datatype,
                entry.datatype_code,
                entry.bits,
                entry.bit_offset
            );
        }
        for issue in &self.review {
            let _ = write!(text, "\n{issue}");
        }
        if let Some(comparison) = &self.comparison {
            let layout = comparison.layout_id.as_deref().unwrap_or("Absent");
            let _ = write!(
                text,
                "\nSelected IODD layout · {layout} · {} bytes",
                comparison.payload_bytes
            );
            if let Some(condition) = &comparison.condition {
                let _ = write!(text, "\nIODD condition · {condition}");
            }
            for field in &comparison.fields {
                let _ = write!(
                    text,
                    "\n{} · {} · {} bits at bit {} · {}",
                    field.name, field.datatype, field.bits, field.bit_offset, field.status
                );
            }
            if !comparison.partly_or_wholly_unmapped_entries.is_empty() {
                let _ = write!(
                    text,
                    "\nEntries covering unmapped bits · {:?}",
                    comparison.partly_or_wholly_unmapped_entries
                );
            }
            if !comparison.outside_payload_entries.is_empty() {
                let _ = write!(
                    text,
                    "\nEntries outside the IODD payload · {:?}",
                    comparison.outside_payload_entries
                );
            }
        }
        text
    }
}

fn datatype_label(code: u8) -> &'static str {
    match code {
        0 => "OctetStringT",
        1 => "Set of BooleanT",
        2 => "UIntegerT",
        3 => "IntegerT",
        4 => "Float32T",
        5 => "StringT",
        6 => "TimeT",
        7 => "TimeSpanT",
        8..=127 => "Reserved datatype",
        _ => "Profile-specific datatype",
    }
}

fn datatype_code(kind: Kind) -> u8 {
    match kind {
        Kind::OctetString => 0,
        Kind::Bool => 1,
        Kind::Unsigned => 2,
        Kind::Signed => 3,
        Kind::Float32 => 4,
        Kind::AsciiString | Kind::Utf8String => 5,
        Kind::Time => 6,
        Kind::TimeSpan => 7,
    }
}

pub fn decode_process_descriptor(
    direction: DescriptorDirection,
    profile: Option<&Profile>,
    hex: &str,
) -> Result<ProcessDescriptor> {
    let bytes = if hex.trim().is_empty() {
        Vec::new()
    } else {
        decode::parse_hex_bounded(hex, MAX_ISDU_BYTES)?
    };
    let (records, tail) = bytes.as_chunks::<3>();
    let entries: Vec<_> = records
        .iter()
        .map(|&[datatype_code, length, offset]| DescriptorEntry {
            datatype_code,
            datatype: datatype_label(datatype_code),
            bits: if length == 0 { 256 } else { length.into() },
            bit_offset: offset.into(),
        })
        .collect();

    let mut review = Vec::new();
    if !tail.is_empty() {
        review.push("Reply must contain complete three-byte descriptor entries".into());
    }
    for (index, entry) in entries.iter().enumerate() {
        if entry.end() > MAX_BITS {
            review.push(format!("Entry {} extends past bit 255", index + 1));
        }
    }
    for (index, pair) in entries.windows(2).enumerate() {
        if pair[1].bit_offset <= pair[0].bit_offset {
            review.push(format!(
                "Entry {} is outside ascending offset order",
                index + 2
            ));
        } else if pair[1].bit_offset < pair[0].end() {
            review.push(format!("Entry {} overlaps the preceding entry", index + 2));
        }
    }

    let (status, comparison) = match (review.is_empty(), profile.and_then(|p| p.iodd.as_ref())) {
        (false, _) => (DescriptorStatus::InvalidReply, None),
        (true, None) => (DescriptorStatus::NoIodd, None),
        (true, Some(iodd)) => {
            let (status, comparison) = compare(direction, iodd, &entries);
            (status, Some(comparison))
        }
    };
    Ok(ProcessDescriptor {
        direction,
        index: direction.index(),
        subindex: 0,
        raw_hex: decode::hex(&bytes),
        status,
        entries,
        comparison,
        review,
    })
}

fn compare(
    direction: DescriptorDirection,
    iodd: &Imported,
    entries: &[DescriptorEntry],
) -> (DescriptorStatus, DescriptorComparison) {
    let (layout_id, capacity, fields, standalone_boolean): (Option<&str>, usize, &[Field], bool) =
        match direction {
            DescriptorDirection::Input if iodd.input_bytes > 0 => (
                Some(&iodd.layout),
                iodd.input_bytes,
                &iodd.fields,
                matches!(
                    iodd.fields.as_slice(),
                    [Field {
                        kind: Kind::Bool,
                        bits: 8,
                        ..
                    }]
                ),
            ),
            DescriptorDirection::Output => {
                iodd.output
                    .as_ref()
                    .map_or((None, 0, &[], false), |output| {
                        (
                            Some(output.id.as_str()),
                            output.bytes(),
                            output.fields.as_slice(),
                            output.standalone_boolean,
                        )
                    })
            }
            _ => (None, 0, &[], false),
        };
    let mut comparison = DescriptorComparison {
        layout_id: layout_id.map(str::to_owned),
        condition: layout_id.and(iodd.condition.clone()),
        payload_bytes: capacity,
        fields: Vec::new(),
        partly_or_wholly_unmapped_entries: Vec::new(),
        outside_payload_entries: Vec::new(),
    };
    let mut different = false;
    let mut partial = false;
    for field in fields {
        let first = field.bit_offset;
        let end = first + field.bits;
        let code = datatype_code(field.kind);
        let status = match entries
            .iter()
            .find(|entry| entry.bit_offset < end && entry.end() > first)
        {
            None => {
                different = true;
                "Missing from reply"
            }
            Some(entry) if entry.datatype_code > 7 => {
                partial = true;
                "Datatype needs profile-specific review"
            }
            Some(entry) if standalone_boolean && entry.datatype_code == 1 => {
                partial = true;
                "Standalone Boolean width needs review"
            }
            Some(entry)
                if entry.datatype_code == code
                    && ((entry.bit_offset == first && entry.end() == end)
                        || (code == 1 && entry.bit_offset <= first && entry.end() >= end)) =>
            {
                "Match"
            }
            Some(_) => {
                different = true;
                "Datatype or bit range differs"
            }
        };
        comparison.fields.push(DescriptorField {
            name: field.name.clone(),
            datatype: datatype_label(code),
            bits: field.bits,
            bit_offset: field.bit_offset,
            status,
        });
    }
    for (index, entry) in entries.iter().enumerate() {
        if usize::from(entry.end()) > capacity * 8 {
            different = true;
            comparison.outside_payload_entries.push(index + 1);
        }
        if (entry.bit_offset..entry.end()).any(|bit| {
            !fields
                .iter()
                .any(|field| (field.bit_offset..field.bit_offset + field.bits).contains(&bit))
        }) {
            partial = true;
            comparison.partly_or_wholly_unmapped_entries.push(index + 1);
        }
    }
    let status = if different {
        DescriptorStatus::Different
    } else if partial || (capacity > 0 && fields.is_empty()) {
        DescriptorStatus::Partial
    } else {
        DescriptorStatus::Match
    };
    (status, comparison)
}
