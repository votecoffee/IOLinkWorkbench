//! Offline, descriptive comparisons. Never align clocks or infer physical origin.
use super::{Inspected, InterfaceReview, inspect};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct ComparisonSetting {
    pub name: &'static str,
    pub left: Option<String>,
    pub right: Option<String>,
    pub relation: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ComparisonMetric {
    pub domain: &'static str,
    pub name: String,
    pub unit: &'static str,
    pub left: Option<u64>,
    pub right: Option<u64>,
    pub assessment: &'static str,
    /// Right relative to left; arithmetic only, never a performance verdict.
    pub change: &'static str,
    pub magnitude: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct CaptureComparison {
    pub format_version: u32,
    pub basis: &'static str,
    pub delta_policy: &'static str,
    pub timing_context: &'static str,
    pub count_context: &'static str,
    pub settings: Vec<ComparisonSetting>,
    pub metrics: Vec<ComparisonMetric>,
    pub left: InterfaceReview,
    pub right: InterfaceReview,
}

fn settings(data: &Inspected) -> Result<Vec<(&'static str, Option<String>)>> {
    let p = &data.profile;
    let m = &data.manifest;
    // Known profile absence is a setting, whereas missing observed metadata is unknown.
    let json = |value: serde_json::Value| Some(value.to_string());
    let optional = |value: serde_json::Value| (!value.is_null()).then(|| value.to_string());
    let api = m
        .local_api
        .as_ref()
        .map(|v| serde_json::json!([v.url, v.queue_capacity, v.history_capacity]));
    let mqtt = m
        .mqtt
        .as_ref()
        .map(|v| serde_json::json!([v.broker, v.topic, v.interval_ms, v.queue_capacity]));
    let shm = m
        .shared_memory
        .as_ref()
        .map(|v| serde_json::json!([v.name, v.interval_ms, v.queue_capacity]));
    Ok(vec![
        ("connection", json(serde_json::to_value(&p.connection)?)),
        (
            "expected_identity",
            json(serde_json::to_value(&p.expected_identity)?),
        ),
        ("input_bytes", json(serde_json::json!(p.input_bytes))),
        ("logging", json(serde_json::to_value(&p.logging)?)),
        (
            "effective_fields",
            json(serde_json::to_value(p.effective_fields())?),
        ),
        (
            "iodd_selection",
            json(serde_json::json!(p.iodd.as_ref().map(|v| (
                &v.variant,
                &v.layout,
                &v.condition,
                &v.communication,
                p.confirm_iodd_condition
            )))),
        ),
        (
            "recorded_route",
            optional(serde_json::json!(
                m.last_status.as_ref().and_then(|s| s.route.as_ref())
            )),
        ),
        (
            "freshness",
            optional(serde_json::to_value(
                m.last_status.as_ref().map(|s| s.freshness),
            )?),
        ),
        ("row_kind", optional(serde_json::json!(m.row_kind))),
        (
            "source_loss_meaning",
            json(serde_json::json!(m.source_update_loss)),
        ),
        (
            "requested_poll_us",
            optional(serde_json::json!(m.requested_poll_us)),
        ),
        (
            "stale_threshold_us",
            json(serde_json::json!(m.stale_threshold_us)),
        ),
        (
            "reported_port_cycle_us",
            json(serde_json::json!(
                m.last_status.as_ref().and_then(|s| s.port_cycle_us)
            )),
        ),
        (
            "simulation_faults",
            optional(serde_json::json!(m.simulation_faults)),
        ),
        (
            "simulated_writer_delay_ms",
            optional(serde_json::json!(m.simulated_writer_delay_ms)),
        ),
        ("http_api_configuration", json(serde_json::json!(api))),
        ("mqtt_configuration", json(serde_json::json!(mqtt))),
        ("shared_memory_configuration", json(serde_json::json!(shm))),
        ("elapsed_us", json(serde_json::json!(m.elapsed_us))),
    ])
}

fn relation<T: PartialEq>(left: &Option<T>, right: &Option<T>) -> &'static str {
    match (left, right) {
        (Some(a), Some(b)) if a == b => "matching",
        (Some(_), Some(_)) => "different",
        _ => "unavailable",
    }
}

fn context(settings: &[ComparisonSetting], counts: bool) -> &'static str {
    let relevant = |s: &&ComparisonSetting| counts || s.name != "elapsed_us";
    if settings
        .iter()
        .filter(relevant)
        .any(|s| s.relation == "different")
    {
        "settings_differ"
    } else if settings
        .iter()
        .filter(relevant)
        .any(|s| s.relation == "unavailable")
    {
        "settings_unavailable"
    } else {
        "matching_recorded_context"
    }
}

