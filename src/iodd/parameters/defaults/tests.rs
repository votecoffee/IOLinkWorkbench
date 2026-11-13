use super::*;
use crate::{
    archive,
    capture::{self, Options, Stop},
    iodd, parameter_snapshot,
};
use std::{path::Path, time::Duration};

const XML: &str = include_str!("../../../../samples/sensor-defaults.xml");

fn profile() -> Result<Profile> {
    let catalog = iodd::parse(XML, "original defaults fixture".into())?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    Ok(Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    })
}

#[test]
fn temporal_defaults_share_scalar_member_array_and_snapshot_views() -> Result<()> {
    let catalog = iodd::parse(
        include_str!("../../../../samples/sensor-time.xml"),
        "synthetic temporal defaults".into(),
    )?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    let profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    let before = toml::to_string_pretty(&profile)?;
    let all = inspect_profile(&profile, None, None)?;
    assert_eq!(
        (
            all.decoded,
            all.not_declared,
            all.needs_review,
            all.unsupported
        ),
        (6, 0, 0, 0)
    );
    let array = profile.parameter("V_SIM_Times")?;
    let value = array.inspect_default(0)?;
    assert_eq!(value.array_elements, Some(2));
    assert_eq!(
        value.representation_hex.as_deref(),
        Some("0000000000000000")
    );
    let member = array.inspect_default(2)?;
    assert!(member.array_elements.is_none());
    assert_eq!(member.representation_hex, value.representation_hex);
    let (snapshot, _) = parameter_snapshot::read(&profile, "V_SIM_TimeRecord", 1)?;
    let review = snapshot.review()?;
    assert_eq!(
        review.decoded.time_value.unwrap().value,
        "1984-01-01 00:00:00.500 UTC"
    );
    assert_eq!(
        review
            .declared_default
            .decoded
            .unwrap()
            .time_value
            .unwrap()
            .value,
        "1984-01-01 00:00:00.001 UTC"
    );
    assert!(!review.declared_default.temporal_conversion.unwrap().exact);
    let mut invalid = profile.parameter("V_SIM_Time")?.clone();
    invalid.default_value = Some("1900-01-01".into());
    let value = invalid.inspect_default(0)?;
    assert_eq!(value.status, Status::NeedsReview);
    assert!(value.representation_hex.is_none());
    assert_eq!(value.declared.as_deref(), Some("1900-01-01"));
    assert_eq!(toml::to_string_pretty(&profile)?, before);
    Ok(())
}

#[test]
fn numeric_defaults_preserve_exact_values_and_use_the_existing_display_decoder() -> Result<()> {
    let profile = profile()?;
    for (id, bytes, raw) in [
        ("V_DEFAULT_Bool", "01", "true"),
        ("V_DEFAULT_Wide", "FFFFFFFFFFFFFFFF", "18446744073709551615"),
        ("V_DEFAULT_Signed", "FFEF", "-17"),
        ("V_DEFAULT_Float", "3FA00000", "1.25"),
    ] {
        let value = profile.parameter(id)?.inspect_default(0)?;
        assert_eq!(value.status, Status::Decoded);
        assert_eq!(value.representation_hex.as_deref(), Some(bytes));
        assert_eq!(value.decoded.unwrap().raw_value.as_deref(), Some(raw));
    }
    let value = profile.parameter("V_DEFAULT_Float")?.inspect_default(0)?;
    assert_eq!(value.decoded.unwrap().displays[0].value, "12.5");
    Ok(())
}

#[test]
fn text_and_octet_defaults_preserve_whitespace_unicode_zeros_and_order() -> Result<()> {
    let profile = profile()?;
    let value = profile.parameter("V_DEFAULT_Text")?.inspect_default(0)?;
    assert_eq!(
        value.representation_hex.as_deref(),
        Some("20C2B56D20000000")
    );
    assert_eq!(value.decoded.unwrap().raw_value.as_deref(), Some(" µm "));
    let value = profile.parameter("V_DEFAULT_Octets")?.inspect_default(0)?;
    assert_eq!(value.representation_hex.as_deref(), Some("0080FF2C"));
    assert_eq!(
        value.decoded.unwrap().raw_value.as_deref(),
        Some("0x00,0x80,0xFF,0x2C")
    );
    let mut text = profile.parameter("V_DEFAULT_Text")?.clone();
    text.default_value = Some("".into());
    let empty = text.inspect_default(0)?;
    assert_eq!(empty.status, Status::Decoded);
    assert_eq!(
        empty.representation_hex.as_deref(),
        Some("0000000000000000")
    );
    for bad in ["123456789", "A\0B"] {
        text.default_value = Some(bad.into());
        assert_eq!(text.inspect_default(0)?.status, Status::NeedsReview);
    }
    text.default_value = Some("µm".into());
    text.encoding = Some("US-ASCII".into());
    assert_eq!(text.inspect_default(0)?.status, Status::NeedsReview);
    let mut octets = profile.parameter("V_DEFAULT_Octets")?.clone();
    for bad in ["0x00,0x80,0xFF", "00,80,FF,2C", "0xGG,0x80,0xFF,0x2C"] {
        octets.default_value = Some(bad.into());
        assert_eq!(octets.inspect_default(0)?.status, Status::NeedsReview);
    }
    Ok(())
}

