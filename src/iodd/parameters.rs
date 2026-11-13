use super::{IODD_NAMESPACE, XSI, child, datatype, label};
use crate::decode::{Field, Kind};
use anyhow::{Context, Result, ensure};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

mod display;
mod record;
mod standard;
mod value;
pub use display::{Display, DisplayValue};
pub use record::{Record, RecordItem};
pub(super) use standard::load as load_standard;
pub use value::{DecodedParameter, RecordValue};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Access {
    #[serde(rename = "ro")]
    ReadOnly,
    #[serde(rename = "rw")]
    ReadWrite,
    #[serde(rename = "wo")]
    WriteOnly,
}

impl Access {
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadOnly => "ro",
            Self::ReadWrite => "rw",
            Self::WriteOnly => "wo",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub value: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueRange {
    pub lower: String,
    pub upper: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub id: String,
    pub name: String,
    pub index: u16,
    pub access: Access,
    pub datatype: String,
    pub bits: Option<u16>,
    pub fixed_length: Option<u16>,
    pub encoding: Option<String>,
    pub default_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<Record>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub displays: Vec<Display>,
    #[serde(default)]
    pub choices: Vec<Choice>,
    #[serde(default)]
    pub ranges: Vec<ValueRange>,
    #[serde(default)]
    pub needs_input: Vec<String>,
}

impl Parameter {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.trim().is_empty() && !self.name.trim().is_empty(),
            "parameter ID or name is empty"
        );
        ensure!(
            !self.datatype.trim().is_empty(),
            "parameter {} has no datatype description",
            self.id
        );
        if let Some(record) = &self.record {
            record.validate(self)?;
        }
        let mut ids = HashSet::new();
        for display in &self.displays {
            ensure!(
                !display.id.trim().is_empty(),
                "parameter display ID is empty"
            );
            ensure!(ids.insert(&display.id), "duplicate parameter display ID");
            let raw = self.scalar_field()?;
            ensure!(
                display.field.bits == raw.bits
                    && display.field.kind == raw.kind
                    && display.field.bit_offset == 0
                    && display.field.byte_order == raw.byte_order,
                "parameter display must preserve the declared raw layout"
            );
            display.field.validate(usize::from(raw.bits).div_ceil(8))?;
        }
        Ok(())
    }

    pub fn target(&self, subindex: u8) -> Result<&Parameter> {
        if subindex == 0 {
            return Ok(self);
        }
        self.record
            .as_ref()
            .and_then(|record| record.items.iter().find(|item| item.subindex == subindex))
            .map(|item| &item.parameter)
            .with_context(|| format!("parameter {} has no declared subindex {subindex}", self.id))
    }

    fn scalar_field(&self) -> Result<Field> {
        let kind = match self.datatype.as_str() {
            "IntegerT" => Kind::Signed,
            "UIntegerT" => Kind::Unsigned,
            "BooleanT" => Kind::Bool,
            "Float32T" => Kind::Float32,
            _ => anyhow::bail!("parameter is not a supported scalar"),
        };
        Ok(Field {
            name: self.id.clone(),
            kind,
            bits: u8::try_from(self.bits.context("IODD parameter width is unavailable")?)?,
            ..Field::default()
        })
    }

    pub fn decode_displays(&self, payload: &[u8]) -> Result<Vec<DisplayValue>> {
        // Validate the raw response width before any scaled interpretation.
        self.decode_raw(payload)?;
        self.displays
            .iter()
            .map(|display| {
                Ok(DisplayValue {
                    id: display.id.clone(),
                    value: display.field.decode(payload)?.value,
                    unit: display.field.unit.clone(),
                })
            })
            .collect()
    }

    /// Return the parameter's raw scalar or text value. Engineering display scaling is separate.
    pub fn decode_raw(&self, payload: &[u8]) -> Result<Option<String>> {
        if self.datatype == "StringT" {
            let length = usize::from(
                self.fixed_length
                    .context("IODD text length is unavailable")?,
            );
            ensure!(
                payload.len() <= length,
                "parameter response exceeds the IODD text length"
            );
            let encoding = self
                .encoding
                .as_deref()
                .context("IODD text encoding is unavailable")?;
            ensure!(
                matches!(encoding, "UTF-8" | "US-ASCII"),
                "unsupported parameter text encoding {encoding}"
            );
            ensure!(
                encoding != "US-ASCII" || payload.is_ascii(),
                "parameter response is not US-ASCII"
            );
            return Ok(Some(
                std::str::from_utf8(payload)?
                    .trim_end_matches('\0')
                    .to_owned(),
            ));
        }
        if !matches!(
            self.datatype.as_str(),
            "IntegerT" | "UIntegerT" | "BooleanT" | "Float32T"
        ) {
            return Ok(None);
        }
        let field = self.scalar_field()?;
        ensure!(
            payload.len() == usize::from(field.bits).div_ceil(8),
            "parameter response width differs from the IODD"
        );
        Ok(Some(field.decode(payload)?.raw))
    }
}

