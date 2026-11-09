use super::{Row, fs};
use crate::{decode, profile::Profile, transport::Validity};
use anyhow::{Context, Result};
use crossbeam_queue::ArrayQueue;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Summary {
    pub rows: u64,
    pub parts: u64,
    pub error: Option<String>,
}

struct Shared {
    queue: ArrayQueue<Row>,
    stop: AtomicBool,
    failed: AtomicBool,
}

pub struct Writer {
    shared: Arc<Shared>,
    thread: JoinHandle<Summary>,
}

impl Writer {
    pub fn start(path: &Path, profile: &Profile, utc: String, delay: Duration) -> Result<Self> {
        let shared = Arc::new(Shared {
            queue: ArrayQueue::new(profile.logging.queue_capacity),
            stop: AtomicBool::new(false),
            failed: AtomicBool::new(false),
        });
        fs::create_dir(path)
            .with_context(|| format!("create new capture directory {}", path.display()))?;
        profile.save(&path.join("profile.toml"))?;
        let profile = profile.clone();
        let path = path.to_owned();
        let output = CsvOutput::new(path, profile, utc)?;
        let worker = shared.clone();
        let thread = thread::Builder::new()
            .name("csv-writer".into())
            .spawn(move || {
                let mut output = output;
                let result = output.consume(&worker, delay);
                if result.is_err() {
                    worker.failed.store(true, Ordering::Release);
                }
                Summary {
                    rows: output.flushed,
                    parts: output.part,
                    error: result.err().map(|error| format!("{error:#}")),
                }
            })?;
        Ok(Self { shared, thread })
    }

    pub fn enqueue(&self, row: Row) -> bool {
        if self.shared.queue.push(row).is_ok() {
            self.thread.thread().unpark();
            true
        } else {
            false
        }
    }

    pub fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Acquire)
    }

    pub fn finish(self) -> Summary {
        self.shared.stop.store(true, Ordering::Release);
        self.thread.thread().unpark();
        self.thread.join().unwrap_or_else(|_| Summary {
            error: Some("CSV writer panicked".into()),
            ..Summary::default()
        })
    }
}

const HEADER: &[&str] = &[
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

struct CsvOutput {
    path: PathBuf,
    profile: Profile,
    utc: String,
    csv: csv::Writer<File>,
    part: u64,
    part_rows: u64,
    part_start_us: u64,
    written: u64,
    flushed: u64,
}

impl CsvOutput {
    fn open_part(path: &Path, profile: &Profile, part: u64) -> Result<csv::Writer<File>> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join(format!("capture-{part:04}.csv")))?;
        let mut csv = csv::WriterBuilder::new()
            .buffer_capacity(64 * 1024)
            .from_writer(file);
        let mut header = HEADER
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
        csv.write_record(header)?;
        Ok(csv)
    }

    fn new(path: PathBuf, profile: Profile, utc: String) -> Result<Self> {
        let csv = Self::open_part(&path, &profile, 1)?;
        Ok(Self {
            path,
            profile,
            utc,
            csv,
            part: 1,
            part_rows: 0,
            part_start_us: 0,
            written: 0,
            flushed: 0,
        })
    }

    fn flush(&mut self) -> Result<()> {
        self.csv.flush()?;
        self.flushed = self.written;
        Ok(())
    }

    fn consume(&mut self, shared: &Shared, delay: Duration) -> Result<()> {
        let mut flush = Instant::now();
        loop {
            if let Some(row) = shared.queue.pop() {
                if !delay.is_zero() {
                    thread::sleep(delay);
                }
                self.write(row)?;
            } else if shared.stop.load(Ordering::Acquire) && shared.queue.is_empty() {
                break;
            } else {
                thread::park_timeout(Duration::from_millis(25));
            }
            if flush.elapsed() >= Duration::from_millis(250) {
                self.flush()?;
                flush = Instant::now();
            }
        }
        self.flush()?;
        self.csv.get_ref().sync_all()?;
        Ok(())
    }

    fn write(&mut self, row: Row) -> Result<()> {
        let settings = &self.profile.logging;
        let rotate_rows = settings.rotate_rows != 0 && self.part_rows >= settings.rotate_rows;
        let rotate_time = settings.rotate_seconds != 0
            && row.elapsed_us.saturating_sub(self.part_start_us)
                >= settings.rotate_seconds.saturating_mul(1_000_000);
        if self.part_rows != 0 && (rotate_rows || rotate_time) {
            self.flush()?;
            self.csv.get_ref().sync_all()?;
            self.csv = Self::open_part(&self.path, &self.profile, self.part + 1)?;
            self.part += 1;
            self.part_rows = 0;
        }
        if self.part_rows == 0 {
            self.part_start_us = row.elapsed_us;
        }
        let input = row.input;
        let number = |n: Option<u64>| n.map(|n| n.to_string()).unwrap_or_default();
        let mut cells = vec![
            self.utc.clone(),
            row.elapsed_us.to_string(),
            row.sample_index.to_string(),
            row.generation.to_string(),
            row.freshness.row_kind().into(),
            input.validity.label().into(),
            row.stale.map(|s| s.to_string()).unwrap_or_default(),
            number(input.counter.map(u64::from)),
            number(input.source_time_us),
            number(input.source_age_us),
            number(input.diagnostic_code.map(u64::from)),
            decode::hex(input.payload.bytes()),
        ];
        for field in self.profile.log_fields() {
            cells.push(
                if input.validity == Validity::Valid && row.stale != Some(true) {
                    field.decode(input.payload.bytes())?.value
                } else {
                    String::new()
                },
            );
        }
        self.csv.write_record(cells)?;
        self.part_rows += 1;
        self.written += 1;
        Ok(())
    }
}
