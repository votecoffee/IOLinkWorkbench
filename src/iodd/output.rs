use super::{ProcessDataLayout, integer_padding_matches};
use crate::decode::{Field, Value};
use anyhow::{Result, bail, ensure};
use serde::Serialize;

#[derive(Serialize)]
pub struct DecodedOutput {
    pub layout_id: String,
    pub bit_length: usize,
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_condition_confirmation: Option<bool>,
    pub fields: Vec<DecodedOutputField>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub standalone_boolean: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub padded_integer: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding_warning: Option<String>,
    pub needs_input: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decode_error: Option<String>,
}

#[derive(Serialize)]
pub struct DecodedOutputField {
    pub field: Field,
    pub value: Value,
}

impl ProcessDataLayout {
    pub(crate) fn check_output_width(&self, received_bytes: usize) -> Result<()> {
        let width = self.bytes();
        ensure!(
            received_bytes == width,
            "IODD output layout {} requires {width} bytes ({} bits), received {received_bytes}",
            self.id,
            self.bit_length
        );
        Ok(())
    }

    pub(crate) fn check_proposed_output(&self, payload: &[u8]) -> Result<()> {
        self.check_output_width(payload.len())?;
        ensure!(
            !self.standalone_boolean || matches!(payload, [0x00] | [0xFF]),
            "standalone Boolean output requires 00 (false) or FF (true)"
        );
        if self.padded_integer {
            ensure!(
                integer_padding_matches(&self.fields[0], payload),
                "standalone integer output requires canonical sign or zero padding"
            );
        }
        Ok(())
    }

    pub(crate) fn output_encoding_warning(&self, payload: &[u8]) -> Option<String> {
        if self.standalone_boolean
            && let [byte @ 0x01..=0xFE] = payload
        {
            return Some(format!(
                "Standalone Boolean byte {byte:02X} is noncanonical. Workbench interprets it as true."
            ));
        }
        if self.padded_integer && !integer_padding_matches(&self.fields[0], payload) {
            return Some(
                "Standalone integer byte padding is noncanonical. The displayed value uses only the declared data bits.".into(),
            );
        }
        None
    }

    pub(crate) fn decode_output_field(&self, field: &Field, payload: &[u8]) -> Result<Value> {
        if !self.standalone_boolean {
            return field.decode(payload);
        }
        let [byte] = payload else {
            bail!("standalone Boolean output requires one byte");
        };
        let value = (*byte != 0).to_string();
        Ok(Value {
            bits_hex: format!("{byte:02X}"),
            raw: value.clone(),
            value,
        })
    }

    /// A recorded condition confirmation is used for a live observation.
    /// Offline callers pass None to interpret bytes under the selected layout.
    pub fn decode_output(
        &self,
        payload: &[u8],
        recorded_confirmation: Option<bool>,
    ) -> DecodedOutput {
        let recorded_condition_confirmation = self.condition.as_ref().and(recorded_confirmation);
        let result = (|| -> Result<Vec<_>> {
            self.validate()?;
            self.check_output_width(payload.len())?;
            ensure!(
                recorded_condition_confirmation != Some(false),
                "Confirm the active IODD layout condition before interpreting the observed image."
            );
            self.fields
                .iter()
                .map(|field| {
                    Ok(DecodedOutputField {
                        field: field.clone(),
                        value: self.decode_output_field(field, payload)?,
                    })
                })
                .collect()
        })();
        let (fields, decode_error) = match result {
            Ok(fields) => (fields, None),
            Err(error) => (Vec::new(), Some(format!("{error:#}"))),
        };
        DecodedOutput {
            layout_id: self.id.clone(),
            bit_length: self.bit_length,
            condition: self.condition.clone(),
            recorded_condition_confirmation,
            fields,
            standalone_boolean: self.standalone_boolean,
            padded_integer: self.padded_integer,
            encoding_warning: decode_error
                .is_none()
                .then(|| self.output_encoding_warning(payload))
                .flatten(),
            needs_input: self.needs_input.clone(),
            decode_error,
        }
    }
}
