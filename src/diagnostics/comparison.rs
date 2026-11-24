//! Compare the two fixed ISDU replies in independently saved diagnostics.
use super::{DetailedStatus, Review, SavedRead, Snapshot};
use crate::parameter_snapshot::Observation;
use anyhow::Result;
use serde::Serialize;
use std::{collections::BTreeMap, fmt::Write as _};

#[derive(Debug, Serialize)]
pub struct Comparison {
    pub left: Review,
    pub right: Review,
    pub device_status_reply: ReplyComparison,
    pub detailed_status_reply: ReplyComparison,
    pub device_status_values: Option<StatusValues>,
    pub pending_events: Option<PendingEventComparison>,
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

#[derive(Debug, Serialize)]
pub struct PendingEventComparison {
    pub left_other_nonempty_entries: usize,
    pub right_other_nonempty_entries: usize,
    pub changed_counts: Vec<PendingEventCount>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct PendingEventCount {
    pub raw_hex: String,
    pub type_name: &'static str,
    pub left_count: usize,
    pub right_count: usize,
    pub left_name: Option<String>,
    pub right_name: Option<String>,
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
        if let Some(events) = &self.pending_events {
            events.write_summary(&mut text);
        }
        text
    }

    pub fn slot_changes_text(&self) -> Option<String> {
        let slots = self.changed_slots.as_ref()?;
        if slots.is_empty() {
            return Some("All recorded slots match".into());
        }
        let mut text = String::new();
        for slot in slots {
            let _ = writeln!(
                text,
                "Slot {} · {} → {}",
                slot.slot,
                slot.left_hex.as_deref().unwrap_or("absent"),
                slot.right_hex.as_deref().unwrap_or("absent")
            );
        }
        Some(text)
    }
}

impl PendingEventComparison {
    fn write_summary(&self, text: &mut String) {
        if self.changed_counts.is_empty() {
            text.push_str("\nRecorded pending-event counts match");
        } else {
            text.push_str("\nRecorded pending-event counts · left → right");
            for event in &self.changed_counts {
                let _ = write!(
                    text,
                    "\n{} · {} → {} · {}",
                    event.raw_hex, event.left_count, event.right_count, event.type_name
                );
                match (event.left_name.as_deref(), event.right_name.as_deref()) {
                    (Some(left), Some(right)) if left != right => {
                        let _ = write!(text, " · {left} → {right}");
                    }
                    (Some(name), _) | (_, Some(name)) => {
                        let _ = write!(text, " · {name}");
                    }
                    (None, None) => {}
                }
            }
        }
        if self.left_other_nonempty_entries != 0 || self.right_other_nonempty_entries != 0 {
            let _ = write!(
                text,
                "\nOther nonempty entries excluded from counts · {} left · {} right",
                self.left_other_nonempty_entries, self.right_other_nonempty_entries
            );
        }
    }
}

impl ReplyComparison {
    pub(super) fn new(
        left: &SavedRead,
        right: &SavedRead,
        context_issue: Option<&'static str>,
        decode_issue: Option<&'static str>,
    ) -> Self {
        let same_wire_bytes = left
            .raw_hex
            .as_ref()
            .zip(right.raw_hex.as_ref())
            .map(|(a, b)| a == b);
        let interpretation_issue = if same_wire_bytes.is_none() {
            Some("One or both saved replies are unavailable")
        } else {
            decode_issue
                .or(context_issue)
                .or_else(|| match_context(left, right).err().map(ContextIssue::message))
        };
        Self {
            same_wire_bytes,
            interpretation_issue,
        }
    }

