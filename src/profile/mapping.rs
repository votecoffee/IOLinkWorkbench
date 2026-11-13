//! Host-only cyclic field edits; imported IODD definitions remain unchanged.
use super::Profile;
use crate::decode::Field;
use anyhow::{Context, Result};

impl Profile {
    pub fn with_field(&self, field: Field) -> Result<Self> {
        field.validate(self.input_bytes)?;
        let mut candidate = self.clone();
        if let Some(existing) = candidate
            .fields
            .iter_mut()
            .find(|item| item.name == field.name)
        {
            *existing = field;
        } else {
            candidate.fields.push(field);
        }
        candidate.validate()?;
        Ok(candidate)
    }

    /// Remove a manual field or reveal its imported IODD definition again.
    pub fn without_manual_field(&self, name: &str) -> Result<Self> {
        let index = self
            .fields
            .iter()
            .position(|field| field.name == name)
            .context("field has no manual mapping or override to remove")?;
        let mut candidate = self.clone();
        candidate.fields.remove(index);
        candidate.validate().context("removing this mapping would invalidate the profile; update the CSV field selection first if needed")?;
        Ok(candidate)
    }
}

#[cfg(test)]
mod tests;
