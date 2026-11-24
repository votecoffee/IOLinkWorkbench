//! Compare saved IODD parameter declarations by ISDU index and record subindex.
use super::{Imported, Profile};
use crate::iodd::{Parameter, RecordItem};
use crate::parameter_snapshot::Snapshot;
use anyhow::Context;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    path::{Path, PathBuf},
};

mod reply;
pub use reply::{ReplyComparison, ReplyRequest};

#[derive(Serialize)]
pub struct Comparison<'a> {
    pub left: Catalog<'a>,
    pub right: Catalog<'a>,
    pub definition_changes: Option<Vec<ParameterChange<'a>>>,
    pub unresolved_references: Option<ReferenceChanges<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<ReplyComparison<'a>>,
}

#[derive(Serialize)]
pub struct Catalog<'a> {
    pub profile_name: &'a str,
    pub vendor_id: Option<u16>,
    pub device_id: Option<u32>,
    pub variant: Option<&'a str>,
    pub parameter_count: Option<usize>,
    pub unresolved_standard_parameters: Option<&'a [String]>,
}

#[derive(Serialize)]
pub struct ParameterChange<'a> {
    pub index: u16,
    pub change: Change,
    pub properties: Vec<&'static str>,
    pub record_members: Vec<MemberChange>,
    pub array_element_properties: Vec<&'static str>,
    pub left: Option<&'a Parameter>,
    pub right: Option<&'a Parameter>,
}

#[derive(Serialize)]
pub struct MemberChange {
    pub subindex: u8,
    pub change: Change,
    pub properties: Vec<&'static str>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Removed,
    Changed,
}

#[derive(Serialize)]
pub struct ReferenceChanges<'a> {
    pub added: Vec<&'a str>,
    pub removed: Vec<&'a str>,
}

impl<'a> From<&'a Profile> for Catalog<'a> {
    fn from(profile: &'a Profile) -> Self {
        let iodd = profile.iodd.as_ref();
        Self {
            profile_name: &profile.name,
            vendor_id: profile.expected_identity.vendor_id,
            device_id: profile.expected_identity.device_id,
            variant: iodd.map(|iodd| iodd.variant.as_str()),
            parameter_count: iodd.map(|iodd| iodd.parameters.len()),
            unresolved_standard_parameters: iodd
                .map(|iodd| iodd.standard_parameter_refs.as_slice()),
        }
    }
}

pub fn compare<'a>(left: &'a Profile, right: &'a Profile) -> Comparison<'a> {
    let catalogs = left.iodd.as_ref().zip(right.iodd.as_ref());
    Comparison {
        left: Catalog::from(left),
        right: Catalog::from(right),
        definition_changes: catalogs.map(|(left, right)| {
            pair_by(&left.parameters, &right.parameters, |parameter| {
                parameter.index
            })
            .into_iter()
            .filter_map(|(index, (left, right))| compare_parameter(index, left, right))
            .collect()
        }),
        unresolved_references: catalogs.map(|(left, right)| compare_references(left, right)),
        reply: None,
    }
}

pub fn compare_with_reply<'a>(
    left: &'a Profile,
    right: &'a Profile,
    request: Option<ReplyRequest<'_>>,
) -> anyhow::Result<Comparison<'a>> {
    let mut comparison = compare(left, right);
    if let Some(request) = request {
        comparison.reply = Some(ReplyComparison::new(left, right, request)?);
    }
    Ok(comparison)
}

pub fn compare_with_snapshot<'a>(
    left: &'a Profile,
    right: &'a Profile,
    path: &Path,
) -> anyhow::Result<Comparison<'a>> {
    SavedReply::load(path)?.compare(left, right, false)
}

/// A validated snapshot that can supply both the recorded reply and its original catalog.
pub struct SavedReply {
    path: PathBuf,
    snapshot: Snapshot,
}

impl SavedReply {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let snapshot =
            Snapshot::load(path).with_context(|| format!("Open saved reply {}", path.display()))?;
        Ok(Self {
            path: path.to_owned(),
            snapshot,
        })
    }

    pub fn compare_to_profile<'a>(
        &'a self,
        candidate: &'a Profile,
    ) -> anyhow::Result<Comparison<'a>> {
        self.compare(&self.snapshot.profile, candidate, true)
    }

    fn compare<'a>(
        &self,
        left: &'a Profile,
        right: &'a Profile,
        baseline_uses_embedded_profile: bool,
    ) -> anyhow::Result<Comparison<'a>> {
        let mut comparison = compare(left, right);
        comparison.reply = Some(ReplyComparison::from_snapshot(
            left,
            right,
            &self.path,
            &self.snapshot,
            baseline_uses_embedded_profile,
        )?);
        Ok(comparison)
    }
}

