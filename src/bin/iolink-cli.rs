use anyhow::Result;
use clap::{Parser, Subcommand};
use iolink_workbench::{decode, iodd, profile::Profile};
use std::path::PathBuf;

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
    },
    /// Save an offline simulator profile from an IODD variant and input layout.
    Import {
        path: PathBuf,
        profile: PathBuf,
        #[arg(long)]
        entry: Option<String>,
        #[arg(long)]
        variant: Option<String>,
        #[arg(long)]
        layout: Option<String>,
    },
    /// Check a profile and list its effective fields.
    Validate { profile: PathBuf },
    /// Decode a hexadecimal process input without a gateway.
    Decode { profile: PathBuf, payload: String },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Iodd { path, entry } => print!(
            "{}",
            toml::to_string_pretty(&iodd::load(&path, entry.as_deref())?)?
        ),
        Command::Import {
            path,
            profile,
            entry,
            variant,
            layout,
        } => {
            let catalog = iodd::load(&path, entry.as_deref())?;
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
