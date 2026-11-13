use super::*;
use crate::profile::Profile;
use std::fs;

const DEFINITIONS: &str = include_str!("../../samples/synthetic-standard-definitions.xml");
const GERMAN: &str = include_str!("../../samples/synthetic-standard-definitions-de.xml");

#[test]
fn standard_labels_events_and_fallbacks_survive_profile_save_and_offline_reads() -> Result<()> {
    let catalog = load_with_options(
        Path::new("samples/sensor.xml"),
        &LoadOptions {
            definitions: Some(Path::new("samples/synthetic-standard-definitions.xml")),
            language: Some("DE"),
            ..Default::default()
        },
    )?;
    let standard = catalog
        .localization
        .as_ref()
        .unwrap()
        .standard_definitions
        .as_ref()
        .unwrap();
    assert_eq!(standard.requested_language.as_deref(), Some("de"));
    assert_eq!(standard.selected_language.as_deref(), Some("de"));
    assert_eq!(
        standard.language_source.as_deref(),
        Some("synthetic-standard-definitions-de.xml")
    );
    assert!(standard.fallback_text_ids.contains(&"offset".into()));
    assert!(
        catalog
            .events
            .iter()
            .any(|event| event.name == "Synthetischer Standardalarm")
    );
    let (expected_identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    let restored = Profile::parse(&toml::to_string_pretty(&profile)?)?;
    assert_eq!(restored, profile);
    let reading = crate::transport::read_parameter(&restored, "V_ProductID")?;
    assert_eq!(reading.name, "Synthetische Produktkennung");
    assert_eq!(reading.raw_value.as_deref(), Some("SIM-SENSOR"));
    assert_eq!(reading.index, 19);
    Ok(())
}

#[test]
fn translated_standard_record_members_choices_and_access_keep_the_raw_contract() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("standard.xml");
    fs::write(&path, DEFINITIONS)?;
    fs::write(folder.path().join("standard-de.xml"), GERMAN)?;
    let definitions = parameters::load_standard(&path, Some("de"), None)?;
    let record = definitions
        .parameters
        .iter()
        .find(|p| p.id == "V_DirectParameters_1")
        .unwrap();
    assert_eq!(record.target(1)?.name, "Synthetische Produktkennung");
    assert_eq!(record.target(16)?.name, "Synthetischer Systembefehl");
    assert_eq!(record.target(16)?.choices[0].name, "Synthetischer Neustart");
    assert_eq!(record.target(16)?.access, Access::WriteOnly);
    assert_eq!(record.bits, Some(16));
    assert_eq!(record.target(1)?.decode_raw(&[42])?.as_deref(), Some("42"));
    Ok(())
}

#[test]
fn absent_standard_translation_uses_primary_language_with_an_explicit_review() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("standard.xml");
    fs::write(&path, DEFINITIONS)?;
    let definitions = parameters::load_standard(&path, Some("de"), None)?;
    let locale = definitions.localization.unwrap();
    assert_eq!(locale.selected_language.as_deref(), Some("en"));
    assert_eq!(locale.requested_language.as_deref(), Some("de"));
    assert_eq!(locale.fallback_text_ids.len(), 8);
    assert!(
        locale
            .review
            .iter()
            .any(|note| note.contains("unavailable"))
    );
    assert!(
        parameters::load_standard(&path, Some("de"), Some(&folder.path().join("missing.xml")))
            .is_err()
    );
    Ok(())
}

#[test]
fn malformed_standard_translations_are_rejected_without_using_partial_labels() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("standard.xml");
    let sidecar = folder.path().join("standard-de.xml");
    fs::write(&path, DEFINITIONS)?;
    for translation in [
        GERMAN.replace("xml:lang=\"de\"", "xml:lang=\"fr\""),
        GERMAN.replace("synthetic-1", "different-version"),
        GERMAN.replace("id=\"product\"", "id=\"unknown\""),
        GERMAN.replace("id=\"product\"", "id=\"STD_TN_DeviceSpecific_99\""),
        GERMAN.replace("id=\"command\"", "id=\"product\""),
        GERMAN.replace("</Language>", "</Language><Language xml:lang=\"de\"/>"),
    ] {
        fs::write(&sidecar, translation)?;
        assert!(parameters::load_standard(&path, Some("de"), None).is_err());
    }
    fs::remove_file(&sidecar)?;
    fs::write(
        &path,
        DEFINITIONS.replace("id=\"command\" value=", "id=\"product\" value="),
    )?;
    assert!(parameters::load_standard(&path, None, None).is_err());
    Ok(())
}

#[test]
fn embedded_standard_language_and_device_primary_selection_are_supported() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("standard.xml");
    let table = GERMAN
        .split("<Language xml:lang=\"de\">")
        .nth(1)
        .unwrap()
        .split("</Language>")
        .next()
        .unwrap();
    let embedded = DEFINITIONS.replace(
        "</ExternalTextCollection>",
        &format!("<Language xml:lang=\"de\">{table}</Language></ExternalTextCollection>"),
    );
    fs::write(&path, embedded)?;
    let sensor = folder.path().join("sensor.xml");
    fs::write(
        &sensor,
        include_str!("../../samples/sensor.xml").replace("xml:lang=\"en\"", "xml:lang=\"de\""),
    )?;
    let catalog = load_with_options(
        &sensor,
        &LoadOptions {
            definitions: Some(&path),
            ..Default::default()
        },
    )?;
    assert_eq!(
        catalog
            .localization
            .unwrap()
            .standard_definitions
            .unwrap()
            .selected_language
            .as_deref(),
        Some("de")
    );
    fs::write(folder.path().join("standard-de.xml"), GERMAN)?;
    assert!(parameters::load_standard(&path, Some("de"), None).is_err());
    Ok(())
}

#[test]
fn explicit_standard_language_file_requires_context_and_checks_release_date() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("standard.xml");
    let language = folder.path().join("custom.xml");
    fs::write(
        &path,
        DEFINITIONS.replace(
            "version=\"synthetic-1\"",
            "version=\"synthetic-1\" releaseDate=\"2025-01-01\"",
        ),
    )?;
    fs::write(
        &language,
        GERMAN.replace(
            "version=\"synthetic-1\"",
            "version=\"synthetic-1\" releaseDate=\"2025-01-02\"",
        ),
    )?;
    assert!(parameters::load_standard(&path, Some("de"), Some(&language)).is_err());
    fs::write(&language, GERMAN)?;
    assert_eq!(
        parameters::load_standard(&path, Some("de"), Some(&language))?
            .localization
            .unwrap()
            .language_source
            .as_deref(),
        Some("custom.xml")
    );
    assert!(
        load_with_options(
            Path::new("samples/sensor.xml"),
            &LoadOptions {
                standard_language_file: Some(&language),
                ..Default::default()
            }
        )
        .is_err()
    );
    Ok(())
}
