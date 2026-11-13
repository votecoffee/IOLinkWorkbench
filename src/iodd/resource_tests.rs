use super::*;
use crate::{profile::Profile, transport};
use std::{fs, io::Write};

const SENSOR: &str = include_str!("../../samples/sensor.xml");
const GERMAN: &str = include_str!("../../samples/sensor-de.xml");

fn profile(catalog: &Catalog) -> Result<Profile> {
    let (expected_identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    Ok(Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    })
}

#[test]
fn sidecar_language_changes_labels_but_preserves_identity_and_raw_decoding() -> Result<()> {
    let catalog = load_with_options(
        Path::new("samples/sensor.xml"),
        &LoadOptions {
            language: Some("DE"),
            ..Default::default()
        },
    )?;
    let locale = catalog.localization.as_ref().unwrap();
    assert_eq!(locale.selected_language.as_deref(), Some("de"));
    assert_eq!(locale.language_source.as_deref(), Some("sensor-de.xml"));
    assert!(!locale.fallback_text_ids.is_empty());
    let profile = profile(&catalog)?;
    assert_eq!(
        profile.expected_identity.product_id.as_deref(),
        Some("SIM-SENSOR")
    );
    assert_eq!(profile.decode(&[0xFF, 0xF9])?[0].0.name, "Füllstand [1]");
    let reading = transport::read_parameter(&profile, "V_SIM_Threshold")?;
    assert_eq!(reading.name, "Grenzwert");
    assert_eq!(reading.choice_name.as_deref(), Some("Nennwert"));
    assert_eq!(reading.raw_value.as_deref(), Some("500"));
    assert_eq!(Profile::parse(&toml::to_string_pretty(&profile)?)?, profile);
    Ok(())
}

