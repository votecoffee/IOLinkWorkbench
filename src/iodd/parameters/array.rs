use super::{Parameter, parse_parameter};
use crate::iodd::{array::Geometry, child};
use anyhow::{Context, Result, ensure};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Array {
    pub count: u8,
    pub subindex_access: bool,
    pub element: Box<Parameter>,
}

impl Array {
    pub(super) fn geometry(&self) -> Result<Geometry> {
        Geometry::new(
            self.element
                .width()
                .context("array element width is unavailable")?,
            self.count,
        )
    }

    pub(super) fn validate(&self, parent: &Parameter) -> Result<()> {
        ensure!(
            parent.datatype == "ArrayT" && parent.record.is_none(),
            "array metadata requires ArrayT"
        );
        ensure!(
            self.element.array.is_none() && self.element.record.is_none(),
            "nested array layouts require manual interpretation"
        );
        ensure!(
            self.element.index == parent.index && self.element.access == parent.access,
            "array element identity or access differs from parent"
        );
        let geometry = self.geometry()?;
        ensure!(
            parent.bits.map(u32::from) == Some(geometry.bits()),
            "array width differs from its element layout"
        );
        self.element.validate()
    }

    pub(super) fn restrict_count(&mut self, count: u8) -> Result<u16> {
        ensure!(
            count > 0 && count <= self.count,
            "invalid standard array length restriction"
        );
        self.count = count;
        Ok(self.geometry()?.bits() as u16)
    }
}

pub(super) fn parse<'a, 'i>(
    dt: Node<'a, 'i>,
    parent: &Parameter,
    types: &HashMap<&str, Node<'a, 'i>>,
    texts: &HashMap<&str, &str>,
) -> Result<Array> {
    let count: u8 = dt
        .attribute("count")
        .context("array has no count")?
        .parse()?;
    ensure!(count > 0, "array count must be 1–255");
    let subindex_access = match dt.attribute("subindexAccessSupported").unwrap_or("true") {
        "true" | "1" => true,
        "false" | "0" => false,
        _ => anyhow::bail!("invalid array subindexAccessSupported"),
    };
    let mut element = parse_parameter(
        dt,
        &format!("{}/element", parent.id),
        parent.index,
        parent.access,
        types,
        texts,
        false,
    )?;
    element.name = format!("{} element", parent.name);
    element.default_value = parent.default_value.clone();
    // An ArrayT contains one simple datatype, not a record's per-member defaults.
    ensure!(
        child(dt, "RecordItem").is_none(),
        "array cannot contain record items"
    );
    let array = Array {
        count,
        subindex_access,
        element: Box::new(element),
    };
    array.geometry()?;
    Ok(array)
}
