//! Read-only binary CRC receipts, IODD V1.1.5 section 7.3.4.
use super::{IODD_NAMESPACE, resources::Companion};
use anyhow::{Context, Result, ensure};
use roxmltree::Document;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Matched,
    Missing,
    Mismatch,
    Malformed,
    Unbound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileReport {
    pub source: String,
    pub sha256: String,
    pub bytes: u64,
    pub encoding: Encoding,
    pub status: Status,
    pub declared_crc: Option<u32>,
    pub calculated_crc: Option<u32>,
    /// Decimal ASCII main CRC appended after a language document's original bytes.
    pub main_crc: Option<u32>,
    pub checker_name: Option<String>,
    pub checker_version: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub format_version: u32,
    pub main: FileReport,
    pub language: Option<FileReport>,
}

impl Report {
    pub fn matched(&self) -> bool {
        self.main.status == Status::Matched
            && self
                .language
                .as_ref()
                .is_none_or(|file| file.status == Status::Matched)
    }

    pub(crate) fn enforce(&self, required: bool) -> Result<()> {
        for file in std::iter::once(&self.main).chain(&self.language) {
            ensure!(
                !matches!(file.status, Status::Mismatch | Status::Malformed),
                "IODD stamp {:?}: {} · {}",
                file.status,
                file.source,
                file.notes.join("; ")
            );
        }
        ensure!(
            !required || self.matched(),
            "matching IODD main and selected-language stamps are required: {}",
            self.summary()
        );
        Ok(())
    }

    pub fn summary(&self) -> String {
        let label = |status| match status {
            Status::Matched => "checksum matched",
            Status::Missing => "stamp missing",
            Status::Mismatch => "checksum mismatch",
            Status::Malformed => "stamp malformed",
            Status::Unbound => "main CRC unavailable; language not verified",
        };
        let mut summary = format!("IODD main {}", label(self.main.status));
        if let Some(file) = &self.language {
            summary.push_str(&format!(" · selected language {}", label(file.status)));
        }
        summary
    }

    /// Validate saved receipt consistency without reopening any recorded path.
    pub fn validate_receipt(&self) -> Result<()> {
        ensure!(
            self.format_version == 1,
            "unsupported IODD stamp receipt version"
        );
        ensure!(
            self.main.main_crc.is_none() && self.main.status != Status::Unbound,
            "main stamp cannot depend on another main CRC"
        );
        for file in std::iter::once(&self.main).chain(&self.language) {
            ensure!(
                !file.source.trim().is_empty() && (1..=super::XML_LIMIT).contains(&file.bytes),
                "invalid stamp source receipt"
            );
            ensure!(
                file.sha256.len() == 64
                    && file
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
                "invalid stamp source hash"
            );
            match file.status {
                Status::Matched | Status::Mismatch => {
                    let declared = file
                        .declared_crc
                        .context("stamp receipt has no declared CRC")?;
                    let calculated = file
                        .calculated_crc
                        .context("stamp receipt has no calculated CRC")?;
                    ensure!(
                        (declared == calculated) == (file.status == Status::Matched),
                        "stamp CRC result contradicts its status"
                    );
                    ensure!(
                        checker(
                            file.checker_name.as_deref(),
                            file.checker_version.as_deref()
                        ),
                        "stamp receipt has invalid checker metadata"
                    );
                }
                Status::Missing => ensure!(
                    file.declared_crc.is_none()
                        && file.calculated_crc.is_none()
                        && file.main_crc.is_none()
                        && file.checker_name.is_none()
                        && file.checker_version.is_none(),
                    "missing stamp receipt contains stamp values"
                ),
                Status::Unbound => ensure!(
                    file.main_crc.is_none()
                        && file.declared_crc.is_some()
                        && file.calculated_crc.is_none()
                        && checker(
                            file.checker_name.as_deref(),
                            file.checker_version.as_deref()
                        ),
                    "invalid unbound language stamp receipt"
                ),
                Status::Malformed => ensure!(
                    !file.notes.is_empty(),
                    "malformed stamp receipt has no explanation"
                ),
            }
        }
        if let Some(file) = &self.language {
            if matches!(file.status, Status::Matched | Status::Mismatch) {
                ensure!(
                    self.main.status == Status::Matched && file.main_crc == self.main.declared_crc,
                    "language stamp binding contradicts the main receipt"
                );
            }
            if file.status == Status::Unbound {
                ensure!(
                    self.main.status != Status::Matched,
                    "language stamp is unbound despite a verified main CRC"
                );
            }
        }
        Ok(())
    }
}

fn checker(name: Option<&str>, version: Option<&str>) -> bool {
    name.is_some_and(|name| !name.trim().is_empty())
        && version
            .and_then(|version| version.strip_prefix('V'))
            .is_some_and(|version| {
                let parts: Vec<_> = version.split('.').collect();
                (2..=8).contains(&parts.len())
                    && parts.iter().all(|part| {
                        !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit())
                    })
            })
}