#[test]
fn invalid_outside_unsupported_and_absent_defaults_keep_distinct_outcomes() -> Result<()> {
    let profile = profile()?;
    let invalid = profile.parameter("V_DEFAULT_Invalid")?.inspect_default(0)?;
    assert_eq!(invalid.status, Status::NeedsReview);
    assert_eq!(invalid.declared.as_deref(), Some("256"));
    assert!(invalid.representation_hex.is_none());
    let outside = profile.parameter("V_DEFAULT_Outside")?.inspect_default(0)?;
    assert_eq!(outside.status, Status::NeedsReview);
    assert_eq!(
        outside.decoded.unwrap().value_assessment.unwrap().status,
        AssessmentStatus::OutsideDefinition
    );
    let time = profile.parameter("V_DEFAULT_Time")?.inspect_default(0)?;
    assert_eq!(time.status, Status::Decoded);
    assert_eq!(time.declared.as_deref(), Some("-PT2S"));
    assert_eq!(time.representation_hex.as_deref(), Some("FFFFFFFE00000000"));
    assert_eq!(
        profile
            .parameter("V_DEFAULT_NaN")?
            .inspect_default(0)?
            .status,
        Status::Unsupported
    );
    assert_eq!(
        profile
            .parameter("V_DEFAULT_Absent")?
            .inspect_default(0)?
            .status,
        Status::NotDeclared
    );
    let mut number = profile.parameter("V_DEFAULT_Signed")?.clone();
    for width in [0, 65] {
        number.bits = Some(width);
        assert_eq!(number.inspect_default(0)?.status, Status::NeedsReview);
    }
    let mut float = profile.parameter("V_DEFAULT_Float")?.clone();
    float.default_value = Some("NaN".into());
    assert_eq!(float.inspect_default(0)?.status, Status::Unsupported);
    float.default_value = Some("INF".into());
    assert_eq!(
        float.inspect_default(0)?.representation_hex.as_deref(),
        Some("7F800000")
    );
    Ok(())
}

#[test]
fn arrays_inspect_one_element_and_flag_inconsistent_saved_defaults() -> Result<()> {
    let profile = profile()?;
    let mut array = profile.parameter("V_DEFAULT_Array")?.clone();
    let whole = array.inspect_default(0)?;
    assert_eq!(whole.status, Status::Decoded);
    assert_eq!(whole.array_elements, Some(3));
    assert_eq!(whole.representation_hex.as_deref(), Some("FF"));
    let selected = array.inspect_default(2)?;
    assert_eq!(selected.array_elements, None);
    assert_eq!(selected.decoded.unwrap().raw_value.as_deref(), Some("-1"));
    array.array.as_mut().unwrap().element.default_value = None;
    assert_eq!(array.inspect_default(0)?.status, Status::Decoded);
    array.array.as_mut().unwrap().element.default_value = Some("-01".into());
    assert_eq!(array.inspect_default(0)?.status, Status::Decoded);
    array.array.as_mut().unwrap().element.default_value = Some("2".into());
    assert_eq!(array.inspect_default(0)?.status, Status::NeedsReview);
    array.default_value = None;
    assert_eq!(array.inspect_default(0)?.status, Status::NeedsReview);
    assert!(array.inspect_default(4).is_err());
    Ok(())
}

