use super::*;

const SENSOR: &str = include_str!("../../samples/sensor.xml");

#[test]
fn names_match_exact_unscaled_array_values_and_exclude_ambiguous_choices() -> Result<()> {
    let xml = include_str!("../../samples/sensor-array.xml").replace(
        r#"<ValueRange lowerValue="-8" upperValue="7"/>"#,
        r#"<SingleValue value="-04"><Name textId="temperature"/></SingleValue>
           <SingleValue value="2"><Name textId="temperature"/></SingleValue>
           <SingleValue value="+02"><Name textId="history"/></SingleValue>
           <SingleValue value="16"><Name textId="temperature"/></SingleValue>
           <SingleValue value="0"><Name textId="missing"/></SingleValue>"#,
    );
    let catalog = parse(&xml, "synthetic array choices".into())?;
    let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let mut profile = crate::profile::Profile::default();
    profile.apply_iodd(identity, imported);
    let profile = crate::profile::Profile::parse(&toml::to_string(&profile)?)?;
    let data = [0x02, 0xC4, 0x95, 0xF0];
    let fields = profile.decode(&data)?;
    assert_eq!(fields[0].2.raw, "2");
    assert!(fields[0].2.value_name.is_none());
    assert_eq!(fields[1].2.raw, "-4");
    assert_eq!(fields[1].2.display(), "Temperature (-1.0)");
    assert_eq!(fields[1].0.csv_value(&data)?, "-1.0");
    assert_eq!(fields[1].0.value_names.len(), 1);
    assert!(fields[6].2.value_name.is_none());
    Ok(())
}

#[test]
fn names_distinguish_adjacent_unsigned_64_bit_values() -> Result<()> {
    let xml = SENSOR.replace(
        r#"<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>"#,
        r#"<ProcessDataIn id="IN" bitLength="64"><Datatype xsi:type="UIntegerT" bitLength="64">
              <SingleValue value="18446744073709551615"><Name textId="nominal"/></SingleValue>
            </Datatype></ProcessDataIn>"#,
    );
    let catalog = parse(&xml, "synthetic unsigned choice".into())?;
    let field = &catalog.layouts[0].fields[0];
    let maximum = u64::MAX.to_be_bytes();
    assert_eq!(
        field.decode(&maximum)?.display(),
        "Nominal setting (18446744073709551615)"
    );
    assert!(
        field
            .decode(&(u64::MAX - 1).to_be_bytes())?
            .value_name
            .is_none()
    );
    assert_eq!(field.csv_value(&maximum)?, u64::MAX.to_string());
    Ok(())
}

#[test]
fn names_match_float32_array_values_before_display_scaling() -> Result<()> {
    let xml = SENSOR.replace(
        r#"<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>"#,
        r#"<ProcessDataIn id="IN" bitLength="64" gradient="2" displayFormat="Dec.2">
               <Datatype xsi:type="ArrayT" count="2"><SimpleDatatype xsi:type="Float32T">
                 <SingleValue value="1.250e0"><Name textId="nominal"/></SingleValue>
                 <SingleValue value="-0"><Name textId="switch"/></SingleValue>
                 <SingleValue value="-2.5"><Name textId="nominal"/></SingleValue>
                 <SingleValue value="-2.50E+0"><Name textId="switch"/></SingleValue>
                 <SingleValue value="1.40129846E-45"><Name textId="level"/></SingleValue>
                 <SingleValue value="INF"><Name textId="nominal"/></SingleValue>
                 <SingleValue value="NaN"><Name textId="switch"/></SingleValue>
               </SimpleDatatype></Datatype>
               <Name textId="level"/>
             </ProcessDataIn>"#,
    );
    let catalog = parse(&xml, "synthetic Float32 choices".into())?;
    let fields = &catalog.layouts[0].fields;
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].value_names.len(), 3);
    let pair = |left: u32, right: u32| [left.to_be_bytes(), right.to_be_bytes()].concat();
    let nominal = 1.25f32.to_bits();
    let payload = pair(nominal, nominal + 1);
    assert_eq!(
        fields[0].decode(&payload)?.display(),
        "Nominal setting (2.50)"
    );
    assert_eq!(fields[0].csv_value(&payload)?, "2.50");
    assert_eq!(fields[1].decode(&payload)?.display(), "2.50");
    for zero in [0.0f32, -0.0f32] {
        let data = pair(zero.to_bits(), zero.to_bits());
        let first = fields[0].decode(&data)?;
        assert_eq!(first.raw, zero.to_string());
        assert_eq!(first.value_name.as_deref(), Some("switch"));
        assert_eq!(
            fields[1].decode(&data)?.value_name.as_deref(),
            Some("switch")
        );
    }
    assert_eq!(
        fields[0].decode(&pair(1, 0))?.value_name.as_deref(),
        Some("level")
    );
    for unlabelled in [(-2.5f32).to_bits(), f32::INFINITY.to_bits(), 0x7fc0_0001] {
        assert!(fields[0].decode(&pair(unlabelled, 0))?.value_name.is_none());
    }
    Ok(())
}

