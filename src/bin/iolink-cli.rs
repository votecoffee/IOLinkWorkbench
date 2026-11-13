use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use iolink_workbench::{
    archive,
    capture::{self, Options, Stop},
    decode, diagnostics, iodd,
    profile::Profile,
    transport,
};
use std::{path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List device variants, process input layouts, and fields needing review.
    Iodd {
        path: PathBuf,
        #[command(flatten)]
        options: IoddOptions,
    },
    /// Save an offline simulator profile from an IODD variant and input layout.
    Import {
        path: PathBuf,
        profile: PathBuf,
        #[command(flatten)]
        options: IoddOptions,
        #[arg(long)]
        variant: Option<String>,
        #[arg(long)]
        layout: Option<String>,
    },
    /// Check a profile and list its effective fields.
    Validate { profile: PathBuf },
    /// Validate selected XML or ZIP member against a supplied local XSD closure.
    CheckIodd {
        path: PathBuf,
        #[arg(long)]
        xsd: PathBuf,
        #[arg(long)]
        entry: Option<String>,
    },
    /// Decode a hexadecimal process input without a gateway.
    Decode { profile: PathBuf, payload: String },
    /// List imported IODD parameters without contacting a gateway.
    Parameters { profile: PathBuf },
    /// List saved IODD event definitions and unresolved standard event references.
    Events { profile: PathBuf },
    /// Explicitly read optional Device Status (36:0) and Detailed Device Status (37:0).
    ReadDiagnostics { profile: PathBuf },
    /// Decode a recorded Detailed Device Status payload without opening a transport.
    DecodeDiagnostics {
        profile: PathBuf,
        payload: String,
        #[arg(long)]
        device_status: Option<String>,
    },
    /// Read one named IODD parameter through the independent acyclic client.
    ReadParameter {
        profile: PathBuf,
        id: String,
        #[arg(long, default_value_t = 0)]
        subindex: u8,
    },
    /// Decode a recorded ISDU payload offline, using the saved IODD catalog.
    DecodeParameter {
        profile: PathBuf,
        id: String,
        payload: String,
        #[arg(long, default_value_t = 0)]
        subindex: u8,
    },
    /// Verify a finalized capture offline and optionally join its rotated CSV parts.
    VerifyCapture {
        directory: PathBuf,
        #[arg(long)]
        joined_csv: Option<PathBuf>,
    },
    /// Read master, firmware, port, and connected identity.
    Status {
        profile: PathBuf,
        #[arg(long)]
        verify_product_id: bool,
    },
    /// Read one explicit acyclic ISDU parameter using a separate client.
    Isdu {
        profile: PathBuf,
        index: u16,
        #[arg(default_value_t = 0)]
        subindex: u8,
    },
    /// Capture a single port to a new directory and write a timing report.
    Capture {
        profile: PathBuf,
        output: PathBuf,
        #[arg(long, default_value_t = 10.0)]
        seconds: f64,
        #[arg(long)]
        simulate_faults: bool,
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=100))]
        simulate_writer_delay_ms: u64,
        #[arg(long)]
        queue_capacity: Option<usize>,
    },
}

