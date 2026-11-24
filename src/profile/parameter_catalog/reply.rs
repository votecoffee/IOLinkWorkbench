//! Interpret one recorded ISDU payload separately under each saved declaration.
use super::{Profile, pair_by};
use crate::{
    decode::{hex, parse_hex_bounded},
    iodd::{Access, DecodedParameter, Parameter, RecordValue},
};
use anyhow::{Context, Result};
use serde::Serialize;
use std::fmt::Write;

pub struct ReplyRequest<'a> {
    pub index: u16,
    pub subindex: u8,
    pub payload: &'a str,
}

#[derive(Serialize)]
pub struct ReplyComparison<'a> {
    pub index: u16,
    pub subindex: u8,
    pub raw_hex: String,
    pub left: Side<'a>,
    pub right: Side<'a>,
    pub differences: Option<Differences>,
}

#[derive(Serialize)]
pub struct Side<'a> {
    pub declaration: Option<Declaration<'a>>,
    pub decoded: Option<DecodedParameter>,
    pub resolution_error: Option<String>,
}

#[derive(Serialize)]
pub struct Declaration<'a> {
    pub parent_id: &'a str,
    pub target_id: &'a str,
    pub name: &'a str,
    pub datatype: &'a str,
    pub access: Access,
    pub direct_subindex_access: Option<bool>,
}

#[derive(Serialize)]
pub struct Differences {
    pub properties: Vec<&'static str>,
    pub compared_members: usize,
    pub members: Vec<MemberDifference>,
}

#[derive(Serialize)]
pub struct MemberDifference {
    pub subindex: u8,
    pub properties: Vec<&'static str>,
}

impl<'a> Side<'a> {
    fn decode(profile: &'a Profile, index: u16, subindex: u8, bytes: &[u8]) -> Self {
        match target(profile, index, subindex) {
            Ok((parent, target)) => Self {
                declaration: Some(Declaration {
                    parent_id: &parent.id,
                    target_id: &target.id,
                    name: &target.name,
                    datatype: &target.datatype,
                    access: target.access,
                    direct_subindex_access: (subindex != 0).then(|| parent.subindex_access()),
                }),
                decoded: Some(target.decode(bytes)),
                resolution_error: None,
            },
            Err(error) => Self {
                declaration: None,
                decoded: None,
                resolution_error: Some(error.to_string()),
            },
        }
    }

    fn ok(&self) -> bool {
        self.decoded
            .as_ref()
            .is_some_and(|decoded| !decoded.needs_review())
    }

    fn write_summary(&self, text: &mut String, label: &str) {
        if let Some(error) = &self.resolution_error {
            let _ = write!(text, "\n{label} · {error}");
        }
        if let Some((declaration, decoded)) = self.declaration.as_ref().zip(self.decoded.as_ref()) {
            let _ = write!(
                text,
                "\n{label} · {} · {} · {}",
                declaration.name,
                declaration.datatype,
                declaration.access.label()
            );
            if let Some(error) = &decoded.decode_error {
                let _ = write!(text, " · {error}");
            } else if let Some(display) = decoded
                .preferred_display
                .as_ref()
                .or(decoded.displays.first())
            {
                let _ = write!(
                    text,
                    " · {}",
                    format!("{} {}", display.value, display.unit).trim_end()
                );
            } else if let Some(raw) = &decoded.raw_value {
                let _ = write!(text, " · Raw value {raw}");
            }
            for item in decoded.record_items.iter().chain(&decoded.array_items) {
                if let Some(error) = &item.value.decode_error {
                    let _ = write!(text, "\n  Subindex {} · {error}", item.subindex);
                }
            }
            if declaration.direct_subindex_access == Some(false) {
                let _ = write!(
                    text,
                    "\n{label} · Saved definition disables direct subindex access"
                );
            }
        }
    }
}

fn target(profile: &Profile, index: u16, subindex: u8) -> Result<(&Parameter, &Parameter)> {
    let catalog = profile
        .iodd
        .as_ref()
        .context("No saved IODD parameter catalog")?;
    let parent = catalog
        .parameters
        .iter()
        .find(|parameter| parameter.index == index)
        .with_context(|| format!("Index {index} is absent from the saved catalog"))?;
    Ok((parent, parent.target(subindex)?))
}

impl<'a> ReplyComparison<'a> {
    pub(super) fn new(
        left: &'a Profile,
        right: &'a Profile,
        request: ReplyRequest<'_>,
    ) -> Result<Self> {
        let ReplyRequest {
            index,
            subindex,
            payload,
        } = request;
        let bytes = parse_hex_bounded(payload, 232)?;
        let left = Side::decode(left, index, subindex, &bytes);
        let right = Side::decode(right, index, subindex, &bytes);
        let differences = left
            .decoded
            .as_ref()
            .zip(right.decoded.as_ref())
            .filter(|(left, right)| left.decode_error.is_none() && right.decode_error.is_none())
            .map(|(left, right)| compare_decoded(left, right));
        Ok(Self {
            index,
            subindex,
            raw_hex: hex(&bytes),
            left,
            right,
            differences,
        })
    }

    pub fn ok(&self) -> bool {
        self.left.ok() && self.right.ok()
    }

    pub(super) fn write_summary(&self, text: &mut String) {
        let _ = write!(
            text,
            "\n\nRecorded reply · {}:{} · {}",
            self.index, self.subindex, self.raw_hex
        );
        self.left.write_summary(text, "First profile");
        self.right.write_summary(text, "Second profile");
        let Some(diff) = &self.differences else {
            text.push_str("\nDecoded value comparison unavailable");
            return;
        };
        if !diff.properties.is_empty() {
            let _ = write!(
                text,
                "\nChanged interpretation · {}",
                diff.properties.join(", ")
            );
        }
        if diff.compared_members != 0 {
            let _ = write!(text, "\nCommon decoded members · {}", diff.compared_members);
        }
        for member in &diff.members {
            let _ = write!(
                text,
                "\n{}:{} · Changed {}",
                self.index,
                member.subindex,
                member.properties.join(", ")
            );
        }
    }
}

fn compare_decoded(left: &DecodedParameter, right: &DecodedParameter) -> Differences {
    let usable_pairs = |a: &[RecordValue], b: &[RecordValue]| {
        pair_by(a, b, |item| item.subindex)
            .into_iter()
            .filter_map(|(subindex, (left, right))| {
                let (left, right) = left.zip(right)?;
                (left.value.decode_error.is_none() && right.value.decode_error.is_none()).then(
                    || MemberDifference {
                        subindex,
                        properties: changed_properties(&left.value, &right.value),
                    },
                )
            })
            .collect::<Vec<_>>()
    };
    let mut members = usable_pairs(&left.record_items, &right.record_items);
    members.extend(usable_pairs(&left.array_items, &right.array_items));
    let compared_members = members.len();
    members.retain(|member| !member.properties.is_empty());
    Differences {
        properties: changed_properties(left, right),
        compared_members,
        members,
    }
}

fn changed_properties(left: &DecodedParameter, right: &DecodedParameter) -> Vec<&'static str> {
    [
        (left.raw_hex != right.raw_hex, "member bytes"),
        (left.raw_value != right.raw_value, "raw value"),
        (left.choice_name != right.choice_name, "named value"),
        (
            left.value_assessment != right.value_assessment,
            "value assessment",
        ),
        (left.displays != right.displays, "display values"),
        (
            left.preferred_display != right.preferred_display,
            "preferred display",
        ),
        (left.time_value != right.time_value, "time interpretation"),
    ]
    .into_iter()
    .filter_map(|(changed, label)| changed.then_some(label))
    .collect()
}
