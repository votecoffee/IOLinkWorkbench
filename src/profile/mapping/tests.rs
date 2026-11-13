use super::*;
use crate::{
    decode::{ByteOrder, Kind},
    profile::LogSelection,
};

fn imported() -> Result<Profile> {
    let catalog = crate::iodd::parse(
        include_str!("../../../samples/sensor-formats.xml"),
        "synthetic field edit fixture".into(),
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
fn overrides_preserve_the_import_and_removal_restores_the_complete_profile() -> Result<()> {
    let profile = imported()?;
    let original = profile.clone();
    let mut field = profile.effective_fields()[0].0.clone();
    field.scale = 0.25;
    field.unit = "changed unit".into();
    let overridden = profile.with_field(field.clone())?;
    assert_eq!(overridden.effective_fields()[0], (&field, "override"));
    assert_eq!(overridden.iodd, original.iodd);
    assert_eq!(overridden.connection, original.connection);
    assert_eq!(overridden.expected_identity, original.expected_identity);
    assert_eq!(overridden.logging, original.logging);
    assert_eq!(overridden.without_manual_field(&field.name)?, original);
    assert!(original.without_manual_field(&field.name).is_err());
    assert_eq!(profile, original);
    Ok(())
}

#[test]
fn manual_fields_update_in_place_without_reordering_csv_selection() -> Result<()> {
    let profile = Profile::default().with_field(Field {
        name: "first".into(),
        ..Default::default()
    })?;
    let second = Field {
        name: "µm, second".into(),
        bit_offset: 8,
        bits: 8,
        kind: Kind::Signed,
        scale: 0.5,
        offset: -1.0,
        unit: "µm".into(),
        decimals: Some(2),
        ..Default::default()
    };
    let mut profile = profile.with_field(second.clone())?;
    profile.set_log_selection(LogSelection::Fields(vec![
        second.name.clone(),
        "first".into(),
    ]))?;
    let mut edited = second.clone();
    edited.unit = "new unit".into();
    let candidate = profile.with_field(edited)?;
    assert_eq!(
        candidate
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "µm, second"]
    );
    assert_eq!(candidate.logging, profile.logging);
    assert_eq!(candidate.log_fields()[0].name, second.name);
    let value = candidate.fields[1].decode(&[0xFF, 0x02])?;
    assert_eq!((value.raw.as_str(), value.value.as_str()), ("-1", "-1.50"));
    assert_eq!(profile.fields[1], second);
    Ok(())
}

#[test]
fn malformed_fields_reject_the_candidate_without_changing_saved_settings() -> Result<()> {
    let profile = imported()?;
    let before = profile.clone();
    let valid = Field {
        name: "manual".into(),
        ..Default::default()
    };
    for field in [
        Field {
            name: "".into(),
            ..valid.clone()
        },
        Field {
            name: "bad\nname".into(),
            ..valid.clone()
        },
        Field {
            bits: 0,
            ..valid.clone()
        },
        Field {
            bits: 65,
            ..valid.clone()
        },
        Field {
            bit_offset: 16,
            ..valid.clone()
        },
        Field {
            scale: f64::NAN,
            ..valid.clone()
        },
        Field {
            offset: f64::INFINITY,
            ..valid.clone()
        },
        Field {
            kind: Kind::Bool,
            ..valid.clone()
        },
        Field {
            kind: Kind::Bool,
            bits: 1,
            scale: 2.0,
            ..valid.clone()
        },
        Field {
            kind: Kind::Float32,
            bits: 16,
            ..valid.clone()
        },
        Field {
            decimals: Some(13),
            ..valid.clone()
        },
    ] {
        assert!(profile.with_field(field).is_err());
        assert_eq!(profile, before);
    }
    Ok(())
}

#[test]
fn removing_selected_manual_fields_requires_an_explicit_csv_change() -> Result<()> {
    let mut profile = Profile::default().with_field(Field {
        name: "selected".into(),
        ..Default::default()
    })?;
    profile.set_log_selection(LogSelection::Fields(vec!["selected".into()]))?;
    assert!(profile.without_manual_field("selected").is_err());
    assert_eq!(profile.fields.len(), 1);
    profile.set_log_selection(LogSelection::RawOnly)?;
    let removed = profile.without_manual_field("selected")?;
    assert!(removed.fields.is_empty() && removed.logging.raw_only);
    let mut base = imported()?;
    let mut field = base.effective_fields()[0].0.clone();
    base.set_log_selection(LogSelection::Fields(vec![field.name.clone()]))?;
    field.unit = "override".into();
    let overridden = base.with_field(field.clone())?;
    assert_eq!(overridden.without_manual_field(&field.name)?, base);
    Ok(())
}

#[test]
fn manual_type_and_order_settings_retain_exact_raw_and_engineering_views() -> Result<()> {
    let profile = Profile {
        input_bytes: 8,
        ..Default::default()
    };
    let wide = Field {
        name: "wide".into(),
        bits: 64,
        decimals: Some(2),
        ..Default::default()
    };
    let candidate = profile.with_field(wide)?;
    let value = candidate.fields[0].decode(&[0xFF; 8])?;
    assert_eq!(value.raw, "18446744073709551615");
    assert_eq!(value.value, "18446744073709551615.00");
    let float = Field {
        name: "float".into(),
        bits: 32,
        kind: Kind::Float32,
        byte_order: ByteOrder::LittleEndian,
        scale: 2.0,
        offset: -1.0,
        decimals: Some(3),
        ..Default::default()
    };
    let candidate = profile.with_field(float)?;
    let value = candidate.fields[0].decode(&[0x00, 0x00, 0xC0, 0x3F, 0, 0, 0, 0])?;
    assert_eq!((value.raw.as_str(), value.value.as_str()), ("1.5", "2.000"));
    Ok(())
}
