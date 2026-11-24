//! Apply the same recorded bytes independently to two saved mappings.
use super::Profile;
use crate::decode::{Value, hex};
use serde::Serialize;
use std::{collections::BTreeMap, fmt::Write};

#[derive(Serialize)]
pub struct PayloadComparison<'a> {
    pub raw_hex: String,
    pub left: PayloadSide<'a>,
    pub right: PayloadSide<'a>,
    pub differences: Option<PayloadDifferences<'a>>,
}

#[derive(Serialize)]
pub struct PayloadSide<'a> {
    pub fields: Option<Vec<DecodedField<'a>>>,
    pub decode_error: Option<String>,
    pub wire_warning: Option<&'static str>,
}

#[derive(Serialize)]
pub struct DecodedField<'a> {
    pub name: &'a str,
    pub unit: &'a str,
    pub value: Value,
}

#[derive(Serialize)]
pub struct PayloadDifferences<'a> {
    pub compared_fields: usize,
    pub changed_fields: Vec<&'a str>,
}

impl<'a> PayloadSide<'a> {
    fn decode(profile: &'a Profile, bytes: &[u8]) -> Self {
        let (fields, decode_error) = match profile.decode(bytes) {
            Ok(decoded) => (
                Some(
                    decoded
                        .into_iter()
                        .map(|(field, _, value)| DecodedField {
                            name: &field.name,
                            unit: &field.unit,
                            value,
                        })
                        .collect(),
                ),
                None,
            ),
            Err(error) => (None, Some(error.to_string())),
        };
        Self {
            fields,
            decode_error,
            wire_warning: profile.input_encoding_warning(bytes),
        }
    }

    fn ok(&self) -> bool {
        self.fields.as_ref().is_some_and(|fields| {
            fields
                .iter()
                .all(|field| field.value.encoding_error.is_none())
        })
    }

    fn write_issues(&self, output: &mut String, label: &str) {
        for issue in [self.decode_error.as_deref(), self.wire_warning]
            .into_iter()
            .flatten()
        {
            let _ = write!(output, "\n{label} · {issue}");
        }
        if let Some(fields) = &self.fields {
            for field in fields {
                if let Some(error) = &field.value.encoding_error {
                    let _ = write!(output, "\n{label} · {} · {error}", field.name);
                }
            }
        }
    }
}

impl DecodedField<'_> {
    fn text(&self) -> String {
        let engineering = format!("{} {}", self.value.display(), self.unit);
        format!(
            "{} (raw {}, bits 0x{})",
            engineering.trim_end(),
            self.value.raw,
            self.value.bits_hex
        )
    }
}

impl<'a> PayloadComparison<'a> {
    pub(super) fn new(left: &'a Profile, right: &'a Profile, bytes: &[u8]) -> Self {
        let left = PayloadSide::decode(left, bytes);
        let right = PayloadSide::decode(right, bytes);
        let differences = left
            .fields
            .as_deref()
            .zip(right.fields.as_deref())
            .map(|(left, right)| compare_values(left, right));
        Self {
            raw_hex: hex(bytes),
            left,
            right,
            differences,
        }
    }

    pub fn ok(&self) -> bool {
        self.left.ok() && self.right.ok()
    }

    pub(super) fn write_summary(&self, output: &mut String) {
        let _ = write!(output, "\n\nRecorded payload · {}", self.raw_hex);
        self.left.write_issues(output, "First profile");
        self.right.write_issues(output, "Second profile");
        let Some(differences) = &self.differences else {
            output.push_str("\nDecoded value comparison unavailable");
            return;
        };
        let _ = write!(
            output,
            "\nCommon fields decoded on both sides · {}\nChanged decoded values · {}",
            differences.compared_fields,
            differences.changed_fields.len()
        );
        for name in &differences.changed_fields {
            let find = |side: &PayloadSide<'_>| {
                side.fields
                    .as_ref()?
                    .iter()
                    .find(|field| &field.name == name)
                    .map(DecodedField::text)
            };
            if let Some((left, right)) = find(&self.left).zip(find(&self.right)) {
                let _ = write!(output, "\n{name} · {left} → {right}");
            }
        }
    }
}

fn compare_values<'a>(
    left: &[DecodedField<'a>],
    right: &[DecodedField<'a>],
) -> PayloadDifferences<'a> {
    let right: BTreeMap<_, _> = right.iter().map(|field| (field.name, field)).collect();
    let mut compared_fields = 0;
    let mut changed_fields = Vec::new();
    for a in left {
        if let Some(b) = right.get(a.name)
            && a.value.encoding_error.is_none()
            && b.value.encoding_error.is_none()
        {
            compared_fields += 1;
            if a.value != b.value || a.unit != b.unit {
                changed_fields.push(a.name);
            }
        }
    }
    PayloadDifferences {
        compared_fields,
        changed_fields,
    }
}
