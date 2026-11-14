//! Hidden native initialization and offscreen rendering, without desktop input or capture.
use anyhow::{Context as _, Result, ensure};
use serde::Serialize;
use std::{
    io::Write,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Instant,
};

mod cases;
mod gpu;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static BACKGROUND_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
const CAPABILITY: &str = "IOLINK_WORKBENCH_OFFSCREEN_SELF_TEST_V1";

pub(super) fn block_background() -> bool {
    if ACTIVE.load(Ordering::SeqCst) {
        BACKGROUND_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
        true
    } else {
        false
    }
}

struct Guard;
impl Guard {
    fn enter() -> Result<Self> {
        ensure!(
            ACTIVE
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            "GUI self-test already active"
        );
        BACKGROUND_ATTEMPTS.store(0, Ordering::SeqCst);
        Ok(Self)
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct StopBeforeFrameLoop;
impl std::fmt::Display for StopBeforeFrameLoop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("offscreen self-test complete; stop before native first-frame visibility")
    }
}
impl std::error::Error for StopBeforeFrameLoop {}

#[derive(Serialize)]
struct Receipt {
    capability: &'static str,
    format_version: u8,
    workbench_version: &'static str,
    os: &'static str,
    arch: &'static str,
    started_utc: String,
    elapsed_ms: u64,
    status: &'static str,
    scope: &'static str,
    background_attempts: usize,
    native: Option<gpu::Native>,
    error: Option<String>,
    limitations: &'static str,
}

pub fn run(path: &Path) -> Result<()> {
    ensure!(
        cfg!(windows),
        "native GUI self-test is supported on Windows only"
    );
    ensure!(
        !path.exists(),
        "self-test receipt already exists; choose a new output file"
    );
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure!(
        parent.is_dir() && path.file_name().is_some(),
        "self-test receipt requires an existing parent directory and a new filename"
    );
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let _guard = Guard::enter()?;
    let started = Instant::now();
    let started_utc = chrono::Utc::now().to_rfc3339();
    let shared: Arc<Mutex<Option<Result<gpu::Native>>>> = Arc::new(Mutex::new(None));
    let destination = shared.clone();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1130.0, 790.0])
            .with_visible(false)
            .with_active(false)
            .with_taskbar(false),
        renderer: eframe::Renderer::Glow,
        run_and_return: true,
        ..Default::default()
    };
    // eframe 0.31.1 shows its window after the first ordinary frame. The creator
    // always returns a typed sentinel error, so that visibility path is never entered.
    let native_result = catch_unwind(AssertUnwindSafe(|| {
        eframe::run_native(
            "IO-Link Workbench offscreen self-test",
            options,
            Box::new(move |cc| {
                let result =
                    catch_unwind(AssertUnwindSafe(|| gpu::render(cc))).unwrap_or_else(|panic| {
                        Err(anyhow::anyhow!(
                            "offscreen rendering panicked: {}",
                            panic_text(panic)
                        ))
                    });
                if let Ok(mut slot) = destination.lock() {
                    *slot = Some(result);
                }
                Err(Box::new(StopBeforeFrameLoop))
            }),
        )
    }));
    let outcome = match native_result {
        Ok(Err(eframe::Error::AppCreation(error))) if error.is::<StopBeforeFrameLoop>() => shared
            .lock()
            .map_err(|_| anyhow::anyhow!("self-test result lock poisoned"))?
            .take()
            .context("native initializer did not return a self-test result")?,
        Ok(Err(error)) => Err(anyhow::anyhow!("native GUI initialization failed: {error}")),
        Ok(Ok(())) => Err(anyhow::anyhow!(
            "native initializer returned without the self-test stop marker"
        )),
        Err(panic) => Err(anyhow::anyhow!(
            "native initialization panicked: {}",
            panic_text(panic)
        )),
    };
    let attempts = BACKGROUND_ATTEMPTS.load(Ordering::SeqCst);
    let outcome = outcome.and_then(|native| {
        ensure!(
            attempts == 0,
            "GUI attempted a background operation during the self-test"
        );
        Ok(native)
    });
    let (native, error) = match outcome {
        Ok(native) => (Some(native), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let receipt = Receipt {
        capability: CAPABILITY,
        format_version: 1,
        workbench_version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        started_utc,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        status: if error.is_none() { "pass" } else { "fail" },
        scope: "hidden native OpenGL initialization and offscreen painting of built-in GUI views; no gateway access, desktop input, screenshots or pixel readback",
        background_attempts: attempts,
        native,
        error,
        limitations: "No normal native frame loop, window presentation, visual comparison, mouse/keyboard interaction, DPI changes, or physical IO-Link sessions are tested",
    };
    serde_json::to_writer_pretty(&mut temporary, &receipt)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .context("publishing new GUI self-test receipt")?;
    ensure!(
        receipt.status == "pass",
        "GUI self-test failed: {}",
        receipt.error.as_deref().unwrap_or("unknown error")
    );
    Ok(())
}

fn panic_text(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else if let Some(text) = panic.downcast_ref::<&str>() {
        (*text).to_owned()
    } else {
        "non-string panic".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_guard_rejects_a_job_before_dispatch() -> Result<()> {
        let _serial = super::super::WORKER_TEST_LOCK.lock().unwrap();
        let guard = Guard::enter()?;
        let called = Arc::new(AtomicBool::new(false));
        let flag = called.clone();
        let result = super::super::background("forbidden-self-test-job", move || {
            flag.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(result.is_err());
        assert!(!called.load(Ordering::SeqCst));
        assert_eq!(BACKGROUND_ATTEMPTS.load(Ordering::SeqCst), 1);
        drop(guard);
        assert!(!ACTIVE.load(Ordering::SeqCst));
        Ok(())
    }

    #[test]
    fn invalid_receipt_destinations_fail_before_native_initialization() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let existing = directory.path().join("existing.json");
        std::fs::write(&existing, b"original bytes\n")?;
        assert!(run(&existing).is_err());
        assert_eq!(std::fs::read(&existing)?, b"original bytes\n");
        let missing = directory.path().join("missing").join("report.json");
        assert!(run(&missing).is_err());
        assert!(!missing.parent().unwrap().exists());
        Ok(())
    }
}
