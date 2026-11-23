use super::{Field, Kind};
use anyhow::{Result, ensure};
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

    pub(crate) fn is_decimal(&self) -> bool {
        *self == Self::Decimal
    }

    pub(crate) fn validate_for(self, field: &Field) -> Result<()> {
        match self {
            Self::Decimal => return Ok(()),
            Self::BooleanDecimal => ensure!(
                field.kind == Kind::Bool,
                "Boolean decimal display requires a Boolean datatype"
            ),
            Self::Hexadecimal | Self::Binary => ensure!(
                matches!(field.kind, Kind::Signed | Kind::Unsigned),
                "binary/hex displays require an integer datatype"
            ),
        }
        ensure!(
            field.scale == 1.0
                && field.offset == 0.0
                && field.unit.is_empty()
                && field.decimals.is_none(),
            "non-default formats require unscaled values with no unit or decimal precision"
        );
        Ok(())
    }

    pub(crate) fn render(self, field: &Field, mut bits: u64) -> Option<String> {
        match self {
            Self::Decimal => return None,
            Self::BooleanDecimal => return Some(u8::from(bits != 0).to_string()),
            Self::Hexadecimal | Self::Binary => {}
        }
        let width = field.bits;
        let display_bits = integer_container_bytes(width) * 8;
        if field.kind == Kind::Signed && width < 64 && bits & (1 << (width - 1)) != 0 {
            bits |= u64::MAX << width;
        }
        bits &= u64::MAX >> (64 - display_bits);
        if self == Self::Hexadecimal {
            return Some(format!("{bits:0digits$X}h", digits = display_bits / 4));
        }
        let digits = format!("{bits:0digits$b}", digits = display_bits);
        let mut grouped = String::with_capacity(digits.len() + display_bits / 4);
        for start in (0..digits.len()).step_by(4) {
            if start != 0 {
                grouped.push(' ');
            }
            grouped.push_str(&digits[start..start + 4]);
        }
        grouped.push('b');
        Some(grouped)
    }
}

pub(crate) fn integer_container_bytes(bits: u16) -> usize {
    usize::from(bits).div_ceil(8).next_power_of_two()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_width_radix_and_exact_csv() -> Result<()> {
        let field = Field {
            name: "Counter".into(),
            bits: 64,
            display_format: NumberFormat::Hexadecimal,
            ..Field::default()
        };
        let bytes = [0xFF; 8];
        let value = field.decode(&bytes)?;
        assert_eq!(value.display(), "FFFFFFFFFFFFFFFFh");
        assert_eq!(value.raw, "18446744073709551615");
        assert_eq!(field.csv_value(&bytes)?, value.raw);
        Ok(())
    }

    #[test]
    fn standalone_boolean_decimal_uses_truth_and_keeps_raw_bits() -> Result<()> {
        let field = Field {
            name: "Switch".into(),
            kind: Kind::Bool,
            display_format: NumberFormat::BooleanDecimal,
            ..Field::default()
        };
        for (byte, display, raw) in [
            (0x00, "0", "false"),
            (0x02, "1", "true"),
            (0xFF, "1", "true"),
        ] {
            let value = field.decode(&[byte])?;
            assert_eq!(value.display(), display);
            assert_eq!(value.bits_hex, format!("{byte:X}"));
            assert_eq!(field.csv_value(&[byte])?, raw);
        }
        Ok(())
    }
}