#[test]
fn named_cyclic_ranges_keep_exact_unsigned_bounds_and_ignore_overlaps() -> Result<()> {
    let xml = SENSOR.replace(
        r#"<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>"#,
        r#"<ProcessDataIn id="IN" bitLength="64">
               <Datatype xsi:type="UIntegerT" bitLength="64">
                 <ValueRange lowerValue="1" upperValue="10"><Name textId="nominal"/></ValueRange>
                 <ValueRange lowerValue="2" upperValue="3"/>
                 <ValueRange lowerValue="4" upperValue="6"><Name textId="nominal"/></ValueRange>
                 <ValueRange lowerValue="20" upperValue="30"><Name textId="nominal"/></ValueRange>
                 <SingleValue value="21"><Name textId="switch"/></SingleValue>
                 <ValueRange lowerValue="9007199254740993" upperValue="9007199254740994"><Name textId="level"/></ValueRange>
                 <ValueRange lowerValue="18446744073709551614" upperValue="18446744073709551615"><Name textId="nominal"/></ValueRange>
                 <ValueRange lowerValue="9" upperValue="4"><Name textId="switch"/></ValueRange>
               </Datatype>
             </ProcessDataIn>"#,
    );
    let catalog = parse(&xml, "synthetic cyclic ranges".into())?;
    let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let mut profile = crate::profile::Profile::default();
    profile.apply_iodd(identity, imported);
    let profile = crate::profile::Profile::parse(&toml::to_string(&profile)?)?;
    let field = profile.effective_fields()[0].0;
    assert_eq!(field.value_ranges.len(), 2);
    for (value, name) in [
        (1u64, None),
        (4, None),
        (9, None),
        (20, None),
        (21, Some("switch")),
        (9_007_199_254_740_992, None),
        (9_007_199_254_740_993, Some("level")),
        (9_007_199_254_740_994, Some("level")),
        (9_007_199_254_740_995, None),
        (u64::MAX, Some("Nominal setting")),
    ] {
        let bytes = value.to_be_bytes();
        let observation = field.decode(&bytes)?;
        assert_eq!(observation.value_name.as_deref(), name);
        assert_eq!(field.csv_value(&bytes)?, value.to_string());
    }
    Ok(())
}

#[test]
fn explicit_acyclic_layout_imports_and_qualifies_only_its_own_selection() -> Result<()> {
    let xml = SENSOR.replace(
        "<ProcessDataIn id=\"IN\" bitLength=\"16\"><DatatypeRef datatypeId=\"PD\"/></ProcessDataIn>",
        "",
    );
    let catalog = parse(&xml, "synthetic acyclic".into())?;
    assert_eq!(catalog.default_layout_id(), Some("PD"));
    let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let mut profile = crate::profile::Profile::default();
    profile.apply_iodd(identity, imported);
    let encoded = toml::to_string(&profile)?;
    let reloaded = crate::profile::Profile::parse(&encoded)?;
    assert!(reloaded.acyclic_only() && !reloaded.has_process_input());
    assert!(reloaded.output_layout().is_none() && reloaded.require_process_output().is_err());
    assert!(reloaded.parameter("V_SIM_Threshold").is_ok());
    let mut contradictory = reloaded;
    contradictory.input_bytes = 2;
    assert!(contradictory.validate().is_err());

    let unavailable = xml.replace(
        "<ProcessData id=\"PD\">",
        "<ProcessData id=\"PD\"><Extension/>",
    );
    let unavailable = parse(&unavailable, "unknown process item".into())?;
    assert!(unavailable.acyclic_layouts.is_empty());
    assert!(unavailable.select(Some("SIM-SENSOR"), None).is_err());
    let mixed = SENSOR.replace("</ProcessDataCollection>",
        "<ProcessData id=\"QUIET\"><Condition variableId=\"V_SIM_Threshold\" value=\"1\"/></ProcessData></ProcessDataCollection>");
    let mixed = parse(&mixed, "mixed layouts".into())?;
    assert!(mixed.default_layout_id().is_none());
    assert!(!mixed.select(Some("SIM-SENSOR"), Some("IN"))?.1.acyclic_only);
    let quiet = mixed.select(Some("SIM-SENSOR"), Some("QUIET"))?.1;
    assert!(quiet.acyclic_only && quiet.condition.as_deref() == Some("V_SIM_Threshold = 1"));
    Ok(())
}

