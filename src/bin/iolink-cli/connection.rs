use clap::{Args, ValueEnum};
use iolink_workbench::profile::{Connection, CycleUnit, Gateway, Profile};

#[derive(Clone, Copy, ValueEnum)]
enum GatewayChoice {
    Simulator,
    IfmAl1350,
}

#[derive(Clone, Copy, ValueEnum)]
enum CycleChoice {
    Unknown,
    Microseconds,
    Milliseconds,
}

#[derive(Args)]
#[group(required = true, multiple = true)]
pub struct Arguments {
    #[arg(long, value_enum)]
    gateway: Option<GatewayChoice>,
    /// Host endpoint origin; empty keeps an unfinished offline template.
    #[arg(long)]
    endpoint: Option<String>,
    #[arg(long)]
    port: Option<u8>,
    /// Expected installed firmware; this does not upgrade firmware.
    #[arg(long)]
    firmware: Option<String>,
    /// Check an existing numeric IO-Link mode; never changes the port mode.
    #[arg(long, conflicts_with = "clear_expected_mode")]
    expected_mode: Option<u8>,
    #[arg(long)]
    clear_expected_mode: bool,
    /// Operator-confirmed unit for reported port cycles; unknown clears it.
    #[arg(long, value_enum)]
    cycle_unit: Option<CycleChoice>,
    #[arg(long)]
    poll_us: Option<u64>,
    #[arg(long)]
    timeout_ms: Option<u64>,
    #[arg(long)]
    stale_us: Option<u64>,
    #[arg(long)]
    reconnect_ms: Option<u64>,
    /// Select simulator snapshot polling (true) or synthetic counters (false).
    #[arg(long)]
    simulator_snapshots: Option<bool>,
}

impl Arguments {
    pub fn apply(self, profile: &Profile) -> anyhow::Result<Profile> {
        let mut connection: Connection = profile.connection.clone();
        if let Some(gateway) = self.gateway {
            connection.gateway = match gateway {
                GatewayChoice::Simulator => Gateway::Simulator,
                GatewayChoice::IfmAl1350 => Gateway::IfmAl1350,
            };
        }
        macro_rules! update {
            ($argument:ident, $field:ident) => {
                if let Some(value) = self.$argument {
                    connection.$field = value;
                }
            };
        }
        update!(endpoint, endpoint);
        update!(port, port);
        update!(firmware, ifm_firmware);
        update!(poll_us, poll_us);
        update!(timeout_ms, timeout_ms);
        update!(stale_us, stale_us);
        update!(reconnect_ms, reconnect_ms);
        update!(simulator_snapshots, simulate_snapshots);
        if let Some(mode) = self.expected_mode {
            connection.ifm_mode = Some(mode);
        }
        if self.clear_expected_mode {
            connection.ifm_mode = None;
        }
        if let Some(unit) = self.cycle_unit {
            connection.ifm_cycle_unit = match unit {
                CycleChoice::Unknown => None,
                CycleChoice::Microseconds => Some(CycleUnit::Microseconds),
                CycleChoice::Milliseconds => Some(CycleUnit::Milliseconds),
            };
        }
        profile.with_connection(connection)
    }
}
