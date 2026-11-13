use super::*;
use crate::{iodd, profile::Profile};
use std::{fs, path::Path};

const XML: &str = include_str!("../../../../samples/sensor-formats.xml");

fn profile(xml: &str) -> Result<Profile> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("formats.xml");
    fs::write(&path, xml)?;
    let catalog = iodd::load(&path, None)?;
    let (expected_identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    profile.validate()?;
    Ok(profile)
}

#[test]
fn scalar_record_and_array_views_follow_declared_formats() -> Result<()> {
    let profile = profile(XML)?;
    let scalar = profile
        .parameter("V_SIM_Threshold")?
        .decode(&500i16.to_be_bytes());
    assert_eq!(scalar.raw_value.as_deref(), Some("500"));
    assert_eq!(
        scalar
            .displays
            .iter()
            .map(|display| display.value.as_str())
            .collect::<Vec<_>>(),
        ["500", "01F4h", "0000 0001 1111 0100b"]
    );
    let record = profile
        .parameter("V_SIM_Record")?
        .decode(&[0xFF, 0xEC, 1, 10, b'T', b'E', b'S', b'T']);
    assert_eq!(
        record.record_items[0].value.raw_value.as_deref(),
        Some("-20")
    );
    assert_eq!(record.record_items[0].value.displays[1].value, "FFECh");
    assert_eq!(
        record.record_items[0].value.displays[2].value,
        "1111 1111 1110 1100b"
    );
    assert_eq!(
        record.record_items[1].value.raw_value.as_deref(),
        Some("true")
    );
    assert_eq!(record.record_items[1].value.displays[0].value, "1");
    assert_eq!(record.record_items[2].value.displays[0].value, "0Ah");
    let array = profile.parameter("V_SIM_History")?;
    let packed = array.decode(&[0x02, 0xC4, 0x95, 0xF0]);
    assert_eq!(packed.array_items[1].value.raw_value.as_deref(), Some("-4"));
    assert_eq!(packed.array_items[1].value.displays[0].value, "FCh");
    assert_eq!(packed.array_items[1].value.displays[1].value, "1111 1100b");
    let member = array.target(2)?.decode(&[0xFC]);
    assert_eq!(
        member.displays[0].value,
        packed.array_items[1].value.displays[0].value
    );
    Ok(())
}

#[test]
fn narrow_signed_values_extend_to_the_next_supported_display_width() -> Result<()> {
    let profile = profile(XML)?;
    let display = &profile.parameter("V_SIM_Threshold")?.displays[1];
    for (bits, bytes, expected) in [
        (1, vec![1], "FFh"),
        (4, vec![15], "FFh"),
        (7, vec![0x40], "C0h"),
        (9, vec![1, 0], "FF00h"),
        (17, vec![1, 0, 0], "FFFF0000h"),
        (24, vec![0x80, 0, 0], "FF800000h"),
        (33, vec![1, 0, 0, 0, 0], "FFFFFFFF00000000h"),
        (64, vec![255; 8], "FFFFFFFFFFFFFFFFh"),
    ] {
        let mut display = display.clone();
        display.field.bits = bits;
        assert_eq!(display.decode(&bytes)?.value, expected);
    }
    Ok(())
}

#[test]
fn unsigned_full_width_and_boolean_decimal_do_not_lose_raw_information() -> Result<()> {
    let profile = profile(XML)?;
    let mut parameter = profile.parameter("V_SIM_Threshold")?.clone();
    parameter.datatype = "UIntegerT".into();
    parameter.bits = Some(64);
    for display in &mut parameter.displays {
        display.field.kind = Kind::Unsigned;
        display.field.bits = 64;
    }
    parameter.validate()?;
    let value = parameter.decode(&[255; 8]);
    assert_eq!(value.raw_value, Some(u64::MAX.to_string()));
    assert_eq!(value.displays[0].value, u64::MAX.to_string());
    assert_eq!(value.displays[1].value, "FFFFFFFFFFFFFFFFh");
    assert_eq!(
        value.displays[2].value,
        format!("{}b", ["1111"; 16].join(" "))
    );
    let boolean = profile.parameter("V_SIM_Record")?.target(2)?;
    assert_eq!(boolean.decode(&[0]).displays[0].value, "0");
    assert_eq!(boolean.decode(&[1]).raw_value.as_deref(), Some("true"));
    Ok(())
}

