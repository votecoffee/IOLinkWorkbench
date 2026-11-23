use anyhow::{Result, anyhow, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::BTreeMap, fmt::Write};

pub const MAX_INPUT: usize = 32;
pub(crate) mod temporal;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ByteOrder {
    #[default]
    BigEndian,
    LittleEndian,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Unsigned,
    Signed,
    Bool,
    Float32,
    OctetString,
    Utf8String,
    AsciiString,
    Time,
    TimeSpan,
}

impl Kind {
    pub(crate) fn is_numeric(self) -> bool {
        matches!(self, Self::Unsigned | Self::Signed | Self::Float32)
    }

    pub(crate) fn is_text(self) -> bool {
        matches!(self, Self::Utf8String | Self::AsciiString)
    }

    pub fn is_byte_sequence(self) -> bool {
        self.is_text() || self == Self::OctetString
    }

    fn temporal_datatype(self) -> Option<&'static str> {
        match self {
            Self::Time => Some("TimeT"),
            Self::TimeSpan => Some("TimeSpanT"),
            _ => None,
        }
    }

    pub fn is_temporal(self) -> bool {
        self.temporal_datatype().is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub bit_offset: u16,
    pub bits: u16,
    pub byte_order: ByteOrder,
    pub kind: Kind,
    pub scale: f64,
    pub offset: f64,
    pub unit: String,
    pub decimals: Option<u8>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub value_names: BTreeMap<String, String>,
}

impl Default for Field {
    fn default() -> Self {
        Self {
            name: String::new(),
            bit_offset: 0,
            bits: 8,
            byte_order: ByteOrder::BigEndian,
            kind: Kind::Unsigned,
            scale: 1.0,
            offset: 0.0,
            unit: String::new(),
            decimals: None,
            value_names: BTreeMap::new(),
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Value {
    pub bits_hex: String,
    pub raw: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_error: Option<String>,
}

impl Value {
    pub fn display(&self) -> Cow<'_, str> {
        match &self.value_name {
            Some(name) => Cow::Owned(format!("{name} ({})", self.value)),
            None => Cow::Borrowed(&self.value),
        }
    }
}

impl Field {
    pub fn same_wire_as(&self, other: &Self) -> bool {
        (self.kind, self.bits, self.bit_offset, self.byte_order)
            == (other.kind, other.bits, other.bit_offset, other.byte_order)
    }

    pub fn validate(&self, input_bytes: usize) -> Result<()> {
        ensure!(!self.name.trim().is_empty(), "field name is empty");
        ensure!(
            !self.name.contains(['\r', '\n']),
            "field name contains a line break"
        );
        let bits = usize::from(self.bits);
        let max_bits = if self.kind.is_byte_sequence() {
            MAX_INPUT * 8
        } else {
            64
        };
        ensure!(
            (1..=max_bits).contains(&bits),
            "{}: width must be 1–{max_bits}",
            self.name
        );
        ensure!(
            usize::from(self.bit_offset) + bits <= input_bytes.saturating_mul(8),
            "{}: field extends outside the payload",
            self.name
        );
        if self.kind.is_byte_sequence() {
            ensure!(
                self.bit_offset.is_multiple_of(8) && self.bits.is_multiple_of(8),
                "{}: string fields require byte-aligned offsets and whole bytes",
                self.name
            );
            ensure!(
                self.byte_order == ByteOrder::BigEndian,
                "{}: string fields use big-endian offsets and preserve wire byte order",
                self.name
            );
        }
        if self.kind.is_temporal() {
            ensure!(
                self.bits == 64
                    && self.bit_offset.is_multiple_of(8)
                    && self.byte_order == ByteOrder::BigEndian,
                "{}: temporal fields require 64 bits with byte-aligned big-endian offsets",
                self.name
            );
        }
        ensure!(
            self.kind != Kind::Bool || matches!(self.bits, 1 | 8),
            "{}: bool needs one bit or one byte",
            self.name
        );
        ensure!(
            self.kind != Kind::Float32 || self.bits == 32,
            "{}: float32 needs 32 bits",
            self.name
        );
        ensure!(
            self.scale.is_finite() && self.offset.is_finite(),
            "{}: nonfinite scaling",
            self.name
        );
        ensure!(
            self.kind.is_numeric() || (self.scale == 1.0 && self.offset == 0.0),
            "{}: {:?} does not support scaling",
            self.name,
            self.kind,
        );
        ensure!(
            self.decimals
                .is_none_or(|places| places <= 12 && self.kind.is_numeric()),
            "{}: decimals must be 0–12 on a numeric field",
            self.name
        );
        Ok(())
    }

    pub fn read_bits(&self, payload: &[u8]) -> Result<u64> {
        self.validate(payload.len())?;
        ensure!(
            !self.kind.is_byte_sequence(),
            "string fields have no integer bit value"
        );
        let mut value = 0;
        for relative in 0..usize::from(self.bits) {
            let absolute = usize::from(self.bit_offset) + relative;
            let index = match self.byte_order {
                ByteOrder::BigEndian => payload.len() - 1 - absolute / 8,
                ByteOrder::LittleEndian => absolute / 8,
            };
            value |= u64::from((payload[index] >> (absolute % 8)) & 1) << relative;
        }
        Ok(value)
    }

    fn aligned_bytes<'a>(&self, payload: &'a [u8]) -> Result<&'a [u8]> {
        self.validate(payload.len())?;
        let end = payload.len() - usize::from(self.bit_offset / 8);
        Ok(&payload[end - usize::from(self.bits / 8)..end])
    }

