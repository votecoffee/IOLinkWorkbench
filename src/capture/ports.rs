//! Concurrent independent cyclic captures for explicitly selected ports.
use super::{Options, Report, Stop, check_layout, run};
use crate::{
    profile::{Gateway, Profile},
    transport::ports::{master_conflict, run_port_workers, validate_selection},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{fs, io::Write, path::PathBuf, time::Duration};

pub struct PortOptions {
    pub output: PathBuf,
    pub duration: Duration,
    pub simulate_faults: bool,
    pub simulated_writer_delay: Duration,
}

#[derive(Debug, Serialize)]
pub struct PortBatch {
    pub format_version: u32,
    pub gateway: Gateway,
    pub stopped_by_request: bool,
    /// Comparable master identifiers disagreed among the recorded status observations.
    pub observed_master_conflict: bool,
    pub ports: Vec<PortResult>,
}

#[derive(Debug, Serialize)]
pub struct PortResult {
    pub port: u8,
    pub profile_name: String,
    /// Relative to the batch folder. A setup error can leave this folder absent or partial.
    pub directory: String,
    pub report_saved: bool,
    pub success: bool,
    pub received: Option<u64>,
    pub csv_rows_flushed: Option<u64>,
    pub error: Option<String>,
}

impl PortBatch {
    pub fn all_successful(&self) -> bool {
        !self.observed_master_conflict && self.ports.iter().all(|port| port.success)
    }
}

pub fn run_ports(profiles: &[Profile], options: &PortOptions, stop: &Stop) -> Result<PortBatch> {
    let gateway = validate_selection(profiles)?;
    for profile in profiles {
        check_layout(profile)
            .with_context(|| format!("check port {} capture layout", profile.connection.port))?;
    }
    ensure!(
        !options.duration.is_zero() && options.duration <= Duration::from_secs(86_400),
        "port capture duration must be positive and at most 24 hours"
    );
    ensure!(
        gateway == Gateway::Simulator
            || (!options.simulate_faults && options.simulated_writer_delay.is_zero()),
        "fault injection and simulated writer delay require simulator profiles"
    );

    fs::create_dir(&options.output).with_context(|| {
        format!(
            "create new port capture directory {}",
            options.output.display()
        )
    })?;
    let results = run_port_workers(profiles, "capture", |profile| {
        run(
            profile,
            &Options {
                output: Some(options.output.join(port_directory(profile))),
                duration: Some(options.duration),
                simulate_faults: options.simulate_faults,
                simulated_writer_delay: options.simulated_writer_delay,
                ..Options::default()
            },
            stop,
            None,
        )
    });
    let results: Vec<_> = results
        .into_iter()
        .map(|result| result.and_then(|report| report))
        .collect();
    let observed_master_conflict = master_conflict(
        results
            .iter()
            .filter_map(|result| result.as_ref().ok())
            .flat_map(|report| report.last_status.iter().chain(&report.rejected_status)),
    );
    let ports = profiles
        .iter()
        .zip(results)
        .map(|(profile, result)| port_result(profile, result))
        .collect();
    let batch = PortBatch {
        format_version: 1,
        gateway,
        stopped_by_request: stop.requested(),
        observed_master_conflict,
        ports,
    };
    let mut summary = tempfile::NamedTempFile::new_in(&options.output)?;
    summary.write_all(toml::to_string_pretty(&batch)?.as_bytes())?;
    summary.as_file().sync_all()?;
    summary
        .persist_noclobber(options.output.join("capture-set.toml"))
        .context("publish port capture summary")?;
    Ok(batch)
}

fn port_directory(profile: &Profile) -> String {
    format!("port-{}", profile.connection.port)
}

fn port_result(profile: &Profile, result: Result<Report>) -> PortResult {
    let (report, error) = match result {
        Ok(report) => (Some(report), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    PortResult {
        port: profile.connection.port,
        profile_name: profile.name.clone(),
        directory: port_directory(profile),
        report_saved: report.is_some(),
        success: report.as_ref().is_some_and(Report::success),
        received: report.as_ref().map(|report| report.counts.received),
        csv_rows_flushed: report.as_ref().map(|report| report.csv_rows_flushed),
        error,
    }
}
