//! Read the master and its port metadata independently of cyclic or ISDU traffic.
use super::{Client, MASTER_SERIAL, PRODUCT_CODE, label};
use crate::profile::{Gateway, Identity, Profile};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::{fmt::Write as _, time::Instant};

#[derive(Debug, Serialize)]
pub struct MasterInspection {
    pub gateway: Gateway,
    pub route: &'static str,
    pub master: MasterIdentity,
    pub host_finished_utc: String,
    pub read_duration_us: u64,
    pub ports: Vec<PortInspection>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MasterIdentity {
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub hardware_revision: Option<String>,
    pub serial: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PortInspection {
    pub port: u8,
    pub mode: Option<String>,
    pub comspeed_raw: Option<String>,
    pub port_cycle_raw: Option<String>,
    pub port_cycle_us: Option<u64>,
    pub device_status: Option<String>,
    pub port_event: Option<String>,
    pub device_event: Option<String>,
    pub identity: Identity,
    pub unavailable: Vec<UnavailablePoint>,
}

#[derive(Debug, Serialize)]
pub struct UnavailablePoint {
    pub path: String,
    pub service_code: Option<u64>,
}

const HARDWARE_REVISION: &str = "/deviceinfo/hwrevision";

/// Inspect one explicitly profiled ifm endpoint. Device identity is optional on every port.
pub fn inspect_master(profile: &Profile) -> Result<MasterInspection> {
    profile.validate()?;
    let started = Instant::now();
    let mut client = Client::new(&profile.connection)?;
    let mut paths = client.master_paths();
    paths.push(HARDWARE_REVISION.into());
    let (_, response) = client.multi(&paths)?;
    let master = MasterIdentity::observed(&client, &response);
    let ports = read_ports(&mut client, &master, paths);
    let (ports, error) = match ports {
        Ok(ports) => (ports, None),
        Err(error) => (Vec::new(), Some(format!("{error:#}"))),
    };
    Ok(MasterInspection {
        gateway: profile.connection.gateway,
        route: client.route,
        master,
        host_finished_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        read_duration_us: started.elapsed().as_micros() as u64,
        ports,
        error,
    })
}

fn read_ports(
    client: &mut Client,
    master: &MasterIdentity,
    mut paths: Vec<String>,
) -> Result<Vec<PortInspection>> {
    ensure!(
        master.model.as_deref() == Some(client.model),
        "expected master {}, observed {}",
        client.model,
        available(&master.model)
    );
    let firmware = master
        .firmware
        .as_deref()
        .context("master firmware is unavailable")?;
    let settings = &client.settings;
    ensure!(
        settings.ifm_firmware.is_empty() || settings.ifm_firmware == firmware,
        "expected firmware {}, observed {firmware}",
        settings.ifm_firmware
    );
    let ports = 1..=settings.gateway.max_port();
    paths.extend(ports.clone().flat_map(port_paths));
    let (_, response) = client.multi(&paths)?;
    let observed = MasterIdentity::observed(client, &response);
    ensure!(
        master.model == observed.model
            && master.firmware == observed.firmware
            && master
                .serial
                .as_ref()
                .is_none_or(|serial| observed.serial.as_ref() == Some(serial)),
        "master identity changed or became unavailable during port inspection"
    );
    Ok(ports
        .map(|port| PortInspection::observed(client, &response, port))
        .collect())
}

fn port_paths(port: u8) -> Vec<String> {
    let path = format!("/iolinkmaster/port[{port}]");
    let device = format!("{path}/iolinkdevice");
    let mut points = Client::device_paths(&device);
    points.extend([
        format!("{path}/mode"),
        format!("{path}/comspeed"),
        format!("{path}/mastercycletime_actual"),
        format!("{path}/portevent"),
        format!("{device}/status"),
        format!("{device}/iolinkevent"),
    ]);
    points
}

impl MasterIdentity {
    fn observed(client: &Client, response: &Value) -> Self {
        let get = |path| label(Client::data(response, path)).filter(|text| !text.trim().is_empty());
        Self {
            model: get(PRODUCT_CODE),
            firmware: client
                .firmware(response)
                .filter(|text| !text.trim().is_empty()),
            hardware_revision: get(HARDWARE_REVISION),
            serial: get(MASTER_SERIAL),
        }
    }
}

impl PortInspection {
    fn observed(client: &Client, response: &Value, port: u8) -> Self {
        let path = format!("/iolinkmaster/port[{port}]");
        let device = format!("{path}/iolinkdevice");
        let mode = Client::data(response, &format!("{path}/mode"));
        let cycle = Client::data(response, &format!("{path}/mastercycletime_actual"));
        let unavailable = port_paths(port)
            .into_iter()
            .filter(|path| Client::data(response, path).is_null())
            .map(|path| UnavailablePoint {
                service_code: Client::point(response, &path)["code"].as_u64(),
                path,
            })
            .collect();
        Self {
            port,
            mode: label(mode),
            comspeed_raw: label(Client::data(response, &format!("{path}/comspeed"))),
            port_cycle_raw: label(cycle),
            port_cycle_us: client.cycle_us(cycle),
            device_status: label(Client::data(response, &format!("{device}/status"))),
            port_event: label(Client::data(response, &format!("{path}/portevent"))),
            device_event: label(Client::data(response, &format!("{device}/iolinkevent"))),
            identity: Client::identity_at(response, &device),
            unavailable,
        }
    }
}

fn available(field: &Option<String>) -> &str {
    field.as_deref().unwrap_or("unavailable")
}

impl MasterInspection {
    pub fn text(&self) -> String {
        let mut text = format!(
            "{} · firmware {} · hardware {} · S/N {}\n{} · {}",
            available(&self.master.model),
            available(&self.master.firmware),
            available(&self.master.hardware_revision),
            available(&self.master.serial),
            self.route,
            self.host_finished_utc,
        );
        if let Some(error) = &self.error {
            let _ = write!(text, "\n{error}");
        }
        for port in &self.ports {
            let _ = write!(
                text,
                "\n\nPort {} · mode {} · COM speed {}",
                port.port,
                available(&port.mode),
                available(&port.comspeed_raw)
            );
            match port.port_cycle_us {
                Some(us) => {
                    let _ = write!(text, " · cycle {us} µs");
                }
                None => {
                    let _ = write!(text, " · cycle raw {}", available(&port.port_cycle_raw));
                }
            }
            if let (Some(vendor), Some(device)) = (port.identity.vendor_id, port.identity.device_id)
                && vendor != 0
                && device != 0
            {
                let _ = write!(
                    text,
                    "\nVendor {vendor} · Device {device} · {} · S/N {}",
                    available(&port.identity.product_name),
                    available(&port.identity.serial)
                );
            } else {
                text.push_str("\nDevice identity unavailable");
            }
            let _ = write!(
                text,
                "\nDevice status {} · port event {} · device event {}",
                available(&port.device_status),
                available(&port.port_event),
                available(&port.device_event)
            );
            if !port.unavailable.is_empty() {
                let codes: std::collections::BTreeSet<_> = port
                    .unavailable
                    .iter()
                    .filter_map(|point| point.service_code)
                    .filter(|code| *code != 200)
                    .collect();
                let _ = write!(text, "\n{} unavailable data points", port.unavailable.len());
                for code in codes {
                    let _ = write!(text, " · IoT Core {code}");
                }
            }
        }
        text
    }
}