#[test]
fn output_only_import_preserves_identity_output_and_dormant_input_mappings() -> Result<()> {
    use crate::profile::Profile;
    let xml = SENSOR
        .replace("ProcessDataIn id=\"IN\"", "ProcessDataOut id=\"OUT\"")
        .replace("</ProcessDataIn>", "</ProcessDataOut>");
    let catalog = parse(&xml, "synthetic output only".into())?;
    assert!(catalog.layouts.is_empty());
    assert_eq!(catalog.default_layout_id(), Some("OUT"));
    let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    assert_eq!(identity.product_id.as_deref(), Some("SIM-SENSOR"));
    assert_eq!(imported.layout, "OUT");
    assert_eq!(imported.output.as_ref().unwrap().bit_length, 16);
    assert!(imported.fields.is_empty() && !imported.parameters.is_empty());
    let profile = Profile {
        input_bytes: imported.input_bytes,
        iodd: Some(imported),
        expected_identity: identity,
        fields: vec![Field {
            name: "retained input".into(),
            bits: 16,
            ..Field::default()
        }],
        ..Profile::default()
    };
    let reloaded = Profile::parse(&toml::to_string(&profile)?)?;
    assert_eq!(reloaded.fields, profile.fields);
    assert!(reloaded.effective_fields().is_empty() && reloaded.log_fields().is_empty());
    assert!(reloaded.output_layout().is_some());
    assert!(reloaded.decode(&[]).is_err() && reloaded.logging_plan().is_err());
    let mut claimed_input = reloaded;
    claimed_input.input_bytes = 2;
    assert!(claimed_input.validate().is_err());
    assert!(
        Profile {
            input_bytes: 0,
            ..Profile::default()
        }
        .validate()
        .is_err()
    );
    Ok(())
}

#[test]
fn standalone_output_booleans_use_one_octet_with_explicit_legacy_observations() -> Result<()> {
    for bits in [1, 8] {
        let xml = SENSOR.replace("</ProcessData>", &format!(
            r#"<ProcessDataOut id="OUT" bitLength="{bits}"><Datatype xsi:type="BooleanT"/><Name textId="switch"/></ProcessDataOut></ProcessData>"#,
        ));
        let catalog = parse(&xml, "synthetic standalone Boolean".into())?;
        let layout = catalog.layouts[0].output.as_ref().unwrap();
        assert!(layout.standalone_boolean && !layout.packed && layout.bytes() == 1);
        assert_eq!(layout.editable_output_fields().count(), 1);
        assert_eq!(layout.edit_output_field(vec![0], "switch", "true")?, [0xFF]);
        assert_eq!(
            layout.edit_output_field(vec![0xFF], "switch", "false")?,
            [0]
        );
        assert!(layout.check_proposed_output(&[1]).is_err());
        assert!(
            layout
                .edit_output_field(vec![0, 0], "switch", "true")
                .is_err()
        );
        let noncanonical = layout.decode_output(&[2], None);
        assert!(noncanonical.encoding_warning.is_some());
        assert_eq!(noncanonical.fields[0].value.raw, "true");
        assert_eq!(noncanonical.fields[0].value.bits_hex, "02");
        let canonical = layout.decode_output(&[0xFF], None);
        assert!(canonical.encoding_warning.is_none());
        assert_eq!(canonical.fields[0].value.bits_hex, "FF");
        let mut invalid = layout.clone();
        invalid.packed = true;
        assert!(invalid.validate().is_err());
    }
    Ok(())
}

