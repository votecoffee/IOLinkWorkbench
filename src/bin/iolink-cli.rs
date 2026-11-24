use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use iolink_workbench::{
    archive,
    capture::{self, Options, Stop},
    decode, diagnostics, iodd, parameter_snapshot,
    profile::{LogSelection, Profile},
    transport,
};
use std::{path::PathBuf, time::Duration};

#[path = "iolink-cli/connection.rs"]
mod connection;
#[path = "iolink-cli/identity.rs"]
mod identity;
#[path = "iolink-cli/local_api.rs"]
mod local_api;
#[path = "iolink-cli/logging.rs"]
mod logging;
#[path = "iolink-cli/mapping.rs"]
mod mapping;
#[path = "iolink-cli/mqtt.rs"]
mod mqtt;
#[path = "iolink-cli/shared_memory.rs"]
mod shared_memory;
#[path = "iolink-cli/write.rs"]
mod write;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Preview an AL1350 write offline, or execute the exact reviewed operation with --confirm.
    Write(write::Arguments),
    /// Read a prospective AL1350 write target after checking the master and applicable device pins.
    ReadCurrent(write::ReadArguments),
    /// Read one consistent Windows shared-memory snapshot; no profile/gateway access.
    ReadSharedMemory {
        name: String,
        #[arg(long, default_value_t = 50)]
        wait_ms: u32,
        #[arg(long, default_value_t = 1000)]
        max_age_ms: u64,
    },
    /// List device variants, process input and output layouts, and fields needing review.
    Iodd {
        path: PathBuf,
        #[command(flatten)]
        options: IoddOptions,
    },
    /// Save an offline profile from an IODD variant and process data layout.
    Import {
        path: PathBuf,
        profile: PathBuf,
        #[command(flatten)]
        options: IoddOptions,
        #[arg(long)]
        variant: Option<String>,
        /// Exact input, output-only or acyclic-only layout ID.
        #[arg(long)]
        layout: Option<String>,
    },
    /// Check a profile and list its effective fields.
    Validate { profile: PathBuf },
    /// Review configured row semantics, settings and CSV columns offline; no gateway access.
    Logging { profile: PathBuf },
    /// Save logging mode, queue, rotation and CSV selection to a new profile offline.
    SetLogging {
        profile: PathBuf,
        output: PathBuf,
        #[command(flatten)]
        changes: logging::Arguments,
    },
    /// Save a cyclic manual field or IODD override to a new profile offline.
    SetField {
        profile: PathBuf,
        output: PathBuf,
        name: String,
        #[command(flatten)]
        changes: mapping::Arguments,
    },
    /// Remove a manual field/override in a new profile; imported IODD definitions are retained.
    RemoveField {
        profile: PathBuf,
        output: PathBuf,
        name: String,
    },
    /// Save host connection settings to a new profile offline; no gateway access or configuration.
    SetConnection {
        profile: PathBuf,
        output: PathBuf,
        #[command(flatten)]
        changes: connection::Arguments,
    },
    /// Save expected device identity pins to a new profile offline; no gateway access.
    SetIdentity {
        profile: PathBuf,
        output: PathBuf,
        #[command(flatten)]
        changes: identity::Arguments,
    },
    /// Compare declared cycle limits and a host poll interval offline; no gateway access.
    Timing {
        profile: PathBuf,
        #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u64).range(1..))]
        target_us: u64,
    },
    /// Validate selected XML or ZIP member against a supplied local XSD closure.
    CheckIodd {
        path: PathBuf,
        #[arg(long)]
        xsd: PathBuf,
        #[arg(long)]
        entry: Option<String>,
    },
    /// Verify binary CRCs for an IODD main XML and any selected external language.
    CheckStamp {
        path: PathBuf,
        #[arg(long)]
        entry: Option<String>,
        #[arg(long)]
        language: Option<String>,
        #[arg(long, requires = "language")]
        language_file: Option<PathBuf>,
    },
    /// Decode a hexadecimal process input without a gateway.
    Decode { profile: PathBuf, payload: String },
    /// Inspect a hexadecimal process output using the selected profile's IODD, offline.
    DecodeOutput { profile: PathBuf, payload: String },
    /// List imported IODD parameters without contacting a gateway.
    Parameters { profile: PathBuf },
    /// Inspect declared defaults from a saved profile without gateway access or writes.
    Defaults {
        profile: PathBuf,
        id: Option<String>,
        #[arg(long, requires = "id")]
        subindex: Option<u8>,
    },
    /// Save a host-only preferred parameter display to an output profile. No gateway access.
    SelectDisplay {
        profile: PathBuf,
        id: String,
        output: PathBuf,
        #[arg(long, default_value_t = 0)]
        subindex: u8,
        #[arg(long, conflicts_with = "clear", required_unless_present = "clear")]
        display: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// List saved IODD event definitions and unresolved standard event references.
    Events { profile: PathBuf },
    /// Explicitly read optional Device Status (36:0) and Detailed Device Status (37:0).
    ReadDiagnostics {
        profile: PathBuf,
        /// Save the raw replies, errors, and connection context to a new JSON file.
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// Review a saved diagnostic snapshot offline using its embedded profile.
    ReviewDiagnostics { snapshot: PathBuf },
    /// Compare two saved diagnostic snapshots by query and wire slot, entirely offline.
    CompareDiagnostics { left: PathBuf, right: PathBuf },
    /// Decode a recorded DeviceAccessLocks reply (12:0).
    DecodeDeviceAccessLocks {
        payload: String,
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Read DeviceAccessLocks once from the configured port.
    ReadDeviceAccessLocks {
        profile: PathBuf,
        /// Save the reply and connection context to a new JSON file.
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// Review saved DeviceAccessLocks evidence offline.
    ReviewDeviceAccessLocks { snapshot: PathBuf },
    /// Read profile IDs and both process descriptors from one configured port.
    ReadDeviceProfile {
        profile: PathBuf,
        /// Save the three replies and their context to a new JSON file.
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// Review saved profile IDs and process descriptors offline.
    ReviewDeviceProfile { snapshot: PathBuf },
    /// Compare two saved device profile snapshots offline.
    CompareDeviceProfiles { left: PathBuf, right: PathBuf },
    /// Decode a recorded Detailed Device Status payload without opening a transport.
    DecodeDiagnostics {
        profile: PathBuf,
        payload: String,
        #[arg(long)]
        device_status: Option<String>,
    },
    /// Decode the two ErrorType bytes from a recorded negative ISDU reply.
    DecodeErrorType {
        payload: String,
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Compare a recorded process data descriptor (14:0 or 15:0) with an optional IODD.
    DecodeProcessDescriptor {
        #[arg(value_enum)]
        direction: ProcessDescriptorDirection,
        payload: String,
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Preview host-only manual fields from a recorded 14:0 reply.
    PlanInputDescriptor {
        profile: PathBuf,
        payload: String,
        /// Save a new profile with the proposed fields.
        #[arg(long)]
        save_profile: Option<PathBuf>,
    },
    /// Compare a recorded whole ProfileCharacteristic (13:0) reply with an optional IODD.
    DecodeProfileCharacteristic {
        payload: String,
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Read one named IODD parameter through the independent acyclic client.
    ReadParameter {
        profile: PathBuf,
        id: String,
        #[arg(long, default_value_t = 0)]
        subindex: u8,
        /// Save the raw response and its context to a new JSON snapshot file.
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// Resolve the selected IODD layout condition offline or explicitly read its selector.
    CheckLayout {
        profile: PathBuf,
        /// Read the resolved selector through the independent acyclic client.
        #[arg(long)]
        read: bool,
    },
    /// Reopen a saved parameter snapshot offline, using its embedded profile.
    ReviewParameter { snapshot: PathBuf },
    /// Compare exact bytes, saved definitions, diagnostics and decoded values from two snapshots.
    CompareParameters { left: PathBuf, right: PathBuf },
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
    /// Verify all saved rows and review independent interface loss/accounting offline.
    ReviewInterfaces { directory: PathBuf },
    /// Verify and compare two saved captures offline, with explicit context/availability.
    CompareCaptures { left: PathBuf, right: PathBuf },
    /// Verify a saved capture and review one logged field without opening a gateway.
    ReviewCapture {
        directory: PathBuf,
        /// Exact saved field name; omitted selects the first logged field.
        #[arg(long)]
        field: Option<String>,
        #[arg(long, default_value_t = 128, value_parser = clap::value_parser!(u16).range(1..=4096))]
        buckets: u16,
    },
    /// Verify a saved capture and export raw payloads decoded with another profile.
    RedecodeCapture {
        directory: PathBuf,
        mapping_profile: PathBuf,
        output: PathBuf,
    },
    /// Verify a re-decode bundle; optionally reverify an explicitly supplied original capture.
    VerifyExport {
        directory: PathBuf,
        #[arg(long)]
        source: Option<PathBuf>,
    },
    /// Read master, firmware, port, and connected identity.
    Status {
        profile: PathBuf,
        #[arg(long)]
        verify_product_id: bool,
    },
    /// Read ifm master diagnostics and port metadata without sampling device inputs.
    InspectMaster {
        profile: PathBuf,
        /// Save the report as a new JSON snapshot.
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// Review a saved master inspection without contacting the gateway.
    ReviewMaster { snapshot: PathBuf },
    /// Compare two saved ifm master inspections offline.
    CompareMaster { left: PathBuf, right: PathBuf },
    /// Read status for explicitly profiled ports of the same master, up to its port limit.
    StatusPorts {
        #[arg(required = true, num_args = 1..=iolink_workbench::profile::MAX_PORTS)]
        profiles: Vec<PathBuf>,
    },
    /// Read one explicit acyclic ISDU parameter using a separate client.
    Isdu {
        profile: PathBuf,
        index: u16,
        #[arg(default_value_t = 0)]
        subindex: u8,
    },
    /// Capture explicitly profiled ports of the same master concurrently, up to its port limit.
    CapturePorts {
        output: PathBuf,
        #[arg(required = true, num_args = 1..=iolink_workbench::profile::MAX_PORTS)]
        profiles: Vec<PathBuf>,
        #[arg(long, default_value_t = 10.0)]
        seconds: f64,
        #[arg(long)]
        simulate_faults: bool,
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=100))]
        simulate_writer_delay_ms: u64,
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
        /// Log only raw inputs and metadata; preserve mappings for offline decoding.
        #[arg(long, conflicts_with_all = ["fields", "all_fields"])]
        raw_only: bool,
        /// Log all mapped values in mapping order.
        #[arg(long, conflicts_with_all = ["fields", "raw_only"])]
        all_fields: bool,
        /// Exact mapped field name; repeat to choose CSV value order.
        #[arg(long = "field", value_name = "NAME", conflicts_with_all = ["raw_only", "all_fields"])]
        fields: Vec<String>,
        #[command(flatten)]
        api: local_api::Arguments,
        #[command(flatten)]
        mqtt: mqtt::Arguments,
        #[command(flatten)]
        shared_memory: shared_memory::Arguments,
    },
}

#[derive(clap::Args)]
struct IoddOptions {
    /// Require matching main and selected external-language CRC stamps.
    #[arg(long)]
    require_stamp: bool,
    #[arg(long)]
    entry: Option<String>,
    #[arg(long)]
    standard_definitions: Option<PathBuf>,
    /// Override the adjacent standard-definition language XML.
    #[arg(long, requires_all = ["language", "standard_definitions"])]
    standard_language_file: Option<PathBuf>,
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
            require_stamp: self.require_stamp,
            entry: self.entry.as_deref(),
            definitions: self.standard_definitions.as_deref(),
            standard_language_file: self.standard_language_file.as_deref(),
            xsd: self.xsd.as_deref(),
            language: self.language.as_deref(),
            language_file: self.language_file.as_deref(),
            unit_definitions: self.unit_definitions.as_deref(),
        }
    }
}

fn capture_duration(seconds: f64) -> Result<Duration> {
    ensure!(
        seconds.is_finite() && (0.01..=86400.0).contains(&seconds),
        "--seconds must be 0.01–86400"
    );
    Ok(Duration::from_secs_f64(seconds))
}

fn interruptible_stop() -> Result<Stop> {
    let stop = Stop::default();
    let interrupt = stop.clone();
    ctrlc::set_handler(move || interrupt.request())?;
    Ok(stop)
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ProcessDescriptorDirection {
    Input,
    Output,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Write(args) => write::run(args)?,
        Command::ReadCurrent(args) => write::read_current(args)?,
        Command::CompareCaptures { left, right } => {
            print!(
                "{}",
                toml::to_string_pretty(&archive::compare_captures(&left, &right)?)?
            );
        }
        Command::ReviewInterfaces { directory } => {
            print!(
                "{}",
                toml::to_string_pretty(&archive::review_interfaces(&directory, None)?)?
            );
        }
        Command::ReadSharedMemory {
            name,
            wait_ms,
            max_age_ms,
        } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&iolink_workbench::shared_memory::read(
                    &name, wait_ms, max_age_ms
                )?)?
            );
        }
        Command::Logging { profile } => {
            let profile = Profile::load(&profile)?;
            print!("{}", toml::to_string_pretty(&profile.logging_plan()?)?);
        }
        Command::SetLogging {
            profile,
            output,
            changes,
        } => {
            let source = Profile::load(&profile)?;
            let candidate = changes.apply(&source)?;
            candidate.save_new(&output)?;
            println!(
                "Saved host logging settings to {}. No gateway access or device configuration was performed.",
                output.display()
            );
        }
        Command::SetField {
            profile,
            output,
            name,
            changes,
        } => {
            let source = Profile::load(&profile)?;
            changes.apply(&source, &name)?.save_new(&output)?;
            println!(
                "Saved host decoding field to {}. No gateway access or device configuration was performed.",
                output.display()
            );
        }
        Command::RemoveField {
            profile,
            output,
            name,
        } => {
            let source = Profile::load(&profile)?;
            source.without_manual_field(&name)?.save_new(&output)?;
            println!(
                "Removed manual decoding field/override in {}. Imported IODD definitions are retained.",
                output.display()
            );
        }
        Command::SetIdentity {
            profile,
            output,
            changes,
        } => {
            let source = Profile::load(&profile)?;
            let candidate = changes.apply(&source)?;
            candidate.save_new(&output)?;
            println!(
                "Saved expected identity pins to {}. These are configured expectations; no device identity was read or verified.",
                output.display()
            );
            if let Some(note) = candidate.identity_variant_note() {
                println!("Review: {note}");
            }
        }
        Command::SetConnection {
            profile,
            output,
            changes,
        } => {
            let profile = Profile::load(&profile)?;
            changes.apply(&profile)?.save_new(&output)?;
            println!(
                "Saved host connection settings to {}. No gateway connection or configuration was performed.",
                output.display()
            );
        }
        Command::VerifyExport { directory, source } => {
            print!(
                "{}",
                toml::to_string_pretty(&archive::verify_export(&directory, source.as_deref())?)?
            );
        }
        Command::RedecodeCapture {
            directory,
            mapping_profile,
            output,
        } => {
            let mapping = Profile::load(&mapping_profile)?;
            print!(
                "{}",
                toml::to_string_pretty(&archive::redecode(&directory, &mapping, &output)?)?
            );
        }
        Command::ReviewCapture {
            directory,
            field,
            buckets,
        } => {
            print!(
                "{}",
                toml::to_string_pretty(&archive::review(
                    &directory,
                    field.as_deref(),
                    usize::from(buckets)
                )?)?
            );
        }
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
        Command::ReadDiagnostics { profile, snapshot } => {
            let profile = Profile::load(&profile)?;
            let report = if let Some(path) = snapshot {
                diagnostics::check_destination(&path)?;
                let (snapshot, report) = diagnostics::read_snapshot(&profile)?;
                print!("{}", toml::to_string_pretty(&report)?);
                snapshot.save(&path)?;
                eprintln!("Saved diagnostic snapshot {}", path.display());
                report
            } else {
                let report = diagnostics::read(&profile)?;
                print!("{}", toml::to_string_pretty(&report)?);
                report
            };
            ensure!(
                report.ok(),
                "diagnostic read needs review; retained results are printed above"
            );
        }
        Command::ReviewDiagnostics { snapshot } => {
            let review = diagnostics::Snapshot::load(&snapshot)?.review()?;
            print!("{}", toml::to_string_pretty(&review)?);
            ensure!(
                review.ok(),
                "saved diagnostic snapshot needs review; retained results are printed above"
            );
        }
        Command::DecodeDeviceAccessLocks { payload, profile } => {
            let profile = profile.as_deref().map(Profile::load).transpose()?;
            let report = diagnostics::decode_device_access_locks(profile.as_ref(), &payload)?;
            print!("{}", toml::to_string_pretty(&report)?);
            ensure!(
                report.ok(),
                "Device Access Locks need review. Retained results are printed above."
            );
        }
        Command::ReadDeviceAccessLocks { profile, snapshot } => {
            let profile = Profile::load(&profile)?;
            if let Some(path) = &snapshot {
                diagnostics::check_destination(path)?;
            }
            let evidence = diagnostics::AccessLocksSnapshot::read(&profile)?;
            let review = evidence.review()?;
            print!("{}", toml::to_string_pretty(&review)?);
            if let Some(path) = snapshot {
                evidence.save(&path)?;
                eprintln!("Saved access locks snapshot {}", path.display());
            }
            ensure!(
                review.ok(),
                "Device Access Locks need review. Retained results are printed above."
            );
        }
        Command::ReviewDeviceAccessLocks { snapshot } => {
            let review = diagnostics::AccessLocksSnapshot::load(&snapshot)?.review()?;
            print!("{}", toml::to_string_pretty(&review)?);
            ensure!(
                review.ok(),
                "Device Access Locks need review. Retained results are printed above."
            );
        }
        Command::ReadDeviceProfile { profile, snapshot } => {
            let profile = Profile::load(&profile)?;
            if let Some(path) = &snapshot {
                diagnostics::check_destination(path)?;
            }
            let evidence = diagnostics::DeviceProfileSnapshot::read(&profile)?;
            let review = evidence.review()?;
            print!("{}", toml::to_string_pretty(&review)?);
            if let Some(path) = snapshot {
                evidence.save(&path)?;
                eprintln!("Saved device profile snapshot {}", path.display());
            }
            ensure!(
                review.ok(),
                "Device profile needs review. Retained results are printed above."
            );
        }
        Command::ReviewDeviceProfile { snapshot } => {
            let review = diagnostics::DeviceProfileSnapshot::load(&snapshot)?.review()?;
            print!("{}", toml::to_string_pretty(&review)?);
            ensure!(
                review.ok(),
                "Device profile needs review. Retained results are printed above."
            );
        }
        Command::CompareDeviceProfiles { left, right } => {
            let comparison = diagnostics::DeviceProfileComparison::load(&left, &right)?;
            print!("{}", toml::to_string_pretty(&comparison)?);
            ensure!(
                comparison.ok(),
                "Device profile comparison needs review. Retained results are printed above."
            );
        }
        Command::CompareDiagnostics { left, right } => {
            let left = diagnostics::Snapshot::load(&left)?;
            let right = diagnostics::Snapshot::load(&right)?;
            let comparison = diagnostics::compare(&left, &right)?;
            print!("{}", toml::to_string_pretty(&comparison)?);
            ensure!(
                comparison.ok(),
                "a saved diagnostic snapshot needs review; retained results are printed above"
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
        Command::DecodeErrorType { payload, profile } => {
            let profile = profile.as_deref().map(Profile::load).transpose()?;
            let bytes = decode::parse_hex_bounded(&payload, 2)?;
            let report = diagnostics::decode_error_type(profile.as_ref(), &bytes)?;
            print!("{}", toml::to_string_pretty(&report)?);
        }
        Command::DecodeProcessDescriptor {
            direction,
            payload,
            profile,
        } => {
            use diagnostics::{DescriptorDirection, DescriptorStatus};
            let direction = match direction {
                ProcessDescriptorDirection::Input => DescriptorDirection::Input,
                ProcessDescriptorDirection::Output => DescriptorDirection::Output,
            };
            let profile = profile.as_deref().map(Profile::load).transpose()?;
            let report =
                diagnostics::decode_process_descriptor(direction, profile.as_ref(), &payload)?;
            print!("{}", toml::to_string_pretty(&report)?);
            ensure!(
                matches!(
                    report.status,
                    DescriptorStatus::Match | DescriptorStatus::NoIodd
                ),
                "Process descriptor needs review. See the report above."
            );
        }
        Command::PlanInputDescriptor {
            profile,
            payload,
            save_profile,
        } => {
            let profile = Profile::load(&profile)?;
            let plan = diagnostics::plan_input_descriptor(&profile, &payload)?;
            print!("{}", toml::to_string_pretty(&plan)?);
            if let Some(path) = save_profile {
                plan.apply(&profile)?.save_new(&path)?;
            }
        }
        Command::DecodeProfileCharacteristic { payload, profile } => {
            use diagnostics::ProfileCharacteristicStatus;
            let profile = profile.as_deref().map(Profile::load).transpose()?;
            let report = diagnostics::decode_profile_characteristic(profile.as_ref(), &payload)?;
            print!("{}", toml::to_string_pretty(&report)?);
            ensure!(
                matches!(
                    report.status,
                    ProfileCharacteristicStatus::Match | ProfileCharacteristicStatus::Undeclared
                ),
                "ProfileCharacteristic needs review. See the report above."
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
        Command::SelectDisplay {
            profile,
            id,
            output,
            subindex,
            display,
            clear: _,
        } => {
            let mut profile = Profile::load(&profile)?;
            profile.set_parameter_display(&id, subindex, display.as_deref())?;
            profile.save(&output)?;
            println!("Saved {}", output.display());
        }
        Command::ReadParameter {
            profile,
            id,
            subindex,
            snapshot,
        } => {
            let profile = Profile::load(&profile)?;
            if let Some(path) = snapshot {
                parameter_snapshot::check_destination(&path)?;
                let (snapshot, reading) = parameter_snapshot::read(&profile, &id, subindex)?;
                print!("{}", toml::to_string_pretty(&reading)?);
                snapshot.save(&path)?;
                eprintln!("Saved snapshot {}", path.display());
            } else {
                print!(
                    "{}",
                    toml::to_string_pretty(&transport::read_parameter_at(
                        &profile, &id, subindex
                    )?)?
                );
            }
        }
        Command::CheckLayout { profile, read } => {
            use transport::layout_condition::{self, Outcome, Plan};
            let profile = Profile::load(&profile)?;
            if read {
                let result = layout_condition::read(&profile)?;
                print!("{}", toml::to_string_pretty(&result)?);
                match result.outcome {
                    Outcome::Matched => {}
                    Outcome::Mismatch => anyhow::bail!("layout condition mismatch"),
                    Outcome::Unavailable => anyhow::bail!("layout selector value unavailable"),
                }
            } else {
                print!(
                    "{}",
                    toml::to_string_pretty(&Plan::from_profile(&profile)?)?
                );
            }
        }
        Command::ReviewParameter { snapshot } => {
            let review = parameter_snapshot::Snapshot::load(&snapshot)?.review()?;
            print!("{}", toml::to_string_pretty(&review)?);
            ensure!(
                !review.decoded.needs_review(),
                "parameter payload needs decoding review; raw bytes retained above"
            );
        }
        Command::CompareParameters { left, right } => {
            let left = parameter_snapshot::Snapshot::load(&left)?;
            let right = parameter_snapshot::Snapshot::load(&right)?;
            let comparison = parameter_snapshot::compare(&left, &right)?;
            print!("{}", toml::to_string_pretty(&comparison)?);
            ensure!(
                !comparison.left.decoded.needs_review() && !comparison.right.decoded.needs_review(),
                "parameter payload needs decoding review; raw bytes retained above"
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
                !decoded.needs_review(),
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
        Command::CheckStamp {
            path,
            entry,
            language,
            language_file,
        } => {
            let report = iodd::check_stamps(
                &path,
                entry.as_deref(),
                language.as_deref(),
                language_file.as_deref(),
            )?;
            print!("{}", toml::to_string_pretty(&report)?);
            if !report.matched() {
                std::process::exit(1);
            }
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
            let mut profile_data = Profile {
                name: imported.variant.clone(),
                ..Profile::default()
            };
            profile_data.apply_iodd(expected_identity, imported);
            profile_data.save(&profile)?;
            println!("Saved {}", profile.display());
        }
        Command::Timing { profile, target_us } => {
            let profile = Profile::load(&profile)?;
            let context = iolink_workbench::timing::assess(
                &profile,
                Some(profile.connection.poll_us),
                Default::default(),
                target_us,
            )?;
            print!("{}", toml::to_string_pretty(&context)?);
        }
        Command::Defaults {
            profile,
            id,
            subindex,
        } => {
            let profile = Profile::load(&profile)?;
            let report = iodd::defaults::inspect_profile(&profile, id.as_deref(), subindex)?;
            print!("{}", toml::to_string_pretty(&report)?);
            ensure!(
                report.needs_review == 0
                    && report.unsupported == 0
                    && report.unresolved_standard_parameter_refs.is_empty(),
                "declared default metadata needs review; declarations retained above"
            );
        }
        Command::Validate { profile } => {
            let profile = Profile::load(&profile)?;
            println!("{} ({} bytes)", profile.name, profile.input_bytes);
            if let Some(field) = profile.padded_input_field() {
                println!(
                    "IODD standalone integer input · {} value bits in {} wire bytes",
                    field.bits, profile.input_bytes
                );
            }
            println!(
                "CSV typed columns · {:?}",
                profile
                    .log_fields()
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<Vec<_>>()
            );
            if let Some(iodd) = &profile.iodd {
                println!(
                    "{}",
                    iodd.communication
                        .as_ref()
                        .map(|value| value.summary())
                        .unwrap_or_else(
                            || "IODD wired communication declaration unavailable".into()
                        )
                );
                if let Some(features) = &iodd.features {
                    println!("IODD declared capabilities\n{}", features.summary());
                }
                println!("{}", iodd.stamp_validation.as_ref().map(|report| report.summary()).unwrap_or_else(|| "IODD checksum not checked in this saved profile".into()));
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
        Command::InspectMaster { profile, snapshot } => {
            let profile = Profile::load(&profile)?;
            if let Some(path) = &snapshot {
                transport::MasterInspectionSnapshot::check_destination(path)?;
            }
            let report = transport::inspect_master(&profile)?;
            print!("{}", toml::to_string_pretty(&report)?);
            let complete = report.error.is_none();
            if let Some(path) = snapshot {
                transport::MasterInspectionSnapshot::new(&profile, report).save(&path)?;
            }
            ensure!(
                complete,
                "master inspection needs review; see the report above"
            );
        }
        Command::ReviewMaster { snapshot } => {
            let saved = transport::MasterInspectionSnapshot::load(&snapshot)?;
            print!("{}", toml::to_string_pretty(&saved)?);
            ensure!(
                saved.inspection.error.is_none(),
                "saved master inspection needs review; see the report above"
            );
        }
        Command::CompareMaster { left, right } => {
            let comparison = transport::MasterInspectionComparison::load(&left, &right)?;
            print!("{}", toml::to_string_pretty(&comparison)?);
            ensure!(
                comparison.observations.is_some(),
                "master comparison needs review; see the report above"
            );
        }
        Command::StatusPorts { profiles } => {
            let profiles = profiles
                .iter()
                .map(|path| Profile::load(path))
                .collect::<Result<Vec<_>>>()?;
            let report = transport::read_port_statuses(&profiles)?;
            print!("{}", toml::to_string_pretty(&report)?);
            ensure!(
                report.all_successful(),
                "one or more port status checks failed; see the port report above"
            );
        }
        Command::Isdu {
            profile,
            index,
            subindex,
        } => {
            let bytes = transport::read_isdu(&Profile::load(&profile)?, index, subindex)?;
            #[derive(serde::Serialize)]
            struct RawReply<'a> {
                hex: String,
                utf8: Option<&'a str>,
            }
            let reply = RawReply {
                hex: decode::hex(&bytes),
                utf8: std::str::from_utf8(&bytes).ok(),
            };
            print!("{}", toml::to_string_pretty(&reply)?);
        }
        Command::CapturePorts {
            output,
            profiles,
            seconds,
            simulate_faults,
            simulate_writer_delay_ms,
        } => {
            let duration = capture_duration(seconds)?;
            let profiles = profiles
                .iter()
                .map(|path| Profile::load(path))
                .collect::<Result<Vec<_>>>()?;
            let stop = interruptible_stop()?;
            let report = capture::run_ports(
                &profiles,
                &capture::PortOptions {
                    output,
                    duration,
                    simulate_faults,
                    simulated_writer_delay: Duration::from_millis(simulate_writer_delay_ms),
                },
                &stop,
            )?;
            print!("{}", toml::to_string_pretty(&report)?);
            ensure!(
                report.all_successful(),
                "the port capture set contains a failure or conflicting master observations; see the report"
            );
            if report.stopped_by_request {
                std::process::exit(130);
            }
        }
        Command::Capture {
            profile,
            output,
            seconds,
            simulate_faults,
            simulate_writer_delay_ms,
            queue_capacity,
            raw_only,
            all_fields,
            fields,
            api,
            mqtt,
            shared_memory,
        } => {
            let duration = capture_duration(seconds)?;
            let mut profile = Profile::load(&profile)?;
            profile.require_process_input()?;
            if let Some(capacity) = queue_capacity {
                profile.logging.queue_capacity = capacity;
            }
            if raw_only {
                profile.set_log_selection(LogSelection::RawOnly)?;
            } else if all_fields {
                profile.set_log_selection(LogSelection::All)?;
            } else if !fields.is_empty() {
                profile.set_log_selection(LogSelection::Fields(fields))?;
            }
            let stop = interruptible_stop()?;
            let report = capture::run(
                &profile,
                &Options {
                    output: Some(output),
                    duration: Some(duration),
                    simulate_faults,
                    simulated_writer_delay: Duration::from_millis(simulate_writer_delay_ms),
                    local_api: api.config(),
                    mqtt: mqtt.config(),
                    shared_memory: shared_memory.config(),
                },
                &stop,
                None,
            )?;
            print!("{}", toml::to_string_pretty(&report)?);
            ensure!(
                report.success(),
                "capture ended with a transport, writer or integration worker error; see the report"
            );
            if report.stopped_by_request {
                std::process::exit(130);
            }
        }
        Command::DecodeOutput { profile, payload } => {
            #[derive(serde::Serialize)]
            struct Inspection {
                raw_hex: String,
                #[serde(skip_serializing_if = "Option::is_none")]
                iodd_output: Option<iodd::DecodedOutput>,
            }
            let profile = Profile::load(&profile)?;
            profile.require_process_output()?;
            let payload = decode::parse_hex(&payload)?;
            let inspection = Inspection {
                raw_hex: decode::hex(&payload),
                iodd_output: profile
                    .output_layout()
                    .map(|layout| layout.decode_output(&payload, None)),
            };
            println!("{}", serde_json::to_string_pretty(&inspection)?);
        }
        Command::Decode { profile, payload } => {
            let profile = Profile::load(&profile)?;
            let payload = decode::parse_hex(&payload)?;
            if let Some(warning) = profile.input_encoding_warning(&payload) {
                eprintln!("{warning}");
            }
            for (field, origin, value) in profile.decode(&payload)? {
                println!(
                    "{} [{origin}]: {} {} (raw {}, bits 0x{})",
                    field.name,
                    value.display(),
                    field.unit,
                    value.raw,
                    value.bits_hex
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::CommandFactory;

    #[test]
    fn argument_groups_are_consistent() {
        Cli::command().debug_assert();
    }
}
