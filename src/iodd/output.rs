use super::ProcessDataLayout;
use crate::decode::{Field, Value};
use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Serialize)]
pub struct DecodedOutput {
    pub layout_id: String,
    pub bit_length: usize,
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recorded_condition_confirmation: Option<bool>,
    pub fields: Vec<DecodedOutputField>,
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
                        value: field.decode(payload)?,
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
            needs_input: self.needs_input.clone(),
            decode_error,
        }
    }
}
