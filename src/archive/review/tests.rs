use crate::{
    archive::{review, verify},
    capture::{CSV_METADATA, csv_header},
    decode::{self, Field, Kind},
    profile::Profile,
    transport::Validity,
};
use anyhow::Result;
use std::{fs, path::Path};

struct Sample {
    at: u64,
    generation: u64,
    validity: Validity,
    stale: bool,
    payload: Vec<u8>,
}

fn saved(fields: Vec<Field>, width: usize, samples: &[Sample]) -> Result<tempfile::TempDir> {
    let scratch = tempfile::tempdir()?;
    let profile = Profile {
        name: "Saved review fixture".into(),
        input_bytes: width,
        fields,
        ..Default::default()
    };
    profile.save(&scratch.path().join("profile.toml"))?;
    let parts = samples.len().max(1).div_ceil(3);
    let elapsed = samples.last().map_or(0, |sample| sample.at);
    let reconnects = samples.last().map_or(0, |sample| sample.generation - 1);
    fs::write(
        scratch.path().join("capture.toml"),
        format!(
            r#"format_version = 1
profile = "Saved review fixture"
utc_start = "2026-11-12T12:00:00.000Z"
elapsed_us = {elapsed}
csv_parts = {parts}
csv_rows_flushed = {rows}
row_kind = "host_poll"
source_update_loss = "unknown"
stale_threshold_us = 5000
simulation_faults = false
simulated_writer_delay_ms = 0
[counts]
selected = {rows}
logger_overruns = 0
reconnects = {reconnects}
[last_status]
freshness = "snapshots"
route = "synthetic saved fixture"
port_cycle_us = 1000
port_cycle_raw = "1000"
[host_receive_spacing]
count = 7
p50_us = 123
p95_us = 456
p99_us = 789
longest_us = 900
mean_hz = 8123.0
"#,
            rows = samples.len()
        ),
    )?;
    for part in 0..parts {
        let mut writer =
            csv::Writer::from_path(scratch.path().join(format!("capture-{:04}.csv", part + 1)))?;
        writer.write_record(csv_header(&profile))?;
        for (index, sample) in samples.iter().enumerate().skip(part * 3).take(3) {
            let mut row = vec![
                "2026-11-12T12:00:00.000Z".into(),
                sample.at.to_string(),
                (index + 1).to_string(),
                sample.generation.to_string(),
                "host_poll".into(),
                sample.validity.label().into(),
                sample.stale.to_string(),
                String::new(),
                String::new(),
                String::new(),
                String::new(),
                decode::hex(&sample.payload),
            ];
            for field in profile.log_fields() {
                row.push(if sample.validity == Validity::Valid && !sample.stale {
                    field.decode(&sample.payload)?.value
                } else {
                    String::new()
                });
            }
            writer.write_record(row)?;
        }
        writer.flush()?;
    }
    Ok(scratch)
}

fn samples(payloads: &[Vec<u8>]) -> Vec<Sample> {
    payloads
        .iter()
        .enumerate()
        .map(|(index, payload)| Sample {
            at: index as u64 * 100,
            generation: 1,
            validity: Validity::Valid,
            stale: false,
            payload: payload.clone(),
        })
        .collect()
}

fn floats() -> Result<tempfile::TempDir> {
    let mut samples = samples(
        &[-2f32, 10., f32::NAN, 7., 8., 4., f32::INFINITY, 6.]
            .iter()
            .map(|value| value.to_be_bytes().to_vec())
            .collect::<Vec<_>>(),
    );
    samples[3].validity = Validity::Invalid;
    samples[4].stale = true;
    samples[5].validity = Validity::Unknown;
    samples[6].generation = 2;
    samples[7].generation = 2;
    saved(
        vec![Field {
            name: "Level, \"µm\"".into(),
            kind: Kind::Float32,
            bits: 32,
            unit: "µm".into(),
            ..Default::default()
        }],
        4,
        &samples,
    )
}

