//! Compare the two fixed ISDU replies in independently saved diagnostics.
use super::{Review, SavedRead, Snapshot};
use crate::{parameter_snapshot::Observation, profile::Gateway};
use anyhow::Result;
use serde::Serialize;
use std::fmt::Write as _;

#[derive(Debug, Serialize)]
pub struct Comparison {
    pub left: Review,
    pub right: Review,
    pub device_status_reply: ReplyComparison,
    pub detailed_status_reply: ReplyComparison,
    pub device_status_values: Option<StatusValues>,
    /// A comparison of slot positions in two saved payloads.
    pub changed_slots: Option<Vec<SlotChange>>,
}

#[derive(Debug, Serialize)]
pub struct ReplyComparison {
    pub same_wire_bytes: Option<bool>,
    pub interpretation_issue: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct StatusValues {
    pub left: u8,
    pub right: u8,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct SlotChange {
    pub slot: usize,
    pub left_hex: Option<String>,
    pub right_hex: Option<String>,
}

impl Comparison {
    pub fn ok(&self) -> bool {
        self.left.ok() && self.right.ok()
    }

    pub fn summary_text(&self) -> String {
        let mut text =
            String::from("Saved diagnostic comparison · source event history and loss unknown");
        self.device_status_reply.write_summary(&mut text, 36);
        if let Some(values) = &self.device_status_values {
            let _ = write!(
                text,
                "\nRecorded Device Status {} → {}",
                values.left, values.right
            );
        }
        self.detailed_status_reply.write_summary(&mut text, 37);
        if let Some(slots) = &self.changed_slots {
            if slots.is_empty() {
                text.push_str("\nAll recorded slots match");
            }
            for slot in slots {
                let _ = write!(
                    text,
                    "\nSlot {} · {} → {}",
                    slot.slot,
                    slot.left_hex.as_deref().unwrap_or("absent"),
                    slot.right_hex.as_deref().unwrap_or("absent")
                );
            }
        }
        text
    }
}

impl ReplyComparison {
    fn new(
        left: &SavedRead,
        right: &SavedRead,
        inconsistent_snapshot: bool,
        decoded: bool,
    ) -> Self {
        let same_wire_bytes = left
            .raw_hex
            .as_ref()
            .zip(right.raw_hex.as_ref())
            .map(|(a, b)| a == b);
        let interpretation_issue = if same_wire_bytes.is_none() {
            Some("One or both saved replies are unavailable")
        } else if !decoded {
            Some("One or both saved replies need length review")
        } else if inconsistent_snapshot {
            Some("Recorded identity or master context differs within a snapshot")
        } else {
            match_context(left, right).err()
        };
        Self {
            same_wire_bytes,
            interpretation_issue,
        }
    }

    fn write_summary(&self, text: &mut String, index: u16) {
        let wire = match self.same_wire_bytes {
            Some(true) => "match",
            Some(false) => "differ",
            None => "unavailable",
        };
        let _ = write!(text, "\n\n{index}:0 · wire bytes {wire}");
        if let Some(issue) = self.interpretation_issue {
            let _ = write!(text, "\n{issue}");
        }
    }
}

/// Review both complete files before deriving per-query differences.
pub fn compare(left: &Snapshot, right: &Snapshot) -> Result<Comparison> {
    let left = left.review()?;
    let right = right.review()?;
    let inconsistent_snapshot = [&left, &right].iter().any(|review| {
        review.observed_master_conflict || review.observed_identities_equal == Some(false)
    });
    let status_values = left
        .device_status
        .as_ref()
        .and_then(|status| status.value)
        .zip(right.device_status.as_ref().and_then(|status| status.value));
    let detailed = left
        .detailed_status
        .as_ref()
        .zip(right.detailed_status.as_ref())
        .filter(|(a, b)| a.decode_error.is_none() && b.decode_error.is_none());
    let device_status_reply = ReplyComparison::new(
        &left.reads[0],
        &right.reads[0],
        inconsistent_snapshot,
        status_values.is_some(),
    );
    let detailed_status_reply = ReplyComparison::new(
        &left.reads[1],
        &right.reads[1],
        inconsistent_snapshot,
        detailed.is_some(),
    );
    let device_status_values = status_values
        .filter(|_| device_status_reply.interpretation_issue.is_none())
        .map(|(left, right)| StatusValues { left, right });
    let changed_slots = detailed
        .filter(|_| detailed_status_reply.interpretation_issue.is_none())
        .map(|(a, b)| compare_slots(&a.raw_hex, &b.raw_hex));
    Ok(Comparison {
        left,
        right,
        device_status_reply,
        detailed_status_reply,
        device_status_values,
        changed_slots,
    })
}

fn match_context(left: &SavedRead, right: &SavedRead) -> Result<(), &'static str> {
    let (a, b) = left
        .observation
        .as_ref()
        .zip(right.observation.as_ref())
        .ok_or("Recorded connection context is unavailable")?;
    if a.gateway != b.gateway || a.route != b.route {
        return Err("Recorded gateway routes differ");
    }
    if a.port != b.port || a.port_mode != b.port_mode {
        return Err("Recorded IO-Link port or mode differs");
    }
    if a.port_mode.trim().is_empty() || b.port_mode.trim().is_empty() {
        return Err("Recorded IO-Link port mode is unavailable");
    }
    if a.model != b.model || a.firmware != b.firmware {
        return Err("Recorded master model or firmware differs");
    }
    let master_serials = a
        .master_serial
        .as_deref()
        .zip(b.master_serial.as_deref())
        .filter(|(a, b)| !a.trim().is_empty() && !b.trim().is_empty());
    match master_serials {
        Some((a, b)) if a != b => return Err("Recorded master serials differ"),
        None if a.gateway == Gateway::IfmAl1350 => {
            return Err("Recorded master serial is unavailable");
        }
        _ => {}
    }
    if a.identity.verify(&b.identity).is_err() {
        return Err("Recorded device identity differs");
    }
    if [a, b].into_iter().any(device_identity_incomplete) {
        return Err("Recorded device Vendor ID, Device ID or serial is unavailable");
    }
    Ok(())
}

fn device_identity_incomplete(observation: &Observation) -> bool {
    let identity = &observation.identity;
    identity.vendor_id.is_none_or(|id| id == 0)
        || identity.device_id.is_none_or(|id| id == 0)
        || identity
            .serial
            .as_deref()
            .is_none_or(|serial| serial.trim().is_empty())
}

fn compare_slots(left: &str, right: &str) -> Vec<SlotChange> {
    // Validated bytes are canonical uppercase hex, six digits per three-byte slot.
    const DIGITS: usize = 6;
    (0..left.len().max(right.len()) / DIGITS)
        .filter_map(|index| {
            let start = index * DIGITS;
            let a = left.get(start..start + DIGITS);
            let b = right.get(start..start + DIGITS);
            (a != b).then(|| SlotChange {
                slot: index + 1,
                left_hex: a.map(str::to_owned),
                right_hex: b.map(str::to_owned),
            })
        })
        .collect()
}
