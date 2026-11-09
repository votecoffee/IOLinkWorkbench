use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_INPUT: usize = 32;

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
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Field {
    pub name: String,
    pub bit_offset: u16,
    pub bits: u8,
    pub byte_order: ByteOrder,
    pub kind: Kind,
    pub scale: f64,
    pub offset: f64,
    pub unit: String,
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
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Value {
    pub bits_hex: String,
    pub raw: String,
    pub value: String,
}

impl Field {
    pub fn validate(&self, input_bytes: usize) -> Result<()> {
        ensure!(!self.name.trim().is_empty(), "field name is empty");
        ensure!(
            !self.name.contains(['\r', '\n']),
            "field name contains a line break"
        );
        ensure!(
            (1..=64).contains(&self.bits),
            "{}: width must be 1–64",
            self.name
        );
        ensure!(
            usize::from(self.bit_offset) + usize::from(self.bits) <= input_bytes * 8,
            "{}: field extends outside the payload",
            self.name
        );
        ensure!(
            self.kind != Kind::Bool || self.bits == 1,
            "{}: bool needs one bit",
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
            self.kind != Kind::Bool || (self.scale == 1.0 && self.offset == 0.0),
            "{}: bool does not support scaling",
            self.name
        );
        Ok(())
    }

    pub fn read_bits(&self, payload: &[u8]) -> Result<u64> {
        self.validate(payload.len())?;
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

    pub fn decode(&self, payload: &[u8]) -> Result<Value> {
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
        };
        let value = if self.scale == 1.0 && self.offset == 0.0 {
            raw.clone()
        } else {
            (number * self.scale + self.offset).to_string()
        };
        Ok(Value {
            bits_hex: format!("{bits:X}"),
            raw,
            value,
        })
    }
}

pub fn parse_hex(text: &str) -> Result<Vec<u8>> {
    let text = text.trim().strip_prefix("0x").unwrap_or(text.trim());
    let clean: String = text
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '_')
        .collect();
    if clean.is_empty()
        || clean.len() > MAX_INPUT * 2
        || !clean.len().is_multiple_of(2)
        || !clean.is_ascii()
    {
        bail!("enter 1–{MAX_INPUT} bytes as pairs of hexadecimal digits");
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| Ok(u8::from_str_radix(&clean[i..i + 2], 16)?))
        .collect()
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02X}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

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