fn metric(
    domain: &'static str,
    name: String,
    unit: &'static str,
    left: Option<u64>,
    right: Option<u64>,
    context: &'static str,
) -> ComparisonMetric {
    let assessment = if left.is_none() || right.is_none() {
        "metric_unavailable"
    } else {
        context
    };
    let (change, magnitude) = match (left, right) {
        (Some(a), Some(b)) if assessment == "matching_recorded_context" => (
            match b.cmp(&a) {
                std::cmp::Ordering::Less => "decrease",
                std::cmp::Ordering::Equal => "equal",
                std::cmp::Ordering::Greater => "increase",
            },
            Some(a.abs_diff(b)),
        ),
        _ => ("not_compared", None),
    };
    ComparisonMetric {
        domain,
        name,
        unit,
        left,
        right,
        assessment,
        change,
        magnitude,
    }
}

fn review(data: Inspected) -> Result<InterfaceReview> {
    InterfaceReview::new(
        data.summary,
        &data.profile,
        &data.manifest,
        &data.profile_bytes,
        &data.manifest_bytes,
    )
}

fn check_timing(data: &Inspected) -> Result<()> {
    let m = &data.manifest;
    for spacing in [
        &m.host_poll_start_spacing,
        &m.host_receive_spacing,
        &m.host_new_update_spacing,
        &m.gateway_update_spacing,
    ]
    .into_iter()
    .flatten()
    {
        ensure!(
            spacing.count > 0
                && spacing.p50_us <= spacing.p95_us
                && spacing.p95_us <= spacing.p99_us
                && spacing.p99_us <= spacing.longest_us
                && spacing.mean_hz.is_finite()
                && spacing.mean_hz >= 0.0,
            "inconsistent recorded spacing statistics"
        );
    }
    Ok(())
}

