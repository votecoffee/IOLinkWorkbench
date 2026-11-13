use super::Parameter;
use crate::{
    decode::{Field, Kind},
    iodd::{IODD_NAMESPACE, child},
};
use anyhow::{Result, ensure};
use roxmltree::Node;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NumberFormat {
    #[default]
    Decimal,
    BooleanDecimal,
    Hexadecimal,
    Binary,
}

impl NumberFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Decimal => "Decimal",
            Self::BooleanDecimal => "Dec (0/1)",
            Self::Hexadecimal => "Hex",
            Self::Binary => "Bin",
        }
    }

    fn is_decimal(&self) -> bool {
        *self == Self::Decimal
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub id: String,
    pub field: Field,
    #[serde(default, skip_serializing_if = "NumberFormat::is_decimal")]
    pub format: NumberFormat,
}

#[derive(Debug, Serialize)]
pub struct DisplayValue {
    pub id: String,
    pub value: String,
    pub unit: String,
}

impl Display {
    pub(super) fn validate_format(&self) -> Result<()> {
        if self.format != NumberFormat::Decimal {
            if self.format == NumberFormat::BooleanDecimal {
                ensure!(
                    self.field.kind == Kind::Bool,
                    "Boolean decimal display requires a Boolean datatype"
                );
            } else {
                ensure!(
                    matches!(self.field.kind, Kind::Signed | Kind::Unsigned),
                    "binary/hex displays require an integer datatype"
                );
            }
            ensure!(
                self.field.scale == 1.0
                    && self.field.offset == 0.0
                    && self.field.unit.is_empty()
                    && self.field.decimals.is_none(),
                "non-default formats require unscaled values with no unit or decimal precision"
            );
        }
        Ok(())
    }

