use super::{Access, AssessmentStatus, DisplayValue, Parameter};
use crate::decode::{extract_bits, hex};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct DecodedParameter {
    pub raw_hex: String,
    pub raw_value: Option<String>,
    pub displays: Vec<DisplayValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_display: Option<DisplayValue>,
    pub choice_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_assessment: Option<super::ValueAssessment>,
    pub decode_error: Option<String>,
    pub record_items: Vec<RecordValue>,
    pub array_items: Vec<RecordValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_value: Option<super::TimeValue>,
}

#[derive(Debug, Serialize)]
pub struct RecordValue {
    pub subindex: u8,
    pub name: String,
    pub bit_offset: u16,
    pub value: DecodedParameter,
}

pub(crate) struct WriteValueReview {
    pub decoded: DecodedParameter,
    pub manual_review: Option<String>,
    pub preserved_read_only: Vec<u8>,
}

impl DecodedParameter {
    pub fn needs_review(&self) -> bool {
        self.decode_error.is_some()
            || self
                .record_items
                .iter()
                .chain(&self.array_items)
                .any(|item| item.value.needs_review())
    }

    pub(super) fn error(payload: &[u8], error: impl ToString) -> Self {
        Self {
            raw_hex: hex(payload),
            raw_value: None,
            displays: Vec::new(),
            preferred_display: None,
            choice_name: None,
            value_assessment: None,
            decode_error: Some(error.to_string()),
            record_items: Vec::new(),
            array_items: Vec::new(),
            time_value: None,
        }
    }

    pub fn text(&self) -> String {
        let mut text = format!("Raw bytes {}", self.raw_hex);
        if let Some(value) = &self.raw_value {
            text.push_str(&format!("\nRaw value {value}"));
        }
        if let Some(name) = &self.choice_name {
            text.push_str(&format!(" · {name}"));
        }
        if let Some(time) = &self.time_value {
            text.push_str(&format!("\n{}", time.text()));
        }
        if let Some(assessment) = &self.value_assessment {
            text.push_str(&format!("\n{}", assessment.text()));
        }
        for display in &self.displays {
            text.push_str(&format!(
                "\nDisplay {}{} · {} {}",
                display.id,
                if self
                    .preferred_display
                    .as_ref()
                    .is_some_and(|preferred| preferred.id == display.id)
                {
                    " (preferred)"
                } else {
                    ""
                },
                display.value,
                display.unit
            ));
        }
        if let Some(error) = &self.decode_error {
            text.push_str(&format!("\nDecode review · {error}"));
        }
        for item in self.record_items.iter().chain(&self.array_items) {
            text.push_str(&format!(
                "\n\n{} · subindex {} · bit {}\n{}",
                item.name,
                item.subindex,
                item.bit_offset,
                item.value.text()
            ));
        }
        text
    }

    fn review_write_value(&self) -> Result<Option<&'static str>> {
        if self.raw_value.is_none() {
            if let Some(error) = &self.decode_error {
                bail!("{error}");
            }
            return Ok(Some(
                "Review the raw bytes against the device documentation for this datatype.",
            ));
        }
        match self.value_assessment.as_ref().map(|value| value.status) {
            Some(AssessmentStatus::OutsideDefinition) => {
                bail!("value is outside the IODD choices or ranges")
            }
            Some(AssessmentStatus::NotComparable) => {
                bail!("value cannot be compared with the IODD choices or ranges")
            }
            Some(AssessmentStatus::DefinitionNeedsReview | AssessmentStatus::NotEvaluated) => {
                Ok(Some("Review the declared choices or ranges manually."))
            }
            _ if self.decode_error.is_some() => {
                Ok(Some("Review the IODD display interpretation manually."))
            }
            _ => Ok(None),
        }
    }
}