/// Each side is verified in its own complete streaming pass. Source files are never written.
pub fn compare_captures(left: &Path, right: &Path) -> Result<CaptureComparison> {
    let left = inspect(left, None, None, None).context("left capture verification failed")?;
    let right = inspect(right, None, None, None).context("right capture verification failed")?;
    check_timing(&left).context("left capture timing failed")?;
    check_timing(&right).context("right capture timing failed")?;
    let settings: Vec<_> = settings(&left)?
        .into_iter()
        .zip(settings(&right)?)
        .map(|((name, left), (_, right))| ComparisonSetting {
            name,
            relation: relation(&left, &right),
            left,
            right,
        })
        .collect();
    let mut timing_context = context(&settings, false);
    let mut count_context = context(&settings, true);
    let left = review(left)?;
    let right = review(right)?;
    let failed = |v: &InterfaceReview| {
        v.capture.capture_had_error
            || v.interfaces
                .local_api
                .as_ref()
                .is_some_and(|s| s.worker_failed)
            || v.interfaces.mqtt.as_ref().is_some_and(|s| s.worker_failed)
            || v.interfaces
                .shared_memory
                .as_ref()
                .is_some_and(|s| s.worker_failed)
    };
    if failed(&left) || failed(&right) {
        timing_context = "capture_or_worker_error";
        count_context = "capture_or_worker_error";
    }
    let mut metrics = Vec::new();
    macro_rules! counts {
        ($domain:literal, $a:expr, $b:expr, $($field:ident),+) => { $(metrics.push(metric($domain, stringify!($field).into(), "count", $a.map(|v| v.$field), $b.map(|v| v.$field), count_context));)+ };
    }
    counts!(
        "csv",
        Some(&left.capture),
        Some(&right.capture),
        rows_checked,
        valid_rows,
        invalid_rows,
        unknown_validity_rows,
        stale_rows,
        missing_sample_indexes,
        generations_in_csv
    );
    for (name, a, b) in [
        (
            "received",
            left.acquisition.recorded_received,
            right.acquisition.recorded_received,
        ),
        (
            "disconnects",
            left.acquisition.recorded_disconnects,
            right.acquisition.recorded_disconnects,
        ),
        (
            "reconnects",
            Some(left.acquisition.recorded_reconnects),
            Some(right.acquisition.recorded_reconnects),
        ),
        (
            "source_gaps",
            left.capture.recorded_source_gaps,
            right.capture.recorded_source_gaps,
        ),
    ] {
        metrics.push(metric(
            "acquisition",
            name.into(),
            "count",
            a,
            b,
            count_context,
        ));
    }
    metrics.push(metric(
        "csv",
        "logger_overruns".into(),
        "count",
        Some(left.acquisition.recorded_logger_overruns),
        Some(right.acquisition.recorded_logger_overruns),
        count_context,
    ));
    counts!(
        "http_api",
        left.interfaces.local_api.as_ref(),
        right.interfaces.local_api.as_ref(),
        events_offered,
        events_ingested,
        queue_dropped,
        history_evicted,
        requests,
        failed_requests
    );
    counts!(
        "mqtt",
        left.interfaces.mqtt.as_ref(),
        right.interfaces.mqtt.as_ref(),
        events_offered,
        queue_dropped,
        coalesced,
        shutdown_discarded,
        socket_writes_completed,
        publish_write_failures,
        encoding_failures,
        broker_connect_attempts,
        broker_connections,
        connect_failures,
        connection_losses
    );
    counts!(
        "shared_memory",
        left.interfaces.shared_memory.as_ref(),
        right.interfaces.shared_memory.as_ref(),
        events_offered,
        queue_dropped,
        coalesced,
        busy_skips,
        snapshots_written,
        snapshots_replaced,
        encoding_failures,
        write_failures,
        abandoned_recoveries
    );
    let a = &left.acquisition.recorded_timing;
    let b = &right.acquisition.recorded_timing;
    for (name, a, b) in [
        (
            "host_poll_start",
            a.host_poll_start_spacing.as_ref(),
            b.host_poll_start_spacing.as_ref(),
        ),
        (
            "host_receive",
            a.host_receive_spacing.as_ref(),
            b.host_receive_spacing.as_ref(),
        ),
        (
            "host_new_update",
            a.host_new_update_spacing.as_ref(),
            b.host_new_update_spacing.as_ref(),
        ),
        (
            "gateway_update",
            a.gateway_update_spacing.as_ref(),
            b.gateway_update_spacing.as_ref(),
        ),
    ] {
        metrics.push(metric(
            name,
            "observations".into(),
            "count",
            a.map(|v| v.count),
            b.map(|v| v.count),
            count_context,
        ));
        macro_rules! quantile { ($($q:ident),+) => { $(metrics.push(metric(name, stringify!($q).into(), "us", a.map(|v|v.$q), b.map(|v|v.$q), timing_context));)+ }; }
        quantile!(p50_us, p95_us, p99_us, longest_us);
    }
    let a = a.logger_metrics.as_ref();
    let b = b.logger_metrics.as_ref();
    metrics.push(metric(
        "logger",
        "final_drain_us".into(),
        "us",
        a.map(|v| v.final_drain_us),
        b.map(|v| v.final_drain_us),
        timing_context,
    ));
    metrics.push(metric(
        "logger",
        "queue_high_water_observed".into(),
        "rows",
        a.map(|v| v.queue_high_water_observed as u64),
        b.map(|v| v.queue_high_water_observed as u64),
        timing_context,
    ));
    for (name, a, b) in [
        (
            "logger_queue_wait",
            a.and_then(|v| v.queue_wait.as_ref()),
            b.and_then(|v| v.queue_wait.as_ref()),
        ),
        (
            "logger_row_service",
            a.and_then(|v| v.row_service.as_ref()),
            b.and_then(|v| v.row_service.as_ref()),
        ),
        (
            "logger_flush",
            a.and_then(|v| v.flush.as_ref()),
            b.and_then(|v| v.flush.as_ref()),
        ),
        (
            "logger_sync",
            a.and_then(|v| v.sync.as_ref()),
            b.and_then(|v| v.sync.as_ref()),
        ),
    ] {
        macro_rules! quantile { ($($q:ident),+) => { $(metrics.push(metric(name, stringify!($q).into(), "us", a.map(|v|v.$q), b.map(|v|v.$q), timing_context));)+ }; }
        quantile!(p50_us, p95_us, p99_us, longest_us);
    }
    Ok(CaptureComparison {
        format_version: 1,
        basis: "offline descriptive comparison of two verified saved captures; recorded configuration/route is not physical-origin proof; no live endpoint or IPC opened; independent clocks are not aligned; no combined loss score or performance verdict",
        delta_policy: "right relative to left, only for available metrics with matching required recorded settings; counts also require identical elapsed_us; unknown stays unknown; no rate normalization, interpolation, significance test or 1 ms acceptance",
        timing_context,
        count_context,
        settings,
        metrics,
        left,
        right,
    })
}

#[cfg(test)]
mod tests;
