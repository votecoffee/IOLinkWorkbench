use crate::{
    decode::{ByteOrder, Field, Kind, MAX_INPUT, insert_bits},
    profile::{Identity, Imported},
};
use anyhow::{Context, Result, bail, ensure};
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
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
mod communication;
pub use communication::{Bitrate, Communication};
mod output;
pub use output::{DecodedOutput, DecodedOutputField};
mod events;
pub use events::{EventDefinition, EventType};
mod parameters;
pub(crate) use parameters::decode_text_parameter;
pub use parameters::defaults;
mod resources;
pub mod schema;
pub mod stamp;
pub use parameters::{
    Access, Array, AssessmentStatus, Choice, DecodedParameter, Display, DisplayValue, NumberFormat,
    Parameter, Record, RecordItem, RecordValue, TimeDefaultConversion, TimeValue, ValueAssessment,
    ValueRange,
};
pub use resources::{Localization, StandardLocalization};

#[derive(Debug, Serialize)]
pub struct Variant {
    pub product_id: String,
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct Layout {
    pub id: String,
    pub input_bytes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<ProcessDataLayout>,
    pub condition: Option<String>,
    pub needs_input: Vec<String>,
    pub fields: Vec<Field>,
}

/// Declared geometry and supported typed mappings for a single process data item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessDataLayout {
    pub id: String,
    pub bit_length: usize,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub packed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub standalone_boolean: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub padded_integer: bool,
    pub condition: Option<String>,
    pub fields: Vec<Field>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs_input: Vec<String>,
}

impl ProcessDataLayout {
    pub fn bytes(&self) -> usize {
        if self.padded_integer
            && let Some(width) = self.integer_padding_width()
        {
            return width;
        }
        self.bit_length.div_ceil(8)
    }

    fn integer_padding_width(&self) -> Option<usize> {
        let [field] = self.fields.as_slice() else {
            return None;
        };
        if self.packed
            || self.standalone_boolean
            || !matches!(field.kind, Kind::Signed | Kind::Unsigned)
            || field.bit_offset != 0
            || !(2..64).contains(&field.bits)
        {
            return None;
        }
        let bits = usize::from(field.bits);
        let width = parameters::integer_container_bytes(u16::from(field.bits));
        (bits < width * 8 && (self.bit_length == bits || self.bit_length == width * 8))
            .then_some(width)
    }

    pub fn editable_output_fields(&self) -> impl Iterator<Item = &Field> {
        self.fields.iter().filter(|field| {
            // Packed values occupy their assigned bits. A standalone numeric value
            // covering a full wire container has no separate padding to regenerate.
            self.packed
                || self.standalone_boolean
                || self.padded_integer
                || (field.bit_offset == 0
                    && usize::from(field.bits) == self.bit_length
                    && matches!(field.bits, 8 | 16 | 32 | 64))
        })
    }

    pub(crate) fn edit_output_field(
        &self,
        mut image: Vec<u8>,
        name: &str,
        input: &str,
    ) -> Result<Vec<u8>> {
        self.validate()?;
        self.check_output_width(image.len())?;
        let field = self
            .editable_output_fields()
            .find(|field| field.name == name)
            .with_context(|| format!("no editable IODD output field named {name:?}"))?;
        let value =
            output_value(field, input).with_context(|| format!("desired value for {name}"))?;
        if self.standalone_boolean {
            image[0] = if value == 0 { 0 } else { 0xFF };
        } else if self.padded_integer {
            let start = 8 - image.len();
            image.copy_from_slice(&value.to_be_bytes()[start..]);
        } else {
            insert_bits(
                &mut image,
                field.bit_offset,
                u16::from(field.bits),
                &value.to_be_bytes(),
            )?;
        }
        Ok(image)
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(!self.id.trim().is_empty(), "missing process layout ID");
        ensure!(
            (1..=MAX_INPUT * 8).contains(&self.bit_length),
            "{}: invalid process data width",
            self.id
        );
        if self.standalone_boolean {
            ensure!(
                !self.packed
                    && matches!(self.bit_length, 1 | 8)
                    && matches!(
                        self.fields.as_slice(),
                        [Field {
                            kind: Kind::Bool,
                            bit_offset: 0,
                            ..
                        }]
                    ),
                "standalone Boolean output requires a single Boolean in one byte"
            );
        }
        ensure!(
            !self.padded_integer || self.integer_padding_width().is_some(),
            "padded integer output requires a standalone signed or unsigned field in its wire container"
        );
        let mut names = HashSet::new();
        let mut mapped = [false; MAX_INPUT * 8];
        for field in &self.fields {
            field.validate(self.bytes())?;
            ensure!(
                field.byte_order == ByteOrder::BigEndian,
                "IODD process data uses big-endian bit numbering"
            );
            ensure!(
                names.insert(&field.name),
                "duplicate process field {}",
                field.name
            );
            let start = usize::from(field.bit_offset);
            let end = start + usize::from(field.bits);
            ensure!(
                end <= self.bit_length,
                "{} extends beyond declared width",
                field.name
            );
            ensure!(
                !mapped[start..end].contains(&true),
                "{} overlaps another field",
                field.name
            );
            mapped[start..end].fill(true);
        }
        Ok(())
    }
}

