//! IODD V1.1.5 7.5.3.1.8/.9 lexical defaults; generated locally, never sent.
use super::{ERA_THRESHOLD, TICKS_PER_SECOND, UNIX_OFFSET};
use anyhow::{Context, Result, ensure};
use chrono::NaiveDate;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DefaultConversion {
    pub exact: bool,
    pub policy: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar_basis: Option<&'static str>,
}

fn digits(text: &str) -> Result<u32> {
    ensure!(
        !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()),
        "temporal default requires ASCII decimal digits"
    );
    text.parse()
        .context("temporal default numeric component is too large")
}

fn millis(text: Option<&str>) -> Result<u32> {
    let Some(text) = text else { return Ok(0) };
    ensure!(
        (1..=3).contains(&text.len()),
        "temporal default permits one to three fractional digits"
    );
    Ok(digits(text)? * 10u32.pow(3 - text.len() as u32))
}

fn fraction(millis: u32) -> (u64, bool) {
    let numerator = u64::from(millis) * TICKS_PER_SECOND;
    (numerator.div_ceil(1000), numerator.is_multiple_of(1000))
}

fn date(text: &str) -> Result<(u64, bool)> {
    ensure!(text.is_ascii(), "TimeT default requires an ASCII date");
    ensure!(
        text.len() == 10 || (19..=23).contains(&text.len()),
        "TimeT default requires yyyy-mm-dd or yyyy-mm-ddThh:mm:ss[.fff], without a timezone"
    );
    ensure!(
        &text[4..5] == "-" && &text[7..8] == "-",
        "invalid TimeT date separators"
    );
    let year = digits(&text[..4])?;
    ensure!(
        year > 0,
        "TimeT default year zero is not an XML Schema date"
    );
    let mut day = NaiveDate::from_ymd_opt(year as i32, digits(&text[5..7])?, digits(&text[8..10])?)
        .context("invalid TimeT calendar date")?;
    let (mut hour, minute, second, millis) = if text.len() == 10 {
        (0, 0, 0, 0)
    } else {
        ensure!(
            &text[10..11] == "T" && &text[13..14] == ":" && &text[16..17] == ":",
            "invalid TimeT time separators"
        );
        let fractional = if text.len() == 19 {
            None
        } else {
            ensure!(
                &text[19..20] == ".",
                "TimeT defaults do not include timezone suffixes"
            );
            Some(&text[20..])
        };
        (
            digits(&text[11..13])?,
            digits(&text[14..16])?,
            digits(&text[17..19])?,
            millis(fractional)?,
        )
    };
    if hour == 24 {
        ensure!(
            minute == 0 && second == 0 && millis == 0,
            "TimeT hour 24 requires exactly midnight"
        );
        day = day.succ_opt().context("TimeT date overflow")?;
        hour = 0;
    }
    let utc = day
        .and_hms_opt(hour, minute, second)
        .context("invalid TimeT clock time; leap seconds are not inferred")?
        .and_utc();
    let seconds = utc.timestamp() + UNIX_OFFSET;
    ensure!(
        (i64::from(ERA_THRESHOLD)..i64::from(ERA_THRESHOLD) + TICKS_PER_SECOND as i64)
            .contains(&seconds),
        "TimeT default is outside the unambiguous default era window [1984-01-01T00:00:00, 2120-02-07T06:28:16); no rollover context is inferred"
    );
    let (fraction, exact) = fraction(millis);
    Ok((
        ((seconds as u64 % TICKS_PER_SECOND) << 32) | fraction,
        exact,
    ))
}

fn duration(text: &str) -> Result<(u64, bool)> {
    let (negative, body) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let body = body.strip_prefix("PT").and_then(|text| text.strip_suffix('S'))
        .context("TimeSpanT default requires [+-]PTseconds[.fff]S; days, hours and minutes are not accepted")?;
    let (seconds, fractional) = body
        .split_once('.')
        .map_or((body, None), |(seconds, fraction)| {
            (seconds, Some(fraction))
        });
    ensure!(
        !seconds.is_empty() && seconds.bytes().all(|byte| byte.is_ascii_digit()),
        "TimeSpanT default requires ASCII seconds"
    );
    let significant = seconds.trim_start_matches('0');
    let seconds: u64 = if significant.is_empty() {
        0
    } else {
        ensure!(
            significant.len() <= 10,
            "TimeSpanT default seconds exceed the signed 64-bit tick range"
        );
        significant.parse()?
    };
    let (fraction, exact) = fraction(millis(fractional)?);
    let magnitude = (u128::from(seconds) << 32) + u128::from(fraction);
    let limit = if negative {
        1u128 << 63
    } else {
        i64::MAX as u128
    };
    ensure!(
        magnitude <= limit,
        "TimeSpanT default exceeds the signed 64-bit tick range"
    );
    let ticks = if negative {
        -(magnitude as i128)
    } else {
        magnitude as i128
    } as i64;
    Ok((ticks as u64, exact))
}

pub(in crate::iodd::parameters) fn encode(
    datatype: &str,
    declared: &str,
) -> Result<(Vec<u8>, DefaultConversion)> {
    let (ticks, exact) = match datatype {
        "TimeT" => date(declared)?,
        "TimeSpanT" => duration(declared)?,
        _ => anyhow::bail!("unsupported temporal default datatype"),
    };
    Ok((ticks.to_be_bytes().to_vec(), DefaultConversion {
        exact,
        policy: "Local representation: fractional magnitude rounds away from zero by less than one 1/2^32-second tick to retain the declared millisecond display; device-selected bytes and clock validity are not inferred",
        calendar_basis: (datatype == "TimeT").then_some("Timezone-free IODD calendar interpreted on the IO-Link UTC timeline using the default 1984-2120 era window"),
    }))
}

#[cfg(test)]
mod tests;