#[test]
fn standalone_integer_outputs_regenerate_canonical_high_bits() -> Result<()> {
    use crate::profile::Profile;
    for (declared, bits, datatype, before, input, after) in [
        (12, 12, "IntegerT", "0001", "-2048", "F800"),
        (16, 12, "IntegerT", "FFFF", "2047", "07FF"),
        (17, 17, "UIntegerT", "00000000", "93786", "00016E5A"),
        (32, 20, "UIntegerT", "00000000", "1048575", "000FFFFF"),
        (
            63,
            63,
            "IntegerT",
            "0000000000000000",
            "-4611686018427387904",
            "C000000000000000",
        ),
    ] {
        let xml = SENSOR.replace("</ProcessData>", &format!(
            r#"<ProcessDataOut id="OUT" bitLength="{declared}"><Datatype xsi:type="{datatype}" bitLength="{bits}"/><Name textId="integer"/></ProcessDataOut></ProcessData>"#,
        ));
        let catalog = parse(&xml, "synthetic standalone integer".into())?;
        let mut profile = Profile::default();
        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
        profile.apply_iodd(identity, imported);
        let serialized = toml::to_string(&profile)?;
        let reloaded = Profile::parse(&serialized)?;
        let layout = reloaded.output_layout().unwrap();
        assert!(layout.padded_integer && !layout.packed);
        let expected_after = crate::decode::parse_hex(after)?;
        let edited =
            layout.edit_output_field(crate::decode::parse_hex(before)?, "integer", input)?;
        assert_eq!(edited, expected_after);
        layout.check_proposed_output(&edited)?;
        let decoded = layout.decode_output(&edited, None);
        assert!(decoded.encoding_warning.is_none());
        assert_eq!(decoded.fields[0].value.raw, input);
        let mut malformed = edited.clone();
        malformed[0] ^= 0x80;
        assert!(layout.check_proposed_output(&malformed).is_err());
        let observed = layout.decode_output(&malformed, None);
        assert!(observed.encoding_warning.is_some());
        assert_eq!(observed.fields[0].value.raw, input);
        assert!(
            layout
                .edit_output_field(edited, "integer", "18446744073709551615")
                .is_err()
        );
        let legacy = Profile::parse(&serialized.replace("padded_integer = true\n", ""))?;
        let legacy = legacy.output_layout().unwrap();
        assert!(!legacy.padded_integer && legacy.editable_output_fields().count() == 0);
        assert_eq!(legacy.bytes(), (declared as usize).div_ceil(8));
        let mut contradictory = layout.clone();
        contradictory.fields.clear();
        assert!(contradictory.validate().is_err());
    }
    Ok(())
}

#[test]
fn standalone_integer_inputs_use_padded_wire_width_and_keep_original_value_bits() -> Result<()> {
    use crate::profile::Profile;
    let process_input =
        r#"<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>"#;
    for (declared, bits, datatype, canonical, raw) in [
        (7usize, 7, "UIntegerT", "7F", "127"),
        (16, 12, "IntegerT", "F800", "-2048"),
        (17, 17, "UIntegerT", "00016E5A", "93786"),
        (32, 17, "IntegerT", "FFFF0000", "-65536"),
        (
            63,
            63,
            "IntegerT",
            "C000000000000000",
            "-4611686018427387904",
        ),
    ] {
        let xml = SENSOR.replace(process_input, &format!(
            r#"<ProcessDataIn id="IN" bitLength="{declared}"><Datatype xsi:type="{datatype}" bitLength="{bits}"/><Name textId="integer"/></ProcessDataIn>"#,
        ));
        let catalog = parse(&xml, "synthetic standalone integer input".into())?;
        assert_eq!(catalog.layouts[0].padded_input_bit_length, Some(declared));
        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
        let mut profile = Profile::default();
        profile.apply_iodd(identity, imported);
        let reloaded = Profile::parse(&toml::to_string(&profile)?)?;
        let payload = crate::decode::parse_hex(canonical)?;
        assert_eq!(reloaded.input_bytes, payload.len());
        assert_eq!(reloaded.padded_input_field().unwrap().bits, bits);
        assert_eq!(reloaded.decode(&payload)?[0].2.raw, raw);
        assert!(reloaded.input_encoding_warning(&payload).is_none());

        let mut changed = payload.clone();
        changed[0] ^= 0x80;
        assert!(reloaded.input_encoding_warning(&changed).is_some());
        assert_eq!(reloaded.decode(&changed)?[0].2.raw, raw);

        let mut legacy = reloaded.clone();
        let legacy_bytes = declared.div_ceil(8);
        legacy.input_bytes = legacy_bytes;
        let saved = legacy.iodd.as_mut().unwrap();
        saved.input_bytes = legacy_bytes;
        saved.padded_input_bit_length = None;
        let legacy = Profile::parse(&toml::to_string(&legacy)?)?;
        let compact = &payload[payload.len() - legacy_bytes..];
        assert!(legacy.input_encoding_warning(compact).is_none());
        assert_eq!(legacy.decode(compact)?[0].2.raw, raw);

        if declared == 17 {
            assert!(reloaded.decode(compact).is_err());
            let mut manual_width = reloaded.clone();
            manual_width.input_bytes = 3;
            manual_width.validate()?;
            assert!(manual_width.padded_input_field().is_none());
            assert_eq!(manual_width.decode(compact)?[0].2.raw, raw);
        }

        let mut contradictory = reloaded;
        contradictory.iodd.as_mut().unwrap().fields[0].byte_order = ByteOrder::LittleEndian;
        assert!(contradictory.validate().is_err());
    }
    let packed = parse(SENSOR, "normal packed input".into())?;
    assert!(packed.layouts[0].padded_input_bit_length.is_none());
    Ok(())
}

