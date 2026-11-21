//! Compare validated master inspection snapshots without opening a transport.
use super::{MasterInspectionSnapshot, PortInspection, UnavailablePoint};
use anyhow::Result;
use serde::Serialize;
use std::{fmt::Write as _, path::Path};

#[derive(Debug, Serialize)]
pub struct MasterInspectionComparison {
    pub left: MasterInspectionSnapshot,
    pub right: MasterInspectionSnapshot,
    pub context_changes: Vec<InspectionChange>,
    pub comparison_issue: Option<&'static str>,
    pub observations: Option<ObservationChanges>,
}

#[derive(Debug, Serialize)]
pub struct ObservationChanges {
    pub cycles_comparable: bool,
    pub supply_changes: Vec<InspectionChange>,
    pub changed_ports: Vec<ChangedPort>,
}

#[derive(Debug, Serialize)]
pub struct ChangedPort {
    pub port: u8,
    pub link: Vec<InspectionChange>,
    pub connected_device: Vec<InspectionChange>,
    pub configuration: Vec<InspectionChange>,
}

#[derive(Debug, Serialize)]
pub struct InspectionChange {
    pub field: &'static str,
    pub left: Option<String>,
    pub right: Option<String>,
}

macro_rules! changes {
    ($left:ident, $right:ident; $($field:ident => $label:literal),+ $(,)?) => {{
        let mut found = Vec::new();
        $(record(&mut found, $label, $left.$field.as_ref(), $right.$field.as_ref());)+
        found
    }};
}

impl MasterInspectionComparison {
    pub fn load(left: &Path, right: &Path) -> Result<Self> {
        let left = MasterInspectionSnapshot::load(left)?;
        let right = MasterInspectionSnapshot::load(right)?;
        let context_changes = context_changes(&left, &right);
        let comparison_issue = comparison_issue(&left, &right);
        let observations = if comparison_issue.is_none() {
            compare_observations(&left, &right)
        } else {
            None
        };
        Ok(Self {
            left,
            right,
            context_changes,
            comparison_issue,
            observations,
        })
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "Saved master comparison · left → right\n{} → {}",
            self.left.inspection.host_finished_utc, self.right.inspection.host_finished_utc
        );
        write_changes(&mut text, "Recorded context", &self.context_changes);
        if let Some(issue) = self.comparison_issue {
            let _ = write!(text, "\n\n{issue}");
        }
        if let Some(observations) = &self.observations {
            if !observations.cycles_comparable {
                text.push_str("\nConverted cycle values require matching confirmed units");
            }
            write_changes(&mut text, "Master supply", &observations.supply_changes);
            for port in &observations.changed_ports {
                let _ = write!(text, "\n\nPort {}", port.port);
                write_changes(&mut text, "Link and diagnostics", &port.link);
                write_changes(&mut text, "Connected device", &port.connected_device);
                write_changes(&mut text, "Configured settings", &port.configuration);
            }
            if observations.supply_changes.is_empty() && observations.changed_ports.is_empty() {
                text.push_str("\n\nNo differences among comparable recorded observations");
            }
        }
        text
    }
}

fn record<T: PartialEq + ToString>(
    changes: &mut Vec<InspectionChange>,
    field: &'static str,
    left: Option<T>,
    right: Option<T>,
) {
    if left != right {
        changes.push(InspectionChange {
            field,
            left: left.map(|v| v.to_string()),
            right: right.map(|v| v.to_string()),
        });
    }
}

fn write_changes(out: &mut String, label: &str, fields: &[InspectionChange]) {
    if !fields.is_empty() {
        let _ = write!(out, "\n{label}");
        for change in fields {
            let _ = write!(
                out,
                "\n  {} · {} → {}",
                change.field,
                change.left.as_deref().unwrap_or("unavailable"),
                change.right.as_deref().unwrap_or("unavailable"),
            );
        }
    }
}

fn context_changes(
    left: &MasterInspectionSnapshot,
    right: &MasterInspectionSnapshot,
) -> Vec<InspectionChange> {
    let mut found = Vec::new();
    for (label, a, b) in [
        ("Endpoint", left.endpoint.as_str(), right.endpoint.as_str()),
        (
            "Profile name",
            left.profile_name.as_str(),
            right.profile_name.as_str(),
        ),
        (
            "Gateway",
            left.inspection.gateway.label(),
            right.inspection.gateway.label(),
        ),
        (
            "Firmware pin",
            left.firmware_pin.as_str(),
            right.firmware_pin.as_str(),
        ),
    ] {
        record(&mut found, label, Some(a), Some(b));
    }
    let cycle_unit = |unit| match unit {
        Some(crate::profile::CycleUnit::Microseconds) => Some("microseconds"),
        Some(crate::profile::CycleUnit::Milliseconds) => Some("milliseconds"),
        None => None,
    };
    record(
        &mut found,
        "Cycle unit",
        cycle_unit(left.cycle_unit),
        cycle_unit(right.cycle_unit),
    );
    let a = &left.inspection.master;
    let b = &right.inspection.master;
    found.extend(changes!(a, b;
        model => "Observed model",
        firmware => "Observed firmware",
        hardware_revision => "Hardware revision",
        serial => "Master serial",
    ));
    found
}

