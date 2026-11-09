use super::{Freshness, Input, InputTransport, Payload, Status, Validity};
use crate::{
    decode,
    profile::{Connection, CycleUnit, Identity},
};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{Url, blocking::Client as Http};
use serde_json::{Value, json};
use std::{
    io::Read,
    net::IpAddr,
    time::{Duration, Instant},
};

const RESPONSE_LIMIT: u64 = 64 * 1024;

pub struct Client {
    http: Http,
    url: Url,
    settings: Connection,
    auth: Option<Value>,
    cid: i64,
    port: String,
    device: String,
    mode: Option<String>,
    pinned_identity: Option<Identity>,
    route: &'static str,
}

impl Client {
    pub fn new(settings: &Connection) -> Result<Self> {
        let url =
            Url::parse(&settings.endpoint).context("enter an explicit http(s) AL1350 origin")?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none(),
            "AL1350 endpoint must be an http(s) origin without path or credentials"
        );
        let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
        let password = std::env::var("IOLINK_IFM_PASSWORD")
            .ok()
            .filter(|s| !s.is_empty());
        ensure!(
            password.is_none() || url.scheme() == "https",
            "AL1350 credentials require HTTPS"
        );
        let auth = password.map(|pass| {
            json!({
                "user": STANDARD.encode("administrator"), "passwd": STANDARD.encode(pass)
            })
        });
        let http = Http::builder()
            .timeout(Duration::from_millis(settings.timeout_ms))
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()?;
        let port = format!("/iolinkmaster/port[{}]", settings.port);
        Ok(Self {
            http,
            url,
            settings: settings.clone(),
            auth,
            cid: 0,
            device: format!("{port}/iolinkdevice"),
            port,
            mode: None,
            pinned_identity: None,
            route: if loopback {
                "AL1350 loopback fixture"
            } else {
                "AL1350 network endpoint"
            },
        })
    }

    // Private helper. The only callers send /getdatamulti or the explicit ISDU read.
    fn request(&mut self, address: &str, data: Value) -> Result<(Instant, Value)> {
        self.cid = self.cid % i64::from(i32::MAX) + 1;
        let mut request = json!({"code":"request", "cid":self.cid, "adr":address, "data":data});
        if let Some(auth) = &self.auth {
            request["auth"] = auth.clone();
        }
        let response = self
            .http
            .post(self.url.clone())
            .json(&request)
            .send()
            .context("AL1350 HTTP request failed")?;
        ensure!(
            response.status().is_success(),
            "AL1350 HTTP {}",
            response.status()
        );
        let mut body = Vec::new();
        response.take(RESPONSE_LIMIT + 1).read_to_end(&mut body)?;
        let received = Instant::now();
        ensure!(
            body.len() as u64 <= RESPONSE_LIMIT,
            "AL1350 response exceeds 64 KiB"
        );
        let response: Value = serde_json::from_slice(&body)?;
        ensure!(
            response["cid"].as_i64() == Some(self.cid),
            "AL1350 response correlation ID mismatch"
        );
        ensure!(
            response["code"].as_u64() == Some(200),
            "AL1350 service code {}",
            response["code"]
        );
        Ok((received, response))
    }

    fn multi(&mut self, paths: &[String]) -> Result<(Instant, Value)> {
        self.request("/getdatamulti", json!({"datatosend": paths}))
    }

    fn point<'a>(response: &'a Value, path: &str) -> &'a Value {
        response["data"]
            .get(path)
            .or_else(|| response["data"].get(path.trim_start_matches('/')))
            .unwrap_or(&Value::Null)
    }

    fn data<'a>(response: &'a Value, path: &str) -> &'a Value {
        let point = Self::point(response, path);
        if point["code"].as_u64() == Some(200) {
            &point["data"]
        } else {
            &Value::Null
        }
    }

    fn identity(&self, response: &Value) -> Identity {
        let get = |key| Self::data(response, &format!("{}/{key}", self.device));
        Identity {
            vendor_id: number(get("vendorid")).and_then(|n| u16::try_from(n).ok()),
            device_id: number(get("deviceid"))
                .and_then(|n| u32::try_from(n).ok())
                .filter(|id| *id <= 0xFF_FFFF),
            product_id: None,
            product_name: get("productname").as_str().map(str::to_owned),
            serial: get("serial")
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        }
    }

    fn device_paths(&self) -> Vec<String> {
        ["vendorid", "deviceid", "productname", "serial"]
            .map(|key| format!("{}/{key}", self.device))
            .to_vec()
    }

    pub fn read_isdu(&mut self, index: u16, subindex: u8) -> Result<Vec<u8>> {
        let adr = format!("{}/iolreadacyclic", self.device);
        let (_, response) = self.request(&adr, json!({"index":index, "subindex":subindex}))?;
        let value = response["data"]["value"]
            .as_str()
            .context("AL1350 ISDU response has no hex value")?;
        ensure!(
            value.len() <= 464 && value.len().is_multiple_of(2) && value.is_ascii(),
            "AL1350 ISDU response has an invalid length"
        );
        (0..value.len())
            .step_by(2)
            .map(|i| Ok(u8::from_str_radix(&value[i..i + 2], 16)?))
            .collect()
    }
}

fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        let s = value.as_str()?;
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            u64::from_str_radix(hex, 16).ok()
        } else {
            s.parse().ok()
        }
    })
}

fn label(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

impl InputTransport for Client {
    fn connect(&mut self) -> Result<Status> {
        let port = self.port.clone();
        let device = self.device.clone();
        let mode_path = format!("{port}/mode");
        let mut paths = self.device_paths();
        paths.extend(
            [
                "/deviceinfo/productcode",
                "/deviceinfo/swrevision",
                "/deviceinfo/serialnumber",
            ]
            .map(str::to_owned),
        );
        paths.extend([
            mode_path.clone(),
            format!("{port}/mastercycletime_actual"),
            format!("{port}/portevent"),
            format!("{device}/status"),
            format!("{device}/iolinkevent"),
        ]);
        let (_, response) = self.multi(&paths)?;
        let get = |path: &str| Self::data(&response, path);
        let model =
            label(get("/deviceinfo/productcode")).context("AL1350 product code unavailable")?;
        ensure!(
            model == "AL1350",
            "expected master AL1350, observed {model}"
        );
        let firmware =
            label(get("/deviceinfo/swrevision")).context("AL1350 firmware unavailable")?;
        if !self.settings.ifm_firmware.is_empty() {
            ensure!(
                firmware == self.settings.ifm_firmware,
                "AL1350 firmware pin mismatch, observed {firmware}"
            );
        }
        let mode = get(&mode_path);
        let port_mode = label(mode).context("AL1350 port mode unavailable")?;
        let text_matches = mode
            .as_str()
            .is_some_and(|mode| mode.eq_ignore_ascii_case("io-link"));
        let numeric_matches = self
            .settings
            .ifm_mode
            .is_some_and(|code| number(mode) == Some(u64::from(code)));
        ensure!(
            text_matches || numeric_matches,
            "port {} reports mode {port_mode}; verify the existing IO-Link mode and set ifm_mode if numeric",
            self.settings.port
        );
        let identity = self.identity(&response);
        ensure!(
            identity.vendor_id.is_some_and(|id| id != 0)
                && identity.device_id.is_some_and(|id| id != 0),
            "connected IO-Link Vendor ID and Device ID are unavailable"
        );
        self.mode = Some(port_mode.clone());
        self.pinned_identity = Some(identity.clone());
        let cycle = get(&format!("{port}/mastercycletime_actual"));
        let port_cycle_us = number(cycle).filter(|us| *us > 0).and_then(|value| {
            match self.settings.ifm_cycle_unit? {
                CycleUnit::Microseconds => Some(value),
                CycleUnit::Milliseconds => value.checked_mul(1000),
            }
        });
        Ok(Status {
            route: self.route,
            model,
            firmware,
            master_serial: label(get("/deviceinfo/serialnumber")),
            port: self.settings.port,
            port_mode,
            device_status: label(get(&format!("{device}/status"))),
            port_event: label(get(&format!("{port}/portevent"))),
            device_event: label(get(&format!("{device}/iolinkevent"))),
            port_cycle_raw: label(cycle),
            port_cycle_us,
            identity,
            freshness: Freshness::Snapshots,
        })
    }

    fn poll(&mut self) -> Result<Input> {
        let mode_path = format!("{}/mode", self.port);
        let pdin = format!("{}/pdin", self.device);
        let mut paths = self.device_paths();
        paths.extend([mode_path.clone(), pdin.clone()]);
        let (received, response) = self.multi(&paths)?;
        ensure!(
            label(Self::data(&response, &mode_path)) == self.mode,
            "AL1350 port mode changed or is unavailable"
        );
        let identity = self.identity(&response);
        let pinned = self
            .pinned_identity
            .as_ref()
            .context("connect before polling")?;
        let point = Self::point(&response, &pdin);
        let code = point["code"]
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .context("AL1350 response missing process-data status code")?;
        // Unavailable means no usable connection. Preserve an invalid row for documented PD errors.
        ensure!(code != 503, "AL1350 process data unavailable (503)");
        if code == 200 {
            ensure!(
                identity == *pinned,
                "AL1350 port identity changed or is unavailable"
            );
        }
        let (validity, payload) = match code {
            200 => (
                Validity::Valid,
                decode::parse_hex(
                    point["data"]
                        .as_str()
                        .context("AL1350 process data is not a hex string")?,
                )?,
            ),
            530 | 531 => (Validity::Invalid, Vec::new()),
            _ => (Validity::Unknown, Vec::new()),
        };
        Ok(Input {
            received,
            payload: Payload::new(&payload)?,
            validity,
            counter: None,
            source_time_us: None,
            source_age_us: None,
            diagnostic_code: Some(code),
        })
    }
}
