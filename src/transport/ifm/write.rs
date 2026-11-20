//! Explicit single-write operations for the documented AL1350 IoT Core route.
use super::{Client, InputTransport, label, number};
use crate::{
    decode::{MAX_INPUT, hex, parse_hex_bounded},
    iodd::Access,
    profile::{Connection, Gateway, Identity, Profile},
};
use anyhow::{Context, Result, ensure};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SYSTEM_COMMAND: u16 = 2;
const MAX_ISDU_BYTES: usize = 232;

pub enum Operation {
    PortMode {
        expected: u8,
        desired: u8,
    },
    Isdu {
        index: u16,
        subindex: u8,
        expected: String,
        desired: String,
    },
    DeviceCommand {
        code: u8,
        meaning: String,
    },
    ProcessOutput {
        expected: String,
        desired: String,
    },
}

#[derive(Serialize)]
pub struct Review {
    format_version: u32,
    pub model: &'static str,
    pub endpoint: String,
    pub firmware: String,
    pub master_serial: String,
    pub port: u8,
    pub numeric_io_link_mode: Option<u8>,
    pub expected_device: Option<Identity>,
    pub operation: &'static str,
    pub address: String,
    pub request_data: Value,
    pub expected_value: Option<Value>,
    pub requested_value: Value,
    pub command_meaning: Option<String>,
}

pub struct Plan {
    connection: Connection,
    review: Review,
    confirmation: String,
    operation: Operation,
}

#[derive(Serialize)]
pub struct Preview<'a> {
    confirmation: &'a str,
    target_and_request: &'a Review,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    ReadbackMatched,
    GatewayAcknowledged,
    Unconfirmed,
}

#[derive(Serialize)]
pub struct Receipt<'a> {
    pub outcome: Outcome,
    preview: Preview<'a>,
    dispatch_utc: String,
    gateway_code: Option<u64>,
    readback: Option<Value>,
    pub detail: Option<String>,
}

