use crate::decode::{Field, MAX_INPUT, Value};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs, io::Write, path::Path};

mod connection;
mod identity;
pub use identity::IdentityDraft;

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
        Ok(match compared {
            0 if expected == 0 => "no identity pin configured",
            0 => "identity unavailable",
            n if n == expected => "matched all configured identity fields",
            _ => "partial identity match",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gateway {
    #[default]
    Simulator,
    IfmAl1350,
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
    pub condition: Option<String>,
    pub input_bytes: usize,
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

impl Profile {
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
            (1..=MAX_INPUT).contains(&self.input_bytes),
            "input_bytes must be 1–{MAX_INPUT}"
        );
        ensure!((1..=4).contains(&self.connection.port), "port must be 1–4");
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
