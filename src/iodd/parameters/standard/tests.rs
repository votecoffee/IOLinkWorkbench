use super::*;
use crate::{
    iodd, parameter_snapshot,
    profile::{Gateway, Profile},
    transport,
};

const XML: &str = include_str!("../../../../samples/sensor-standard-values.xml");
const DEFINITIONS: &str = "samples/synthetic-value-standard-definitions.xml";

fn catalog(xml: &str) -> Result<iodd::Catalog> {
    let definitions = load(Path::new(DEFINITIONS), None, None)?;
    iodd::parse_with_definitions(
        xml,
        "synthetic value overrides".into(),
        &definitions.parameters,
    )
}

fn profile(language: &str) -> Result<Profile> {
    let catalog = iodd::load_with_options(
        Path::new("samples/sensor-standard-values.xml"),
        &iodd::LoadOptions {
            language: Some(language),
            definitions: Some(Path::new(DEFINITIONS)),
            ..Default::default()
        },
    )?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    Ok(Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    })
}

#[test]
fn named_ranges_are_localized_and_assessed_before_display_scaling() -> Result<()> {
    for (language, band, zero, measurement) in [
        ("en", "Operating band", "Zero offset", "Measurement range"),
        ("de", "Arbeitsbereich", "Nullversatz", "Messbereich"),
    ] {
        let profile = profile(language)?;
        let offset = profile.parameter("V_OffsetTime")?;
        assert_eq!(offset.ranges[0].lower, "+020");
        assert_eq!(offset.ranges[0].name.as_deref(), Some(band));
        let value = offset.decode(&[42]);
        assert_eq!(value.raw_value.as_deref(), Some("42"));
        assert_eq!(value.displays[0].value, "21");
        assert_eq!(value.value_assessment.unwrap().matched_range_names, [band]);
        assert_eq!(offset.decode(&[0]).choice_name.as_deref(), Some(zero));
        let inline = profile
            .parameter("V_SIM_Threshold")?
            .decode(&500i16.to_be_bytes());
        assert_eq!(
            inline.value_assessment.unwrap().matched_range_names,
            [measurement]
        );
        assert_eq!(Profile::parse(&toml::to_string_pretty(&profile)?)?, profile);
    }
    Ok(())
}

#[test]
fn ranges_cannot_redefine_reserved_values_even_without_standard_value_references() -> Result<()> {
    for (lower, upper) in [("128", "129"), ("120", "128"), ("127", "129")] {
        let xml = XML.replace(
            "lowerValue=\"200\" upperValue=\"210\"",
            &format!("lowerValue=\"{lower}\" upperValue=\"{upper}\""),
        );
        for candidate in [
            xml.clone(),
            xml.replace("<StdSingleValueRef value=\"128\"/>", ""),
        ] {
            assert!(
                catalog(&candidate)
                    .unwrap_err()
                    .to_string()
                    .contains("standard choice 128")
            );
        }
    }
    let xml = XML.replace("lowerValue=\"+020\"", "lowerValue=\"0\"");
    assert!(catalog(&xml).is_err());
    assert!(catalog(&xml.replace("<StdSingleValueRef value=\"+00\"/>", "")).is_err());
    // The same rule applies recursively to a direct-parameter command member.
    let xml = XML;
    let at = xml.find("<StdRecordItemRef").unwrap();
    let xml = format!(
        "{}{}",
        &xml[..at],
        xml[at..].replace("lowerValue=\"200\"", "lowerValue=\"128\"")
    );
    assert!(catalog(&xml).is_err());
    Ok(())
}

#[test]
fn ambiguous_out_of_domain_and_unknown_override_directives_reject_import() -> Result<()> {
    let variants = [
        XML.replace(
            "<StdSingleValueRef value=\"+00\"/>",
            "<StdSingleValueRef value=\"+00\"/><StdSingleValueRef value=\"0\"/>",
        ),
        XML.replace("lowerValue=\"+020\"", "lowerValue=\"8\""),
        XML.replace("upperValue=\"+050\"", "upperValue=\"20\""),
        XML.replace("upperValue=\"+050\"", "upperValue=\"19\""),
        XML.replace("upperValue=\"+050\"", "upperValue=\"256\""),
        XML.replace("lowerValue=\"+020\"", "lowerValue=\"-1\""),
        XML.replace("value=\"+010\"", "value=\"NaN\""),
        XML.replace(
            "<StdSingleValueRef value=\"+00\"/>",
            "<StdValueRangeRef lowerValue=\"1\" upperValue=\"255\"/>",
        ),
        XML.replace(
            "</StdVariableRef>",
            "<ValueRange lowerValue=\"50\" upperValue=\"60\"/></StdVariableRef>",
        ),
    ];
    for xml in variants {
        assert!(catalog(&xml).is_err());
    }
    Ok(())
}

