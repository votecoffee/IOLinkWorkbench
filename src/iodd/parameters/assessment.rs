//! Compare unscaled numeric values with IODD SingleValue/ValueRange definitions.
use super::Parameter;
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssessmentStatus {
    Allowed,
    OutsideDefinition,
    DefinitionNeedsReview,
    NotComparable,
    NotEvaluated,
}

#[derive(Debug, Serialize)]
pub struct ValueAssessment {
    pub status: AssessmentStatus,
    pub matched_choices: usize,
    pub matched_ranges: usize,
    pub notes: Vec<String>,
}

impl ValueAssessment {
    fn note(&mut self, message: impl Into<String>) {
        // Bound diagnostic output even for a large, malformed local catalog.
        if self.notes.len() < 16 {
            self.notes.push(message.into());
        } else if self.notes.len() == 16 {
            self.notes
                .push("Additional definition issues omitted".into());
        }
    }

    pub fn text(&self) -> String {
        let status = match self.status {
            AssessmentStatus::Allowed => "allowed by declared values",
            AssessmentStatus::OutsideDefinition => "outside declared values",
            AssessmentStatus::DefinitionNeedsReview => "definition needs review",
            AssessmentStatus::NotComparable => "value is not comparable",
            AssessmentStatus::NotEvaluated => "datatype is not evaluated",
        };
        let mut text = format!(
            "IODD value assessment · {status} · {} choice / {} range matches",
            self.matched_choices, self.matched_ranges
        );
        for note in &self.notes {
            text.push_str(&format!("\n  {note}"));
        }
        text
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd)]
enum Atom {
    Boolean(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f32),
}

