use crate::{
    decode::{Field, Kind, MAX_INPUT},
    profile::{Identity, Imported},
};
use anyhow::{Context, Result, bail, ensure};
use roxmltree::{Document, Node};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};

const XML_LIMIT: u64 = 8 * 1024 * 1024;
const IODD_NAMESPACE: &str = "http://www.io-link.com/IODD/2010/10";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

mod array;
mod events;
pub use events::{EventDefinition, EventType};
mod parameters;
mod resources;
pub mod schema;
pub use parameters::{
    Access, Array, Choice, DecodedParameter, Display, DisplayValue, NumberFormat, Parameter,
    Record, RecordItem, RecordValue, ValueRange,
};
pub use resources::Localization;

#[derive(Debug, Serialize)]
pub struct Variant {
    pub product_id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct Layout {
    pub id: String,
    pub input_bytes: usize,
    pub condition: Option<String>,
    pub needs_input: Vec<String>,
    pub fields: Vec<Field>,
}

#[derive(Debug, Serialize)]
pub struct Catalog {
    pub source: String,
    pub vendor_id: u16,
    pub device_id: u32,
    pub variants: Vec<Variant>,
    pub layouts: Vec<Layout>,
    pub parameters: Vec<Parameter>,
    pub standard_parameter_refs: Vec<String>,
    pub schema_validation: Option<schema::Report>,
    pub localization: Option<Localization>,
    pub events: Vec<EventDefinition>,
    pub standard_event_refs: Vec<u16>,
}

#[derive(Default)]
pub(super) struct Definitions {
    pub parameters: Vec<Parameter>,
    pub events: Vec<EventDefinition>,
}

impl Catalog {
    pub fn select(
        &self,
        variant: Option<&str>,
        layout: Option<&str>,
    ) -> Result<(Identity, Imported)> {
        let variant = match variant {
            Some(id) => self
                .variants
                .iter()
                .find(|variant| variant.product_id == id)
                .with_context(|| format!("unknown product ID {id}"))?,
            None if self.variants.len() == 1 => &self.variants[0],
            None => bail!("select the connected product ID"),
        };
        let layout = match layout {
            Some(id) => self
                .layouts
                .iter()
                .find(|layout| layout.id == id)
                .with_context(|| format!("unknown input layout {id}"))?,
            None if self.layouts.len() == 1 && self.layouts[0].condition.is_none() => {
                &self.layouts[0]
            }
            None => bail!("select the input layout and confirm any condition against the device"),
        };
        let mut needs_input = layout.needs_input.clone();
        if let Some(localization) = &self.localization {
            needs_input.extend(localization.review.iter().cloned());
        }
        if let Some(condition) = &layout.condition {
            needs_input.push(format!(
                "Confirm active layout condition {condition} before capture"
            ));
        }
        Ok((
            Identity {
                vendor_id: Some(self.vendor_id),
                device_id: Some(self.device_id),
                product_id: Some(variant.product_id.clone()),
                ..Identity::default()
            },
            Imported {
                source: self.source.clone(),
                variant: variant.product_id.clone(),
                layout: layout.id.clone(),
                condition: layout.condition.clone(),
                input_bytes: layout.input_bytes,
                needs_input,
                fields: layout.fields.clone(),
                parameters: self.parameters.clone(),
                standard_parameter_refs: self.standard_parameter_refs.clone(),
                schema_validation: self.schema_validation.clone(),
                localization: self.localization.clone(),
                events: self.events.clone(),
                standard_event_refs: self.standard_event_refs.clone(),
            },
        ))
    }
}

pub fn load(path: &Path, entry: Option<&str>) -> Result<Catalog> {
    load_with_definitions(path, entry, None)
}

/// Optional definitions are local XML; only resolved metadata is saved in profiles.
pub fn load_with_definitions(
    path: &Path,
    entry: Option<&str>,
    definitions: Option<&Path>,
) -> Result<Catalog> {
    load_checked(path, entry, definitions, None)
}

pub fn load_checked(
    path: &Path,
    entry: Option<&str>,
    definitions: Option<&Path>,
    xsd: Option<&Path>,
) -> Result<Catalog> {
    load_with_options(
        path,
        &LoadOptions {
            entry,
            definitions,
            xsd,
            ..Default::default()
        },
    )
}

#[derive(Default)]
pub struct LoadOptions<'a> {
    pub entry: Option<&'a str>,
    pub definitions: Option<&'a Path>,
    pub xsd: Option<&'a Path>,
    pub language: Option<&'a str>,
    pub language_file: Option<&'a Path>,
    pub unit_definitions: Option<&'a Path>,
}

