use super::{IODD_NAMESPACE, child};
use anyhow::{Context, Result, ensure};
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

const XML: &str = "http://www.w3.org/XML/1998/namespace";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Localization {
    pub primary_language: Option<String>,
    pub selected_language: Option<String>,
    pub available_languages: Vec<String>,
    pub language_source: Option<String>,
    pub unit_source: Option<String>,
    pub unit_version: Option<String>,
    pub fallback_text_ids: Vec<String>,
    pub review: Vec<String>,
}

impl Localization {
    pub fn summary(&self) -> String {
        format!(
            "Text language {} · {} primary-language fallbacks · units {}",
            self.selected_language.as_deref().unwrap_or("unspecified"),
            self.fallback_text_ids.len(),
            self.unit_source
                .as_deref()
                .unwrap_or("built-in common codes")
        )
    }
}

pub(super) fn language_tag(value: &str) -> Result<String> {
    ensure!(
        value.len() <= 35
            && !value.is_empty()
            && value
                .split('-')
                .enumerate()
                .all(|(i, part)| !part.is_empty()
                    && part.len() <= 8
                    && part.bytes().all(|b| if i == 0 {
                        b.is_ascii_alphabetic()
                    } else {
                        b.is_ascii_alphanumeric()
                    })),
        "invalid language tag"
    );
    Ok(value.to_ascii_lowercase())
}

pub(super) struct Companion {
    pub xml: String,
    pub source: String,
}

pub(super) struct Texts {
    pub values: HashMap<String, String>,
    pub metadata: Localization,
}

fn text_table(node: Node<'_, '_>) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    for text in node.children().filter(|node| {
        node.has_tag_name((IODD_NAMESPACE, "Text"))
            || node.has_tag_name((IODD_NAMESPACE, "TextRedefine"))
    }) {
        let id = text.attribute("id").context("text has no ID")?;
        if text.has_tag_name((IODD_NAMESPACE, "TextRedefine")) {
            ensure!(
                id.strip_prefix("STD_TN_DeviceSpecific_")
                    .and_then(|id| id.parse::<u8>().ok())
                    .is_some_and(|n| (1..=16).contains(&n)),
                "unsupported standard text redefinition {id}"
            );
        }
        let value = text.attribute("value").context("text has no value")?;
        ensure!(
            values.insert(id.into(), value.into()).is_none(),
            "duplicate text ID {id}"
        );
    }
    Ok(values)
}