#[derive(clap::Args)]
struct IoddOptions {
    #[arg(long)]
    entry: Option<String>,
    #[arg(long)]
    standard_definitions: Option<PathBuf>,
    /// Require a main-XML XSD validation pass against a local root schema.
    #[arg(long)]
    xsd: Option<PathBuf>,
    /// Select an embedded or companion language by its xml:lang tag.
    #[arg(long)]
    language: Option<String>,
    /// Use an explicit ExternalTextDocument XML instead of an automatic companion.
    #[arg(long, requires = "language")]
    language_file: Option<PathBuf>,
    /// Load abbreviations from a local IODDStandardUnitDefinitions XML.
    #[arg(long)]
    unit_definitions: Option<PathBuf>,
}
impl IoddOptions {
    fn load(&self) -> iodd::LoadOptions<'_> {
        iodd::LoadOptions {
            entry: self.entry.as_deref(),
            definitions: self.standard_definitions.as_deref(),
            xsd: self.xsd.as_deref(),
            language: self.language.as_deref(),
            language_file: self.language_file.as_deref(),
            unit_definitions: self.unit_definitions.as_deref(),
        }
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Events { profile } => {
            let profile = Profile::load(&profile)?;
            let imported = profile
                .iodd
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("import an IODD first"))?;
            #[derive(serde::Serialize)]
            struct Events<'a> {
                events: &'a [iodd::EventDefinition],
                standard_event_refs: &'a [u16],
            }
            print!(
                "{}",
                toml::to_string_pretty(&Events {
                    events: &imported.events,
                    standard_event_refs: &imported.standard_event_refs
                })?
            );
        }
        Command::ReadDiagnostics { profile } => {
            let report = diagnostics::read(&Profile::load(&profile)?)?;
            print!("{}", toml::to_string_pretty(&report)?);
            ensure!(
                report.ok(),
                "diagnostic read is incomplete or observed identity fields changed; retained results are printed above"
            );
        }
        Command::DecodeDiagnostics {
            profile,
            payload,
            device_status,
        } => {
            let profile = Profile::load(&profile)?;
            let detailed =
                diagnostics::decode_detailed(&profile, &decode::parse_hex_bounded(&payload, 232)?);
            let status = device_status
                .as_deref()
                .map(|text| {
                    Ok::<_, anyhow::Error>(diagnostics::decode_device_status(
                        &decode::parse_hex_bounded(text, 232)?,
                    ))
                })
                .transpose()?;
            #[derive(serde::Serialize)]
            struct Decoded {
                device_status: Option<diagnostics::DeviceStatus>,
                detailed_status: diagnostics::DetailedStatus,
            }
            let ok = detailed.decode_error.is_none()
                && status
                    .as_ref()
                    .is_none_or(|status| status.decode_error.is_none());
            print!(
                "{}",
                toml::to_string_pretty(&Decoded {
                    device_status: status,
                    detailed_status: detailed
                })?
            );
            ensure!(
                ok,
                "diagnostic payload needs decoding review; raw bytes are retained above"
            );
        }
        Command::Parameters { profile } => {
            let profile = Profile::load(&profile)?;
            let iodd = profile
                .iodd
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("import an IODD first"))?;
            #[derive(serde::Serialize)]
            struct Catalog<'a> {
                parameters: &'a [iodd::Parameter],
                standard_parameter_refs: &'a [String],
            }
            print!(
                "{}",
                toml::to_string_pretty(&Catalog {
                    parameters: &iodd.parameters,
                    standard_parameter_refs: &iodd.standard_parameter_refs,
                })?
            );
        }
        Command::ReadParameter {
            profile,
            id,
            subindex,
        } => {
            let profile = Profile::load(&profile)?;
            print!(
                "{}",
                toml::to_string_pretty(&transport::read_parameter_at(&profile, &id, subindex)?)?
            );
        }
        Command::DecodeParameter {
            profile,
            id,
            payload,
            subindex,
        } => {
            let profile = Profile::load(&profile)?;
            let parameter = profile.parameter(&id)?.target(subindex)?;
            let bytes = decode::parse_hex_bounded(&payload, 232)?;
            let decoded = parameter.decode(&bytes);
            print!("{}", toml::to_string_pretty(&decoded)?);
            ensure!(
                decoded.decode_error.is_none()
                    && decoded
                        .record_items
                        .iter()
                        .chain(&decoded.array_items)
                        .all(|item| item.value.decode_error.is_none()),
                "parameter payload needs decoding review; raw bytes retained above"
            );
        }
        Command::VerifyCapture {
            directory,
            joined_csv,
        } => {
            let summary = archive::verify(&directory, joined_csv.as_deref())?;
            print!("{}", toml::to_string_pretty(&summary)?);
        }
        Command::CheckIodd { path, xsd, entry } => {
            let report = iodd::check_schema(&path, entry.as_deref(), &xsd)?;
            print!("{}", toml::to_string_pretty(&report)?);
            ensure!(
                report.passed,
                "XSD validation failed; diagnostics are printed above"
            );
        }
        Command::Iodd { path, options } => print!(
            "{}",
            toml::to_string_pretty(&iodd::load_with_options(&path, &options.load())?)?
        ),
        Command::Import {
            path,
            profile,
            options,
            variant,
            layout,
        } => {
            let catalog = iodd::load_with_options(&path, &options.load())?;
            let (expected_identity, imported) =
                catalog.select(variant.as_deref(), layout.as_deref())?;
            let profile_data = Profile {
                name: imported.variant.clone(),
                input_bytes: imported.input_bytes,
                expected_identity,
                iodd: Some(imported),
                ..Profile::default()
            };
            profile_data.save(&profile)?;
            println!("Saved {}", profile.display());
        }
        Command::Validate { profile } => {
            let profile = Profile::load(&profile)?;
            println!("{} ({} bytes)", profile.name, profile.input_bytes);
            if let Some(iodd) = &profile.iodd {
                if let Some(localization) = &iodd.localization {
                    println!("{}", localization.summary());
                }
                println!(
                    "{}",
                    iodd.schema_validation
                        .as_ref()
                        .map(|report| report.summary())
                        .unwrap_or_else(
                            || "Parser checks only; XSD validation not performed".into()
                        )
                );
                for item in &iodd.needs_input {
                    println!("Review: {item}");
                }
            }
            for (field, origin) in profile.effective_fields() {
                println!(
                    "{} [{origin}] bit {} width {} {:?} {:?}, × {} + {} {}",
                    field.name,
                    field.bit_offset,
                    field.bits,
                    field.kind,
                    field.byte_order,
                    field.scale,
                    field.offset,
                    field.unit
                );
            }
        }
        Command::Status {
            profile,
            verify_product_id,
        } => {
            let profile = Profile::load(&profile)?;
            let (mut status, mut check) = capture::probe(&profile)?;
            if verify_product_id {
                status.identity.product_id = Some(transport::verify_product_id(&profile)?);
                check = profile.expected_identity.verify(&status.identity)?;
            }
            println!("identity_check = {check:?}");
            print!("{}", toml::to_string_pretty(&status)?);
        }
        Command::Isdu {
            profile,
            index,
            subindex,
        } => {
            let bytes = transport::read_isdu(&Profile::load(&profile)?, index, subindex)?;
            println!("hex = {:?}", decode::hex(&bytes));
            if let Ok(text) = std::str::from_utf8(&bytes) {
                println!("utf8 = {text:?}");
            }
        }
        Command::Capture {
            profile,
            output,
            seconds,
            simulate_faults,
            simulate_writer_delay_ms,
            queue_capacity,
        } => {
            ensure!(
                seconds.is_finite() && (0.01..=86400.0).contains(&seconds),
                "--seconds must be 0.01–86400"
            );
            let mut profile = Profile::load(&profile)?;
            if let Some(capacity) = queue_capacity {
                profile.logging.queue_capacity = capacity;
            }
            let stop = Stop::default();
            let interrupt = stop.clone();
            ctrlc::set_handler(move || interrupt.request())?;
            let report = capture::run(
                &profile,
                &Options {
                    output: Some(output),
                    duration: Some(Duration::from_secs_f64(seconds)),
                    simulate_faults,
                    simulated_writer_delay: Duration::from_millis(simulate_writer_delay_ms),
                },
                &stop,
                None,
            )?;
            print!("{}", toml::to_string_pretty(&report)?);
            ensure!(
                report.success(),
                "capture ended with a transport or writer error; see the report"
            );
            if report.stopped_by_request {
                std::process::exit(130);
            }
        }
        Command::Decode { profile, payload } => {
            let profile = Profile::load(&profile)?;
            let payload = decode::parse_hex(&payload)?;
            for (field, origin, value) in profile.decode(&payload)? {
                println!(
                    "{} [{origin}]: {} {} (raw {}, bits 0x{})",
                    field.name, value.value, field.unit, value.raw, value.bits_hex
                );
            }
        }
    }
    Ok(())
}
