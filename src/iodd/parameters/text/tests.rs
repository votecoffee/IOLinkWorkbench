use super::*;
use crate::{iodd, profile::Profile, transport};

const XML: &str = include_str!("../../../../samples/sensor-strings.xml");

fn profile() -> Result<Profile> {
    let catalog = iodd::parse(XML, "original synthetic text/octet fixture".into())?;
    let (expected_identity, imported) = catalog.select(None, None)?;
    Ok(Profile {
        expected_identity,
        iodd: Some(imported),
        ..Default::default()
    })
}

#[test]
fn text_supports_condensed_unicode_and_nul_only_padding_using_octet_limits() -> Result<()> {
    for payload in [b"HELLO".as_slice(), b"HELLO\0\0\0".as_slice()] {
        assert_eq!(
            decode("StringT", Some(8), Some("US-ASCII"), payload)?,
            "HELLO"
        );
    }
    assert_eq!(
        decode("StringT", Some(8), Some("UTF-8"), "µm".as_bytes())?,
        "µm"
    );
    assert_eq!(
        decode(
            "StringT",
            Some(8),
            Some("UTF-8"),
            &[0xC2, 0xB5, b'm', 0, 0, 0, 0, 0]
        )?,
        "µm"
    );
    assert_eq!(decode("StringT", Some(8), Some("UTF-8"), &[0; 8])?, "");
    assert!(decode("StringT", Some(2), Some("UTF-8"), "µm".as_bytes()).is_err());
    Ok(())
}

#[test]
fn malformed_padding_encoding_and_metadata_preserve_the_original_payload() -> Result<()> {
    let profile = profile()?;
    let parameter = profile.parameter("V_SIM_Text")?;
    for payload in [vec![b'A', 0, b'B'], vec![0xC2], vec![0xFF], vec![b'A'; 9]] {
        let result = parameter.decode(&payload);
        assert!(result.needs_review() && result.raw_value.is_none());
        assert_eq!(result.raw_hex, crate::decode::hex(&payload));
    }
    assert!(decode("StringT", Some(8), Some("US-ASCII"), "µm".as_bytes()).is_err());
    for length in [None, Some(0), Some(233)] {
        assert!(decode("StringT", length, Some("UTF-8"), b"A").is_err());
        assert!(decode("OctetStringT", length, None, &[1]).is_err());
    }
    assert!(decode("StringT", Some(8), Some("unknown"), b"A").is_err());
    Ok(())
}

#[test]
fn octet_strings_keep_zeros_order_and_maximum_supported_length() -> Result<()> {
    assert_eq!(
        decode("OctetStringT", Some(4), None, &[0, 0x80, 0xFF, 0x2C])?,
        "0x00,0x80,0xFF,0x2C"
    );
    let payload: Vec<u8> = (0..232).collect();
    let text = decode("OctetStringT", Some(232), None, &payload)?;
    assert_eq!(text.split(',').count(), 232);
    assert!(text.starts_with("0x00,0x01") && text.ends_with("0xE7"));
    for payload in [vec![], vec![0; 3], vec![0; 5]] {
        assert!(decode("OctetStringT", Some(4), None, &payload).is_err());
    }
    let parameter = profile()?.parameter("V_SIM_Octets")?.clone();
    let bad = parameter.decode(&[0, 0, 0]);
    assert_eq!(bad.raw_hex, "000000");
    assert!(bad.decode_error.is_some());
    Ok(())
}

#[test]
fn whole_records_arrays_and_condensed_member_reads_share_default_views() -> Result<()> {
    let profile = profile()?;
    let whole = transport::read_parameter(&profile, "V_SIM_BlobRecord")?;
    let expected = ["0x00,0x80,0xFF,0x2C", "µm", "OK"];
    for (offset, value) in expected.iter().enumerate() {
        assert_eq!(
            whole.record_items[offset].value.raw_value.as_deref(),
            Some(*value)
        );
        assert_eq!(
            transport::read_parameter_at(&profile, "V_SIM_BlobRecord", offset as u8 + 1)?
                .raw_value
                .as_deref(),
            Some(*value)
        );
    }
    for id in ["V_SIM_BlobArray", "V_SIM_TextArray"] {
        let whole = transport::read_parameter(&profile, id)?;
        for subindex in 1..=2 {
            assert_eq!(
                whole.array_items[usize::from(subindex - 1)].value.raw_value,
                transport::read_parameter_at(&profile, id, subindex)?.raw_value
            );
        }
    }
    let whole = transport::read_parameter(&profile, "V_SIM_TextArray")?;
    let member = transport::read_parameter_at(&profile, "V_SIM_TextArray", 1)?;
    assert_eq!(whole.array_items[0].value.raw_hex, "C3A90000");
    assert_eq!(member.raw_hex, "C3A9");
    assert_eq!(member.raw_value.as_deref(), Some("é"));
    Ok(())
}

#[test]
fn default_menu_views_are_supported_and_numeric_string_formats_need_review() -> Result<()> {
    let profile = profile()?;
    assert!(profile.parameter("V_SIM_Octets")?.needs_input.is_empty());
    assert!(profile.parameter("V_SIM_Text")?.needs_input.is_empty());
    for attribute in [
        "displayFormat=\"Hex\"",
        "displayFormat=\"Dec\"",
        "gradient=\"1\"",
        "unitCode=\"1997\"",
    ] {
        let catalog = iodd::parse(
            &XML.replacen("displayFormat=\"default\"", attribute, 1),
            "invalid octet display".into(),
        )?;
        assert!(!catalog.parameters[0].needs_input.is_empty());
        assert_eq!(
            catalog.parameters[0]
                .decode(&[0, 0x80, 0xFF, 0x2C])
                .raw_value
                .as_deref(),
            Some("0x00,0x80,0xFF,0x2C")
        );
    }
    Ok(())
}

#[test]
fn snapshots_keep_malformed_text_and_other_record_members_remain_available() -> Result<()> {
    let profile = Profile::parse(&toml::to_string_pretty(&profile()?)?)?;
    let (mut snapshot, _) = crate::parameter_snapshot::read(&profile, "V_SIM_BlobRecord", 0)?;
    // Replace the UTF-8 member with A, NUL, then nonzero padding.
    snapshot.raw_hex = "0080FF2C41004200000000004F4B0000".into();
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("malformed-text.json");
    snapshot.save(&path)?;
    let review = crate::parameter_snapshot::Snapshot::load(&path)?.review()?;
    assert!(review.decoded.needs_review());
    assert_eq!(
        review.decoded.record_items[1].value.raw_hex,
        "4100420000000000"
    );
    assert!(
        review.decoded.record_items[1]
            .value
            .decode_error
            .as_ref()
            .unwrap()
            .contains("padding")
    );
    assert_eq!(
        review.decoded.record_items[0].value.raw_value.as_deref(),
        Some("0x00,0x80,0xFF,0x2C")
    );
    assert_eq!(
        review.decoded.record_items[2].value.raw_value.as_deref(),
        Some("OK")
    );
    Ok(())
}
