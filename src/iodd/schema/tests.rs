use super::*;
use crate::{iodd, profile::Profile};
use std::{fs, path::PathBuf, time::Duration};

const SENSOR: &str = include_str!("../../../samples/sensor.xml");
const ROOT: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:i="http://www.io-link.com/IODD/2010/10" targetNamespace="http://www.io-link.com/IODD/2010/10" elementFormDefault="qualified"><xs:include schemaLocation="types.xsd"/><xs:element name="IODevice" type="i:DeviceT"/></xs:schema>"#;
const TYPES: &str = r#"<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="http://www.io-link.com/IODD/2010/10" elementFormDefault="qualified"><xs:complexType name="DeviceT"><xs:sequence><xs:any minOccurs="0" maxOccurs="unbounded" processContents="skip"/></xs:sequence><xs:attribute name="validation_marker" type="xs:integer" use="required"/></xs:complexType></xs:schema>"#;

struct Fixture {
    _directory: tempfile::TempDir,
    root: PathBuf,
    xml: PathBuf,
}
impl Fixture {
    fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("schema '$ local");
        fs::create_dir(&path)?;
        fs::write(path.join("fixture.xsd"), ROOT)?;
        fs::write(path.join("types.xsd"), TYPES)?;
        let xml = path.join("sensor.xml");
        fs::write(
            &xml,
            SENSOR.replacen("<IODevice ", "<IODevice validation_marker=\"7\" ", 1),
        )?;
        Ok(Self {
            _directory: directory,
            root: path.join("fixture.xsd"),
            xml,
        })
    }
}

#[test]
fn checked_import_persists_exact_schema_closure_and_survives_profile_reload() -> Result<()> {
    let fixture = Fixture::new()?;
    let catalog = iodd::load_checked(&fixture.xml, None, None, Some(&fixture.root))?;
    let receipt = catalog.schema_validation.as_ref().unwrap();
    assert_eq!(receipt.schemas.len(), 2);
    assert_eq!(receipt.root_schema, "fixture.xsd");
    assert_eq!(receipt.schemas[0].name, "fixture.xsd");
    receipt.validate_receipt()?;
    let (expected_identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    let restored = Profile::parse(&toml::to_string_pretty(&profile)?)?;
    assert_eq!(profile, restored);
    assert!(iodd::load(&fixture.xml, None)?.schema_validation.is_none());
    Ok(())
}

#[test]
fn schema_failures_include_locations_and_prevent_import() -> Result<()> {
    let fixture = Fixture::new()?;
    let xml = fs::read_to_string(&fixture.xml)?
        .replace("validation_marker=\"7\"", "validation_marker=\"invalid\"");
    fs::write(&fixture.xml, xml)?;
    let report = iodd::check_schema(&fixture.xml, None, &fixture.root)?;
    assert!(!report.passed);
    assert!(report.diagnostics[0].line > 0);
    assert!(report.diagnostics[0].message.contains("validation_marker"));
    assert!(
        iodd::load_checked(&fixture.xml, None, None, Some(&fixture.root))
            .unwrap_err()
            .to_string()
            .contains("XSD validation failed")
    );
    Ok(())
}

#[test]
fn remote_and_escaping_schema_references_are_rejected() -> Result<()> {
    let fixture = Fixture::new()?;
    for (location, expected) in [
        ("http://127.0.0.1:9/remote.xsd", "Only local schema"),
        ("../outside.xsd", "escapes"),
    ] {
        fs::write(&fixture.root, ROOT.replace("types.xsd", location))?;
        let report = iodd::check_schema(&fixture.xml, None, &fixture.root)?;
        assert!(!report.passed);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|item| item.message.contains(expected))
        );
    }
    let report = iodd::check_schema(
        &fixture.xml,
        None,
        Path::new(r"\\127.0.0.1\must-not-open\schema.xsd"),
    )?;
    assert!(!report.passed);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|item| item.message.contains("Root schema must be a local file"))
    );
    Ok(())
}

#[test]
fn dtds_are_rejected_and_document_schema_hints_are_ignored() -> Result<()> {
    let fixture = Fixture::new()?;
    let xml = fs::read_to_string(&fixture.xml)?;
    let hinted = xml.replace("validation_marker=\"7\"", "validation_marker=\"7\" xsi:schemaLocation=\"http://www.io-link.com/IODD/2010/10 http://127.0.0.1:9/never.xsd\"");
    // The synthetic schema permits this attribute but cannot fetch its location.
    fs::write(
        fixture.root.with_file_name("types.xsd"),
        TYPES.replace(
            "</xs:complexType>",
            "<xs:anyAttribute processContents=\"skip\"/></xs:complexType>",
        ),
    )?;
    fs::write(&fixture.xml, hinted)?;
    assert!(iodd::check_schema(&fixture.xml, None, &fixture.root)?.passed);
    fs::write(
        &fixture.xml,
        xml.replacen(
            "?>",
            "?><!DOCTYPE IODevice [<!ENTITY hidden SYSTEM 'file:///not-read'>]>",
            1,
        ),
    )?;
    let report = iodd::check_schema(&fixture.xml, None, &fixture.root)?;
    assert!(!report.passed);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|item| item.message.contains("DTD"))
    );
    Ok(())
}

#[test]
fn helper_timeout_is_bounded_and_stops_the_child() -> Result<()> {
    let fixture = Fixture::new()?;
    let error = validate_with_timeout(
        &fs::read(&fixture.xml)?,
        &fixture.root,
        Duration::from_millis(1),
    )
    .unwrap_err();
    assert!(error.to_string().contains("exceeded"));
    Ok(())
}