pub(super) fn inspect(
    bytes: &[u8],
    source: String,
    language: bool,
    main_crc: Option<u32>,
) -> Result<FileReport> {
    let xml = super::decode_xml(bytes.to_vec())?;
    let document = Document::parse(&xml)?;
    let root = document.root_element();
    ensure!(
        root.has_tag_name((
            IODD_NAMESPACE,
            if language {
                "ExternalTextDocument"
            } else {
                "IODevice"
            }
        )),
        "unexpected XML root for IODD stamp verification"
    );
    let encoding = match bytes {
        [0xFF, 0xFE, ..] => Encoding::Utf16Le,
        [0xFE, 0xFF, ..] => Encoding::Utf16Be,
        _ => Encoding::Utf8,
    };
    let mut report = FileReport {
        source,
        sha256: format!("{:x}", Sha256::digest(bytes)),
        bytes: bytes.len() as u64,
        encoding,
        status: Status::Missing,
        declared_crc: None,
        calculated_crc: None,
        main_crc: None,
        checker_name: None,
        checker_version: None,
        notes: Vec::new(),
    };
    let stamps: Vec<_> = root
        .descendants()
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "Stamp")))
        .collect();
    if stamps.is_empty() {
        report
            .notes
            .push("Stamp is absent; checksum not verified".into());
        return Ok(report);
    }
    report.status = Status::Malformed;
    if stamps.len() != 1 || stamps[0].parent() != Some(root) {
        report
            .notes
            .push("Expected exactly one direct Stamp child".into());
        return Ok(report);
    }
    let stamp = stamps[0];
    let checkers: Vec<_> = stamp
        .children()
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "Checker")))
        .collect();
    if checkers.len() == 1 {
        report.checker_name = checkers[0].attribute("name").map(str::to_owned);
        report.checker_version = checkers[0].attribute("version").map(str::to_owned);
    }
    if !checker(
        report.checker_name.as_deref(),
        report.checker_version.as_deref(),
    ) {
        report
            .notes
            .push("Stamp requires one Checker with a name and V-prefixed numeric version".into());
    }
    let Some(attribute) = stamp.attribute_node("crc") else {
        report.notes.push("Stamp has no crc attribute".into());
        return Ok(report);
    };
    let lexical = attribute.value().trim_matches([' ', '\t', '\r', '\n']);
    let digits = lexical.strip_prefix('+').unwrap_or(lexical);
    if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
        report.declared_crc = lexical.parse().ok();
    }
    if report.declared_crc.is_none() {
        report
            .notes
            .push("Stamp crc must be a decimal unsigned 32-bit value".into());
    }
    if language && main_crc.is_none() {
        if report.notes.is_empty() {
            report.status = Status::Unbound;
            report
                .notes
                .push("Language CRC requires a matching main-document stamp".into());
        }
        return Ok(report);
    }
    // roxmltree ranges locate the actual attribute, avoiding text/comment decoys.
    // Map UTF-8 parser offsets back to the original encoding; never normalize EOL/BOM.
    let raw_offset = |offset: usize| match encoding {
        Encoding::Utf8 => bytes.len() - xml.len() + offset,
        Encoding::Utf16Le | Encoding::Utf16Be => {
            bytes.len() - xml.encode_utf16().count() * 2 + xml[..offset].encode_utf16().count() * 2
        }
    };
    let range = attribute.range_value();
    let start = raw_offset(range.start);
    let end = raw_offset(range.end);
    let mut crc = crc32fast::Hasher::new();
    crc.update(&bytes[..start]);
    crc.update(&bytes[end..]);
    if language {
        report.main_crc = main_crc;
        crc.update(main_crc.unwrap().to_string().as_bytes());
    }
    report.calculated_crc = Some(crc.finalize());
    if report.notes.is_empty() {
        report.status = if report.declared_crc == report.calculated_crc {
            Status::Matched
        } else {
            Status::Mismatch
        };
        if report.status == Status::Mismatch {
            report.notes.push(format!(
                "Declared CRC {} differs from calculated CRC {}",
                report.declared_crc.unwrap(),
                report.calculated_crc.unwrap()
            ));
        }
    }
    Ok(report)
}

pub(super) fn check(bytes: &[u8], source: String, companion: Option<&Companion>) -> Result<Report> {
    let main = inspect(bytes, source, false, None)?;
    let main_crc = (main.status == Status::Matched)
        .then_some(main.declared_crc)
        .flatten();
    let language = companion
        .map(|file| inspect(&file.bytes, file.source.clone(), true, main_crc))
        .transpose()?;
    let report = Report {
        format_version: 1,
        main,
        language,
    };
    report.validate_receipt()?;
    Ok(report)
}

#[cfg(test)]
mod tests;
