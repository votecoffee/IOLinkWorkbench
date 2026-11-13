use super::*;
use crate::iodd::{self, EventType, LoadOptions};
use std::{fs, path::Path};

fn profile() -> Result<Profile> {
    let catalog = iodd::load(Path::new("samples/sensor.xml"), None)?;
    let (expected_identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    Ok(Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    })
}

#[test]
fn event_catalogs_preserve_translated_vendor_names_and_resolve_standard_references() -> Result<()> {
    let catalog = iodd::load_with_options(
        Path::new("samples/sensor.xml"),
        &LoadOptions {
            language: Some("de"),
            definitions: Some(Path::new("samples/synthetic-standard-definitions.xml")),
            ..Default::default()
        },
    )?;
    assert_eq!(catalog.events.len(), 2);
    assert!(catalog.standard_event_refs.is_empty());
    assert_eq!(catalog.events[0].name, "Synthetische Bereichswarnung");
    assert!(catalog.events[1].origin.is_some());
    let (expected_identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
    let profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    let restored = Profile::parse(&toml::to_string_pretty(&profile)?)?;
    assert_eq!(restored, profile);
    assert_eq!(
        restored.iodd.unwrap().events[0].event_type,
        EventType::Warning
    );
    Ok(())
}

#[test]
fn qualifier_fields_and_empty_slots_are_decoded_in_network_order() -> Result<()> {
    let profile = profile()?;
    let decoded = decode_detailed(&profile, &[0xE4, 0x8D, 0xFE, 0, 0, 0]);
    assert_eq!(decoded.decode_error, None);
    assert_eq!(decoded.empty_slots, 1);
    let event = &decoded.events[0];
    assert_eq!(
        (
            event.slot,
            event.instance,
            event.source,
            event.event_type,
            event.mode
        ),
        (1, 4, 0, 2, 3)
    );
    assert_eq!(event.code_hex, "0x8DFE");
    assert!(event.pending_shape && event.review.is_empty());
    assert_eq!(
        event.definition.as_ref().unwrap().name,
        "Synthetic range warning"
    );
    let empty = decode_detailed(&profile, &[0; 192]);
    assert_eq!(empty.empty_slots, 64);
    assert!(empty.events.is_empty());
    Ok(())
}

#[test]
fn source_type_mode_and_unknown_codes_keep_their_raw_evidence() -> Result<()> {
    let profile = profile()?;
    let master = decode_detailed(&profile, &[0xEC, 0x8D, 0xFE]);
    assert_eq!(master.events[0].source_name, "Master/Port");
    assert!(master.events[0].definition.is_none());
    assert!(!master.events[0].pending_shape);
    let mismatch = decode_detailed(&profile, &[0xF4, 0x8D, 0xFE]);
    assert!(
        mismatch.events[0]
            .review
            .iter()
            .any(|note| note.contains("differs"))
    );
    let disappearing = decode_detailed(&profile, &[0xA4, 0x8D, 0xFE]);
    assert_eq!(disappearing.events[0].mode_name, "Disappears");
    assert!(!disappearing.events[0].pending_shape);
    let unknown = decode_detailed(&profile, &[0xE4, 0x12, 0x34]);
    assert_eq!(unknown.events[0].raw_hex, "E41234");
    assert!(unknown.events[0].definition.is_none() && !unknown.events[0].review.is_empty());
    Ok(())
}

#[test]
fn malformed_payloads_and_reserved_device_status_are_not_misreported_as_healthy() -> Result<()> {
    let profile = profile()?;
    for bytes in [Vec::new(), vec![0; 4], vec![0; 195]] {
        let decoded = decode_detailed(&profile, &bytes);
        assert!(decoded.decode_error.is_some());
        assert!(decoded.events.is_empty());
        assert_eq!(decoded.raw_hex, decode::hex(&bytes));
    }
    assert_eq!(decode_device_status(&[4]).meaning, Some("Failure"));
    assert_eq!(
        decode_device_status(&[0]).meaning,
        Some("Operating properly")
    );
    assert_eq!(
        decode_device_status(&[5]).meaning,
        Some("Reserved device-status value")
    );
    assert!(decode_device_status(&[]).decode_error.is_some());
    assert!(decode_device_status(&[0, 0]).decode_error.is_some());
    Ok(())
}

#[test]
fn simulator_diagnostics_are_explicit_snapshots_and_identity_failures_remain_errors() -> Result<()>
{
    let mut profile = profile()?;
    let report = read(&profile)?;
    assert!(report.ok());
    assert_eq!(report.observed_identities_equal, Some(true));
    assert_eq!(report.device_status.as_ref().unwrap().value, Some(2));
    assert_eq!(report.detailed_status.as_ref().unwrap().events.len(), 1);
    assert!(
        report
            .reads
            .iter()
            .all(|read| read.status.as_ref().unwrap().route == "synthetic simulator")
    );
    profile.expected_identity.device_id = Some(2);
    let denied = read(&profile)?;
    assert!(!denied.ok());
    assert!(denied.device_status.is_none() && denied.detailed_status.is_none());
    assert!(denied.reads.iter().all(|read| {
        read.read_error
            .as_ref()
            .unwrap()
            .contains("identity mismatch")
    }));
    Ok(())
}

#[test]
fn duplicate_event_codes_or_invalid_types_are_rejected() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("bad.xml");
    let xml = fs::read_to_string("samples/sensor.xml")?;
    fs::write(&path, xml.replace("code=\"36350\"", "code=\"16912\""))?;
    assert!(
        iodd::load(&path, None)
            .unwrap_err()
            .to_string()
            .contains("duplicate event code")
    );
    fs::write(&path, xml.replace("type=\"Warning\"", "type=\"Invalid\""))?;
    assert!(
        iodd::load(&path, None)
            .unwrap_err()
            .to_string()
            .contains("invalid type")
    );
    Ok(())
}