pub fn load_with_options(path: &Path, options: &LoadOptions<'_>) -> Result<Catalog> {
    ensure!(
        options.language_file.is_none() || options.language.is_some(),
        "a language file requires an explicit language tag"
    );
    let definitions = options
        .definitions
        .map(parameters::load_standard)
        .transpose()?
        .unwrap_or_default();
    let language = options.language.map(resources::language_tag).transpose()?;
    let input = read_source(
        path,
        options.entry,
        language.as_deref(),
        options.language_file,
    )?;
    let validation = options
        .xsd
        .map(|xsd| schema::validate(&input.bytes, xsd))
        .transpose()?;
    if let Some(report) = &validation {
        ensure!(report.passed, "{}", report.summary());
    }
    let units = resources::Units::load(options.unit_definitions)?;
    let mut catalog = parse_resources(
        &decode_xml(input.bytes)?,
        input.name,
        &definitions.parameters,
        language.as_deref(),
        input.companion.as_ref(),
        &input.languages,
        &units,
    )?;
    catalog.schema_validation = validation;
    catalog.standard_event_refs.retain(|code| {
        if let Some(event) = definitions.events.iter().find(|event| event.code == *code) {
            catalog.events.push(event.clone());
            false
        } else {
            true
        }
    });
    Ok(catalog)
}

/// Check only the selected main XML against the supplied local XSD closure.
pub fn check_schema(path: &Path, entry: Option<&str>, xsd: &Path) -> Result<schema::Report> {
    let (bytes, _) = source_bytes(path, entry)?;
    schema::validate(&bytes, xsd)
}

struct Source {
    bytes: Vec<u8>,
    name: String,
    companion: Option<resources::Companion>,
    languages: Vec<String>,
}

fn source_bytes(path: &Path, entry: Option<&str>) -> Result<(Vec<u8>, String)> {
    let source = read_source(path, entry, None, None)?;
    Ok((source.bytes, source.name))
}

