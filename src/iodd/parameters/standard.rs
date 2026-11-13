use super::{Choice, Parameter, ValueRange};
use crate::iodd::{IODD_NAMESPACE, child};
use anyhow::{Context, Result, ensure};
use roxmltree::{Document, Node};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

pub(in crate::iodd) fn load(
    path: &Path,
    language: Option<&str>,
    language_file: Option<&Path>,
) -> Result<crate::iodd::Definitions> {
    let xml = crate::iodd::read_xml(path)?;
    let companion = crate::iodd::resources::local_companion(path, language, language_file)?;
    parse_localized(
        &xml,
        &path
            .file_name()
            .context("missing definitions filename")?
            .to_string_lossy(),
        language,
        companion.as_ref(),
    )
}

fn parse_localized(
    xml: &str,
    source: &str,
    requested: Option<&str>,
    companion: Option<&crate::iodd::resources::Companion>,
) -> Result<crate::iodd::Definitions> {
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
    let units = crate::iodd::resources::Units::default();
    let primary = crate::iodd::resources::Texts::load(root, None, None, &[], &units)?;
    let requested = requested
        .map(crate::iodd::resources::language_tag)
        .transpose()?;
    let available = requested
        .as_ref()
        .is_some_and(|tag| primary.metadata.available_languages.contains(tag));
    let selected_tag = requested
        .as_deref()
        .filter(|_| available || companion.is_some());
    let selected = crate::iodd::resources::Texts::load(root, selected_tag, companion, &[], &units)
        .context("standard definitions language")?;
    for id in selected.values.keys() {
        ensure!(
            primary.values.contains_key(id),
            "standard translation contains unknown text ID {id}"
        );
    }
    if let Some(companion) = companion {
        let external = Document::parse(&companion.xml)?;
        if let (Some(a), Some(b)) = (
            child(root, "DocumentInfo").and_then(|node| node.attribute("version")),
            child(external.root_element(), "DocumentInfo")
                .and_then(|node| node.attribute("version")),
        ) {
            ensure!(a == b, "standard language version differs from definitions");
        }
    }
    let mut review = selected.metadata.review.clone();
    let mut fallback = selected.metadata.fallback_text_ids.clone();
    if requested.is_some() && requested != selected.metadata.selected_language {
        review.push(format!(
            "requested language {} is unavailable; using primary language {}",
            requested.as_deref().unwrap(),
            selected
                .metadata
                .primary_language
                .as_deref()
                .unwrap_or("unspecified")
        ));
        fallback = primary.values.keys().cloned().collect();
        fallback.sort();
    }
    let texts = selected
        .values
        .iter()
        .map(|(id, value)| (id.as_str(), value.as_str()))
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
    Ok(crate::iodd::Definitions {
        parameters,
        events,
        localization: Some(crate::iodd::StandardLocalization {
            source: source.into(),
            version: version.into(),
            primary_language: selected.metadata.primary_language,
            requested_language: requested,
            selected_language: selected.metadata.selected_language,
            language_source: selected.metadata.language_source,
            fallback_text_ids: fallback,
            review,
        }),
    })
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
        if value.is_element() && value.tag_name().namespace() == Some(IODD_NAMESPACE) {
            ensure!(
                matches!(
                    value.tag_name().name(),
                    "StdSingleValueRef" | "SingleValue" | "ValueRange" | "StdRecordItemRef"
                ),
                "unsupported standard-reference directive {}",
                value.tag_name().name()
            );
        }
        if value.has_tag_name((IODD_NAMESPACE, "StdSingleValueRef")) {
            let raw = value
                .attribute("value")
                .context("standard choice reference has no value")?;
            let mut matching = definition
                .choices
                .iter()
                .filter(|choice| super::assessment::equivalent(definition, &choice.value, raw));
            let choice = matching
                .next()
                .with_context(|| format!("unknown standard choice {raw}"))?;
            ensure!(matching.next().is_none(), "ambiguous standard choice {raw}");
            choices.push(choice.clone());
        }
        if value.has_tag_name((IODD_NAMESPACE, "SingleValue")) {
            let raw = value
                .attribute("value")
                .context("parameter choice has no value")?;
            ensure!(
                !definition.choices.iter().any(|choice| {
                    super::assessment::equivalent(definition, &choice.value, raw)
                }),
                "standard choice cannot be redefined"
            );
            choices.push(Choice {
                value: raw.into(),
                name: crate::iodd::label(value, texts, raw),
            });
        }
        if value.has_tag_name((IODD_NAMESPACE, "ValueRange")) {
            let range = ValueRange {
                name: child(value, "Name").map(|_| crate::iodd::label(value, texts, "")),
                lower: value
                    .attribute("lowerValue")
                    .context("missing lower value")?
                    .into(),
                upper: value
                    .attribute("upperValue")
                    .context("missing upper value")?
                    .into(),
            };
            for choice in &definition.choices {
                ensure!(
                    !super::assessment::range_contains(definition, &range, &choice.value)?,
                    "vendor range cannot redefine standard choice {}",
                    choice.value
                );
            }
            ranges.push(range);
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
        super::assessment::validate_values(&parameter).with_context(|| {
            format!("standard parameter {} allowed-value override", parameter.id)
        })?;
    }
    parameter.validate()?;
    Ok(parameter)
}

#[cfg(test)]
mod tests;