impl Texts {
    pub fn load(
        root: Node<'_, '_>,
        requested: Option<&str>,
        companion: Option<&Companion>,
        discovered: &[String],
        units: &Units,
    ) -> Result<Self> {
        let collection = child(root, "ExternalTextCollection");
        ensure!(
            collection.is_none_or(|node| node
                .children()
                .filter(|node| node.has_tag_name((IODD_NAMESPACE, "PrimaryLanguage")))
                .count()
                <= 1),
            "duplicate primary language"
        );
        let primary = collection.and_then(|node| child(node, "PrimaryLanguage"));
        let primary_language = primary
            .and_then(|node| node.attribute((XML, "lang")))
            .map(language_tag)
            .transpose()?;
        let primary_values = primary.map(text_table).transpose()?.unwrap_or_default();
        let mut languages = HashMap::new();
        for node in collection
            .into_iter()
            .flat_map(|node| node.children())
            .filter(|node| node.has_tag_name((IODD_NAMESPACE, "Language")))
        {
            let tag = language_tag(
                node.attribute((XML, "lang"))
                    .context("language has no xml:lang")?,
            )?;
            ensure!(
                Some(&tag) != primary_language.as_ref()
                    && languages.insert(tag.clone(), text_table(node)?).is_none(),
                "duplicate language {tag}"
            );
        }
        let mut available: BTreeSet<String> = languages
            .keys()
            .cloned()
            .chain(discovered.iter().cloned())
            .collect();
        available.extend(primary_language.iter().cloned());
        let selected = requested
            .map(language_tag)
            .transpose()?
            .or_else(|| primary_language.clone());
        let mut language_source = None;
        if let Some(companion) = companion {
            let doc = Document::parse(&companion.xml)?;
            let external = doc.root_element();
            ensure!(
                external.has_tag_name((IODD_NAMESPACE, "ExternalTextDocument")),
                "expected IODD ExternalTextDocument namespace"
            );
            let node = child(external, "Language")
                .context("external language document has no Language")?;
            ensure!(
                external
                    .children()
                    .filter(|node| node.has_tag_name((IODD_NAMESPACE, "Language")))
                    .count()
                    == 1,
                "external language document must contain exactly one Language"
            );
            let tag = language_tag(
                node.attribute((XML, "lang"))
                    .context("external language has no xml:lang")?,
            )?;
            ensure!(
                Some(&tag) == selected.as_ref(),
                "external language does not match the requested tag"
            );
            ensure!(
                Some(&tag) != primary_language.as_ref() && !languages.contains_key(&tag),
                "language {tag} has both embedded and external definitions"
            );
            if let (Some(main), Some(other)) =
                (child(root, "DocumentInfo"), child(external, "DocumentInfo"))
                && let (Some(a), Some(b)) = (
                    main.attribute("releaseDate"),
                    other.attribute("releaseDate"),
                )
            {
                ensure!(
                    a == b,
                    "language document releaseDate differs from main IODD"
                );
            }
            languages.insert(tag.clone(), text_table(node)?);
            available.insert(tag);
            language_source = Some(companion.source.clone());
        }
        let mut fallback = Vec::new();
        let mut values = primary_values.clone();
        if selected != primary_language {
            let tag = selected.as_deref().context("select a language")?;
            let translated = languages.get(tag).with_context(|| {
                format!(
                    "language {tag} is unavailable; available: {}",
                    available.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            })?;
            for id in translated.keys() {
                ensure!(
                    primary_values.contains_key(id) || id.starts_with("STD_TN_DeviceSpecific_"),
                    "translation contains unknown text ID {id}"
                );
            }
            fallback = primary_values
                .keys()
                .filter(|id| !translated.contains_key(*id))
                .cloned()
                .collect();
            fallback.sort();
            values.extend(
                translated
                    .iter()
                    .map(|(id, value)| (id.clone(), value.clone())),
            );
        }
        let mut review = Vec::new();
        if !fallback.is_empty() {
            review.push(format!(
                "{} text IDs use the primary language because the selected translation omits them",
                fallback.len()
            ));
        }
        if primary_language.is_none() {
            review.push("Primary language metadata is unavailable".into());
        }
        Ok(Self {
            values,
            metadata: Localization {
                primary_language,
                selected_language: selected,
                available_languages: available.into_iter().collect(),
                language_source,
                unit_source: units.source.clone(),
                unit_version: units.version.clone(),
                fallback_text_ids: fallback,
                review,
            },
        })
    }

    pub fn redefine_standard_records(&self, definitions: &mut [super::Parameter]) {
        for parameter in definitions
            .iter_mut()
            .filter(|p| p.id == "V_DirectParameters_2")
        {
            if let Some(record) = &mut parameter.record {
                for item in &mut record.items {
                    if let Some(name) = self
                        .values
                        .get(&format!("STD_TN_DeviceSpecific_{}", item.subindex))
                    {
                        item.parameter.name = name.clone();
                    }
                }
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Units {
    values: HashMap<u16, String>,
    pub source: Option<String>,
    pub version: Option<String>,
}

impl Units {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let Some(path) = path else {
            return Ok(Self::default());
        };
        let xml = super::read_xml(path)?;
        let doc = Document::parse(&xml)?;
        let root = doc.root_element();
        ensure!(
            root.has_tag_name((IODD_NAMESPACE, "IODDStandardUnitDefinitions")),
            "expected IODD standard unit definitions namespace"
        );
        let mut values = HashMap::new();
        for unit in child(root, "UnitCollection")
            .context("unit definitions have no UnitCollection")?
            .children()
            .filter(|node| node.has_tag_name((IODD_NAMESPACE, "Unit")))
        {
            ensure!(values.len() < 4096, "too many unit definitions");
            let code: u16 = unit
                .attribute("code")
                .context("unit has no code")?
                .parse()?;
            let abbreviation = unit.attribute("abbr").context("unit has no abbreviation")?;
            ensure!(
                abbreviation.len() <= 128 && !abbreviation.contains(['\r', '\n', '\0']),
                "invalid unit abbreviation"
            );
            ensure!(
                values.insert(code, abbreviation.into()).is_none(),
                "duplicate unit code {code}"
            );
        }
        ensure!(!values.is_empty(), "unit definitions are empty");
        Ok(Self {
            values,
            source: Some(
                path.file_name()
                    .context("unit definitions have no filename")?
                    .to_string_lossy()
                    .into_owned(),
            ),
            version: child(root, "DocumentInfo")
                .and_then(|node| node.attribute("version"))
                .map(str::to_owned),
        })
    }

    pub fn resolve(&self, code: Option<&str>) -> String {
        let parsed = code.and_then(|code| code.parse::<u16>().ok());
        if let Some(value) = parsed.and_then(|code| self.values.get(&code)) {
            return value.clone();
        }
        let normalized = parsed.map(|code| code.to_string());
        super::unit(normalized.as_deref().or(code))
    }
}