// Caller has validated the integer geometry and confirmed a wire-sized payload.
fn integer_padding_matches(field: &Field, payload: &[u8]) -> bool {
    let bits = usize::from(field.bits);
    let mut word = [0; 8];
    word[8 - payload.len()..].copy_from_slice(payload);
    let word = u64::from_be_bytes(word);
    let negative = field.kind == Kind::Signed && word & (1 << (bits - 1)) != 0;
    let padding = if negative {
        (1 << (payload.len() * 8 - bits)) - 1
    } else {
        0
    };
    word >> bits == padding
}

fn output_value(field: &Field, input: &str) -> Result<u64> {
    let input = input.trim();
    Ok(match field.kind {
        Kind::Bool => match input {
            "true" | "1" => 1,
            "false" | "0" => 0,
            _ => bail!("expected true, false, 1 or 0"),
        },
        Kind::Unsigned => {
            let value: u64 = input
                .parse()
                .context("expected an unsigned decimal integer")?;
            ensure!(
                u128::from(value) < (1u128 << field.bits),
                "value exceeds the declared unsigned width"
            );
            value
        }
        Kind::Signed => {
            let value: i64 = input.parse().context("expected a signed decimal integer")?;
            let bound = 1i128 << (field.bits - 1);
            ensure!(
                (-bound..bound).contains(&i128::from(value)),
                "value exceeds the declared signed width"
            );
            value as u64
        }
        Kind::Float32 => {
            let value: f32 = input.parse().context("expected a decimal Float32 value")?;
            ensure!(value.is_finite(), "expected a finite Float32 value");
            u64::from(value.to_bits())
        }
    })
}

