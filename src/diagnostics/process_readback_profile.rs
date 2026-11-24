//! Preview an existing process readback under a revised mapping, offline.
use super::{
    ProcessReadback, ProcessReadbackReview, ProcessReadbackSnapshot, ReadbackDirection,
    ReadbackFieldValue, process_readback::decode_bytes, process_readback_comparison::mapped_fields,
};
use crate::profile::{Profile, input_mapping};
use anyhow::Result;
use serde::Serialize;
use std::fmt::Write;

#[derive(Serialize)]
pub struct ReadbackProfileComparison<'a> {
    pub original: ProcessReadbackReview<'a>,
    pub revised_profile: &'a str,
    pub revised_identity_check: Option<String>,
    pub revised: Option<ProcessReadback>,
    pub mapping: ReadbackMapping<'a>,
    pub value_differences: Option<ReadbackValueDifferences>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum ReadbackMapping<'a> {
    Input(input_mapping::Comparison<'a>),
    Output(input_mapping::OutputComparison<'a>),
}

#[derive(Serialize)]
pub struct ReadbackValueDifferences {
    pub compared_fields: usize,
    pub changes: Vec<ReadbackProfileFieldChange>,
}

#[derive(Serialize)]
pub struct ReadbackProfileFieldChange {
    pub name: String,
    pub left: ReadbackFieldValue,
    pub right: ReadbackFieldValue,
    pub left_unit: String,
    pub right_unit: String,
}

pub fn compare_process_readback_profile<'a>(
    snapshot: &'a ProcessReadbackSnapshot,
    candidate: &'a Profile,
) -> Result<ReadbackProfileComparison<'a>> {
    candidate.validate()?;
    let (original, bytes) = snapshot.review_with_bytes()?;
    let revised = bytes
        .as_deref()
        .map(|bytes| decode_bytes(snapshot.direction, Some(candidate), bytes));
    let revised_identity_check = snapshot.read.observation.as_ref().map(|observation| {
        match candidate.expected_identity.verify(&observation.identity) {
            Ok(status) => status.to_owned(),
            Err(error) => error.to_string(),
        }
    });
    let mapping = match snapshot.direction {
        ReadbackDirection::Input => {
            ReadbackMapping::Input(input_mapping::compare(&snapshot.profile, candidate))
        }
        ReadbackDirection::Output => ReadbackMapping::Output(input_mapping::compare_outputs(
            &snapshot.profile,
            candidate,
            None,
        )?),
    };
    let value_differences = compare_values(original.decoded.as_ref(), revised.as_ref());
    Ok(ReadbackProfileComparison {
        original,
        revised_profile: &candidate.name,
        revised_identity_check,
        revised,
        mapping,
        value_differences,
    })
}

fn compare_values(
    left: Option<&ProcessReadback>,
    right: Option<&ProcessReadback>,
) -> Option<ReadbackValueDifferences> {
    let left = mapped_fields(left);
    let right = mapped_fields(right);
    if left.is_empty() || right.is_empty() {
        return None;
    }
    let mut compared_fields = 0;
    let mut changes = Vec::new();
    for (name, (left_field, left_value)) in left {
        if let Some((right_field, right_value)) = right.get(name).copied()
            && left_value.encoding_error.is_none()
            && right_value.encoding_error.is_none()
        {
            compared_fields += 1;
            if left_value != right_value || left_field.unit != right_field.unit {
                changes.push(ReadbackProfileFieldChange {
                    name: name.into(),
                    left: left_value.into(),
                    right: right_value.into(),
                    left_unit: left_field.unit.clone(),
                    right_unit: right_field.unit.clone(),
                });
            }
        }
    }
    Some(ReadbackValueDifferences {
        compared_fields,
        changes,
    })
}

impl ReadbackProfileComparison<'_> {
    pub fn ok(&self) -> bool {
        self.original.ok()
            && self.revised.as_ref().is_some_and(ProcessReadback::ok)
            && self.value_differences.is_some()
    }

    pub fn summary_text(&self) -> String {
        let read = self.original.read;
        let mut text = format!(
            "Saved reply under a revised profile · {}\n{}\nHost read completed {}",
            self.original.direction.label(),
            self.original.observation_kind,
            read.host_read_utc
        );
        match read.raw_hex.as_deref() {
            Some("") => text.push_str("\nRaw reply · Empty"),
            Some(hex) => {
                let _ = write!(text, "\nRaw reply · {hex}");
            }
            None => {
                if let Some(error) = &read.read_error {
                    let _ = write!(text, "\nSaved read error · {error}");
                }
            }
        }
        if let Some(check) = &self.revised_identity_check {
            let _ = write!(text, "\nRevised profile identity · {check}");
        }
        write_decoding_issues(
            &mut text,
            "Snapshot profile",
            self.original.decoded.as_ref(),
        );
        write_decoding_issues(&mut text, "Revised profile", self.revised.as_ref());
        let mapping = match &self.mapping {
            ReadbackMapping::Input(report) => report.summary_text(),
            ReadbackMapping::Output(report) => report.summary_text(),
        };
        let _ = write!(text, "\n\n{mapping}");
        if let Some(differences) = &self.value_differences {
            let _ = write!(
                text,
                "\nCommon decoded fields · {}\nChanged interpretations · {}",
                differences.compared_fields,
                differences.changes.len()
            );
            for field in &differences.changes {
                let left = format!("{} {}", field.left.display, field.left_unit);
                let right = format!("{} {}", field.right.display, field.right_unit);
                let _ = write!(
                    text,
                    "\n{} · {} → {} · bits 0x{} → 0x{}",
                    field.name,
                    left.trim_end(),
                    right.trim_end(),
                    field.left.bits_hex,
                    field.right.bits_hex,
                );
            }
        } else {
            text.push_str("\nDecoded field comparison unavailable");
        }
        text
    }

    pub fn source_texts(&self) -> Result<[String; 2]> {
        let [original_mapping, revised_mapping] = match &self.mapping {
            ReadbackMapping::Input(report) => report.source_texts()?,
            ReadbackMapping::Output(report) => report.source_texts()?,
        };
        let original = self.original.text();
        let revised = self
            .revised
            .as_ref()
            .map_or_else(|| "Saved reply unavailable".into(), ProcessReadback::text);
        Ok([
            format!("{original}\n\nSaved mapping\n{original_mapping}"),
            format!("{revised}\n\nRevised mapping\n{revised_mapping}"),
        ])
    }
}

fn write_decoding_issues(text: &mut String, label: &str, decoded: Option<&ProcessReadback>) {
    let Some(decoded) = decoded else {
        return;
    };
    let output = decoded.iodd_output.as_ref();
    for issue in [
        decoded.mapping_note,
        decoded.decode_error.as_deref(),
        decoded.input_encoding_warning,
        output.and_then(|output| output.decode_error.as_deref()),
        output.and_then(|output| output.encoding_warning.as_deref()),
    ]
    .into_iter()
    .flatten()
    {
        let _ = write!(text, "\n{label} · {issue}");
    }
    for (name, (_, value)) in mapped_fields(Some(decoded)) {
        if let Some(error) = &value.encoding_error {
            let _ = write!(text, "\n{label} · {name} · {error}");
        }
    }
}
