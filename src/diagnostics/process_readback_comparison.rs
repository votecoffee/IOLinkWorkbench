//! Compare saved readback bytes and fields under recorded connection and mapping evidence.

use super::{
    ProcessReadback, ProcessReadbackReview, ProcessReadbackSnapshot, ReadbackDirection,
    ReplyComparison,
};
use crate::{
    decode::{Field, Value},
    profile::Profile,
};
use anyhow::Result;
use serde::Serialize;
use std::{collections::BTreeMap, fmt::Write};

#[derive(Serialize)]
pub struct ProcessReadbackComparison<'a> {
    pub left: ProcessReadbackReview<'a>,
    pub right: ProcessReadbackReview<'a>,
    pub reply: ReplyComparison,
    pub byte_changes: Option<Vec<ReadbackByteChange>>,
    pub field_changes: Option<Vec<ReadbackFieldChange>>,
}

#[derive(Serialize)]
pub struct ReadbackByteChange {
    pub byte_offset: usize,
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub xor_mask: Option<u8>,
}

#[derive(Serialize)]
pub struct ReadbackFieldChange {
    pub name: String,
    pub bit_offset: u16,
    pub bits: u16,
    pub unit: String,
    pub left: ReadbackFieldValue,
    pub right: ReadbackFieldValue,
}

#[derive(Serialize)]
pub struct ReadbackFieldValue {
    pub bits_hex: String,
    pub raw: String,
    pub display: String,
}

impl From<&Value> for ReadbackFieldValue {
    fn from(value: &Value) -> Self {
        Self {
            bits_hex: value.bits_hex.clone(),
            raw: value.raw.clone(),
            display: value.display().into_owned(),
        }
    }
}

impl ProcessReadbackComparison<'_> {
    pub fn ok(&self) -> bool {
        self.field_changes.is_some()
    }

    pub fn summary_text(&self) -> String {
        let wire = match self.reply.same_wire_bytes {
            Some(true) => "match",
            Some(false) => "differ",
            None => "unavailable",
        };
        let mut text = format!(
            "Saved process readback comparison · left → right\n{} → {}\nProfiles · {} → {}\nWire bytes · {wire}\n{}",
            self.left.direction.label(),
            self.right.direction.label(),
            self.left.profile_name,
            self.right.profile_name,
            self.left.observation_kind,
        );
        if let Some(changes) = &self.byte_changes {
            for change in changes {
                let label = |byte: Option<u8>| {
                    byte.map_or_else(|| "absent".into(), |b| format!("0x{b:02X}"))
                };
                let _ = write!(
                    text,
                    "\nByte {} · {} → {}",
                    change.byte_offset,
                    label(change.left),
                    label(change.right),
                );
                if let Some(mask) = change.xor_mask {
                    let _ = write!(text, " · XOR 0x{mask:02X}");
                }
            }
        }
        if let Some(issue) = self.reply.interpretation_issue {
            let _ = write!(text, "\nTyped comparison · {issue}");
        }
        if let Some(changes) = &self.field_changes {
            if changes.is_empty() {
                text.push_str("\nMapped fields match");
            }
            for change in changes {
                let right = format!("{} {}", change.right.display, change.unit);
                let _ = write!(
                    text,
                    "\n{} · bit {} · {} → {} · bits 0x{} → 0x{}",
                    change.name,
                    change.bit_offset,
                    change.left.display,
                    right.trim_end(),
                    change.left.bits_hex,
                    change.right.bits_hex,
                );
            }
        }
        text
    }
}

type MappedFields<'a> = BTreeMap<&'a str, (&'a Field, &'a Value)>;

pub fn compare_process_readbacks<'a>(
    left: &'a ProcessReadbackSnapshot,
    right: &'a ProcessReadbackSnapshot,
) -> Result<ProcessReadbackComparison<'a>> {
    let (left_review, left_bytes) = left.review_with_bytes()?;
    let (right_review, right_bytes) = right.review_with_bytes()?;
    let byte_changes = left_bytes
        .as_deref()
        .zip(right_bytes.as_deref())
        .map(|(left, right)| compare_bytes(left, right));
    let left_fields = mapped_fields(left_review.decoded.as_ref());
    let right_fields = mapped_fields(right_review.decoded.as_ref());
    let context_issue = if left.direction != right.direction {
        Some("Saved process readback directions differ")
    } else if left.profile.connection.endpoint != right.profile.connection.endpoint {
        Some("Saved gateway endpoints differ")
    } else {
        None
    };
    let mapping_issue = if !left_review.ok() || !right_review.ok() {
        Some("One or both saved replies need decoding review")
    } else if left_fields.is_empty() || right_fields.is_empty() {
        Some("One or both saved replies have no decoded process fields")
    } else if !compatible_layout(left.direction, &left.profile, &right.profile)
        || !left_fields
            .iter()
            .map(|(name, (field, _))| (name, field))
            .eq(right_fields.iter().map(|(name, (field, _))| (name, field)))
    {
        Some("Saved process data mappings differ")
    } else {
        None
    };
    let mut reply = ReplyComparison::new(&left.read, &right.read, context_issue, None);
    reply.interpretation_issue = reply.interpretation_issue.or(mapping_issue);
    let field_changes = reply.interpretation_issue.is_none().then(|| {
        left_fields
            .values()
            .zip(right_fields.values())
            .filter(|((_, left), (_, right))| left != right)
            .map(|((field, left), (_, right))| ReadbackFieldChange {
                name: field.name.clone(),
                bit_offset: field.bit_offset,
                bits: field.bits,
                unit: field.unit.clone(),
                left: (*left).into(),
                right: (*right).into(),
            })
            .collect()
    });
    Ok(ProcessReadbackComparison {
        left: left_review,
        right: right_review,
        reply,
        byte_changes,
        field_changes,
    })
}

fn compare_bytes(left: &[u8], right: &[u8]) -> Vec<ReadbackByteChange> {
    (0..left.len().max(right.len()))
        .filter_map(|byte_offset| {
            let left = left.get(byte_offset).copied();
            let right = right.get(byte_offset).copied();
            (left != right).then_some(ReadbackByteChange {
                byte_offset,
                left,
                right,
                xor_mask: left.zip(right).map(|(a, b)| a ^ b),
            })
        })
        .collect()
}

fn mapped_fields(decoded: Option<&ProcessReadback>) -> MappedFields<'_> {
    let Some(decoded) = decoded else {
        return BTreeMap::new();
    };
    if let Some(output) = &decoded.iodd_output {
        output
            .fields
            .iter()
            .map(|item| (item.field.name.as_str(), (&item.field, &item.value)))
            .collect()
    } else {
        decoded
            .input_fields
            .iter()
            .map(|item| (item.field.name.as_str(), (&item.field, &item.value)))
            .collect()
    }
}

fn compatible_layout(direction: ReadbackDirection, left: &Profile, right: &Profile) -> bool {
    fn input_condition(profile: &Profile) -> Option<&str> {
        profile.iodd.as_ref()?.condition.as_deref()
    }
    match direction {
        ReadbackDirection::Input => {
            left.input_bytes == right.input_bytes
                && left.input_encoding() == right.input_encoding()
                && input_condition(left) == input_condition(right)
        }
        ReadbackDirection::Output => left.output_layout() == right.output_layout(),
    }
}