#[test]
fn embedded_languages_use_primary_fallbacks_and_reject_conflicts() -> Result<()> {
    let embedded = GERMAN
        .split("<Language xml:lang=\"de\">")
        .nth(1)
        .unwrap()
        .split("</Language>")
        .next()
        .unwrap();
    let xml = SENSOR.replace(
        "</ExternalTextCollection>",
        &format!("<Language xml:lang=\"de\">{embedded}</Language></ExternalTextCollection>"),
    );
    let catalog = parse_resources(
        &xml,
        "embedded fixture".into(),
        &[],
        Some("de"),
        None,
        &[],
        &resources::Units::default(),
    )?;
    assert_eq!(catalog.parameters[0].name, "Grenzwert");
    assert_eq!(catalog.parameters[1].name, "Synthetic label");
    assert!(
        parse_resources(
            &xml,
            "conflict".into(),
            &[],
            Some("de"),
            Some(&resources::Companion {
                xml: GERMAN.into(),
                bytes: GERMAN.as_bytes().to_vec(),
                source: "explicit.xml".into()
            }),
            &[],
            &resources::Units::default()
        )
        .is_err()
    );
    assert!(
        parse_resources(
            &SENSOR.replace("id=\"alt\"", "id=\"sensor\""),
            "duplicate text".into(),
            &[],
            None,
            None,
            &[],
            &resources::Units::default()
        )
        .is_err()
    );
    assert!(
        parse_resources(
            &xml,
            "unknown language".into(),
            &[],
            Some("fr"),
            None,
            &[],
            &resources::Units::default()
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn zip_companions_are_read_in_place_and_ambiguous_members_are_rejected() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("bundle.zip");
    for duplicate in [false, true] {
        let mut zip = zip::ZipWriter::new(File::create(&path)?);
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("folder/Synthetic-IODD1.1.xml", options)?;
        zip.write_all(SENSOR.as_bytes())?;
        zip.start_file("folder/Synthetic-IODD1.1-de.xml", options)?;
        zip.write_all(GERMAN.as_bytes())?;
        if duplicate {
            zip.start_file("folder/Synthetic-IODD1.1-DE.xml", options)?;
            zip.write_all(GERMAN.as_bytes())?;
        }
        zip.finish()?;
        let result = load_with_options(
            &path,
            &LoadOptions {
                language: Some("de"),
                ..Default::default()
            },
        );
        if duplicate {
            assert!(result.is_err());
        } else {
            let catalog = result?;
            assert_eq!(catalog.parameters[0].name, "Grenzwert");
            assert!(
                catalog
                    .localization
                    .unwrap()
                    .language_source
                    .unwrap()
                    .contains("#folder/")
            );
        }
    }
    assert!(!dir.path().join("folder").exists());
    Ok(())
}

#[test]
fn supplied_units_resolve_new_codes_and_preserve_scaling_and_no_unit() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("sensor.xml");
    fs::write(
        &path,
        SENSOR.replace("unitCode=\"1013\"", "unitCode=\"1015\""),
    )?;
    let unknown = load(&path, None)?;
    assert_eq!(
        unknown.parameters[0].displays[0].field.unit,
        "IODD unit 1015"
    );
    let known = load_with_options(
        &path,
        &LoadOptions {
            unit_definitions: Some(Path::new("samples/synthetic-unit-definitions.xml")),
            ..Default::default()
        },
    )?;
    let reading = transport::read_parameter(&profile(&known)?, "V_SIM_Threshold")?;
    assert_eq!(reading.displays[0].unit, "nm");
    assert_eq!(reading.displays[0].value, "52.0");
    let units = resources::Units::load(Some(Path::new("samples/synthetic-unit-definitions.xml")))?;
    assert_eq!(units.resolve(Some("01997")), "");
    assert_eq!(units.resolve(Some("1001")), "°C");
    assert_eq!(units.resolve(Some("65000")), "IODD unit 65000");
    Ok(())
}

#[test]
fn mismatched_language_tags_dates_and_duplicate_units_are_rejected() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let xml = dir.path().join("sensor.xml");
    let language = dir.path().join("translation.xml");
    fs::write(
        &xml,
        SENSOR.replace(
            "<ProfileBody>",
            "<DocumentInfo releaseDate=\"2026-01-01\"/><ProfileBody>",
        ),
    )?;
    fs::write(
        &language,
        GERMAN.replace(
            "<Language",
            "<DocumentInfo releaseDate=\"2026-01-02\"/><Language",
        ),
    )?;
    assert!(
        load_with_options(
            &xml,
            &LoadOptions {
                language: Some("de"),
                language_file: Some(&language),
                ..Default::default()
            }
        )
        .is_err()
    );
    fs::write(
        &language,
        GERMAN.replace("xml:lang=\"de\"", "xml:lang=\"fr\""),
    )?;
    assert!(
        load_with_options(
            &xml,
            &LoadOptions {
                language: Some("de"),
                language_file: Some(&language),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(
        load_with_options(
            &xml,
            &LoadOptions {
                language: Some("../de"),
                ..Default::default()
            }
        )
        .is_err()
    );
    let units = dir.path().join("units.xml");
    fs::write(
        &units,
        include_str!("../../samples/synthetic-unit-definitions.xml")
            .replace("code=\"1997\"", "code=\"1015\""),
    )?;
    assert!(resources::Units::load(Some(&units)).is_err());
    Ok(())
}

#[test]
fn direct_parameter_page_two_uses_declared_standard_text_redefinitions() -> Result<()> {
    let mut definition = parse(
        include_str!("../../samples/sensor-record.xml"),
        "synthetic definition".into(),
    )?
    .parameters
    .remove(0);
    definition.id = "V_DirectParameters_2".into();
    definition.index = 1;
    for item in &mut definition.record.as_mut().unwrap().items {
        item.parameter.index = 1;
    }
    let xml = SENSOR.replace("</VariableCollection>", "<StdVariableRef id=\"V_DirectParameters_2\"/></VariableCollection>")
        .replace("</PrimaryLanguage>", "<TextRedefine id=\"STD_TN_DeviceSpecific_1\" value=\"Device slot one\"/></PrimaryLanguage>");
    let catalog = parse_with_definitions(&xml, "redefined text fixture".into(), &[definition])?;
    let parameter = catalog
        .parameters
        .iter()
        .find(|parameter| parameter.id == "V_DirectParameters_2")
        .unwrap();
    assert_eq!(parameter.target(1)?.name, "Device slot one");
    assert!(
        parse(
            &xml.replace("STD_TN_DeviceSpecific_1", "STD_TN_NotAllowed"),
            "invalid redefinition".into()
        )
        .is_err()
    );
    Ok(())
}