    pub(crate) fn csv_value(&self, payload: &[u8]) -> Result<String> {
        if let Some(datatype) = self.kind.temporal_datatype() {
            return temporal::raw_ticks(datatype, self.aligned_bytes(payload)?);
        }
        Ok(self.decode_unlabeled(payload)?.value)
    }

    pub fn decode(&self, payload: &[u8]) -> Result<Value> {
        let mut value = self.decode_unlabeled(payload)?;
        let mut name = self.value_names.get(&value.raw);
        if name.is_none() && self.kind == Kind::Float32 && value.raw == "-0" {
            // Keep negative zero in the raw display while matching IODD numeric equality.
            name = self.value_names.get("0");
        }
        value.value_name = name.cloned();
        Ok(value)
    }

    fn decode_unlabeled(&self, payload: &[u8]) -> Result<Value> {
        if let Some(datatype) = self.kind.temporal_datatype() {
            let bytes = self.aligned_bytes(payload)?;
            let decoded = temporal::decode(datatype, bytes)?;
            return Ok(Value {
                raw: decoded.raw_ticks,
                bits_hex: hex(bytes),
                value: decoded.value,
                value_name: None,
                encoding_error: None,
            });
        }
        if self.kind.is_byte_sequence() {
            let bytes = self.aligned_bytes(payload)?;
            let bits_hex = hex(bytes);
            let (value, encoding_error) = if self.kind == Kind::OctetString {
                (octet_sequence(bytes), None)
            } else {
                match decode_padded_text(bytes, self.kind == Kind::AsciiString) {
                    Ok(text) => (serde_json::to_string(text)?, None),
                    Err(error) => {
                        let error = error.to_string();
                        (format!("Invalid StringT ({error})"), Some(error))
                    }
                }
            };
            return Ok(Value {
                raw: bits_hex.clone(),
                bits_hex,
                value,
                value_name: None,
                encoding_error,
            });
        }
        let bits = self.read_bits(payload)?;
        let (raw, number) = match self.kind {
            Kind::Unsigned => (bits.to_string(), bits as f64),
            Kind::Signed => {
                let shift = 64 - self.bits;
                let signed = ((bits << shift) as i64) >> shift;
                (signed.to_string(), signed as f64)
            }
            Kind::Bool => ((bits != 0).to_string(), bits as f64),
            Kind::Float32 => {
                let number = f32::from_bits(bits as u32);
                (number.to_string(), f64::from(number))
            }
            Kind::OctetString | Kind::Utf8String | Kind::AsciiString => {
                bail!("string fields have no numeric value")
            }
            Kind::Time | Kind::TimeSpan => bail!("use the fixed-point temporal decoder"),
        };
        let unscaled = self.scale == 1.0 && self.offset == 0.0;
        let value = match self.decimals.map(usize::from) {
            None if unscaled => raw.clone(),
            None => (number * self.scale + self.offset).to_string(),
            Some(places) if unscaled && matches!(self.kind, Kind::Signed | Kind::Unsigned) => {
                if places == 0 {
                    raw.clone()
                } else {
                    format!("{raw}.{}", "0".repeat(places))
                }
            }
            Some(places) => format!("{:.places$}", number * self.scale + self.offset),
        };
        Ok(Value {
            bits_hex: format!("{bits:X}"),
            raw,
            value,
            value_name: None,
            encoding_error: None,
        })
    }
}

pub fn parse_hex(text: &str) -> Result<Vec<u8>> {
    parse_hex_bounded(text, MAX_INPUT)
}

