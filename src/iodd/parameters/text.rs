//! Default StringT/OctetStringT views, IO-Link Interface V1.1.5 F.2.6/F.2.7.
use anyhow::{Context, Result, ensure};

pub(super) fn decode(
    datatype: &str,
    length: Option<u16>,
    encoding: Option<&str>,
    payload: &[u8],
) -> Result<String> {
    let length = usize::from(length.context("IODD string length is unavailable")?);
    ensure!(
        (1..=232).contains(&length),
        "declared string length must be 1–232 octets"
    );
    if datatype == "OctetStringT" {
        ensure!(
            payload.len() == length,
            "octet string response width differs from the IODD"
        );
        return Ok(payload
            .iter()
            .map(|byte| format!("0x{byte:02X}"))
            .collect::<Vec<_>>()
            .join(","));
    }
    ensure!(datatype == "StringT", "unsupported string datatype");
    ensure!(
        payload.len() <= length,
        "parameter response exceeds the IODD text length"
    );
    let encoding = encoding.context("IODD text encoding is unavailable")?;
    ensure!(
        matches!(encoding, "UTF-8" | "US-ASCII"),
        "unsupported parameter text encoding {encoding}"
    );
    let end = payload
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(payload.len());
    ensure!(
        payload[end..].iter().all(|byte| *byte == 0),
        "nonzero string padding follows the first NUL"
    );
    let text = &payload[..end];
    ensure!(
        encoding != "US-ASCII" || text.is_ascii(),
        "parameter response is not US-ASCII"
    );
    Ok(std::str::from_utf8(text)?.to_owned())
}

#[cfg(test)]
mod tests;
