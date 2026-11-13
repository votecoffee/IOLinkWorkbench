//! Quality domains overlap. Only compare each population with its own recorded counters.
use super::{Manifest, Row};
use crate::{
    profile::Profile,
    transport::{Freshness, Validity},
};
use anyhow::{Context, Result, ensure};

pub(super) fn validate(m: &Manifest, profile: &Profile) -> Result<()> {
    ensure!(
        m.stale_threshold_us == profile.connection.stale_us,
        "stale threshold differs from saved source profile"
    );
    let c = &m.counts;
    if let Some(received) = c.received {
        for (name, count) in [
            ("invalid", c.invalid),
            ("validity_unknown", c.validity_unknown),
            ("stale", c.stale),
            ("freshness_unknown", c.freshness_unknown),
            ("duplicate", c.duplicate),
            ("out_of_order", c.out_of_order),
        ] {
            ensure!(
                count.is_none_or(|v| v <= received),
                "{name} count exceeds received inputs"
            );
        }
        for (name, a, b) in [
            ("validity", c.invalid, c.validity_unknown),
            ("staleness", c.stale, c.freshness_unknown),
        ] {
            if let (Some(a), Some(b)) = (a, b) {
                ensure!(
                    u128::from(a) + u128::from(b) <= u128::from(received),
                    "{name} category counts exceed received inputs"
                );
            }
        }
    }
    if let Some(status) = &m.last_status {
        if status.freshness == Freshness::Snapshots {
            ensure!(
                c.duplicate.is_none_or(|v| v == 0) && c.out_of_order.is_none_or(|v| v == 0),
                "snapshot freshness cannot claim verified duplicate or late source updates"
            );
        } else if let (Some(received), Some(new), Some(duplicates), Some(late)) = (
            c.received,
            c.new_updates_observed,
            c.duplicate,
            c.out_of_order,
        ) {
            ensure!(
                u128::from(new) + u128::from(duplicates) + u128::from(late) == u128::from(received),
                "new, duplicate and late classifications differ from received input count"
            );
        }
    }
    Ok(())
}

#[derive(Default)]
pub(super) struct Rows {
    validity: [u64; 3],  // valid, invalid, unknown
    staleness: [u64; 3], // false, true, unknown
}

impl Rows {
    pub(super) fn observe(&mut self, row: &Row<'_>) {
        self.validity[match row.validity {
            Validity::Valid => 0,
            Validity::Invalid => 1,
            Validity::Unknown => 2,
        }] += 1;
        self.staleness[match row.stale {
            Some(false) => 0,
            Some(true) => 1,
            None => 2,
        }] += 1;
    }

    pub(super) fn finish(&self, m: &Manifest) -> Result<()> {
        // Selected CSV rows are subsets of received inputs. Exact equality requires
        // recorded complete receive coverage, independent of the configured log mode.
        let complete =
            m.counts.logger_overruns == 0 && m.counts.received == Some(m.counts.selected);
        let c = &m.counts;
        let check = |name, rows: u64, count: Option<u128>| -> Result<()> {
            if let Some(count) = count {
                ensure!(
                    u128::from(rows) <= count,
                    "CSV {name} rows exceed recorded received-input category"
                );
                ensure!(
                    !complete || u128::from(rows) == count,
                    "complete CSV {name} population differs from recorded received-input category"
                );
            }
            Ok(())
        };
        check("invalid", self.validity[1], c.invalid.map(u128::from))?;
        check(
            "unknown validity",
            self.validity[2],
            c.validity_unknown.map(u128::from),
        )?;
        check("stale", self.staleness[1], c.stale.map(u128::from))?;
        check(
            "unknown staleness",
            self.staleness[2],
            c.freshness_unknown.map(u128::from),
        )?;
        if let (Some(received), Some(invalid), Some(unknown)) =
            (c.received, c.invalid, c.validity_unknown)
        {
            let valid = u128::from(received)
                .checked_sub(u128::from(invalid) + u128::from(unknown))
                .context("validity category counts exceed received inputs")?;
            check("valid", self.validity[0], Some(valid))?;
        }
        if let (Some(received), Some(stale), Some(unknown)) =
            (c.received, c.stale, c.freshness_unknown)
        {
            let fresh = u128::from(received)
                .checked_sub(u128::from(stale) + u128::from(unknown))
                .context("staleness category counts exceed received inputs")?;
            check("not-stale", self.staleness[0], Some(fresh))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