fn read_source(
    path: &Path,
    entry: Option<&str>,
    language: Option<&str>,
    language_file: Option<&Path>,
) -> Result<Source> {
    let name = path
        .file_name()
        .context("missing file name")?
        .to_string_lossy();
    let file = File::open(path).with_context(|| path.display().to_string())?;
    let mut bytes = Vec::new();
    let mut companion = None;
    let mut languages = Vec::new();
    let source = if name.to_ascii_lowercase().ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file)?;
        ensure!(zip.len() <= 2048, "too many entries in IODD ZIP");
        let names: Vec<String> = zip.file_names().map(str::to_owned).collect();
        let candidates: Vec<_> = names
            .iter()
            .filter(|name| {
                let lower = name.to_ascii_lowercase();
                lower.ends_with("-iodd1.1.xml") || lower.ends_with("-iodd1.0.1.xml")
            })
            .collect();
        let entry = match entry {
            Some(name) => name,
            None if candidates.len() == 1 => candidates[0],
            None => bail!(
                "select ZIP entry with --entry or extract the XML; candidates: {}",
                candidates
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        ensure!(
            names.iter().filter(|name| *name == entry).count() == 1,
            "selected ZIP entry is missing or duplicated"
        );
        zip.by_name(entry)?
            .take(XML_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if entry.to_ascii_lowercase().ends_with(".xml") {
            let prefix = format!("{}-", &entry[..entry.len() - 4]).to_ascii_lowercase();
            let mut selected = Vec::new();
            for candidate in &names {
                let lower = candidate.to_ascii_lowercase();
                if let Some(tag) = lower
                    .strip_prefix(&prefix)
                    .and_then(|name| name.strip_suffix(".xml"))
                    && let Ok(tag) = resources::language_tag(tag)
                {
                    languages.push(tag.clone());
                    if language == Some(tag.as_str()) {
                        selected.push(candidate);
                    }
                }
            }
            if language_file.is_none() {
                ensure!(
                    selected.len() <= 1,
                    "selected ZIP language companion is ambiguous"
                );
                if let Some(name) = selected.first() {
                    let mut bytes = Vec::new();
                    zip.by_name(name)?
                        .take(XML_LIMIT + 1)
                        .read_to_end(&mut bytes)?;
                    companion = Some(resources::Companion {
                        xml: decode_xml(bytes)?,
                        source: format!("{}#{name}", path.file_name().unwrap().to_string_lossy()),
                    });
                }
            }
        }
        format!("{name}#{entry}")
    } else {
        ensure!(entry.is_none(), "ZIP entry applies only to ZIP files");
        file.take(XML_LIMIT + 1).read_to_end(&mut bytes)?;
        if language_file.is_none()
            && let Some(language) = language
            && let Some(stem) = path.file_stem()
        {
            let sidecar = path.with_file_name(format!("{}-{language}.xml", stem.to_string_lossy()));
            let metadata = match std::fs::symlink_metadata(&sidecar) {
                Ok(metadata) => Some(metadata),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error.into()),
            };
            if let Some(metadata) = metadata {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    ensure!(
                        metadata.file_attributes() & 0x400 == 0,
                        "automatic language sidecar is a reparse point"
                    );
                }
                ensure!(
                    !metadata.file_type().is_symlink(),
                    "automatic language sidecar is a symbolic link"
                );
                companion = Some(resources::Companion {
                    xml: read_xml(&sidecar)?,
                    source: sidecar.file_name().unwrap().to_string_lossy().into_owned(),
                });
            }
        }
        name.into_owned()
    };
    ensure!(bytes.len() as u64 <= XML_LIMIT, "IODD XML exceeds 8 MiB");
    if let Some(path) = language_file {
        companion = Some(resources::Companion {
            xml: read_xml(path)?,
            source: path
                .file_name()
                .context("language file has no filename")?
                .to_string_lossy()
                .into_owned(),
        });
    }
    Ok(Source {
        bytes,
        name: source,
        companion,
        languages,
    })
}

fn read_xml(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| path.display().to_string())?
        .take(XML_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    decode_xml(bytes)
}

fn decode_xml(bytes: Vec<u8>) -> Result<String> {
    ensure!(bytes.len() as u64 <= XML_LIMIT, "IODD XML exceeds 8 MiB");
    let xml = match bytes.as_slice() {
        [0xFF, 0xFE, body @ ..] | [0xFE, 0xFF, body @ ..] => {
            ensure!(body.len().is_multiple_of(2), "truncated UTF-16 XML");
            let little = bytes[0] == 0xFF;
            let units: Vec<_> = body
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&p| {
                    if little {
                        u16::from_le_bytes(p)
                    } else {
                        u16::from_be_bytes(p)
                    }
                })
                .collect();
            String::from_utf16(&units)?
        }
        _ => String::from_utf8(bytes)?,
    };
    Ok(xml.trim_start_matches('\u{feff}').to_owned())
}

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|n| n.has_tag_name((IODD_NAMESPACE, name)))
}

