use super::*;
use crate::{
    archive,
    capture::{self, Options, Stop},
    iodd, parameter_snapshot,
};
use std::{fs, time::Duration};

const XML: &str = include_str!("../../samples/sensor-communication.xml");

fn from_xml(xml: &str) -> Result<Profile> {
    let catalog = iodd::parse(xml, "original communication fixture".into())?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    Ok(Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    })
}

#[test]
fn wired_declarations_persist_in_profiles_and_parameter_snapshots() -> Result<()> {
    let profile = from_xml(XML)?;
    let declaration = profile.communication().unwrap();
    assert_eq!(declaration.minimum_cycle_us, 2300);
    assert_eq!(declaration.bitrate, Bitrate::Com2);
    assert!(declaration.sio_supported);
    assert_eq!(declaration.m_sequence_capability, 27);
    assert_eq!(declaration.compatible_with.as_deref(), Some("V1.0"));
    assert_eq!(Profile::parse(&toml::to_string_pretty(&profile)?)?, profile);
    let (snapshot, _) = parameter_snapshot::read(&profile, "V_SIM_Threshold", 0)?;
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("parameter.json");
    snapshot.save(&path)?;
    let bytes = fs::read(&path)?;
    let restored = parameter_snapshot::Snapshot::load(&path)?;
    assert_eq!(restored.profile.communication(), Some(declaration));
    assert_eq!(restored.review()?.decoded.raw_value.as_deref(), Some("500"));
    assert_eq!(fs::read(path)?, bytes);
    Ok(())
}

#[test]
fn iodds_enforce_microsecond_ranges_steps_and_unambiguous_wired_metadata() -> Result<()> {
    for bitrate in ["COM1", "COM2", "COM3"] {
        let profile = from_xml(
            &XML.replace("COM2", bitrate)
                .replace("minCycleTime=\"2300\"", "minCycleTime=\" +02300 \"")
                .replace("sioSupported=\"true\"", "sioSupported=\"1\""),
        )?;
        assert_eq!(profile.communication().unwrap().bitrate.label(), bitrate);
        assert!(profile.communication().unwrap().sio_supported);
    }
    for value in [0, 100, 6300, 6400, 6800, 31600, 32000, 33600, 132800] {
        let profile = from_xml(&XML.replace(
            "minCycleTime=\"2300\"",
            &format!("minCycleTime=\"{value}\""),
        ))?;
        assert_eq!(profile.communication().unwrap().minimum_cycle_us, value);
    }
    for value in [1, 6330, 6500, 31700, 31900, 132801, 134400] {
        assert!(
            from_xml(&XML.replace(
                "minCycleTime=\"2300\"",
                &format!("minCycleTime=\"{value}\"")
            ))
            .is_err()
        );
    }
    for xml in [
        XML.replace("COM2", "COM4"),
        XML.replace("sioSupported=\"true\"", "sioSupported=\"unknown\""),
        XML.replace("mSequenceCapability=\"27\"", "mSequenceCapability=\"256\""),
        XML.replace("iolinkRevision=\"V1.1\"", "iolinkRevision=\"V2.0\""),
        XML.replace("compatibleWith=\"V1.0\"", "compatibleWith=\"V9.0\""),
        XML.replace("</TransportLayers>", "<PhysicalLayer/></TransportLayers>"),
        XML.replace(
            "</CommNetworkProfile>",
            "<TransportLayers/></CommNetworkProfile>",
        ),
        XML.replace("</IODevice>", "<CommNetworkProfile/></IODevice>"),
    ] {
        assert!(from_xml(&xml).is_err());
    }
    Ok(())
}

#[test]
fn shorter_target_and_reported_cycle_do_not_qualify_the_sensor_or_gateway() -> Result<()> {
    let profile = from_xml(XML)?;
    let context = assess(
        &profile,
        Some(1000),
        Observation {
            port_cycle_us: Some(1000),
            freshness: Some(Freshness::Counter32),
            route: Some("synthetic simulator"),
        },
        1000,
    )?;
    assert_eq!(context.target_vs_iodd_minimum, Relation::Shorter);
    assert_eq!(context.host_poll_vs_iodd_minimum, Relation::Shorter);
    assert_eq!(context.reported_port_vs_iodd_minimum, Relation::Shorter);
    assert_eq!(context.target_vs_reported_port, Relation::Equal);
    assert!(
        context
            .notes
            .iter()
            .any(|note| note.contains("verify identity"))
    );
    assert!(context.notes.iter().any(|note| note.contains("synthetic")));
    Ok(())
}

