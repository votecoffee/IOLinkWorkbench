use super::*;
use crate::{iodd, profile::Profile};

const MAIN: &str = include_str!("../../../samples/sensor-checksum.xml");
const LANGUAGE: &str = include_str!("../../../samples/sensor-checksum-de.xml");

fn companion(xml: &str) -> Companion {
    Companion {
        xml: xml.into(),
        bytes: xml.as_bytes().to_vec(),
        source: "synthetic-de.xml".into(),
    }
}

#[test]
fn independent_zlib_vectors_match_main_and_bound_language_bytes() -> Result<()> {
    // Values calculated independently with Python zlib, not this implementation.
    let report = check(
        MAIN.as_bytes(),
        "synthetic.xml".into(),
        Some(&companion(LANGUAGE)),
    )?;
    assert_eq!(report.main.calculated_crc, Some(2556892730));
    let language = report.language.as_ref().unwrap();
    assert_eq!(language.calculated_crc, Some(3967000593));
    assert_eq!(language.main_crc, Some(2556892730));
    assert!(report.matched());
    report.enforce(true)?;
    // Language CRCs append decimal ASCII even when the document itself is UTF-16.
    for (little, expected) in [(true, 3214440319u32), (false, 2753261195)] {
        let xml = LANGUAGE
            .replace("3967000593", &expected.to_string())
            .replace("encoding=\"UTF-8\"", "encoding=\"UTF-16\"");
        let mut bytes = if little {
            vec![0xFF, 0xFE]
        } else {
            vec![0xFE, 0xFF]
        };
        for unit in xml.encode_utf16() {
            bytes.extend_from_slice(&if little {
                unit.to_le_bytes()
            } else {
                unit.to_be_bytes()
            });
        }
        let language = inspect(&bytes, "UTF-16 language.xml".into(), true, Some(2556892730))?;
        assert_eq!(language.status, Status::Matched);
        assert_eq!(language.calculated_crc, Some(expected));
    }
    Ok(())
}

