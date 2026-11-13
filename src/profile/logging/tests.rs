use super::*;
use crate::{decode::Field, profile::LogSelection};

fn mapped() -> Result<Profile> {
    let catalog = crate::iodd::parse(
        include_str!("../../../samples/sensor-formats.xml"),
        "synthetic logging fixture".into(),
    )?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    let mut profile = Profile {
        input_bytes: imported.input_bytes,
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    profile.set_parameter_display("V_SIM_Threshold", 0, Some("M_SIM_Threshold/2"))?;
    Ok(profile)
}

#[test]
fn saved_logging_candidates_preserve_every_other_profile_setting() -> Result<()> {
    let profile = mapped()?.with_field(Field {
        name: "auxiliary".into(),
        ..Default::default()
    })?;
    let before = profile.clone();
    let settings = Logging {
        raw_only: true,
        mode: LogMode::OnChange,
        queue_capacity: 17,
        rotate_rows: 7,
        rotate_seconds: 3,
        fields: Vec::new(),
    };
    let mut candidate = profile.with_logging(settings.clone())?;
    assert_eq!(candidate.logging, settings);
    candidate.logging = before.logging.clone();
    assert_eq!(candidate, before);
    assert_eq!(profile, before);
    Ok(())
}

#[test]
fn invalid_logging_choices_reject_the_candidate_without_mutation() -> Result<()> {
    let profile = mapped()?;
    let before = profile.clone();
    let name = profile.effective_fields()[0].0.name.clone();
    for settings in [
        Logging {
            queue_capacity: 0,
            ..Default::default()
        },
        Logging {
            queue_capacity: 1_000_001,
            ..Default::default()
        },
        Logging {
            raw_only: true,
            fields: vec![name.clone()],
            ..Default::default()
        },
        Logging {
            fields: vec!["missing".into()],
            ..Default::default()
        },
        Logging {
            fields: vec![name.clone(), name],
            ..Default::default()
        },
        Logging {
            rotate_rows: u64::MAX,
            ..Default::default()
        },
        Logging {
            rotate_seconds: u64::MAX,
            ..Default::default()
        },
    ] {
        assert!(profile.with_logging(settings).is_err());
        assert_eq!(profile, before);
    }
    profile.with_logging(Logging {
        rotate_rows: i64::MAX as u64,
        rotate_seconds: i64::MAX as u64,
        ..Default::default()
    })?;
    Ok(())
}

#[test]
fn plan_retains_explicit_order_and_effective_mapping_origins() -> Result<()> {
    let profile = mapped()?;
    let mut base = profile.effective_fields()[0].0.clone();
    base.unit = "overridden unit".into();
    let name = base.name.clone();
    let mut profile = profile.with_field(base)?.with_field(Field {
        name: "aux, µm".into(),
        ..Default::default()
    })?;
    profile.set_log_selection(LogSelection::Fields(vec!["aux, µm".into(), name.clone()]))?;
    let plan = profile.logging_plan()?;
    assert_eq!(plan.selection, "explicit_order");
    assert_eq!(
        plan.selected_fields
            .iter()
            .map(|field| (field.position, field.mapping.name.as_str(), field.origin))
            .collect::<Vec<_>>(),
        vec![(1, "aux, µm", "manual"), (2, name.as_str(), "override")]
    );
    assert_eq!(plan.selected_fields[1].mapping.unit, "overridden unit");
    assert_eq!(plan.csv_columns, crate::capture::csv_header(&profile));
    let text = toml::to_string_pretty(&plan)?;
    assert!(text.contains("saved configuration only"));
    assert!(plan.text().contains("explicit_order"));
    Ok(())
}

#[test]
fn raw_only_and_unmapped_plans_keep_metadata_without_inventing_values() -> Result<()> {
    let mut profile = mapped()?;
    profile.set_log_selection(LogSelection::RawOnly)?;
    let plan = profile.logging_plan()?;
    assert_eq!(plan.selection, "raw_only");
    assert!(plan.selected_fields.is_empty());
    assert_eq!(plan.csv_columns, crate::capture::CSV_METADATA);
    let empty = Profile::default();
    let plan = empty.logging_plan()?;
    assert_eq!(plan.selection, "all_mapped");
    assert!(plan.selected_fields.is_empty());
    assert_eq!(plan.csv_columns, crate::capture::CSV_METADATA);
    Ok(())
}

#[test]
fn configured_snapshot_and_counter_expectations_are_not_observed_rates() -> Result<()> {
    let mut profile = Profile::default();
    let plan = profile.logging_plan()?;
    assert_eq!(plan.configured_row_kind, "source_update");
    assert!(plan.source_update_loss.contains("synthetic"));
    profile.connection.simulate_snapshots = true;
    assert_eq!(profile.logging_plan()?.configured_row_kind, "host_poll");
    profile.connection.gateway = Gateway::IfmAl1350;
    profile.connection.endpoint = "http://127.0.0.1:9".into();
    let plan = profile.logging_plan()?;
    assert_eq!(plan.configured_row_kind, "host_poll");
    assert!(plan.source_update_loss.contains("unknown"));
    assert!(plan.basis.contains("no gateway accessed"));
    profile.logging.mode = LogMode::OnChange;
    let plan = profile.logging_plan()?;
    assert!(plan.row_eligibility.contains("complete raw payload"));
    assert!(plan.rotation_rule.contains("receive elapsed timestamps"));
    Ok(())
}

#[test]
fn explicit_logging_reset_allows_a_selected_manual_mapping_to_be_removed() -> Result<()> {
    let mut profile = Profile::default().with_field(Field {
        name: "selected".into(),
        ..Default::default()
    })?;
    profile.set_log_selection(LogSelection::Fields(vec!["selected".into()]))?;
    assert!(profile.without_manual_field("selected").is_err());
    let all = profile.with_logging(Logging {
        fields: Vec::new(),
        ..profile.logging.clone()
    })?;
    assert!(all.without_manual_field("selected")?.fields.is_empty());
    assert_eq!(profile.logging.fields, vec!["selected"]);
    Ok(())
}