#[derive(Debug, Serialize)]
pub struct AcyclicLayout {
    pub id: String,
    pub condition: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Catalog {
    pub source: String,
    pub vendor_id: u16,
    pub device_id: u32,
    pub communication: Option<Communication>,
    pub variants: Vec<Variant>,
    pub layouts: Vec<Layout>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub output_only_layouts: Vec<ProcessDataLayout>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub acyclic_layouts: Vec<AcyclicLayout>,
    pub parameters: Vec<Parameter>,
    pub standard_parameter_refs: Vec<String>,
    pub schema_validation: Option<schema::Report>,
    pub stamp_validation: Option<stamp::Report>,
    pub localization: Option<Localization>,
    pub events: Vec<EventDefinition>,
    pub standard_event_refs: Vec<u16>,
}

#[derive(Default)]
pub(super) struct Definitions {
    pub parameters: Vec<Parameter>,
    pub events: Vec<EventDefinition>,
    pub localization: Option<StandardLocalization>,
}

impl Catalog {
    pub fn default_layout_id(&self) -> Option<&str> {
        match (
            self.layouts.as_slice(),
            self.output_only_layouts.as_slice(),
            self.acyclic_layouts.as_slice(),
        ) {
            ([input], [], []) if input.condition.is_none() => Some(&input.id),
            ([], [output], []) if output.condition.is_none() => Some(&output.id),
            ([], [], [acyclic]) if acyclic.condition.is_none() => Some(&acyclic.id),
            _ => None,
        }
    }

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
        let layout_id = layout.or_else(|| self.default_layout_id()).context(
            "select the process data layout and confirm any condition against the device",
        )?;
        let (input_bytes, condition, fields, output, mut needs_input) =
            if let Some(input) = self.layouts.iter().find(|input| input.id == layout_id) {
                (
                    input.input_bytes,
                    &input.condition,
                    input.fields.as_slice(),
                    input.output.as_ref(),
                    input.needs_input.clone(),
                )
            } else if let Some(output) = self
                .output_only_layouts
                .iter()
                .find(|output| output.id == layout_id)
            {
                (
                    0,
                    &output.condition,
                    &[][..],
                    Some(output),
                    output.needs_input.clone(),
                )
            } else {
                let acyclic = self
                    .acyclic_layouts
                    .iter()
                    .find(|layout| layout.id == layout_id)
                    .with_context(|| format!("unknown process data layout {layout_id}"))?;
                (0, &acyclic.condition, &[][..], None, Vec::new())
            };
        let acyclic_only = input_bytes == 0 && output.is_none();
        if let Some(localization) = &self.localization {
            needs_input.extend(localization.review.iter().cloned());
        }
        if let Some(condition) = condition
            && !acyclic_only
        {
            let workflow = if input_bytes == 0 {
                "reviewing live output"
            } else {
                "capture"
            };
            needs_input.push(format!(
                "Confirm active layout condition {condition} before {workflow}"
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
                layout: layout_id.into(),
                acyclic_only,
                condition: condition.clone(),
                input_bytes,
                output: output.cloned(),
                needs_input,
                fields: fields.to_vec(),
                parameters: self.parameters.clone(),
                standard_parameter_refs: self.standard_parameter_refs.clone(),
                schema_validation: self.schema_validation.clone(),
                stamp_validation: self.stamp_validation.clone(),
                communication: self.communication.clone(),
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
    pub require_stamp: bool,
    pub entry: Option<&'a str>,
    pub definitions: Option<&'a Path>,
    pub xsd: Option<&'a Path>,
    pub language: Option<&'a str>,
    pub language_file: Option<&'a Path>,
    pub unit_definitions: Option<&'a Path>,
    pub standard_language_file: Option<&'a Path>,
}

pub fn load_with_options(path: &Path, options: &LoadOptions<'_>) -> Result<Catalog> {
    ensure!(
        options.language_file.is_none() || options.language.is_some(),
        "a language file requires an explicit language tag"
    );
    ensure!(
        options.standard_language_file.is_none()
            || (options.language.is_some() && options.definitions.is_some()),
        "a standard language file requires a language tag and standard definitions"
    );
    let language = options.language.map(resources::language_tag).transpose()?;
    let input = read_source(
        path,
        options.entry,
        language.as_deref(),
        options.language_file,
    )?;
    let stamps = stamp::check(&input.bytes, input.name.clone(), input.companion.as_ref())?;
    stamps.enforce(options.require_stamp)?;
    let validation = options
        .xsd
        .map(|xsd| schema::validate(&input.bytes, xsd))
        .transpose()?;
    if let Some(report) = &validation {
        ensure!(report.passed, "{}", report.summary());
    }
    let units = resources::Units::load(options.unit_definitions)?;
    let xml = decode_xml(input.bytes)?;
    let doc = Document::parse(&xml)?;
    let primary_language = child(doc.root_element(), "ExternalTextCollection")
        .and_then(|texts| child(texts, "PrimaryLanguage"))
        .and_then(|primary| primary.attribute(("http://www.w3.org/XML/1998/namespace", "lang")))
        .map(resources::language_tag)
        .transpose()?;
    let definitions = options
        .definitions
        .map(|path| {
            parameters::load_standard(
                path,
                language.as_deref().or(primary_language.as_deref()),
                options.standard_language_file,
            )
        })
        .transpose()?
        .unwrap_or_default();
    let mut catalog = parse_resources(
        &xml,
        input.name,
        &definitions.parameters,
        language.as_deref(),
        input.companion.as_ref(),
        &input.languages,
        &units,
    )?;
    catalog.schema_validation = validation;
    catalog.stamp_validation = Some(stamps);
    if let Some(localization) = &mut catalog.localization {
        if let Some(standard) = &definitions.localization {
            localization.review.extend(
                standard
                    .review
                    .iter()
                    .map(|note| format!("Standard definitions: {note}")),
            );
        }
        localization.standard_definitions = definitions.localization;
    }
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

/// Inspect original main/selected-language bytes without importing a device mapping.
pub fn check_stamps(
    path: &Path,
    entry: Option<&str>,
    language: Option<&str>,
    language_file: Option<&Path>,
) -> Result<stamp::Report> {
    ensure!(
        language_file.is_none() || language.is_some(),
        "a language file requires an explicit language tag"
    );
    let language = language.map(resources::language_tag).transpose()?;
    let input = read_source(path, entry, language.as_deref(), language_file)?;
    stamp::check(&input.bytes, input.name, input.companion.as_ref())
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
                        xml: decode_xml(bytes.clone())?,
                        bytes,
                        source: format!("{}#{name}", path.file_name().unwrap().to_string_lossy()),
                    });
                }
            }
        }
        format!("{name}#{entry}")
    } else {
        ensure!(entry.is_none(), "ZIP entry applies only to ZIP files");
        file.take(XML_LIMIT + 1).read_to_end(&mut bytes)?;
        if language_file.is_none() {
            companion = resources::local_companion(path, language, None)?;
        }
        name.into_owned()
    };
    ensure!(bytes.len() as u64 <= XML_LIMIT, "IODD XML exceeds 8 MiB");
    if let Some(path) = language_file {
        let raw = read_xml_bytes(path)?;
        companion = Some(resources::Companion {
            xml: decode_xml(raw.clone())?,
            bytes: raw,
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
    decode_xml(read_xml_bytes(path)?)
}

fn read_xml_bytes(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| path.display().to_string())?
        .take(XML_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= XML_LIMIT, "IODD XML exceeds 8 MiB");
    Ok(bytes)
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
    let mut output_only_layouts = Vec::new();
    let mut acyclic_layouts = Vec::new();
    let mut process_ids = HashSet::new();
    for process in child(function, "ProcessDataCollection")
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "ProcessData")))
    {
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
        let mut read_layout = |node| {
            let layout = parse_process_item(
                node,
                condition.clone(),
                &types,
                &texts,
                &display_refs,
                units,
            )?;
            ensure!(
                process_ids.insert(layout.id.clone()),
                "duplicate process layout ID {}",
                layout.id
            );
            Ok::<_, anyhow::Error>(layout)
        };
        let input = child(process, "ProcessDataIn")
            .map(&mut read_layout)
            .transpose()?;
        let output = child(process, "ProcessDataOut")
            .map(read_layout)
            .transpose()?;
        if let Some(input) = input {
            let input_bytes = input.bytes();
            layouts.push(Layout {
                id: input.id,
                input_bytes,
                output,
                condition: input.condition,
                needs_input: input.needs_input,
                fields: input.fields,
            });
        } else if let Some(output) = output {
            output_only_layouts.push(output);
        } else if process
            .children()
            .filter(Node::is_element)
            .all(|node| node.has_tag_name((IODD_NAMESPACE, "Condition")))
        {
            let id = process
                .attribute("id")
                .filter(|id| !id.trim().is_empty())
                .context("acyclic layout has no ID")?;
            ensure!(
                process_ids.insert(id.into()),
                "duplicate process layout ID {id}"
            );
            acyclic_layouts.push(AcyclicLayout {
                id: id.into(),
                condition,
            });
        }
    }
    Ok(Catalog {
        source,
        vendor_id,
        device_id,
        communication: communication::parse(root)?,
        variants,
        layouts,
        output_only_layouts,
        acyclic_layouts,
        parameters,
        standard_parameter_refs,
        schema_validation: None,
        stamp_validation: None,
        localization: Some(selected.metadata),
        events,
        standard_event_refs,
    })
}

