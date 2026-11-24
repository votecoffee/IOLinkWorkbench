use super::Link;
use crate::profile::Profile;
use serde::Serialize;
use std::fmt::{Display, Write};

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Agreement {
    Match,
    Different,
    Unavailable,
}

impl Agreement {
    fn label(self) -> &'static str {
        match self {
            Self::Match => "Match",
            Self::Different => "Different",
            Self::Unavailable => "Unavailable",
        }
    }
}

#[derive(Serialize)]
struct Compared<T> {
    declared: Option<T>,
    recorded: Option<T>,
    status: Agreement,
}

impl<T: Eq> Compared<T> {
    fn new(declared: Option<T>, recorded: Option<T>) -> Self {
        let status = match (&declared, &recorded) {
            (Some(declared), Some(recorded)) if declared == recorded => Agreement::Match,
            (Some(_), Some(_)) => Agreement::Different,
            _ => Agreement::Unavailable,
        };
        Self {
            declared,
            recorded,
            status,
        }
    }
}

impl<T: Display> Compared<T> {
    fn append(&self, text: &mut String, name: &str) {
        let declared = self
            .declared
            .as_ref()
            .map_or_else(|| "—".into(), ToString::to_string);
        let recorded = self
            .recorded
            .as_ref()
            .map_or_else(|| "—".into(), ToString::to_string);
        let _ = write!(
            text,
            "\n{name} · IODD {declared} · Page {recorded} · {}",
            self.status.label()
        );
    }
}

#[derive(Serialize)]
pub struct IoddComparison {
    variant: String,
    layout: String,
    context_issue: Option<&'static str>,
    layout_issue: Option<&'static str>,
    minimum_cycle_us: Option<Compared<u32>>,
    sio_supported: Option<Compared<bool>>,
    m_sequence_capability: Option<Compared<u8>>,
    input_bytes: Option<Compared<usize>>,
    output_bytes: Option<Compared<usize>>,
}

impl IoddComparison {
    pub(super) fn new(profile: &Profile, link: &Link) -> Option<Self> {
        let iodd = profile.iodd.as_ref()?;
        let mut review = Self {
            variant: iodd.variant.clone(),
            layout: iodd.layout.clone(),
            context_issue: None,
            layout_issue: None,
            minimum_cycle_us: None,
            sio_supported: None,
            m_sequence_capability: None,
            input_bytes: None,
            output_bytes: None,
        };
        review.context_issue = match (link.expected_vendor_id, link.expected_device_id) {
            (None | Some(0), _) | (_, None | Some(0)) => {
                Some("The selected profile needs both Vendor ID and Device ID pins")
            }
            (Some(vendor_id), Some(device_id))
                if vendor_id != link.vendor_id || device_id != link.device_id =>
            {
                Some("The recorded Vendor ID or Device ID differs from the profile pins")
            }
            _ => None,
        };
        if review.context_issue.is_some() {
            return Some(review);
        }

        if let Some(comm) = profile.communication() {
            review.minimum_cycle_us = Some(Compared::new(
                (comm.minimum_cycle_us != 0).then_some(comm.minimum_cycle_us),
                link.device_min_cycle_us,
            ));
            review.sio_supported = Some(Compared::new(
                Some(comm.sio_supported),
                Some(link.sio_supported),
            ));
            review.m_sequence_capability = Some(Compared::new(
                Some(comm.m_sequence_capability),
                (link.m_sequence_capability & 0xc0 == 0).then_some(link.m_sequence_capability),
            ));
        }
        if iodd.condition.is_some() && !profile.confirm_iodd_condition {
            review.layout_issue =
                Some("Confirm the selected IODD layout condition to compare payload widths");
        } else {
            review.input_bytes = Some(Compared::new(
                Some(iodd.input_bytes),
                link.input.bytes_if_valid(0x20),
            ));
            review.output_bytes = iodd.output.as_ref().map(|layout| {
                Compared::new(Some(layout.bytes()), link.output.bytes_if_valid(0x60))
            });
        }
        Some(review)
    }

    pub fn has_difference(&self) -> bool {
        [
            self.minimum_cycle_us.as_ref().map(|check| check.status),
            self.sio_supported.as_ref().map(|check| check.status),
            self.m_sequence_capability
                .as_ref()
                .map(|check| check.status),
            self.input_bytes.as_ref().map(|check| check.status),
            self.output_bytes.as_ref().map(|check| check.status),
        ]
        .contains(&Some(Agreement::Different))
    }

    pub fn text(&self) -> String {
        let mut text = format!("Variant · {}\nLayout · {}", self.variant, self.layout);
        if let Some(issue) = self.context_issue {
            let _ = write!(text, "\n{issue}");
            return text;
        }
        if let Some(check) = &self.minimum_cycle_us {
            check.append(&mut text, "Minimum cycle (µs)");
        }
        if let Some(check) = &self.sio_supported {
            check.append(&mut text, "SIO supported");
        }
        if let Some(check) = &self.m_sequence_capability {
            check.append(&mut text, "M-sequence capability");
        }
        if let Some(issue) = self.layout_issue {
            let _ = write!(text, "\n{issue}");
        }
        if let Some(check) = &self.input_bytes {
            check.append(&mut text, "Input bytes");
        }
        if let Some(check) = &self.output_bytes {
            check.append(&mut text, "Output bytes");
        }
        text
    }
}