#[test]
fn review_streams_rotations_and_separates_blank_and_non_finite_cells() -> Result<()> {
    let saved = floats()?;
    let review = review(saved.path(), None, 1)?;
    assert_eq!(review.capture.rows_checked, 8);
    assert_eq!(review.capture.csv_parts, 3);
    assert_eq!(review.capture.generations_in_csv, 2);
    assert_eq!(review.capture.recorded_source_update_loss, "unknown");
    let signal = &review.signal;
    assert_eq!(
        (
            signal.numeric.finite_values,
            signal.numeric.blank_values,
            signal.numeric.non_finite_values
        ),
        (3, 3, 2)
    );
    assert_eq!(
        (signal.numeric.minimum, signal.numeric.maximum),
        (Some(-2.), Some(10.))
    );
    assert!((signal.numeric.mean.unwrap() - 14. / 3.).abs() < 1e-12);
    assert_eq!(signal.numeric.first_csv_value.as_deref(), Some("-2"));
    assert_eq!(signal.numeric.last_csv_value.as_deref(), Some("6"));
    assert_eq!(signal.buckets[0].minimum, Some(-2.));
    assert_eq!(signal.buckets[0].maximum, Some(10.));
    assert_eq!(signal.buckets[0].last, Some(6.));
    assert_eq!(signal.buckets[0].last_generation, Some(2));
    assert_eq!(
        signal
            .recorded_timing
            .host_receive_spacing
            .as_ref()
            .unwrap()
            .p50_us,
        123
    );
    assert!(signal.recorded_timing.gateway_update_spacing.is_none());
    assert!(toml::to_string_pretty(&review)?.contains("non_finite_values = 2"));
    Ok(())
}

#[test]
fn bounded_bins_cover_capture_end_and_leave_empty_time_bins() -> Result<()> {
    let field = Field {
        name: "level".into(),
        ..Default::default()
    };
    let mut samples = samples(&[vec![1], vec![200]]);
    samples[1].at = i64::MAX as u64;
    let saved = saved(vec![field], 1, &samples)?;
    let review = review(saved.path(), Some("level"), 4096)?;
    let buckets = &review.signal.buckets;
    assert_eq!(buckets.len(), 4096);
    assert_eq!(buckets[0].finite_values, 1);
    assert_eq!(buckets[4095].finite_values, 1);
    assert_eq!(buckets[4095].last_host_us, Some(i64::MAX as u64));
    assert_eq!(
        buckets.iter().filter(|bucket| bucket.rows == 0).count(),
        4094
    );
    assert!(review.signal.numeric.mean.unwrap().is_finite());
    Ok(())
}

#[test]
fn boolean_values_and_exact_wide_csv_text_are_preserved() -> Result<()> {
    let saved_bool = saved(
        vec![Field {
            name: "switch".into(),
            bits: 1,
            kind: Kind::Bool,
            ..Default::default()
        }],
        1,
        &samples(&[vec![0], vec![1], vec![1]]),
    )?;
    let boolean = review(saved_bool.path(), None, 2)?.signal.numeric;
    assert_eq!(boolean.first_csv_value.as_deref(), Some("false"));
    assert_eq!(boolean.last_csv_value.as_deref(), Some("true"));
    assert_eq!(boolean.minimum, Some(0.));
    assert!((boolean.mean.unwrap() - 2. / 3.).abs() < 1e-12);
    let saved_wide = saved(
        vec![Field {
            name: "counter".into(),
            bits: 64,
            ..Default::default()
        }],
        8,
        &samples(&[
            (u64::MAX - 1).to_be_bytes().to_vec(),
            u64::MAX.to_be_bytes().to_vec(),
        ]),
    )?;
    let wide = review(saved_wide.path(), None, 1)?.signal;
    assert_eq!(
        wide.numeric.first_csv_value,
        Some((u64::MAX - 1).to_string())
    );
    assert_eq!(wide.numeric.last_csv_value, Some(u64::MAX.to_string()));
    assert!(wide.numeric_basis.contains("approximate f64"));
    Ok(())
}

