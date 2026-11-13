use super::{Choice, Parameter, ValueRange};
use crate::iodd::{IODD_NAMESPACE, child};
use anyhow::{Context, Result, ensure};
use roxmltree::{Document, Node};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

pub(in crate::iodd) fn load(path: &Path) -> Result<crate::iodd::Definitions> {
    let xml = crate::iodd::read_xml(path)?;
    parse(
        &xml,
        &path
            .file_name()
            .context("missing definitions filename")?
            .to_string_lossy(),
    )
}

pub(super) fn parse(xml: &str, source: &str) -> Result<crate::iodd::Definitions> {
    let doc = Document::parse(xml)?;
    let root = doc.root_element();
    ensure!(
        root.has_tag_name((IODD_NAMESPACE, "IODDStandardDefinitions")),
        "expected IODD 1.1 standard definitions namespace"
    );
    let mut types = HashMap::new();
    for dt in child(root, "DatatypeCollection")
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "Datatype")))
    {
        let id = dt.attribute("id").context("standard datatype has no ID")?;
        ensure!(
            types.insert(id, dt).is_none(),
            "duplicate standard datatype ID {id}"
        );
    }
    let texts = child(root, "ExternalTextCollection")
        .and_then(|node| child(node, "PrimaryLanguage"))
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "Text")))
        .filter_map(|node| Some((node.attribute("id")?, node.attribute("value")?)))
        .collect();
    let (mut parameters, unresolved) = super::parse(
        root,
        &types,
        &texts,
        &[],
        &crate::iodd::resources::Units::default(),
    )?;
    ensure!(
        unresolved.is_empty(),
        "standard definitions contain unresolved references"
    );
    let version = child(root, "DocumentInfo")
        .and_then(|node| node.attribute("version"))
        .unwrap_or("unspecified version");
    for parameter in &mut parameters {
        parameter.origin = Some(format!("{source} · {version}"));
        if let Some(record) = &mut parameter.record {
            for item in &mut record.items {
                item.parameter.origin = parameter.origin.clone();
            }
        }
        if let Some(array) = &mut parameter.array {
            array.element.origin = parameter.origin.clone();
        }
    }
    let (mut events, references) = crate::iodd::events::parse(root, &texts)?;
    ensure!(
        references.is_empty(),
        "standard definitions contain unresolved event references"
    );
    for event in &mut events {
        event.origin = Some(format!("{source} · {version}"));
    }
    Ok(crate::iodd::Definitions { parameters, events })
}

pub(super) fn resolve(
    definition: &Parameter,
    reference: Node<'_, '_>,
    texts: &HashMap<&str, &str>,
) -> Result<Parameter> {
    let mut parameter = definition.clone();
    if let Some(value) = reference.attribute("defaultValue") {
        parameter.default_value = Some(value.into());
        if let Some(array) = &mut parameter.array {
            array.element.default_value = Some(value.into());
        }
    }
    if let Some(length) = reference.attribute("fixedLengthRestriction") {
        let length: u16 = length.parse()?;
        if matches!(parameter.datatype.as_str(), "StringT" | "OctetStringT") {
            ensure!(
                length > 0
                    && parameter
                        .fixed_length
                        .is_some_and(|maximum| length <= maximum),
                "invalid standard parameter length restriction"
            );
            ensure!(
                parameter.id != "V_ApplicationSpecificTag" || length >= 16,
                "application tag restriction must be at least 16"
            );
            parameter.fixed_length = Some(length);
        } else if let Some(array) = &mut parameter.array {
            parameter.bits = Some(array.restrict_count(u8::try_from(length)?)?);
        } else {
            parameter
                .needs_input
                .push("Complex standard length restriction requires raw interpretation".into());
        }
    }
    let mut choices = Vec::new();
    let mut ranges = Vec::new();
    let mut subindexes = HashSet::new();
    for value in reference.children() {
        if value.has_tag_name((IODD_NAMESPACE, "StdSingleValueRef")) {
            let raw = value
                .attribute("value")
                .context("standard choice reference has no value")?;
            choices.push(
                definition
                    .choices
                    .iter()
                    .find(|choice| choice.value == raw)
                    .with_context(|| format!("unknown standard choice {raw}"))?
                    .clone(),
            );
        }
        if value.has_tag_name((IODD_NAMESPACE, "SingleValue")) {
            let raw = value
                .attribute("value")
                .context("parameter choice has no value")?;
            ensure!(
                !definition.choices.iter().any(|choice| choice.value == raw),
                "standard choice cannot be redefined"
            );
            choices.push(Choice {
                value: raw.into(),
                name: crate::iodd::label(value, texts, raw),
            });
        }
        if value.has_tag_name((IODD_NAMESPACE, "ValueRange")) {
            ranges.push(ValueRange {
                lower: value
                    .attribute("lowerValue")
                    .context("missing lower value")?
                    .into(),
                upper: value
                    .attribute("upperValue")
                    .context("missing upper value")?
                    .into(),
            });
        }
        if value.has_tag_name((IODD_NAMESPACE, "StdRecordItemRef")) {
            let subindex: u8 = value
                .attribute("subindex")
                .context("standard record reference has no subindex")?
                .parse()?;
            ensure!(
                subindexes.insert(subindex),
                "duplicate standard record item reference"
            );
            let item = parameter
                .record
                .as_mut()
                .and_then(|record| {
                    record
                        .items
                        .iter_mut()
                        .find(|item| item.subindex == subindex)
                })
                .context("standard record reference has no declared member")?;
            item.parameter = resolve(&item.parameter, value, texts)?;
        }
    }
    if !choices.is_empty() || !ranges.is_empty() || parameter.id == "V_SystemCommand" {
        parameter.choices = choices;
        parameter.ranges = ranges;
    }
    parameter.validate()?;
    Ok(parameter)
}
