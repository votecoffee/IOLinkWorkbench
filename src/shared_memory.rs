//! Optional Windows latest snapshot, with a consumer-independent bounded sampler handoff.
use crate::{
    profile::Profile,
    telemetry::{self, Sample},
    transport::Status,
};
use anyhow::{Result, ensure};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

mod layout;
pub use layout::{HEADER_BYTES, Header, MAP_BYTES};
#[cfg(windows)]
mod native;
#[cfg(test)]
mod tests;
static INSTANCE: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Debug)]
pub struct Config {
    pub name: String,
    pub interval_ms: u64,
    pub queue_capacity: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            name: "input".into(),
            interval_ms: 10,
            queue_capacity: 256,
        }
    }
}
impl Config {
    fn validate(&self) -> Result<()> {
        ensure!(
            (1..=64).contains(&self.name.len())
                && self
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')),
            "shared-memory name must be 1–64 ASCII letters, digits, underscore, dash or dot"
        );
        ensure!(
            (1..=1000).contains(&self.interval_ms) && (1..=16384).contains(&self.queue_capacity),
            "shared-memory interval must be 1–1000 ms and queue 1–16384 events"
        );
        Ok(())
    }
    pub fn mapping_name(&self) -> String {
        format!("Local\\IOLinkWorkbench.Snapshot.{}", self.name)
    }
    pub fn mutex_name(&self) -> String {
        format!("{}.Mutex", self.mapping_name())
    }
}

#[derive(Debug, Serialize)]
pub struct Readout {
    pub format_version: u32,
    pub mapping_name: String,
    pub header: Header,
    pub publish_age_us: u64,
    pub publish_age_exceeds_limit: bool,
    pub publisher_process_alive: Option<bool>,
    pub payload: Value,
}
pub fn read(name: &str, wait_ms: u32, max_age_ms: u64) -> Result<Readout> {
    let config = Config {
        name: name.into(),
        ..Default::default()
    };
    config.validate()?;
    ensure!(
        wait_ms <= 1000 && (1..=60000).contains(&max_age_ms),
        "reader wait must be 0–1000 ms and age limit 1–60000 ms"
    );
    #[cfg(windows)]
    {
        native::read(&config, wait_ms, max_age_ms)
    }
    #[cfg(not(windows))]
    {
        anyhow::bail!("shared memory requires Windows")
    }
}