impl Plan {
    /// Construct a review entirely offline. Credentials stay in the existing transport.
    pub fn new(profile: &Profile, master_serial: &str, operation: Operation) -> Result<Self> {
        profile.validate()?;
        let connection = &profile.connection;
        ensure!(
            connection.gateway == Gateway::IfmAl1350,
            "writes use the AL1350 adapter"
        );
        super::check_route(connection)?;
        let firmware = &connection.ifm_firmware;
        ensure!(
            firmware.strip_prefix("3.1.").is_some_and(
                |patch| !patch.is_empty() && patch.bytes().all(|byte| byte.is_ascii_digit())
            ),
            "set an exact AL1350 3.1.x firmware pin before planning a write"
        );
        ensure!(
            !master_serial.trim().is_empty(),
            "set the expected AL1350 serial number"
        );

        let port = format!("/iolinkmaster/port[{}]", connection.port);
        let device = format!("{port}/iolinkdevice");
        let expected_device = if matches!(operation, Operation::PortMode { .. }) {
            None
        } else {
            let pins = &profile.expected_identity;
            ensure!(
                pins.vendor_id.is_some_and(|id| id != 0)
                    && pins.device_id.is_some_and(|id| id != 0),
                "pin the connected device's Vendor ID and Device ID before planning a write"
            );
            ensure!(
                profile.identity_variant_note().is_none(),
                "expected Product ID differs from the selected IODD variant"
            );
            Some(pins.clone())
        };
        let (kind, address, request_data, expected_value, requested_value, command_meaning) =
            match &operation {
                Operation::PortMode { expected, desired } => (
                    "port_mode",
                    format!("{port}/mode/setdata"),
                    json!({"newvalue": desired}),
                    Some(json!(expected)),
                    json!(desired),
                    None,
                ),
                Operation::Isdu {
                    index,
                    subindex,
                    expected,
                    desired,
                } => {
                    ensure!(
                        matches!(*index, 4..=254 | 256..=0x41ff | 0x4300..=0x4fff),
                        "use a parameter index supported by this write path. Use device-command for SystemCommand"
                    );
                    check_iodd_access(profile, *index, *subindex)?;
                    let before = hex(&parse_hex_bounded(expected, MAX_ISDU_BYTES)?);
                    let after = hex(&parse_hex_bounded(desired, MAX_ISDU_BYTES)?);
                    (
                        "isdu",
                        format!("{device}/iolwriteacyclic"),
                        json!({"index": index, "subindex": subindex, "value": after}),
                        Some(json!(before)),
                        json!(after),
                        None,
                    )
                }
                Operation::DeviceCommand { code, meaning } => {
                    ensure!(
                        matches!(*code, 0x40..=0x83 | 0xa0..=0xff),
                        "use a documented one-shot SystemCommand code"
                    );
                    ensure!(
                        !meaning.trim().is_empty(),
                        "state the documented meaning of this command"
                    );
                    let value = hex(&[*code]);
                    (
                        "device_command",
                        format!("{device}/iolwriteacyclic"),
                        json!({"index": SYSTEM_COMMAND, "subindex": 0, "value": value}),
                        None,
                        json!(value),
                        Some(meaning.clone()),
                    )
                }
                Operation::ProcessOutput { expected, desired } => {
                    let before = parse_hex_bounded(expected, MAX_INPUT)?;
                    let after = parse_hex_bounded(desired, MAX_INPUT)?;
                    ensure!(
                        before.len() == after.len(),
                        "process output requires the same full byte width before and after"
                    );
                    (
                        "process_output",
                        format!("{device}/pdout/setdata"),
                        json!({"newvalue": hex(&after)}),
                        Some(json!(hex(&before))),
                        json!(hex(&after)),
                        None,
                    )
                }
            };
        ensure!(
            expected_value.as_ref() != Some(&requested_value),
            "requested value already equals expected value"
        );
        let review = Review {
            format_version: 1,
            model: "AL1350",
            endpoint: connection.origin()?.to_string(),
            firmware: firmware.clone(),
            master_serial: master_serial.into(),
            port: connection.port,
            numeric_io_link_mode: expected_device.as_ref().and(connection.ifm_mode),
            expected_device,
            operation: kind,
            address,
            request_data,
            expected_value,
            requested_value,
            command_meaning,
        };
        let confirmation = format!("{:x}", Sha256::digest(serde_json::to_vec(&review)?));
        Ok(Self {
            connection: connection.clone(),
            review,
            confirmation,
            operation,
        })
    }