#[test]
fn opposite_large_finite_values_do_not_overflow_the_mean() -> Result<()> {
    let saved = saved(
        vec![Field {
            name: "large".into(),
            kind: Kind::Signed,
            scale: 8e307,
            ..Default::default()
        }],
        1,
        &samples(&[vec![255], vec![1], vec![2], vec![254]]),
    )?;
    let numeric = review(saved.path(), None, 1)?.signal.numeric;
    assert_eq!(numeric.finite_values, 4);
    assert_eq!(numeric.non_finite_values, 0);
    assert_eq!(numeric.minimum, Some(-1.6e308));
    assert_eq!(numeric.maximum, Some(1.6e308));
    assert!(numeric.mean.unwrap().abs() < 1e293);
    Ok(())
}

#[test]
fn no_finite_values_and_empty_capture_have_no_fabricated_statistics() -> Result<()> {
    let saved_nan = saved(
        vec![Field {
            name: "float".into(),
            kind: Kind::Float32,
            bits: 32,
            ..Default::default()
        }],
        4,
        &samples(&[f32::NAN.to_be_bytes().to_vec()]),
    )?;
    let nan = review(saved_nan.path(), None, 128)?.signal;
    assert_eq!(nan.numeric.non_finite_values, 1);
    assert_eq!(nan.numeric.mean, None);
    assert_eq!(nan.numeric.minimum, None);
    let empty = saved(
        vec![Field {
            name: "empty".into(),
            ..Default::default()
        }],
        1,
        &[],
    )?;
    let empty = review(empty.path(), None, 128)?;
    assert_eq!(empty.capture.rows_checked, 0);
    assert!(
        empty
            .signal
            .buckets
            .iter()
            .all(|bucket| bucket.minimum.is_none() && bucket.rows == 0)
    );
    Ok(())
}

#[test]
fn unsupported_field_and_limits_fail_while_raw_only_capture_still_verifies() -> Result<()> {
    let saved = floats()?;
    assert!(review(saved.path(), Some("missing"), 128).is_err());
    for count in [0, 4097, usize::MAX] {
        assert!(review(saved.path(), None, count).is_err());
    }
    let raw = saved_raw()?;
    assert!(verify(raw.path(), None).is_ok());
    assert!(review(raw.path(), None, 128).is_err());
    Ok(())
}

fn saved_raw() -> Result<tempfile::TempDir> {
    saved(Vec::new(), 1, &samples(&[vec![1]]))
}

#[test]
fn an_unselected_corrupted_field_prevents_any_review_result() -> Result<()> {
    let fields = vec![
        Field {
            name: "first".into(),
            ..Default::default()
        },
        Field {
            name: "other".into(),
            bits: 1,
            kind: Kind::Bool,
            ..Default::default()
        },
    ];
    let saved = saved(fields, 1, &samples(&[vec![1], vec![2], vec![3], vec![4]]))?;
    let part = saved.path().join("capture-0002.csv");
    corrupt(&part)?;
    let error = review(saved.path(), Some("first"), 1).unwrap_err();
    assert!(format!("{error:#}").contains("typed value for other"));
    Ok(())
}

fn corrupt(path: &Path) -> Result<()> {
    let mut reader = csv::Reader::from_path(path)?;
    let header = reader.headers()?.clone();
    let mut cells = reader
        .records()
        .next()
        .unwrap()?
        .iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    drop(reader);
    cells[CSV_METADATA.len() + 1] = "corrupt".into();
    let mut writer = csv::Writer::from_path(path)?;
    writer.write_record(header.iter())?;
    writer.write_record(cells)?;
    writer.flush()?;
    Ok(())
}
