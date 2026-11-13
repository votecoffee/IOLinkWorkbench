use super::*;

fn format_profile() -> Result<Profile> {
    let catalog = crate::iodd::parse(
        include_str!("../../samples/sensor-formats.xml"),
        "synthetic display preferences".into(),
    )?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    Ok(Profile {
        input_bytes: imported.input_bytes,
        expected_identity,
        iodd: Some(imported),
        ..Profile::default()
    })
}

#[test]
fn preferred_scalar_display_round_trips_without_changing_raw_or_other_views() -> Result<()> {
    let mut profile = format_profile()?;
    let before = profile.clone();
    assert!(!toml::to_string_pretty(&profile)?.contains("preferred_display"));
    profile.set_parameter_display("V_SIM_Threshold", 0, Some("M_SIM_Threshold/2"))?;
    let reading = crate::transport::read_parameter(&profile, "V_SIM_Threshold")?;
    assert_eq!(reading.raw_hex, "01F4");
    assert_eq!(reading.raw_value.as_deref(), Some("500"));
    assert_eq!(reading.preferred_display.as_ref().unwrap().value, "01F4h");
    assert_eq!(reading.displays.len(), 3);
    assert_eq!(profile.connection, before.connection);
    assert_eq!(profile.logging, before.logging);
    assert_eq!(profile.expected_identity, before.expected_identity);
    assert_eq!(profile.effective_fields(), before.effective_fields());
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("preferred.toml");
    profile.save(&path)?;
    assert_eq!(Profile::load(&path)?, profile);
    profile.set_parameter_display("V_SIM_Threshold", 0, None)?;
    assert_eq!(profile, before);
    Ok(())
}

#[test]
fn record_member_preference_applies_to_whole_record_and_individual_read() -> Result<()> {
    let mut profile = format_profile()?;
    profile.set_parameter_display("V_SIM_Record", 1, Some("M_SIM_Record/2"))?;
    profile.set_parameter_display("V_SIM_Record", 2, Some("M_SIM_Record/4"))?;
    let whole = crate::transport::read_parameter(&profile, "V_SIM_Record")?;
    assert!(whole.preferred_display.is_none());
    assert_eq!(whole.raw_hex, "FFEC010A54455354");
    let signed = &whole.record_items[0].value;
    assert_eq!(signed.raw_value.as_deref(), Some("-20"));
    assert_eq!(signed.preferred_display.as_ref().unwrap().value, "FFECh");
    assert_eq!(signed.displays.len(), 3);
    assert!(signed.text().contains("M_SIM_Record/2 (preferred)"));
    assert_eq!(
        whole.record_items[1]
            .value
            .preferred_display
            .as_ref()
            .unwrap()
            .value,
        "1"
    );
    let member = crate::transport::read_parameter_at(&profile, "V_SIM_Record", 1)?;
    assert_eq!(member.preferred_display.unwrap().value, "FFECh");
    Ok(())
}

#[test]
fn array_preference_is_shared_by_all_elements_and_permitted_member_reads() -> Result<()> {
    let mut profile = format_profile()?;
    profile.set_parameter_display("V_SIM_History", 0, Some("M_SIM_History/2"))?;
    let whole = crate::transport::read_parameter(&profile, "V_SIM_History")?;
    assert!(whole.preferred_display.is_none());
    assert_eq!(whole.array_items.len(), 7);
    for item in &whole.array_items {
        assert_eq!(
            item.value.preferred_display.as_ref().unwrap().id,
            "M_SIM_History/2"
        );
        assert_eq!(item.value.displays.len(), 2);
    }
    let member = crate::transport::read_parameter_at(&profile, "V_SIM_History", 2)?;
    assert_eq!(member.raw_value.as_deref(), Some("-4"));
    assert_eq!(member.preferred_display.unwrap().value, "1111 1100b");
    profile.set_parameter_display("V_SIM_History", 7, None)?;
    assert!(
        profile
            .parameter("V_SIM_History")?
            .array
            .as_ref()
            .unwrap()
            .element
            .preferred_display
            .is_none()
    );
    Ok(())
}

