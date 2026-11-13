//! Offline expected identity pins. No discovery, parameter read, or device change.
use super::{Identity, Profile};
use anyhow::{Context, Result, ensure};

/// Optional editable text retains unset versus literal empty string pins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IdentityDraft {
    pub vendor_id: Option<String>,
    pub device_id: Option<String>,
    pub product_id: Option<String>,
    pub product_name: Option<String>,
    pub serial: Option<String>,
}

impl From<&Identity> for IdentityDraft {
    fn from(identity: &Identity) -> Self {
        Self {
            vendor_id: identity.vendor_id.map(|value| value.to_string()),
            device_id: identity.device_id.map(|value| value.to_string()),
            product_id: identity.product_id.clone(),
            product_name: identity.product_name.clone(),
            serial: identity.serial.clone(),
        }
    }
}

fn number(text: &str, maximum: u64, name: &str) -> Result<u64> {
    let text = text.trim();
    let (digits, radix) = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .map_or((text, 10), |digits| (digits, 16));
    ensure!(
        !digits.is_empty()
            && digits.bytes().all(|byte| if radix == 16 {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            }),
        "{name} requires unsigned decimal digits or a 0x-prefixed hexadecimal value"
    );
    let value =
        u64::from_str_radix(digits, radix).with_context(|| format!("{name} is too large"))?;
    ensure!(
        value <= maximum,
        "{name} exceeds its supported identity width"
    );
    Ok(value)
}

impl IdentityDraft {
    pub fn parse(&self) -> Result<Identity> {
        Ok(Identity {
            vendor_id: self
                .vendor_id
                .as_deref()
                .map(|text| {
                    number(text, u64::from(u16::MAX), "Vendor ID").map(|value| value as u16)
                })
                .transpose()?,
            device_id: self
                .device_id
                .as_deref()
                .map(|text| number(text, 0xFF_FFFF, "Device ID").map(|value| value as u32))
                .transpose()?,
            product_id: self.product_id.clone(),
            product_name: self.product_name.clone(),
            serial: self.serial.clone(),
        })
    }
}

impl Profile {
    /// Validate a candidate, preserving all other settings and the original profile.
    pub fn with_identity(&self, identity: Identity) -> Result<Self> {
        let mut candidate = self.clone();
        candidate.expected_identity = identity;
        candidate.validate()?;
        Ok(candidate)
    }

    /// Mapping consistency only; this does not compare with any connected device.
    pub fn identity_variant_note(&self) -> Option<&'static str> {
        let variant = &self.iodd.as_ref()?.variant;
        let expected = self.expected_identity.product_id.as_ref()?;
        (variant != expected).then_some("Expected Product ID differs from the selected IODD variant; review the selected variant and mapping before acquisition")
    }
}

#[cfg(test)]
mod tests;
