//! IO-Link Interface V1.1.5 F.2.8/F.2.9; fixed-point seconds, not host freshness metadata.
use anyhow::{Result, ensure};
use serde::Serialize;

const TICKS_PER_SECOND: u64 = 1 << 32;
const ERA_THRESHOLD: u32 = 0x9DFF_4400;
const UNIX_OFFSET: i64 = 2_208_988_800;

mod defaults;
pub use defaults::DefaultConversion;
pub(super) use defaults::encode as encode_default;

#[derive(Debug, Serialize)]
pub struct TimeValue {
    pub datatype: &'static str,
    pub bits: u8,
    pub byte_order: &'static str,
    pub signed: bool,
    pub value: String,
    pub raw_ticks: String,
    pub ticks_per_second: u64,
    pub display_resolution_us: u64,
    pub sub_millisecond_truncated: bool,
    pub epoch_utc: Option<&'static str>,
    pub interpretation: &'static str,
}

impl TimeValue {
    pub fn text(&self) -> String {
        format!(
            "{} · {}\n{} bits · {} · signed {}\nExact ticks {} / {} s · display resolution {} µs · sub-millisecond fraction truncated {}\n{}",
            self.datatype,
            self.value,
            self.bits,
            self.byte_order,
            self.signed,
            self.raw_ticks,
            self.ticks_per_second,
            self.display_resolution_us,
            self.sub_millisecond_truncated,
            self.interpretation
        )
    }
}

fn word(payload: &[u8]) -> Result<u64> {
    ensure!(
        payload.len() == 8,
        "time parameter response must contain exactly eight bytes"
    );
    Ok(u64::from_be_bytes(
        payload.try_into().expect("checked eight-byte width"),
    ))
}

pub(super) fn raw_ticks(datatype: &str, payload: &[u8]) -> Result<String> {
    let word = word(payload)?;
    match datatype {
        "TimeT" => Ok(word.to_string()),
        "TimeSpanT" => Ok((word as i64).to_string()),
        _ => anyhow::bail!("unsupported temporal datatype"),
    }
}

pub(super) fn decode(datatype: &str, payload: &[u8]) -> Result<TimeValue> {
    let word = word(payload)?;
    let (datatype, raw_ticks, magnitude) = match datatype {
        "TimeT" => ("TimeT", word.to_string(), word),
        "TimeSpanT" => (
            "TimeSpanT",
            (word as i64).to_string(),
            (word as i64).unsigned_abs(),
        ),
        _ => anyhow::bail!("unsupported temporal datatype"),
    };
    let fractional_millis = (magnitude & u64::from(u32::MAX)) * 1000;
    let millis = fractional_millis / TICKS_PER_SECOND;
    let (value, epoch_utc, interpretation) = if datatype == "TimeT" {
        let seconds = (word >> 32) as u32;
        let era = u64::from(seconds < ERA_THRESHOLD);
        let epoch = if era == 0 {
            "1900-01-01T00:00:00Z"
        } else {
            "2036-02-07T06:28:16Z"
        };
        let unix_seconds = (u64::from(seconds) + era * TICKS_PER_SECOND) as i64 - UNIX_OFFSET;
        let utc = chrono::DateTime::<chrono::Utc>::from_timestamp(
            unix_seconds,
            (millis * 1_000_000) as u32,
        )
        .ok_or_else(|| anyhow::anyhow!("time value is outside the UTC formatter range"))?;
        (
            utc.format("%Y-%m-%d %H:%M:%S%.3f UTC").to_string(),
            Some(epoch),
            "IO-Link default era rule at 0x9DFF4400; additional 136-year rollovers require external context; clock validity is not inferred",
        )
    } else {
        let seconds = magnitude / TICKS_PER_SECOND;
        let days = seconds / 86_400;
        let sign = if (word as i64) < 0 { "-" } else { "+" };
        let day = if days == 0 {
            String::new()
        } else {
            format!("{days} ")
        };
        (
            format!(
                "{sign}{day}{:02}:{:02}:{:02}.{millis:03}",
                (seconds / 3600) % 24,
                (seconds / 60) % 60,
                seconds % 60
            ),
            None,
            "Signed IO-Link fixed-point duration; millisecond display truncates magnitude toward zero",
        )
    };
    Ok(TimeValue {
        datatype,
        bits: 64,
        byte_order: "big_endian",
        signed: datatype == "TimeSpanT",
        value,
        raw_ticks,
        ticks_per_second: TICKS_PER_SECOND,
        display_resolution_us: 1000,
        sub_millisecond_truncated: fractional_millis % TICKS_PER_SECOND != 0,
        epoch_utc,
        interpretation,
    })
}

#[cfg(test)]
mod tests;
