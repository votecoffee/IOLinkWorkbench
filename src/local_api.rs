//! Opt-in loopback telemetry. The sampler only offers events to a preallocated queue.
use crate::{profile::Profile, transport::Status};
use anyhow::{Context, Result, ensure};
use crossbeam_queue::ArrayQueue;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

mod http;
#[cfg(test)]
mod tests;

const MAX_REPLY: usize = 1_048_576;
const MAX_LIMIT: usize = 128;
static SESSION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: SocketAddr,
    pub queue_capacity: usize,
    pub history_capacity: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8765).into(),
            queue_capacity: 1024,
            history_capacity: 4096,
        }
    }
}

impl Config {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.listen.ip() == std::net::IpAddr::V4(Ipv4Addr::LOCALHOST),
            "local API must bind to numeric IPv4 127.0.0.1"
        );
        ensure!(
            (1..=16384).contains(&self.queue_capacity)
                && (1..=16384).contains(&self.history_capacity),
            "local API queue/history capacity must be 1–16384 events"
        );
        Ok(())
    }
}

use crate::telemetry::Sample;
#[cfg(test)]
use crate::telemetry::truncate;

enum Body {
    Connected {
        status: Status,
        text_truncated: bool,
    },
    Sample(Sample),
    Disconnected,
    Finished,
}

struct Event {
    sequence: u64,
    elapsed_us: u64,
    generation: u64,
    body: Body,
}

struct Shared {
    queue: ArrayQueue<Event>,
    offered: AtomicU64,
    dropped: AtomicU64,
    stop: AtomicBool,
    requests: AtomicU64,
    failed_requests: AtomicU64,
}

struct Store {
    history: VecDeque<Event>,
    capacity: usize,
    ingested: u64,
    evicted: u64,
    master: Option<(u64, Status, bool)>,
}

impl Store {
    fn ingest(&mut self, event: Event) {
        if let Body::Connected {
            status,
            text_truncated,
        } = &event.body
        {
            self.master = Some((event.generation, status.clone(), *text_truncated));
        }
        self.ingested += 1;
        if self.history.len() == self.capacity {
            self.history.pop_front();
            self.evicted += 1;
        }
        self.history.push_back(event);
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Summary {
    pub format_version: u32,
    pub url: String,
    pub session_id: String,
    pub queue_capacity: usize,
    pub history_capacity: usize,
    pub events_offered: u64,
    pub events_ingested: u64,
    pub queue_dropped: u64,
    pub history_evicted: u64,
    pub requests: u64,
    pub failed_requests: u64,
    pub worker_failed: bool,
}

impl Summary {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.format_version == 1 && !self.session_id.is_empty() && self.session_id.len() <= 256,
            "invalid local API version/session"
        );
        let address = self
            .url
            .strip_prefix("http://")
            .context("invalid local API URL")?
            .parse::<SocketAddr>()?;
        let config = Config {
            listen: address,
            queue_capacity: self.queue_capacity,
            history_capacity: self.history_capacity,
        };
        config.validate()?;
        let accounted = u128::from(self.events_ingested) + u128::from(self.queue_dropped);
        ensure!(
            accounted <= u128::from(self.events_offered)
                && (self.worker_failed || accounted == u128::from(self.events_offered)),
            "local API queue accounting is inconsistent"
        );
        ensure!(
            self.history_evicted
                == self
                    .events_ingested
                    .saturating_sub(self.history_capacity as u64)
                && self.failed_requests <= self.requests,
            "local API history/request accounting is inconsistent"
        );
        Ok(())
    }
}