#[test]
fn zero_missing_and_unsupported_declarations_remain_unknown() -> Result<()> {
    let zero = from_xml(&XML.replace("minCycleTime=\"2300\"", "minCycleTime=\"0\""))?;
    let context = assess(
        &zero,
        Some(1000),
        Observation {
            port_cycle_us: Some(0),
            ..Default::default()
        },
        1000,
    )?;
    assert_eq!(context.iodd_minimum_cycle_us, Some(0));
    assert_eq!(context.target_vs_iodd_minimum, Relation::Unknown);
    assert_eq!(context.target_vs_reported_port, Relation::Unknown);
    assert!(
        context
            .notes
            .iter()
            .any(|note| note.contains("M-sequence timing"))
    );
    let legacy = Profile::default();
    let context = assess(&legacy, None, Observation::default(), 1000)?;
    assert!(context.iodd_minimum_cycle_us.is_none() && context.requested_host_poll_us.is_none());
    assert_eq!(context.host_poll_vs_iodd_minimum, Relation::Unknown);
    let wireless =
        from_xml(&XML.replace("iolinkRevision=\"V1.1\"", "iolinkWirelessRevision=\"V1.1\""))?;
    assert!(wireless.communication().is_none());
    Ok(())
}

#[test]
fn snapshot_delivery_and_longer_intervals_are_independent_of_the_declared_minimum() -> Result<()> {
    let mut profile = from_xml(XML)?;
    profile.connection.gateway = Gateway::IfmAl1350;
    profile.connection.endpoint = "http://127.0.0.1:9".into();
    let context = assess(
        &profile,
        Some(10000),
        Observation {
            port_cycle_us: Some(3000),
            freshness: Some(Freshness::Snapshots),
            route: Some("loopback fixture"),
        },
        5000,
    )?;
    assert_eq!(context.target_vs_iodd_minimum, Relation::Longer);
    assert_eq!(context.host_poll_vs_iodd_minimum, Relation::Longer);
    assert_eq!(context.reported_port_vs_iodd_minimum, Relation::Longer);
    assert_eq!(context.target_vs_reported_port, Relation::Longer);
    assert!(
        context
            .notes
            .iter()
            .any(|note| note.contains("source-update loss remains unknown"))
    );
    let equal = from_xml(&XML.replace("minCycleTime=\"2300\"", "minCycleTime=\"1000\""))?;
    assert_eq!(
        assess(&equal, Some(1000), Default::default(), 1000)?.target_vs_iodd_minimum,
        Relation::Equal
    );
    assert!(assess(&equal, Some(1000), Default::default(), 0).is_err());
    Ok(())
}

#[test]
fn captures_keep_declared_cycles_separate_from_measured_and_reported_timing() -> Result<()> {
    let profile = from_xml(XML)?;
    let folder = tempfile::tempdir()?;
    let output = folder.path().join("capture");
    let report = capture::run(
        &profile,
        &Options {
            output: Some(output.clone()),
            duration: Some(Duration::from_millis(100)),
            ..Default::default()
        },
        &Stop::default(),
        None,
    )?;
    assert_eq!(report.iodd_communication.as_ref(), profile.communication());
    assert!(archive::verify(&output, None)?.rows_checked > 0);
    let review = archive::review(&output, None, 3)?;
    assert_eq!(
        review.signal.cycle_context.iodd_minimum_cycle_us,
        Some(2300)
    );
    assert_eq!(
        review.signal.cycle_context.reported_port_cycle_us,
        Some(1000)
    );
    assert_eq!(review.signal.recorded_timing.port_cycle_us, Some(1000));
    let mut mapping = profile.clone();
    mapping.iodd.as_mut().unwrap().communication = None;
    let export = folder.path().join("redecoded");
    archive::redecode(&output, &mapping, &export)?;
    let verified = archive::verify_export(&export, Some(&output))?;
    assert_eq!(verified.rows_checked, report.csv_rows_flushed);
    assert!(verified.original_source_reverified);
    let path = output.join("capture.toml");
    let original = fs::read(&path)?;
    let mut manifest: toml::Value = toml::from_str(std::str::from_utf8(&original)?)?;
    manifest["iodd_communication"]["minimum_cycle_us"] = toml::Value::Integer(2400);
    fs::write(&path, toml::to_string(&manifest)?)?;
    assert!(archive::verify(&output, None).is_err());
    let mut manifest: toml::Value = toml::from_str(std::str::from_utf8(&original)?)?;
    manifest["requested_poll_us"] = toml::Value::Integer(2000);
    fs::write(&path, toml::to_string(&manifest)?)?;
    assert!(archive::verify(&output, None).is_err());
    // Older reports may omit the requested interval; do not infer it from the profile.
    manifest.as_table_mut().unwrap().remove("requested_poll_us");
    fs::write(&path, toml::to_string(&manifest)?)?;
    let review = archive::review(&output, None, 3)?;
    assert!(review.signal.cycle_context.requested_host_poll_us.is_none());
    assert_eq!(
        review.signal.cycle_context.host_poll_vs_iodd_minimum,
        Relation::Unknown
    );
    fs::write(&path, original)?;
    archive::verify(&output, None)?;
    Ok(())
}
