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