fn label(node: Node<'_, '_>, texts: &HashMap<&str, &str>, fallback: &str) -> String {
    child(node, "Name")
        .and_then(|name| name.attribute("textId"))
        .map(|id| texts.get(id).copied().unwrap_or(id))
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn datatype<'a, 'i>(
    node: Node<'a, 'i>,
    types: &HashMap<&str, Node<'a, 'i>>,
) -> Result<Node<'a, 'i>> {
    if let Some(dt) = child(node, "Datatype").or_else(|| child(node, "SimpleDatatype")) {
        return Ok(dt);
    }
    let id = child(node, "DatatypeRef")
        .and_then(|dt| dt.attribute("datatypeId"))
        .context("missing datatype")?;
    types
        .get(id)
        .copied()
        .with_context(|| format!("unresolved datatype {id}"))
}

fn field(
    dt: Node<'_, '_>,
    owner: Node<'_, '_>,
    name: String,
    bit_offset: u16,
    units: &resources::Units,
) -> Result<(Field, bool)> {
    let datatype = dt
        .attribute((XSI, "type"))
        .context("missing datatype xsi:type")?;
    let (kind, bits) = match datatype.rsplit(':').next().unwrap_or(datatype) {
        "UIntegerT" => (
            Kind::Unsigned,
            dt.attribute("bitLength")
                .context("missing width")?
                .parse()?,
        ),
        "IntegerT" => (
            Kind::Signed,
            dt.attribute("bitLength")
                .context("missing width")?
                .parse()?,
        ),
        "BooleanT" => (Kind::Bool, 1),
        "Float32T" => (Kind::Float32, 32),
        other => bail!("unsupported datatype {other}"),
    };
    let scale = owner.attribute("gradient").unwrap_or("1").parse()?;
    let offset = owner.attribute("offset").unwrap_or("0").parse()?;
    let decimals = owner
        .attribute("displayFormat")
        .and_then(|format| format.strip_prefix("Dec."))
        .map(str::parse)
        .transpose()?;
    let unit = units.resolve(owner.attribute("unitCode"));
    Ok((
        Field {
            name,
            bit_offset,
            kind,
            bits,
            scale,
            offset,
            unit,
            decimals,
            ..Field::default()
        },
        owner.attribute("unitCode").is_some(),
    ))
}

// Common unit codes from IO-Link Standard Unit Definitions V1.1.9.
fn unit(code: Option<&str>) -> String {
    code.map(|code| {
        match code {
            "1000" => "K",
            "1001" => "°C",
            "1002" => "°F",
            "1005" => "°",
            "1010" => "m",
            "1012" => "cm",
            "1013" => "mm",
            "1014" => "µm",
            "1019" => "in",
            "1038" => "L",
            "1054" => "s",
            "1056" => "ms",
            "1057" => "µs",
            "1061" => "m/s",
            "1077" => "Hz",
            "1085" => "rpm",
            "1130" => "Pa",
            "1133" => "kPa",
            "1137" => "bar",
            "1209" => "A",
            "1211" => "mA",
            "1240" => "V",
            "1342" => "%",
            "1352" => "L/min",
            "1997" => "",
            _ => return format!("IODD unit {code}"),
        }
        .to_owned()
    })
    .unwrap_or_default()
}

pub fn parse(xml: &str, source: String) -> Result<Catalog> {
    parse_with_definitions(xml, source, &[])
}

fn parse_with_definitions(xml: &str, source: String, definitions: &[Parameter]) -> Result<Catalog> {
    parse_resources(
        xml,
        source,
        definitions,
        None,
        None,
        &[],
        &resources::Units::default(),
    )
}

fn parse_resources(
    xml: &str,
    source: String,
    definitions: &[Parameter],
    language: Option<&str>,
    companion: Option<&resources::Companion>,
    discovered: &[String],
    units: &resources::Units,
) -> Result<Catalog> {
    let doc = Document::parse(xml)?;
    let root = doc.root_element();
    ensure!(
        root.has_tag_name((IODD_NAMESPACE, "IODevice")),
        "expected IODD 1.1 IODevice namespace"
    );
    let body = child(root, "ProfileBody").context("missing ProfileBody")?;
    let identity = child(body, "DeviceIdentity").context("missing DeviceIdentity")?;
    let vendor_id = identity
        .attribute("vendorId")
        .context("missing vendorId")?
        .parse()?;
    let device_id: u32 = identity
        .attribute("deviceId")
        .context("missing deviceId")?
        .parse()?;
    ensure!(device_id <= 0xFF_FFFF, "Device ID exceeds 24 bits");
    let selected = resources::Texts::load(root, language, companion, discovered, units)?;
    let texts = selected
        .values
        .iter()
        .map(|(id, value)| (id.as_str(), value.as_str()))
        .collect();
    let mut definitions = definitions.to_vec();
    selected.redefine_standard_records(&mut definitions);
    let mut product_ids = HashSet::new();
    let variants = child(identity, "DeviceVariantCollection")
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "DeviceVariant")))
        .map(|node| {
            let id = node
                .attribute("productId")
                .context("variant has no productId")?;
            ensure!(product_ids.insert(id), "duplicate variant Product ID {id}");
            Ok(Variant {
                product_id: id.into(),
                name: label(node, &texts, id),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let function = child(body, "DeviceFunction").context("missing DeviceFunction")?;
    let mut types = HashMap::new();
    for node in child(function, "DatatypeCollection")
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "Datatype")))
    {
        let id = node.attribute("id").context("datatype has no id")?;
        ensure!(
            types.insert(id, node).is_none(),
            "duplicate datatype ID {id}"
        );
    }
    let display_refs = child(function, "UserInterface")
        .and_then(|ui| child(ui, "ProcessDataRefCollection"))
        .into_iter()
        .flat_map(|refs| refs.children())
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "ProcessDataRef")))
        .filter_map(|node| Some((node.attribute("processDataId")?, node)))
        .collect::<HashMap<_, _>>();
    let (parameters, standard_parameter_refs) =
        parameters::parse(function, &types, &texts, &definitions, units)?;
    let (events, standard_event_refs) = events::parse(function, &texts)?;
    let mut layouts = Vec::new();
    for process in child(function, "ProcessDataCollection")
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "ProcessData")))
    {
        let Some(input) = child(process, "ProcessDataIn") else {
            continue;
        };
        let id = input.attribute("id").context("input layout has no id")?;
        let bits: usize = input
            .attribute("bitLength")
            .context("missing input bitLength")?
            .parse()?;
        ensure!(
            (1..=MAX_INPUT * 8).contains(&bits),
            "{id}: invalid input width"
        );
        let condition = child(process, "Condition").map(|n| {
            format!(
                "{}{} = {}",
                n.attribute("variableId").unwrap_or("?"),
                n.attribute("subindex")
                    .map(|s| format!("[{s}]"))
                    .unwrap_or_default(),
                n.attribute("value").unwrap_or("?")
            )
        });
        let mut layout = Layout {
            id: id.into(),
            input_bytes: bits.div_ceil(8),
            condition,
            fields: Vec::new(),
            needs_input: Vec::new(),
        };
        let dt = datatype(input, &types);
        let display = display_refs.get(id).copied();
        let mut missing_unit = false;
        let mut add = |name: String, result: Result<(Field, bool)>| -> Result<()> {
            let result = result.and_then(|(field, unit_defined)| {
                field.validate(layout.input_bytes)?;
                ensure!(
                    usize::from(field.bit_offset) + usize::from(field.bits) <= bits,
                    "extends beyond declared input width"
                );
                Ok((field, unit_defined))
            });
            match result {
                Ok((field, unit_defined)) => {
                    for existing in &layout.fields {
                        ensure!(
                            field.bit_offset >= existing.bit_offset + u16::from(existing.bits)
                                || existing.bit_offset >= field.bit_offset + u16::from(field.bits),
                            "{id}: fields {} and {} overlap",
                            existing.name,
                            field.name
                        );
                    }
                    missing_unit |= field.kind != Kind::Bool && !unit_defined;
                    if field.unit.starts_with("IODD unit ") {
                        layout
                            .needs_input
                            .push(format!("{name}: resolve {} to a unit symbol", field.unit));
                    }
                    layout.fields.push(field);
                }
                Err(error) => layout
                    .needs_input
                    .push(format!("{name}: {error}; provide a manual field if needed")),
            }
            Ok(())
        };
        if let Ok(record) = dt.as_ref()
            && record
                .attribute((XSI, "type"))
                .is_some_and(|kind| kind.rsplit(':').next() == Some("RecordT"))
        {
            for item in record
                .children()
                .filter(|node| node.has_tag_name((IODD_NAMESPACE, "RecordItem")))
            {
                let subindex = item.attribute("subindex").unwrap_or("?");
                let mut name = label(item, &texts, &format!("subindex_{subindex}"));
                // Keep a stable mapping key even when a vendor repeats a display name.
                name = format!("{name} [{subindex}]");
                add(
                    name.clone(),
                    (|| {
                        let bit_offset = item
                            .attribute("bitOffset")
                            .context("missing bitOffset")?
                            .parse()?;
                        let info = display
                            .and_then(|reference| {
                                reference.children().find(|node| {
                                    node.has_tag_name((IODD_NAMESPACE, "ProcessDataRecordItemInfo"))
                                        && node.attribute("subindex") == Some(subindex)
                                })
                            })
                            .unwrap_or(item);
                        field(datatype(item, &types)?, info, name, bit_offset, units)
                    })(),
                )?;
            }
        } else if dt.as_ref().is_ok_and(|dt| {
            dt.attribute((XSI, "type"))
                .is_some_and(|kind| kind.rsplit(':').next() == Some("ArrayT"))
        }) {
            let name = label(input, &texts, id);
            let info = display
                .and_then(|reference| child(reference, "ProcessDataInfo"))
                .unwrap_or(input);
            let expanded = (|| -> Result<_> {
                let dt = dt?;
                let (template, unit_defined) =
                    field(datatype(dt, &types)?, info, name.clone(), 0, units)?;
                let count = dt
                    .attribute("count")
                    .context("array has no count")?
                    .parse()?;
                let geometry = array::Geometry::new(u16::from(template.bits), count)?;
                ensure!(
                    geometry.bits() as usize <= bits,
                    "array extends beyond declared input width"
                );
                Ok((template, unit_defined, geometry))
            })();
            match expanded {
                Ok((template, unit_defined, geometry)) => {
                    for subindex in 1..=geometry.count {
                        let mut field = template.clone();
                        field.name = format!("{name} [{subindex}]");
                        field.bit_offset = geometry.offset(subindex);
                        add(field.name.clone(), Ok((field, unit_defined)))?;
                    }
                }
                Err(error) => add(name, Err(error))?,
            }
        } else {
            let name = label(input, &texts, id);
            let info = display
                .and_then(|reference| child(reference, "ProcessDataInfo"))
                .unwrap_or(input);
            add(
                name.clone(),
                dt.and_then(|dt| field(dt, info, name, 0, units)),
            )?;
        }
        if missing_unit {
            layout.needs_input.push(
                "Review engineering units and scaling where the IODD has no display metadata"
                    .into(),
            );
        }
        layouts.push(layout);
    }
    Ok(Catalog {
        source,
        vendor_id,
        device_id,
        variants,
        layouts,
        parameters,
        standard_parameter_refs,
        schema_validation: None,
        localization: Some(selected.metadata),
        events,
        standard_event_refs,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod record_tests;

#[cfg(test)]
mod array_tests;

#[cfg(test)]
mod resource_tests;