#[test]
fn invalid_display_ids_and_targets_leave_the_profile_unchanged() -> Result<()> {
    let mut profile = format_profile()?;
    profile.set_parameter_display("V_SIM_Threshold", 0, Some("M_SIM_Threshold/2"))?;
    let before = profile.clone();
    for (id, subindex, display) in [
        ("missing", 0, Some("M_SIM_Threshold/2")),
        ("V_SIM_Threshold", 0, Some("M_SIM_Record/2")),
        ("V_SIM_Threshold", 1, None),
        ("V_SIM_Record", 0, Some("M_SIM_Record/2")),
        ("V_SIM_Record", 1, Some("M_SIM_Threshold/2")),
        ("V_SIM_Record", 5, None),
        ("V_SIM_History", 8, None),
        ("V_SIM_History", 1, Some("")),
        ("V_SIM_History", 1, Some("M_SIM_Record/2")),
    ] {
        assert!(
            profile
                .set_parameter_display(id, subindex, display)
                .is_err()
        );
        assert_eq!(profile, before);
    }
    // A stale ID in hand-edited TOML is rejected by normal profile loading too.
    let text = toml::to_string_pretty(&profile)?.replace(
        "preferred_display = \"M_SIM_Threshold/2\"",
        "preferred_display = \"missing\"",
    );
    assert!(Profile::parse(&text).is_err());
    Ok(())
}

#[test]
fn failed_payload_decode_retains_raw_bytes_without_a_fabricated_preferred_value() -> Result<()> {
    let mut profile = format_profile()?;
    profile.set_parameter_display("V_SIM_Threshold", 0, Some("M_SIM_Threshold/2"))?;
    let decoded = profile.parameter("V_SIM_Threshold")?.decode(&[0xFF]);
    assert_eq!(decoded.raw_hex, "FF");
    assert!(decoded.decode_error.is_some());
    assert!(decoded.preferred_display.is_none() && decoded.displays.is_empty());
    Ok(())
}

#[test]
fn host_display_choice_does_not_require_or_grant_individual_array_read_access() -> Result<()> {
    let mut profile = format_profile()?;
    let array = profile
        .iodd
        .as_mut()
        .unwrap()
        .parameters
        .iter_mut()
        .find(|p| p.id == "V_SIM_History")
        .unwrap()
        .array
        .as_mut()
        .unwrap();
    array.subindex_access = false;
    profile.set_parameter_display("V_SIM_History", 2, Some("M_SIM_History/1"))?;
    assert!(!profile.parameter("V_SIM_History")?.subindex_access());
    assert!(crate::transport::read_parameter_at(&profile, "V_SIM_History", 2).is_err());
    let decoded = profile
        .parameter("V_SIM_History")?
        .decode(&[0x02, 0xC4, 0x95, 0xF0]);
    assert_eq!(
        decoded.array_items[1]
            .value
            .preferred_display
            .as_ref()
            .unwrap()
            .value,
        "FCh"
    );
    Ok(())
}

