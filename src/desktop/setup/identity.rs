use eframe::egui::{self, Ui};
use iolink_workbench::profile::{Identity, IdentityDraft, Profile};

pub struct Form {
    baseline: IdentityDraft,
    draft: IdentityDraft,
}

impl Form {
    pub fn new(identity: &Identity) -> Self {
        let draft = IdentityDraft::from(identity);
        Self {
            baseline: draft.clone(),
            draft,
        }
    }

    pub fn sync(&mut self, identity: &Identity) {
        *self = Self::new(identity);
    }

    pub fn pending(&self) -> bool {
        self.draft != self.baseline
    }

    fn candidate(&self, profile: &Profile) -> anyhow::Result<Profile> {
        profile.with_identity(self.draft.parse()?)
    }

    pub fn show(&mut self, ui: &mut Ui, profile: &mut Profile, message: &mut String) -> bool {
        ui.heading("Expected device identity");
        ui.label("Check the fields to pin. Numeric IDs accept unsigned decimal or 0x hexadecimal; text pins compare exactly, including spaces.");
        ui.small("These are configured expectations, not observed identity. Applying pins updates the host profile only. A field unavailable from the gateway remains unverified; Product ID can be checked with the explicit Verify Product ID read.");
        egui::Grid::new("expected-device-identity")
            .num_columns(3)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                entry(ui, "Vendor ID", &mut self.draft.vendor_id, "16-bit ID");
                entry(ui, "Device ID", &mut self.draft.device_id, "24-bit ID");
                entry(
                    ui,
                    "Product ID",
                    &mut self.draft.product_id,
                    "Exact Product ID",
                );
                entry(
                    ui,
                    "Product name",
                    &mut self.draft.product_name,
                    "Exact product name",
                );
                entry(ui, "Serial", &mut self.draft.serial, "Exact serial");
            });
        if let Some(import) = &profile.iodd {
            ui.small(format!(
                "Selected IODD product variant · {}",
                import.variant
            ));
            if self
                .draft
                .product_id
                .as_ref()
                .is_some_and(|value| value != &import.variant)
            {
                ui.colored_label(super::WARNING, "Expected Product ID differs from the selected IODD variant; review the variant and mapping before acquisition");
            }
        }
        if [
            &self.draft.product_id,
            &self.draft.product_name,
            &self.draft.serial,
        ]
        .iter()
        .any(|value| value.as_ref().is_some_and(String::is_empty))
        {
            ui.small(
                "A checked empty text field pins the empty string. Uncheck a field to ignore it.",
            );
        }
        let mut applied = false;
        ui.horizontal(|ui| {
            if ui.add_enabled(self.pending(), egui::Button::new("Apply identity pins")).clicked() {
                match self.candidate(profile) {
                    Ok(candidate) => {
                        *profile = candidate;
                        self.sync(&profile.expected_identity);
                        *message = "Expected identity pins applied locally; no device identity was read or verified".into();
                        applied = true;
                    }
                    Err(error) => *message = format!("{error:#}"),
                }
            }
            if ui.add_enabled(self.pending(), egui::Button::new("Discard identity pins")).clicked() {
                self.draft = self.baseline.clone();
                message.clear();
            }
            if self.pending() { ui.colored_label(super::WARNING, "Pending identity pins"); }
        });
        applied
    }
}

fn entry(ui: &mut Ui, name: &str, value: &mut Option<String>, hint: &str) {
    ui.label(name);
    ui.push_id(name, |ui| {
        let mut enabled = value.is_some();
        if ui.checkbox(&mut enabled, "Pin").changed() {
            *value = enabled.then(String::new);
        }
    });
    if let Some(text) = value {
        ui.add(
            egui::TextEdit::singleline(text)
                .id(egui::Id::new(("expected-identity-pin", name)))
                .hint_text(hint),
        );
    } else {
        ui.weak("Not pinned");
    }
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_numeric_pins_stay_pending_until_corrected_or_discarded() {
        let profile = Profile::default();
        let mut form = Form::new(&profile.expected_identity);
        form.draft.vendor_id = Some(String::new());
        assert!(form.pending() && form.candidate(&profile).is_err());
        assert_eq!(profile.expected_identity, Identity::default());
        form.draft.vendor_id = Some("0xFFFF".into());
        let candidate = form.candidate(&profile).unwrap();
        assert_eq!(candidate.expected_identity.vendor_id, Some(65535));
        form.sync(&candidate.expected_identity);
        assert!(!form.pending());
        form.draft.vendor_id = None;
        assert!(form.pending());
        form.sync(&candidate.expected_identity);
        assert!(!form.pending());
    }

    #[test]
    fn shared_pending_guard_covers_identity_drafts_and_literal_empty_pins() {
        let profile = Profile::default();
        let mut editor = super::super::Editor::new(&profile);
        editor.identity.draft.serial = Some(String::new());
        assert!(editor.pending());
        let candidate = editor.identity.candidate(&profile).unwrap();
        assert_eq!(candidate.expected_identity.serial.as_deref(), Some(""));
        editor.sync(&candidate);
        assert!(!editor.pending());
        editor.identity.draft.serial = None;
        assert!(editor.pending());
        editor.sync(&profile);
        assert!(!editor.pending());
    }
}
