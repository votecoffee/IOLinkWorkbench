use super::*;

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
