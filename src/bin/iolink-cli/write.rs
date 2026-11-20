use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand};
use iolink_workbench::{
    profile::Profile,
    transport::write::{CurrentRead, Operation, Outcome, Plan, ReadPoint},
};
use std::path::PathBuf;

#[derive(Args)]
pub(super) struct Arguments {
    profile: PathBuf,
    /// Exact master serial as shown by status or the vendor tool.
    #[arg(long)]
    master_serial: String,
    /// Confirmation printed by the offline preview of this exact operation.
    #[arg(long)]
    confirm: Option<String>,
    #[command(subcommand)]
    operation: WriteOperation,
}

#[derive(Args)]
pub(super) struct ReadArguments {
    profile: PathBuf,
    /// Exact master serial as shown by status or the vendor tool.
    #[arg(long)]
    master_serial: String,
    #[command(subcommand)]
    point: CurrentPoint,
}

#[derive(Subcommand)]
enum CurrentPoint {
    /// Read the current numeric operating mode.
    PortMode,
    /// Read the current raw value of a prospective ISDU write target.
    Isdu {
        #[arg(long)]
        index: u16,
        #[arg(long, default_value_t = 0)]
        subindex: u8,
    },
    /// Read the complete process output image reported by the gateway.
    ProcessOutput,
}

pub(super) fn read_current(args: ReadArguments) -> Result<()> {
    let point = match args.point {
        CurrentPoint::PortMode => ReadPoint::PortMode,
        CurrentPoint::Isdu { index, subindex } => ReadPoint::Isdu { index, subindex },
        CurrentPoint::ProcessOutput => ReadPoint::ProcessOutput,
    };
    let profile = Profile::load(&args.profile)?;
    let reading = CurrentRead::new(&profile, &args.master_serial, point)?.read()?;
    println!("{}", serde_json::to_string_pretty(&reading)?);
    Ok(())
}

#[derive(Subcommand)]
enum WriteOperation {
    /// Change a port's documented numeric operating mode.
    PortMode {
        #[arg(long)]
        expected: u8,
        #[arg(long)]
        desired: u8,
    },
    /// Write one readable ISDU parameter using reviewed raw or IODD-typed input.
    Isdu {
        #[arg(long)]
        index: u16,
        #[arg(long, default_value_t = 0)]
        subindex: u8,
        /// Exact current bytes in hexadecimal.
        #[arg(long)]
        expected: String,
        /// Proposed raw bytes in hexadecimal.
        #[arg(
            long,
            required_unless_present = "desired_value",
            conflicts_with = "desired_value"
        )]
        desired: Option<String>,
        /// Proposed unscaled IODD value, including UTC time and signed duration where supported.
        #[arg(long, allow_negative_numbers = true)]
        desired_value: Option<String>,
    },
    /// Send a single documented IO-Link SystemCommand byte.
    DeviceCommand {
        /// Command code in decimal.
        #[arg(long)]
        code: u8,
        #[arg(long)]
        meaning: String,
    },
    /// Write a complete IO-Link process output image as hex bytes.
    ProcessOutput {
        #[arg(long)]
        expected: String,
        #[arg(long)]
        desired: String,
    },
}

impl WriteOperation {
    fn into_operation(self, profile: &Profile) -> Result<Operation> {
        Ok(match self {
            Self::PortMode { expected, desired } => Operation::PortMode { expected, desired },
            Self::Isdu {
                index,
                subindex,
                expected,
                desired,
                desired_value,
            } => {
                if let Some(value) = desired_value {
                    Operation::isdu_typed(profile, index, subindex, expected, &value)?
                } else {
                    Operation::Isdu {
                        index,
                        subindex,
                        expected,
                        desired: desired.context("specify --desired or --desired-value")?,
                    }
                }
            }
            Self::DeviceCommand { code, meaning } => Operation::DeviceCommand { code, meaning },
            Self::ProcessOutput { expected, desired } => {
                Operation::ProcessOutput { expected, desired }
            }
        })
    }
}

pub(super) fn run(args: Arguments) -> Result<()> {
    let profile = Profile::load(&args.profile)?;
    let plan = Plan::new(
        &profile,
        &args.master_serial,
        args.operation.into_operation(&profile)?,
    )?;
    if let Some(confirmation) = args.confirm {
        let receipt = plan.apply(&confirmation)?;
        println!("{}", serde_json::to_string_pretty(&receipt)?);
        ensure!(
            receipt.outcome != Outcome::Unconfirmed,
            "write outcome unconfirmed. Inspect the target's current state before taking another action"
        );
    } else {
        println!("{}", serde_json::to_string_pretty(&plan.preview())?);
    }
    Ok(())
}