#[test]
fn illegal_iodd_radix_combinations_are_reviewed_without_losing_raw_values() -> Result<()> {
    for replacement in [
        "displayFormat=\"Hex\" gradient=\"1\"",
        "displayFormat=\"Hex\" offset=\"0\"",
        "displayFormat=\"Hex\" unitCode=\"1997\"",
        "displayFormat=\"Oct\"",
    ] {
        let xml = XML.replace(
            "<VariableRef variableId=\"V_SIM_Threshold\" displayFormat=\"Hex\"/>",
            &format!("<VariableRef variableId=\"V_SIM_Threshold\" {replacement}/>"),
        );
        let profile = profile(&xml)?;
        let parameter = profile.parameter("V_SIM_Threshold")?;
        assert!(
            parameter
                .needs_input
                .iter()
                .any(|note| note.contains("manual interpretation"))
        );
        assert_eq!(parameter.displays.len(), 2);
        assert_eq!(
            parameter.decode(&[1, 0xF4]).raw_value.as_deref(),
            Some("500")
        );
    }
    Ok(())
}

#[test]
fn float_radix_and_boolean_scaling_are_not_silently_accepted() -> Result<()> {
    let float = XML.replacen(
        "<Datatype xsi:type=\"IntegerT\" bitLength=\"16\"/>",
        "<Datatype xsi:type=\"Float32T\"/>",
        1,
    );
    let float = profile(&float)?;
    let parameter = float.parameter("V_SIM_Threshold")?;
    assert_eq!(parameter.displays.len(), 1);
    assert_eq!(
        parameter.decode(&1.25f32.to_be_bytes()).displays[0].value,
        "1.25"
    );
    assert!(
        parameter
            .needs_input
            .iter()
            .any(|note| note.contains("integer datatype"))
    );
    let xml = XML.replace(
        "subindex=\"2\" displayFormat=\"Dec\"",
        "subindex=\"2\" displayFormat=\"Dec\" gradient=\"1\"",
    );
    let boolean = profile(&xml)?;
    assert!(
        boolean
            .parameter("V_SIM_Record")?
            .target(2)?
            .displays
            .is_empty()
    );
    let zero = XML.replace("gradient=\"0.1\"", "gradient=\"0\"");
    let zero = profile(&zero)?;
    assert!(
        zero.parameter("V_SIM_Record")?
            .target(1)?
            .needs_input
            .iter()
            .any(|note| note.contains("must not be zero"))
    );
    Ok(())
}

#[test]
fn profile_round_trip_defaults_and_format_overrides_are_validated() -> Result<()> {
    let profile = profile(XML)?;
    let encoded = toml::to_string_pretty(&profile)?;
    assert!(
        encoded.contains("format = \"hexadecimal\"") && encoded.contains("format = \"binary\"")
    );
    assert!(!encoded.contains("format = \"decimal\""));
    assert_eq!(Profile::parse(&encoded)?, profile);
    let mut legacy_boolean = profile.parameter("V_SIM_Record")?.target(2)?.clone();
    legacy_boolean.displays[0].format = NumberFormat::Decimal;
    assert_eq!(legacy_boolean.decode(&[1]).displays[0].value, "true");
    let mut invalid = profile.clone();
    invalid
        .iodd
        .as_mut()
        .unwrap()
        .parameters
        .iter_mut()
        .find(|parameter| parameter.id == "V_SIM_Threshold")
        .unwrap()
        .displays[1]
        .field
        .scale = 0.5;
    assert!(invalid.validate().is_err());
    let legacy = iodd::load(Path::new("samples/sensor.xml"), None)?;
    assert_eq!(
        legacy
            .parameters
            .iter()
            .find(|parameter| parameter.id == "V_SIM_Threshold")
            .unwrap()
            .displays[0]
            .format,
        NumberFormat::Decimal
    );
    Ok(())
}

#[test]
fn malformed_responses_retain_bytes_and_suppress_formatted_values() -> Result<()> {
    let profile = profile(XML)?;
    let parameter = profile.parameter("V_SIM_Threshold")?;
    let short = parameter.decode(&[0xFF]);
    assert_eq!(short.raw_hex, "FF");
    assert!(short.decode_error.is_some() && short.displays.is_empty());
    let empty = parameter.decode(&[]);
    assert!(empty.decode_error.is_some() && empty.displays.is_empty());
    Ok(())
}
