use super::*;
use crate::{decode::hex, iodd::parameters::time::decode};

#[test]
fn calendar_defaults_cover_dates_midnight_leap_days_and_both_eras() -> Result<()> {
    for (declared, bytes, displayed, exact) in [
        (
            "1984-01-01",
            "9DFF440000000000",
            "1984-01-01 00:00:00.000 UTC",
            true,
        ),
        (
            "1984-01-01T00:00:00.5",
            "9DFF440080000000",
            "1984-01-01 00:00:00.500 UTC",
            true,
        ),
        (
            "2036-02-07T06:28:16",
            "0000000000000000",
            "2036-02-07 06:28:16.000 UTC",
            true,
        ),
        (
            "2120-02-07T06:28:15.999",
            "9DFF43FFFFBE76C9",
            "2120-02-07 06:28:15.999 UTC",
            false,
        ),
    ] {
        let (payload, conversion) = encode("TimeT", declared)?;
        assert_eq!(hex(&payload), bytes);
        assert_eq!(decode("TimeT", &payload)?.value, displayed);
        assert_eq!(conversion.exact, exact);
        assert!(conversion.calendar_basis.is_some());
    }
    let (payload, _) = encode("TimeT", "2000-02-29T24:00:00.000")?;
    assert_eq!(
        decode("TimeT", &payload)?.value,
        "2000-03-01 00:00:00.000 UTC"
    );
    assert_eq!(payload, encode("TimeT", "2000-03-01")?.0);
    Ok(())
}

#[test]
fn invalid_calendars_and_unknown_rollover_context_are_not_wrapped() {
    for declared in [
        "",
        "0000-01-01",
        "1983-12-31T23:59:59.999",
        "2120-02-07T06:28:16",
        "9999-12-31",
        "1900-01-01",
        "2025-02-29",
        "2000-13-01",
        "2000-01-00",
        "2000-01-32",
        "2000-01-01T24:00:00.001",
        "2000-01-01T24:01:00",
        "2000-01-01T25:00:00",
        "2000-01-01T12:60:00",
        "2000-01-01T12:00:60",
        "2000-01-01T12:00:00.",
        "2000-01-01T12:00:00.0001",
        "2000-01-01T12:00:00Z",
        "2000-01-01T12:00:00+01:00",
        "2000-01-01Z",
        " 2000-01-01",
        "2000-01-01 ",
        "2000-1-01",
        "2000/01/01",
        "2000-01-01t00:00:00",
        "2000-01-01 00:00:00",
        "２０００-01-01",
    ] {
        assert!(encode("TimeT", declared).is_err(), "{declared:?}");
    }
}

#[test]
fn duration_defaults_preserve_sign_and_the_signed_tick_limits() -> Result<()> {
    for (declared, bytes, displayed, exact) in [
        ("PT0S", "0000000000000000", "+00:00:00.000", true),
        ("-PT000.000S", "0000000000000000", "+00:00:00.000", true),
        ("+PT0002.750S", "00000002C0000000", "+00:00:02.750", true),
        ("-PT2.75S", "FFFFFFFD40000000", "-00:00:02.750", true),
        ("-PT0.001S", "FFFFFFFFFFBE76C8", "-00:00:00.001", false),
        (
            "PT2147483647.999S",
            "7FFFFFFFFFBE76C9",
            "+24855 03:14:07.999",
            false,
        ),
        (
            "-PT2147483648S",
            "8000000000000000",
            "-24855 03:14:08.000",
            true,
        ),
    ] {
        let (payload, conversion) = encode("TimeSpanT", declared)?;
        assert_eq!(hex(&payload), bytes);
        assert_eq!(decode("TimeSpanT", &payload)?.value, displayed);
        assert_eq!(conversion.exact, exact);
        assert!(conversion.calendar_basis.is_none());
    }
    Ok(())
}

#[test]
fn duration_grammar_excludes_general_iso_durations_and_out_of_range_values() {
    for declared in [
        "",
        "PT",
        "PTS",
        "PT.1S",
        "PT1.S",
        "PT1.0000S",
        "PT1.2.3S",
        "PT-1S",
        "PT+1S",
        "P1D",
        "PT1H",
        "PT1M",
        "PT1M1S",
        "PT1e3S",
        "PTNaNS",
        "PTINFS",
        "PT１S",
        "pt1s",
        " PT1S",
        "PT1S ",
        "PT2147483648S",
        "-PT2147483648.001S",
        "PT999999999999999999999999S",
    ] {
        assert!(encode("TimeSpanT", declared).is_err(), "{declared:?}");
    }
}

#[test]
fn every_millisecond_keeps_its_display_with_a_bounded_explicit_tick_conversion() -> Result<()> {
    for millis in 0u32..1000 {
        let numerator = u64::from(millis) * TICKS_PER_SECOND;
        for datatype in ["TimeT", "TimeSpanT"] {
            for negative in [false, true] {
                if datatype == "TimeT" && negative {
                    continue;
                }
                let declared = if datatype == "TimeT" {
                    format!("1984-01-01T00:00:00.{millis:03}")
                } else {
                    format!("{}PT0.{millis:03}S", if negative { "-" } else { "+" })
                };
                let (payload, conversion) = encode(datatype, &declared)?;
                let value = decode(datatype, &payload)?;
                assert!(
                    value.value.contains(&format!(".{millis:03}")),
                    "{declared} -> {}",
                    value.value
                );
                assert_eq!(conversion.exact, millis.is_multiple_of(125));
                let word = u64::from_be_bytes(payload.try_into().unwrap());
                let fractional_ticks = if datatype == "TimeT" {
                    word & u64::from(u32::MAX)
                } else {
                    (word as i64).unsigned_abs()
                };
                let error_numerator = fractional_ticks * 1000 - numerator;
                assert!(error_numerator < 1000);
                assert_eq!(error_numerator == 0, conversion.exact);
            }
        }
    }
    Ok(())
}
