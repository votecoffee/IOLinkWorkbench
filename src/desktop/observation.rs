//! Presentation of held GUI input. Never changes acquisition quality or CSV data.
use iolink_workbench::{
    capture::{LIVE_REFRESH_INTERVAL, Live},
    profile::Connection,
    transport::{Freshness, Validity},
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum State {
    Recent,
    Stopped,
    Disconnected,
    HostSilence,
    UpdateOld,
    StaleAtReceipt,
    InvalidAtReceipt,
    UnknownValidity,
}

impl State {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Recent => "Last received observation · within host-age limit",
            Self::Stopped => "Session stopped · retained raw input; typed values hidden",
            Self::Disconnected => "Disconnected · retained raw input; typed values hidden",
            Self::HostSilence => "No recent host response · typed values hidden",
            Self::UpdateOld => {
                "Last observed update exceeds display-age limit · typed values hidden"
            }
            Self::StaleAtReceipt => "Input was stale at receipt · typed values hidden",
            Self::InvalidAtReceipt => "Input was invalid at receipt · typed values hidden",
            Self::UnknownValidity => "Input validity unknown at receipt · typed values hidden",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Observation {
    pub receive_age: Duration,
    pub observed_update_age: Option<Duration>,
    pub source_age_at_receipt_us: Option<u64>,
    pub display_limit: Duration,
    pub state: State,
}

impl Observation {
    pub(super) fn at(
        live: &Live,
        running: bool,
        now: Instant,
        connection: &Connection,
    ) -> Option<Self> {
        let input = live.input?;
        let receive_age = now.saturating_duration_since(input.received);
        // This host timer measures time since an observed source update only
        // when the connected adapter supplies verifiable update metadata.
        let observed_update_age = live
            .status
            .as_ref()
            .filter(|status| status.freshness != Freshness::Snapshots)
            .and(live.last_new_at)
            .map(|last| now.saturating_duration_since(last));
        // The sampler publishes at most once per GUI interval, and the UI has
        // its own repaint interval. Neither interval is a source-update gap.
        let limit = Duration::from_micros(connection.stale_us.max(connection.poll_us))
            .saturating_add(LIVE_REFRESH_INTERVAL.saturating_mul(2));
        let state = if !running {
            State::Stopped
        } else if !live.connected {
            State::Disconnected
        } else if receive_age > limit {
            State::HostSilence
        } else if live.stale == Some(true) {
            State::StaleAtReceipt
        } else if observed_update_age.is_some_and(|age| age > limit) {
            State::UpdateOld
        } else {
            match input.validity {
                Validity::Valid => State::Recent,
                Validity::Invalid => State::InvalidAtReceipt,
                Validity::Unknown => State::UnknownValidity,
            }
        };
        Some(Self {
            receive_age,
            observed_update_age,
            source_age_at_receipt_us: input.source_age_us,
            display_limit: limit,
            state,
        })
    }

    pub(super) fn show_typed(&self) -> bool {
        self.state == State::Recent
    }
}

#[cfg(test)]
mod tests;