    pub(super) fn decode(&self, payload: &[u8]) -> Result<DisplayValue> {
        self.validate_format()?;
        let value = if self.format == NumberFormat::Decimal {
            self.field.decode(payload)?.value
        } else if self.format == NumberFormat::BooleanDecimal {
            self.field.read_bits(payload)?.to_string()
        } else {
            let width = self.field.bits;
            let display_bits = match width {
                1..=8 => 8,
                9..=16 => 16,
                17..=32 => 32,
                _ => 64,
            };
            let mut bits = self.field.read_bits(payload)?;
            if self.field.kind == Kind::Signed && width < 64 && bits & (1 << (width - 1)) != 0 {
                bits |= u64::MAX << width;
            }
            bits &= u64::MAX >> (64 - display_bits);
            if self.format == NumberFormat::Hexadecimal {
                format!("{bits:0digits$X}h", digits = display_bits / 4)
            } else {
                let digits = format!("{bits:0digits$b}", digits = display_bits);
                format!(
                    "{}b",
                    digits
                        .as_bytes()
                        .chunks(4)
                        .map(|nibble| std::str::from_utf8(nibble).expect("binary digits"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            }
        };
        Ok(DisplayValue {
            id: self.id.clone(),
            value,
            unit: self.field.unit.clone(),
        })
    }
}

pub(super) fn apply(
    function: Node<'_, '_>,
    parameters: &mut [Parameter],
    units: &crate::iodd::resources::Units,
) {
    let Some(menus) = child(function, "UserInterface").and_then(|ui| child(ui, "MenuCollection"))
    else {
        return;
    };
    for menu in menus
        .children()
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "Menu")))
    {
        let menu_id = menu.attribute("id").unwrap_or("unnamed-menu");
        for (ordinal, reference) in menu
            .children()
            .filter(|node| {
                node.has_tag_name((IODD_NAMESPACE, "VariableRef"))
                    || node.has_tag_name((IODD_NAMESPACE, "RecordItemRef"))
            })
            .enumerate()
        {
            let Some(parameter) = parameters
                .iter_mut()
                .find(|p| Some(p.id.as_str()) == reference.attribute("variableId"))
            else {
                continue;
            };
            let parameter = if reference.has_tag_name((IODD_NAMESPACE, "RecordItemRef")) {
                let Some(subindex) = reference
                    .attribute("subindex")
                    .and_then(|value| value.parse::<u8>().ok())
                else {
                    parameter
                        .needs_input
                        .push("Menu record reference has no valid subindex".into());
                    continue;
                };
                let Some(item) = parameter.record.as_mut().and_then(|record| {
                    record
                        .items
                        .iter_mut()
                        .find(|item| item.subindex == subindex)
                }) else {
                    parameter.needs_input.push(format!(
                        "Menu reference to unresolved record subindex {subindex}"
                    ));
                    continue;
                };
                &mut item.parameter
            } else if parameter.array.is_some() {
                parameter
                    .array
                    .as_mut()
                    .expect("checked array")
                    .element
                    .as_mut()
            } else {
                parameter
            };
            if !["gradient", "offset", "unitCode", "displayFormat"]
                .iter()
                .any(|attr| reference.attribute(*attr).is_some())
            {
                continue;
            }
            let id = format!("{menu_id}/{}", ordinal + 1);
            let display = (|| -> Result<Display> {
                let mut field = parameter.scalar_field()?;
                let raw_format = reference.attribute("displayFormat");
                let mut format = match raw_format {
                    Some("Hex") => NumberFormat::Hexadecimal,
                    Some("Bin") => NumberFormat::Binary,
                    None | Some("Dec") => NumberFormat::Decimal,
                    Some(value) if value.starts_with("Dec.") => {
                        field.decimals = Some(value[4..].parse()?);
                        NumberFormat::Decimal
                    }
                    Some(value) => anyhow::bail!("unsupported display format {value}"),
                };
                let scaled = reference.attribute("gradient").is_some()
                    || reference.attribute("offset").is_some();
                let unit = reference.attribute("unitCode");
                if format != NumberFormat::Decimal {
                    ensure!(
                        !scaled && unit.is_none(),
                        "Bin/Hex cannot specify gradient, offset, or unitCode"
                    );
                }
                if field.kind == Kind::Bool {
                    ensure!(
                        raw_format == Some("Dec") && !scaled && unit.is_none(),
                        "Boolean menu displays support plain Dec without gradient, offset, or unitCode"
                    );
                    format = NumberFormat::BooleanDecimal;
                }
                field.scale = reference.attribute("gradient").unwrap_or("1").parse()?;
                ensure!(field.scale != 0.0, "IODD display gradient must not be zero");
                field.offset = reference.attribute("offset").unwrap_or("0").parse()?;
                field.unit = units.resolve(unit);
                field.validate(usize::from(field.bits).div_ceil(8))?;
                let display = Display {
                    id: id.clone(),
                    field,
                    format,
                };
                display.validate_format()?;
                Ok(display)
            })();
            match display {
                Ok(display) => {
                    if display.format == NumberFormat::Decimal
                        && display.field.kind != Kind::Bool
                        && (reference.attribute("unitCode").is_none()
                            || display.field.unit.starts_with("IODD unit "))
                    {
                        parameter
                            .needs_input
                            .push(format!("Display {id} needs a confirmed unit"));
                    }
                    parameter.displays.push(display);
                }
                Err(error) => parameter.needs_input.push(format!(
                    "Display {id} requires manual interpretation: {error}"
                )),
            }
        }
    }
    for parameter in parameters {
        mark_multiple(parameter);
        if let Some(record) = &mut parameter.record {
            for item in &mut record.items {
                mark_multiple(&mut item.parameter);
            }
        }
        if let Some(array) = &mut parameter.array {
            mark_multiple(&mut array.element);
        }
    }
}

fn mark_multiple(parameter: &mut Parameter) {
    if parameter.displays.len() > 1 {
        parameter.needs_input.push(
            "Multiple menu displays are retained separately; confirm which display applies".into(),
        );
    }
}

#[cfg(test)]
mod tests;