fn comparison_issue(
    left: &MasterInspectionSnapshot,
    right: &MasterInspectionSnapshot,
) -> Option<&'static str> {
    let a = &left.inspection;
    let b = &right.inspection;
    if a.error.is_some() || b.error.is_some() {
        Some("One or both saved inspections are incomplete")
    } else if a.gateway != b.gateway || left.endpoint != right.endpoint {
        Some("The recorded master routes differ")
    } else if [&a.master, &b.master].iter().any(|master| {
        master
            .serial
            .as_deref()
            .is_none_or(|serial| serial.trim().is_empty())
    }) {
        Some("One or both saved master serial numbers are unavailable")
    } else if a.master.serial != b.master.serial {
        Some("The observed master serial numbers differ")
    } else if a.master.firmware != b.master.firmware {
        Some("The observed master firmware revisions differ")
    } else if let (Some(a), Some(b)) = (&a.master.hardware_revision, &b.master.hardware_revision)
        && a != b
    {
        Some("The observed master hardware revisions differ")
    } else {
        None
    }
}

fn compare_observations(
    left: &MasterInspectionSnapshot,
    right: &MasterInspectionSnapshot,
) -> Option<ObservationChanges> {
    let cycles_comparable = left.cycle_unit.is_some() && left.cycle_unit == right.cycle_unit;
    let a = left.inspection.master_diagnostics.as_ref()?;
    let b = right.inspection.master_diagnostics.as_ref()?;
    let mut supply_changes = changes!(a, b;
        temperature_c => "Master temperature (°C)",
        supply_voltage_mv => "Supply voltage (mV)",
        sensor_current_ma => "Sensor current (mA)",
        supply_status_raw => "Supply status (raw)",
    );
    compare_availability(&mut supply_changes, &a.unavailable, &b.unavailable);
    Some(ObservationChanges {
        cycles_comparable,
        supply_changes,
        changed_ports: left
            .inspection
            .ports
            .iter()
            .zip(&right.inspection.ports)
            .filter_map(|(a, b)| compare_port(a, b, cycles_comparable))
            .collect(),
    })
}

fn compare_port(
    a: &PortInspection,
    b: &PortInspection,
    cycles_comparable: bool,
) -> Option<ChangedPort> {
    let mut link = changes!(a, b;
        mode => "Port mode",
        comspeed_raw => "COM speed (raw)",
        port_cycle_raw => "Reported cycle (raw)",
        device_status => "Device status",
        port_event => "Port event",
        device_event => "Device event",
    );
    if cycles_comparable {
        record(
            &mut link,
            "Reported cycle (µs)",
            a.port_cycle_us,
            b.port_cycle_us,
        );
    }
    compare_availability(&mut link, &a.unavailable, &b.unavailable);
    let port = a.port;
    let left = &a.identity;
    let right = &b.identity;
    let connected_device = changes!(left, right;
        vendor_id => "Connected Vendor ID",
        device_id => "Connected Device ID",
        product_id => "Connected Product ID",
        product_name => "Connected product name",
        serial => "Connected device serial",
    );
    let left = &a.configuration;
    let right = &b.configuration;
    let mut configuration = changes!(left, right;
        preset_cycle_raw => "Preset cycle (raw)",
        preset_cycle_automatic => "Automatic cycle preset",
        validation_mode_raw => "Validation / Data Storage (raw)",
        validation_vendor_id => "Configured Vendor ID",
        validation_device_id => "Configured Device ID",
    );
    if cycles_comparable {
        record(
            &mut configuration,
            "Preset cycle (µs)",
            left.preset_cycle_us,
            right.preset_cycle_us,
        );
    }
    compare_availability(&mut configuration, &left.unavailable, &right.unavailable);
    if link.is_empty() && connected_device.is_empty() && configuration.is_empty() {
        None
    } else {
        Some(ChangedPort {
            port,
            link,
            connected_device,
            configuration,
        })
    }
}

fn compare_availability(
    changes: &mut Vec<InspectionChange>,
    left: &[UnavailablePoint],
    right: &[UnavailablePoint],
) {
    fn summary(points: &[UnavailablePoint]) -> String {
        let mut points: Vec<_> = points.iter().map(|p| (&p.path, p.service_code)).collect();
        points.sort_unstable();
        points
            .iter()
            .map(|(path, code)| match code {
                Some(code) => format!("{path} (IoT Core {code})"),
                None => path.to_string(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
    if !(left.is_empty() && right.is_empty()) {
        record(
            changes,
            "Unavailable data points",
            Some(if left.is_empty() {
                String::from("none reported")
            } else {
                summary(left)
            }),
            Some(if right.is_empty() {
                String::from("none reported")
            } else {
                summary(right)
            }),
        );
    }
}
