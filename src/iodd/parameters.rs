use super::{IODD_NAMESPACE, XSI, child, datatype, label};
use crate::decode::{Field, Kind};
use anyhow::{Context, Result, ensure};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
        Ok(())
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
        let kind = match self.datatype.as_str() {
            "IntegerT" => Kind::Signed,
            "UIntegerT" => Kind::Unsigned,
            "BooleanT" => Kind::Bool,
            "Float32T" => Kind::Float32,
            _ => return Ok(None),
        };
        let bits = u8::try_from(self.bits.context("IODD parameter width is unavailable")?)?;
        ensure!(
            payload.len() == usize::from(bits).div_ceil(8),
            "parameter response width differs from the IODD"
        );
        let field = Field {
            name: self.id.clone(),
            kind,
            bits,
            ..Field::default()
        };
        Ok(Some(field.decode(payload)?.raw))
    }
}

pub(super) fn parse<'a, 'i>(
    function: Node<'a, 'i>,
    types: &HashMap<&str, Node<'a, 'i>>,
    texts: &HashMap<&str, &str>,
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
            standard.push(id.to_owned());
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
                        "Text length or encoding requires manual interpretation of raw bytes"
                            .into(),
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
                if !matches!(
                    parameter.datatype.as_str(),
                    "IntegerT" | "UIntegerT" | "BooleanT" | "Float32T" | "StringT"
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
        parameters.push(parameter);
    }
    Ok((parameters, standard))
}