fn pair_by<'a, K: Ord, T>(
    left: &'a [T],
    right: &'a [T],
    key: impl Fn(&T) -> K,
) -> BTreeMap<K, (Option<&'a T>, Option<&'a T>)> {
    let mut pairs = BTreeMap::<K, (Option<&T>, Option<&T>)>::new();
    for entry in left {
        pairs.entry(key(entry)).or_default().0 = Some(entry);
    }
    for entry in right {
        pairs.entry(key(entry)).or_default().1 = Some(entry);
    }
    pairs
}

fn compare_parameter<'a>(
    index: u16,
    left: Option<&'a Parameter>,
    right: Option<&'a Parameter>,
) -> Option<ParameterChange<'a>> {
    let mut properties = Vec::new();
    let mut record_members = Vec::new();
    let mut array_element_properties = Vec::new();
    let change = match (left, right) {
        (Some(left), Some(right)) => {
            properties = changed_properties(left, right);
            properties.extend(
                [
                    (
                        left.record.is_some() != right.record.is_some(),
                        "record layout",
                    ),
                    (
                        left.subindex_access() != right.subindex_access(),
                        "subindex access",
                    ),
                    (
                        left.array.is_some() != right.array.is_some(),
                        "array layout",
                    ),
                ]
                .into_iter()
                .filter_map(|(changed, name)| changed.then_some(name)),
            );
            record_members = compare_members(left, right);
            if let Some((left, right)) = left.array.as_ref().zip(right.array.as_ref()) {
                if left.count != right.count {
                    properties.push("array length");
                }
                array_element_properties = changed_properties(&left.element, &right.element);
            }
            if properties.is_empty()
                && record_members.is_empty()
                && array_element_properties.is_empty()
            {
                return None;
            }
            Change::Changed
        }
        (None, Some(_)) => Change::Added,
        (Some(_), None) => Change::Removed,
        (None, None) => return None,
    };
    Some(ParameterChange {
        index,
        change,
        properties,
        record_members,
        array_element_properties,
        left,
        right,
    })
}

fn compare_members(left: &Parameter, right: &Parameter) -> Vec<MemberChange> {
    fn items(parameter: &Parameter) -> &[RecordItem] {
        parameter
            .record
            .as_ref()
            .map_or(&[], |record| &record.items)
    }
    pair_by(items(left), items(right), |item| item.subindex)
        .into_iter()
        .filter_map(|(subindex, (left, right))| {
            let (change, properties) = match (left, right) {
                (Some(left), Some(right)) => {
                    let mut properties = changed_properties(&left.parameter, &right.parameter);
                    if left.bit_offset != right.bit_offset {
                        properties.push("bit offset");
                    }
                    if properties.is_empty() {
                        return None;
                    }
                    (Change::Changed, properties)
                }
                (None, Some(_)) => (Change::Added, Vec::new()),
                (Some(_), None) => (Change::Removed, Vec::new()),
                (None, None) => return None,
            };
            Some(MemberChange {
                subindex,
                change,
                properties,
            })
        })
        .collect()
}

fn changed_properties(left: &Parameter, right: &Parameter) -> Vec<&'static str> {
    [
        (left.id != right.id, "IODD ID"),
        (left.name != right.name, "name"),
        (left.access != right.access, "access"),
        (left.datatype != right.datatype, "datatype"),
        (left.bits != right.bits, "bit length"),
        (left.fixed_length != right.fixed_length, "fixed length"),
        (left.encoding != right.encoding, "encoding"),
        (left.default_value != right.default_value, "default"),
        (left.origin != right.origin, "origin"),
        (left.displays != right.displays, "display definitions"),
        (
            left.preferred_display != right.preferred_display,
            "preferred display",
        ),
        (left.choices != right.choices, "single values"),
        (left.ranges != right.ranges, "value ranges"),
        (left.needs_input != right.needs_input, "review notes"),
    ]
    .into_iter()
    .filter_map(|(changed, name)| changed.then_some(name))
    .collect()
}

