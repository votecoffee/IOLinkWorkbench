use super::Distribution;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

/// Monotonic host operation duration, separate from source or receive spacing.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Latency {
    pub count: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub longest_us: u64,
    pub mean_us: f64,
}

impl Distribution {
    pub(super) fn latency(&self) -> Option<Latency> {
        let count = self.histogram.len();
        (count > 0).then(|| Latency {
            count,
            p50_us: self.histogram.value_at_quantile(0.50).min(self.longest),
            p95_us: self.histogram.value_at_quantile(0.95).min(self.longest),
            p99_us: self.histogram.value_at_quantile(0.99).min(self.longest),
            longest_us: self.longest,
            mean_us: self.total_us as f64 / count as f64,
        })
    }
}

impl Latency {
    fn validate(&self) -> Result<()> {
        ensure!(self.count > 0, "logger latency has no attempts");
        ensure!(
            self.p50_us <= self.p95_us
                && self.p95_us <= self.p99_us
                && self.p99_us <= self.longest_us,
            "logger latency quantiles are inconsistent"
        );
        ensure!(
            self.mean_us.is_finite()
                && self.mean_us >= 0.0
                && self.mean_us <= self.longest_us as f64,
            "logger latency mean is inconsistent"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LoggerMetrics {
    pub queue_capacity: usize,
    /// Lower bound from observations after accepted pushes, and full-queue failures.
    pub queue_high_water_observed: usize,
    pub rows_dequeued: u64,
    pub final_drain_us: u64,
    pub queue_wait: Option<Latency>,
    pub row_service: Option<Latency>,
    pub flush: Option<Latency>,
    pub sync: Option<Latency>,
}

impl LoggerMetrics {
    pub(crate) fn validate(
        &self,
        capacity: usize,
        accepted: u64,
        flushed: u64,
        parts: u64,
        writer_failed: bool,
    ) -> Result<()> {
        ensure!(
            self.queue_capacity == capacity && self.queue_high_water_observed <= capacity,
            "logger queue metrics differ from the saved capacity"
        );
        ensure!(
            flushed <= self.rows_dequeued && self.rows_dequeued <= accepted,
            "logger dequeued-row accounting is inconsistent"
        );
        ensure!(
            self.rows_dequeued == 0 || self.queue_high_water_observed > 0,
            "logger reports rows without observed queue occupancy"
        );
        for latency in [&self.queue_wait, &self.row_service, &self.flush, &self.sync]
            .into_iter()
            .flatten()
        {
            latency.validate()?;
        }
        for latency in [&self.queue_wait, &self.row_service] {
            ensure!(
                latency.as_ref().map_or(0, |value| value.count) == self.rows_dequeued,
                "logger row timing count differs from dequeued rows"
            );
        }
        let flushes = self.flush.as_ref().map_or(0, |value| value.count);
        let syncs = self.sync.as_ref().map_or(0, |value| value.count);
        ensure!(
            flushes >= syncs,
            "logger sync attempts exceed flush attempts"
        );
        if !writer_failed {
            ensure!(
                self.rows_dequeued == accepted && flushes > 0 && syncs == parts && parts > 0,
                "completed logger metrics are incomplete"
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_quantiles_measure_durations_without_inventing_receive_rates() {
        let mut distribution = Distribution::new();
        assert!(distribution.latency().is_none());
        for value in [0, 10, 20, 30, 100] {
            distribution.add(value);
        }
        let latency = distribution.latency().unwrap();
        assert_eq!(latency.count, 5);
        assert_eq!(
            (
                latency.p50_us,
                latency.p95_us,
                latency.p99_us,
                latency.longest_us
            ),
            (20, 100, 100, 100)
        );
        assert_eq!(latency.mean_us, 32.0);
        latency.validate().unwrap();
    }
}