pub fn parse_hex_bounded(text: &str, max_bytes: usize) -> Result<Vec<u8>> {
    let text = text.trim().strip_prefix("0x").unwrap_or(text.trim());
    let clean: String = text
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '_')
        .collect();
    if clean.is_empty()
        || clean.len() > max_bytes.saturating_mul(2)
        || !clean.len().is_multiple_of(2)
        || !clean.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("enter 1–{max_bytes} bytes as pairs of hexadecimal digits");
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&clean[i..i + 2], 16)?))
        .collect()
}

pub fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02X}");
    }
    output
}

pub(crate) fn octet_sequence(bytes: &[u8]) -> String {
    let mut output = String::with_capacity((bytes.len() * 5).saturating_sub(1));
    for (index, byte) in bytes.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        let _ = write!(output, "0x{byte:02X}");
    }
    output
}

pub(crate) fn decode_padded_text(payload: &[u8], ascii: bool) -> Result<&str> {
    let end = payload
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(payload.len());
    ensure!(
        payload[end..].iter().all(|byte| *byte == 0),
        "nonzero string padding follows the first NUL"
    );
    let text = &payload[..end];
    ensure!(!ascii || text.is_ascii(), "text is not US-ASCII");
    std::str::from_utf8(text)
        .map_err(|error| anyhow!("invalid UTF-8 at byte {}", error.valid_up_to()))
}

pub(crate) fn extract_bits(payload: &[u8], bit_offset: u16, width: u16) -> Result<Vec<u8>> {
    let width = usize::from(width);
    let offset = usize::from(bit_offset);
    ensure!(
        width > 0 && offset + width <= payload.len() * 8,
        "member exceeds response width"
    );
    let mut bytes = vec![0; width.div_ceil(8)];
    let length = bytes.len();
    for bit in 0..width {
        let absolute = offset + bit;
        let value = (payload[payload.len() - 1 - absolute / 8] >> (absolute % 8)) & 1;
        bytes[length - 1 - bit / 8] |= value << (bit % 8);
    }
    Ok(bytes)
}

