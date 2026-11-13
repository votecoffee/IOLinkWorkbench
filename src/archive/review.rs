use super::{Manifest, Row, Summary};
use crate::{
    capture::{CSV_METADATA, Spacing},
    decode::Field,
    profile::{Gateway, LogMode, Profile},
};
use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct NumericSummary {
    pub finite_values: u64,
    pub blank_values: u64,
    pub non_finite_values: u64,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub mean: Option<f64>,
    pub first_csv_value: Option<String>,
    pub last_csv_value: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Bucket {
    pub bin: usize,
    pub first_host_us: Option<u64>,
    pub last_host_us: Option<u64>,
    pub first_generation: Option<u64>,
    pub last_generation: Option<u64>,
    pub rows: u64,
    pub blank_values: u64,
    pub non_finite_values: u64,
    pub finite_values: u64,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub last: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct RecordedTiming {
    pub port_cycle_raw: Option<String>,
    pub port_cycle_us: Option<u64>,
    pub host_poll_start_spacing: Option<Spacing>,
    pub host_receive_spacing: Option<Spacing>,
    pub host_new_update_spacing: Option<Spacing>,
    pub gateway_update_spacing: Option<Spacing>,
    pub logger_metrics: Option<crate::capture::LoggerMetrics>,
}

#[derive(Debug, Serialize)]
pub struct Review {
    pub capture: Summary,
    pub signal: SignalReview,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interfaces: Option<super::RecordedInterfaces>,
}

#[derive(Debug, Serialize)]
pub struct SignalReview {
    pub configured_gateway: Gateway,
    pub recorded_route: Option<String>,
    pub simulation_faults: Option<bool>,
    pub simulated_writer_delay_ms: Option<u64>,
    pub log_mode: LogMode,
    pub elapsed_us: u64,
    pub requested_poll_us: Option<u64>,
    pub stale_threshold_us: u64,
    pub recorded_logger_overruns: u64,
    pub recorded_timing: RecordedTiming,
    pub cycle_context: crate::timing::Context,
    pub available_fields: Vec<Field>,
    pub field: Field,
    pub numeric_basis: &'static str,
    pub numeric: NumericSummary,
    pub buckets_requested: usize,
    pub buckets: Vec<Bucket>,
}

pub(super) struct Collector {
    column: usize,
    signal: SignalReview,
}

fn minimum(current: Option<f64>, value: f64) -> Option<f64> {
    Some(current.map_or(value, |old| old.min(value)))
}

fn maximum(current: Option<f64>, value: f64) -> Option<f64> {
    Some(current.map_or(value, |old| old.max(value)))
}

impl Collector {
    pub fn new(
        profile: &Profile,
        manifest: &Manifest,
        field: Option<&str>,
        buckets: usize,
    ) -> Result<Self> {
        let fields = profile.log_fields();
        let column = match field {
            Some(name) => fields
                .iter()
                .position(|field| field.name == name)
                .with_context(|| {
                    format!("field {name:?} is absent from the saved logging selection")
                })?,
            None => 0,
        };
        let field = fields
            .get(column)
            .context("capture has no logged typed fields; raw CSV remains available")?;
        let status = manifest.last_status.as_ref();
        Ok(Self {
            column: CSV_METADATA.len() + column,
            signal: SignalReview {
                configured_gateway: profile.connection.gateway,
                recorded_route: status.and_then(|status| status.route.clone()),
                simulation_faults: manifest.simulation_faults,
                simulated_writer_delay_ms: manifest.simulated_writer_delay_ms,
                log_mode: profile.logging.mode,
                elapsed_us: manifest.elapsed_us,
                requested_poll_us: manifest.requested_poll_us,
                stale_threshold_us: manifest.stale_threshold_us,
                recorded_logger_overruns: manifest.counts.logger_overruns,
                cycle_context: crate::timing::assess(
                    profile,
                    manifest.requested_poll_us,
                    crate::timing::Observation {
                        port_cycle_us: status.and_then(|status| status.port_cycle_us),
                        freshness: status.map(|status| status.freshness),
                        route: status.and_then(|status| status.route.as_deref()),
                    },
                    1000,
                )?,
                recorded_timing: RecordedTiming {
                    port_cycle_raw: status.and_then(|status| status.port_cycle_raw.clone()),
                    port_cycle_us: status.and_then(|status| status.port_cycle_us),
                    host_poll_start_spacing: manifest.host_poll_start_spacing.clone(),
                    host_receive_spacing: manifest.host_receive_spacing.clone(),
                    host_new_update_spacing: manifest.host_new_update_spacing.clone(),
                    gateway_update_spacing: manifest.gateway_update_spacing.clone(),
                    logger_metrics: manifest.logger_metrics.clone(),
                },
                available_fields: fields.iter().map(|field| (*field).clone()).collect(),
                field: (*field).clone(),
                numeric_basis: "approximate f64 statistics of saved typed CSV cells; Boolean false/true = 0/1; blank and non-finite cells excluded; first/last retain nonblank CSV text; original CSV unchanged",
                numeric: NumericSummary::default(),
                buckets_requested: buckets,
                buckets: (0..buckets)
                    .map(|bin| Bucket {
                        bin,
                        ..Default::default()
                    })
                    .collect(),
            },
        })
    }

    pub fn add(&mut self, row: &Row<'_>, record: &csv::StringRecord) {
        let review = &mut self.signal;
        let bin = (u128::from(row.host_elapsed_us) * review.buckets_requested as u128
            / (u128::from(review.elapsed_us) + 1)) as usize;
        let bucket = &mut review.buckets[bin];
        bucket.rows += 1;
        bucket.first_host_us.get_or_insert(row.host_elapsed_us);
        bucket.last_host_us = Some(row.host_elapsed_us);
        bucket.first_generation.get_or_insert(row.generation);
        bucket.last_generation = Some(row.generation);
        let cell = &record[self.column];
        if cell.is_empty() {
            review.numeric.blank_values += 1;
            bucket.blank_values += 1;
            return;
        }
        review
            .numeric
            .first_csv_value
            .get_or_insert_with(|| cell.to_owned());
        review.numeric.last_csv_value = Some(cell.to_owned());
        let value = match cell {
            "false" => 0.0,
            "true" => 1.0,
            // The shared verifier already checked this cell against the numeric decoder.
            _ => cell.parse::<f64>().unwrap_or(f64::NAN),
        };
        if !value.is_finite() {
            review.numeric.non_finite_values += 1;
            bucket.non_finite_values += 1;
            return;
        }
        let numeric = &mut review.numeric;
        numeric.finite_values += 1;
        numeric.minimum = minimum(numeric.minimum, value);
        numeric.maximum = maximum(numeric.maximum, value);
        let old = numeric.mean.unwrap_or(value);
        let weight = 1.0 / numeric.finite_values as f64;
        // Avoid overflowing the difference between finite values of opposite signs.
        numeric.mean = Some(if old.is_sign_negative() == value.is_sign_negative() {
            old + (value - old) * weight
        } else {
            old * (1.0 - weight) + value * weight
        });
        bucket.finite_values += 1;
        bucket.minimum = minimum(bucket.minimum, value);
        bucket.maximum = maximum(bucket.maximum, value);
        bucket.last = Some(value);
    }

    pub fn finish(self, summary: Summary) -> Review {
        Review {
            capture: summary,
            signal: self.signal,
            interfaces: None,
        }
    }
}

#[cfg(test)]
mod tests;