pub(super) fn parse<'a, 'i>(
    function: Node<'a, 'i>,
    types: &HashMap<&str, Node<'a, 'i>>,
    texts: &HashMap<&str, &str>,
    definitions: &[Parameter],
) -> Result<(Vec<Parameter>, Vec<String>)> {
    let mut parameters = Vec::new();
    let mut standard = Vec::new();
    let mut ids = HashSet::new();
    let mut indexes = HashSet::new();
    for variable in child(function, "VariableCollection")
        .into_iter()
        .flat_map(|node| node.children())
    {
        if variable.has_tag_name((IODD_NAMESPACE, "StdVariableRef")) {
            let id = variable
                .attribute("id")
                .context("standard parameter reference has no ID")?;
            ensure!(ids.insert(id), "duplicate parameter ID {id}");
            if let Some(definition) = definitions.iter().find(|parameter| parameter.id == id) {
                let parameter = standard::resolve(definition, variable, texts)?;
                ensure!(
                    indexes.insert(parameter.index),
                    "duplicate parameter index {}",
                    parameter.index
                );
                parameters.push(parameter);
            } else {
                standard.push(id.to_owned());
            }
        }
        if !variable.has_tag_name((IODD_NAMESPACE, "Variable")) {
            continue;
        }
        let id = variable.attribute("id").context("parameter has no ID")?;
        ensure!(ids.insert(id), "duplicate parameter ID {id}");
        let index = variable
            .attribute("index")
            .context("parameter has no index")?
            .parse()?;
        ensure!(indexes.insert(index), "duplicate parameter index {index}");
        let access = match variable.attribute("accessRights") {
            Some("ro") => Access::ReadOnly,
            Some("rw") => Access::ReadWrite,
            Some("wo") => Access::WriteOnly,
            _ => anyhow::bail!("parameter {id} has missing or invalid access rights"),
        };
        let parameter = parse_parameter(variable, id, index, access, types, texts, true)?;
        parameters.push(parameter);
    }
    display::apply(function, &mut parameters);
    for parameter in &parameters {
        parameter.validate()?;
    }
    Ok((parameters, standard))
}

fn parse_parameter<'a, 'i>(
    variable: Node<'a, 'i>,
    id: &str,
    index: u16,
    access: Access,
    types: &HashMap<&str, Node<'a, 'i>>,
    texts: &HashMap<&str, &str>,
    allow_record: bool,
) -> Result<Parameter> {
    let mut parameter = Parameter {
        id: id.into(),
        name: label(variable, texts, id),
        index,
        access,
        datatype: "unresolved".into(),
        bits: None,
        fixed_length: None,
        encoding: None,
        default_value: variable.attribute("defaultValue").map(str::to_owned),
        origin: None,
        record: None,
        displays: Vec::new(),
        choices: Vec::new(),
        ranges: Vec::new(),
        needs_input: Vec::new(),
    };
    match datatype(variable, types) {
        Ok(dt) => {
            parameter.datatype = dt
                .attribute((XSI, "type"))
                .unwrap_or("unresolved")
                .rsplit(':')
                .next()
                .unwrap_or("unresolved")
                .to_owned();
            parameter.bits = match parameter.datatype.as_str() {
                "BooleanT" => Some(1),
                "Float32T" => Some(32),
                _ => dt.attribute("bitLength").map(str::parse).transpose()?,
            };
            parameter.fixed_length = dt.attribute("fixedLength").map(str::parse).transpose()?;
            parameter.encoding = dt.attribute("encoding").map(str::to_owned);
            if matches!(parameter.datatype.as_str(), "IntegerT" | "UIntegerT")
                && !parameter.bits.is_some_and(|bits| (1..=64).contains(&bits))
            {
                parameter
                    .needs_input
                    .push("Numeric width requires manual interpretation of raw bytes".into());
            }
            if parameter.datatype == "StringT"
                && (parameter.fixed_length.is_none()
                    || !parameter
                        .encoding
                        .as_deref()
                        .is_some_and(|encoding| matches!(encoding, "UTF-8" | "US-ASCII")))
            {
                parameter.needs_input.push(
                    "Text length or encoding requires manual interpretation of raw bytes".into(),
                );
            }
            for value in dt.children() {
                if value.has_tag_name((IODD_NAMESPACE, "SingleValue")) {
                    let raw = value
                        .attribute("value")
                        .context("parameter choice has no value")?;
                    parameter.choices.push(Choice {
                        value: raw.into(),
                        name: label(value, texts, raw),
                    });
                }
                if value.has_tag_name((IODD_NAMESPACE, "ValueRange")) {
                    parameter.ranges.push(ValueRange {
                        lower: value
                            .attribute("lowerValue")
                            .context("parameter range has no lower bound")?
                            .into(),
                        upper: value
                            .attribute("upperValue")
                            .context("parameter range has no upper bound")?
                            .into(),
                    });
                }
            }
            if parameter.datatype == "RecordT" && allow_record {
                parameter.record = Some(record::parse(variable, dt, &parameter, types, texts)?);
            }
            if parameter.datatype == "RecordT" && !allow_record {
                parameter
                    .needs_input
                    .push("Nested record datatypes require manual interpretation".into());
            }
            if !matches!(
                parameter.datatype.as_str(),
                "IntegerT" | "UIntegerT" | "BooleanT" | "Float32T" | "StringT" | "RecordT"
            ) {
                parameter.needs_input.push(format!(
                    "{} is available as raw hexadecimal",
                    parameter.datatype
                ));
            }
        }
        Err(error) => parameter.needs_input.push(error.to_string()),
    }
    parameter.validate()?;
    Ok(parameter)
}
