use super::*;
use crate::{
    profile::{Gateway, Profile},
    transport,
};

const XML: &str = include_str!("../../samples/sensor-array.xml");
const PAYLOAD: &[u8] = &[0x02, 0xC4, 0x95, 0xF0];

fn profile(xml: &str) -> Result<Profile> {
    let catalog = parse(xml, "synthetic array fixture".into())?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    let profile = Profile {
        expected_identity,
        input_bytes: imported.input_bytes,
        iodd: Some(imported),
        ..Default::default()
    };
    profile.validate()?;
    Ok(profile)
}

#[test]
fn published_nibble_layout_matches_cyclic_fields_parameter_values_and_member_reads() -> Result<()> {
    let profile = profile(XML)?;
    let restored = Profile::parse(&toml::to_string_pretty(&profile)?)?;
    assert_eq!(restored, profile);
    let parameter = restored.parameter("V_SIM_History")?;
    assert_eq!(
        parameter
            .array
            .as_ref()
            .unwrap()
            .element
            .default_value
            .as_deref(),
        Some("0")
    );
    let decoded = parameter.decode(PAYLOAD);
    assert_eq!(decoded.decode_error, None);
    assert_eq!(
        decoded
            .array_items
            .iter()
            .map(|item| item.value.raw_value.as_deref())
            .collect::<Vec<_>>(),
        [
            Some("2"),
            Some("-4"),
            Some("4"),
            Some("-7"),
            Some("5"),
            Some("-1"),
            Some("0")
        ]
    );
    let cyclic = restored.decode(PAYLOAD)?;
    assert_eq!(cyclic.len(), 7);
    for (index, item) in decoded.array_items.iter().enumerate() {
        assert_eq!(cyclic[index].0.bit_offset, item.bit_offset);
        assert_eq!(
            cyclic[index].2.raw,
            item.value.raw_value.as_deref().unwrap()
        );
        assert_eq!(cyclic[index].2.value, item.value.displays[0].value);
        assert_eq!(cyclic[index].0.unit, "°C");
        let member = transport::read_parameter_at(&restored, "V_SIM_History", index as u8 + 1)?;
        assert_eq!(member.raw_value, item.value.raw_value);
        assert_eq!(member.displays[0].value, item.value.displays[0].value);
    }
    // ISDU scalar sign padding differs from the right-justified packed member bytes.
    assert_eq!(decoded.array_items[1].value.raw_hex, "0C");
    assert_eq!(
        transport::read_parameter_at(&restored, "V_SIM_History", 2)?.raw_hex,
        "FC"
    );
    Ok(())
}

#[test]
fn odd_subbyte_widths_do_not_cross_octet_boundaries() -> Result<()> {
    let xml = XML
        .replace("bitLength=\"4\"", "bitLength=\"3\"")
        .replace("count=\"7\"", "count=\"3\"")
        .replace("bitLength=\"28\"", "bitLength=\"11\"");
    let profile = profile(&xml)?;
    let parameter = profile.parameter("V_SIM_History")?;
    let decoded = parameter.decode(&[1, 0x1A]);
    assert_eq!(decoded.decode_error, None);
    assert_eq!(
        decoded
            .array_items
            .iter()
            .map(|item| item.bit_offset)
            .collect::<Vec<_>>(),
        [8, 3, 0]
    );
    assert_eq!(
        decoded
            .array_items
            .iter()
            .map(|item| item.value.raw_value.as_deref())
            .collect::<Vec<_>>(),
        [Some("1"), Some("3"), Some("2")]
    );
    assert_eq!(profile.decode(&[1, 0x1A])?[1].2.raw, "3");
    Ok(())
}

