use crate::{
    decode::{Field, MAX_INPUT, Value},
    iodd::ProcessDataLayout,
};
use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Serialize)]
pub struct ProcessOutputReview {
    pub layout_id: String,
    pub bit_length: usize,
    pub condition: Option<String>,
    pub fields: Vec<OutputFieldReview>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub standalone_boolean: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_encoding_warning: Option<String>,
    pub changed_unmapped_bits: Vec<usize>,
    pub needs_input: Vec<String>,
}

#[derive(Serialize)]
pub struct OutputFieldReview {
    pub field: Field,
    pub expected: Value,
    pub proposed: Value,
}

impl ProcessOutputReview {
    pub(super) fn new(layout: &ProcessDataLayout, before: &[u8], after: &[u8]) -> Result<Self> {
        let width = layout.bytes();
        layout.check_output_width(before.len())?;
        layout.check_proposed_output(after)?;
        let mut mapped = [false; MAX_INPUT * 8];
        let mut fields = Vec::with_capacity(layout.fields.len());
        for field in &layout.fields {
            let start = usize::from(field.bit_offset);
            let end = start + usize::from(field.bits);
            mapped[start..end].fill(true);
            fields.push(OutputFieldReview {
                field: field.clone(),
                expected: layout.decode_output_field(field, before)?,
                proposed: layout.decode_output_field(field, after)?,
            });
        }
        let mut changed_unmapped_bits = Vec::new();
        if !layout.standalone_boolean {
            for (bit, mapped) in mapped.iter().enumerate().take(width * 8) {
                let byte = width - 1 - bit / 8;
                if (before[byte] ^ after[byte]) & (1 << (bit % 8)) == 0 {
                    continue;
                }
                ensure!(
                    bit < layout.bit_length,
                    "process output changes padding outside the declared IODD width at bit {bit}"
                );
                if !mapped {
                    changed_unmapped_bits.push(bit);
                }
            }
        }
        Ok(Self {
            layout_id: layout.id.clone(),
            bit_length: layout.bit_length,
            condition: layout.condition.clone(),
            fields,
            standalone_boolean: layout.standalone_boolean,
            expected_encoding_warning: layout.output_encoding_warning(before),
            changed_unmapped_bits,
            needs_input: layout.needs_input.clone(),
        })
    }
}