    pub fn preview(&self) -> Preview<'_> {
        Preview {
            confirmation: &self.confirmation,
            target_and_request: &self.review,
        }
    }

    pub fn review(&self) -> &Review {
        &self.review
    }

    pub fn confirmation(&self) -> &str {
        &self.confirmation
    }

    /// Recheck the reviewed target and current value, then dispatch at most one write request.
    pub fn apply(&self, confirmation: &str) -> Result<Receipt<'_>> {
        ensure!(
            confirmation == self.confirmation,
            "confirmation mismatch. Generate and review the write preview again"
        );
        let mut client = Client::new(&self.connection)?;
        let current = self
            .read_current(&mut client)
            .context("write preflight failed before dispatch")?;
        ensure!(
            current == self.review.expected_value,
            "write blocked before dispatch: expected current {:?}, observed {:?}",
            self.review.expected_value,
            current
        );
        let mut receipt = Receipt {
            outcome: Outcome::Unconfirmed,
            preview: self.preview(),
            dispatch_utc: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            gateway_code: None,
            readback: None,
            detail: None,
        };
        match client.exchange(&self.review.address, self.review.request_data.clone()) {
            Ok((_, response)) => receipt.gateway_code = response["code"].as_u64(),
            Err(error) => {
                receipt.detail = Some(format!("Write response unavailable: {error:#}"));
                return Ok(receipt);
            }
        }
        if receipt.gateway_code != Some(200) {
            receipt.detail = Some("Gateway did not return a success code for the write".into());
            return Ok(receipt);
        }
        if matches!(self.operation, Operation::DeviceCommand { .. }) {
            receipt.outcome = Outcome::GatewayAcknowledged;
            return Ok(receipt);
        }
        match self.read_current(&mut client) {
            Ok(value) => {
                if value.as_ref() == Some(&self.review.requested_value) {
                    receipt.outcome = Outcome::ReadbackMatched;
                } else {
                    receipt.detail =
                        Some("Gateway acknowledged the write but readback differed".into());
                }
                receipt.readback = value;
            }
            Err(error) => {
                receipt.detail = Some(format!(
                    "Gateway acknowledged the write but readback failed: {error:#}"
                ))
            }
        }
        Ok(receipt)
    }

    fn read_current(&self, client: &mut Client) -> Result<Option<Value>> {
        if matches!(self.operation, Operation::PortMode { .. }) {
            let mode = format!("{}/mode", client.port);
            let model = "/deviceinfo/productcode";
            let firmware = "/deviceinfo/swrevision";
            let serial = "/deviceinfo/serialnumber";
            let (_, response) =
                client.multi(&[model.into(), firmware.into(), serial.into(), mode.clone()])?;
            let get = |path| Client::data(&response, path);
            ensure!(
                get(model).as_str() == Some(self.review.model),
                "AL1350 model pin mismatch"
            );
            ensure!(
                get(firmware).as_str() == Some(self.review.firmware.as_str()),
                "AL1350 firmware pin mismatch"
            );
            self.check_serial(label(get(serial)).as_deref())?;
            let current = number(get(&mode))
                .and_then(|mode| u8::try_from(mode).ok())
                .context("AL1350 port did not report a numeric mode")?;
            return Ok(Some(json!(current)));
        }
        let mut status = client.connect()?;
        self.check_serial(status.master_serial.as_deref())?;
        let expected = self
            .review
            .expected_device
            .as_ref()
            .context("device write target missing identity pins")?;
        if expected.product_id.is_some() {
            status.identity.product_id = Some(read_id(client, 19)?);
        }
        if expected.serial.is_some() && status.identity.serial.is_none() {
            status.identity.serial = Some(read_id(client, 21)?);
        }
        expected.verify_all(&status.identity)?;
        match &self.operation {
            Operation::Isdu {
                index, subindex, ..
            } => Ok(Some(json!(hex(&client.read_isdu(*index, *subindex)?)))),
            Operation::DeviceCommand { .. } => Ok(None),
            Operation::ProcessOutput { .. } => {
                let address = format!("{}/pdout/getdata", client.device);
                let (_, response) = client.request(&address, json!({}))?;
                let value = response["data"]["value"]
                    .as_str()
                    .context("AL1350 process output is not a hex string")?;
                Ok(Some(json!(hex(&parse_hex_bounded(value, MAX_INPUT)?))))
            }
            Operation::PortMode { .. } => unreachable!("port mode handled before device lookup"),
        }
    }

    fn check_serial(&self, actual: Option<&str>) -> Result<()> {
        ensure!(
            actual == Some(self.review.master_serial.as_str()),
            "AL1350 serial pin mismatch or unavailable"
        );
        Ok(())
    }
}

fn read_id(client: &mut Client, index: u16) -> Result<String> {
    Ok(std::str::from_utf8(&client.read_isdu(index, 0)?)?
        .trim_end_matches('\0')
        .into())
}

fn check_iodd_access(profile: &Profile, index: u16, subindex: u8) -> Result<()> {
    if let Some(parameter) = profile.iodd.as_ref().and_then(|iodd| {
        iodd.parameters
            .iter()
            .find(|parameter| parameter.index == index)
    }) {
        ensure!(
            subindex == 0 || parameter.subindex_access(),
            "IODD does not permit access to this subindex"
        );
        ensure!(
            parameter.access == Access::ReadWrite
                && parameter.target(subindex)?.access == Access::ReadWrite,
            "IODD does not declare this parameter read/write"
        );
    }
    Ok(())
}