pub(crate) struct Service {
    shared: Arc<Shared>,
    store: Arc<Mutex<Store>>,
    ingest: Option<JoinHandle<()>>,
    http: Option<JoinHandle<Result<()>>>,
    config: Config,
    url: String,
    session_id: String,
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
        profile.validate()?;
        let fields = profile.effective_fields();
        ensure!(
            fields.len() <= 128
                && fields
                    .iter()
                    .all(|(f, _)| f.name.len() <= 256 && f.unit.len() <= 256),
            "local API supports at most 128 mapped fields with names/units up to 256 UTF-8 bytes"
        );
        let schema = json!({"format_version":1,"application":"IOLinkWorkbench","workbench_version":env!("CARGO_PKG_VERSION"),
            "input_bytes":profile.input_bytes,"fields":fields.iter().map(|(field,origin)| json!({"origin":origin,"mapping":field})).collect::<Vec<_>>(),
            "configured_gateway":profile.connection.gateway,"requested_host_poll_us":profile.connection.poll_us,
            "declared_communication":profile.communication(),"stale_threshold_us":profile.connection.stale_us,"logging":profile.logging,
            "event_basis":"normalized input responses, including duplicates and late responses; independent of CSV selection",
            "clock":"host_elapsed_us is monotonic within this session; source_time_us is transport-specific within its generation",
            "source_update_loss":"unknown for snapshots; counter gaps only within observed generations; disconnect loss unknown"});
        ensure!(
            serde_json::to_vec(&schema)?.len() < MAX_REPLY,
            "local API schema exceeds response limit"
        );
        let listener = TcpListener::bind(config.listen).context("bind read-only local API")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let shared = Arc::new(Shared {
            queue: ArrayQueue::new(config.queue_capacity),
            offered: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            requests: AtomicU64::new(0),
            failed_requests: AtomicU64::new(0),
        });
        let store = Arc::new(Mutex::new(Store {
            history: VecDeque::with_capacity(config.history_capacity),
            capacity: config.history_capacity,
            ingested: 0,
            evicted: 0,
            master: None,
        }));
        let session_id = format!(
            "{}-{}-{}",
            utc.chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>(),
            std::process::id(),
            SESSION.fetch_add(1, Ordering::Relaxed)
        );
        let server = http::Server {
            shared: shared.clone(),
            store: store.clone(),
            schema,
            profile: profile.clone(),
            session_id: session_id.clone(),
            utc_start: utc.into(),
            start,
            authority: address.to_string(),
            queue_capacity: config.queue_capacity,
            history_capacity: config.history_capacity,
        };
        let http = thread::Builder::new()
            .name("local-api-http".into())
            .spawn(move || server.run(listener))?;
        let ingest_shared = shared.clone();
        let ingest_store = store.clone();
        let ingest = match thread::Builder::new()
            .name("local-api-ingest".into())
            .spawn(move || {
                loop {
                    if let Some(event) = ingest_shared.queue.pop() {
                        ingest_store
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .ingest(event);
                    } else if ingest_shared.stop.load(Ordering::Acquire) {
                        break;
                    } else {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            }) {
            Ok(worker) => worker,
            Err(error) => {
                shared.stop.store(true, Ordering::Release);
                let _ = http.join();
                return Err(error.into());
            }
        };
        let url = format!("http://{address}");
        eprintln!("Local API listening at {url} (read-only, session {session_id})");
        Ok(Self {
            shared,
            store,
            ingest: Some(ingest),
            http: Some(http),
            config: config.clone(),
            url,
            session_id,
            start,
        })
    }

    fn offer(&self, generation: u64, elapsed_us: u64, body: Body) {
        let sequence = self.shared.offered.fetch_add(1, Ordering::Relaxed) + 1;
        if self
            .shared
            .queue
            .push(Event {
                sequence,
                elapsed_us,
                generation,
                body,
            })
            .is_err()
        {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn connected(&self, generation: u64, status: &Status) {
        let (status, text_truncated) = crate::telemetry::bounded_status(status);
        self.offer(
            generation,
            self.elapsed(),
            Body::Connected {
                status,
                text_truncated,
            },
        );
    }
    pub(crate) fn sample(&self, generation: u64, sample: Sample) {
        self.offer(
            generation,
            micros(sample.input.received.saturating_duration_since(self.start)),
            Body::Sample(sample),
        );
    }
    pub(crate) fn disconnected(&self, generation: u64) {
        self.offer(generation, self.elapsed(), Body::Disconnected);
    }
    pub(crate) fn ended(&self, generation: u64) {
        self.offer(generation, self.elapsed(), Body::Finished);
    }
    fn elapsed(&self) -> u64 {
        micros(self.start.elapsed())
    }
    fn stop(&mut self) -> bool {
        self.shared.stop.store(true, Ordering::Release);
        let ingest_failed = self.ingest.take().is_some_and(|t| t.join().is_err());
        let http_failed = self
            .http
            .take()
            .is_some_and(|t| !matches!(t.join(), Ok(Ok(()))));
        ingest_failed || http_failed
    }
    pub(crate) fn finish(mut self) -> Summary {
        let failed = self.stop();
        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        Summary {
            format_version: 1,
            url: self.url.clone(),
            session_id: self.session_id.clone(),
            queue_capacity: self.config.queue_capacity,
            history_capacity: self.config.history_capacity,
            events_offered: self.shared.offered.load(Ordering::Relaxed),
            events_ingested: store.ingested,
            queue_dropped: self.shared.dropped.load(Ordering::Relaxed),
            history_evicted: store.evicted,
            requests: self.shared.requests.load(Ordering::Relaxed),
            failed_requests: self.shared.failed_requests.load(Ordering::Relaxed),
            worker_failed: failed,
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

fn micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn event_json(event: &Event, profile: &Profile) -> Value {
    let body = match &event.body {
        Body::Connected {
            status,
            text_truncated,
        } => {
            json!({"kind":"connected","status":status,"text_limit_utf8_bytes":256,"text_truncated":text_truncated})
        }
        Body::Disconnected => json!({"kind":"disconnected"}),
        Body::Finished => json!({"kind":"capture_finished"}),
        Body::Sample(sample) => crate::telemetry::sample_json(*sample, profile),
    };
    json!({"sequence":event.sequence,"host_elapsed_us":event.elapsed_us,"generation":event.generation,"data":body})
}
