use super::*;

fn sample() -> Result<Snapshot> {
    let catalog = crate::iodd::parse(
        include_str!("../../samples/sensor-formats.xml"),
        "original synthetic fixture".into(),
    )?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    let mut profile = Profile {
        input_bytes: imported.input_bytes,
        expected_identity,
        iodd: Some(imported),
        ..Profile::default()
    };
    profile.set_parameter_display("V_SIM_Threshold", 0, Some("M_SIM_Threshold/2"))?;
    Ok(read(&profile, "V_SIM_Threshold", 0)?.0)
}

#[test]
fn json_snapshot_preserves_exact_unsigned_64_bit_values_and_radix_views() -> Result<()> {
    let mut snapshot = sample()?;
    let parameter = snapshot
        .profile
        .iodd
        .as_mut()
        .unwrap()
        .parameters
        .iter_mut()
        .find(|p| p.id == "V_SIM_Threshold")
        .unwrap();
    parameter.datatype = "UIntegerT".into();
    parameter.bits = Some(64);
    for display in &mut parameter.displays {
        display.field.bits = 64;
        display.field.kind = crate::decode::Kind::Unsigned;
    }
    snapshot.raw_hex = "FFFFFFFFFFFFFFFF".into();
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("wide.json");
    snapshot.save(&path)?;
    let review = Snapshot::load(&path)?.review()?;
    assert_eq!(
        review.decoded.raw_value.as_deref(),
        Some("18446744073709551615")
    );
    assert_eq!(
        review.decoded.preferred_display.unwrap().value,
        "FFFFFFFFFFFFFFFFh"
    );
    Ok(())
}

#[test]
fn scalar_snapshot_round_trips_raw_profile_identity_time_and_all_views() -> Result<()> {
    let mut snapshot = sample()?;
    snapshot.profile.name = "測定 · mm".into();
    let folder = tempfile::tempdir()?;
    let file = folder.path().join("read.json");
    snapshot.save(&file)?;
    let bytes = std::fs::read(&file)?;
    assert!(bytes.ends_with(b"\n") && !bytes.contains(&b'\r'));
    let restored = Snapshot::load(&file)?;
    assert_eq!(restored.profile, snapshot.profile);
    assert_eq!(restored.host_read_utc, snapshot.host_read_utc);
    assert_eq!(restored.read_duration_us, snapshot.read_duration_us);
    assert_eq!(restored.observation.route, "synthetic simulator");
    assert_eq!(
        restored.observation.identity.serial.as_deref(),
        Some("SIM-0001")
    );
    let review = restored.review()?;
    assert_eq!(review.decoded.raw_hex, "01F4");
    assert_eq!(review.decoded.raw_value.as_deref(), Some("500"));
    assert_eq!(review.decoded.preferred_display.unwrap().value, "01F4h");
    assert_eq!(review.decoded.displays.len(), 3);
    assert!(!review.identity_check.is_empty());
    Ok(())
}

#[test]
fn whole_records_arrays_and_permitted_members_reopen_with_their_own_layouts() -> Result<()> {
    let snapshot = sample()?;
    for (id, subindex, raw, count) in [
        ("V_SIM_Record", 0, "FFEC010A54455354", 4),
        ("V_SIM_Record", 1, "FFEC", 0),
        ("V_SIM_History", 0, "02C495F0", 7),
        ("V_SIM_History", 2, "FC", 0),
    ] {
        let (snapshot, reading) = read(&snapshot.profile, id, subindex)?;
        let review = snapshot.review()?;
        assert_eq!(reading.raw_hex, raw);
        assert_eq!(review.decoded.raw_hex, raw);
        assert_eq!(
            review.decoded.record_items.len() + review.decoded.array_items.len(),
            count
        );
        assert!(!review.decoded.needs_review());
    }
    Ok(())
}

#[test]
fn malformed_parameter_responses_remain_saveable_for_offline_review() -> Result<()> {
    let mut snapshot = sample()?;
    snapshot.raw_hex = "FF".into();
    let folder = tempfile::tempdir()?;
    let file = folder.path().join("short.json");
    snapshot.save(&file)?;
    let review = Snapshot::load(&file)?.review()?;
    assert_eq!(review.decoded.raw_hex, "FF");
    assert!(review.decoded.needs_review());
    assert!(review.decoded.preferred_display.is_none());
    Ok(())
}

#[test]
fn existing_destinations_and_failed_validation_preserve_pinned_bytes() -> Result<()> {
    let mut snapshot = sample()?;
    let folder = tempfile::tempdir()?;
    let file = folder.path().join("existing.json");
    std::fs::write(&file, b"pinned original")?;
    assert!(snapshot.save(&file).is_err());
    assert_eq!(std::fs::read(&file)?, b"pinned original");
    snapshot.index = 19;
    let new = folder.path().join("invalid.json");
    assert!(snapshot.save(&new).is_err() && !new.exists());
    assert!(check_destination(&folder.path().join("missing/read.json")).is_err());
    Ok(())
}

#[test]
fn query_context_identity_version_and_raw_encoding_are_checked() -> Result<()> {
    let original = serde_json::to_value(sample()?)?;
    for (field, field_value) in [
        ("format_version", serde_json::json!(2)),
        ("index", serde_json::json!(19)),
        ("subindex", serde_json::json!(1)),
        ("parameter_id", serde_json::json!("missing")),
        (
            "host_read_utc",
            serde_json::json!("2026-11-12T10:00:00-05:00"),
        ),
        ("raw_hex", serde_json::json!("01f4")),
        ("raw_hex", serde_json::json!("")),
        ("raw_hex", serde_json::json!("FF".repeat(233))),
    ] {
        let mut document = original.clone();
        document[field] = field_value;
        assert!(
            serde_json::from_value::<Snapshot>(document)?
                .validate()
                .is_err()
        );
    }
    for changed in ["port", "gateway", "identity"] {
        let mut document = original.clone();
        document["observation"][changed] = match changed {
            "port" => serde_json::json!(2),
            "gateway" => serde_json::json!("ifm_al1350"),
            _ => serde_json::json!({"vendor_id": 123}),
        };
        assert!(
            serde_json::from_value::<Snapshot>(document)?
                .validate()
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn offline_review_never_opens_stored_endpoints_or_iodd_paths() -> Result<()> {
    let mut snapshot = sample()?;
    snapshot.profile.connection.gateway = Gateway::IfmAl1350;
    snapshot.profile.connection.endpoint = "http://127.0.0.1:1".into();
    snapshot.observation.gateway = Gateway::IfmAl1350;
    snapshot.observation.route = "AL1350 loopback fixture".into();
    snapshot.profile.iodd.as_mut().unwrap().source = "Z:/missing/vendor.zip".into();
    assert_eq!(snapshot.review()?.decoded.raw_value.as_deref(), Some("500"));
    Ok(())
}

#[test]
fn snapshot_json_rejects_unknown_fields_duplicates_truncation_and_oversized_input() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("bad.json");
    let text = serde_json::to_string(&sample()?)?;
    for bad in [
        text[..text.len() - 1].to_owned(),
        text.replacen('{', "{\"unknown\":1,", 1),
        text.replacen('{', "{\"format_version\":1,", 1),
    ] {
        std::fs::write(&path, bad)?;
        assert!(Snapshot::load(&path).is_err());
    }
    std::fs::write(&path, vec![b' '; MAX_BYTES + 1])?;
    assert!(
        Snapshot::load(&path)
            .unwrap_err()
            .to_string()
            .contains("16 MiB")
    );
    Ok(())
}
