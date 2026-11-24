//! Compare effective process input definitions from saved host profiles.
use super::{InputEncoding, Profile};
use crate::decode::{Field, Kind};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
};

#[derive(Serialize)]
pub struct Comparison<'a> {
    pub left: Mapping<'a>,
    pub right: Mapping<'a>,
    pub input_width_changed: bool,
    pub input_encoding_changed: bool,
    pub selection_changed: bool,
    pub common_field_order_changed: bool,
    pub changes: Vec<FieldChange<'a>>,
}

#[derive(Serialize)]
pub struct Mapping<'a> {
    pub profile_name: &'a str,
    pub input_bytes: usize,
    pub encoding: Encoding,
    pub selection: Option<Selection<'a>>,
    pub fields: Vec<MappedField<'a>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Encoding {
    NoInput,
    Standard,
    StandaloneBoolean,
    PaddedInteger { bits: u16, signed: bool },
}

#[derive(PartialEq, Eq, Serialize)]
pub struct Selection<'a> {
    pub variant: &'a str,
    pub layout: &'a str,
    pub condition: Option<&'a str>,
    pub condition_confirmed: Option<bool>,
}

#[derive(Serialize)]
pub struct MappedField<'a> {
    pub origin: &'static str,
    pub definition: &'a Field,
}

#[derive(Serialize)]
pub struct FieldChange<'a> {
    pub name: &'a str,
    pub status: Change,
    pub properties: Vec<&'static str>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Removed,
    Changed,
}

impl<'a> From<&'a Profile> for Mapping<'a> {
    fn from(profile: &'a Profile) -> Self {
        let encoding = match profile.input_encoding() {
            Some(InputEncoding::StandaloneBoolean) => Encoding::StandaloneBoolean,
            Some(InputEncoding::PaddedInteger(field)) => Encoding::PaddedInteger {
                bits: field.bits,
                signed: field.kind == Kind::Signed,
            },
            None if profile.has_process_input() => Encoding::Standard,
            None => Encoding::NoInput,
        };
        Self {
            profile_name: &profile.name,
            input_bytes: profile.input_bytes,
            encoding,
            selection: profile.iodd.as_ref().map(|iodd| Selection {
                variant: &iodd.variant,
                layout: &iodd.layout,
                condition: iodd.condition.as_deref(),
                condition_confirmed: iodd
                    .condition
                    .as_ref()
                    .map(|_| profile.confirm_iodd_condition),
            }),
            fields: profile
                .effective_fields()
                .into_iter()
                .map(|(definition, origin)| MappedField { origin, definition })
                .collect(),
        }
    }
}

pub fn compare<'a>(left: &'a Profile, right: &'a Profile) -> Comparison<'a> {
    let left = Mapping::from(left);
    let right = Mapping::from(right);
    let left_names = fields_by_name(&left);
    let right_names = fields_by_name(&right);
    let names: BTreeSet<_> = left_names
        .keys()
        .chain(right_names.keys())
        .copied()
        .collect();
    let changes = names
        .into_iter()
        .filter_map(|name| {
            let (status, properties) = match (left_names.get(name), right_names.get(name)) {
                (Some(a), Some(b)) => {
                    let properties = changed_properties(a, b);
                    if properties.is_empty() {
                        return None;
                    }
                    (Change::Changed, properties)
                }
                (Some(_), None) => (Change::Removed, Vec::new()),
                (None, Some(_)) => (Change::Added, Vec::new()),
                (None, None) => unreachable!("name comes from either mapping"),
            };
            Some(FieldChange {
                name,
                status,
                properties,
            })
        })
        .collect();
    let common_field_order_changed = !left
        .fields
        .iter()
        .map(|field| field.definition.name.as_str())
        .filter(|name| right_names.contains_key(name))
        .eq(right
            .fields
            .iter()
            .map(|field| field.definition.name.as_str())
            .filter(|name| left_names.contains_key(name)));
    Comparison {
        input_width_changed: left.input_bytes != right.input_bytes,
        input_encoding_changed: left.encoding != right.encoding,
        selection_changed: left.selection != right.selection,
        common_field_order_changed,
        left,
        right,
        changes,
    }
}

fn fields_by_name<'a, 'b>(mapping: &'b Mapping<'a>) -> BTreeMap<&'a str, &'b MappedField<'a>> {
    mapping
        .fields
        .iter()
        .map(|field| (field.definition.name.as_str(), field))
        .collect()
}

fn changed_properties(left: &MappedField<'_>, right: &MappedField<'_>) -> Vec<&'static str> {
    let (a, b) = (left.definition, right.definition);
    [
        (a.bit_offset != b.bit_offset, "bit offset"),
        (a.bits != b.bits, "width"),
        (a.kind != b.kind, "datatype"),
        (a.byte_order != b.byte_order, "byte order"),
        (a.scale.to_bits() != b.scale.to_bits(), "scale"),
        (
            a.offset.to_bits() != b.offset.to_bits(),
            "engineering offset",
        ),
        (a.unit != b.unit, "unit"),
        (a.decimals != b.decimals, "precision"),
        (a.display_format != b.display_format, "display format"),
        (a.value_names != b.value_names, "named values"),
        (a.value_ranges != b.value_ranges, "named ranges"),
        (left.origin != right.origin, "origin"),
    ]
    .into_iter()
    .filter_map(|(changed, name)| changed.then_some(name))
    .collect()
}

impl Comparison<'_> {
    pub fn summary_text(&self) -> String {
        let mut text = format!("{} → {}", self.left.profile_name, self.right.profile_name);
        if self.input_width_changed {
            let _ = write!(
                text,
                "\nInput width · {} → {} bytes",
                self.left.input_bytes, self.right.input_bytes
            );
        }
        for (changed, description) in [
            (
                self.selection_changed,
                "Saved IODD selection or condition changed",
            ),
            (self.input_encoding_changed, "Input encoding changed"),
            (
                self.common_field_order_changed,
                "Order of common input fields changed",
            ),
        ] {
            if changed {
                let _ = write!(text, "\n{description}");
            }
        }
        for change in &self.changes {
            let _ = write!(text, "\n{}", change.name);
            match change.status {
                Change::Added => text.push_str(" · Added"),
                Change::Removed => text.push_str(" · Removed"),
                Change::Changed => {
                    let _ = write!(text, " · Changed {}", change.properties.join(", "));
                }
            }
        }
        if !self.input_width_changed
            && !self.input_encoding_changed
            && !self.selection_changed
            && !self.common_field_order_changed
            && self.changes.is_empty()
        {
            text.push_str("\nSaved input mappings match");
        }
        text
    }
}
