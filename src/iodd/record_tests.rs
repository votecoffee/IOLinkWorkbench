use super::*;
use crate::{
    profile::{Gateway, Profile},
    transport,
};

const XML: &str = include_str!("../../samples/sensor-record.xml");
const PAYLOAD: &[u8] = &[0xFF, 0xEC, 1, 10, b'T', b'E', b'S', b'T'];

fn profile(xml: &str) -> Result<Profile> {
    let catalog = parse(xml, "synthetic record fixture".into())?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    let profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    profile.validate()?;
    Ok(profile)
}

#[test]
fn record_layout_defaults_and_displays_survive_reload_and_match_member_reads() -> Result<()> {
    let profile = profile(XML)?;
    let restored = Profile::parse(&toml::to_string_pretty(&profile)?)?;
    assert_eq!(restored, profile);
    let parameter = restored.parameter("V_SIM_Record")?;
    let record = parameter.record.as_ref().unwrap();
    assert_eq!(
        record.items[0].parameter.default_value.as_deref(),
        Some("-20")
    );
    assert_eq!(record.items[0].parameter.access, Access::ReadOnly);
    let decoded = parameter.decode(PAYLOAD);
    assert_eq!(decoded.decode_error, None);
    assert_eq!(decoded.record_items.len(), 4);
    assert_eq!(
        decoded.record_items[0].value.raw_value.as_deref(),
        Some("-20")
    );
    assert_eq!(decoded.record_items[0].value.displays[0].value, "0.0");
    assert_eq!(
        decoded.record_items[1].value.raw_value.as_deref(),
        Some("true")
    );
    assert_eq!(
        decoded.record_items[2].value.choice_name.as_deref(),
        Some("Synthetic mode")
    );
    assert_eq!(
        decoded.record_items[3].value.raw_value.as_deref(),
        Some("TEST")
    );
    let whole = transport::read_parameter(&restored, "V_SIM_Record")?;
    assert_eq!(whole.raw_hex, "FFEC010A54455354");
    for subindex in 1..=4 {
        let member = transport::read_parameter_at(&restored, "V_SIM_Record", subindex)?;
        let value = &whole.record_items[usize::from(subindex - 1)].value;
        assert_eq!(member.subindex, subindex);
        assert_eq!(member.raw_value, value.raw_value);
        assert_eq!(member.raw_hex, value.raw_hex);
        assert!(member.record_items.is_empty());
    }
    Ok(())
}

#[test]
fn prohibited_member_reads_fail_before_opening_a_gateway() -> Result<()> {
    let mut disabled = profile(&XML.replace(
        "subindexAccessSupported=\"true\"",
        "subindexAccessSupported=\"false\"",
    ))?;
    disabled.connection.gateway = Gateway::IfmAl1350;
    disabled.connection.endpoint = "must never open".into();
    assert!(
        transport::read_parameter_at(&disabled, "V_SIM_Record", 1)
            .unwrap_err()
            .to_string()
            .contains("does not permit")
    );
    let mut profile = profile(XML)?;
    profile.connection.gateway = Gateway::IfmAl1350;
    profile.connection.endpoint = "must never open".into();
    assert!(
        transport::read_parameter_at(&profile, "V_SIM_Record", 9)
            .unwrap_err()
            .to_string()
            .contains("no declared subindex")
    );
    profile.iodd.as_mut().unwrap().parameters[0]
        .record
        .as_mut()
        .unwrap()
        .items[0]
        .parameter
        .access = Access::WriteOnly;
    assert!(
        transport::read_parameter_at(&profile, "V_SIM_Record", 1)
            .unwrap_err()
            .to_string()
            .contains("write-only")
    );
    Ok(())
}

#[test]
fn ambiguous_or_outside_record_layouts_are_rejected() {
    for xml in [
        XML.replace("subindex=\"2\"", "subindex=\"1\""),
        XML.replace("subindex=\"2\"", "subindex=\"0\""),
        XML.replace("bitOffset=\"48\"", "bitOffset=\"40\""),
        XML.replace("bitLength=\"64\"", "bitLength=\"63\""),
        XML.replace("bitLength=\"64\"", "bitLength=\"1857\""),
        XML.replace(
            "<RecordItemInfo subindex=\"1\"",
            "<RecordItemInfo subindex=\"9\"",
        ),
        XML.replace("accessRights=\"rw\"", "accessRights=\"ro\"")
            .replace(
                "accessRightRestriction=\"ro\"",
                "accessRightRestriction=\"rw\"",
            ),
    ] {
        assert!(profile(&xml).is_err(), "invalid record was accepted");
    }
}