enum Kind {
    Connected(Status, bool),
    Input(Sample),
    Disconnected,
    Finished,
}
struct Event {
    sequence: u64,
    elapsed_us: u64,
    generation: u64,
    kind: Kind,
}
struct Shared {
    queue: ArrayQueue<Event>,
    offered: AtomicU64,
    dropped: AtomicU64,
    stop: AtomicBool,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Summary {
    pub format_version: u32,
    pub name: String,
    pub session_id: String,
    pub interval_ms: u64,
    pub queue_capacity: usize,
    pub events_offered: u64,
    pub events_dequeued: u64,
    pub queue_dropped: u64,
    pub coalesced: u64,
    pub publish_attempts: u64,
    pub final_refresh_attempts: u64,
    pub snapshots_written: u64,
    pub snapshots_replaced: u64,
    pub busy_skips: u64,
    pub encoding_failures: u64,
    pub write_failures: u64,
    pub abandoned_recoveries: u64,
    pub final_snapshot_written: bool,
    pub worker_failed: bool,
    pub last_error: Option<String>,
}
impl Summary {
    pub(crate) fn validate(&self) -> Result<()> {
        Config {
            name: self.name.clone(),
            interval_ms: self.interval_ms,
            queue_capacity: self.queue_capacity,
        }
        .validate()?;
        ensure!(
            self.format_version == 1 && !self.session_id.is_empty() && self.session_id.len() <= 256,
            "invalid shared-memory version/session"
        );
        let offered = u128::from(self.events_dequeued) + u128::from(self.queue_dropped);
        let consumed = u128::from(self.coalesced) + u128::from(self.publish_attempts);
        ensure!(
            offered <= u128::from(self.events_offered)
                && consumed <= u128::from(self.events_dequeued)
                && (self.worker_failed
                    || (offered == u128::from(self.events_offered)
                        && consumed == u128::from(self.events_dequeued))),
            "shared-memory handoff accounting is inconsistent"
        );
        ensure!(
            u128::from(self.publish_attempts) + u128::from(self.final_refresh_attempts)
                == u128::from(self.snapshots_written)
                    + u128::from(self.busy_skips)
                    + u128::from(self.encoding_failures)
                    + u128::from(self.write_failures)
                && self.final_refresh_attempts <= 1
                && self.snapshots_replaced == self.snapshots_written.saturating_sub(1)
                && self.abandoned_recoveries <= self.snapshots_written
                && (!self.final_snapshot_written || self.snapshots_written > 0),
            "shared-memory commit accounting is inconsistent"
        );
        ensure!(
            self.last_error.as_ref().is_none_or(|s| s.len() <= 256),
            "shared-memory error text exceeds bounds"
        );
        Ok(())
    }
    fn error(&mut self, error: impl std::fmt::Display) {
        let mut message = error.to_string();
        telemetry::truncate(&mut message);
        self.last_error = Some(message);
    }
}

pub(crate) struct Service {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<Summary>>,
    initial: Option<Summary>,
    start: Instant,
}
impl Service {
    pub(crate) fn start(
        config: &Config,
        profile: &Profile,
        start: Instant,
        utc: &str,
    ) -> Result<Self> {
        config.validate()?;
        telemetry::validate_mapping(profile)?;
        #[cfg(not(windows))]
        {
            let _ = (start, utc);
            anyhow::bail!("shared memory requires Windows");
        }
        #[cfg(windows)]
        {
            let mapping = native::Mapping::create(config)?;
            let summary = Summary {
                format_version: 1,
                name: config.name.clone(),
                session_id: format!(
                    "shm-{}-{}-{}",
                    utc.chars()
                        .filter(char::is_ascii_alphanumeric)
                        .collect::<String>(),
                    std::process::id(),
                    INSTANCE.fetch_add(1, Ordering::Relaxed)
                ),
                interval_ms: config.interval_ms,
                queue_capacity: config.queue_capacity,
                ..Default::default()
            };
            let backup = summary.clone();
            let shared = Arc::new(Shared {
                queue: ArrayQueue::new(config.queue_capacity),
                offered: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
                stop: AtomicBool::new(false),
            });
            let worker_shared = shared.clone();
            let worker_profile = profile.clone();
            let utc = utc.to_owned();
            let worker = thread::Builder::new()
                .name("shared-memory-latest".into())
                .spawn(move || run(worker_shared, mapping, worker_profile, start, utc, summary))?;
            eprintln!(
                "Shared memory {} (latest snapshot, mutex {})",
                config.mapping_name(),
                config.mutex_name()
            );
            Ok(Self {
                shared,
                worker: Some(worker),
                initial: Some(backup),
                start,
            })
        }
    }
    fn offer(&self, generation: u64, elapsed_us: u64, kind: Kind) {
        let sequence = self.shared.offered.fetch_add(1, Ordering::Relaxed) + 1;
        if self
            .shared
            .queue
            .push(Event {
                sequence,
                elapsed_us,
                generation,
                kind,
            })
            .is_err()
        {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(crate) fn connected(&self, generation: u64, status: &Status) {
        let (status, truncated) = telemetry::bounded_status(status);
        self.offer(
            generation,
            micros(self.start.elapsed()),
            Kind::Connected(status, truncated),
        );
    }
    pub(crate) fn sample(&self, generation: u64, sample: Sample) {
        self.offer(
            generation,
            micros(sample.input.received.saturating_duration_since(self.start)),
            Kind::Input(sample),
        );
    }
    pub(crate) fn disconnected(&self, generation: u64) {
        self.offer(generation, micros(self.start.elapsed()), Kind::Disconnected);
    }
    pub(crate) fn ended(&self, generation: u64) {
        self.offer(generation, micros(self.start.elapsed()), Kind::Finished);
    }
    fn stop(&mut self) -> Option<Summary> {
        self.shared.stop.store(true, Ordering::Release);
        self.worker.take().map(|worker| match worker.join() {
            Ok(summary) => summary,
            Err(_) => {
                let mut summary = self.initial.take().expect("startup identity");
                summary.worker_failed = true;
                summary.error("shared-memory worker panicked; internal counts unavailable");
                summary
            }
        })
    }
    pub(crate) fn finish(mut self) -> Summary {
        let mut summary = self.stop().expect("active shared-memory worker");
        summary.events_offered = self.shared.offered.load(Ordering::Relaxed);
        summary.queue_dropped = self.shared.dropped.load(Ordering::Relaxed);
        summary
    }
}
impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Default)]
struct Latest {
    pending: bool,
    sequence: u64,
    elapsed_us: u64,
    generation: u64,
    connected: bool,
    finished: bool,
    sample: Option<(Sample, u64)>,
    master: Option<(u64, Status, bool)>,
}
impl Latest {
    fn ingest(&mut self, event: Event, summary: &mut Summary) {
        summary.events_dequeued += 1;
        if self.pending {
            summary.coalesced += 1;
        }
        self.pending = true;
        self.sequence = event.sequence;
        self.elapsed_us = event.elapsed_us;
        self.generation = event.generation;
        match event.kind {
            Kind::Connected(status, truncated) => {
                self.connected = true;
                self.sample = None;
                self.master = Some((event.generation, status, truncated));
            }
            Kind::Input(sample) => {
                self.connected = true;
                self.sample = Some((sample, event.elapsed_us));
            }
            Kind::Disconnected => {
                self.connected = false;
                self.sample = None;
            }
            Kind::Finished => self.finished = true,
        }
    }
    fn payload(
        &self,
        profile: &Profile,
        summary: &Summary,
        shared: &Shared,
        start: Instant,
        utc: &str,
    ) -> Result<Vec<u8>> {
        let master = self
            .master
            .as_ref()
            .filter(|(generation, _, _)| *generation == self.generation);
        let value = json!({"format_version":1,"workbench_version":env!("CARGO_PKG_VERSION"),"session_id":summary.session_id,"capture_utc_start":utc,"host_elapsed_us":micros(start.elapsed()),"snapshot_sequence":summary.snapshots_written+1,
            "event_sequence":self.sequence,"event_elapsed_us":self.elapsed_us,"generation":self.generation,"sampling_active":!self.finished,"gateway_connected_last_observed":self.connected,
            "delivery":"latest snapshot; intermediate observations and reader delivery unknown",
            "observed_master":master.map(|(_,status,_)|status),"master_text_truncated":master.map(|(_,_,truncated)|truncated),
            "input":self.sample.as_ref().map(|(sample,_)|telemetry::sample_json(*sample,profile)),"input_elapsed_us":self.sample.as_ref().map(|(_,time)|time),"input_host_age_us_at_encoding":self.sample.as_ref().map(|(_,time)|micros(start.elapsed()).saturating_sub(*time)),
            "fields":profile.effective_fields().iter().map(|(field,origin)|json!({"origin":origin,"mapping":field})).collect::<Vec<_>>(),"requested_host_poll_us":profile.connection.poll_us,"declared_communication":profile.communication(),
            "shared_memory_counts":{"events_offered":shared.offered.load(Ordering::Relaxed),"events_dequeued":summary.events_dequeued,"queue_dropped":shared.dropped.load(Ordering::Relaxed),"coalesced":summary.coalesced,"publish_attempts":summary.publish_attempts,"snapshots_written_before_this":summary.snapshots_written,"busy_skips":summary.busy_skips}});
        let bytes = serde_json::to_vec(&value)?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAP_BYTES - HEADER_BYTES,
            "shared-memory JSON exceeds bounds"
        );
        Ok(bytes)
    }
}
fn micros(time: Duration) -> u64 {
    time.as_micros().min(u128::from(u64::MAX)) as u64
}

