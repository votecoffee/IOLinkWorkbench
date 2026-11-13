//! Opt-in, lossy latest-state MQTT export on an independent preallocated handoff.
use crate::{
    profile::Profile,
    telemetry::{self, Sample},
    transport::Status,
};
use anyhow::{Result, ensure};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(test)]
mod tests;
mod wire;
static INSTANCE: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Debug)]
pub struct Config {
    pub broker: SocketAddr,
    pub topic: String,
    pub interval_ms: u64,
    pub queue_capacity: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            broker: "127.0.0.1:1883".parse().expect("constant address"),
            topic: "iolink-workbench/input".into(),
            interval_ms: 100,
            queue_capacity: 256,
        }
    }
}
impl Config {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.broker.ip().is_unspecified()
                && !self.broker.ip().is_multicast()
                && !matches!(self.broker.ip(), std::net::IpAddr::V4(ip) if ip.is_broadcast())
                && self.broker.port() != 0,
            "MQTT requires a numeric unicast broker address and nonzero port"
        );
        ensure!(
            (10..=60000).contains(&self.interval_ms),
            "MQTT interval must be 10–60000 ms"
        );
        ensure!(
            (1..=16384).contains(&self.queue_capacity),
            "MQTT queue capacity must be 1–16384 events"
        );
        ensure!(
            !self.topic.is_empty()
                && self.topic.len() <= 512
                && !self.topic.chars().any(|c| c.is_control()
                    || matches!(c, '+' | '#')
                    || (0xFDD0..=0xFDEF).contains(&(c as u32))
                    || (c as u32) & 0xFFFF >= 0xFFFE),
            "MQTT topic must be 1–512 UTF-8 bytes without wildcards, controls or noncharacters"
        );
        Ok(())
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

#[derive(Clone, Default, Debug, Deserialize, Serialize)]
pub struct Summary {
    pub format_version: u32,
    pub broker: String,
    pub topic: String,
    pub session_id: String,
    pub client_id: String,
    pub interval_ms: u64,
    pub queue_capacity: usize,
    pub events_offered: u64,
    pub queue_dropped: u64,
    pub events_dequeued: u64,
    pub coalesced: u64,
    pub shutdown_discarded: u64,
    pub publish_attempts: u64,
    pub socket_writes_completed: u64,
    pub publish_write_failures: u64,
    pub encoding_failures: u64,
    pub broker_connect_attempts: u64,
    pub broker_connections: u64,
    pub connect_failures: u64,
    pub connection_losses: u64,
    pub keepalive_pings: u64,
    pub disconnect_write_completed: bool,
    pub worker_failed: bool,
    pub last_error: Option<String>,
}
impl Summary {
    pub(crate) fn validate(&self) -> Result<()> {
        Config {
            broker: self.broker.parse()?,
            topic: self.topic.clone(),
            interval_ms: self.interval_ms,
            queue_capacity: self.queue_capacity,
        }
        .validate()?;
        ensure!(
            self.format_version == 1
                && !self.session_id.is_empty()
                && self.session_id.len() <= 256
                && (1..=23).contains(&self.client_id.len())
                && self.client_id.bytes().all(|b| b.is_ascii_alphanumeric()),
            "invalid MQTT version/session/client ID"
        );
        let offered = u128::from(self.events_dequeued) + u128::from(self.queue_dropped);
        let consumed = u128::from(self.coalesced)
            + u128::from(self.shutdown_discarded)
            + u128::from(self.publish_attempts);
        ensure!(
            offered <= u128::from(self.events_offered)
                && consumed <= u128::from(self.events_dequeued)
                && (self.worker_failed
                    || (offered == u128::from(self.events_offered)
                        && consumed == u128::from(self.events_dequeued))),
            "MQTT handoff/coalescing accounting is inconsistent"
        );
        ensure!(
            u128::from(self.publish_attempts)
                == u128::from(self.socket_writes_completed)
                    + u128::from(self.publish_write_failures)
                    + u128::from(self.encoding_failures),
            "MQTT publish accounting is inconsistent"
        );
        ensure!(
            u128::from(self.broker_connect_attempts)
                == u128::from(self.broker_connections) + u128::from(self.connect_failures)
                && self.connection_losses <= self.broker_connections
                && self.last_error.as_ref().is_none_or(|s| s.len() <= 256),
            "MQTT connection accounting is inconsistent"
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
        let serial = INSTANCE.fetch_add(1, Ordering::Relaxed);
        let initial = Summary {
            format_version: 1,
            broker: config.broker.to_string(),
            topic: config.topic.clone(),
            session_id: format!(
                "mqtt-{}-{}-{serial}",
                utc.chars()
                    .filter(char::is_ascii_alphanumeric)
                    .collect::<String>(),
                std::process::id()
            ),
            client_id: format!("IOLWB{:08X}{serial:08X}", std::process::id()),
            interval_ms: config.interval_ms,
            queue_capacity: config.queue_capacity,
            ..Default::default()
        };
        let shared = Arc::new(Shared {
            queue: ArrayQueue::new(config.queue_capacity),
            offered: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        });
        let worker_shared = shared.clone();
        let worker_config = config.clone();
        let worker_profile = profile.clone();
        let summary = initial;
        // Preserve startup identity even if the worker panics before returning counters.
        let backup = summary.clone();
        let utc = utc.to_owned();
        let worker = thread::Builder::new()
            .name("mqtt-latest-state".into())
            .spawn(move || {
                run(
                    worker_shared,
                    worker_config,
                    worker_profile,
                    start,
                    utc,
                    summary,
                )
            })?;
        eprintln!(
            "MQTT publishing to {} topic {} (QoS 0, not retained, latest-state)",
            config.broker, config.topic
        );
        Ok(Self {
            shared,
            worker: Some(worker),
            initial: Some(backup),
            start,
        })
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
                summary.error("MQTT worker panicked; internal counts unavailable");
                summary
            }
        })
    }
    pub(crate) fn finish(mut self) -> Summary {
        let mut summary = self.stop().expect("active MQTT worker");
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
        utc: &str,
        start: Instant,
    ) -> Result<Vec<u8>> {
        let master = self
            .master
            .as_ref()
            .filter(|(generation, _, _)| *generation == self.generation);
        let data = json!({"format_version":1,"workbench_version":env!("CARGO_PKG_VERSION"),"session_id":summary.session_id,"capture_utc_start":utc,"host_elapsed_us":micros(start.elapsed()),
            "event_sequence":self.sequence,"event_elapsed_us":self.elapsed_us,"generation":self.generation,"sampling_active":!self.finished,"gateway_connected_last_observed":self.connected,
            "publisher_connection":summary.broker_connections,"publish_index":summary.publish_attempts,"delivery":"QoS 0 latest-state, not retained; broker/subscriber delivery unknown",
            "observed_master":master.map(|(_,status,_)|status),"master_text_truncated":master.map(|(_,_,truncated)|truncated),
            "input":self.sample.as_ref().map(|(sample,_)|telemetry::sample_json(*sample,profile)),"input_elapsed_us":self.sample.as_ref().map(|(_,time)|time),
            "input_host_age_us":self.sample.as_ref().map(|(_,time)|micros(start.elapsed()).saturating_sub(*time)),
            "fields":profile.effective_fields().iter().map(|(field,origin)|json!({"origin":origin,"mapping":field})).collect::<Vec<_>>(),
            "requested_host_poll_us":profile.connection.poll_us,"declared_communication":profile.communication(),
            "mqtt_counts":{"events_offered":shared.offered.load(Ordering::Relaxed),"queue_dropped":shared.dropped.load(Ordering::Relaxed),"events_dequeued":summary.events_dequeued,"coalesced":summary.coalesced,"publish_attempts":summary.publish_attempts,"socket_writes_completed":summary.socket_writes_completed,"publish_write_failures":summary.publish_write_failures,"encoding_failures":summary.encoding_failures,"connection_losses":summary.connection_losses}});
        let bytes = serde_json::to_vec(&data)?;
        ensure!(bytes.len() <= wire::MAX_PAYLOAD, "MQTT JSON exceeds 1 MiB");
        Ok(bytes)
    }
}
fn micros(time: Duration) -> u64 {
    time.as_micros().min(u128::from(u64::MAX)) as u64
}

