use crate::decode::{Field, MAX_INPUT, Value};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Write, path::Path};

mod connection;
mod identity;
mod logging;
mod mapping;
pub use identity::IdentityDraft;
pub use logging::{CapturePlan, PlannedField};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Identity {
    pub vendor_id: Option<u16>,
    pub device_id: Option<u32>,
    pub product_id: Option<String>,
    pub product_name: Option<String>,
    pub serial: Option<String>,
}

impl Identity {
    pub fn verify(&self, actual: &Self) -> Result<&'static str> {
        let (compared, expected) = self.compare(actual)?;
        Ok(match compared {
            0 if expected == 0 => "no identity pin configured",
            0 => "identity unavailable",
            n if n == expected => "matched all configured identity fields",
            _ => "partial identity match",
        })
    }

    pub(crate) fn verify_all(&self, actual: &Self) -> Result<()> {
        let (compared, expected) = self.compare(actual)?;
        ensure!(
            compared == expected,
            "one or more required device identity pins are unavailable"
        );
        Ok(())
    }

    fn compare(&self, actual: &Self) -> Result<(usize, usize)> {
        let mut compared = 0;
        let mut expected = 0;
        macro_rules! compare {
            ($key:ident) => {
                if let Some(wanted) = &self.$key {
                    expected += 1;
                    if let Some(got) = &actual.$key {
                        ensure!(
                            wanted == got,
                            "identity mismatch for {} (expected {}, observed {})",
                            stringify!($key),
                            wanted,
                            got
                        );
                        compared += 1;
                    }
                }
            };
        }
        compare!(vendor_id);
        compare!(device_id);
        compare!(product_id);
        compare!(product_name);
        compare!(serial);
        Ok((compared, expected))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gateway {
    #[default]
    Simulator,
    IfmAl1350,
    IfmAl1352,
}

pub const MAX_PORTS: usize = 8;

impl Gateway {
    pub fn max_port(self) -> u8 {
        match self {
            Self::Simulator | Self::IfmAl1350 => 4,
            Self::IfmAl1352 => MAX_PORTS as u8,
        }
    }

    pub fn ifm_model(self) -> Option<&'static str> {
        match self {
            Self::Simulator => None,
            Self::IfmAl1350 => Some("AL1350"),
            Self::IfmAl1352 => Some("AL1352"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Simulator => "Offline simulator",
            Self::IfmAl1350 => "ifm AL1350",
            Self::IfmAl1352 => "ifm AL1352",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Connection {
    pub ifm_mode: Option<u8>,
    pub ifm_firmware: String,
    pub ifm_cycle_unit: Option<CycleUnit>,
    pub gateway: Gateway,
    pub port: u8,
    pub endpoint: String,
    pub poll_us: u64,
    pub timeout_ms: u64,
    pub stale_us: u64,
    pub reconnect_ms: u64,
    pub simulate_snapshots: bool,
}

impl Default for Connection {
    fn default() -> Self {
        Self {
            gateway: Gateway::Simulator,
            ifm_mode: None,
            ifm_firmware: String::new(),
            ifm_cycle_unit: None,
            port: 1,
            endpoint: String::new(),
            poll_us: 1000,
            timeout_ms: 1000,
            stale_us: 5000,
            reconnect_ms: 50,
            simulate_snapshots: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogMode {
    #[default]
    EveryUpdate,
    OnChange,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Logging {
    #[serde(skip_serializing_if = "is_false")]
    pub raw_only: bool,
    pub mode: LogMode,
    pub queue_capacity: usize,
    pub rotate_rows: u64,
    pub rotate_seconds: u64,
    pub fields: Vec<String>,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogSelection {
    All,
    RawOnly,
    Fields(Vec<String>),
}

impl Default for Logging {
    fn default() -> Self {
        Self {
            raw_only: false,
            mode: LogMode::EveryUpdate,
            queue_capacity: 8192,
            rotate_rows: 0,
            rotate_seconds: 0,
            fields: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Imported {
    pub source: String,
    pub variant: String,
    pub layout: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub acyclic_only: bool,
    pub condition: Option<String>,
    pub input_bytes: usize,
    /// IODD ProcessDataIn bitLength for a standalone padded integer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub padded_input_bit_length: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<crate::iodd::ProcessDataLayout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub communication: Option<crate::iodd::Communication>,
    #[serde(default)]
    pub needs_input: Vec<String>,
    pub fields: Vec<Field>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<crate::iodd::Parameter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub standard_parameter_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_validation: Option<crate::iodd::schema::Report>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stamp_validation: Option<crate::iodd::stamp::Report>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub localization: Option<crate::iodd::Localization>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<crate::iodd::EventDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub standard_event_refs: Vec<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub error_types: Vec<crate::iodd::ErrorTypeDefinition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub standard_error_type_refs: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub version: u32,
    pub name: String,
    pub input_bytes: usize,
    pub connection: Connection,
    pub expected_identity: Identity,
    pub logging: Logging,
    pub iodd: Option<Imported>,
    pub confirm_iodd_condition: bool,
    /// With an IODD, matching names replace imported fields and new names add fields.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            version: 1,
            name: "New profile".into(),
            input_bytes: 2,
            connection: Connection::default(),
            expected_identity: Identity::default(),
            logging: Logging::default(),
            iodd: None,
            confirm_iodd_condition: false,
            fields: Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum InputEncoding<'a> {
    StandaloneBoolean,
    PaddedInteger(&'a Field),
}

impl InputEncoding<'_> {
    pub(crate) fn noncanonical(self, payload: &[u8]) -> bool {
        match self {
            Self::StandaloneBoolean => matches!(payload, [0x01..=0xFE]),
            Self::PaddedInteger(field) => !crate::iodd::integer_padding_matches(field, payload),
        }
    }
}

impl Profile {
    pub fn has_process_input(&self) -> bool {
        self.input_bytes > 0
    }

    pub fn acyclic_only(&self) -> bool {
        self.iodd.as_ref().is_some_and(|import| import.acyclic_only)
    }

    pub fn require_process_input(&self) -> Result<()> {
        let selection = if self.acyclic_only() {
            "acyclic-only"
        } else {
            "output-only"
        };
        ensure!(
            self.has_process_input(),
            "This profile has an {selection} layout. Input monitoring and capture require a process input layout."
        );
        Ok(())
    }

    pub fn require_process_output(&self) -> Result<()> {
        ensure!(
            !self.acyclic_only(),
            "This profile has an acyclic-only layout. Process output operations require an output layout."
        );
        Ok(())
    }

    /// Apply a selected IODD and reset CSV field selection for its layout.
    pub fn apply_iodd(&mut self, identity: Identity, imported: Imported) {
        self.input_bytes = imported.input_bytes;
        self.expected_identity = identity;
        self.iodd = Some(imported);
        self.confirm_iodd_condition = false;
        self.logging.fields.clear();
    }

    /// The original IODD field used to inspect padding at the selected input wire width.
    pub fn padded_input_field(&self) -> Option<&Field> {
        let imported = self.iodd.as_ref()?;
        let bits = imported.padded_input_bit_length?;
        let width = crate::iodd::padded_integer_wire_bytes(bits, &imported.fields)?;
        (width == self.input_bytes && width == imported.input_bytes).then(|| &imported.fields[0])
    }

    pub(crate) fn input_encoding(&self) -> Option<InputEncoding<'_>> {
        let imported = self.iodd.as_ref()?;
        if self.input_bytes == 1
            && imported.input_bytes == 1
            && matches!(
                imported.fields.as_slice(),
                [Field {
                    kind: crate::decode::Kind::Bool,
                    bit_offset: 0,
                    bits: 8,
                    ..
                }]
            )
        {
            return Some(InputEncoding::StandaloneBoolean);
        }
        self.padded_input_field().map(InputEncoding::PaddedInteger)
    }

    /// Explain noncanonical wire bytes without changing the decoded value or transport validity.
    pub fn input_encoding_warning(&self, payload: &[u8]) -> Option<&'static str> {
        let encoding = self.input_encoding()?;
        if payload.len() != self.input_bytes || !encoding.noncanonical(payload) {
            return None;
        }
        Some(match encoding {
            InputEncoding::StandaloneBoolean => {
                "Standalone Boolean input has a noncanonical byte. The IODD interprets any nonzero byte as true."
            }
            InputEncoding::PaddedInteger(_) => {
                "Standalone integer input has noncanonical padding. The IODD value uses the declared low bits."
            }
        })
    }

    pub fn output_layout(&self) -> Option<&crate::iodd::ProcessDataLayout> {
        self.iodd.as_ref()?.output.as_ref()
    }

    pub fn communication(&self) -> Option<&crate::iodd::Communication> {
        self.iodd
            .as_ref()
            .and_then(|iodd| iodd.communication.as_ref())
    }
    /// Host-only presentation choice. Validate a candidate before replacing the profile.
    pub fn set_parameter_display(
        &mut self,
        id: &str,
        subindex: u8,
        display: Option<&str>,
    ) -> Result<()> {
        let mut candidate = self.clone();
        let parameter = candidate
            .iodd
            .as_mut()
            .and_then(|iodd| {
                iodd.parameters
                    .iter_mut()
                    .find(|parameter| parameter.id == id)
            })
            .ok_or_else(|| anyhow::anyhow!("parameter {id} is not in the imported IODD catalog"))?;
        parameter.display_target_mut(subindex)?.preferred_display = display.map(str::to_owned);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn parameter(&self, id: &str) -> Result<&crate::iodd::Parameter> {
        self.iodd
            .as_ref()
            .and_then(|iodd| iodd.parameters.iter().find(|parameter| parameter.id == id))
            .ok_or_else(|| anyhow::anyhow!("parameter {id} is not in the imported IODD catalog"))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let profile: Self = toml::from_str(text)?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::parse(&fs::read_to_string(path)?).with_context(|| path.display().to_string())
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.save_profile(path, true)
    }

    /// Publish a new normalized host profile without replacing an existing file.
    pub fn save_new(&self, path: &Path) -> Result<()> {
        self.save_profile(path, false)
    }

    fn save_profile(&self, path: &Path, replace: bool) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        temporary.as_file().sync_all()?;
        if replace {
            temporary.persist(path)
        } else {
            temporary.persist_noclobber(path)
        }
        .with_context(|| path.display().to_string())?;
        Ok(())
    }

    pub fn effective_fields(&self) -> Vec<(&Field, &'static str)> {
        if !self.has_process_input() {
            return Vec::new();
        }
        let Some(import) = &self.iodd else {
            return self.fields.iter().map(|field| (field, "manual")).collect();
        };
        let mut fields: Vec<_> = import.fields.iter().map(|field| (field, "IODD")).collect();
        for field in &self.fields {
            if let Some(slot) = fields.iter_mut().find(|(base, _)| base.name == field.name) {
                *slot = (field, "override");
            } else {
                fields.push((field, "manual"));
            }
        }
        fields
    }

    /// Apply CSV selection atomically. Mappings and connection settings are preserved.
    pub fn set_log_selection(&mut self, selection: LogSelection) -> Result<()> {
        let old = self.logging.clone();
        match selection {
            LogSelection::All => {
                self.logging.raw_only = false;
                self.logging.fields.clear();
            }
            LogSelection::RawOnly => {
                self.logging.raw_only = true;
                self.logging.fields.clear();
            }
            LogSelection::Fields(fields) => {
                ensure!(
                    !fields.is_empty(),
                    "choose at least one field, or use raw-only logging"
                );
                self.logging.raw_only = false;
                self.logging.fields = fields;
            }
        }
        if let Err(error) = self.validate() {
            self.logging = old;
            return Err(error);
        }
        Ok(())
    }

    pub fn log_fields(&self) -> Vec<&Field> {
        if self.logging.raw_only {
            return Vec::new();
        }
        let fields = self.effective_fields();
        if self.logging.fields.is_empty() {
            return fields.into_iter().map(|(field, _)| field).collect();
        }
        self.logging
            .fields
            .iter()
            .filter_map(|name| {
                fields
                    .iter()
                    .find(|(field, _)| field.name == *name)
                    .map(|(field, _)| *field)
            })
            .collect()
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported profile version {}",
            self.version
        );
        ensure!(!self.name.trim().is_empty(), "profile name is empty");
        ensure!(
            self.input_bytes <= MAX_INPUT,
            "input_bytes must be 1–{MAX_INPUT}, or zero for an imported output-only or acyclic-only layout"
        );
        let no_input_layout = self
            .iodd
            .as_ref()
            .is_some_and(|import| import.input_bytes == 0);
        if let Some(import) = self.iodd.as_ref().filter(|import| import.acyclic_only) {
            ensure!(
                !self.has_process_input()
                    && no_input_layout
                    && import.output.is_none()
                    && import.fields.is_empty()
                    && !import.layout.trim().is_empty(),
                "an acyclic-only layout requires its IODD layout ID, zero input bytes and no process fields"
            );
        } else if no_input_layout || !self.has_process_input() {
            ensure!(
                !self.has_process_input()
                    && no_input_layout
                    && self.iodd.as_ref().is_some_and(|import| {
                        import.fields.is_empty()
                            && import
                                .output
                                .as_ref()
                                .is_some_and(|output| output.id == import.layout)
                    }),
                "zero input width requires a matching imported output-only layout and no imported input fields"
            );
        }
        let max_port = self.connection.gateway.max_port();
        ensure!(
            (1..=max_port).contains(&self.connection.port),
            "port must be 1–{max_port}"
        );
        ensure!(
            (100..=60_000_000).contains(&self.connection.poll_us),
            "poll_us must be 100–60000000"
        );
        ensure!(
            (1..=30_000).contains(&self.connection.timeout_ms),
            "timeout_ms must be 1–30000"
        );
        ensure!(
            (1..=60_000).contains(&self.connection.reconnect_ms),
            "reconnect_ms must be 1–60000"
        );
        ensure!(
            (1..=1_000_000).contains(&self.logging.queue_capacity),
            "queue_capacity must be 1–1000000"
        );
        ensure!(
            self.expected_identity.device_id.unwrap_or_default() <= 0xFF_FFFF,
            "Device ID exceeds 24 bits"
        );
        let mut names = HashSet::new();
        if let Some(imported) = &self.iodd {
            if let Some(bits) = imported.padded_input_bit_length {
                ensure!(
                    crate::iodd::padded_integer_wire_bytes(bits, &imported.fields)
                        == Some(imported.input_bytes),
                    "invalid IODD padded integer input geometry"
                );
            }
            if let Some(output) = &imported.output {
                output.validate().context("invalid IODD output layout")?;
                ensure!(
                    output.condition == imported.condition,
                    "IODD input and output layout conditions differ"
                );
            }
            if let Some(communication) = &imported.communication {
                communication.validate()?;
            }
            let mut codes = HashSet::new();
            for event in &imported.events {
                event.validate()?;
                ensure!(codes.insert(event.code), "duplicate event code");
            }
            for code in &imported.standard_event_refs {
                ensure!(
                    codes.insert(*code),
                    "duplicate or resolved standard event reference"
                );
            }
            let mut error_codes = HashSet::new();
            for definition in &imported.error_types {
                definition.validate()?;
                ensure!(
                    error_codes.insert(definition.code),
                    "duplicate saved IODD error type"
                );
            }
            for code in &imported.standard_error_type_refs {
                ensure!(
                    code >> 8 == 0x80,
                    "standard ErrorType references require ErrorCode 0x80"
                );
                ensure!(
                    error_codes.insert(*code),
                    "duplicate or resolved standard ErrorType reference"
                );
            }
            let mut indexes = HashSet::new();
            if let Some(validation) = &imported.schema_validation {
                validation.validate_receipt()?;
            }
            if let Some(validation) = &imported.stamp_validation {
                validation.validate_receipt()?;
                validation.enforce(false)?;
                if let Some(schema) = &imported.schema_validation {
                    ensure!(
                        schema.xml_sha256 == validation.main.sha256,
                        "XSD and stamp receipts describe different main XML bytes"
                    );
                }
            }
            for parameter in &imported.parameters {
                parameter.validate()?;
                ensure!(
                    names.insert(&parameter.id),
                    "duplicate parameter ID {}",
                    parameter.id
                );
                ensure!(
                    indexes.insert(parameter.index),
                    "duplicate parameter index {}",
                    parameter.index
                );
            }
            names.clear();
        }
        for field in &self.fields {
            ensure!(names.insert(&field.name), "duplicate field {}", field.name);
            if no_input_layout {
                field.validate(MAX_INPUT)?;
            }
        }
        names.clear();
        for (field, _) in self.effective_fields() {
            ensure!(
                names.insert(&field.name),
                "duplicate effective field {}",
                field.name
            );
            field.validate(self.input_bytes)?;
        }
        ensure!(
            !self.logging.raw_only || self.logging.fields.is_empty(),
            "raw-only logging cannot also select typed fields"
        );
        let mut selected = HashSet::new();
        for name in &self.logging.fields {
            ensure!(names.contains(name), "log field {name} does not exist");
            ensure!(selected.insert(name), "duplicate log selection {name}");
        }
        let mut labels = HashSet::new();
        for label in crate::capture::csv_header(self) {
            ensure!(
                labels.insert(label.clone()),
                "CSV column label collides: {label}"
            );
        }
        Ok(())
    }

    pub fn decode(&self, payload: &[u8]) -> Result<Vec<(&Field, &'static str, Value)>> {
        self.require_process_input()?;
        ensure!(
            payload.len() == self.input_bytes,
            "expected {} bytes, received {}",
            self.input_bytes,
            payload.len()
        );
        self.effective_fields()
            .into_iter()
            .map(|(field, origin)| Ok((field, origin, field.decode(payload)?)))
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleUnit {
    Microseconds,
    Milliseconds,
}

#[cfg(test)]
mod tests;
