//! Explicit, independent read-only status observations for ports on one configured master.
use super::{Status, check_route, read_status};
use crate::profile::{Gateway, Profile};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{thread, time::Instant};

#[derive(Debug, Serialize)]
pub struct PortStatusReport {
    pub gateway: Gateway,
    /// Comparable identifiers from successful status responses disagreed.
    pub observed_master_conflict: bool,
    pub ports: Vec<PortStatus>,
}

#[derive(Debug, Serialize)]
pub struct PortStatus {
    pub profile_name: String,
    pub port: u8,
    pub host_finished_utc: String,
    pub read_duration_us: u64,
    pub identity_check: Option<&'static str>,
    pub status: Option<Status>,
    pub error: Option<String>,
}

impl PortStatusReport {
    pub fn all_successful(&self) -> bool {
        !self.observed_master_conflict && self.ports.iter().all(|port| port.error.is_none())
    }
}

/// Validate the entire selection before starting independent clients.
/// Each result has its own host timestamps. Reports retain the profile order.
pub fn read_port_statuses(profiles: &[Profile]) -> Result<PortStatusReport> {
    let gateway = validate_selection(profiles)?;
    let ports: Vec<_> = run_port_workers(profiles, "status", read_port)
        .into_iter()
        .zip(profiles)
        .map(|(result, profile)| {
            result.unwrap_or_else(|error| failed_worker(profile, format!("{error:#}")))
        })
        .collect();
    Ok(PortStatusReport {
        gateway,
        observed_master_conflict: master_conflict(
            ports.iter().filter_map(|port| port.status.as_ref()),
        ),
        ports,
    })
}

/// Launch every selected port before joining results in selection order.
pub(crate) fn run_port_workers<T: Send>(
    profiles: &[Profile],
    operation_name: &str,
    operation: impl Fn(&Profile) -> T + Sync,
) -> Vec<Result<T>> {
    thread::scope(|scope| {
        let workers: Vec<_> = profiles
            .iter()
            .map(|profile| {
                thread::Builder::new()
                    .name(format!("{operation_name}-port-{}", profile.connection.port))
                    .spawn_scoped(scope, || operation(profile))
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .context("start port worker")?
                    .join()
                    .map_err(|_| anyhow::anyhow!("port worker stopped unexpectedly"))
            })
            .collect()
    })
}

pub(crate) fn master_conflict<'a>(mut statuses: impl Iterator<Item = &'a Status>) -> bool {
    statuses.next().is_some_and(|first| {
        let mut serial = first.master_serial.as_deref();
        statuses.any(|other| {
            let conflict = other
                .master_serial
                .as_deref()
                .zip(serial)
                .is_some_and(|(a, b)| a != b);
            serial = serial.or(other.master_serial.as_deref());
            conflict || first.model != other.model || first.firmware != other.firmware
        })
    })
}

/// Check saved routes for a port status query without opening a client.
pub fn validate_selection(profiles: &[Profile]) -> Result<Gateway> {
    ensure!(
        (1..=4).contains(&profiles.len()),
        "select one to four port profiles"
    );
    let gateway = profiles[0].connection.gateway;
    let mut seen = [false; 4];
    let mut origin = None;
    let mut firmware = None;
    for profile in profiles {
        let connection = &profile.connection;
        profile
            .validate()
            .with_context(|| format!("validate port {} profile", connection.port))?;
        ensure!(
            connection.gateway == gateway,
            "port profiles must use the same gateway route"
        );
        let port = &mut seen[usize::from(connection.port - 1)];
        ensure!(!*port, "port {} is listed more than once", connection.port);
        *port = true;
        if gateway == Gateway::IfmAl1350 {
            let target = connection.origin()?;
            ensure!(
                origin.as_ref().is_none_or(|origin| origin == &target),
                "AL1350 port profiles must target the same configured origin"
            );
            origin = Some(target);
            if !connection.ifm_firmware.is_empty() {
                let pin = connection.ifm_firmware.as_str();
                ensure!(
                    firmware.is_none_or(|firmware| firmware == pin),
                    "AL1350 port profiles have conflicting firmware pins"
                );
                firmware = Some(pin);
            }
        }
        check_route(profile).with_context(|| format!("validate port {} route", connection.port))?;
    }
    Ok(gateway)
}

fn read_port(profile: &Profile) -> PortStatus {
    let start = Instant::now();
    let observation = read_status(profile);
    let finished = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true);
    let duration = start.elapsed().as_micros() as u64;
    let (status, identity_check, error) = match observation {
        Ok(status) => match profile.expected_identity.verify(&status.identity) {
            Ok(check) => (Some(status), Some(check), None),
            Err(error) => (Some(status), None, Some(format!("{error:#}"))),
        },
        Err(error) => (None, None, Some(format!("{error:#}"))),
    };
    PortStatus {
        profile_name: profile.name.clone(),
        port: profile.connection.port,
        host_finished_utc: finished,
        read_duration_us: duration,
        identity_check,
        status,
        error,
    }
}

fn failed_worker(profile: &Profile, error: String) -> PortStatus {
    PortStatus {
        profile_name: profile.name.clone(),
        port: profile.connection.port,
        host_finished_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        read_duration_us: 0,
        identity_check: None,
        status: None,
        error: Some(error),
    }
}
