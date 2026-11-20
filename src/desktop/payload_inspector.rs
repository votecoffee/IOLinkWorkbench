//! In-memory inspection of input and output bytes or a held Inputs observation.
use super::{WARNING, decode, observation::Observation, payload_view};
use eframe::egui::{self, Ui};
use iolink_workbench::{
    capture::Live,
    iodd::DecodedOutput,
    profile::Profile,
    transport::{Freshness, Input},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(super) enum Page {
    #[default]
    Pasted,
    Observation,
    Output,
}

struct Copied {
    input: Input,
    view_at_copy: Observation,
    stale_at_receipt: Option<bool>,
    freshness: Option<Freshness>,
    generation: u64,
    profile: Profile,
}

pub(super) struct Inspector {
    pub page: Page,
    pasted: String,
    pub output_pasted: String,
    copied: Option<Copied>,
}

impl Default for Inspector {
    fn default() -> Self {
        Self {
            page: Page::Pasted,
            pasted: "FFF9".into(),
            output_pasted: String::new(),
            copied: None,
        }
    }
}

impl Inspector {
    pub fn copy(&mut self, input: Input, view: Observation, live: &Live, profile: &Profile) {
        self.copied = Some(Copied {
            input,
            view_at_copy: view,
            stale_at_receipt: live.stale,
            freshness: live.status.as_ref().map(|status| status.freshness),
            generation: live.generation,
            profile: profile.clone(),
        });
        self.page = Page::Observation;
    }

    pub fn show(&mut self, ui: &mut Ui, applied: &Profile) {
        ui.heading("Offline payload decoder");
        if applied.acyclic_only() {
            if let Some(copied) = &self.copied {
                copied.show(ui);
            } else {
                ui.label("Acyclic IODD · inspect Parameter bytes or Event codes.");
            }
            return;
        }
        if !applied.has_process_input() && self.page == Page::Pasted {
            self.page = Page::Output;
        }
        ui.horizontal_wrapped(|ui| {
            if applied.has_process_input() {
                ui.selectable_value(&mut self.page, Page::Pasted, "Input bytes");
            }
            ui.selectable_value(&mut self.page, Page::Output, "Output bytes");
            if self.copied.is_some() {
                ui.selectable_value(&mut self.page, Page::Observation, "From Inputs");
            }
        });
        ui.separator();
        if self.page == Page::Output {
            self.show_output(ui, applied);
            return;
        }
        if self.page == Page::Observation
            && let Some(copied) = &self.copied
        {
            copied.show(ui);
            return;
        }
        ui.label(format!(
            "{} · {} expected bytes",
            applied.name, applied.input_bytes
        ));
        ui.add(
            egui::TextEdit::multiline(&mut self.pasted)
                .code_editor()
                .desired_rows(2)
                .desired_width(f32::INFINITY),
        );
        match decode::parse_hex(&self.pasted) {
            Ok(payload) => payload_view(ui, applied, &payload, true),
            Err(error) => {
                ui.colored_label(WARNING, error.to_string());
            }
        }
    }

    fn show_output(&mut self, ui: &mut Ui, applied: &Profile) {
        ui.label(format!("Selected mapping · {}", applied.name));
        ui.add(
            egui::TextEdit::multiline(&mut self.output_pasted)
                .code_editor()
                .desired_rows(2)
                .desired_width(f32::INFINITY)
                .hint_text("Paste the full output image in hex"),
        );
        if self.output_pasted.trim().is_empty() {
            return;
        }
        match decode::parse_hex(&self.output_pasted) {
            Ok(payload) => {
                ui.label(format!("Raw output · {}", decode::hex(&payload)));
                if let Some(layout) = applied.output_layout() {
                    show_output_fields(ui, &layout.decode_output(&payload, None));
                } else {
                    ui.small("Import an IODD output layout to see typed fields.");
                }
            }
            Err(error) => {
                ui.colored_label(WARNING, error.to_string());
            }
        }
    }
}

pub(super) fn show_output_fields(ui: &mut Ui, output: &DecodedOutput) {
    ui.label(format!(
        "IODD output · {} · {} bits",
        output.layout_id, output.bit_length
    ));
    if let Some(condition) = &output.condition {
        let basis = match output.recorded_condition_confirmation {
            Some(true) => "Recorded condition confirmation",
            Some(false) => "Unconfirmed layout condition",
            None => "Selected layout condition",
        };
        ui.small(format!("{basis} · {condition}"));
    }
    if output.standalone_boolean {
        ui.small("Standalone Boolean · 00 false · FF true");
    }
    if let Some(warning) = &output.encoding_warning {
        ui.colored_label(WARNING, warning);
    }
    if let Some(error) = &output.decode_error {
        ui.colored_label(WARNING, error);
    }
    if !output.fields.is_empty() || !output.needs_input.is_empty() {
        ui.collapsing("IODD output fields", |ui| {
            for entry in &output.fields {
                let field = &entry.field;
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{} · bit {} · {} bits · {:?} · {:?}",
                        field.name, field.bit_offset, field.bits, field.kind, field.byte_order
                    ));
                    let display = format!("{} {}", entry.value.value, field.unit);
                    ui.monospace(format!(
                        "{} (raw {}, bits 0x{})",
                        display.trim_end(),
                        entry.value.raw,
                        entry.value.bits_hex
                    ));
                });
            }
            for note in &output.needs_input {
                ui.colored_label(WARNING, note);
            }
        });
    }
}

impl Copied {
    fn show(&self, ui: &mut Ui) {
        let view = &self.view_at_copy;
        ui.label(format!(
            "Copied mapping · {} · {} expected bytes",
            self.profile.name, self.profile.input_bytes
        ));
        ui.small("Offline values use the mapping and bytes copied from Inputs.");
        ui.label(format!(
            "At copy · connection generation {} · host receive age {} ms",
            self.generation,
            view.receive_age.as_millis()
        ));
        ui.label(format!(
            "Validity at receipt {} · staleness at receipt {}",
            self.input.validity.label(),
            match self.stale_at_receipt {
                Some(true) => "stale",
                Some(false) => "within limit",
                None => "unknown",
            }
        ));
        let state = format!("Inputs at copy · {}", view.state.label());
        if view.show_typed() {
            ui.label(state);
        } else {
            ui.colored_label(WARNING, state);
        }
        match self.freshness {
            Some(Freshness::Snapshots) => {
                ui.small("Host poll · source age and update count unknown");
            }
            Some(Freshness::Counter32 | Freshness::Timestamp) => {
                ui.horizontal_wrapped(|ui| {
                    if let Some(counter) = self.input.counter {
                        ui.small(format!("Source counter {counter}"));
                    }
                    if let Some(stamp) = self.input.source_time_us {
                        ui.small(format!("Source timestamp {stamp} µs"));
                    }
                    if let Some(age) = self.input.source_age_us {
                        ui.small(format!("Gateway input age at receipt {age} µs"));
                    }
                });
            }
            None => {}
        }
        payload_view(ui, &self.profile, self.input.payload.bytes(), true);
    }
}
