use crate::{
    profile::{LogMode, Profile},
    transport::{self, Freshness, Input, InputTransport, Status, Validity},
};
use anyhow::{Context, Result, ensure};
use chrono::{SecondsFormat, Utc};
use crossbeam_queue::ArrayQueue;
use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

mod deadline;
mod logger;
mod writer;
pub use logger::{Latency, LoggerMetrics};

pub(crate) const CSV_METADATA: &[&str] = &[
    "capture_utc_start",
    "host_elapsed_us",
    "sample_index",
    "generation",
    "row_kind",
    "validity",
    "stale",
    "source_counter",
    "source_time_us",
    "source_age_us",
    "diagnostic_code",
    "raw_hex",
];

pub(crate) fn csv_header(profile: &Profile) -> Vec<String> {
    let mut header = CSV_METADATA
        .iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    header.extend(profile.log_fields().into_iter().map(|field| {
        if field.unit.is_empty() {
            format!("value.{}", field.name)
        } else {
            format!("value.{} [{}]", field.name, field.unit)
        }
    }));
    header
}

#[derive(Clone, Default)]
pub struct Stop(Arc<AtomicBool>);

impl Stop {
    pub fn request(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn requested(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Default)]
pub struct Options {
    pub output: Option<PathBuf>,
    pub duration: Option<Duration>,
    pub simulate_faults: bool,
    pub simulated_writer_delay: Duration,
    pub local_api: Option<crate::local_api::Config>,
    pub mqtt: Option<crate::mqtt::Config>,
    pub shared_memory: Option<crate::shared_memory::Config>,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Counts {
    pub poll_attempts: u64,
    pub received: u64,
    pub invalid: u64,
    pub validity_unknown: u64,
    pub stale: u64,
    pub freshness_unknown: u64,
    pub duplicate: u64,
    pub out_of_order: u64,
    pub new_updates_observed: u64,
    pub disconnects: u64,
    pub reconnects: u64,
    pub transport_errors: u64,
    pub skipped_host_deadlines: u64,
    pub unchanged: u64,
    pub selected: u64,
    pub logger_overruns: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Spacing {
    pub count: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub longest_us: u64,
    pub mean_hz: f64,
}

impl Spacing {
    /// Consistency of recorded aggregates, not a remeasurement or timing guarantee.
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(self.count > 0, "spacing has no observations");
        ensure!(
            self.p50_us <= self.p95_us
                && self.p95_us <= self.p99_us
                && self.p99_us <= self.longest_us,
            "spacing quantiles are inconsistent"
        );
        ensure!(
            self.mean_hz.is_finite() && self.mean_hz >= 0.0,
            "spacing frequency is invalid"
        );
        if self.longest_us == 0 {
            // Distribution uses zero when the sum of measured integer microseconds is zero.
            ensure!(
                self.mean_hz == 0.0,
                "zero-duration spacing has a nonzero frequency"
            );
        } else {
            let lower = 1_000_000.0 / self.longest_us as f64;
            let upper = self.count as f64 * lower;
            ensure!(
                self.mean_hz >= lower * (1.0 - 1e-9) && self.mean_hz <= upper * (1.0 + 1e-9),
                "spacing frequency is inconsistent with its count and longest interval"
            );
        }
        Ok(())
    }
}

struct Distribution {
    histogram: Histogram<u64>,
    longest: u64,
    total_us: u128,
}

impl Distribution {
    fn new() -> Self {
        Self {
            histogram: Histogram::new(3).expect("valid histogram precision"),
            longest: 0,
            total_us: 0,
        }
    }

    fn add(&mut self, us: u64) {
        self.longest = self.longest.max(us);
        self.total_us += u128::from(us);
        self.histogram.record(us).expect("auto-resizing histogram");
    }

