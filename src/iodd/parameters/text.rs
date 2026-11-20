//! StringT and OctetStringT values, IO-Link Interface V1.1.5 F.2.6/F.2.7.
use super::Parameter;
use anyhow::{Context, Result, ensure};

fn checked_length(length: Option<u16>) -> Result<usize> {
    let length = usize::from(length.context("IODD string length is unavailable")?);
    ensure!(
        (1..=232).contains(&length),
        "declared string length must be 1–232 octets"
    );
    Ok(length)
}

pub(super) fn encode(parameter: &Parameter, input: &str) -> Result<Vec<u8>> {
    let length = checked_length(parameter.fixed_length)?;
    if parameter.datatype == "StringT" {
        ensure!(
            matches!(parameter.encoding.as_deref(), Some("UTF-8" | "US-ASCII")),
            "IODD text encoding is unavailable or unsupported"
        );
        ensure!(
            input.len() <= length,
            "text exceeds the declared octet length"
        );
        ensure!(
            parameter.encoding.as_deref() != Some("US-ASCII") || input.is_ascii(),
            "text is not US-ASCII"
        );
        ensure!(!input.contains('\0'), "text contains NUL");
        let mut bytes = input.as_bytes().to_vec();
        bytes.resize(length, 0);
        return Ok(bytes);
    }

    ensure!(
        parameter.datatype == "OctetStringT",
        "unsupported string datatype"
    );
    ensure!(
        input.len() < 5 * length,
        "octet sequence exceeds the declared length"
    );
    let bytes = input
        .split(',')
        .map(|part| -> Result<u8> {
            let digits = part
                .strip_prefix("0x")
                .context("use comma-separated 0xNN entries")?;
            ensure!(
                digits.len() == 2 && digits.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "invalid octet"
            );
            Ok(u8::from_str_radix(digits, 16)?)
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        bytes.len() == length,
        "octet count differs from the declared length"
    );
    Ok(bytes)
}

pub(crate) fn decode(
    datatype: &str,
    length: Option<u16>,
    encoding: Option<&str>,
    payload: &[u8],
) -> Result<String> {
    let length = checked_length(length)?;
    if datatype == "OctetStringT" {
        ensure!(
            payload.len() == length,
            "octet string response width differs from the IODD"
        );
        return Ok(crate::decode::octet_sequence(payload));
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
    Ok(crate::decode::decode_padded_text(payload, encoding == "US-ASCII")?.to_owned())
}

#[cfg(test)]
mod tests;
