//! Review recorded Data Storage metadata as specified in IO-Link V1.1.5, B.2.3.

use crate::decode;
use anyhow::{Result, bail};
use serde::Serialize;
use std::fmt::Write as _;

pub const DATA_STORAGE_INDEX: u16 = 3;
const MAX_STORAGE_BYTES: u32 = 2048;
const MAX_PAGE_ENTRIES: usize = 70;
const ENTRY_BYTES: usize = 3;
const MARKER_BYTES: usize = 2;
const RESERVED_STATE_BITS: u8 = 0x79;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum DataStoragePart {
    #[default]
    State = 2,
    Size = 3,
    Checksum = 4,
    IndexList = 5,
}

impl DataStoragePart {
    pub const ALL: [Self; 4] = [Self::State, Self::Size, Self::Checksum, Self::IndexList];

    pub fn label(self) -> &'static str {
        match self {
            Self::State => "State (3:2)",
            Self::Size => "Storage size (3:3)",
            Self::Checksum => "Parameter checksum (3:4)",
            Self::IndexList => "Index list (3:5)",
        }
    }
}

impl TryFrom<u8> for DataStoragePart {
    type Error = anyhow::Error;

    fn try_from(subindex: u8) -> Result<Self> {
        match subindex {
            2 => Ok(Self::State),
            3 => Ok(Self::Size),
            4 => Ok(Self::Checksum),
            5 => Ok(Self::IndexList),
            _ => bail!("Data Storage metadata requires subindex 2, 3, 4, or 5"),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DataStorageState {
    Inactive,
    Upload,
    Download,
    Locked,
}

impl DataStorageState {
    fn label(&self) -> &'static str {
        match self {
            Self::Inactive => "Inactive",
            Self::Upload => "Upload",
            Self::Download => "Download",
            Self::Locked => "Locked",
        }
    }
}

#[derive(Serialize)]
pub struct DataStorageAddress {
    pub index: u16,
    pub subindex: u8,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataStorageValue {
    State {
        state: DataStorageState,
        upload_requested: bool,
        reserved_bits: u8,
    },
    Size {
        bytes: u32,
    },
    Checksum {
        revision_marker: u32,
    },
    IndexList {
        entries: Vec<DataStorageAddress>,
        next_index: u16,
    },
}

#[derive(Serialize)]
pub struct DataStorageReply {
    pub index: u16,
    pub subindex: u8,
    pub raw_hex: String,
    pub issue: Option<&'static str>,
    pub value: Option<DataStorageValue>,
}

impl DataStorageReply {
    pub fn ok(&self) -> bool {
        self.issue.is_none()
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "Device Data Storage · {}:{}\nRaw reply · {}",
            self.index, self.subindex, self.raw_hex
        );
        match &self.value {
            Some(DataStorageValue::State {
                state,
                upload_requested,
                reserved_bits,
            }) => {
                let request = if *upload_requested {
                    "Pending"
                } else {
                    "Clear"
                };
                let _ = write!(
                    text,
                    "\nState · {}\nUpload request · {request}",
                    state.label()
                );
                if *reserved_bits != 0 {
                    let _ = write!(text, "\nReserved state bits · 0x{reserved_bits:02X}");
                }
            }
            Some(DataStorageValue::Size { bytes }) => {
                let _ = write!(text, "\nStorage size · {bytes} bytes");
            }
            Some(DataStorageValue::Checksum { revision_marker }) => {
                let _ = write!(
                    text,
                    "\nParameter set revision marker · 0x{revision_marker:08X}"
                );
            }
            Some(DataStorageValue::IndexList {
                entries,
                next_index,
            }) => {
                let _ = write!(text, "\nEntries · {}", entries.len());
                for entry in entries {
                    let _ = write!(text, "\n  0x{:04X}:{}", entry.index, entry.subindex);
                }
                if *next_index == 0 {
                    text.push_str("\nEnd of index list");
                } else {
                    let _ = write!(text, "\nNext page · 0x{next_index:04X}");
                }
            }
            None => {}
        }
        if let Some(issue) = self.issue {
            let _ = write!(text, "\nReview · {issue}");
        }
        text
    }
}

/// Decode exactly one captured metadata reply. A continuation is displayed as
/// an index supplied by the device, with no implied access or page traversal.
pub fn decode_data_storage(part: DataStoragePart, hex: &str) -> Result<DataStorageReply> {
    let bytes = if hex.trim().is_empty() {
        Vec::new()
    } else {
        decode::parse_hex_bounded(hex, 232)?
    };
    let mut report = DataStorageReply {
        index: DATA_STORAGE_INDEX,
        subindex: part as u8,
        raw_hex: decode::hex(&bytes),
        issue: None,
        value: None,
    };
    match (part, bytes.as_slice()) {
        (DataStoragePart::State, &[flags]) => {
            let state = match flags & 0x06 {
                0x00 => DataStorageState::Inactive,
                0x02 => DataStorageState::Upload,
                0x04 => DataStorageState::Download,
                _ => DataStorageState::Locked,
            };
            let reserved_bits = flags & RESERVED_STATE_BITS;
            if reserved_bits != 0 {
                report.issue = Some("Reserved bits are set in State_Property");
            }
            report.value = Some(DataStorageValue::State {
                state,
                upload_requested: flags & 0x80 != 0,
                reserved_bits,
            });
        }
        (DataStoragePart::State, _) => {
            report.issue = Some("State_Property requires exactly one byte");
        }
        (DataStoragePart::Size | DataStoragePart::Checksum, &[a, b, c, d]) => {
            let value = u32::from_be_bytes([a, b, c, d]);
            if part == DataStoragePart::Size {
                if value > MAX_STORAGE_BYTES {
                    report.issue = Some("Data_Storage_Size exceeds 2048 bytes");
                }
                report.value = Some(DataStorageValue::Size { bytes: value });
            } else {
                report.value = Some(DataStorageValue::Checksum {
                    revision_marker: value,
                });
            }
        }
        (DataStoragePart::Size | DataStoragePart::Checksum, _) => {
            report.issue = Some("Storage size and parameter checksum require exactly four bytes");
        }
        (DataStoragePart::IndexList, _) => match decode_index_list(&bytes) {
            Ok(value) => report.value = Some(value),
            Err(issue) => report.issue = Some(issue),
        },
    }
    Ok(report)
}

fn decode_index_list(bytes: &[u8]) -> Result<DataStorageValue, &'static str> {
    let Some((entries, marker)) = bytes.split_last_chunk::<MARKER_BYTES>() else {
        return Err("Index_List is missing its two-byte termination marker");
    };
    let (entries, leftover) = entries.as_chunks::<ENTRY_BYTES>();
    if !leftover.is_empty() {
        return Err("Index_List requires complete three-byte entries before the marker");
    }
    if entries.len() > MAX_PAGE_ENTRIES {
        return Err("Index_List exceeds 70 entries per page");
    }
    Ok(DataStorageValue::IndexList {
        entries: entries
            .iter()
            .map(|&[high, low, subindex]| DataStorageAddress {
                index: u16::from_be_bytes([high, low]),
                subindex,
            })
            .collect(),
        next_index: u16::from_be_bytes(*marker),
    })
}