fn supported(parameter: &Parameter) -> bool {
    matches!(
        parameter.datatype.as_str(),
        "BooleanT" | "UIntegerT" | "IntegerT" | "Float32T"
    )
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

fn lexical(parameter: &Parameter, text: &str) -> Result<Atom> {
    // XML Schema numeric/Boolean whitespace is collapsed; strings are not touched.
    let text = text.trim_matches([' ', '\t', '\r', '\n']);
    Ok(match parameter.datatype.as_str() {
        "BooleanT" => Atom::Boolean(match text {
            "true" | "1" => true,
            "false" | "0" => false,
            _ => bail!("expected true, false, 1 or 0"),
        }),
        "UIntegerT" | "IntegerT" => {
            let bits = parameter.bits.context("numeric width is unavailable")?;
            ensure!((1..=64).contains(&bits), "numeric width is unsupported");
            let unsigned = parameter.datatype == "UIntegerT";
            let magnitude = text.strip_prefix('+').unwrap_or(text);
            let magnitude = if unsigned {
                magnitude
            } else {
                magnitude.strip_prefix('-').unwrap_or(magnitude)
            };
            ensure!(digits(magnitude), "expected a decimal integer");
            if unsigned {
                let value: u64 = text.parse()?;
                ensure!(
                    u128::from(value) < (1u128 << bits),
                    "value exceeds the declared unsigned width"
                );
                Atom::Unsigned(value)
            } else {
                let value: i64 = text.parse()?;
                let limit = 1i128 << (bits - 1);
                ensure!(
                    (-limit..limit).contains(&i128::from(value)),
                    "value exceeds the declared signed width"
                );
                Atom::Signed(value)
            }
        }
        "Float32T" => {
            let value = match text {
                "INF" => f32::INFINITY,
                "-INF" => f32::NEG_INFINITY,
                "NaN" => bail!("NaN cannot define an ordered value or range"),
                _ => {
                    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
                    let mut exponent = body.split(['e', 'E']);
                    let mantissa = exponent.next().unwrap_or("");
                    if let Some(power) = exponent.next() {
                        ensure!(
                            digits(power.strip_prefix(['+', '-']).unwrap_or(power)),
                            "invalid float exponent"
                        );
                    }
                    ensure!(exponent.next().is_none(), "invalid float exponent");
                    let mut parts = mantissa.split('.');
                    let whole = parts.next().unwrap_or("");
                    let fraction = parts.next();
                    ensure!(parts.next().is_none(), "invalid float mantissa");
                    ensure!(
                        (digits(whole)
                            && fraction.is_none_or(|part| part.is_empty() || digits(part)))
                            || (whole.is_empty() && fraction.is_some_and(digits)),
                        "expected an XML Schema float"
                    );
                    let number: f32 = text.parse()?;
                    ensure!(
                        number.is_finite(),
                        "non-finite numeric literal; use INF or -INF explicitly"
                    );
                    number
                }
            };
            Atom::Float(value)
        }
        _ => bail!("datatype does not support numeric value assessment"),
    })
}

/// Semantic equality for standard definition references; no float widening.
pub(super) fn equivalent(parameter: &Parameter, left: &str, right: &str) -> bool {
    if !supported(parameter) {
        return left == right;
    }
    matches!((lexical(parameter, left), lexical(parameter, right)), (Ok(a), Ok(b)) if a == b)
}

pub(super) fn assess(
    parameter: &Parameter,
    raw: &str,
) -> (Option<ValueAssessment>, Option<String>) {
    if parameter.choices.is_empty() && parameter.ranges.is_empty() {
        return (None, None);
    }
    let mut result = ValueAssessment {
        status: AssessmentStatus::NotEvaluated,
        matched_choices: 0,
        matched_ranges: 0,
        notes: Vec::new(),
    };
    if !supported(parameter) {
        result.note("Only BooleanT, IntegerT, UIntegerT and Float32T definitions are evaluated");
        return (Some(result), None);
    }
    let mut choices = Vec::new();
    let mut ranges = Vec::new();
    let mut intervals = Vec::new();
    for choice in &parameter.choices {
        match lexical(parameter, &choice.value) {
            Ok(atom) => {
                choices.push((atom, choice));
                intervals.push((atom, atom));
            }
            Err(error) => result.note(format!("Choice {}: {error}", choice.value)),
        }
    }
    for range in &parameter.ranges {
        let parsed = (|| -> Result<(Atom, Atom)> {
            ensure!(
                parameter.datatype != "BooleanT",
                "Boolean ranges are unsupported"
            );
            let lower = lexical(parameter, &range.lower)?;
            let upper = lexical(parameter, &range.upper)?;
            ensure!(lower < upper, "lower bound must be less than upper bound");
            Ok((lower, upper))
        })();
        match parsed {
            Ok(bounds) => {
                ranges.push(bounds);
                intervals.push(bounds);
            }
            Err(error) => result.note(format!("Range {}…{}: {error}", range.lower, range.upper)),
        }
    }
    // All atoms belong to one declared datatype and metadata NaNs are rejected.
    // Sorting also catches aliases, shared inclusive boundaries and nested ranges.
    intervals.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(Ordering::Equal));
    let mut previous_upper = None;
    for (lower, upper) in intervals {
        if previous_upper.is_some_and(|previous| previous >= lower) {
            result.note("SingleValue/ValueRange definitions overlap or contain equivalent choices");
        }
        if previous_upper.is_none_or(|previous| upper > previous) {
            previous_upper = Some(upper);
        }
    }
    let observed = if parameter.datatype == "Float32T" {
        raw.parse::<f32>()
            .map(Atom::Float)
            .map_err(anyhow::Error::from)
    } else {
        lexical(parameter, raw)
    };
    let mut choice_name = None;
    match observed {
        Ok(atom) if atom.partial_cmp(&atom).is_some() => {
            for (candidate, choice) in choices {
                if atom == candidate {
                    result.matched_choices += 1;
                    choice_name = Some(choice.name.clone());
                }
            }
            result.matched_ranges = ranges
                .iter()
                .filter(|(lower, upper)| *lower <= atom && atom <= *upper)
                .count();
            result.status = if !result.notes.is_empty() {
                AssessmentStatus::DefinitionNeedsReview
            } else if result.matched_choices + result.matched_ranges > 0 {
                AssessmentStatus::Allowed
            } else {
                AssessmentStatus::OutsideDefinition
            };
        }
        _ => {
            result.status = if result.notes.is_empty() {
                AssessmentStatus::NotComparable
            } else {
                AssessmentStatus::DefinitionNeedsReview
            };
            result.note("Observed value has no comparable numeric value (for example NaN)");
        }
    }
    if result.matched_choices != 1 {
        choice_name = None;
    }
    (Some(result), choice_name)
}

#[cfg(test)]
mod tests;
