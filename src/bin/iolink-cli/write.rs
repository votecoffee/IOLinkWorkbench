use anyhow::{Result, ensure};
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
    /// Write one readable ISDU parameter using reviewed raw bytes.
    Isdu {
        #[arg(long)]
        index: u16,
        #[arg(long, default_value_t = 0)]
        subindex: u8,
        #[arg(long)]
        expected: String,
        #[arg(long)]
        desired: String,
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

impl From<WriteOperation> for Operation {
    fn from(value: WriteOperation) -> Self {
        match value {
            WriteOperation::PortMode { expected, desired } => Self::PortMode { expected, desired },
            WriteOperation::Isdu {
                index,
                subindex,
                expected,
                desired,
            } => Self::Isdu {
                index,
                subindex,
                expected,
                desired,
            },
            WriteOperation::DeviceCommand { code, meaning } => {
                Self::DeviceCommand { code, meaning }
            }
            WriteOperation::ProcessOutput { expected, desired } => {
                Self::ProcessOutput { expected, desired }
            }
        }
    }
}

pub(super) fn run(args: Arguments) -> Result<()> {
    let profile = Profile::load(&args.profile)?;
    let plan = Plan::new(&profile, &args.master_serial, args.operation.into())?;
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