    pub(super) fn write_summary(&self, text: &mut String, index: u16) {
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
    let context_issue = [&left, &right]
        .iter()
        .any(|review| {
            review.observed_master_conflict || review.observed_identities_equal == Some(false)
        })
        .then_some("Recorded identity or master context differs within a snapshot");
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
        context_issue,
        status_values
            .is_none()
            .then_some("One or both saved replies need length review"),
    );
    let detailed_status_reply = ReplyComparison::new(
        &left.reads[1],
        &right.reads[1],
        context_issue,
        detailed
            .is_none()
            .then_some("One or both saved replies need length review"),
    );
    let device_status_values = status_values
        .filter(|_| device_status_reply.interpretation_issue.is_none())
        .map(|(left, right)| StatusValues { left, right });
    let detailed = detailed.filter(|_| detailed_status_reply.interpretation_issue.is_none());
    let pending_events = detailed.map(|(a, b)| compare_pending_events(a, b));
    let changed_slots = detailed.map(|(a, b)| compare_slots(&a.raw_hex, &b.raw_hex));
    Ok(Comparison {
        left,
        right,
        device_status_reply,
        detailed_status_reply,
        device_status_values,
        pending_events,
        changed_slots,
    })
}

pub(super) enum ContextIssue {
    Unavailable(&'static str),
    Different(&'static str),
}

impl ContextIssue {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Unavailable(message) | Self::Different(message) => message,
        }
    }
}

pub(super) fn match_context(left: &SavedRead, right: &SavedRead) -> Result<(), ContextIssue> {
    use ContextIssue::{Different, Unavailable};
    let (a, b) = left
        .observation
        .as_ref()
        .zip(right.observation.as_ref())
        .ok_or(Unavailable("Recorded connection context is unavailable"))?;
    if a.gateway != b.gateway || a.route != b.route {
        return Err(Different("Recorded gateway routes differ"));
    }
    if a.port != b.port || a.port_mode != b.port_mode {
        return Err(Different("Recorded IO-Link port or mode differs"));
    }
    if a.model != b.model || a.firmware != b.firmware {
        return Err(Different("Recorded master model or firmware differs"));
    }
    let master_serials = a
        .master_serial
        .as_deref()
        .zip(b.master_serial.as_deref())
        .filter(|(a, b)| !a.trim().is_empty() && !b.trim().is_empty());
    if master_serials.is_some_and(|(a, b)| a != b) {
        return Err(Different("Recorded master serials differ"));
    }
    if a.identity.verify(&b.identity).is_err() {
        return Err(Different("Recorded device identity differs"));
    }
    if a.port_mode.trim().is_empty() || b.port_mode.trim().is_empty() {
        return Err(Unavailable("Recorded IO-Link port mode is unavailable"));
    }
    if master_serials.is_none() && a.gateway.ifm_model().is_some() {
        return Err(Unavailable("Recorded master serial is unavailable"));
    }
    if [a, b].into_iter().any(device_identity_incomplete) {
        return Err(Unavailable(
            "Recorded device Vendor ID, Device ID or serial is unavailable",
        ));
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

fn compare_pending_events(left: &DetailedStatus, right: &DetailedStatus) -> PendingEventComparison {
    #[derive(Default)]
    struct Count<'a> {
        entries: usize,
        name: Option<&'a str>,
    }

    // Compare complete qualifier/code pairs, independent of position or IODD text.
    let mut counts = BTreeMap::<(u8, u16, &'static str), [Count<'_>; 2]>::new();
    let mut other = [0, 0];
    for (side, report) in [left, right].into_iter().enumerate() {
        for event in &report.events {
            if event.pending_shape {
                let key = (event.qualifier, event.code, event.type_name);
                let count = &mut counts.entry(key).or_default()[side];
                count.entries += 1;
                count.name = event
                    .definition
                    .as_ref()
                    .map(|definition| definition.name.as_str());
            } else {
                other[side] += 1;
            }
        }
    }
    let changed_counts = counts
        .into_iter()
        .filter(|(_, [a, b])| a.entries != b.entries)
        .map(|((qualifier, code, type_name), [a, b])| PendingEventCount {
            raw_hex: format!("{qualifier:02X}{code:04X}"),
            type_name,
            left_count: a.entries,
            right_count: b.entries,
            left_name: a.name.map(str::to_owned),
            right_name: b.name.map(str::to_owned),
        })
        .collect();
    let [left_other_nonempty_entries, right_other_nonempty_entries] = other;
    PendingEventComparison {
        left_other_nonempty_entries,
        right_other_nonempty_entries,
        changed_counts,
    }
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