#[test]
fn standalone_boolean_input_round_trips_and_uses_the_whole_byte() -> Result<()> {
    use crate::profile::Profile;

    let old =
        r#"<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>"#;
    for bits in [1, 8] {
        let xml = SENSOR.replace(
            old,
            &format!(
                r#"<ProcessDataIn id="IN" bitLength="{bits}"><Datatype xsi:type="BooleanT"/><Name textId="switch"/></ProcessDataIn>"#,
            ),
        );
        let catalog = parse(&xml, "synthetic standalone Boolean input".into())?;
        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
        assert_eq!(imported.input_bytes, 1);
        assert_eq!(imported.fields[0].bits, 8);
        let mut profile = Profile::default();
        profile.apply_iodd(identity, imported);
        let profile = Profile::parse(&toml::to_string(&profile)?)?;
        for byte in 0..=u8::MAX {
            assert_eq!(profile.decode(&[byte])?[0].2.raw, (byte != 0).to_string());
            assert_eq!(
                profile.input_encoding_warning(&[byte]).is_some(),
                matches!(byte, 1..=254)
            );
        }
        assert!(profile.decode(&[0, 2]).is_err());
        assert!(profile.input_encoding_warning(&[0, 2]).is_none());

        let mut legacy = profile.clone();
        legacy.iodd.as_mut().unwrap().fields[0].bits = 1;
        let legacy = Profile::parse(&toml::to_string(&legacy)?)?;
        assert_eq!(legacy.decode(&[2])?[0].2.raw, "false");
        assert!(legacy.input_encoding_warning(&[2]).is_none());

        let mut manual = profile.clone();
        let mut field = manual.iodd.as_ref().unwrap().fields[0].clone();
        field.bits = 1;
        manual.fields.push(field);
        manual.validate()?;
        assert_eq!(manual.decode(&[2])?[0].2.raw, "false");
        assert!(manual.input_encoding_warning(&[2]).is_some());

        manual.input_bytes = 2;
        manual.validate()?;
        assert!(manual.input_encoding_warning(&[0, 2]).is_none());
    }
    let packed = parse(SENSOR, "synthetic record input".into())?;
    assert!(
        packed.layouts[0]
            .fields
            .iter()
            .any(|field| field.kind == Kind::Bool && field.bits == 1)
    );
    Ok(())
}

#[test]
fn octet_string_process_data_retains_fixed_widths_offsets_and_member_order() -> Result<()> {
    use crate::profile::Profile;
    let original =
        r#"<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>"#;
    let full = r#"<ProcessDataIn id="IN" bitLength="256"><Datatype xsi:type="OctetStringT" fixedLength="32"/><Name textId="octets"/></ProcessDataIn>"#;
    let packed = r#"<ProcessDataIn id="IN" bitLength="40"><Datatype xsi:type="RecordT" bitLength="40"><RecordItem subindex="1" bitOffset="8"><SimpleDatatype xsi:type="OctetStringT" fixedLength="4"/><Name textId="octets"/></RecordItem><RecordItem subindex="2" bitOffset="0"><SimpleDatatype xsi:type="BooleanT"/><Name textId="flag"/></RecordItem></Datatype><Name textId="record"/></ProcessDataIn>"#;
    let array = r#"<ProcessDataIn id="IN" bitLength="32"><Datatype xsi:type="ArrayT" count="2"><SimpleDatatype xsi:type="OctetStringT" fixedLength="2"/></Datatype><Name textId="array"/></ProcessDataIn>"#;
    let import = |layout: &str| -> Result<Profile> {
        let catalog = parse(
            &SENSOR.replace(original, layout),
            "synthetic process octets".into(),
        )?;
        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
        let mut profile = Profile::default();
        profile.apply_iodd(identity, imported);
        Profile::parse(&toml::to_string(&profile)?)
    };
    let profile = import(full)?;
    assert_eq!(profile.input_bytes, MAX_INPUT);
    let mut bytes = [0; MAX_INPUT];
    bytes[30] = 255;
    let decoded = profile.decode(&bytes)?;
    assert_eq!(decoded[0].0.bits, 256);
    assert_eq!(decoded[0].2.raw, crate::decode::hex(&bytes));
    assert_eq!(decoded[0].2.value.split(',').count(), MAX_INPUT);
    assert!(decoded[0].2.value.ends_with("0xFF,0x00"));
    assert!(profile.decode(&bytes[..31]).is_err());

    let profile = import(packed)?;
    let decoded = profile.decode(&[0, 0x80, 0xFF, 0, 1])?;
    assert_eq!(decoded[0].2.value, "0x00,0x80,0xFF,0x00");
    assert_eq!(decoded[1].2.value, "true");

    let profile = import(array)?;
    let decoded = profile.decode(&[0, 1, 0, 2])?;
    assert_eq!(decoded[0].2.value, "0x00,0x01");
    assert_eq!(decoded[1].2.value, "0x00,0x02");

    let bad_offset = packed.replace("bitOffset=\"8\"", "bitOffset=\"7\"");
    let profile = import(&bad_offset)?;
    let imported = profile.iodd.as_ref().unwrap();
    assert_eq!(imported.fields.len(), 1);
    assert!(
        imported
            .needs_input
            .iter()
            .any(|note| note.contains("byte-aligned"))
    );

    let bad_length = full.replace("fixedLength=\"32\"", "fixedLength=\"31\"");
    let profile = import(&bad_length)?;
    let imported = profile.iodd.as_ref().unwrap();
    assert!(imported.fields.is_empty());
    assert!(
        imported
            .needs_input
            .iter()
            .any(|note| note.contains("length differs"))
    );
    Ok(())
}