#[test]
fn no_override_keeps_the_base_named_range_and_unnamed_legacy_ranges_load() -> Result<()> {
    let start = XML.find("<StdVariableRef id=\"V_OffsetTime\"").unwrap();
    let end = start + XML[start..].find("</StdVariableRef>").unwrap() + "</StdVariableRef>".len();
    let mut xml = XML.to_owned();
    xml.replace_range(start..end, "<StdVariableRef id=\"V_OffsetTime\"/>");
    let catalog = catalog(&xml)?;
    let offset = catalog
        .parameters
        .iter()
        .find(|p| p.id == "V_OffsetTime")
        .unwrap();
    assert_eq!(
        offset.ranges[0].name.as_deref(),
        Some("Standard nonzero values")
    );
    let mut legacy = profile("en")?;
    legacy
        .iodd
        .as_mut()
        .unwrap()
        .parameters
        .iter_mut()
        .for_each(|parameter| {
            for range in &mut parameter.ranges {
                range.name = None;
            }
        });
    assert_eq!(Profile::parse(&toml::to_string_pretty(&legacy)?)?, legacy);
    Ok(())
}

#[test]
fn range_redefinition_checks_keep_exact_wide_and_binary32_semantics() -> Result<()> {
    let profile = profile("en")?;
    let mut definition = profile.parameter("V_OffsetTime")?.clone();
    definition.displays.clear();
    definition.preferred_display = None;
    definition.bits = Some(64);
    definition.choices = vec![Choice {
        value: "9007199254740993".into(),
        name: "Reserved exact value".into(),
    }];
    for (range, accepted) in [
        ("9007199254740992\" upperValue=\"9007199254740993", false),
        ("9007199254740994\" upperValue=\"18446744073709551615", true),
    ] {
        let xml = format!(
            "<StdVariableRef xmlns=\"{IODD_NAMESPACE}\"><ValueRange lowerValue=\"{range}\"/></StdVariableRef>"
        );
        let doc = Document::parse(&xml)?;
        assert_eq!(
            resolve(&definition, doc.root_element(), &HashMap::new()).is_ok(),
            accepted
        );
    }
    definition.datatype = "Float32T".into();
    definition.bits = Some(32);
    definition.choices[0].value = "1.250e0".into();
    for (upper, accepted) in [("1.25", false), ("1.2", true), ("NaN", false)] {
        let xml = format!(
            "<StdVariableRef xmlns=\"{IODD_NAMESPACE}\"><ValueRange lowerValue=\"1.0\" upperValue=\"{upper}\"/></StdVariableRef>"
        );
        let doc = Document::parse(&xml)?;
        assert_eq!(
            resolve(&definition, doc.root_element(), &HashMap::new()).is_ok(),
            accepted
        );
    }
    Ok(())
}

#[test]
fn snapshot_labels_survive_offline_review_and_commands_remain_write_only() -> Result<()> {
    let mut profile = profile("de")?;
    let (snapshot, reading) = parameter_snapshot::read(&profile, "V_OffsetTime", 0)?;
    assert_eq!(
        reading.value_assessment.unwrap().matched_range_names,
        ["Arbeitsbereich"]
    );
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("parameter.json");
    snapshot.save(&path)?;
    let before = std::fs::read(&path)?;
    let review = parameter_snapshot::Snapshot::load(&path)?.review()?;
    assert_eq!(
        review.decoded.value_assessment.unwrap().matched_range_names,
        ["Arbeitsbereich"]
    );
    assert_eq!(before, std::fs::read(path)?);
    profile.connection.gateway = Gateway::IfmAl1350;
    profile.connection.endpoint = "must not open".into();
    for (id, subindex) in [("V_SystemCommand", 0), ("V_DirectParameters_1", 16)] {
        assert!(
            transport::read_parameter_at(&profile, id, subindex)
                .unwrap_err()
                .to_string()
                .contains("write-only")
        );
    }
    Ok(())
}

#[test]
fn array_and_record_members_keep_range_names_on_the_existing_raw_decoder() -> Result<()> {
    let xml = include_str!("../../../../samples/sensor-values.xml")
        .replace("<ValueRange lowerValue=\"+00\" upperValue=\"+07\"/>", "<ValueRange lowerValue=\"+00\" upperValue=\"+07\"><Name textId=\"array\"/></ValueRange>")
        .replace("<ValueRange lowerValue=\"+0\" upperValue=\"+100\"/>", "<ValueRange lowerValue=\"+0\" upperValue=\"+100\"><Name textId=\"signed\"/></ValueRange>");
    let catalog = iodd::parse(&xml, "original named member ranges".into())?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    let profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    let array = transport::read_parameter(&profile, "V_SIM_ValueArray")?;
    let member = transport::read_parameter_at(&profile, "V_SIM_ValueArray", 2)?;
    assert_eq!(
        array.array_items[1]
            .value
            .value_assessment
            .as_ref()
            .unwrap()
            .matched_range_names,
        ["Packed values"]
    );
    assert_eq!(
        member.value_assessment.unwrap().matched_range_names,
        ["Packed values"]
    );
    let record = profile
        .parameter("V_SIM_ValueRecord")?
        .decode(&[1, 0, 42, 3]);
    assert_eq!(
        record.record_items[1].value.raw_value.as_deref(),
        Some("42")
    );
    assert_eq!(
        record.record_items[1]
            .value
            .value_assessment
            .as_ref()
            .unwrap()
            .matched_range_names,
        ["Signed value"]
    );
    Ok(())
}
