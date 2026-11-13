use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use iolink_workbench::{
    archive,
    capture::{self, Options, Stop},
    decode, iodd,
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
        #[arg(long)]
        entry: Option<String>,
        /// Local IODD-StandardDefinitions1.1.xml supplied by the user.
        #[arg(long)]
        standard_definitions: Option<PathBuf>,
    },
    /// Save an offline simulator profile from an IODD variant and input layout.
    Import {
        path: PathBuf,
        profile: PathBuf,
        #[arg(long)]
        entry: Option<String>,
        #[arg(long)]
        standard_definitions: Option<PathBuf>,
        #[arg(long)]
        variant: Option<String>,
        #[arg(long)]
        layout: Option<String>,
    },
    /// Check a profile and list its effective fields.
    Validate { profile: PathBuf },
    /// Decode a hexadecimal process input without a gateway.
    Decode { profile: PathBuf, payload: String },
    /// List imported IODD parameters without contacting a gateway.
    Parameters { profile: PathBuf },
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

fn main() -> Result<()> {
    match Cli::parse().command {
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
        Command::Iodd {
            path,
            entry,
            standard_definitions,
        } => print!(
            "{}",
            toml::to_string_pretty(&iodd::load_with_definitions(
                &path,
                entry.as_deref(),
                standard_definitions.as_deref()
            )?)?
        ),
        Command::Import {
            path,
            profile,
            entry,
            standard_definitions,
            variant,
            layout,
        } => {
            let catalog = iodd::load_with_definitions(
                &path,
                entry.as_deref(),
                standard_definitions.as_deref(),
            )?;
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
