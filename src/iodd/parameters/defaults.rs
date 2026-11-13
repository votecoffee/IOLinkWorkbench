//! Declared offline defaults: inspect saved metadata without transport access.
use super::{Access, AssessmentStatus, DecodedParameter, Parameter, assessment};
use crate::{decode::hex, profile::Profile};
use anyhow::{Context, Result, ensure};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    NotDeclared,
    Decoded,
    NeedsReview,
    Unsupported,
}

#[derive(Debug, Serialize)]
pub struct Value {
    pub status: Status,
    pub declared: Option<String>,
    pub representation_datatype: String,
    pub representation_scope: &'static str,
    pub array_elements: Option<u8>,
    pub representation_hex: Option<String>,
    pub decoded: Option<DecodedParameter>,
    pub review: Vec<String>,
}

impl Value {
    pub fn needs_review(&self) -> bool {
        matches!(self.status, Status::NeedsReview | Status::Unsupported)
    }

    pub fn text(&self) -> String {
        let mut text = format!(
            "IODD declared default (offline) · {:?}\nDeclaration {:?}\n{} · {}",
            self.status, self.declared, self.representation_scope, self.representation_datatype
        );
        if let Some(count) = self.array_elements {
            text.push_str(&format!(" · applies independently to {count} elements"));
        }
        if let Some(bytes) = &self.representation_hex {
            text.push_str(&format!("\nGenerated local representation {bytes}"));
        }
        if let Some(decoded) = &self.decoded {
            text.push_str(&format!("\n{}", decoded.text()));
        }
        for note in &self.review {
            text.push_str(&format!("\nReview · {note}"));
        }
        text.push_str(
            "\nThis is saved default metadata, not a device reading or a write operation.",
        );
        text
    }
}

fn representation(parameter: &Parameter, declared: &str) -> Result<Vec<u8>> {
    match parameter.datatype.as_str() {
        "IntegerT" | "UIntegerT" | "BooleanT" | "Float32T" => {
            assessment::default_numeric_bytes(parameter, declared)
        }
        "StringT" | "OctetStringT" => {
            let length = usize::from(
                parameter
                    .fixed_length
                    .context("declared string length is unavailable")?,
            );
            ensure!(
                (1..=232).contains(&length),
                "declared string length must be 1–232 octets"
            );
            if parameter.datatype == "StringT" {
                ensure!(
                    matches!(parameter.encoding.as_deref(), Some("UTF-8" | "US-ASCII")),
                    "default text encoding is unavailable or unsupported"
                );
                ensure!(
                    parameter.encoding.as_deref() != Some("US-ASCII") || declared.is_ascii(),
                    "default text is not US-ASCII"
                );
                ensure!(!declared.contains('\0'), "default text contains NUL");
                ensure!(
                    declared.len() <= length,
                    "default text exceeds the declared octet length"
                );
                let mut bytes = declared.as_bytes().to_vec();
                bytes.resize(length, 0);
                Ok(bytes)
            } else {
                ensure!(
                    declared.len() < 5 * length,
                    "default octet sequence exceeds the declared length"
                );
                let bytes = declared
                    .split(',')
                    .map(|part| -> Result<u8> {
                        let digits = part
                            .strip_prefix("0x")
                            .context("octet default requires comma-separated 0xNN entries")?;
                        ensure!(
                            digits.len() == 2
                                && digits.bytes().all(|byte| byte.is_ascii_hexdigit()),
                            "invalid default octet"
                        );
                        Ok(u8::from_str_radix(digits, 16)?)
                    })
                    .collect::<Result<Vec<_>>>()?;
                ensure!(
                    bytes.len() == length,
                    "default octet count differs from the declared length"
                );
                Ok(bytes)
            }
        }
        _ => anyhow::bail!("default representation is unsupported"),
    }
}

fn inspect(parameter: &Parameter, declared: Option<&str>) -> Value {
    let mut result = Value {
        status: Status::NotDeclared,
        declared: declared.map(str::to_owned),
        representation_datatype: parameter.datatype.clone(),
        representation_scope: "one singular value; generated locally",
        array_elements: None,
        representation_hex: None,
        decoded: None,
        review: Vec::new(),
    };
    let Some(declared) = declared else {
        return result;
    };
    if !matches!(
        parameter.datatype.as_str(),
        "IntegerT" | "UIntegerT" | "BooleanT" | "Float32T" | "StringT" | "OctetStringT"
    ) {
        result.status = Status::Unsupported;
        result.review.push("Declared default retained; temporal, whole-record and unresolved complex representations require manual interpretation".into());
        return result;
    }
    if parameter.datatype == "Float32T" && declared.trim() == "NaN" {
        result.status = Status::Unsupported;
        result
            .review
            .push("NaN default interpretation is unsupported; declaration retained".into());
        return result;
    }
    match representation(parameter, declared) {
        Ok(bytes) => {
            result.representation_hex = Some(hex(&bytes));
            let decoded = parameter.decode(&bytes);
            if decoded.needs_review() {
                result
                    .review
                    .push("Generated representation needs decoding review".into());
            }
            if decoded
                .value_assessment
                .as_ref()
                .is_some_and(|value| value.status != AssessmentStatus::Allowed)
            {
                result.review.push(
                    "Declared default needs review against the saved value definitions".into(),
                );
            }
            result.status = if result.review.is_empty() {
                Status::Decoded
            } else {
                Status::NeedsReview
            };
            result.decoded = Some(decoded);
        }
        Err(error) => {
            result.status = Status::NeedsReview;
            result.review.push(error.to_string());
        }
    }
    result
}

