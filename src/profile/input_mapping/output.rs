//! Compare IODD process output layouts and inspect recorded bytes offline.
use super::{
    Encoding, FieldChange, MappedField, PayloadComparison, Profile, Selection, compare_fields,
    mapping_source_texts, write_field_changes,
};
use crate::decode::{Kind, parse_hex};
use anyhow::Result;
use serde::Serialize;
use std::fmt::Write;

#[derive(Serialize)]
pub struct OutputComparison<'a> {
    pub left: OutputMapping<'a>,
    pub right: OutputMapping<'a>,
    pub output_width_changed: bool,
    pub output_bit_length_changed: bool,
    pub output_encoding_changed: bool,
    pub selection_changed: bool,
    pub common_field_order_changed: bool,
    pub review_notes_changed: bool,
    pub changes: Vec<FieldChange<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<PayloadComparison<'a>>,
}

#[derive(Serialize)]
pub struct OutputMapping<'a> {
    pub profile_name: &'a str,
    pub output_bytes: Option<usize>,
    pub bit_length: Option<usize>,
    pub encoding: Encoding,
    pub selection: Option<Selection<'a>>,
    pub fields: Vec<MappedField<'a>>,
    pub needs_input: &'a [String],
}

impl<'a> From<&'a Profile> for OutputMapping<'a> {
    fn from(profile: &'a Profile) -> Self {
        let saved = profile.iodd.as_ref().zip(profile.output_layout());
        let Some((iodd, layout)) = saved else {
            return Self {
                profile_name: &profile.name,
                output_bytes: None,
                bit_length: None,
                encoding: Encoding::NoOutput,
                selection: None,
                fields: Vec::new(),
                needs_input: &[],
            };
        };
        let encoding = if layout.standalone_boolean {
            Encoding::StandaloneBoolean
        } else if layout.padded_integer
            && let Some(field) = layout.fields.first()
        {
            Encoding::PaddedInteger {
                bits: field.bits,
                signed: field.kind == Kind::Signed,
            }
        } else if layout.packed {
            Encoding::Packed
        } else {
            Encoding::Standard
        };
        Self {
            profile_name: &profile.name,
            output_bytes: Some(layout.bytes()),
            bit_length: Some(layout.bit_length),
            encoding,
            selection: Some(Selection {
                variant: &iodd.variant,
                layout: &layout.id,
                condition: layout.condition.as_deref(),
                condition_confirmed: layout
                    .condition
                    .as_ref()
                    .map(|_| profile.confirm_iodd_condition),
            }),
            fields: layout
                .fields
                .iter()
                .map(|definition| MappedField {
                    origin: "IODD",
                    definition,
                })
                .collect(),
            needs_input: &layout.needs_input,
        }
    }
}

pub fn compare_outputs<'a>(
    left: &'a Profile,
    right: &'a Profile,
    payload: Option<&str>,
) -> Result<OutputComparison<'a>> {
    let payload = payload
        .map(parse_hex)
        .transpose()?
        .map(|bytes| PayloadComparison::for_output(left, right, &bytes));
    let left = OutputMapping::from(left);
    let right = OutputMapping::from(right);
    let (changes, common_field_order_changed) = compare_fields(&left.fields, &right.fields);
    Ok(OutputComparison {
        output_width_changed: left.output_bytes != right.output_bytes,
        output_bit_length_changed: left.bit_length != right.bit_length,
        output_encoding_changed: left.encoding != right.encoding,
        selection_changed: left.selection != right.selection,
        common_field_order_changed,
        review_notes_changed: left.needs_input != right.needs_input,
        left,
        right,
        changes,
        payload,
    })
}

fn write_width(
    text: &mut String,
    label: &str,
    left: Option<usize>,
    right: Option<usize>,
    unit: &str,
) {
    let display = |value: Option<usize>| match value {
        Some(value) => format!("{value} {unit}"),
        None => "absent".into(),
    };
    let _ = write!(text, "\n{label} · {} → {}", display(left), display(right));
}

impl OutputComparison<'_> {
    pub fn ok(&self) -> bool {
        self.payload.as_ref().is_none_or(PayloadComparison::ok)
    }

    pub fn source_texts(&self) -> Result<[String; 2]> {
        mapping_source_texts(&self.left, &self.right, self.payload.as_ref())
    }

    pub fn summary_text(&self) -> String {
        let mut text = format!("{} → {}", self.left.profile_name, self.right.profile_name);
        if self.output_width_changed {
            write_width(
                &mut text,
                "Output width",
                self.left.output_bytes,
                self.right.output_bytes,
                "bytes",
            );
        }
        if self.output_bit_length_changed {
            write_width(
                &mut text,
                "IODD bit length",
                self.left.bit_length,
                self.right.bit_length,
                "bits",
            );
        }
        let context = [
            (
                self.selection_changed,
                "Saved IODD output selection or condition changed",
            ),
            (self.output_encoding_changed, "Output encoding changed"),
            (
                self.common_field_order_changed,
                "Order of common output fields changed",
            ),
            (self.review_notes_changed, "Output review notes changed"),
        ];
        for (changed, description) in context {
            if changed {
                let _ = write!(text, "\n{description}");
            }
        }
        write_field_changes(&mut text, &self.changes);
        if self.left.output_bytes.is_none() && self.right.output_bytes.is_none() {
            text.push_str("\nBoth profiles lack a saved IODD output layout");
        } else if !self.output_width_changed
            && !self.output_bit_length_changed
            && self.changes.is_empty()
            && context.iter().all(|(changed, _)| !changed)
        {
            text.push_str("\nSaved output mappings match");
        }
        if let Some(payload) = &self.payload {
            payload.write_summary(&mut text);
        }
        text
    }
}