#[test]
fn malformed_record_responses_preserve_bytes_and_member_errors_are_independent() -> Result<()> {
    let profile = profile(XML)?;
    let parameter = profile.parameter("V_SIM_Record")?;
    let short = parameter.decode(&PAYLOAD[..7]);
    assert_eq!(short.raw_hex, crate::decode::hex(&PAYLOAD[..7]));
    assert!(short.decode_error.unwrap().contains("width differs"));
    assert!(short.record_items.is_empty());
    let mut invalid_text = PAYLOAD.to_vec();
    invalid_text[7] = 255;
    let decoded = parameter.decode(&invalid_text);
    assert_eq!(
        decoded.record_items[0].value.raw_value.as_deref(),
        Some("-20")
    );
    assert!(
        decoded.record_items[3]
            .value
            .decode_error
            .as_deref()
            .unwrap()
            .contains("not US-ASCII")
    );
    assert_eq!(decoded.record_items[3].value.raw_hex, "544553FF");
    Ok(())
}

#[test]
fn standard_record_restrictions_are_merged_without_enabling_commands() -> Result<()> {
    let definitions = parameters::load_standard(
        Path::new("samples/synthetic-standard-definitions.xml"),
        None,
        None,
    )?
    .parameters;
    let xml = XML.replace("</VariableCollection>", r#"<StdVariableRef id="V_DirectParameters_1"><StdRecordItemRef subindex="1" defaultValue="7"/><StdRecordItemRef subindex="16"><StdSingleValueRef value="128"/></StdRecordItemRef></StdVariableRef></VariableCollection>"#);
    let catalog = parse_with_definitions(&xml, "standard record fixture".into(), &definitions)?;
    let standard = &catalog.parameters[1];
    assert_eq!(standard.target(1)?.default_value.as_deref(), Some("7"));
    assert_eq!(standard.target(16)?.choices[0].name, "Synthetic reset");
    assert!(standard.target(16)?.origin.is_some());
    let decoded = standard.decode(&[7, 128]);
    assert_eq!(
        decoded.record_items[0].value.raw_value.as_deref(),
        Some("7")
    );
    assert!(
        decoded.record_items[1]
            .value
            .decode_error
            .as_deref()
            .unwrap()
            .contains("write-only")
    );
    assert!(
        parse_with_definitions(
            &xml.replace(
                "subindex=\"16\"><StdSingleValueRef",
                "subindex=\"99\"><StdSingleValueRef"
            ),
            "bad member".into(),
            &definitions
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn full_size_acyclic_record_does_not_expand_the_cyclic_payload_limit() -> Result<()> {
    let xml = XML
        .replace("bitLength=\"64\"", "bitLength=\"1856\"")
        .replace("bitOffset=\"48\"", "bitOffset=\"1840\"")
        .replace("bitOffset=\"40\"", "bitOffset=\"1832\"")
        .replace("bitOffset=\"32\"", "bitOffset=\"1824\"")
        .replace("fixedLength=\"4\"", "fixedLength=\"200\"");
    let profile = profile(&xml)?;
    let mut bytes = vec![b'A'; 232];
    bytes[0..4].copy_from_slice(&PAYLOAD[..4]);
    let text = crate::decode::hex(&bytes);
    assert!(crate::decode::parse_hex(&text).is_err());
    assert_eq!(crate::decode::parse_hex_bounded(&text, 232)?, bytes);
    assert!(crate::decode::parse_hex_bounded(&"00".repeat(233), 232).is_err());
    let decoded = profile.parameter("V_SIM_Record")?.decode(&bytes);
    assert_eq!(decoded.decode_error, None);
    assert_eq!(
        decoded.record_items[3].value.raw_value.as_deref(),
        Some("A".repeat(200)).as_deref()
    );
    Ok(())
}

#[test]
fn packed_booleans_use_declared_bit_offsets_within_the_last_octet() -> Result<()> {
    let mut profile = profile(XML)?;
    let parameter = &mut profile.iodd.as_mut().unwrap().parameters[0];
    parameter.bits = Some(4);
    for (offset, item) in parameter
        .record
        .as_mut()
        .unwrap()
        .items
        .iter_mut()
        .enumerate()
    {
        item.bit_offset = offset as u16;
        item.parameter.datatype = "BooleanT".into();
        item.parameter.bits = Some(1);
        item.parameter.fixed_length = None;
        item.parameter.encoding = None;
        item.parameter.displays.clear();
        item.parameter.choices.clear();
        item.parameter.default_value = None;
    }
    profile.validate()?;
    let decoded = profile.parameter("V_SIM_Record")?.decode(&[0b1101]);
    assert_eq!(
        decoded
            .record_items
            .iter()
            .map(|item| item.value.raw_value.as_deref())
            .collect::<Vec<_>>(),
        [Some("true"), Some("false"), Some("true"), Some("true")]
    );
    Ok(())
}