#[cfg(windows)]
fn attempt(
    latest: &mut Latest,
    mapping: &mut native::Mapping,
    profile: &Profile,
    summary: &mut Summary,
    shared: &Shared,
    clock: (Instant, &str),
    refresh: bool,
) {
    let (start, utc) = clock;
    if refresh {
        summary.final_refresh_attempts += 1;
    } else {
        latest.pending = false;
        summary.publish_attempts += 1;
    }
    match latest.payload(profile, summary, shared, start, utc) {
        Err(error) => {
            summary.encoding_failures += 1;
            summary.error(error);
        }
        Ok(bytes) => match mapping.write(
            &bytes,
            summary.snapshots_written + 1,
            u32::from(!latest.finished) | (u32::from(latest.connected) << 1),
        ) {
            Ok(None) => summary.busy_skips += 1,
            Ok(Some(abandoned)) => {
                summary.snapshots_written += 1;
                summary.snapshots_replaced = summary.snapshots_written.saturating_sub(1);
                summary.abandoned_recoveries += u64::from(abandoned);
                if latest.finished {
                    summary.final_snapshot_written = true;
                }
            }
            Err(error) => {
                summary.write_failures += 1;
                summary.error(error);
            }
        },
    }
}
#[cfg(windows)]
fn run(
    shared: Arc<Shared>,
    mut mapping: native::Mapping,
    profile: Profile,
    start: Instant,
    utc: String,
    mut summary: Summary,
) -> Summary {
    let mut latest = Latest::default();
    let mut next = Instant::now();
    loop {
        for _ in 0..summary.queue_capacity {
            if let Some(event) = shared.queue.pop() {
                latest.ingest(event, &mut summary);
            } else {
                break;
            }
        }
        if shared.stop.load(Ordering::Acquire) && shared.queue.is_empty() {
            break;
        }
        if latest.pending && Instant::now() >= next {
            attempt(
                &mut latest,
                &mut mapping,
                &profile,
                &mut summary,
                &shared,
                (start, &utc),
                false,
            );
            next = Instant::now() + Duration::from_millis(summary.interval_ms);
        }
        thread::sleep(Duration::from_millis(1));
    }
    latest.finished = true;
    if latest.pending {
        attempt(
            &mut latest,
            &mut mapping,
            &profile,
            &mut summary,
            &shared,
            (start, &utc),
            false,
        );
    } else if !summary.final_snapshot_written {
        attempt(
            &mut latest,
            &mut mapping,
            &profile,
            &mut summary,
            &shared,
            (start, &utc),
            true,
        );
    }
    summary
}
