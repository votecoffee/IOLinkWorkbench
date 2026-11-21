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
    pub master_diagnostics: Option<MasterDiagnostics>,
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
pub struct MasterDiagnostics {
    pub temperature_c: Option<f64>,
    pub supply_voltage_mv: Option<f64>,
    pub sensor_current_ma: Option<f64>,
    pub supply_status_raw: Option<String>,
    pub unavailable: Vec<UnavailablePoint>,
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
const TEMPERATURE: &str = "/processdatamaster/temperature";
const VOLTAGE: &str = "/processdatamaster/voltage";
const CURRENT: &str = "/processdatamaster/current";
const SUPPLY_STATUS: &str = "/processdatamaster/supervisionstatus";

/// Inspect one explicitly profiled ifm endpoint. Device identity is optional on every port.
pub fn inspect_master(profile: &Profile) -> Result<MasterInspection> {
    profile.validate()?;
    let started = Instant::now();
    let mut client = Client::new(&profile.connection)?;
    let mut paths = client.master_paths();
    paths.push(HARDWARE_REVISION.into());
    let (_, response) = client.multi(&paths)?;
    let master = MasterIdentity::observed(&client, &response);
    let (master_diagnostics, ports, error) = match read_details(&mut client, &master, paths) {
        Ok((diagnostics, ports)) => (Some(diagnostics), ports, None),
        Err(error) => (None, Vec::new(), Some(format!("{error:#}"))),
    };
    Ok(MasterInspection {
        gateway: profile.connection.gateway,
        route: client.route,
        master,
        master_diagnostics,
        host_finished_utc: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        read_duration_us: started.elapsed().as_micros() as u64,
        ports,
        error,
    })
}

fn read_details(
    client: &mut Client,
    master: &MasterIdentity,
    mut paths: Vec<String>,
) -> Result<(MasterDiagnostics, Vec<PortInspection>)> {
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
    paths.extend([TEMPERATURE, VOLTAGE, CURRENT, SUPPLY_STATUS].map(str::to_owned));
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
    Ok((
        MasterDiagnostics::observed(&response),
        ports
            .map(|port| PortInspection::observed(client, &response, port))
            .collect(),
    ))
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

impl MasterDiagnostics {
    fn observed(response: &Value) -> Self {
        let numeric = |path| {
            let value = Client::data(response, path);
            value
                .as_f64()
                .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
                .filter(|value| value.is_finite())
        };
        let temperature_c = numeric(TEMPERATURE);
        let supply_voltage_mv = numeric(VOLTAGE);
        let sensor_current_ma = numeric(CURRENT);
        let supply_status_raw =
            label(Client::data(response, SUPPLY_STATUS)).filter(|text| !text.trim().is_empty());
        let unavailable = [
            (TEMPERATURE, temperature_c.is_none()),
            (VOLTAGE, supply_voltage_mv.is_none()),
            (CURRENT, sensor_current_ma.is_none()),
            (SUPPLY_STATUS, supply_status_raw.is_none()),
        ]
        .into_iter()
        .filter(|(_, missing)| *missing)
        .map(|(path, _)| UnavailablePoint::observed(response, path.to_owned()))
        .collect();
        Self {
            temperature_c,
            supply_voltage_mv,
            sensor_current_ma,
            supply_status_raw,
            unavailable,
        }
    }
}

impl UnavailablePoint {
    fn observed(response: &Value, path: String) -> Self {
        Self {
            service_code: Client::point(response, &path)["code"].as_u64(),
            path,
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
            .map(|path| UnavailablePoint::observed(response, path))
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

fn quantity(value: Option<f64>, unit: &str) -> String {
    match value {
        Some(value) => format!("{value} {unit}"),
        None => "unavailable".into(),
    }
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
        if let Some(diagnostics) = &self.master_diagnostics {
            let _ = write!(
                text,
                "\nMaster temperature {} · Supply voltage {} · Sensor current {}\nSupply status (raw) {}",
                quantity(diagnostics.temperature_c, "°C"),
                quantity(diagnostics.supply_voltage_mv, "mV"),
                quantity(diagnostics.sensor_current_ma, "mA"),
                available(&diagnostics.supply_status_raw)
            );
            for point in &diagnostics.unavailable {
                if let Some(code) = point.service_code.filter(|code| *code != 200) {
                    let _ = write!(text, "\n{} · IoT Core {code}", point.path);
                }
            }
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