#[test]
fn distinct_field_names_cannot_create_ambiguous_selected_csv_columns() -> Result<()> {
    let mut profile = Profile {
        fields: vec![
            Field {
                name: "Level".into(),
                unit: "mm".into(),
                ..Default::default()
            },
            Field {
                name: "Level [mm]".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let header = crate::capture::csv_header(&profile);
    assert_eq!(header[12], header[13]);
    assert!(
        profile
            .validate()
            .unwrap_err()
            .to_string()
            .contains("CSV column label collides")
    );
    // Selecting one interpretation removes the ambiguity without renaming a saved field.
    profile.logging.fields = vec!["Level".into()];
    profile.validate()?;
    let parsed = Profile::parse(&toml::to_string_pretty(&profile)?)?;
    assert_eq!(parsed.log_fields().len(), 1);
    assert_eq!(
        crate::capture::csv_header(&parsed)
            .last()
            .map(String::as_str),
        Some("value.Level [mm]")
    );
    Ok(())
}

fn sample() -> Result<Profile> {
    Profile::parse(include_str!("../../samples/simulator.toml"))
}

#[test]
fn raw_only_round_trips_while_mapping_and_typed_preview_remain_available() -> Result<()> {
    let mut profile = sample()?;
    let before = profile.clone();
    assert!(!toml::to_string_pretty(&profile)?.contains("raw_only"));
    profile.set_log_selection(LogSelection::RawOnly)?;
    assert!(profile.logging.raw_only && profile.logging.fields.is_empty());
    assert!(profile.log_fields().is_empty());
    assert_eq!(
        crate::capture::csv_header(&profile).len(),
        crate::capture::CSV_METADATA.len()
    );
    assert_eq!(profile.fields, before.fields);
    assert_eq!(profile.connection, before.connection);
    assert_eq!(profile.expected_identity, before.expected_identity);
    assert_eq!(
        profile.decode(&[0xFF, 0xF9])?,
        before.decode(&[0xFF, 0xF9])?
    );
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("raw-only.toml");
    profile.save(&path)?;
    assert_eq!(Profile::load(&path)?, profile);
    profile.set_log_selection(LogSelection::All)?;
    assert!(!profile.logging.raw_only && profile.logging.fields.is_empty());
    assert_eq!(profile.log_fields().len(), 2);
    assert!(!toml::to_string_pretty(&profile)?.contains("raw_only"));
    Ok(())
}

#[test]
fn selection_order_and_failed_changes_preserve_existing_logging_settings() -> Result<()> {
    let mut profile = sample()?;
    profile.logging.mode = LogMode::OnChange;
    profile.logging.rotate_rows = 11;
    profile.set_log_selection(LogSelection::Fields(vec!["switch".into(), "level".into()]))?;
    let header = crate::capture::csv_header(&profile);
    assert_eq!(&header[12..], &["value.switch", "value.level [mm]"]);
    let before = profile.clone();
    for names in [
        vec!["missing".into()],
        vec!["level".into(), "level".into()],
        Vec::new(),
    ] {
        assert!(
            profile
                .set_log_selection(LogSelection::Fields(names))
                .is_err()
        );
        assert_eq!(profile, before);
    }
    assert_eq!(profile.logging.mode, LogMode::OnChange);
    assert_eq!(profile.logging.rotate_rows, 11);
    Ok(())
}

#[test]
fn conflicting_raw_and_typed_settings_are_rejected() -> Result<()> {
    let mut profile = sample()?;
    profile.logging.raw_only = true;
    assert!(
        profile
            .validate()
            .unwrap_err()
            .to_string()
            .contains("raw-only logging cannot")
    );
    assert!(Profile::parse(&toml::to_string_pretty(&profile)?).is_err());
    profile.logging.fields.clear();
    profile.validate()?;
    Ok(())
}

#[test]
fn raw_only_can_keep_ambiguous_labels_until_a_safe_subset_is_selected() -> Result<()> {
    let mut profile = Profile {
        fields: vec![
            Field {
                name: "Level".into(),
                unit: "mm".into(),
                ..Default::default()
            },
            Field {
                name: "Level [mm]".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    profile.set_log_selection(LogSelection::RawOnly)?;
    let before = profile.clone();
    assert!(profile.set_log_selection(LogSelection::All).is_err());
    assert_eq!(profile, before);
    profile.set_log_selection(LogSelection::Fields(vec!["Level".into()]))?;
    assert!(!profile.logging.raw_only);
    assert_eq!(profile.log_fields().len(), 1);
    Ok(())
}

#[test]
fn imported_overrides_and_manual_fields_keep_their_order_and_provenance() -> Result<()> {
    let catalog = crate::iodd::load(std::path::Path::new("samples/sensor.xml"), None)?;
    let (expected_identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let name = imported.fields[0].name.clone();
    let mut overridden = imported.fields[0].clone();
    overridden.scale = 0.25;
    overridden.unit = "cm".into();
    let mut profile = Profile {
        expected_identity,
        iodd: Some(imported),
        fields: vec![
            overridden,
            Field {
                name: "Manual, \"flag\"".into(),
                kind: crate::decode::Kind::Bool,
                bits: 1,
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    profile.set_log_selection(LogSelection::Fields(vec![
        "Manual, \"flag\"".into(),
        name.clone(),
    ]))?;
    let fields = profile.log_fields();
    assert_eq!(fields[0].name, "Manual, \"flag\"");
    assert_eq!(fields[1].unit, "cm");
    assert_eq!(fields[1].scale, 0.25);
    let before = profile.clone();
    profile.set_log_selection(LogSelection::RawOnly)?;
    assert_eq!(profile.iodd, before.iodd);
    assert_eq!(profile.fields, before.fields);
    profile.set_log_selection(LogSelection::Fields(vec![name]))?;
    assert_eq!(Profile::parse(&toml::to_string_pretty(&profile)?)?, profile);
    Ok(())
}
