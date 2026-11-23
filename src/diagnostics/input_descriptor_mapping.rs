//! Build host-only input mappings from a recorded whole PDInputDescriptor reply.
use super::{DescriptorDirection, DescriptorStatus, decode_process_descriptor};
use crate::{
    decode::{ByteOrder, Field, Kind, MAX_INPUT},
    profile::Profile,
};
use anyhow::{Result, bail, ensure};
use serde::Serialize;
use std::{collections::HashSet, fmt::Write};

#[derive(Debug, PartialEq, Serialize)]
pub struct InputDescriptorPlan {
    pub profile_name: String,
    pub layout_id: Option<String>,
    pub input_bytes: usize,
    pub raw_hex: String,
    pub additions: Vec<Field>,
    pub skipped_entries: Vec<SkippedEntry>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct SkippedEntry {
    pub entry: usize,
    pub datatype: &'static str,
    pub bit_offset: u16,
    pub bits: u16,
    pub reason: String,
}

impl InputDescriptorPlan {
    pub fn text(&self) -> String {
        let mut text = format!(
            "Suggested manual fields · {}\nInput width · {} bytes",
            self.additions.len(),
            self.input_bytes
        );
        for field in &self.additions {
            let _ = write!(
                text,
                "\n{} · {:?} · width {} · offset {}",
                field.name, field.kind, field.bits, field.bit_offset
            );
        }
        for entry in &self.skipped_entries {
            let _ = write!(
                text,
                "\nEntry {} · {} · width {} · offset {} · {}",
                entry.entry, entry.datatype, entry.bits, entry.bit_offset, entry.reason
            );
        }
        text
    }

    pub fn apply(&self, profile: &Profile) -> Result<Profile> {
        ensure!(
            *self == plan_input_descriptor(profile, &self.raw_hex)?,
            "Input mapping context changed. Review the suggestions again."
        );
        ensure!(
            !self.additions.is_empty(),
            "There are no manual fields to add"
        );
        let mut candidate = profile.clone();
        candidate.fields.extend_from_slice(&self.additions);
        candidate.validate()?;
        Ok(candidate)
    }
}

pub fn plan_input_descriptor(profile: &Profile, hex: &str) -> Result<InputDescriptorPlan> {
    profile.require_process_input()?;
    ensure!(
        profile.input_bytes <= MAX_INPUT,
        "Invalid input width in the profile"
    );
    let report = decode_process_descriptor(DescriptorDirection::Input, None, hex)?;
    ensure!(
        report.status != DescriptorStatus::InvalidReply,
        "{}",
        report.review.join(". ")
    );

    let input_bits = profile.input_bytes * 8;
    let mut occupied = [false; MAX_INPUT * 8];
    let mut names = HashSet::new();
    for field in profile
        .fields
        .iter()
        .chain(profile.iodd.iter().flat_map(|iodd| &iodd.fields))
    {
        names.insert(field.name.as_str());
        let first = usize::from(field.bit_offset);
        let end = (first + usize::from(field.bits)).min(input_bits);
        for bit in first..end {
            // Normalize existing little-endian offsets to the IODD coordinate system.
            let wire_bit = match field.byte_order {
                ByteOrder::BigEndian => bit,
                ByteOrder::LittleEndian => (profile.input_bytes - 1 - bit / 8) * 8 + bit % 8,
            };
            occupied[wire_bit] = true;
        }
    }

    let mut additions = Vec::new();
    let mut skipped_entries = Vec::new();
    for (index, entry) in report.entries.iter().enumerate() {
        let fields = (|| -> Result<Vec<Field>> {
            let end = entry.bit_offset + entry.bits;
            let first = usize::from(entry.bit_offset);
            let end_index = usize::from(end);
            ensure!(
                end_index <= input_bits,
                "Entry extends beyond the profile input width"
            );
            ensure!(
                !occupied[first..end_index].iter().any(|bit| *bit),
                "Entry overlaps an existing input mapping"
            );
            let kind = match entry.datatype_code {
                0 => Kind::OctetString,
                1 => Kind::Bool,
                2 => Kind::Unsigned,
                3 => Kind::Signed,
                4 => Kind::Float32,
                5 => bail!("StringT character encoding requires a manual mapping"),
                6 => Kind::Time,
                7 => Kind::TimeSpan,
                _ => bail!("Datatype requires a manual mapping"),
            };
            let (count, bits) = if kind == Kind::Bool {
                (entry.bits, 1)
            } else {
                (1, entry.bits)
            };
            (entry.bit_offset..entry.bit_offset + count)
                .map(|bit_offset| {
                    let field = Field {
                        name: format!("Input bit {bit_offset}"),
                        bit_offset,
                        bits,
                        kind,
                        ..Field::default()
                    };
                    ensure!(
                        !names.contains(field.name.as_str()),
                        "Suggested field name already exists"
                    );
                    field.validate(profile.input_bytes)?;
                    Ok(field)
                })
                .collect()
        })();
        match fields {
            Ok(fields) => additions.extend(fields),
            Err(error) => skipped_entries.push(SkippedEntry {
                entry: index + 1,
                datatype: entry.datatype,
                bit_offset: entry.bit_offset,
                bits: entry.bits,
                reason: error.to_string(),
            }),
        }
    }
    Ok(InputDescriptorPlan {
        profile_name: profile.name.clone(),
        layout_id: profile.iodd.as_ref().map(|iodd| iodd.layout.clone()),
        input_bytes: profile.input_bytes,
        raw_hex: report.raw_hex,
        additions,
        skipped_entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_suggestions_cannot_replace_a_mapping_added_after_review() -> Result<()> {
        let source = Profile::default();
        let plan = plan_input_descriptor(&source, "020800")?;
        let added = source.with_field(Field {
            name: "Operator field".into(),
            ..Field::default()
        })?;
        let pinned = added.clone();
        assert!(plan.apply(&added).is_err());
        assert_eq!(added, pinned);
        assert_eq!(plan.apply(&source)?.fields, plan.additions);
        Ok(())
    }
}
