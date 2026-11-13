use super::{Access, Parameter, parse_parameter};
use crate::iodd::IODD_NAMESPACE;
use anyhow::{Context, Result, ensure};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub subindex_access: bool,
    pub items: Vec<RecordItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordItem {
    pub subindex: u8,
    pub bit_offset: u16,
    pub parameter: Parameter,
}

impl Record {
    pub(super) fn validate(&self, parent: &Parameter) -> Result<()> {
        ensure!(
            parent.datatype == "RecordT",
            "record metadata requires RecordT"
        );
        let bits = parent.bits.context("record width is unavailable")?;
        ensure!(
            (1..=1856).contains(&bits),
            "record width must be 1–1856 bits"
        );
        ensure!(!self.items.is_empty(), "record has no items");
        let mut subindexes = HashSet::new();
        let mut used = HashSet::new();
        for item in &self.items {
            ensure!(
                item.subindex > 0 && subindexes.insert(item.subindex),
                "duplicate or zero record subindex"
            );
            ensure!(
                item.parameter.index == parent.index,
                "record member index differs from parent"
            );
            ensure!(
                item.parameter.record.is_none(),
                "nested parameter records are unsupported"
            );
            restrict(parent.access, Some(item.parameter.access))?;
            item.parameter.validate()?;
            ensure!(
                item.bit_offset < bits,
                "record item extends outside the record"
            );
            if let Some(width) = item.width() {
                let end = u32::from(item.bit_offset) + u32::from(width);
                ensure!(
                    width > 0 && end <= u32::from(bits),
                    "record item extends outside the record"
                );
                for bit in u32::from(item.bit_offset)..end {
                    ensure!(used.insert(bit), "record items overlap");
                }
            }
        }
        Ok(())
    }
}

impl RecordItem {
    pub fn width(&self) -> Option<u16> {
        match self.parameter.datatype.as_str() {
            "StringT" | "OctetStringT" => self.parameter.fixed_length?.checked_mul(8),
            _ => self.parameter.bits,
        }
    }

    /// Right-justify the member, preserving IO-Link's big-endian bit numbering.
    pub(super) fn extract(&self, payload: &[u8]) -> Result<Vec<u8>> {
        let width = usize::from(self.width().context("record member width is unavailable")?);
        let offset = usize::from(self.bit_offset);
        ensure!(
            width > 0 && offset + width <= payload.len() * 8,
            "record member exceeds response width"
        );
        let mut bytes = vec![0; width.div_ceil(8)];
        let length = bytes.len();
        for bit in 0..width {
            let absolute = offset + bit;
            let value = (payload[payload.len() - 1 - absolute / 8] >> (absolute % 8)) & 1;
            bytes[length - 1 - bit / 8] |= value << (bit % 8);
        }
        Ok(bytes)
    }
}

pub(super) fn restrict(parent: Access, restriction: Option<Access>) -> Result<Access> {
    let access = restriction.unwrap_or(parent);
    ensure!(
        parent == Access::ReadWrite || access == parent,
        "record item increases parent access rights"
    );
    Ok(access)
}

pub(super) fn parse<'a, 'i>(
    owner: Node<'a, 'i>,
    dt: Node<'a, 'i>,
    parent: &Parameter,
    types: &HashMap<&str, Node<'a, 'i>>,
    texts: &HashMap<&str, &str>,
) -> Result<Record> {
    let subindex_access = match dt.attribute("subindexAccessSupported").unwrap_or("true") {
        "true" | "1" => true,
        "false" | "0" => false,
        _ => anyhow::bail!("invalid record subindexAccessSupported"),
    };
    let mut info = HashMap::new();
    for item in owner
        .children()
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "RecordItemInfo")))
    {
        let subindex: u8 = item
            .attribute("subindex")
            .context("record item info has no subindex")?
            .parse()?;
        ensure!(
            info.insert(subindex, item).is_none(),
            "duplicate record item info subindex"
        );
    }
    let mut items = Vec::new();
    for item in dt
        .children()
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "RecordItem")))
    {
        let subindex = item
            .attribute("subindex")
            .context("record item has no subindex")?
            .parse()?;
        let access = match item.attribute("accessRightRestriction") {
            None => None,
            Some("ro") => Some(Access::ReadOnly),
            Some("rw") => Some(Access::ReadWrite),
            Some("wo") => Some(Access::WriteOnly),
            _ => anyhow::bail!("invalid record access restriction"),
        };
        let mut parameter = parse_parameter(
            item,
            &format!("{}/{subindex}", parent.id),
            parent.index,
            restrict(parent.access, access)?,
            types,
            texts,
            false,
        )?;
        if let Some(default) = info
            .remove(&subindex)
            .and_then(|item| item.attribute("defaultValue"))
        {
            parameter.default_value = Some(default.into());
        }
        let mut member = RecordItem {
            subindex,
            bit_offset: item
                .attribute("bitOffset")
                .context("record item has no bit offset")?
                .parse()?,
            parameter,
        };
        if member.width().is_none() {
            member.parameter.needs_input.push(
                "Member width is unresolved; complete overlap verification is unavailable".into(),
            );
        }
        items.push(member);
    }
    ensure!(
        info.is_empty(),
        "record item info references an undeclared subindex"
    );
    let record = Record {
        subindex_access,
        items,
    };
    record.validate(parent)?;
    Ok(record)
}
