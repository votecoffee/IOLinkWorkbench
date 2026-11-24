//! Checked reads and explicit single-write operations for the AL1350 IoT Core route.
use super::{Client, InputTransport, label, number};
use crate::{
    decode::{MAX_INPUT, hex, parse_hex_bounded},
    diagnostics::{DATA_STORAGE_INDEX, ERROR_COUNT_INDEX},
    iodd::{
        Access, DecodedOutput, DecodedParameter, Parameter, ProcessDataLayout,
        decode_text_parameter,
    },
    profile::{Connection, Gateway, Identity, Profile},
};
use anyhow::{Context, Result, ensure};
use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod output;
pub use output::ProcessOutputReview;

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

impl Operation {
    /// Edit one IODD field in the full expected gateway output image.
    pub fn process_output_field(
        profile: &Profile,
        expected: String,
        field: &str,
        desired_value: &str,
    ) -> Result<Self> {
        profile.require_process_output()?;
        let layout = profile
            .output_layout()
            .context("typed process output requires an imported IODD output layout")?;
        let image = parse_hex_bounded(&expected, MAX_INPUT)?;
        let proposed = layout.edit_output_field(image, field, desired_value)?;
        Ok(Self::ProcessOutput {
            expected,
            desired: hex(&proposed),
        })
    }

    /// Encode a desired IODD value locally for the same byte-for-byte review as a raw write.
    pub fn isdu_typed(
        profile: &Profile,
        index: u16,
        subindex: u8,
        expected: String,
        desired_value: &str,
    ) -> Result<Self> {
        let parameter = ReadPoint::Isdu { index, subindex }
            .validate(profile)?
            .context("typed ISDU entry requires an IODD declaration for this parameter")?;
        let bytes = parameter
            .encode_typed_write(desired_value)
            .with_context(|| format!("desired value for {}", parameter.name))?;
        Ok(Self::Isdu {
            index,
            subindex,
            expected,
            desired: hex(&bytes),
        })
    }

