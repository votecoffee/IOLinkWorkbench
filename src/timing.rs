//! Pure comparisons of profile declarations and separately reported port timing.
use crate::{
    iodd::Bitrate,
    profile::{Gateway, Profile},
    transport::{Freshness, Status},
};
use anyhow::{Result, ensure};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Shorter,
    Equal,
    Longer,
    Unknown,
}

fn relation(interval: Option<u64>, reference: Option<u64>) -> Relation {
    match (
        interval.filter(|value| *value > 0),
        reference.filter(|value| *value > 0),
    ) {
        (Some(a), Some(b)) => match a.cmp(&b) {
            std::cmp::Ordering::Less => Relation::Shorter,
            std::cmp::Ordering::Equal => Relation::Equal,
            std::cmp::Ordering::Greater => Relation::Longer,
        },
        _ => Relation::Unknown,
    }
}

#[derive(Default)]
pub struct Observation<'a> {
    pub port_cycle_us: Option<u64>,
    pub freshness: Option<Freshness>,
    pub route: Option<&'a str>,
}

impl<'a> From<&'a Status> for Observation<'a> {
    fn from(status: &'a Status) -> Self {
        Self {
            port_cycle_us: status.port_cycle_us,
            freshness: Some(status.freshness),
            route: Some(status.route),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Context {
    pub target_us: u64,
    pub requested_host_poll_us: Option<u64>,
    pub iodd_minimum_cycle_us: Option<u32>,
    pub iodd_bitrate: Option<Bitrate>,
    pub reported_port_cycle_us: Option<u64>,
    pub configured_gateway: Gateway,
    pub reported_route: Option<String>,
    pub freshness: Option<Freshness>,
    pub target_vs_iodd_minimum: Relation,
    pub host_poll_vs_iodd_minimum: Relation,
    pub target_vs_reported_port: Relation,
    pub host_poll_vs_reported_port: Relation,
    pub reported_port_vs_iodd_minimum: Relation,
    pub notes: Vec<String>,
}

impl Context {
    pub fn text(&self) -> String {
        let value = |number: Option<u64>| {
            number
                .map(|n| format!("{n} µs"))
                .unwrap_or_else(|| "unavailable".into())
        };
        let mut text = format!(
            "Cycle comparison · target {} µs · configured {:?} · route {}\nIODD minimum {} · requested host poll {} · reported port cycle {}\nTarget vs IODD {:?} · host poll vs IODD {:?} · reported port vs IODD {:?}\nTarget vs port {:?} · host poll vs port {:?}",
            self.target_us,
            self.configured_gateway,
            self.reported_route.as_deref().unwrap_or("not observed"),
            value(self.iodd_minimum_cycle_us.map(u64::from)),
            value(self.requested_host_poll_us),
            value(self.reported_port_cycle_us),
            self.target_vs_iodd_minimum,
            self.host_poll_vs_iodd_minimum,
            self.reported_port_vs_iodd_minimum,
            self.target_vs_reported_port,
            self.host_poll_vs_reported_port
        );
        for note in &self.notes {
            text.push_str(&format!("\n{note}"));
        }
        text
    }
}

/// No transport is opened. Zero means unavailable, never an infinitely fast cycle.
pub fn assess(
    profile: &Profile,
    host_poll_us: Option<u64>,
    observed: Observation<'_>,
    target_us: u64,
) -> Result<Context> {
    ensure!(target_us > 0, "timing target must be positive");
    let communication = profile.communication();
    if let Some(declaration) = communication {
        declaration.validate()?;
    }
    let minimum = communication.map(|value| value.minimum_cycle_us);
    let min = minimum.map(u64::from);
    let port = observed.port_cycle_us;
    let mut result = Context {
        target_us,
        requested_host_poll_us: host_poll_us,
        iodd_minimum_cycle_us: minimum,
        iodd_bitrate: communication.map(|value| value.bitrate),
        reported_port_cycle_us: port,
        configured_gateway: profile.connection.gateway,
        reported_route: observed.route.map(str::to_owned),
        freshness: observed.freshness,
        target_vs_iodd_minimum: relation(Some(target_us), min),
        host_poll_vs_iodd_minimum: relation(host_poll_us, min),
        target_vs_reported_port: relation(Some(target_us), port),
        host_poll_vs_reported_port: relation(host_poll_us, port),
        reported_port_vs_iodd_minimum: relation(port, min),
        notes: Vec::new(),
    };
    if result.target_vs_iodd_minimum == Relation::Shorter {
        result
            .notes
            .push("Target interval is shorter than the IODD-declared minimum cycle".into());
    }
    if result.host_poll_vs_iodd_minimum == Relation::Shorter
        || result.host_poll_vs_reported_port == Relation::Shorter
    {
        result.notes.push("Faster host polls can repeat a device update; polling rate does not establish sensor update rate".into());
    }
    if result.reported_port_vs_iodd_minimum == Relation::Shorter {
        result.notes.push("Reported port cycle is shorter than the IODD minimum; verify identity, cycle units and active configuration".into());
    }
    if minimum == Some(0) {
        result.notes.push("IODD minimum cycle is zero (not provided); the master must determine M-sequence timing".into());
    } else if minimum.is_none() {
        result
            .notes
            .push("IODD wired minimum cycle declaration is unavailable".into());
    }
    if port.is_none_or(|value| value == 0) {
        result.notes.push(
            "A positive IO-Link port cycle has not been reported in known microsecond units".into(),
        );
    }
    match observed.freshness {
        Some(Freshness::Snapshots) => result.notes.push("Gateway snapshots do not verify new source updates; source-update loss remains unknown".into()),
        Some(_) => result.notes.push("Freshness metadata permits observed update tracking; delivery still requires measured qualification".into()),
        None => result.notes.push("Gateway update delivery has not been observed".into()),
    }
    if profile.connection.gateway == Gateway::Simulator {
        result.notes.push("Simulator timing is synthetic and does not qualify physical sensor or gateway performance".into());
    }
    result.notes.push("COM line rate and minimum/port cycles do not establish sensor conversion, gateway update or host receive spacing; compare those measurements separately".into());
    Ok(result)
}

#[cfg(test)]
mod tests;
