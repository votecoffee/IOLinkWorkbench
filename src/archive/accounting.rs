//! Recorded acquisition/selection consistency. Never equate filtering with source loss.
use super::{Manifest, Row};
use crate::{
    profile::{LogMode, Profile},
    transport::Freshness,
};
use anyhow::{Result, ensure};

pub(super) fn validate(m: &Manifest, profile: &Profile) -> Result<()> {
    let c = &m.counts;
    if let Some(disconnects) = c.disconnects {
        ensure!(
            c.reconnects <= disconnects,
            "successful reconnects exceed recorded disconnects"
        );
        if m.last_status.is_some()
            && let Some(connected) = m.ended_connected
        {
            ensure!(
                u128::from(disconnects) == u128::from(c.reconnects) + u128::from(!connected),
                "capture end connection state contradicts disconnect/reconnect counts"
            );
        }
    }
    if let Some(received) = c.received {
        ensure!(
            c.selected <= received,
            "selected rows exceed received inputs"
        );
        if let Some(updates) = c.new_updates_observed {
            ensure!(
                updates <= received,
                "new source updates exceed received inputs"
            );
        }
    }
    if let Some(polls) = c.poll_attempts {
        ensure!(
            c.received.is_none_or(|v| v <= polls) && c.disconnects.is_none_or(|v| v <= polls),
            "receive/disconnect count exceeds poll attempts"
        );
        if let (Some(received), Some(disconnects)) = (c.received, c.disconnects) {
            ensure!(
                u128::from(received) + u128::from(disconnects) <= u128::from(polls),
                "received inputs plus disconnects exceed poll attempts"
            );
        }
    }
    if profile.logging.mode == LogMode::EveryUpdate {
        ensure!(
            c.unchanged.is_none_or(|v| v == 0),
            "every-update logging cannot report unchanged-value filtering"
        );
    }
    if let Some(status) = &m.last_status {
        if status.freshness == Freshness::Snapshots {
            ensure!(
                c.new_updates_observed.is_none_or(|v| v == 0),
                "snapshot freshness cannot claim verifiably new source updates"
            );
        }
        ensure!(
            m.row_kind
                .as_deref()
                .is_none_or(|v| v == status.freshness.row_kind()),
            "report row kind contradicts recorded freshness"
        );
        let eligible = match status.freshness {
            Freshness::Snapshots => c.received,
            Freshness::Counter32 | Freshness::Timestamp => c.new_updates_observed,
        };
        if let Some(eligible) = eligible {
            ensure!(
                c.selected <= eligible,
                "selected rows exceed the freshness contract's eligible inputs"
            );
            if profile.logging.mode == LogMode::EveryUpdate {
                ensure!(
                    c.selected == eligible,
                    "every-update selection differs from eligible input count"
                );
            }
            if let Some(unchanged) = c.unchanged {
                ensure!(
                    u128::from(c.selected) + u128::from(unchanged) == u128::from(eligible),
                    "selected plus unchanged inputs differ from the eligible input count"
                );
            }
        }
        if let Some(gaps) = m.source_gaps_observed {
            ensure!(
                status.freshness == Freshness::Counter32,
                "source-gap total requires counter-based freshness, not snapshots or timestamps"
            );
            if let Some(updates) = c.new_updates_observed {
                let upper = u128::from(updates.saturating_sub(1)) * u128::from((1_u64 << 31) - 2);
                ensure!(
                    u128::from(gaps) <= upper,
                    "source gaps exceed the recorded source-update population"
                );
            }
        }
    }
    Ok(())
}

#[derive(Default)]
pub(super) struct Rows {
    counter_gaps: u128,
}

impl Rows {
    // Call only after validate_row; counter order and half-range bounds are checked there.
    pub(super) fn observe(&mut self, row: &Row<'_>, previous: Option<&Row<'_>>) {
        if let Some(previous) = previous
            && row.generation == previous.generation
            && let (Some(now), Some(before)) = (row.source_counter, previous.source_counter)
        {
            self.counter_gaps += u128::from(now.wrapping_sub(before).saturating_sub(1));
        }
    }

    pub(super) fn finish(&self, m: &Manifest, profile: &Profile) -> Result<()> {
        // Only a complete, unfiltered population can reproduce observed source gaps.
        // Counter jumps in selected/dropped rows otherwise mix several loss domains.
        if profile.logging.mode == LogMode::EveryUpdate
            && m.counts.logger_overruns == 0
            && m.counts.new_updates_observed == Some(m.counts.selected)
            && m.last_status
                .as_ref()
                .is_some_and(|s| s.freshness == Freshness::Counter32)
            && let Some(gaps) = m.source_gaps_observed
        {
            ensure!(
                self.counter_gaps == u128::from(gaps),
                "source counter gaps in complete CSV disagree with the capture report"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