#[test]
fn original_bom_endings_utf16_and_unicode_offsets_are_preserved() -> Result<()> {
    for (encoding, crc) in [
        (Encoding::Utf8, 3189494275u32),
        (Encoding::Utf16Le, 33993713),
        (Encoding::Utf16Be, 34650337),
    ] {
        let xml = MAIN.replace("2556892730", &crc.to_string());
        let mut bytes = match encoding {
            Encoding::Utf8 => vec![0xEF, 0xBB, 0xBF],
            Encoding::Utf16Le => vec![0xFF, 0xFE],
            Encoding::Utf16Be => vec![0xFE, 0xFF],
        };
        if encoding == Encoding::Utf8 {
            bytes.extend_from_slice(xml.as_bytes());
        } else {
            for unit in xml
                .replace("encoding=\"UTF-8\"", "encoding=\"UTF-16\"")
                .encode_utf16()
            {
                bytes.extend_from_slice(&if encoding == Encoding::Utf16Le {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
        }
        let report = check(&bytes, "original encoding.xml".into(), None)?;
        assert_eq!(report.main.encoding, encoding);
        assert_eq!(report.main.calculated_crc, Some(crc));
        assert_eq!(report.main.bytes, bytes.len() as u64);
        assert!(report.matched());
    }
    let crlf = MAIN
        .replace("2556892730", "169324514")
        .replace('\n', "\r\n");
    assert!(check(crlf.as_bytes(), "CRLF.xml".into(), None)?.matched());
    // Converting line endings without recomputing the stamp must fail.
    let changed = MAIN.replace('\n', "\r\n");
    assert_eq!(
        check(changed.as_bytes(), "changed.xml".into(), None)?
            .main
            .status,
        Status::Mismatch
    );
    Ok(())
}

/// Generate deliberately unusual test serializations; never applied to vendor input.
fn synthetic(xml: &str, main_crc: Option<u32>) -> String {
    let empty = xml.replace("__CRC__", "");
    let mut crc = crc32fast::Hasher::new();
    crc.update(empty.as_bytes());
    if let Some(main) = main_crc {
        crc.update(main.to_string().as_bytes());
    }
    xml.replace("__CRC__", &crc.finalize().to_string())
}

#[test]
fn actual_xml_stamp_is_used_instead_of_comment_decoys() -> Result<()> {
    let xml = MAIN
        .replace("2556892730", "__CRC__")
        .replace(
            "<ProfileBody>",
            "<!-- <Stamp crc=\"123\"><Checker/></Stamp> -->\n<ProfileBody>",
        )
        .replace(
            "<Stamp crc=\"__CRC__\">",
            "<Stamp other=\"test\" crc = '__CRC__'>",
        );
    let signed = synthetic(&xml, None);
    assert!(check(signed.as_bytes(), "comment decoy.xml".into(), None)?.matched());
    let duplicate = signed.replace("</IODevice>", "<Stamp crc=\"1\"/></IODevice>");
    assert_eq!(
        check(duplicate.as_bytes(), "duplicate.xml".into(), None)?
            .main
            .status,
        Status::Malformed
    );
    let nested = signed
        .replace("<Stamp other=", "<Nested><Stamp other=")
        .replace("</Stamp>", "</Stamp></Nested>");
    assert_eq!(
        check(nested.as_bytes(), "nested.xml".into(), None)?
            .main
            .status,
        Status::Malformed
    );
    Ok(())
}

#[test]
fn missing_and_malformed_stamps_do_not_claim_a_matching_checksum() -> Result<()> {
    let no_stamp = include_str!("../../../samples/sensor.xml");
    let report = check(no_stamp.as_bytes(), "no stamp.xml".into(), None)?;
    assert_eq!(report.main.status, Status::Missing);
    report.enforce(false)?;
    assert!(report.enforce(true).is_err());
    for xml in [
        MAIN.replace("crc=\"2556892730\"", "other=\"1\""),
        MAIN.replace("2556892730", "4294967296"),
        MAIN.replace("2556892730", ""),
        MAIN.replace("name=\"Workbench synthetic CRC test fixture\"", "name=\"\""),
        MAIN.replace("V0.0.0.1", "unknown"),
    ] {
        let report = check(xml.as_bytes(), "malformed.xml".into(), None)?;
        assert_eq!(report.main.status, Status::Malformed);
        assert!(report.enforce(false).is_err());
    }
    Ok(())
}

#[test]
fn language_checksums_require_the_verified_main_crc() -> Result<()> {
    let main = MAIN
        .replace("2556892730", "__CRC__")
        .replace("Threshold", "New threshold");
    let main = synthetic(&main, None);
    let wrong_binding = check(
        main.as_bytes(),
        "changed main.xml".into(),
        Some(&companion(LANGUAGE)),
    )?;
    assert_eq!(wrong_binding.main.status, Status::Matched);
    assert_eq!(wrong_binding.language.unwrap().status, Status::Mismatch);
    let missing_main = include_str!("../../../samples/sensor.xml");
    let unbound = check(
        missing_main.as_bytes(),
        "no stamp.xml".into(),
        Some(&companion(LANGUAGE)),
    )?;
    assert_eq!(unbound.language.as_ref().unwrap().status, Status::Unbound);
    unbound.enforce(false)?;
    assert!(unbound.enforce(true).is_err());
    let missing_language = LANGUAGE
        .lines()
        .filter(|line| !line.contains("<Stamp"))
        .collect::<Vec<_>>()
        .join("\n");
    let report = check(
        MAIN.as_bytes(),
        "main.xml".into(),
        Some(&companion(&missing_language)),
    )?;
    assert_eq!(report.language.as_ref().unwrap().status, Status::Missing);
    assert!(!report.matched());
    report.enforce(false)?;
    assert!(report.enforce(true).is_err());
    Ok(())
}

#[test]
fn profiles_and_snapshots_keep_receipts_without_reopening_sources() -> Result<()> {
    let catalog = iodd::load_with_options(
        std::path::Path::new("samples/sensor-checksum.xml"),
        &iodd::LoadOptions {
            language: Some("de"),
            require_stamp: true,
            ..Default::default()
        },
    )?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    assert_eq!(imported.parameters[0].name, "Schwellwert");
    let mut profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    profile.iodd.as_mut().unwrap().source = "nonexistent-original.xml".into();
    let saved = toml::to_string_pretty(&profile)?;
    let restored = Profile::parse(&saved)?;
    assert_eq!(profile, restored);
    let (snapshot, _) = crate::parameter_snapshot::read(&restored, "V_SIM_Threshold", 0)?;
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("parameter.json");
    snapshot.save(&path)?;
    let bytes = std::fs::read(&path)?;
    let review = crate::parameter_snapshot::Snapshot::load(&path)?.review()?;
    assert_eq!(review.decoded.raw_value.as_deref(), Some("500"));
    assert!(review.iodd_stamp.as_ref().unwrap().matched());
    assert_eq!(bytes, std::fs::read(&path)?);
    Ok(())
}

#[test]
fn edited_saved_receipts_are_rejected_and_legacy_profiles_remain_loadable() -> Result<()> {
    let report = check(
        MAIN.as_bytes(),
        "main.xml".into(),
        Some(&companion(LANGUAGE)),
    )?;
    let mut bad = report.clone();
    bad.main.declared_crc = Some(1);
    assert!(bad.validate_receipt().is_err());
    let mut bad = report.clone();
    bad.language.as_mut().unwrap().main_crc = Some(1);
    assert!(bad.validate_receipt().is_err());
    let mut bad = report;
    bad.main.sha256 = "not a hash".into();
    assert!(bad.validate_receipt().is_err());
    let catalog = iodd::parse(
        include_str!("../../../samples/sensor.xml"),
        "in-memory input".into(),
    )?;
    assert!(catalog.stamp_validation.is_none());
    let (expected_identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    assert_eq!(Profile::parse(&toml::to_string_pretty(&profile)?)?, profile);
    Ok(())
}

#[test]
fn file_loading_checks_before_accepting_a_mapping_and_never_rewrites_bytes() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("main.xml");
    let bytes = MAIN.replace("Threshold", "Changed text").into_bytes();
    std::fs::write(&path, &bytes)?;
    assert!(iodd::load(&path, None).is_err());
    assert_eq!(std::fs::read(&path)?, bytes);
    std::fs::write(&path, include_bytes!("../../../samples/sensor.xml"))?;
    assert!(iodd::load(&path, None).is_ok());
    assert!(
        iodd::load_with_options(
            &path,
            &iodd::LoadOptions {
                require_stamp: true,
                ..Default::default()
            }
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn saved_xsd_and_stamp_receipts_must_describe_the_same_original_bytes() -> Result<()> {
    let catalog = iodd::load(std::path::Path::new("samples/sensor-checksum.xml"), None)?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    let mut profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    let imported = profile.iodd.as_mut().unwrap();
    imported.schema_validation = Some(iodd::schema::Report {
        passed: true,
        engine: "synthetic receipt test".into(),
        runtime: "offline".into(),
        xml_sha256: "0".repeat(64),
        root_schema: "test.xsd".into(),
        schemas: vec![iodd::schema::SchemaFile {
            name: "test.xsd".into(),
            sha256: "1".repeat(64),
        }],
        diagnostics: Vec::new(),
        diagnostics_truncated: false,
    });
    assert!(profile.validate().is_err());
    let imported = profile.iodd.as_mut().unwrap();
    imported.schema_validation.as_mut().unwrap().xml_sha256 = imported
        .stamp_validation
        .as_ref()
        .unwrap()
        .main
        .sha256
        .clone();
    profile.validate()?;
    Ok(())
}