#[test]
fn string_process_data_uses_declared_encoding_and_isolates_bad_members() -> Result<()> {
    use crate::profile::Profile;
    let original =
        r#"<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>"#;
    let record = r#"<ProcessDataIn id="IN" bitLength="40"><Datatype xsi:type="RecordT" bitLength="40"><RecordItem subindex="1" bitOffset="8"><SimpleDatatype xsi:type="StringT" fixedLength="4" encoding="UTF-8"/><Name textId="label"/></RecordItem><RecordItem subindex="2" bitOffset="0"><SimpleDatatype xsi:type="BooleanT"/><Name textId="flag"/></RecordItem></Datatype><Name textId="record"/></ProcessDataIn>"#;
    let import = |layout: &str| -> Result<Profile> {
        let catalog = parse(
            &SENSOR.replace(original, layout),
            "synthetic process text".into(),
        )?;
        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
        let mut profile = Profile::default();
        profile.apply_iodd(identity, imported);
        Profile::parse(&toml::to_string(&profile)?)
    };
    let profile = import(record)?;
    let values = profile.decode(&[0xC3, 0xA9, 0, 0, 1])?;
    assert_eq!(values[0].0.kind, Kind::Utf8String);
    assert_eq!(values[0].2.value, "\"é\"");
    assert_eq!(values[1].2.value, "true");
    let values = profile.decode(b"A\0B\0\x01")?;
    assert!(values[0].2.encoding_error.is_some());
    assert_eq!(values[1].2.value, "true");
    let unsupported = import(&record.replace("UTF-8", "UTF-16"))?;
    assert_eq!(unsupported.effective_fields().len(), 1);
    assert!(
        unsupported
            .iodd
            .as_ref()
            .unwrap()
            .needs_input
            .iter()
            .any(|note| note.contains("encoding declaration"))
    );

    let array = r#"<ProcessDataIn id="IN" bitLength="32"><Datatype xsi:type="ArrayT" count="2"><SimpleDatatype xsi:type="StringT" fixedLength="2" encoding="US-ASCII"/></Datatype><Name textId="array"/></ProcessDataIn>"#;
    let profile = import(array)?;
    let values = profile.decode(b"AB\0\0")?;
    assert_eq!(values[0].0.kind, Kind::AsciiString);
    assert_eq!(values[0].2.value, "\"AB\"");
    assert_eq!(values[1].2.value, "\"\"");
    Ok(())
}

#[test]
fn temporal_process_data_imports_records_arrays_and_standalone_fields() -> Result<()> {
    use crate::profile::Profile;
    let original =
        r#"<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>"#;
    let record = r#"<ProcessDataIn id="IN" bitLength="128"><Datatype xsi:type="RecordT" bitLength="128"><RecordItem subindex="1" bitOffset="64"><SimpleDatatype xsi:type="TimeT"/><Name textId="clock"/></RecordItem><RecordItem subindex="2" bitOffset="0"><SimpleDatatype xsi:type="TimeSpanT"/><Name textId="duration"/></RecordItem></Datatype><Name textId="record"/></ProcessDataIn>"#;
    let import = |layout: &str| -> Result<Profile> {
        let catalog = parse(
            &SENSOR.replace(original, layout),
            "synthetic device time".into(),
        )?;
        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
        let mut profile = Profile::default();
        profile.apply_iodd(identity, imported);
        Profile::parse(&toml::to_string(&profile)?)
    };
    let profile = import(record)?;
    let payload = crate::decode::parse_hex("0000000000000001FFFFFFFFFFFFFFFF")?;
    let decoded = profile.decode(&payload)?;
    assert_eq!(decoded[0].0.kind, Kind::Time);
    assert_eq!(decoded[0].2.value, "2036-02-07 06:28:16.000 UTC");
    assert_eq!(decoded[0].2.raw, "1");
    assert_eq!(decoded[1].0.kind, Kind::TimeSpan);
    assert_eq!(decoded[1].2.raw, "-1");

    let misaligned = import(&record.replace("bitOffset=\"64\"", "bitOffset=\"63\""))?;
    let mapped = misaligned.iodd.as_ref().unwrap();
    assert_eq!(mapped.fields.len(), 1);
    assert!(
        mapped
            .needs_input
            .iter()
            .any(|note| note.contains("byte-aligned"))
    );

    let array = r#"<ProcessDataIn id="IN" bitLength="128"><Datatype xsi:type="ArrayT" count="2"><SimpleDatatype xsi:type="TimeSpanT"/></Datatype><Name textId="intervals"/></ProcessDataIn>"#;
    let profile = import(array)?;
    let decoded = profile.decode(&payload)?;
    assert_eq!(
        (decoded[0].2.raw.as_str(), decoded[1].2.raw.as_str()),
        ("1", "-1")
    );

    let standalone = r#"<ProcessDataIn id="IN" bitLength="64"><Datatype xsi:type="TimeT"/><Name textId="clock"/></ProcessDataIn>"#;
    let profile = import(standalone)?;
    assert_eq!(profile.decode(&payload[..8])?[0].0.bits, 64);
    assert!(
        import(&standalone.replace("64", "72"))?
            .effective_fields()
            .is_empty()
    );
    Ok(())
}

