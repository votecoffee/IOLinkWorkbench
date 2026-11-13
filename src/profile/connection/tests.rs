use super::*;
use crate::profile::CycleUnit;

#[test]
fn host_edits_preserve_every_other_profile_setting_and_the_source() -> Result<()> {
    let catalog = crate::iodd::parse(
        include_str!("../../../samples/sensor-formats.xml"),
        "synthetic connection editor fixture".into(),
    )?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    let mut profile = Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    };
    profile.set_parameter_display("V_SIM_Threshold", 0, Some("M_SIM_Threshold/2"))?;
    profile.logging.raw_only = true;
    profile.expected_identity.serial = Some("saved-pin".into());
    let before = toml::to_string_pretty(&profile)?;
    let connection = Connection {
        gateway: Gateway::IfmAl1350,
        endpoint: "  http://127.0.0.1:9  ".into(),
        ifm_firmware: " 3.1.42 ".into(),
        ifm_mode: Some(3),
        ifm_cycle_unit: Some(CycleUnit::Microseconds),
        port: 4,
        poll_us: 20_000,
        timeout_ms: 2000,
        stale_us: 10_000,
        reconnect_ms: 500,
        simulate_snapshots: true,
    };
    let mut candidate = profile.with_connection(connection)?;
    assert_eq!(candidate.connection.endpoint, "http://127.0.0.1:9");
    assert_eq!(candidate.connection.ifm_firmware, "3.1.42");
    assert_eq!(candidate.connection.port, 4);
    candidate.connection = profile.connection.clone();
    assert_eq!(candidate, profile);
    assert_eq!(toml::to_string_pretty(&profile)?, before);
    Ok(())
}

#[test]
fn origin_validation_is_shared_and_does_not_echo_credentials() -> Result<()> {
    let profile = Profile::default();
    for endpoint in [
        "not a URL",
        "ftp://localhost",
        "http://localhost/api",
        "https://localhost/?query=x",
        "https://localhost/#anchor",
        "https://user:secret123@localhost",
        "https://localhost/path/../api",
    ] {
        let result = profile.with_connection(Connection {
            gateway: Gateway::IfmAl1350,
            endpoint: endpoint.into(),
            ..Default::default()
        });
        let error = format!("{:#}", result.unwrap_err());
        assert!(!error.contains("secret123"));
    }
    for endpoint in [
        "http://127.0.0.1:9",
        "https://localhost/",
        "http://[::1]:8080",
    ] {
        let connection = Connection {
            gateway: Gateway::IfmAl1350,
            endpoint: endpoint.into(),
            ..Default::default()
        };
        assert_eq!(
            profile.with_connection(connection)?.connection.endpoint,
            endpoint
        );
    }
    Ok(())
}

#[test]
fn invalid_intervals_and_ports_reject_the_candidate_without_mutation() -> Result<()> {
    let profile = Profile::default();
    let before = profile.clone();
    let connection = &profile.connection;
    for invalid in [
        Connection {
            port: 0,
            ..connection.clone()
        },
        Connection {
            port: 5,
            ..connection.clone()
        },
        Connection {
            poll_us: 99,
            ..connection.clone()
        },
        Connection {
            poll_us: 60_000_001,
            ..connection.clone()
        },
        Connection {
            timeout_ms: 0,
            ..connection.clone()
        },
        Connection {
            timeout_ms: 30_001,
            ..connection.clone()
        },
        Connection {
            reconnect_ms: 0,
            ..connection.clone()
        },
        Connection {
            reconnect_ms: 60_001,
            ..connection.clone()
        },
    ] {
        assert!(profile.with_connection(invalid).is_err());
        assert_eq!(profile, before);
    }
    for valid in [
        Connection {
            port: 1,
            poll_us: 100,
            timeout_ms: 1,
            reconnect_ms: 1,
            stale_us: 0,
            ..connection.clone()
        },
        Connection {
            port: 4,
            poll_us: 60_000_000,
            timeout_ms: 30_000,
            reconnect_ms: 60_000,
            stale_us: u64::MAX,
            ..connection.clone()
        },
    ] {
        profile.with_connection(valid)?;
    }
    Ok(())
}

#[test]
fn unfinished_templates_and_inactive_gateway_settings_remain_explicit() -> Result<()> {
    let profile = Profile::default();
    let unfinished = Connection {
        gateway: Gateway::IfmAl1350,
        ..Default::default()
    };
    let candidate = profile.with_connection(unfinished)?;
    assert!(candidate.connection.endpoint.is_empty());
    assert_eq!(candidate.connection.ifm_mode, None);
    assert_eq!(candidate.connection.ifm_cycle_unit, None);
    let mut inactive = candidate.connection.clone();
    inactive.gateway = Gateway::Simulator;
    inactive.endpoint = "retained inactive draft".into();
    inactive.ifm_mode = Some(7);
    inactive.ifm_cycle_unit = Some(CycleUnit::Milliseconds);
    let candidate = profile.with_connection(inactive.clone())?;
    assert_eq!(candidate.connection, inactive);
    inactive.gateway = Gateway::IfmAl1350;
    assert!(profile.with_connection(inactive).is_err());
    Ok(())
}

#[test]
fn new_profile_publication_never_replaces_source_or_existing_destination() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let source = folder.path().join("source.toml");
    let original = format!(
        "# Keep this source comment and formatting\n{}",
        toml::to_string_pretty(&Profile::default())?
    );
    std::fs::write(&source, original.as_bytes())?;
    let profile = Profile::load(&source)?;
    let candidate = profile.with_connection(Connection {
        poll_us: 20_000,
        ..profile.connection.clone()
    })?;
    assert!(candidate.save_new(&source).is_err());
    assert_eq!(std::fs::read(&source)?, original.as_bytes());
    let output = folder.path().join("new.toml");
    candidate.save_new(&output)?;
    assert_eq!(Profile::load(&output)?, candidate);
    let saved = std::fs::read(&output)?;
    assert!(profile.save_new(&output).is_err());
    assert_eq!(std::fs::read(&output)?, saved);
    assert_eq!(std::fs::read_dir(folder.path())?.count(), 2);
    Ok(())
}

#[test]
fn legacy_profile_load_does_not_reinterpret_saved_routes() -> Result<()> {
    let mut profile = Profile::default();
    profile.connection.gateway = Gateway::IfmAl1350;
    profile.connection.endpoint = "legacy incomplete endpoint".into();
    let text = toml::to_string_pretty(&profile)?;
    assert_eq!(Profile::parse(&text)?, profile);
    assert!(profile.with_connection(profile.connection.clone()).is_err());
    Ok(())
}
