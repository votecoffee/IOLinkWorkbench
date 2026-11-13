//! Pure shared telemetry input representation; no transport or consumer state.
use crate::{
    capture::Counts,
    decode,
    profile::Profile,
    transport::{Freshness, Input, Status},
};
use anyhow::{Result, ensure};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub(crate) struct Sample {
    pub input: Input,
    pub freshness: Freshness,
    pub stale: Option<bool>,
    pub classification: &'static str,
    pub csv_sample_index: Option<u64>,
    pub csv_enqueued: Option<bool>,
    pub counts: Counts,
    pub source_gaps: Option<u64>,
}

pub(crate) fn sample_json(sample: Sample, profile: &Profile) -> Value {
    let typed = (sample.input.validity == crate::transport::Validity::Valid && sample.stale != Some(true)).then(|| profile.decode(sample.input.payload.bytes()).map(|values| values.into_iter().map(|(field,origin,value)| json!({"name":field.name,"origin":origin,"unit":field.unit,"decoded":value})).collect::<Vec<_>>()));
    let (values, error) = match typed {
        Some(Ok(values)) => (Some(values), None),
        Some(Err(error)) => (None, Some(error.to_string())),
        None => (None, None),
    };
    json!({"kind":"input","receive_index":sample.counts.received,"classification":sample.classification,
                "row_kind":sample.freshness.row_kind(),"freshness":sample.freshness,"validity":sample.input.validity,"stale":sample.stale,
                "source_counter":sample.input.counter,"source_time_us":sample.input.source_time_us,"source_age_us":sample.input.source_age_us,
                "source_gaps_observed":sample.source_gaps,"source_update_loss":if sample.freshness==Freshness::Counter32 {"counter gaps within generations; disconnect loss unknown"} else {"unknown"},
                "diagnostic_code":sample.input.diagnostic_code,"raw_hex":decode::hex(sample.input.payload.bytes()),"values":values,"decode_error":error,
                "csv_sample_index":sample.csv_sample_index,"csv_enqueued":sample.csv_enqueued,"capture_counts":sample.counts})
}

pub(crate) fn validate_mapping(profile: &Profile) -> Result<()> {
    profile.validate()?;
    let fields = profile.effective_fields();
    ensure!(
        fields.len() <= 128
            && fields
                .iter()
                .all(|(f, _)| f.name.len() <= 256 && f.unit.len() <= 256),
        "telemetry supports at most 128 mapped fields with names/units up to 256 UTF-8 bytes"
    );
    Ok(())
}

pub(crate) fn bounded_status(status: &Status) -> (Status, bool) {
    let mut status = status.clone();
    let mut truncated = false;
    for text in [
        &mut status.model,
        &mut status.firmware,
        &mut status.port_mode,
    ] {
        truncated |= truncate(text);
    }
    for text in [
        &mut status.master_serial,
        &mut status.device_status,
        &mut status.port_event,
        &mut status.device_event,
        &mut status.port_cycle_raw,
        &mut status.identity.product_id,
        &mut status.identity.product_name,
        &mut status.identity.serial,
    ]
    .into_iter()
    .flatten()
    {
        truncated |= truncate(text);
    }
    (status, truncated)
}

pub(crate) fn truncate(text: &mut String) -> bool {
    let truncated = text.len() > 256;
    let mut end = text.len().min(256);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    truncated
}