impl Parameter {
    /// Interpret a default using saved metadata only, including write-only parameters.
    pub fn inspect_default(&self, subindex: u8) -> Result<Value> {
        let target = self.target(subindex)?;
        if let Some(array) = &self.array {
            let declared = self
                .default_value
                .as_deref()
                .or(array.element.default_value.as_deref());
            let mut value = inspect(&array.element, declared);
            value.representation_scope =
                "one array element in singular form; no whole-array bytes constructed";
            value.array_elements = (subindex == 0).then_some(array.count);
            if let Some(element) = &array.element.default_value {
                let consistent = self.default_value.as_deref().is_some_and(|parent| {
                    parent == element || assessment::equivalent(&array.element, parent, element)
                        || matches!((representation(&array.element, parent), representation(&array.element, element)), (Ok(a), Ok(b)) if a == b)
                });
                if !consistent {
                    value.review.push(format!("Array parent and saved element default differ or the parent is missing; saved element default {element:?}"));
                    if value.status != Status::Unsupported {
                        value.status = Status::NeedsReview;
                    }
                }
            }
            return Ok(value);
        }
        let mut value = inspect(target, target.default_value.as_deref());
        if target.record.is_some() {
            value.representation_scope =
                "record defaults are inspected by member; no whole-record bytes constructed";
        }
        Ok(value)
    }
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub parameter_id: String,
    pub name: String,
    pub index: u16,
    pub subindex: u8,
    pub access: Access,
    pub datatype: String,
    pub bit_offset: Option<u16>,
    pub value: Value,
}

#[derive(Debug, Serialize)]
pub struct Inspection {
    pub basis: &'static str,
    pub unresolved_standard_parameter_refs: Vec<String>,
    pub decoded: usize,
    pub not_declared: usize,
    pub needs_review: usize,
    pub unsupported: usize,
    pub entries: Vec<Entry>,
}

fn entry(parameter: &Parameter, subindex: u8) -> Result<Entry> {
    let target = parameter.target(subindex)?;
    let bit_offset = if subindex == 0 {
        None
    } else if let Some(record) = &parameter.record {
        record
            .items
            .iter()
            .find(|item| item.subindex == subindex)
            .map(|item| item.bit_offset)
    } else if let Some(array) = &parameter.array {
        Some(array.geometry()?.offset(subindex))
    } else {
        None
    };
    Ok(Entry {
        parameter_id: parameter.id.clone(),
        name: target.name.clone(),
        index: parameter.index,
        subindex,
        access: target.access,
        datatype: target.datatype.clone(),
        bit_offset,
        value: parameter.inspect_default(subindex)?,
    })
}

pub fn inspect_profile(
    profile: &Profile,
    id: Option<&str>,
    subindex: Option<u8>,
) -> Result<Inspection> {
    ensure!(
        subindex.is_none() || id.is_some(),
        "a default subindex requires a parameter ID"
    );
    let parameters: Vec<_> = if let Some(id) = id {
        vec![profile.parameter(id)?]
    } else {
        profile
            .iodd
            .as_ref()
            .context("import an IODD to inspect declared defaults")?
            .parameters
            .iter()
            .collect()
    };
    let mut entries = Vec::new();
    for parameter in parameters {
        if let Some(subindex) = subindex {
            entries.push(entry(parameter, subindex)?);
        } else if let Some(record) = &parameter.record {
            if parameter.default_value.is_some() {
                entries.push(entry(parameter, 0)?);
            }
            for item in &record.items {
                entries.push(entry(parameter, item.subindex)?);
            }
        } else {
            entries.push(entry(parameter, 0)?);
        }
    }
    Ok(Inspection {
        basis: "saved IODD-declared offline defaults; local representations only; no gateway access or writes",
        unresolved_standard_parameter_refs: if id.is_none() {
            profile
                .iodd
                .as_ref()
                .map(|iodd| iodd.standard_parameter_refs.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        },
        decoded: entries
            .iter()
            .filter(|entry| entry.value.status == Status::Decoded)
            .count(),
        not_declared: entries
            .iter()
            .filter(|entry| entry.value.status == Status::NotDeclared)
            .count(),
        needs_review: entries
            .iter()
            .filter(|entry| entry.value.status == Status::NeedsReview)
            .count(),
        unsupported: entries
            .iter()
            .filter(|entry| entry.value.status == Status::Unsupported)
            .count(),
        entries,
    })
}

#[cfg(test)]
mod tests;
