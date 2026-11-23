//! Capabilities declared by the device description.
use super::IODD_NAMESPACE;
use anyhow::{Context, Result, bail, ensure};
use roxmltree::Node;
use serde::{Deserialize, Serialize};

const MAX_PROFILE_IDS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Features {
    pub block_parameter: bool,
    pub data_storage: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_characteristic: Option<Vec<u16>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_locks: Option<AccessLocks>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessLocks {
    pub parameter: bool,
    pub data_storage: bool,
    pub local_parameterization: bool,
    pub local_user_interface: bool,
}

impl Features {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.profile_characteristic
                .as_ref()
                .is_none_or(|ids| (1..=MAX_PROFILE_IDS).contains(&ids.len())),
            "IODD profileCharacteristic requires 1–{MAX_PROFILE_IDS} profile identifiers"
        );
        Ok(())
    }

    pub fn summary(&self) -> String {
        let profile_ids = match &self.profile_characteristic {
            Some(ids) => ids
                .iter()
                .map(|id| format!("0x{id:04X} ({id})"))
                .collect::<Vec<_>>()
                .join(", "),
            None => "Undeclared".into(),
        };
        let access_locks = match &self.access_locks {
            Some(locks) => format!(
                "Parameter {} · Data Storage {} · Local parameterization {} · Local user interface {}",
                yes_no(locks.parameter),
                yes_no(locks.data_storage),
                yes_no(locks.local_parameterization),
                yes_no(locks.local_user_interface)
            ),
            None => "Undeclared".into(),
        };
        format!(
            "Block parameter transfer · {}\nDevice Data Storage · {}\nProfile IDs · {profile_ids}\nAccess lock support · {access_locks}",
            yes_no(self.block_parameter),
            yes_no(self.data_storage)
        )
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "Yes" } else { "No" }
}

fn unique<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Result<Option<Node<'a, 'i>>> {
    let mut children = node
        .children()
        .filter(|child| child.has_tag_name((IODD_NAMESPACE, name)));
    let first = children.next();
    ensure!(children.next().is_none(), "duplicate IODD {name} element");
    Ok(first)
}

fn flag(node: Node<'_, '_>, name: &str) -> Result<bool> {
    match node.attribute(name).map(str::trim) {
        Some("true" | "1") => Ok(true),
        Some("false" | "0") => Ok(false),
        _ => bail!(
            "missing or invalid IODD {}/{name} boolean",
            node.tag_name().name()
        ),
    }
}

pub(super) fn parse(function: Node<'_, '_>) -> Result<Option<Features>> {
    let Some(node) = unique(function, "Features")? else {
        return Ok(None);
    };
    let profile_characteristic = node
        .attribute("profileCharacteristic")
        .map(|text| {
            text.split_ascii_whitespace()
                .take(MAX_PROFILE_IDS + 1)
                .map(|token| {
                    token
                        .parse()
                        .context("invalid IODD profileCharacteristic identifier")
                })
                .collect::<Result<Vec<u16>>>()
        })
        .transpose()?;
    let access_locks = unique(node, "SupportedAccessLocks")?
        .map(|locks| {
            Ok::<_, anyhow::Error>(AccessLocks {
                parameter: flag(locks, "parameter")?,
                data_storage: flag(locks, "dataStorage")?,
                local_parameterization: flag(locks, "localParameterization")?,
                local_user_interface: flag(locks, "localUserInterface")?,
            })
        })
        .transpose()?;
    let features = Features {
        block_parameter: flag(node, "blockParameter")?,
        data_storage: flag(node, "dataStorage")?,
        profile_characteristic,
        access_locks,
    };
    features.validate()?;
    Ok(Some(features))
}