/// Copy the right-aligned value's low bits into a big-endian packed image.
pub(crate) fn insert_bits(
    payload: &mut [u8],
    bit_offset: u16,
    width: u16,
    value: &[u8],
) -> Result<()> {
    let width = usize::from(width);
    let offset = usize::from(bit_offset);
    ensure!(
        width > 0 && offset + width <= payload.len() * 8,
        "member exceeds image width"
    );
    ensure!(
        width <= value.len() * 8,
        "encoded member is shorter than its declared width"
    );
    let length = payload.len();
    for bit in 0..width {
        let absolute = offset + bit;
        let mask = 1 << (absolute % 8);
        let bit_value = (value[value.len() - 1 - bit / 8] >> (bit % 8)) & 1;
        let byte = &mut payload[length - 1 - absolute / 8];
        *byte = (*byte & !mask) | (bit_value << (absolute % 8));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporal_fields_format_device_time_and_log_exact_ticks() -> Result<()> {
        let mut field = Field {
            name: "Device time".into(),
            bits: 64,
            kind: Kind::Time,
            bit_offset: 8,
            ..Field::default()
        };
        let timestamp = 0x9DFF_4400_8000_0000u64;
        let mut image = [0; 9];
        image[..8].copy_from_slice(&timestamp.to_be_bytes());
        image[8] = 1;
        let value = field.decode(&image)?;
        assert_eq!(value.value, "1984-01-01 00:00:00.500 UTC");
        assert_eq!(value.bits_hex, "9DFF440080000000");
        assert_eq!(value.raw, timestamp.to_string());
        assert_eq!(field.csv_value(&image)?, value.raw);

        field.kind = Kind::TimeSpan;
        image[..8].copy_from_slice(&(-1i64).to_be_bytes());
        assert_eq!(field.decode(&image)?.value, "-00:00:00.000");
        assert_eq!(field.csv_value(&image)?, "-1");
        image[..8].copy_from_slice(&i64::MIN.to_be_bytes());
        assert_eq!(field.csv_value(&image)?, i64::MIN.to_string());
        assert!(field.csv_value(&image[..8]).is_err());
        assert!(
            Field {
                bit_offset: 1,
                ..field.clone()
            }
            .validate(9)
            .is_err()
        );
        assert!(
            Field {
                bits: 56,
                ..field.clone()
            }
            .validate(9)
            .is_err()
        );
        assert!(
            Field {
                byte_order: ByteOrder::LittleEndian,
                ..field.clone()
            }
            .validate(9)
            .is_err()
        );
        assert!(
            Field {
                scale: 0.5,
                ..field
            }
            .validate(9)
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn fixed_text_keeps_raw_bytes_and_distinguishes_empty_and_malformed_values() -> Result<()> {
        let mut field = Field {
            name: "Label".into(),
            kind: Kind::Utf8String,
            bits: 48,
            ..Field::default()
        };
        let payload = [0xC3, 0xA9, b'\n', b'"', 0, 0];
        let decoded = field.decode(&payload)?;
        assert_eq!(decoded.raw, "C3A90A220000");
        assert!(decoded.encoding_error.is_none() && !decoded.value.contains('\n'));
        assert_eq!(serde_json::from_str::<String>(&decoded.value)?, "é\n\"");
        let empty = field.decode(&[0; 6])?;
        assert_eq!(empty.value, "\"\"");
        assert!(empty.encoding_error.is_none());

        let malformed = field.decode(&[b'A', 0, b'B', 0, 0, 0])?;
        assert_eq!(malformed.raw, "410042000000");
        assert_eq!(
            malformed.encoding_error.as_deref(),
            Some("nonzero string padding follows the first NUL")
        );
        assert!(malformed.value.starts_with("Invalid StringT"));
        assert!(
            field
                .decode(&[0xC3, 0, 0, 0, 0, 0])?
                .encoding_error
                .is_some()
        );

        field.kind = Kind::AsciiString;
        assert_eq!(field.decode(b"ABC\0\0\0")?.value, "\"ABC\"");
        assert_eq!(
            field.decode(&payload)?.encoding_error.as_deref(),
            Some("text is not US-ASCII")
        );
        assert!(field.decode(&[0; 5]).is_err());
        Ok(())
    }

    #[test]
    fn octet_fields_preserve_wire_order_and_require_whole_bytes() -> Result<()> {
        let field = Field {
            name: "Device bytes".into(),
            kind: Kind::OctetString,
            bit_offset: 8,
            bits: 32,
            ..Field::default()
        };
        let payload = [0xAB, 0x00, 0x80, 0xFF, 0x00, 0xCD];
        let decoded = field.decode(&payload)?;
        assert_eq!(decoded.raw, "0080FF00");
        assert_eq!(decoded.bits_hex, decoded.raw);
        assert_eq!(decoded.value, "0x00,0x80,0xFF,0x00");
        assert!(field.read_bits(&payload).is_err());
        assert!(field.decode(&payload[..4]).is_err());
        for invalid in [
            Field {
                bit_offset: 7,
                ..field.clone()
            },
            Field {
                bits: 31,
                ..field.clone()
            },
            Field {
                byte_order: ByteOrder::LittleEndian,
                ..field.clone()
            },
            Field {
                scale: 2.0,
                ..field.clone()
            },
            Field {
                decimals: Some(0),
                ..field
            },
        ] {
            assert!(invalid.decode(&payload).is_err());
        }
        Ok(())
    }

    #[test]
    fn insert_bits_changes_only_the_selected_unaligned_bits() -> Result<()> {
        let mut image = [0xa5, 0x5a];
        insert_bits(&mut image, 3, 12, &[0xff, 0xef])?;
        assert_eq!(image, [0xff, 0x7a]);
        assert_eq!(extract_bits(&image, 3, 12)?, [0x0f, 0xef]);
        assert!(insert_bits(&mut image, 3, 12, &[0xff]).is_err());
        assert_eq!(image, [0xff, 0x7a]);
        Ok(())
    }

    #[test]
    fn signed_unaligned_big_endian_and_scaled() -> Result<()> {
        let field = Field {
            name: "position".into(),
            bits: 12,
            bit_offset: 3,
            kind: Kind::Signed,
            scale: 0.25,
            offset: 1.0,
            ..Field::default()
        };
        let value = field.decode(&[0x7F, 0xF0])?;
        assert_eq!((value.raw.as_str(), value.value.as_str()), ("-2", "0.5"));
        Ok(())
    }

    #[test]
    fn little_endian_and_full_width() -> Result<()> {
        let field = Field {
            name: "wide".into(),
            bits: 64,
            byte_order: ByteOrder::LittleEndian,
            ..Field::default()
        };
        assert_eq!(field.decode(&[255; 8])?.value, u64::MAX.to_string());
        assert_eq!(field.decode(&[0, 1, 0, 0, 0, 0, 0, 0])?.raw, "256");
        assert!(field.decode(&[0]).is_err());
        Ok(())
    }
}