#[test]
fn standard_array_length_restrictions_update_the_saved_layout() -> Result<()> {
    let definitions = parameters::load_standard(
        Path::new("samples/synthetic-standard-definitions.xml"),
        None,
        None,
    )?
    .parameters;
    let xml = XML.replace("</VariableCollection>", r#"<StdVariableRef id="V_DetailedDeviceStatus" fixedLengthRestriction="3"/></VariableCollection>"#);
    let catalog = parse_with_definitions(&xml, "standard array fixture".into(), &definitions)?;
    let parameter = &catalog.parameters[1];
    assert_eq!(parameter.bits, Some(72));
    assert_eq!(parameter.array.as_ref().unwrap().count, 3);
    assert!(parameter.array.as_ref().unwrap().element.origin.is_some());
    let decoded = parameter.decode(&[1, 2, 3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(decoded.decode_error, None);
    assert_eq!(decoded.array_items[1].value.raw_hex, "040506");
    assert_eq!(
        decoded.array_items[1].value.raw_value.as_deref(),
        Some("0x04,0x05,0x06")
    );
    assert!(
        parse_with_definitions(
            &xml.replace(
                "fixedLengthRestriction=\"3\"",
                "fixedLengthRestriction=\"5\""
            ),
            "invalid restriction".into(),
            &definitions
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn text_array_errors_are_isolated_and_boolean_arrays_keep_the_published_bit_order() -> Result<()> {
    let boolean = XML
        .replace(
            "xsi:type=\"IntegerT\" bitLength=\"4\"",
            "xsi:type=\"BooleanT\"",
        )
        .replace("count=\"7\"", "count=\"3\"")
        .replace("bitLength=\"28\"", "bitLength=\"3\"");
    let catalog = parse(&boolean, "boolean array".into())?;
    let decoded = catalog.parameters[0].decode(&[4]);
    assert_eq!(
        decoded
            .array_items
            .iter()
            .map(|item| item.value.raw_value.as_deref())
            .collect::<Vec<_>>(),
        [Some("true"), Some("false"), Some("false")]
    );
    let text = XML
        .replace(
            "xsi:type=\"IntegerT\" bitLength=\"4\"",
            "xsi:type=\"StringT\" fixedLength=\"4\" encoding=\"US-ASCII\"",
        )
        .replace("count=\"7\"", "count=\"2\"");
    let catalog = parse(&text, "text array".into())?;
    let decoded = catalog.parameters[0].decode(&[b'A', b'B', b'C', b'D', b'W', b'X', b'Y', 255]);
    assert_eq!(
        decoded.array_items[0].value.raw_value.as_deref(),
        Some("ABCD")
    );
    assert_eq!(decoded.array_items[1].value.raw_hex, "575859FF");
    assert!(decoded.array_items[1].value.decode_error.is_some());
    Ok(())
}

#[test]
fn whole_octet_arrays_keep_network_byte_order() -> Result<()> {
    let xml = XML
        .replace("bitLength=\"4\"", "bitLength=\"16\"")
        .replace("count=\"7\"", "count=\"3\"")
        .replace("bitLength=\"28\"", "bitLength=\"48\"");
    let profile = profile(&xml)?;
    let payload = [0xFF, 0xFC, 0, 100, 0x7F, 0xFF];
    let values = profile.parameter("V_SIM_History")?.decode(&payload);
    assert_eq!(
        values
            .array_items
            .iter()
            .map(|item| item.value.raw_value.as_deref())
            .collect::<Vec<_>>(),
        [Some("-4"), Some("100"), Some("32767")]
    );
    assert_eq!(profile.decode(&payload)?[1].2.raw, "100");
    Ok(())
}

#[test]
fn array_permissions_and_bounds_are_checked_before_opening_a_gateway() -> Result<()> {
    let mut profile = profile(XML)?;
    profile.connection.gateway = Gateway::IfmAl1350;
    profile.connection.endpoint = "must not open".into();
    assert!(
        transport::read_parameter_at(&profile, "V_SIM_History", 8)
            .unwrap_err()
            .to_string()
            .contains("no declared subindex")
    );
    profile.iodd.as_mut().unwrap().parameters[0]
        .array
        .as_mut()
        .unwrap()
        .subindex_access = false;
    assert!(
        transport::read_parameter_at(&profile, "V_SIM_History", 1)
            .unwrap_err()
            .to_string()
            .contains("does not permit")
    );
    let parameter = &mut profile.iodd.as_mut().unwrap().parameters[0];
    parameter.access = Access::WriteOnly;
    parameter.array.as_mut().unwrap().element.access = Access::WriteOnly;
    assert!(
        transport::read_parameter_at(&profile, "V_SIM_History", 0)
            .unwrap_err()
            .to_string()
            .contains("write-only")
    );
    Ok(())
}

#[test]
fn malformed_arrays_preserve_raw_payloads_and_unsupported_layouts_need_review() -> Result<()> {
    let profile = profile(XML)?;
    let decoded = profile.parameter("V_SIM_History")?.decode(&[2, 3]);
    assert_eq!(decoded.raw_hex, "0203");
    assert!(
        decoded
            .decode_error
            .unwrap()
            .contains("array response width")
    );
    assert!(decoded.array_items.is_empty());
    for xml in [
        XML.replace("count=\"7\"", "count=\"0\""),
        XML.replace("count=\"7\"", "count=\"256\""),
        XML.replace("bitLength=\"4\"", "bitLength=\"9\""),
        XML.replace("bitLength=\"4\"", "bitLength=\"65528\""),
    ] {
        let catalog = parse(&xml, "unsupported array".into())?;
        assert!(catalog.parameters[0].array.is_none());
        assert!(!catalog.parameters[0].needs_input.is_empty());
        assert!(catalog.layouts[0].fields.is_empty());
        assert!(!catalog.layouts[0].needs_input.is_empty());
    }
    Ok(())
}