    fn summarize(&self) -> Option<Spacing> {
        let count = self.histogram.len();
        (count != 0).then(|| Spacing {
            count,
            p50_us: self.histogram.value_at_quantile(0.50).min(self.longest),
            p95_us: self.histogram.value_at_quantile(0.95).min(self.longest),
            p99_us: self.histogram.value_at_quantile(0.99).min(self.longest),
            longest_us: self.longest,
            mean_hz: if self.total_us == 0 {
                0.0
            } else {
                count as f64 * 1_000_000.0 / self.total_us as f64
            },
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct Live {
    pub status: Option<Status>,
    pub identity_check: Option<&'static str>,
    pub input: Option<Input>,
    pub last_new_at: Option<Instant>,
    pub stale: Option<bool>,
    pub generation: u64,
    pub connected: bool,
    pub message: String,
    pub counts: Counts,
}

pub type LiveQueue = Arc<ArrayQueue<Live>>;

#[derive(Debug, Serialize)]
pub struct Report {
    pub format_version: u32,
    pub profile: String,
    pub utc_start: String,
    pub elapsed_us: u64,
    pub stopped_by_request: bool,
    pub ended_connected: bool,
    pub last_status: Option<Status>,
    pub rejected_status: Option<Status>,
    pub identity_check: Option<&'static str>,
    pub row_kind: Option<&'static str>,
    pub requested_poll_us: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iodd_communication: Option<crate::iodd::Communication>,
    pub stale_threshold_us: u64,
    pub requested_duration_ms: Option<u64>,
    pub simulation_faults: bool,
    pub simulated_writer_delay_ms: u64,
    pub source_update_loss: &'static str,
    pub source_gaps_observed: Option<u64>,
    pub last_error: Option<String>,
    pub terminal_error: Option<String>,
    pub writer_error: Option<String>,
    pub csv_rows_flushed: u64,
    pub csv_rows_unconfirmed: u64,
    pub csv_parts: u64,
    pub logger_metrics: Option<LoggerMetrics>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_api: Option<crate::local_api::Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mqtt: Option<crate::mqtt::Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_memory: Option<crate::shared_memory::Summary>,
    pub counts: Counts,
    pub host_timer: &'static str,
    pub host_poll_start_spacing: Option<Spacing>,
    pub host_receive_spacing: Option<Spacing>,
    pub host_new_update_spacing: Option<Spacing>,
    pub gateway_update_spacing: Option<Spacing>,
    pub time_since_last_receive_us: Option<u64>,
}

impl Report {
    pub fn success(&self) -> bool {
        self.terminal_error.is_none()
            && self.writer_error.is_none()
            && self.local_api.as_ref().is_none_or(|api| !api.worker_failed)
            && self.mqtt.as_ref().is_none_or(|mqtt| !mqtt.worker_failed)
            && self
                .shared_memory
                .as_ref()
                .is_none_or(|shm| !shm.worker_failed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Update {
    First,
    Forward(u32),
    Same,
    Backward,
    Snapshot,
}

#[derive(Default)]
struct FreshnessState {
    counter: Option<u32>,
    source_time_us: Option<u64>,
    last_new: Option<Instant>,
    previous_arrival: Option<Instant>,
}

impl FreshnessState {
    fn stale(&self, input: &Input, threshold_us: u64) -> Option<bool> {
        let observed_age = self
            .last_new
            .map(|last| us(input.received.saturating_duration_since(last)));
        input
            .source_age_us
            .into_iter()
            .chain(observed_age)
            .max()
            .map(|age| age > threshold_us)
    }

    fn classify(&self, freshness: Freshness, input: &Input) -> Result<Update> {
        Ok(match freshness {
            Freshness::Snapshots => Update::Snapshot,
            Freshness::Counter32 => {
                let count = input
                    .counter
                    .context("gateway omitted promised update counter")?;
                match self.counter.map(|old| count.wrapping_sub(old)) {
                    None => Update::First,
                    Some(0) => Update::Same,
                    Some(n) if n < (1 << 31) => Update::Forward(n),
                    Some(_) => Update::Backward,
                }
            }
            Freshness::Timestamp => {
                let time = input
                    .source_time_us
                    .context("gateway omitted promised update timestamp")?;
                match self.source_time_us {
                    None => Update::First,
                    Some(old) if time > old => Update::Forward(1),
                    Some(old) if time == old => Update::Same,
                    _ => Update::Backward,
                }
            }
        })
    }
}

#[derive(Clone, Copy)]
struct Row {
    input: Input,
    elapsed_us: u64,
    sample_index: u64,
    generation: u64,
    freshness: Freshness,
    stale: Option<bool>,
}

fn us(time: Duration) -> u64 {
    time.as_micros().try_into().unwrap_or(u64::MAX)
}

fn wait_until(wait: &mut deadline::DeadlineWait, target: Instant, stop: &Stop) {
    while !stop.requested() {
        let remaining = target.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        wait.wait(remaining.min(Duration::from_millis(20)));
    }
}

pub fn probe(profile: &Profile) -> Result<(Status, &'static str)> {
    let status = transport::open(profile, false)?.connect()?;
    let check = profile.expected_identity.verify(&status.identity)?;
    Ok((status, check))
}

pub fn run(
    profile: &Profile,
    options: &Options,
    stop: &Stop,
    live: Option<&LiveQueue>,
) -> Result<Report> {
    if profile.connection.gateway == crate::profile::Gateway::IfmAl1350
        && let Some(condition) = profile
            .iodd
            .as_ref()
            .and_then(|iodd| iodd.condition.as_ref())
    {
        ensure!(
            profile.confirm_iodd_condition,
            "confirm the active IODD layout condition {condition} before capture"
        );
    }
    let transport = transport::open(profile, options.simulate_faults)?;
    ensure!(
        profile.connection.gateway == crate::profile::Gateway::Simulator
            || options.simulated_writer_delay.is_zero(),
        "writer delay is simulator-only"
    );
    run_with_transport(profile, options, stop, live, transport)
}

fn run_with_transport(
    profile: &Profile,
    options: &Options,
    stop: &Stop,
    live: Option<&LiveQueue>,
    mut transport: Box<dyn InputTransport>,
) -> Result<Report> {
    let start = Instant::now();
    let utc_start = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let local_api = options
        .local_api
        .as_ref()
        .map(|config| crate::local_api::Service::start(config, profile, start, &utc_start))
        .transpose()?;
    let mqtt = options
        .mqtt
        .as_ref()
        .map(|config| crate::mqtt::Service::start(config, profile, start, &utc_start))
        .transpose()?;
    let shared_memory = options
        .shared_memory
        .as_ref()
        .map(|config| crate::shared_memory::Service::start(config, profile, start, &utc_start))
        .transpose()?;
    let deadline = options
        .duration
        .and_then(|duration| start.checked_add(duration));
    ensure!(
        options.duration.is_none() || deadline.is_some(),
        "capture duration overflow"
    );
    let writer = options
        .output
        .as_ref()
        .map(|path| {
            writer::Writer::start(
                path,
                profile,
                utc_start.clone(),
                options.simulated_writer_delay,
            )
        })
        .transpose()?;
    let poll = Duration::from_micros(profile.connection.poll_us);
    let retry = Duration::from_millis(profile.connection.reconnect_ms);
    let mut state = Live::default();
    let mut last_host = None::<Instant>;
    let mut timer = deadline::DeadlineWait::new();
    let mut last_poll_start = None::<Instant>;
    let mut poll_start = Distribution::new();
    let mut host = Distribution::new();
    let mut host_new = Distribution::new();
    let mut gateway = Distribution::new();
    let mut gaps = 0u64;
    let mut previous_key = None;
    let mut freshness_state = FreshnessState::default();
    let mut route_pin = None::<Status>;
    let mut rejected_status = None;
    let mut last_error = None;
    let mut terminal_error = None;
    let mut last_ui = start;
    let mut next = start;

    loop {
        if stop.requested()
            || deadline.is_some_and(|end| Instant::now() >= end)
            || writer.as_ref().is_some_and(writer::Writer::failed)
        {
            break;
        }
        wait_until(&mut timer, deadline.map_or(next, |end| next.min(end)), stop);
        if stop.requested() || deadline.is_some_and(|end| Instant::now() >= end) {
            break;
        }

        if !state.connected {
            match transport.connect() {
                Ok(status) => {
                    let check = profile.expected_identity.verify(&status.identity).and_then(
                        |check| {
                            if let Some(pin) = &route_pin {
                                ensure!(
                                    pin.identity == status.identity
                                        && pin.master_serial == status.master_serial
                                        && pin.model == status.model
                                        && pin.firmware == status.firmware
                                        && pin.freshness == status.freshness,
                                    "gateway, firmware, or device identity changed after reconnect"
                                );
                            }
                            Ok(check)
                        },
                    );
                    match check {
                        Ok(check) => state.identity_check = Some(check),
                        Err(error) => {
                            rejected_status = Some(status);
                            state.message = error.to_string();
                            terminal_error = Some(error.to_string());
                            break;
                        }
                    }
                    state.status = Some(status.clone());
                    if state.generation != 0 {
                        state.counts.reconnects += 1;
                    }
                    state.generation += 1;
                    state.connected = true;
                    if let Some(api) = &local_api {
                        api.connected(state.generation, &status);
                    }
                    if let Some(mqtt) = &mqtt {
                        mqtt.connected(state.generation, &status);
                    }
                    if let Some(shm) = &shared_memory {
                        shm.connected(state.generation, &status);
                    }
                    state.message.clear();
                    state.message.push_str("Connected read-only");
                    route_pin = Some(status);
                    previous_key = None;
                    freshness_state = FreshnessState::default();
                    next = Instant::now();
                }
                Err(error) => {
                    state.counts.transport_errors += 1;
                    state.message = format!("{error:#}");
                    last_error = Some(state.message.clone());
                    if state.generation == 0 {
                        terminal_error = last_error.clone();
                        break;
                    }
                    next = Instant::now() + retry;
                }
            }
            if let Some(live) = live {
                live.force_push(state.clone());
            }
            continue;
        }

        let freshness = state.status.as_ref().expect("connected status").freshness;
        let poll_at = Instant::now();
        if let Some(previous) = last_poll_start.replace(poll_at) {
            poll_start.add(us(poll_at.duration_since(previous)));
        }
        state.counts.poll_attempts += 1;
        let result = transport.poll().and_then(|input| {
            let update = freshness_state.classify(freshness, &input)?;
            Ok((input, update))
        });
        let (mut input, update) = match result {
            Ok(sample) => sample,
            Err(error) => {
                state.counts.transport_errors += 1;
                state.counts.disconnects += 1;
                state.connected = false;
                state.message = format!("{error:#}");
                last_error = Some(state.message.clone());
                state.input = None;
                state.last_new_at = None;
                state.stale = None;
                if let Some(api) = &local_api {
                    api.disconnected(state.generation);
                }
                if let Some(mqtt) = &mqtt {
                    mqtt.disconnected(state.generation);
                }
                if let Some(shm) = &shared_memory {
                    shm.disconnected(state.generation);
                }
                if let Some(live) = live {
                    live.force_push(state.clone());
                }
                next = Instant::now() + retry;
                continue;
            }
        };
        if deadline.is_some_and(|end| input.received >= end) || stop.requested() {
            break;
        }
        let stamp = input.received;
        state.counts.received += 1;
        if let Some(prior) = last_host.replace(stamp) {
            host.add(us(stamp.saturating_duration_since(prior)));
        }
        if input.validity == Validity::Valid && input.payload.bytes().len() != profile.input_bytes {
            input.validity = Validity::Invalid;
            state.message = format!(
                "Expected {} input bytes, received {}",
                profile.input_bytes,
                input.payload.bytes().len()
            );
        } else {
            state.message.clear();
            state.message.push_str("Connected read-only");
        }
        match input.validity {
            Validity::Invalid => state.counts.invalid += 1,
            Validity::Unknown => state.counts.validity_unknown += 1,
            Validity::Valid => {}
        }
        let eligible = match update {
            Update::Same => {
                state.counts.duplicate += 1;
                false
            }
            Update::Backward => {
                state.counts.out_of_order += 1;
                false
            }
            Update::Snapshot => true,
            Update::First | Update::Forward(_) => {
                state.counts.new_updates_observed += 1;
                if let Update::Forward(step) = update {
                    if freshness == Freshness::Counter32 {
                        gaps += u64::from(step - 1);
                    }
                    if step == 1
                        && let (Some(old), Some(new)) =
                            (freshness_state.source_time_us, input.source_time_us)
                        && new > old
                    {
                        gateway.add(new - old);
                    }
                }
                if let Some(prior) = freshness_state.previous_arrival.replace(stamp) {
                    host_new.add(us(stamp.saturating_duration_since(prior)));
                }
                freshness_state.counter = input.counter;
                freshness_state.source_time_us = input.source_time_us;
                freshness_state.last_new = Some(stamp);
                true
            }
        };
        let stale = freshness_state.stale(&input, profile.connection.stale_us);
        match stale {
            Some(true) => state.counts.stale += 1,
            None => state.counts.freshness_unknown += 1,
            Some(false) => {}
        }
        // A late update cannot replace the displayed payload or form a log row.
        if update != Update::Backward {
            state.input = Some(input);
            state.last_new_at = freshness_state.last_new;
            state.stale = stale;
        }
        let mut csv_sample_index = None;
        let mut csv_enqueued = None;
        if eligible && let Some(writer) = &writer {
            let key = (input.payload, input.validity, stale, input.diagnostic_code);
            if profile.logging.mode == LogMode::OnChange && previous_key == Some(key) {
                state.counts.unchanged += 1;
            } else {
                state.counts.selected += 1;
                let row = Row {
                    input,
                    elapsed_us: us(stamp.saturating_duration_since(start)),
                    sample_index: state.counts.selected,
                    generation: state.generation,
                    freshness,
                    stale,
                };
                csv_sample_index = Some(row.sample_index);
                let enqueued = writer.enqueue(row);
                csv_enqueued = Some(enqueued);
                if !enqueued {
                    state.counts.logger_overruns += 1;
                } else {
                    previous_key = Some(key);
                }
            }
        }
        if local_api.is_some() || mqtt.is_some() || shared_memory.is_some() {
            let sample = crate::telemetry::Sample {
                input,
                freshness,
                stale,
                classification: match update {
                    Update::First => "first",
                    Update::Forward(_) => "forward",
                    Update::Same => "duplicate",
                    Update::Backward => "out_of_order",
                    Update::Snapshot => "host_poll",
                },
                csv_sample_index,
                csv_enqueued,
                counts: state.counts,
                source_gaps: (freshness == Freshness::Counter32).then_some(gaps),
            };
            if let Some(api) = &local_api {
                api.sample(state.generation, sample);
            }
            if let Some(mqtt) = &mqtt {
                mqtt.sample(state.generation, sample);
            }
            if let Some(shm) = &shared_memory {
                shm.sample(state.generation, sample);
            }
        }
        if stamp.saturating_duration_since(last_ui) >= Duration::from_millis(80) {
            if let Some(live) = live {
                live.force_push(state.clone());
            }
            last_ui = stamp;
        }
        next += poll;
        let now = Instant::now();
        if now > next {
            let skipped = us(now.duration_since(next)) / profile.connection.poll_us + 1;
            state.counts.skipped_host_deadlines += skipped;
            next += Duration::from_micros(skipped * profile.connection.poll_us);
        }
    }

    if state.counts.received == 0 && !stop.requested() && terminal_error.is_none() {
        terminal_error = Some("no input responses arrived inside the capture window".into());
    }
    if !state.connected && !stop.requested() && terminal_error.is_none() {
        terminal_error = Some("capture ended during a disconnect".into());
    }
    if let Some(live) = live {
        live.force_push(state.clone());
    }
    let end = Instant::now();
    let summary = writer.map(writer::Writer::finish).unwrap_or_default();
    let local_api = local_api.map(|api| {
        api.ended(state.generation);
        api.finish()
    });
    let mqtt = mqtt.map(|mqtt| {
        mqtt.ended(state.generation);
        mqtt.finish()
    });
    let shared_memory = shared_memory.map(|shm| {
        shm.ended(state.generation);
        shm.finish()
    });
    let freshness = state.status.as_ref().map(|status| status.freshness);
    let report = Report {
        format_version: 1,
        profile: profile.name.clone(),
        utc_start,
        elapsed_us: us(end.duration_since(start)),
        stopped_by_request: stop.requested(),
        ended_connected: state.connected,
        last_status: state.status,
        rejected_status,
        identity_check: state.identity_check,
        row_kind: freshness.map(Freshness::row_kind),
        requested_poll_us: profile.connection.poll_us,
        iodd_communication: profile.communication().cloned(),
        stale_threshold_us: profile.connection.stale_us,
        requested_duration_ms: options.duration.map(|d| d.as_millis() as u64),
        simulation_faults: options.simulate_faults,
        simulated_writer_delay_ms: options.simulated_writer_delay.as_millis() as u64,
        source_update_loss: match freshness {
            Some(Freshness::Counter32) => {
                "counter gaps within connection generations; loss during disconnects is unknown"
            }
            _ => "unknown",
        },
        source_gaps_observed: (freshness == Some(Freshness::Counter32)).then_some(gaps),
        last_error,
        terminal_error,
        writer_error: summary.error,
        csv_rows_flushed: summary.rows,
        csv_rows_unconfirmed: state
            .counts
            .selected
            .saturating_sub(state.counts.logger_overruns)
            .saturating_sub(summary.rows),
        csv_parts: summary.parts,
        logger_metrics: summary.metrics,
        local_api,
        mqtt,
        shared_memory,
        counts: state.counts,
        host_timer: timer.label(),
        host_poll_start_spacing: poll_start.summarize(),
        host_receive_spacing: host.summarize(),
        host_new_update_spacing: host_new.summarize(),
        gateway_update_spacing: gateway.summarize(),
        time_since_last_receive_us: last_host.map(|t| us(end.duration_since(t))),
    };
    if let Some(path) = &options.output {
        save_report(path, &report)?;
    }
    Ok(report)
}

fn save_report(output: &Path, report: &Report) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("capture.toml"))?;
    file.write_all(toml::to_string_pretty(report)?.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests;