fn parse_process_item<'a, 'input>(
    node: Node<'a, 'input>,
    condition: Option<String>,
    types: &HashMap<&str, Node<'a, 'input>>,
    texts: &HashMap<&str, &str>,
    display_refs: &HashMap<&str, Node<'a, 'input>>,
    units: &resources::Units,
) -> Result<ProcessDataLayout> {
    let id = node
        .attribute("id")
        .filter(|id| !id.trim().is_empty())
        .context("process layout has no ID")?;
    let bits: usize = node
        .attribute("bitLength")
        .context("missing process data bitLength")?
        .parse()?;
    ensure!(
        (1..=MAX_INPUT * 8).contains(&bits),
        "{id}: invalid process data width"
    );
    let dt = datatype(node, types);
    let kind = dt
        .as_ref()
        .ok()
        .and_then(|dt| dt.attribute((XSI, "type")))
        .and_then(|kind| kind.rsplit(':').next());
    let mut layout = ProcessDataLayout {
        id: id.into(),
        bit_length: bits,
        packed: matches!(kind, Some("RecordT" | "ArrayT")),
        standalone_boolean: false,
        padded_integer: false,
        condition,
        fields: Vec::new(),
        needs_input: Vec::new(),
    };
    let mapping_hint = if node.has_tag_name((IODD_NAMESPACE, "ProcessDataIn")) {
        "; provide a manual field if needed"
    } else {
        "; review these bits from the device documentation"
    };
    let display = display_refs.get(id).copied();
    let mut missing_unit = false;
    let mut add = |name: String, result: Result<(Field, bool)>| -> Result<()> {
        let result = result.and_then(|(field, unit_defined)| {
            field.validate(layout.bytes())?;
            ensure!(
                usize::from(field.bit_offset) + usize::from(field.bits) <= bits,
                "extends beyond declared process data width"
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
                .push(format!("{name}: {error}{mapping_hint}")),
        }
        Ok(())
    };
    if let Ok(record) = dt.as_ref()
        && kind == Some("RecordT")
    {
        for item in record
            .children()
            .filter(|node| node.has_tag_name((IODD_NAMESPACE, "RecordItem")))
        {
            let subindex = item.attribute("subindex").unwrap_or("?");
            let mut name = label(item, texts, &format!("subindex_{subindex}"));
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
                    field(datatype(item, types)?, info, name, bit_offset, units)
                })(),
            )?;
        }
    } else if kind == Some("ArrayT") {
        let name = label(node, texts, id);
        let info = display
            .and_then(|reference| child(reference, "ProcessDataInfo"))
            .unwrap_or(node);
        let expanded = (|| -> Result<_> {
            let dt = dt?;
            let (template, unit_defined) =
                field(datatype(dt, types)?, info, name.clone(), 0, units)?;
            let count = dt
                .attribute("count")
                .context("array has no count")?
                .parse()?;
            let geometry = array::Geometry::new(u16::from(template.bits), count)?;
            ensure!(
                geometry.bits() as usize <= bits,
                "array extends beyond declared process data width"
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
        let name = label(node, texts, id);
        let info = display
            .and_then(|reference| child(reference, "ProcessDataInfo"))
            .unwrap_or(node);
        add(
            name.clone(),
            dt.and_then(|dt| field(dt, info, name, 0, units)),
        )?;
    }
    layout.standalone_boolean = kind == Some("BooleanT")
        && node.has_tag_name((IODD_NAMESPACE, "ProcessDataOut"))
        && matches!(bits, 1 | 8)
        && layout.fields.len() == 1;
    layout.padded_integer = matches!(kind, Some("IntegerT" | "UIntegerT"))
        && node.has_tag_name((IODD_NAMESPACE, "ProcessDataOut"))
        && layout.integer_padding_width().is_some();
    if missing_unit {
        layout.needs_input.push(
            "Review engineering units and scaling where the IODD has no display metadata".into(),
        );
    }
    Ok(layout)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod record_tests;

#[cfg(test)]
mod array_tests;

#[cfg(test)]
mod resource_tests;

#[cfg(test)]
mod standard_language_tests;
