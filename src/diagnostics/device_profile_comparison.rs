//! Compare saved Common Profile replies and decoded descriptor geometry.
use super::{
    DescriptorStatus, DeviceProfileReview, DeviceProfileSnapshot, ProfileCharacteristicStatus,
    ReplyComparison, process_descriptor::DescriptorEntry,
};
use crate::iodd::format_profile_ids;
use anyhow::Result;
use serde::Serialize;
use std::{collections::BTreeMap, fmt::Write, path::Path};

#[derive(Debug, Serialize)]
pub struct DeviceProfileComparison {
    pub left: DeviceProfileReview,
    pub right: DeviceProfileReview,
    pub profile_characteristic_reply: ReplyComparison,
    pub input_descriptor_reply: ReplyComparison,
    pub output_descriptor_reply: ReplyComparison,
    pub profile_ids: Option<ProfileIdChanges>,
    pub input_changes: Option<Vec<DescriptorChange>>,
    pub output_changes: Option<Vec<DescriptorChange>>,
}

#[derive(Debug, Serialize)]
pub struct ProfileIdChanges {
    pub added_ids: Vec<u16>,
    pub removed_ids: Vec<u16>,
}

#[derive(Debug, Serialize)]
pub struct DescriptorChange {
    pub bit_offset: u16,
    pub left: Option<DescriptorEntry>,
    pub right: Option<DescriptorEntry>,
}

impl DeviceProfileComparison {
    pub fn load(left: &Path, right: &Path) -> Result<Self> {
        compare_device_profiles(
            &DeviceProfileSnapshot::load(left)?,
            &DeviceProfileSnapshot::load(right)?,
        )
    }

    pub fn ok(&self) -> bool {
        [
            &self.profile_characteristic_reply,
            &self.input_descriptor_reply,
            &self.output_descriptor_reply,
        ]
        .into_iter()
        .all(|reply| reply.interpretation_issue.is_none())
    }

    pub fn summary_text(&self) -> String {
        let mut text = String::from("Saved device profile comparison · left → right");
        let _ = write!(
            text,
            "\nProfiles · {} → {}",
            self.left.profile_name, self.right.profile_name
        );
        self.profile_characteristic_reply
            .write_summary(&mut text, 13);
        if let Some(changes) = &self.profile_ids {
            if changes.added_ids.is_empty() && changes.removed_ids.is_empty() {
                text.push_str("\nProfile IDs match");
            }
            if !changes.added_ids.is_empty() {
                let _ = write!(
                    text,
                    "\nAdded profile IDs · {}",
                    format_profile_ids(&changes.added_ids)
                );
            }
            if !changes.removed_ids.is_empty() {
                let _ = write!(
                    text,
                    "\nRemoved profile IDs · {}",
                    format_profile_ids(&changes.removed_ids)
                );
            }
        }
        self.input_descriptor_reply.write_summary(&mut text, 14);
        write_descriptor_changes(&mut text, self.input_changes.as_deref());
        self.output_descriptor_reply.write_summary(&mut text, 15);
        write_descriptor_changes(&mut text, self.output_changes.as_deref());
        text
    }
}

fn write_descriptor_changes(text: &mut String, changes: Option<&[DescriptorChange]>) {
    let Some(changes) = changes else { return };
    if changes.is_empty() {
        text.push_str("\nDescriptor entries match");
    }
    let describe = |entry: Option<&DescriptorEntry>| {
        entry.map_or_else(
            || "absent".to_string(),
            |entry| {
                format!(
                    "{} (0x{:02X}), width {}",
                    entry.datatype, entry.datatype_code, entry.bits
                )
            },
        )
    };
    for change in changes {
        let _ = write!(
            text,
            "\nBit offset {} · {} → {}",
            change.bit_offset,
            describe(change.left.as_ref()),
            describe(change.right.as_ref()),
        );
    }
}

pub fn compare_device_profiles(
    left: &DeviceProfileSnapshot,
    right: &DeviceProfileSnapshot,
) -> Result<DeviceProfileComparison> {
    let context_issue = if left.profile.connection.endpoint != right.profile.connection.endpoint {
        Some("Saved gateway endpoints differ")
    } else {
        None
    };
    let left = left.review()?;
    let right = right.review()?;
    let inconsistent = [&left, &right]
        .into_iter()
        .any(|review| review.observed_identity_conflict || review.observed_master_conflict)
        .then_some("Recorded identity or master context differs within a snapshot");
    let context_issue = inconsistent.or(context_issue);
    let ids = left
        .profile_characteristic
        .as_ref()
        .zip(right.profile_characteristic.as_ref())
        .filter(|(a, b)| {
            a.status != ProfileCharacteristicStatus::InvalidReply
                && b.status != ProfileCharacteristicStatus::InvalidReply
        })
        .map(|(a, b)| (a.reply_ids.as_slice(), b.reply_ids.as_slice()));
    let profile_characteristic_reply = ReplyComparison::new(
        &left.reads[0],
        &right.reads[0],
        context_issue,
        ids.is_none()
            .then_some("One or both saved profile ID lists need decoding review"),
    );
    let profile_ids = ids
        .filter(|_| profile_characteristic_reply.interpretation_issue.is_none())
        .map(|(a, b)| ProfileIdChanges {
            added_ids: b
                .iter()
                .copied()
                .filter(|id| a.binary_search(id).is_err())
                .collect(),
            removed_ids: a
                .iter()
                .copied()
                .filter(|id| b.binary_search(id).is_err())
                .collect(),
        });
    let (input_descriptor_reply, input_changes) = descriptor_changes(
        (&left.reads[1], &right.reads[1]),
        left.input_descriptor
            .as_ref()
            .zip(right.input_descriptor.as_ref()),
        context_issue,
    );
    let (output_descriptor_reply, output_changes) = descriptor_changes(
        (&left.reads[2], &right.reads[2]),
        left.output_descriptor
            .as_ref()
            .zip(right.output_descriptor.as_ref()),
        context_issue,
    );
    Ok(DeviceProfileComparison {
        left,
        right,
        profile_characteristic_reply,
        input_descriptor_reply,
        output_descriptor_reply,
        profile_ids,
        input_changes,
        output_changes,
    })
}

fn descriptor_changes(
    reads: (&super::SavedRead, &super::SavedRead),
    decoded: Option<(&super::ProcessDescriptor, &super::ProcessDescriptor)>,
    context_issue: Option<&'static str>,
) -> (ReplyComparison, Option<Vec<DescriptorChange>>) {
    let decoded = decoded.filter(|(a, b)| {
        a.status != DescriptorStatus::InvalidReply && b.status != DescriptorStatus::InvalidReply
    });
    let reply = ReplyComparison::new(
        reads.0,
        reads.1,
        context_issue,
        decoded
            .is_none()
            .then_some("One or both saved descriptors need decoding review"),
    );
    let changes = decoded
        .filter(|_| reply.interpretation_issue.is_none())
        .map(|(left, right)| {
            let mut offsets = BTreeMap::new();
            for entry in &left.entries {
                offsets.entry(entry.bit_offset).or_insert((None, None)).0 = Some(entry);
            }
            for entry in &right.entries {
                offsets.entry(entry.bit_offset).or_insert((None, None)).1 = Some(entry);
            }
            offsets
                .into_iter()
                .filter(|(_, (left, right))| left != right)
                .map(|(bit_offset, (left, right))| DescriptorChange {
                    bit_offset,
                    left: left.cloned(),
                    right: right.cloned(),
                })
                .collect()
        });
    (reply, changes)
}
