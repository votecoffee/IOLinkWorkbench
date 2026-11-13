use super::*;
use crate::{iodd, profile::Profile, transport};

const XML: &str = include_str!("../../../../samples/sensor-time.xml");

fn profile() -> Result<Profile> {
    let catalog = iodd::parse(XML, "original synthetic time fixture".into())?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    Ok(Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    })
}

#[test]
fn time_era_boundary_and_exact_fraction_have_explicit_interpretations() -> Result<()> {
    let value = decode("TimeT", &0x9DFF_4400_8000_0000u64.to_be_bytes())?;
    assert_eq!(value.value, "1984-01-01 00:00:00.500 UTC");
    assert_eq!(value.raw_ticks, 0x9DFF_4400_8000_0000u64.to_string());
    assert_eq!(value.epoch_utc, Some("1900-01-01T00:00:00Z"));
    assert!(!value.signed && value.bits == 64 && value.byte_order == "big_endian");
    assert!(!value.sub_millisecond_truncated);
    let rollover = decode("TimeT", &0u64.to_be_bytes())?;
    assert_eq!(rollover.value, "2036-02-07 06:28:16.000 UTC");
    assert_eq!(rollover.epoch_utc, Some("2036-02-07T06:28:16Z"));
    let before = decode("TimeT", &0x9DFF_43FF_0000_0000u64.to_be_bytes())?;
    assert!(before.value.starts_with("2120-"));
    assert_eq!(before.epoch_utc, Some("2036-02-07T06:28:16Z"));
    let maximum = decode("TimeT", &u64::MAX.to_be_bytes())?;
    assert_eq!(maximum.raw_ticks, "18446744073709551615");
    assert_eq!(maximum.value, "2036-02-07 06:28:15.999 UTC");
    assert!(maximum.sub_millisecond_truncated);
    Ok(())
}

#[test]
fn durations_preserve_signed_extremes_negative_sub_millisecond_and_days() -> Result<()> {
    for (ticks, expected) in [
        (0, "+00:00:00.000"),
        (1, "+00:00:00.000"),
        (-1, "-00:00:00.000"),
        (-(11i64 << 30), "-00:00:02.750"),
        (i64::MIN, "-24855 03:14:08.000"),
        (i64::MAX, "+24855 03:14:07.999"),
    ] {
        let decoded = decode("TimeSpanT", &ticks.to_be_bytes())?;
        assert_eq!(decoded.value, expected);
        assert_eq!(decoded.raw_ticks, ticks.to_string());
        assert!(decoded.epoch_utc.is_none());
        assert!(decoded.signed);
    }
    assert!(decode("TimeSpanT", &(-1i64).to_be_bytes())?.sub_millisecond_truncated);
    assert!(!decode("TimeSpanT", &i64::MIN.to_be_bytes())?.sub_millisecond_truncated);
    Ok(())
}

#[test]
fn inferred_widths_enable_whole_records_arrays_and_permitted_member_reads() -> Result<()> {
    let profile = profile()?;
    for id in ["V_SIM_Time", "V_SIM_TimeSpan"] {
        let parameter = profile.parameter(id)?;
        assert_eq!(parameter.width(), Some(64));
        assert!(parameter.needs_input.is_empty());
        let reading = transport::read_parameter(&profile, id)?;
        assert_eq!(
            reading.time_value.as_ref().unwrap().raw_ticks,
            reading.raw_value.unwrap()
        );
        assert!(reading.decode_error.is_none());
    }
    let whole = transport::read_parameter(&profile, "V_SIM_TimeRecord")?;
    assert_eq!(whole.record_items.len(), 2);
    for subindex in 1..=2 {
        let member = transport::read_parameter_at(&profile, "V_SIM_TimeRecord", subindex)?;
        assert_eq!(
            member.time_value.unwrap().value,
            whole.record_items[usize::from(subindex - 1)]
                .value
                .time_value
                .as_ref()
                .unwrap()
                .value
        );
    }
    for id in ["V_SIM_Times", "V_SIM_TimeSpans"] {
        let whole = transport::read_parameter(&profile, id)?;
        assert_eq!(whole.array_items.len(), 2);
        for subindex in 1..=2 {
            let member = transport::read_parameter_at(&profile, id, subindex)?;
            assert_eq!(
                member.time_value.unwrap().value,
                whole.array_items[usize::from(subindex - 1)]
                    .value
                    .time_value
                    .as_ref()
                    .unwrap()
                    .value
            );
        }
    }
    Ok(())
}

#[test]
fn malformed_payloads_keep_raw_bytes_and_manual_width_changes_are_rejected() -> Result<()> {
    let profile = profile()?;
    let parameter = profile.parameter("V_SIM_Time")?;
    for payload in [vec![0; 7], vec![0; 9]] {
        let decoded = parameter.decode(&payload);
        assert_eq!(decoded.raw_hex, crate::decode::hex(&payload));
        assert!(decoded.needs_review() && decoded.time_value.is_none());
    }
    let mut legacy = profile.clone();
    legacy.iodd.as_mut().unwrap().parameters[0].bits = None;
    let saved = Profile::parse(&toml::to_string_pretty(&legacy)?)?;
    assert_eq!(
        transport::read_parameter(&saved, "V_SIM_Time")?
            .time_value
            .unwrap()
            .value,
        "1984-01-01 00:00:00.500 UTC"
    );
    legacy.iodd.as_mut().unwrap().parameters[0].bits = Some(32);
    assert!(legacy.validate().is_err());
    Ok(())
}

#[test]
fn unsupported_menu_scaling_is_reviewed_while_the_raw_time_remains_readable() -> Result<()> {
    for attribute in [
        "displayFormat=\"Hex\"",
        "displayFormat=\"Dec.1\"",
        "gradient=\"2\"",
        "unitCode=\"1997\"",
    ] {
        let catalog = iodd::parse(
            &XML.replacen("displayFormat=\"default\"", attribute, 1),
            "invalid temporal display".into(),
        )?;
        let parameter = &catalog.parameters[0];
        assert!(!parameter.needs_input.is_empty());
        assert!(
            parameter
                .decode(&0x9DFF_4400_8000_0000u64.to_be_bytes())
                .time_value
                .is_some()
        );
    }
    Ok(())
}

#[test]
fn saved_profile_and_snapshot_reopen_temporal_values_without_changing_raw_context() -> Result<()> {
    let profile = Profile::parse(&toml::to_string_pretty(&profile()?)?)?;
    let (mut snapshot, reading) = crate::parameter_snapshot::read(&profile, "V_SIM_TimeRecord", 0)?;
    snapshot.workbench_version = "earlier-producer-fixture".into();
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("time.json");
    snapshot.save(&path)?;
    let restored = crate::parameter_snapshot::Snapshot::load(&path)?.review()?;
    assert_eq!(restored.workbench_version, "earlier-producer-fixture");
    assert_eq!(restored.decoder_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(restored.decoded.raw_hex, reading.raw_hex);
    assert_eq!(
        restored.decoded.record_items[1]
            .value
            .time_value
            .as_ref()
            .unwrap()
            .value,
        "-00:00:02.750"
    );
    Ok(())
}