fn attempt(
    latest: &mut Latest,
    connection: &mut Option<wire::Connection>,
    profile: &Profile,
    summary: &mut Summary,
    shared: &Shared,
    utc: &str,
    start: Instant,
) {
    latest.pending = false;
    summary.publish_attempts += 1;
    match latest.payload(profile, summary, shared, utc, start) {
        Err(error) => {
            summary.encoding_failures += 1;
            summary.error(error);
        }
        Ok(payload) => match connection
            .as_mut()
            .expect("connected publisher")
            .publish(&summary.topic, &payload)
        {
            Ok(()) => summary.socket_writes_completed += 1,
            Err(error) => {
                summary.publish_write_failures += 1;
                summary.connection_losses += 1;
                summary.error(error);
                *connection = None;
            }
        },
    }
}
fn run(
    shared: Arc<Shared>,
    config: Config,
    profile: Profile,
    start: Instant,
    utc: String,
    mut summary: Summary,
) -> Summary {
    let mut latest = Latest::default();
    let mut connection: Option<wire::Connection> = None;
    let mut retry = Instant::now();
    let mut next_send = Instant::now();
    loop {
        for _ in 0..config.queue_capacity {
            if let Some(event) = shared.queue.pop() {
                latest.ingest(event, &mut summary);
            } else {
                break;
            }
        }
        if shared.stop.load(Ordering::Acquire) && shared.queue.is_empty() {
            break;
        }
        if let Some(client) = connection.as_mut() {
            match client.maintain() {
                Ok(true) => summary.keepalive_pings += 1,
                Ok(false) => {}
                Err(error) => {
                    summary.connection_losses += 1;
                    summary.error(error);
                    connection = None;
                    retry = Instant::now() + Duration::from_millis(500);
                }
            }
        }
        if connection.is_none() && latest.pending && Instant::now() >= retry {
            summary.broker_connect_attempts += 1;
            match wire::Connection::connect(config.broker, &summary.client_id) {
                Ok(client) => {
                    connection = Some(client);
                    summary.broker_connections += 1;
                    continue;
                }
                Err(error) => {
                    summary.connect_failures += 1;
                    summary.error(error);
                    retry = Instant::now() + Duration::from_millis(500);
                }
            }
        }
        if connection.is_some() && latest.pending && Instant::now() >= next_send {
            attempt(
                &mut latest,
                &mut connection,
                &profile,
                &mut summary,
                &shared,
                &utc,
                start,
            );
            next_send = Instant::now() + Duration::from_millis(config.interval_ms);
            if connection.is_none() {
                retry = Instant::now() + Duration::from_millis(500);
            }
        }
        thread::sleep(Duration::from_millis(1));
    }
    // Worker shutdown is known even if the queued final control event was dropped.
    latest.finished = true;
    if latest.pending {
        if connection.is_some() {
            attempt(
                &mut latest,
                &mut connection,
                &profile,
                &mut summary,
                &shared,
                &utc,
                start,
            );
        } else {
            latest.pending = false;
            summary.shutdown_discarded += 1;
        }
    }
    if let Some(connection) = connection {
        summary.disconnect_write_completed = connection.disconnect();
    }
    summary
}