    /// Derive a whole-parameter write image by replacing one writable IODD member.
    pub fn isdu_member(
        profile: &Profile,
        index: u16,
        member_subindex: u8,
        expected: String,
        value: &str,
    ) -> Result<Self> {
        let parameter = ReadPoint::Isdu { index, subindex: 0 }
            .validate(profile)?
            .context("member editing requires an IODD declaration for this parameter")?;
        let before = parse_hex_bounded(&expected, MAX_ISDU_BYTES)?;
        let proposed = parameter.edit_packed_member(&before, member_subindex, value)?;
        Ok(Self::Isdu {
            index,
            subindex: 0,
            expected,
            desired: hex(&proposed),
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadPoint {
    PortMode,
    Isdu { index: u16, subindex: u8 },
    ProcessOutput,
}

struct Target {
    connection: Connection,
    endpoint: String,
    master_serial: String,
    expected_device: Option<Identity>,
}

/// A checked read of a prospective write target. It contains no desired value or write request.
pub struct CurrentRead {
    target: Target,
    point: ReadPoint,
    parameter: Option<Parameter>,
    output: Option<(ProcessDataLayout, bool)>,
}

enum ReadValue {
    Mode(u8),
    Bytes(Vec<u8>),
}

impl From<ReadValue> for Value {
    fn from(value: ReadValue) -> Self {
        match value {
            ReadValue::Mode(mode) => Value::from(mode),
            ReadValue::Bytes(bytes) => Value::String(hex(&bytes)),
        }
    }
}

#[derive(Serialize)]
pub struct CurrentValue {
    pub endpoint: String,
    pub port: u8,
    pub master_serial: String,
    pub firmware: String,
    pub expected_device: Option<Identity>,
    pub point: ReadPoint,
    pub observed_utc: String,
    pub value: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iodd_value: Option<CurrentIoddValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iodd_output: Option<DecodedOutput>,
}

#[derive(Serialize)]
pub struct CurrentIoddValue {
    pub parameter_id: String,
    pub name: String,
    pub datatype: String,
    pub interpretation: DecodedParameter,
}

impl CurrentValue {
    pub fn value_text(&self) -> String {
        self.value
            .as_str()
            .map_or_else(|| self.value.to_string(), str::to_owned)
    }
}

impl CurrentRead {
    pub fn new(profile: &Profile, master_serial: &str, point: ReadPoint) -> Result<Self> {
        let target = Target::new(profile, master_serial, point != ReadPoint::PortMode)?;
        let parameter = point.validate(profile)?.cloned();
        let output = match point {
            ReadPoint::ProcessOutput => profile
                .output_layout()
                .map(|layout| (layout.clone(), profile.confirm_iodd_condition)),
            _ => None,
        };
        Ok(Self {
            target,
            point,
            parameter,
            output,
        })
    }

    pub fn read(self) -> Result<CurrentValue> {
        let mut client = Client::new(&self.target.connection)?;
        let value = self.target.read(&mut client, self.point)?;
        let observed_utc = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let iodd_value = match (self.parameter, &value) {
            (Some(parameter), ReadValue::Bytes(bytes)) => {
                let interpretation = parameter.decode(bytes);
                Some(CurrentIoddValue {
                    parameter_id: parameter.id,
                    name: parameter.name,
                    datatype: parameter.datatype,
                    interpretation,
                })
            }
            _ => None,
        };
        let iodd_output = match (self.output, &value) {
            (Some((layout, confirmed)), ReadValue::Bytes(bytes)) => {
                Some(layout.decode_output(bytes, Some(confirmed)))
            }
            _ => None,
        };
        Ok(CurrentValue {
            endpoint: self.target.endpoint,
            port: self.target.connection.port,
            master_serial: self.target.master_serial,
            firmware: self.target.connection.ifm_firmware,
            expected_device: self.target.expected_device,
            point: self.point,
            observed_utc,
            value: value.into(),
            iodd_value,
            iodd_output,
        })
    }
}

impl ReadPoint {
    pub fn typed_isdu_parameter(self, profile: &Profile) -> Option<&Parameter> {
        self.validate(profile)
            .ok()?
            .filter(|parameter| parameter.supports_typed_write())
    }

    /// Members that can be edited in a whole-image draft, including when direct subindex access is unavailable.
    pub fn typed_isdu_members(self, profile: &Profile) -> Vec<(u8, &Parameter)> {
        if !matches!(self, Self::Isdu { subindex: 0, .. }) {
            return Vec::new();
        }
        let Some(parameter) = self.validate(profile).ok().flatten() else {
            return Vec::new();
        };
        if parameter.check_write_layout().is_err() {
            return Vec::new();
        }
        parameter.typed_write_members().collect()
    }

    fn validate(self, profile: &Profile) -> Result<Option<&Parameter>> {
        match self {
            Self::Isdu { index, subindex } => {
                check_isdu_index(index)?;
                writable_parameter(profile, index, subindex)
            }
            Self::ProcessOutput => {
                profile.require_process_output()?;
                Ok(None)
            }
            Self::PortMode => Ok(None),
        }
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isdu_value_review: Option<IsduValueReview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_output_review: Option<ProcessOutputReview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_readback: Option<TextReadback>,
}

#[derive(Serialize)]
pub struct TextReadback {
    fixed_length: u16,
    encoding: String,
}

impl TextReadback {
    fn for_parameter(parameter: &Parameter) -> Option<Self> {
        if parameter.datatype != "StringT" || !parameter.supports_typed_write() {
            return None;
        }
        Some(Self {
            fixed_length: parameter.fixed_length?,
            encoding: parameter.encoding.clone()?,
        })
    }

    fn matches(&self, requested: &Value, observed: &Value) -> bool {
        let decode = |value: &Value| {
            let bytes = parse_hex_bounded(value.as_str()?, MAX_ISDU_BYTES).ok()?;
            decode_text_parameter(
                "StringT",
                Some(self.fixed_length),
                Some(&self.encoding),
                &bytes,
            )
            .ok()
        };
        matches!((decode(requested), decode(observed)), (Some(requested), Some(observed)) if requested == observed)
    }
}

#[derive(Serialize)]
#[serde(tag = "basis", rename_all = "snake_case")]
pub enum IsduValueReview {
    NoIoddParameter,
    Iodd {
        parameter_id: String,
        name: String,
        datatype: String,
        interpretation: Box<DecodedParameter>,
        #[serde(skip_serializing_if = "Option::is_none")]
        manual_review: Option<String>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        preserved_read_only: Vec<u8>,
    },
}

#[derive(Serialize)]
pub struct IsduComparison {
    pub expected: Box<DecodedParameter>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changed_members: Vec<IsduMemberChange>,
}

#[derive(Serialize)]
pub struct IsduMemberChange {
    pub subindex: u8,
    pub name: String,
    pub bit_offset: u16,
    pub expected_hex: String,
    pub proposed_hex: String,
}

impl IsduComparison {
    fn new(expected: DecodedParameter, proposed: &DecodedParameter) -> Self {
        // Both images use the same validated compound layout and declared member order.
        let before = expected.record_items.iter().chain(&expected.array_items);
        let after = proposed.record_items.iter().chain(&proposed.array_items);
        let changed_members = before
            .zip(after)
            .filter(|(before, after)| before.value.raw_hex != after.value.raw_hex)
            .map(|(before, after)| IsduMemberChange {
                subindex: after.subindex,
                name: after.name.clone(),
                bit_offset: after.bit_offset,
                expected_hex: before.value.raw_hex.clone(),
                proposed_hex: after.value.raw_hex.clone(),
            })
            .collect();
        Self {
            expected: Box::new(expected),
            changed_members,
        }
    }
}

pub struct Plan {
    target: Target,
    review: Review,
    confirmation: String,
    point: Option<ReadPoint>,
    isdu_comparison: Option<IsduComparison>,
}

#[derive(Serialize)]
pub struct Preview<'a> {
    confirmation: &'a str,
    target_and_request: &'a Review,
    #[serde(skip_serializing_if = "Option::is_none")]
    isdu_comparison: Option<&'a IsduComparison>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    ReadbackMatched,
    TextReadbackMatched,
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

impl Target {
    fn new(profile: &Profile, master_serial: &str, device: bool) -> Result<Self> {
        profile.validate()?;
        let connection = &profile.connection;
        ensure!(
            connection.gateway == Gateway::IfmAl1350,
            "checked write targets use the AL1350 adapter"
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

        let expected_device = if device {
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
        } else {
            None
        };
        Ok(Self {
            connection: connection.clone(),
            endpoint: connection.origin()?.to_string(),
            master_serial: master_serial.into(),
            expected_device,
        })
    }
}

impl Plan {
    /// Construct a review entirely offline. Credentials stay in the existing transport.
    pub fn new(profile: &Profile, master_serial: &str, operation: Operation) -> Result<Self> {
        let point = match operation {
            Operation::PortMode { .. } => Some(ReadPoint::PortMode),
            Operation::Isdu {
                index, subindex, ..
            } => Some(ReadPoint::Isdu { index, subindex }),
            Operation::ProcessOutput { .. } => Some(ReadPoint::ProcessOutput),
            Operation::DeviceCommand { .. } => None,
        };
        let target = Target::new(profile, master_serial, point != Some(ReadPoint::PortMode))?;
        let iodd_parameter = match point {
            Some(point) => point.validate(profile)?,
            None => None,
        };
        let port = format!("/iolinkmaster/port[{}]", target.connection.port);
        let device = format!("{port}/iolinkdevice");
        let mut isdu_value_review = None;
        let mut isdu_comparison = None;
        let mut process_output_review = None;
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
                    let before = parse_hex_bounded(expected, MAX_ISDU_BYTES)?;
                    let proposed = parse_hex_bounded(desired, MAX_ISDU_BYTES)?;
                    let (review, comparison) =
                        review_isdu_value(iodd_parameter, &before, &proposed)?;
                    isdu_value_review = Some(review);
                    isdu_comparison = comparison;
                    let after = hex(&proposed);
                    (
                        "isdu",
                        format!("{device}/iolwriteacyclic"),
                        json!({"index": index, "subindex": subindex, "value": after}),
                        Some(json!(hex(&before))),
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
                    if let Some(layout) = profile.output_layout() {
                        ensure!(
                            layout.condition.is_none() || profile.confirm_iodd_condition,
                            "confirm the active IODD layout condition before preparing an output write"
                        );
                        process_output_review =
                            Some(ProcessOutputReview::new(layout, &before, &after)?);
                    }
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
            endpoint: target.endpoint.clone(),
            firmware: target.connection.ifm_firmware.clone(),
            master_serial: target.master_serial.clone(),
            port: target.connection.port,
            numeric_io_link_mode: target
                .expected_device
                .as_ref()
                .and(target.connection.ifm_mode),
            expected_device: target.expected_device.clone(),
            operation: kind,
            address,
            request_data,
            expected_value,
            requested_value,
            command_meaning,
            isdu_value_review,
            process_output_review,
            text_readback: iodd_parameter.and_then(TextReadback::for_parameter),
        };
        let confirmation = format!("{:x}", Sha256::digest(serde_json::to_vec(&review)?));
        Ok(Self {
            target,
            review,
            confirmation,
            point,
            isdu_comparison,
        })
    }

    pub fn preview(&self) -> Preview<'_> {
        Preview {
            confirmation: &self.confirmation,
            target_and_request: &self.review,
            isdu_comparison: self.isdu_comparison(),
        }
    }

    pub fn review(&self) -> &Review {
        &self.review
    }

    pub fn read_point(&self) -> Option<ReadPoint> {
        self.point
    }

    pub fn isdu_comparison(&self) -> Option<&IsduComparison> {
        self.isdu_comparison.as_ref()
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
        let mut client = Client::new(&self.target.connection)?;
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
        if self.point.is_none() {
            receipt.outcome = Outcome::GatewayAcknowledged;
            return Ok(receipt);
        }
        match self.read_current(&mut client) {
            Ok(value) => {
                if value.as_ref() == Some(&self.review.requested_value) {
                    receipt.outcome = Outcome::ReadbackMatched;
                } else if let Some(policy) = &self.review.text_readback
                    && let Some(observed) = &value
                    && policy.matches(&self.review.requested_value, observed)
                {
                    receipt.outcome = Outcome::TextReadbackMatched;
                    receipt.detail =
                        Some("The same IODD text was returned with different zero padding".into());
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
        if let Some(point) = self.point {
            self.target
                .read(client, point)
                .map(|value| Some(value.into()))
        } else {
            self.target.check_device(client)?;
            Ok(None)
        }
    }
}

impl Target {
    fn read(&self, client: &mut Client, point: ReadPoint) -> Result<ReadValue> {
        if point == ReadPoint::PortMode {
            let mode = format!("{}/mode", client.port);
            let model = "/deviceinfo/productcode";
            let firmware = "/deviceinfo/swrevision";
            let serial = "/deviceinfo/serialnumber";
            let (_, response) =
                client.multi(&[model.into(), firmware.into(), serial.into(), mode.clone()])?;
            let get = |path| Client::data(&response, path);
            ensure!(
                get(model).as_str() == Some("AL1350"),
                "AL1350 model pin mismatch"
            );
            ensure!(
                get(firmware).as_str() == Some(self.connection.ifm_firmware.as_str()),
                "AL1350 firmware pin mismatch"
            );
            self.check_serial(label(get(serial)).as_deref())?;
            let current = number(get(&mode))
                .and_then(|mode| u8::try_from(mode).ok())
                .context("AL1350 port did not report a numeric mode")?;
            return Ok(ReadValue::Mode(current));
        }
        self.check_device(client)?;
        match point {
            ReadPoint::Isdu { index, subindex } => {
                Ok(ReadValue::Bytes(client.read_isdu(index, subindex)?))
            }
            ReadPoint::ProcessOutput => {
                let address = format!("{}/pdout/getdata", client.device);
                let (_, response) = client.request(&address, json!({}))?;
                let value = response["data"]["value"]
                    .as_str()
                    .context("AL1350 process output is not a hex string")?;
                Ok(ReadValue::Bytes(parse_hex_bounded(value, MAX_INPUT)?))
            }
            ReadPoint::PortMode => unreachable!("port mode handled before device lookup"),
        }
    }

    fn check_device(&self, client: &mut Client) -> Result<()> {
        let mut status = client.connect()?;
        self.check_serial(status.master_serial.as_deref())?;
        let expected = self
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
        Ok(())
    }

    fn check_serial(&self, actual: Option<&str>) -> Result<()> {
        ensure!(
            actual == Some(self.master_serial.as_str()),
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

fn check_isdu_index(index: u16) -> Result<()> {
    ensure!(
        index != ERROR_COUNT_INDEX,
        "ErrorCount index 32 is read-only"
    );
    ensure!(
        index != DATA_STORAGE_INDEX,
        "DataStorageIndex 3 is reserved for master/device exchanges"
    );
    ensure!(
        matches!(index, 4..=254 | 256..=0x41ff | 0x4300..=0x4fff),
        "use a parameter index supported by this write path. Use device-command for SystemCommand"
    );
    Ok(())
}

/// Check whether a known IODD parameter or member can be selected for write review.
pub fn isdu_write_target(parameter: &Parameter, subindex: u8) -> Result<&Parameter> {
    check_isdu_index(parameter.index)?;
    let selected = read_write_subindex(parameter, subindex)?;
    selected.check_write_layout()?;
    Ok(selected)
}

fn writable_parameter(profile: &Profile, index: u16, subindex: u8) -> Result<Option<&Parameter>> {
    let Some(parameter) = profile.iodd.as_ref().and_then(|iodd| {
        iodd.parameters
            .iter()
            .find(|parameter| parameter.index == index)
    }) else {
        return Ok(None);
    };
    read_write_subindex(parameter, subindex).map(Some)
}

fn read_write_subindex(parameter: &Parameter, subindex: u8) -> Result<&Parameter> {
    ensure!(
        subindex == 0 || parameter.subindex_access(),
        "IODD does not permit access to this subindex"
    );
    let selected = parameter.target(subindex)?;
    ensure!(
        parameter.access == Access::ReadWrite && selected.access == Access::ReadWrite,
        "IODD does not declare this parameter read/write"
    );
    Ok(selected)
}

fn review_isdu_value(
    parameter: Option<&Parameter>,
    expected: &[u8],
    proposed: &[u8],
) -> Result<(IsduValueReview, Option<IsduComparison>)> {
    let Some(parameter) = parameter else {
        return Ok((IsduValueReview::NoIoddParameter, None));
    };
    let reviewed = parameter
        .review_write(expected, proposed)
        .with_context(|| format!("ISDU write review for {}", parameter.name))?;
    let comparison = IsduComparison::new(parameter.decode(expected), &reviewed.decoded);
    let review = IsduValueReview::Iodd {
        parameter_id: parameter.id.clone(),
        name: parameter.name.clone(),
        datatype: parameter.datatype.clone(),
        interpretation: Box::new(reviewed.decoded),
        manual_review: reviewed.manual_review,
        preserved_read_only: reviewed.preserved_read_only,
    };
    Ok((review, Some(comparison)))
}
