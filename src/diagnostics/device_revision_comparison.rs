//! Compare recorded device revision strings with their per-query context.

use super::{
    DeviceRevisionsReview, DeviceRevisionsSnapshot, FIRMWARE_REVISION_INDEX,
    HARDWARE_REVISION_INDEX, ReplyComparison, RevisionValue, SavedRead,
};
use anyhow::Result;
use serde::Serialize;
use std::fmt::Write;

#[derive(Debug, Serialize)]
pub struct DeviceRevisionComparison<'a> {
    pub left: DeviceRevisionsReview<'a>,
    pub right: DeviceRevisionsReview<'a>,
    pub hardware_reply: ReplyComparison,
    pub firmware_reply: ReplyComparison,
    pub hardware_text_equal: Option<bool>,
    pub firmware_text_equal: Option<bool>,
}

impl DeviceRevisionComparison<'_> {
    /// A comparable change is a result, even when both strings differ.
    pub fn ok(&self) -> bool {
        self.hardware_text_equal.is_some() && self.firmware_text_equal.is_some()
    }

    pub fn summary_text(&self) -> String {
        let mut text = format!(
            "Saved device revision comparison · left → right\nProfiles · {} → {}",
            self.left.profile_name, self.right.profile_name
        );
        self.hardware_reply
            .write_summary(&mut text, HARDWARE_REVISION_INDEX);
        write_text_comparison(
            &mut text,
            "Hardware",
            self.hardware_text_equal,
            self.left.hardware.as_ref(),
            self.right.hardware.as_ref(),
        );
        self.firmware_reply
            .write_summary(&mut text, FIRMWARE_REVISION_INDEX);
        write_text_comparison(
            &mut text,
            "Firmware",
            self.firmware_text_equal,
            self.left.firmware.as_ref(),
            self.right.firmware.as_ref(),
        );
        text
    }
}

fn write_text_comparison(
    text: &mut String,
    name: &str,
    same: Option<bool>,
    left: Option<&RevisionValue>,
    right: Option<&RevisionValue>,
) {
    if let Some(equal) = same
        && let Some((left, right)) = revision_strings(left, right)
    {
        if equal {
            let _ = write!(text, "\n{name} revision text matches · {left:?}");
        } else {
            let _ = write!(
                text,
                "\n{name} revision text differs · {left:?} → {right:?}"
            );
        }
    }
}

fn revision_strings<'a>(
    left: Option<&'a RevisionValue>,
    right: Option<&'a RevisionValue>,
) -> Option<(&'a str, &'a str)> {
    let (left, right) = left.zip(right).filter(|(a, b)| a.ok() && b.ok())?;
    left.value.as_deref().zip(right.value.as_deref())
}

pub fn compare_device_revisions<'a>(
    left: &'a DeviceRevisionsSnapshot,
    right: &'a DeviceRevisionsSnapshot,
) -> Result<DeviceRevisionComparison<'a>> {
    let endpoint_issue = (left.profile.connection.endpoint != right.profile.connection.endpoint)
        .then_some("Saved gateway endpoints differ");
    let left = left.review()?;
    let right = right.review()?;
    let context_issue = [&left, &right]
        .into_iter()
        .any(|review| review.observed_context_matches == Some(false))
        .then_some("Recorded identity or master context differs within a snapshot")
        .or(endpoint_issue);
    let (hardware_reply, hardware_text_equal) = compare_reply(
        &left.reads[0],
        &right.reads[0],
        revision_strings(left.hardware.as_ref(), right.hardware.as_ref()),
        context_issue,
    );
    let (firmware_reply, firmware_text_equal) = compare_reply(
        &left.reads[1],
        &right.reads[1],
        revision_strings(left.firmware.as_ref(), right.firmware.as_ref()),
        context_issue,
    );
    Ok(DeviceRevisionComparison {
        left,
        right,
        hardware_reply,
        firmware_reply,
        hardware_text_equal,
        firmware_text_equal,
    })
}

fn compare_reply(
    left: &SavedRead,
    right: &SavedRead,
    strings: Option<(&str, &str)>,
    context_issue: Option<&'static str>,
) -> (ReplyComparison, Option<bool>) {
    let wire = ReplyComparison::new(
        left,
        right,
        context_issue,
        strings
            .is_none()
            .then_some("One or both saved revision strings need decoding review"),
    );
    let same = strings
        .filter(|_| wire.interpretation_issue.is_none())
        .map(|(left, right)| left == right);
    (wire, same)
}