#[test]
fn typed_output_field_edits_keep_wire_width_and_neighboring_bits() -> Result<()> {
    let mut layout = ProcessDataLayout {
        id: "OUT".into(),
        bit_length: 72,
        packed: true,
        standalone_boolean: false,
        padded_integer: false,
        condition: None,
        fields: vec![Field {
            name: "Wide value".into(),
            bit_offset: 3,
            bits: 64,
            ..Field::default()
        }],
        needs_input: Vec::new(),
    };
    let zeros = [0xf8, 0, 0, 0, 0, 0, 0, 0, 7];
    assert_eq!(
        layout.edit_output_field(vec![0xff; 9], "Wide value", "0")?,
        zeros
    );
    assert_eq!(
        layout.edit_output_field(zeros.to_vec(), "Wide value", "18446744073709551615")?,
        [0xff; 9]
    );
    assert!(
        layout
            .edit_output_field(zeros.to_vec(), "Wide value", "18446744073709551616")
            .is_err()
    );

    layout.bit_length = 40;
    layout.fields[0].kind = Kind::Float32;
    layout.fields[0].bits = 32;
    assert_eq!(
        layout.edit_output_field(vec![0xff; 5], "Wide value", "-0")?,
        [0xfc, 0, 0, 0, 0x07]
    );
    assert!(
        layout
            .edit_output_field(vec![0xff; 5], "Wide value", "NaN")
            .is_err()
    );
    assert!(
        layout
            .edit_output_field(vec![0xff; 5], "Wide value", "1e300")
            .is_err()
    );

    layout.packed = false;
    layout.bit_length = 8;
    layout.fields[0].kind = Kind::Signed;
    layout.fields[0].bits = 8;
    layout.fields[0].bit_offset = 0;
    assert_eq!(
        layout.edit_output_field(vec![0x7f], "Wide value", "-128")?,
        [0x80]
    );
    assert!(
        layout
            .edit_output_field(vec![0x7f], "Wide value", "-129")
            .is_err()
    );

    // A singular one-bit value can require a full-octet representation.
    layout.bit_length = 1;
    layout.fields[0].kind = Kind::Bool;
    layout.fields[0].bits = 1;
    assert_eq!(layout.editable_output_fields().count(), 0);
    assert!(
        layout
            .edit_output_field(vec![0], "Wide value", "true")
            .is_err()
    );
    Ok(())
}