#[test]
fn record_defaults_are_member_views_without_constructing_reserved_or_missing_bits() -> Result<()> {
    let profile = profile()?;
    let report = inspect_profile(&profile, Some("V_DEFAULT_Record"), None)?;
    assert_eq!(report.entries.len(), 4);
    assert_eq!(report.decoded, 3);
    assert_eq!(report.not_declared, 1);
    assert_eq!(report.entries[0].bit_offset, Some(56));
    assert_eq!(
        report.entries[1].value.representation_hex.as_deref(),
        Some("002A")
    );
    assert_eq!(
        report.entries[2].value.representation_hex.as_deref(),
        Some("4F4B0000")
    );
    let mut record = profile.parameter("V_DEFAULT_Record")?.clone();
    assert_eq!(record.inspect_default(0)?.status, Status::NotDeclared);
    record.default_value = Some("uninterpreted whole-record declaration".into());
    assert_eq!(record.inspect_default(0)?.status, Status::Unsupported);
    Ok(())
}

#[test]
fn catalog_selection_and_write_only_inspection_are_metadata_only() -> Result<()> {
    let mut profile = profile()?;
    profile.connection.gateway = crate::profile::Gateway::IfmAl1350;
    profile.connection.endpoint = "http://127.0.0.1:9".into();
    let report = inspect_profile(&profile, None, None)?;
    assert_eq!(
        (
            report.entries.len(),
            report.decoded,
            report.not_declared,
            report.needs_review,
            report.unsupported
        ),
        (17, 12, 2, 2, 1)
    );
    let command = inspect_profile(&profile, Some("V_DEFAULT_Command"), None)?;
    assert_eq!(command.entries[0].access, Access::WriteOnly);
    assert_eq!(
        command.entries[0].value.representation_hex.as_deref(),
        Some("80")
    );
    assert!(inspect_profile(&profile, None, Some(1)).is_err());
    assert!(inspect_profile(&profile, Some("missing"), None).is_err());
    assert!(inspect_profile(&profile, Some("V_DEFAULT_Bool"), Some(1)).is_err());
    profile
        .iodd
        .as_mut()
        .unwrap()
        .standard_parameter_refs
        .push("V_Unresolved".into());
    assert_eq!(
        inspect_profile(&profile, None, None)?.unresolved_standard_parameter_refs,
        ["V_Unresolved"]
    );
    assert!(
        inspect_profile(&profile, Some("V_DEFAULT_Bool"), None)?
            .unresolved_standard_parameter_refs
            .is_empty()
    );
    Ok(())
}

#[test]
fn inspections_preserve_saved_profiles_snapshots_and_independent_cyclic_capture() -> Result<()> {
    let catalog = iodd::load_with_options(
        Path::new("samples/sensor-standard-values.xml"),
        &iodd::LoadOptions {
            definitions: Some(Path::new(
                "samples/synthetic-value-standard-definitions.xml",
            )),
            ..Default::default()
        },
    )?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    let profile_with_default = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    let (mut snapshot, _) = parameter_snapshot::read(&profile_with_default, "V_OffsetTime", 0)?;
    snapshot
        .profile
        .iodd
        .as_mut()
        .unwrap()
        .parameters
        .iter_mut()
        .find(|parameter| parameter.id == "V_OffsetTime")
        .unwrap()
        .default_value = Some("41".into());
    let review = snapshot.review()?;
    assert_eq!(review.decoded.raw_value.as_deref(), Some("42"));
    assert_eq!(review.declared_default.declared.as_deref(), Some("41"));
    assert_eq!(
        review
            .declared_default
            .decoded
            .unwrap()
            .raw_value
            .as_deref(),
        Some("41")
    );
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("parameter.json");
    snapshot.save(&path)?;
    let before = std::fs::read(&path)?;
    let restored = parameter_snapshot::Snapshot::load(&path)?;
    assert_eq!(
        inspect_profile(&restored.profile, Some("V_OffsetTime"), None)?.decoded,
        1
    );
    assert_eq!(before, std::fs::read(path)?);
    let profile = profile()?;
    let text = toml::to_string_pretty(&profile)?;
    inspect_profile(&profile, None, None)?;
    assert_eq!(text, toml::to_string_pretty(&profile)?);
    let output = folder.path().join("capture");
    let report = capture::run(
        &profile,
        &Options {
            output: Some(output.clone()),
            duration: Some(Duration::from_millis(100)),
            ..Default::default()
        },
        &Stop::default(),
        None,
    )?;
    assert!(report.success());
    assert!(archive::verify(&output, None)?.rows_checked > 0);
    Ok(())
}
