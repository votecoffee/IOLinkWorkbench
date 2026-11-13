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
