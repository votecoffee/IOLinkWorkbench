use super::*;

fn mapped_profile() -> Result<Profile> {
    let catalog = crate::iodd::parse(
        include_str!("../../../samples/sensor-formats.xml"),
        "synthetic identity fixture".into(),
    )?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    let mut profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    profile.set_parameter_display("V_SIM_Threshold", 0, Some("M_SIM_Threshold/2"))?;
    Ok(profile)
}

#[test]
fn drafts_retain_unset_empty_unicode_and_space_sensitive_pins() -> Result<()> {
    let identity = Identity {
        vendor_id: Some(0),
        device_id: Some(0xFF_FFFF),
        product_id: Some(String::new()),
        product_name: Some("  µm sensor  ".into()),
        serial: None,
    };
    let draft = IdentityDraft::from(&identity);
    assert_eq!(draft.parse()?, identity);
    assert_eq!(draft.product_id.as_deref(), Some(""));
    assert!(draft.serial.is_none());
    let profile = Profile::default().with_identity(identity.clone())?;
    assert_eq!(Profile::parse(&toml::to_string_pretty(&profile)?)?, profile);
    let empty = Identity {
        product_id: Some(String::new()),
        ..Default::default()
    };
    assert_eq!(empty.verify(&Identity::default())?, "identity unavailable");
    assert_eq!(
        empty.verify(&empty)?,
        "matched all configured identity fields"
    );
    Ok(())
}

#[test]
fn decimal_and_hexadecimal_ids_are_checked_without_guessing_blank_values() -> Result<()> {
    let draft = IdentityDraft {
        vendor_id: Some(" 0Xffff ".into()),
        device_id: Some("0x00FFFFFF".into()),
        ..Default::default()
    };
    let identity = draft.parse()?;
    assert_eq!(identity.vendor_id, Some(65535));
    assert_eq!(identity.device_id, Some(0xFF_FFFF));
    for invalid in [
        "",
        " ",
        "0x",
        "0xGG",
        "+1",
        "-1",
        "1_0",
        "0b1",
        "1 2",
        "18446744073709551616",
    ] {
        let bad = IdentityDraft {
            vendor_id: Some(invalid.into()),
            ..Default::default()
        };
        assert!(bad.parse().is_err(), "{invalid:?}");
    }
    for (vendor_id, device_id) in [(Some("65536"), None), (None, Some("0x1000000"))] {
        assert!(
            IdentityDraft {
                vendor_id: vendor_id.map(str::to_owned),
                device_id: device_id.map(str::to_owned),
                ..Default::default()
            }
            .parse()
            .is_err()
        );
    }
    assert_eq!(
        IdentityDraft {
            device_id: Some("000001".into()),
            ..Default::default()
        }
        .parse()?
        .device_id,
        Some(1)
    );
    Ok(())
}

#[test]
fn identity_candidates_preserve_mapping_connection_preferences_and_logging() -> Result<()> {
    let mut profile = mapped_profile()?;
    profile.connection.endpoint = "retained inactive endpoint".into();
    profile.logging.raw_only = true;
    let before = profile.clone();
    let mut candidate = profile.with_identity(Identity {
        vendor_id: Some(12),
        device_id: Some(34),
        serial: Some("unit-7".into()),
        ..Default::default()
    })?;
    candidate.expected_identity = before.expected_identity.clone();
    assert_eq!(candidate, before);
    assert_eq!(profile, before);
    assert!(
        profile
            .with_identity(Identity {
                device_id: Some(0x100_0000),
                ..Default::default()
            })
            .is_err()
    );
    assert_eq!(profile, before);
    Ok(())
}

#[test]
fn configured_pins_do_not_turn_missing_observations_into_matches() -> Result<()> {
    let expected = Identity {
        vendor_id: Some(65535),
        device_id: Some(1),
        product_id: Some("SIM-SENSOR".into()),
        product_name: Some("Simulated level sensor".into()),
        serial: Some("SIM-0001".into()),
    };
    assert_eq!(
        expected.verify(&expected)?,
        "matched all configured identity fields"
    );
    assert_eq!(
        expected.verify(&Identity::default())?,
        "identity unavailable"
    );
    assert_eq!(
        expected.verify(&Identity {
            vendor_id: Some(65535),
            ..Default::default()
        })?,
        "partial identity match"
    );
    for mismatch in [
        Identity {
            vendor_id: Some(1),
            ..expected.clone()
        },
        Identity {
            device_id: Some(2),
            ..expected.clone()
        },
        Identity {
            product_id: Some("OTHER".into()),
            ..expected.clone()
        },
        Identity {
            product_name: Some("other name".into()),
            ..expected.clone()
        },
        Identity {
            serial: Some("SIM-0002".into()),
            ..expected.clone()
        },
    ] {
        assert!(expected.verify(&mismatch).is_err());
    }
    assert_eq!(
        Identity::default().verify(&expected)?,
        "no identity pin configured"
    );
    Ok(())
}

#[test]
fn a_product_pin_conflict_is_visible_without_rewriting_the_selected_iodd() -> Result<()> {
    let profile = mapped_profile()?;
    let mapping = profile.iodd.clone();
    assert!(profile.identity_variant_note().is_none());
    let candidate = profile.with_identity(Identity {
        product_id: Some("OTHER".into()),
        ..profile.expected_identity.clone()
    })?;
    assert!(candidate.identity_variant_note().is_some());
    assert_eq!(candidate.iodd, mapping);
    assert!(
        candidate
            .with_identity(Identity::default())?
            .identity_variant_note()
            .is_none()
    );
    Ok(())
}

#[test]
fn saved_identity_edits_keep_the_existing_profile_schema_and_source_bytes() -> Result<()> {
    let profile = mapped_profile()?;
    let folder = tempfile::tempdir()?;
    let source = folder.path().join("source.toml");
    profile.save_new(&source)?;
    let before = std::fs::read(&source)?;
    let candidate = profile.with_identity(Identity {
        serial: Some("  µ-unit  ".into()),
        ..profile.expected_identity.clone()
    })?;
    let output = folder.path().join("pinned.toml");
    candidate.save_new(&output)?;
    assert_eq!(Profile::load(&output)?, candidate);
    assert_eq!(std::fs::read(&source)?, before);
    assert!(candidate.save_new(&source).is_err());
    assert_eq!(std::fs::read(&source)?, before);
    Ok(())
}
