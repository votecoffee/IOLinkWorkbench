use super::*;

const SENSOR: &str = include_str!("../../samples/sensor.xml");

fn with_display(unit: &str) -> String {
    SENSOR.replace("</DeviceFunction>", &format!(r#"
        <UserInterface><ProcessDataRefCollection><ProcessDataRef processDataId="IN">
          <ProcessDataRecordItemInfo subindex="1" gradient="0.1" offset="2" unitCode="{unit}" displayFormat="Dec.1"/>
          <ProcessDataRecordItemInfo subindex="2"/>
        </ProcessDataRef></ProcessDataRefCollection></UserInterface>
      </DeviceFunction>"#))
}

#[test]
fn display_metadata_drives_the_same_decoder_as_manual_fields() -> Result<()> {
    let catalog = parse(&with_display("1013"), "synthetic display fixture".into())?;
    let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    assert!(imported.needs_input.is_empty());
    let profile = crate::profile::Profile {
        expected_identity: identity,
        iodd: Some(imported),
        ..crate::profile::Profile::default()
    };
    profile.validate()?;
    let payload = crate::decode::parse_hex("FFF9")?;
    let fields = profile.decode(&payload)?;
    assert_eq!(fields[0].0.unit, "mm");
    assert_eq!(fields[0].2.value, "1.6");
    assert_eq!(fields[1].2.value, "true");
    Ok(())
}

#[test]
fn explicit_no_unit_is_distinct_from_missing_or_unknown_unit_metadata() -> Result<()> {
    let known = parse(&with_display("1997"), "known no-unit fixture".into())?;
    assert_eq!(known.layouts[0].fields[0].unit, "");
    assert!(known.layouts[0].needs_input.is_empty());
    let missing = parse(SENSOR, "missing-unit fixture".into())?;
    assert!(!missing.layouts[0].needs_input.is_empty());
    let unknown = parse(&with_display("65500"), "unknown-unit fixture".into())?;
    assert_eq!(unknown.layouts[0].fields[0].unit, "IODD unit 65500");
    assert!(unknown.layouts[0].needs_input[0].contains("65500"));
    Ok(())
}

#[test]
fn ambiguous_ids_and_overlapping_iodd_fields_are_rejected() {
    let variant = SENSOR.replace("SIM-ALT", "SIM-SENSOR");
    assert!(
        parse(&variant, "duplicate variants".into())
            .unwrap_err()
            .to_string()
            .contains("duplicate variant Product ID")
    );
    let datatype = SENSOR.replace(
        "</DatatypeCollection>",
        "<Datatype id=\"PD\" xsi:type=\"UIntegerT\" bitLength=\"16\"/></DatatypeCollection>",
    );
    assert!(
        parse(&datatype, "duplicate types".into())
            .unwrap_err()
            .to_string()
            .contains("duplicate datatype ID")
    );
    let overlap = SENSOR.replace(
        "subindex=\"2\" bitOffset=\"0\"",
        "subindex=\"2\" bitOffset=\"1\"",
    );
    assert!(
        parse(&overlap, "overlapping fields".into())
            .unwrap_err()
            .to_string()
            .contains("overlap")
    );
}

fn imported_profile() -> Result<crate::profile::Profile> {
    let catalog = parse(SENSOR, "synthetic parameter fixture".into())?;
    let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    Ok(crate::profile::Profile {
        expected_identity: identity,
        iodd: Some(imported),
        ..crate::profile::Profile::default()
    })
}

#[test]
fn parameter_metadata_and_defaults_survive_a_profile_round_trip() -> Result<()> {
    let profile = imported_profile()?;
    let text = toml::to_string_pretty(&profile)?;
    let restored = crate::profile::Profile::parse(&text)?;
    assert_eq!(profile, restored);
    let imported = restored.iodd.unwrap();
    assert_eq!(imported.parameters.len(), 3);
    assert_eq!(imported.standard_parameter_refs, ["V_ProductID"]);
    let threshold = &imported.parameters[0];
    assert_eq!(threshold.id, "V_SIM_Threshold");
    assert_eq!(threshold.index, 64);
    assert_eq!(threshold.default_value.as_deref(), Some("500"));
    assert_eq!(threshold.ranges[0].lower, "-1000");
    assert_eq!(threshold.choices[0].name, "Nominal setting");
    Ok(())
}

#[test]
fn named_parameter_reads_decode_raw_values_without_scaling_process_data() -> Result<()> {
    let profile = imported_profile()?;
    let threshold = crate::transport::read_parameter(&profile, "V_SIM_Threshold")?;
    assert_eq!(threshold.raw_hex, "01F4");
    assert_eq!(threshold.raw_value.as_deref(), Some("500"));
    assert_eq!(threshold.choice_name.as_deref(), Some("Nominal setting"));
    assert_eq!(threshold.decode_error, None);
    let label = crate::transport::read_parameter(&profile, "V_SIM_Label")?;
    assert_eq!(label.raw_value.as_deref(), Some("synthetic sensor"));
    Ok(())
}

#[test]
fn write_only_and_unknown_parameters_are_rejected_before_transport_open() -> Result<()> {
    let mut profile = imported_profile()?;
    profile.connection.gateway = crate::profile::Gateway::IfmAl1350;
    profile.connection.endpoint = "invalid endpoint must never be opened".into();
    assert!(
        crate::transport::read_parameter(&profile, "V_SIM_Command")
            .unwrap_err()
            .to_string()
            .contains("write-only")
    );
    assert!(
        crate::transport::read_parameter(&profile, "missing")
            .unwrap_err()
            .to_string()
            .contains("not in the imported")
    );
    Ok(())
}

#[test]
fn malformed_typed_responses_retain_raw_bytes_and_identity_checks_apply() -> Result<()> {
    let mut profile = imported_profile()?;
    profile.iodd.as_mut().unwrap().parameters[0].bits = Some(32);
    let reading = crate::transport::read_parameter(&profile, "V_SIM_Threshold")?;
    assert_eq!(reading.raw_hex, "01F4");
    assert_eq!(reading.raw_value, None);
    assert!(reading.decode_error.unwrap().contains("width differs"));
    profile.expected_identity.device_id = Some(2);
    assert!(
        crate::transport::read_parameter(&profile, "V_SIM_Threshold")
            .unwrap_err()
            .to_string()
            .contains("identity mismatch")
    );
    Ok(())
}

#[test]
fn parameter_raw_decoding_preserves_wide_integers_and_validates_text() -> Result<()> {
    let mut parameter = imported_profile()?.iodd.unwrap().parameters.remove(0);
    parameter.datatype = "UIntegerT".into();
    parameter.bits = Some(64);
    assert_eq!(
        parameter.decode_raw(&[255; 8])?.as_deref(),
        Some("18446744073709551615")
    );
    parameter.datatype = "StringT".into();
    parameter.fixed_length = Some(8);
    parameter.encoding = Some("UTF-8".into());
    assert_eq!(
        parameter.decode_raw(b"level\0\0\0")?.as_deref(),
        Some("level")
    );
    assert!(parameter.decode_raw(&[255]).is_err());
    Ok(())
}

#[test]
fn duplicate_parameter_ids_or_indexes_are_rejected() {
    let id = SENSOR.replace("id=\"V_SIM_Label\"", "id=\"V_SIM_Threshold\"");
    assert!(
        parse(&id, "duplicate parameter IDs".into())
            .unwrap_err()
            .to_string()
            .contains("duplicate parameter ID")
    );
    let index = SENSOR.replace("index=\"65\"", "index=\"64\"");
    assert!(
        parse(&index, "duplicate parameter indexes".into())
            .unwrap_err()
            .to_string()
            .contains("duplicate parameter index")
    );
}
