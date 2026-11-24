use crate::{decode, iodd::format_profile_ids, profile::Profile};
use anyhow::Result;
use serde::Serialize;
use std::{collections::BTreeSet, fmt::Write};

const MAX_ISDU_BYTES: usize = 232;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileCharacteristicStatus {
    Match,
    Different,
    Undeclared,
    InvalidReply,
}

#[derive(Debug, Serialize)]
pub struct ProfileCharacteristic {
    pub index: u16,
    pub subindex: u8,
    pub status: ProfileCharacteristicStatus,
    pub raw_hex: String,
    pub declared_ids: Option<Vec<u16>>,
    pub reply_ids: Vec<u16>,
    pub comparison: Option<ProfileIdComparison>,
    pub decode_error: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct ProfileIdComparison {
    pub missing_ids: Vec<u16>,
    pub unexpected_ids: Vec<u16>,
}

impl ProfileCharacteristic {
    pub fn text(&self) -> String {
        let label = match self.status {
            ProfileCharacteristicStatus::Match => "Profile IDs match the IODD",
            ProfileCharacteristicStatus::Different => "Profile IDs differ from the IODD",
            ProfileCharacteristicStatus::Undeclared => "IODD profile IDs are undeclared",
            ProfileCharacteristicStatus::InvalidReply => "ProfileCharacteristic reply needs review",
        };
        let declared = self
            .declared_ids
            .as_deref()
            .map_or_else(|| "Undeclared".into(), format_profile_ids);
        let mut text = format!(
            "{label}\nRaw reply · {}\nIODD profile IDs · {declared}",
            self.raw_hex
        );
        if !self.reply_ids.is_empty() {
            let _ = write!(
                text,
                "\nReply profile IDs · {}",
                format_profile_ids(&self.reply_ids)
            );
        }
        if let Some(error) = self.decode_error {
            let _ = write!(text, "\n{error}");
        }
        if let Some(comparison) = &self.comparison {
            if !comparison.missing_ids.is_empty() {
                let _ = write!(
                    text,
                    "\nMissing from reply · {}",
                    format_profile_ids(&comparison.missing_ids)
                );
            }
            if !comparison.unexpected_ids.is_empty() {
                let _ = write!(
                    text,
                    "\nUnexpected in reply · {}",
                    format_profile_ids(&comparison.unexpected_ids)
                );
            }
        }
        text
    }
}

/// Interpret the complete ProfileCharacteristic ISDU reply at index 13, subindex 0.
pub fn decode_profile_characteristic(
    profile: Option<&Profile>,
    hex: &str,
) -> Result<ProfileCharacteristic> {
    let bytes = if hex.trim().is_empty() {
        Vec::new()
    } else {
        decode::parse_hex_bounded(hex, MAX_ISDU_BYTES)?
    };
    let (words, tail) = bytes.as_chunks::<2>();
    let declared_ids = profile
        .and_then(|profile| profile.iodd.as_ref())
        .and_then(|iodd| iodd.features.as_ref())
        .and_then(|features| features.profile_characteristic.clone());
    let mut report = ProfileCharacteristic {
        index: 13,
        subindex: 0,
        status: ProfileCharacteristicStatus::InvalidReply,
        raw_hex: decode::hex(&bytes),
        declared_ids,
        reply_ids: Vec::new(),
        comparison: None,
        decode_error: None,
    };
    if words.is_empty() || !tail.is_empty() {
        report.decode_error =
            Some("ProfileCharacteristic (13:0) requires one or more complete 16-bit identifiers");
        return Ok(report);
    }
    report.reply_ids = words.iter().copied().map(u16::from_be_bytes).collect();
    if report.reply_ids.windows(2).any(|pair| pair[0] >= pair[1]) {
        report.decode_error = Some("Reply profile IDs must be unique and in ascending order");
        return Ok(report);
    }
    let Some(declared) = &report.declared_ids else {
        report.status = ProfileCharacteristicStatus::Undeclared;
        return Ok(report);
    };
    let expected: BTreeSet<_> = declared.iter().copied().collect();
    let observed: BTreeSet<_> = report.reply_ids.iter().copied().collect();
    report.status = if expected == observed {
        ProfileCharacteristicStatus::Match
    } else {
        ProfileCharacteristicStatus::Different
    };
    report.comparison = Some(ProfileIdComparison {
        missing_ids: expected.difference(&observed).copied().collect(),
        unexpected_ids: observed.difference(&expected).copied().collect(),
    });
    Ok(report)
}