impl Parameter {
    /// Review a write image offline. A compound image may change only declared writable bits.
    pub(crate) fn review_write(
        &self,
        expected: &[u8],
        proposed: &[u8],
    ) -> Result<WriteValueReview> {
        let decoded = self.decode(proposed);
        if !matches!(self.datatype.as_str(), "RecordT" | "ArrayT") {
            return Ok(WriteValueReview {
                manual_review: decoded.review_write_value()?.map(str::to_owned),
                decoded,
                preserved_read_only: Vec::new(),
            });
        }

        ensure!(
            self.record.is_some() || self.array.is_some(),
            "IODD compound layout is unavailable"
        );
        let length = self.width().map(|bits| usize::from(bits).div_ceil(8));
        ensure!(
            length == Some(proposed.len()),
            "proposed compound payload width differs from the IODD"
        );
        ensure!(
            length == Some(expected.len()),
            "expected compound payload width differs from the IODD"
        );
        if let Some(error) = &decoded.decode_error {
            bail!("{error}");
        }

        let mut writable = vec![0u8; proposed.len()];
        let mut preserved_read_only = Vec::new();
        let mut manual_members = Vec::new();
        for item in decoded.record_items.iter().chain(&decoded.array_items) {
            let member = self.target(item.subindex)?;
            let subindex = item.subindex;
            ensure!(
                member.access != Access::WriteOnly,
                "subindex {subindex} is write-only and cannot be verified by readback"
            );
            let width = member
                .width()
                .with_context(|| format!("IODD width is unavailable for subindex {subindex}"))?;
            if member.access == Access::ReadOnly {
                ensure!(
                    extract_bits(expected, item.bit_offset, width)?
                        == extract_bits(proposed, item.bit_offset, width)?,
                    "proposed value changes read-only subindex {subindex}"
                );
                preserved_read_only.push(subindex);
            } else {
                if item
                    .value
                    .review_write_value()
                    .with_context(|| format!("subindex {subindex} ({})", item.name))?
                    .is_some()
                {
                    manual_members.push(subindex.to_string());
                }
                let length = writable.len();
                for bit in item.bit_offset..item.bit_offset + width {
                    writable[length - 1 - usize::from(bit / 8)] |= 1 << (bit % 8);
                }
            }
        }
        ensure!(
            expected
                .iter()
                .zip(proposed)
                .zip(&writable)
                .all(|((&before, &after), &mask)| (before ^ after) & !mask == 0),
            "proposed payload changes bits outside declared writable members"
        );
        Ok(WriteValueReview {
            decoded,
            manual_review: (!manual_members.is_empty()).then(|| {
                format!(
                    "Review the IODD details for subindexes {} manually.",
                    manual_members.join(", ")
                )
            }),
            preserved_read_only,
        })
    }

    /// Decode an acyclic payload without any transport access. Raw bytes survive errors.
    pub fn decode(&self, payload: &[u8]) -> DecodedParameter {
        if payload.is_empty() || payload.len() > 232 {
            return DecodedParameter::error(payload, "parameter payload must contain 1–232 bytes");
        }
        let raw_value = match self.decode_raw(payload) {
            Ok(value) => value,
            Err(error) => return DecodedParameter::error(payload, error),
        };
        let mut value = DecodedParameter {
            raw_hex: hex(payload),
            raw_value,
            displays: Vec::new(),
            preferred_display: None,
            choice_name: None,
            value_assessment: None,
            decode_error: None,
            record_items: Vec::new(),
            array_items: Vec::new(),
            time_value: None,
        };
        if let Some(raw) = &value.raw_value {
            (value.value_assessment, value.choice_name) = super::assessment::assess(self, raw);
        }
        if matches!(self.datatype.as_str(), "TimeT" | "TimeSpanT") {
            match super::time::decode(&self.datatype, payload) {
                Ok(time) => value.time_value = Some(time),
                Err(error) => value.decode_error = Some(error.to_string()),
            }
        }
        match self.decode_displays(payload) {
            Ok(displays) => value.displays = displays,
            Err(error) => value.decode_error = Some(error.to_string()),
        }
        value.preferred_display = value
            .displays
            .iter()
            .find(|display| Some(&display.id) == self.preferred_display.as_ref())
            .cloned();
        if let Some(record) = &self.record {
            if self.bits.map(|bits| usize::from(bits).div_ceil(8)) != Some(payload.len()) {
                return DecodedParameter::error(
                    payload,
                    "record response width differs from the IODD",
                );
            }
            for item in &record.items {
                let decoded = match item.extract(payload) {
                    Ok(bytes) if item.parameter.access == Access::WriteOnly => {
                        DecodedParameter::error(
                            &bytes,
                            "write-only record item; typed interpretation skipped",
                        )
                    }
                    Ok(bytes) => item.parameter.decode(&bytes),
                    Err(error) => DecodedParameter::error(&[], error),
                };
                value.record_items.push(RecordValue {
                    subindex: item.subindex,
                    name: item.parameter.name.clone(),
                    bit_offset: item.bit_offset,
                    value: decoded,
                });
            }
        }
        if let Some(array) = &self.array {
            let geometry = match array.geometry() {
                Ok(layout) => layout,
                Err(error) => return DecodedParameter::error(payload, error),
            };
            if geometry.bits().div_ceil(8) as usize != payload.len() {
                return DecodedParameter::error(
                    payload,
                    "array response width differs from the IODD",
                );
            }
            for subindex in 1..=array.count {
                let bit_offset = geometry.offset(subindex);
                let decoded = match crate::decode::extract_bits(payload, bit_offset, geometry.width)
                {
                    Ok(bytes) => array.element.decode(&bytes),
                    Err(error) => DecodedParameter::error(&[], error),
                };
                value.array_items.push(RecordValue {
                    subindex,
                    name: format!("{} [{subindex}]", self.name),
                    bit_offset,
                    value: decoded,
                });
            }
        }
        value
    }
}
