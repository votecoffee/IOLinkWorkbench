use super::{IODD_NAMESPACE, child, label};
use anyhow::{Context, Result, ensure};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorTypeDefinition {
    /// Full ErrorType in wire order, including ErrorCode and AdditionalCode.
    pub code: u16,
    pub name: String,
    pub description: Option<String>,
    pub origin: Option<String>,
}

impl ErrorTypeDefinition {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.code >> 8, 0x80 | 0x81),
            "IODD error type 0x{:04X} must use ErrorCode 0x80 or 0x81",
            self.code
        );
        ensure!(
            !self.name.trim().is_empty(),
            "IODD error type name is empty"
        );
        Ok(())
    }
}

pub(super) fn parse(
    parent: Node<'_, '_>,
    texts: &HashMap<&str, &str>,
    standard: bool,
) -> Result<(Vec<ErrorTypeDefinition>, Vec<u16>)> {
    let mut definitions = Vec::new();
    let mut references = Vec::new();
    let mut codes = HashSet::new();
    for entry in child(parent, "ErrorTypeCollection")
        .into_iter()
        .flat_map(|node| node.children())
    {
        let reference = entry.has_tag_name((IODD_NAMESPACE, "StdErrorTypeRef"));
        if !reference && !entry.has_tag_name((IODD_NAMESPACE, "ErrorType")) {
            continue;
        }
        let expected = if reference || standard { 0x80 } else { 0x81 };
        let high = entry
            .attribute("code")
            .map(|code| code.trim().parse::<u8>())
            .transpose()
            .context("invalid ErrorType code byte")?
            .unwrap_or(expected);
        ensure!(
            high == expected,
            "IODD error type must use ErrorCode 0x{expected:02X}"
        );
        let additional = entry
            .attribute("additionalCode")
            .context("ErrorType has no additionalCode")?
            .trim()
            .parse()
            .context("invalid ErrorType additionalCode byte")?;
        let code = u16::from_be_bytes([high, additional]);
        ensure!(codes.insert(code), "duplicate IODD error type 0x{code:04X}");
        if reference {
            references.push(code);
            continue;
        }
        definitions.push(ErrorTypeDefinition {
            code,
            name: label(entry, texts, &format!("ErrorType 0x{code:04X}")),
            description: child(entry, "Description")
                .and_then(|node| node.attribute("textId"))
                .map(|id| texts.get(id).copied().unwrap_or(id).to_owned()),
            origin: None,
        });
    }
    Ok((definitions, references))
}
