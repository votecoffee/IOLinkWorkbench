use crate::{iodd::ErrorTypeDefinition, profile::Profile};
use anyhow::{Result, bail};
use serde::Serialize;
use std::fmt::Write as _;

#[derive(Debug, Serialize)]
pub struct ErrorTypeReport {
    pub raw_hex: String,
    pub code: u16,
    pub error_code: u8,
    pub additional_code: u8,
    pub application_family: &'static str,
    pub standard_meaning: Option<&'static str>,
    pub iodd_definition: Option<ErrorTypeDefinition>,
    pub unresolved_standard_reference: bool,
}

impl ErrorTypeReport {
    pub fn text(&self) -> String {
        let mut text = format!("ErrorType 0x{} · {}", self.raw_hex, self.application_family);
        if let Some(meaning) = self.standard_meaning {
            let _ = write!(text, "\n{meaning}");
        }
        if let Some(definition) = &self.iodd_definition {
            let _ = write!(text, "\nIODD · {}", definition.name);
            if let Some(description) = &definition.description {
                let _ = write!(text, "\n{description}");
            }
            if let Some(origin) = &definition.origin {
                let _ = write!(text, "\n{origin}");
            }
        }
        if self.unresolved_standard_reference {
            text.push_str("\nIODD standard reference needs supplied definitions");
        }
        text
    }
}

/// Decode the two bytes of an explicitly identified ErrorType from a saved reply.
pub fn decode_error_type(profile: Option<&Profile>, bytes: &[u8]) -> Result<ErrorTypeReport> {
    let [error_code, additional_code] = *bytes else {
        bail!("an ISDU ErrorType requires exactly two bytes");
    };
    let code = u16::from_be_bytes([error_code, additional_code]);
    let imported = profile.and_then(|profile| profile.iodd.as_ref());
    let iodd_definition = imported
        .and_then(|iodd| iodd.error_types.iter().find(|item| item.code == code))
        .cloned();
    let unresolved_standard_reference =
        imported.is_some_and(|iodd| iodd.standard_error_type_refs.contains(&code));
    // IO-Link Interface and System Specification, Annex C.2, Table C.1.
    let standard_meaning = match code {
        0x8000 => Some("Device application refused the service without further details"),
        0x8011 => Some("Index is unavailable"),
        0x8012 => Some("Subindex is unavailable"),
        0x8020 => Some("Service is temporarily unavailable"),
        0x8021 => Some("Service is temporarily unavailable during local control"),
        0x8022 => Some("Service is temporarily unavailable during device control"),
        0x8023 => Some("Access is denied"),
        0x8030 => Some("Parameter value is outside its range"),
        0x8031 => Some("Parameter value exceeds its limit"),
        0x8032 => Some("Parameter value falls below its limit"),
        0x8033 => Some("Parameter data is longer than required"),
        0x8034 => Some("Parameter data is shorter than required"),
        0x8035 => Some("Function is unsupported"),
        0x8036 => Some("Function is temporarily unavailable"),
        0x8040 => Some("Parameter value conflicts with the current parameter set"),
        0x8041 => Some("Transferred parameter set is inconsistent"),
        0x8082 => Some("Device application is temporarily unavailable"),
        _ => None,
    };
    let application_family = match error_code {
        0x80 if standard_meaning.is_some() || iodd_definition.is_some() => {
            "Standard device application error"
        }
        0x80 => "Unrecognized standard device application ErrorType",
        0x81 => "Vendor-specific device application error",
        _ => "Outside the device application ErrorType table",
    };
    Ok(ErrorTypeReport {
        raw_hex: format!("{code:04X}"),
        code,
        error_code,
        additional_code,
        application_family,
        standard_meaning,
        iodd_definition,
        unresolved_standard_reference,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_exact_wire_bytes_without_assuming_unknown_codes() -> Result<()> {
        let standard = decode_error_type(None, &[0x80, 0x11])?;
        assert_eq!(standard.code, 0x8011);
        assert_eq!(standard.standard_meaning, Some("Index is unavailable"));
        assert!(standard.iodd_definition.is_none() && !standard.unresolved_standard_reference);
        let vendor = decode_error_type(None, &[0x81, 0x11])?;
        assert_eq!((vendor.error_code, vendor.additional_code), (0x81, 0x11));
        assert!(vendor.standard_meaning.is_none());
        let unknown = decode_error_type(None, &[0xff, 0x11])?;
        assert_eq!(unknown.raw_hex, "FF11");
        assert!(unknown.standard_meaning.is_none());
        for bytes in [&[][..], &[0x80], &[0x80, 0x11, 0x00]] {
            assert!(decode_error_type(None, bytes).is_err());
        }
        Ok(())
    }
}
