use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaFile {
    pub name: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub severity: String,
    pub message: String,
    pub line: u64,
    pub position: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub passed: bool,
    pub engine: String,
    pub runtime: String,
    pub xml_sha256: String,
    pub root_schema: String,
    pub schemas: Vec<SchemaFile>,
    pub diagnostics: Vec<Diagnostic>,
    pub diagnostics_truncated: bool,
}

impl Report {
    pub fn validate_receipt(&self) -> Result<()> {
        ensure!(
            self.passed && self.diagnostics.is_empty() && !self.diagnostics_truncated,
            "saved XSD receipt must describe a completed pass"
        );
        fn digest(value: &str) -> bool {
            value.len() == 64
                && value
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        ensure!(digest(&self.xml_sha256), "invalid XSD receipt XML hash");
        ensure!(
            !self.engine.is_empty() && !self.runtime.is_empty(),
            "missing XSD engine description"
        );
        ensure!(
            (1..=64).contains(&self.schemas.len()),
            "invalid XSD schema closure size"
        );
        let mut names = HashSet::new();
        for schema in &self.schemas {
            ensure!(
                digest(&schema.sha256) && names.insert(&schema.name),
                "invalid or duplicate XSD schema receipt"
            );
            ensure!(
                !schema.name.is_empty()
                    && !schema.name.contains(['\\', ':'])
                    && schema
                        .name
                        .split('/')
                        .all(|part| !matches!(part, "" | "." | "..")),
                "invalid relative schema name"
            );
        }
        ensure!(
            self.schemas
                .iter()
                .any(|schema| schema.name == self.root_schema),
            "root schema is absent from receipt"
        );
        Ok(())
    }

    pub fn summary(&self) -> String {
        if self.passed {
            format!(
                "Main XML XSD passed against supplied {} · {} schema files",
                self.root_schema,
                self.schemas.len()
            )
        } else {
            let mut text = "XSD validation failed".to_owned();
            for issue in self.diagnostics.iter().take(6) {
                text.push_str(&format!(
                    "\n{} {}:{} · {}",
                    issue.severity, issue.line, issue.position, issue.message
                ));
            }
            if self.diagnostics_truncated || self.diagnostics.len() > 6 {
                text.push_str("\nAdditional diagnostics are available in check-iodd output");
            }
            text
        }
    }
}

#[cfg(windows)]
pub(super) fn validate(xml: &[u8], schema: &Path) -> Result<Report> {
    validate_with_timeout(xml, schema, std::time::Duration::from_secs(20))
}

#[cfg(windows)]
fn validate_with_timeout(xml: &[u8], schema: &Path, budget: std::time::Duration) -> Result<Report> {
    use anyhow::Context;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::os::windows::io::AsRawHandle;
    use std::{
        fs::File,
        io::{Read, Write},
        os::windows::{ffi::OsStringExt, process::CommandExt},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                SetInformationJobObject,
            },
            SystemInformation::GetSystemDirectoryW,
        },
    };

    struct Job(HANDLE);
    impl Drop for Job {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    let job = Job(unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) });
    ensure!(!job.0.is_null(), "create XSD helper lifecycle job");
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    ensure!(
        unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } != 0,
        "configure XSD helper lifecycle job"
    );

    ensure!(
        xml.len() as u64 <= super::XML_LIMIT,
        "IODD XML exceeds 8 MiB"
    );
    let mut input = tempfile::NamedTempFile::new()?;
    input.write_all(xml)?;
    let input = input.into_temp_path();
    let mut request = tempfile::NamedTempFile::new()?;
    serde_json::to_writer(
        &mut request,
        &serde_json::json!({"xml": input.to_path_buf(), "schema": std::path::absolute(schema)?}),
    )?;
    let request = request.into_temp_path();
    let response = tempfile::NamedTempFile::new()?.into_temp_path();
    let mut system = [0u16; 32768];
    // The installed Windows system directory is used instead of searching PATH.
    let length = unsafe { GetSystemDirectoryW(system.as_mut_ptr(), system.len() as u32) } as usize;
    ensure!(
        length > 0 && length < system.len(),
        "cannot locate Windows system directory"
    );
    let executable = std::path::PathBuf::from(std::ffi::OsString::from_wide(&system[..length]))
        .join("WindowsPowerShell/v1.0/powershell.exe");
    let script: Vec<u8> = include_str!("schema.ps1")
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let encoded = STANDARD.encode(script);
    ensure!(
        encoded.len() < 30000,
        "embedded XSD helper exceeds Windows command limit"
    );
    let mut child = Command::new(executable)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encoded,
        ])
        .env("IOLINK_XSD_REQUEST", request.as_os_str())
        .env("IOLINK_XSD_RESPONSE", response.as_os_str())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .context("start local XSD validator using Windows PowerShell")?;
    let start = Instant::now();
    if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) } == 0 {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("attach local XSD helper to its lifecycle job");
    }
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "local XSD helper exited with {status}");
            break;
        }
        if start.elapsed() >= budget {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!(
                "local XSD validation exceeded {} seconds and was stopped",
                budget.as_secs_f64()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut bytes = Vec::new();
    File::open(&response)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1_048_576, "XSD report exceeds 1 MiB");
    let report: Report =
        serde_json::from_slice(&bytes).context("read local XSD validation report")?;
    if report.passed {
        report.validate_receipt()?;
    }
    Ok(report)
}

#[cfg(not(windows))]
pub(super) fn validate(_xml: &[u8], _schema: &Path) -> Result<Report> {
    anyhow::bail!("XSD validation requires Windows PowerShell and System.Xml")
}

#[cfg(all(test, windows))]
mod tests;