fn compare_references<'a>(left: &'a Imported, right: &'a Imported) -> ReferenceChanges<'a> {
    let left: BTreeSet<_> = left
        .standard_parameter_refs
        .iter()
        .map(String::as_str)
        .collect();
    let right: BTreeSet<_> = right
        .standard_parameter_refs
        .iter()
        .map(String::as_str)
        .collect();
    ReferenceChanges {
        added: right.difference(&left).copied().collect(),
        removed: left.difference(&right).copied().collect(),
    }
}

impl Comparison<'_> {
    pub fn ok(&self) -> bool {
        self.definition_changes.is_some() && self.reply.as_ref().is_none_or(ReplyComparison::ok)
    }

    pub fn summary_text(&self) -> String {
        let mut text = format!("{} → {}", self.left.profile_name, self.right.profile_name);
        for (side, catalog) in [
            ("First profile", &self.left),
            ("Second profile", &self.right),
        ] {
            if let Some(count) = catalog.parameter_count {
                let _ = write!(text, "\n{side} · {count} saved parameter declarations");
            } else {
                let _ = write!(text, "\n{side} · no saved IODD parameter catalog");
            }
        }
        if self.left.variant != self.right.variant
            || self.left.vendor_id != self.right.vendor_id
            || self.left.device_id != self.right.device_id
        {
            text.push_str("\nSaved variant or configured device IDs differ");
        }
        if let Some(changes) = &self.definition_changes {
            let _ = write!(text, "\nChanged parameter indexes · {}", changes.len());
            for entry in changes {
                if let Some(parameter) = entry.right.or(entry.left) {
                    let _ = write!(text, "\n{}:0 · {}", entry.index, parameter.name);
                }
                entry.change.write_label(&mut text, &entry.properties);
                if let Some((left, right)) = entry.left.zip(entry.right)
                    && left.access != right.access
                {
                    let _ = write!(
                        text,
                        " · {} → {}",
                        left.access.label(),
                        right.access.label()
                    );
                }
                for member in &entry.record_members {
                    let _ = write!(text, "\n  {}:{}", entry.index, member.subindex);
                    member.change.write_label(&mut text, &member.properties);
                }
                if !entry.array_element_properties.is_empty() {
                    let _ = write!(
                        text,
                        "\n  Shared array element · {}",
                        entry.array_element_properties.join(", ")
                    );
                }
            }
        }
        if let Some(references) = &self.unresolved_references {
            for (description, ids) in [
                ("Added unresolved standard references", &references.added),
                (
                    "Removed unresolved standard references",
                    &references.removed,
                ),
            ] {
                if !ids.is_empty() {
                    let _ = write!(text, "\n{description} · {}", ids.join(", "));
                }
            }
        }
        if let Some(reply) = &self.reply {
            reply.write_summary(&mut text);
        }
        text
    }

    pub fn source_texts(&self) -> anyhow::Result<[String; 2]> {
        #[derive(Serialize)]
        struct Source<'a, 'b> {
            catalog: &'b Catalog<'a>,
            changed_definitions: Vec<&'a Parameter>,
            #[serde(skip_serializing_if = "Option::is_none")]
            reply: Option<&'b reply::Side<'a>>,
        }
        let entries = self.definition_changes.as_deref().unwrap_or_default();
        Ok([
            toml::to_string_pretty(&Source {
                catalog: &self.left,
                changed_definitions: entries.iter().filter_map(|entry| entry.left).collect(),
                reply: self.reply.as_ref().map(|reply| &reply.left),
            })?,
            toml::to_string_pretty(&Source {
                catalog: &self.right,
                changed_definitions: entries.iter().filter_map(|entry| entry.right).collect(),
                reply: self.reply.as_ref().map(|reply| &reply.right),
            })?,
        ])
    }
}

impl Change {
    fn write_label(self, text: &mut String, properties: &[&str]) {
        let label = match self {
            Self::Added => "Added",
            Self::Removed => "Removed",
            Self::Changed => "Changed",
        };
        let _ = write!(text, " · {label}");
        if !properties.is_empty() {
            let _ = write!(text, " {}", properties.join(", "));
        }
    }
}