fn with_display(unit: &str) -> String {
    SENSOR.replace("<UserInterface>", "<UnusedInterface>").replace("</UserInterface>", "</UnusedInterface>").replace("</DeviceFunction>", &format!(r#"
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
    assert_eq!(threshold.displays[0].value, "52.0");
    assert_eq!(threshold.displays[0].unit, "mm");
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
    profile.iodd.as_mut().unwrap().parameters[0]
        .displays
        .clear();
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
    parameter.displays.clear();
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

fn definitions() -> Result<Vec<Parameter>> {
    Ok(parameters::load_standard(
        Path::new("samples/synthetic-standard-definitions.xml"),
        None,
        None,
    )?
    .parameters)
}

#[test]
fn standard_definitions_resolve_only_referenced_parameters_and_survive_reload() -> Result<()> {
    let xml = SENSOR.replace("<StdVariableRef id=\"V_ProductID\"/>", "<StdVariableRef id=\"V_ProductID\" fixedLengthRestriction=\"10\" defaultValue=\"SIM-SENSOR\"/>");
    let catalog =
        parse_with_definitions(&xml, "synthetic standard import".into(), &definitions()?)?;
    assert_eq!(catalog.parameters.len(), 4);
    assert!(catalog.standard_parameter_refs.is_empty());
    let product = &catalog.parameters[0];
    assert_eq!(product.fixed_length, Some(10));
    assert_eq!(product.name, "Synthetic product ID");
    assert!(product.origin.as_deref().unwrap().contains("synthetic-1"));
    let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let profile = crate::profile::Profile {
        expected_identity: identity,
        iodd: Some(imported),
        ..Default::default()
    };
    let restored = crate::profile::Profile::parse(&toml::to_string(&profile)?)?;
    let reading = crate::transport::read_parameter(&restored, "V_ProductID")?;
    assert_eq!(reading.raw_value.as_deref(), Some("SIM-SENSOR"));
    assert!(reading.displays.is_empty());
    assert!(
        product
            .decode_raw(b"too long for the declared length")
            .is_err()
    );
    Ok(())
}

#[test]
fn standard_command_choices_and_restrictions_do_not_enable_writes() -> Result<()> {
    let xml = SENSOR.replace("id=\"V_SIM_Command\" index=\"2\"", "id=\"V_SIM_Command\" index=\"3\"")
        .replace("</VariableCollection>", r#"<StdVariableRef id="V_SystemCommand"><StdSingleValueRef value="128"/><SingleValue value="200"><Name textId="nominal"/></SingleValue></StdVariableRef></VariableCollection>"#);
    let catalog = parse_with_definitions(&xml, "standard command fixture".into(), &definitions()?)?;
    let command = catalog
        .parameters
        .iter()
        .find(|p| p.id == "V_SystemCommand")
        .unwrap();
    assert_eq!(command.access, Access::WriteOnly);
    assert_eq!(command.choices[0].name, "Synthetic reset");
    assert_eq!(command.choices[1].name, "Nominal setting");
    let (_, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let mut profile = crate::profile::Profile {
        iodd: Some(imported),
        ..Default::default()
    };
    profile.connection.gateway = crate::profile::Gateway::IfmAl1350;
    profile.connection.endpoint = "must not open".into();
    assert!(
        crate::transport::read_parameter(&profile, "V_SystemCommand")
            .unwrap_err()
            .to_string()
            .contains("write-only")
    );
    assert!(
        parse_with_definitions(
            &xml.replace("value=\"128\"", "value=\"129\""),
            "bad choice".into(),
            &definitions()?
        )
        .is_err()
    );
    assert!(
        parse_with_definitions(
            &SENSOR.replace(
                "id=\"V_ProductID\"/>",
                "id=\"V_ProductID\" fixedLengthRestriction=\"65\"/>"
            ),
            "bad length".into(),
            &definitions()?
        )
        .is_err()
    );
    assert!(
        parse_with_definitions(
            &SENSOR.replace("index=\"65\"", "index=\"19\""),
            "index collision".into(),
            &definitions()?
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn distinct_menu_displays_remain_separate_and_invalid_metadata_keeps_raw_reads() -> Result<()> {
    let xml = SENSOR.replace("</MenuCollection>", r#"<Menu id="M_SIM_Other"><VariableRef variableId="V_SIM_Threshold" gradient="2" offset="-3" unitCode="1997" displayFormat="Dec.2"/></Menu></MenuCollection>"#);
    let catalog = parse(&xml, "conflicting menus".into())?;
    let parameter = &catalog.parameters[0];
    assert_eq!(parameter.displays.len(), 2);
    let values = parameter.decode_displays(&[1, 244])?;
    assert_eq!(values[0].id, "M_SIM_Settings/1");
    assert_eq!(values[0].value, "52.0");
    assert_eq!(values[1].value, "997.00");
    assert_eq!(values[1].unit, "");
    assert!(
        parameter
            .needs_input
            .iter()
            .any(|item| item.contains("Multiple"))
    );
    for bad in ["NaN", "inf", "invalid"] {
        let catalog = parse(
            &SENSOR.replace("gradient=\"0.1\"", &format!("gradient=\"{bad}\"")),
            "invalid scaling".into(),
        )?;
        let parameter = &catalog.parameters[0];
        assert!(parameter.displays.is_empty());
        assert!(!parameter.needs_input.is_empty());
        assert_eq!(parameter.decode_raw(&[1, 244])?.as_deref(), Some("500"));
    }
    let mut profile = imported_profile()?;
    profile.iodd.as_mut().unwrap().parameters[0].displays[0]
        .field
        .bits = 8;
    assert!(profile.validate().is_err());
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
