use super::{Access, DisplayValue, Parameter};
use crate::decode::hex;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DecodedParameter {
    pub raw_hex: String,
    pub raw_value: Option<String>,
    pub displays: Vec<DisplayValue>,
    pub choice_name: Option<String>,
    pub decode_error: Option<String>,
    pub record_items: Vec<RecordValue>,
}

#[derive(Debug, Serialize)]
pub struct RecordValue {
    pub subindex: u8,
    pub name: String,
    pub bit_offset: u16,
    pub value: DecodedParameter,
}

impl DecodedParameter {
    pub(super) fn error(payload: &[u8], error: impl ToString) -> Self {
        Self {
            raw_hex: hex(payload),
            raw_value: None,
            displays: Vec::new(),
            choice_name: None,
            decode_error: Some(error.to_string()),
            record_items: Vec::new(),
        }
    }

    pub fn text(&self) -> String {
        let mut text = format!("Raw bytes {}", self.raw_hex);
        if let Some(value) = &self.raw_value {
            text.push_str(&format!("\nRaw value {value}"));
        }
        if let Some(name) = &self.choice_name {
            text.push_str(&format!(" · {name}"));
        }
        for display in &self.displays {
            text.push_str(&format!(
                "\nDisplay {} · {} {}",
                display.id, display.value, display.unit
            ));
        }
        if let Some(error) = &self.decode_error {
            text.push_str(&format!("\nDecode review · {error}"));
        }
        for item in &self.record_items {
            text.push_str(&format!(
                "\n\n{} · subindex {} · bit {}\n{}",
                item.name,
                item.subindex,
                item.bit_offset,
                item.value.text()
            ));
        }
        text
    }
}

impl Parameter {
    /// Decode an acyclic payload without any transport access. Raw bytes survive errors.
    pub fn decode(&self, payload: &[u8]) -> DecodedParameter {
        if payload.is_empty() || payload.len() > 232 {
            return DecodedParameter::error(payload, "parameter payload must contain 1–232 bytes");
        }
        let raw_value = match self.decode_raw(payload) {
            Ok(value) => value,
            Err(error) => return DecodedParameter::error(payload, error),
        };
        let mut value = DecodedParameter {
            raw_hex: hex(payload),
            raw_value,
            displays: Vec::new(),
            choice_name: None,
            decode_error: None,
            record_items: Vec::new(),
        };
        value.choice_name = value
            .raw_value
            .as_deref()
            .and_then(|raw| self.choices.iter().find(|choice| choice.value == raw))
            .map(|choice| choice.name.clone());
        match self.decode_displays(payload) {
            Ok(displays) => value.displays = displays,
            Err(error) => value.decode_error = Some(error.to_string()),
        }
        if let Some(record) = &self.record {
            if self.bits.map(|bits| usize::from(bits).div_ceil(8)) != Some(payload.len()) {
                return DecodedParameter::error(
                    payload,
                    "record response width differs from the IODD",
                );
            }
            for item in &record.items {
                let decoded = match item.extract(payload) {
                    Ok(bytes) if item.parameter.access == Access::WriteOnly => {
                        DecodedParameter::error(
                            &bytes,
                            "write-only record item; typed interpretation skipped",
                        )
                    }
                    Ok(bytes) => item.parameter.decode(&bytes),
                    Err(error) => DecodedParameter::error(&[], error),
                };
                value.record_items.push(RecordValue {
                    subindex: item.subindex,
                    name: item.parameter.name.clone(),
                    bit_offset: item.bit_offset,
                    value: decoded,
                });
            }
        }
        value
    }
}
