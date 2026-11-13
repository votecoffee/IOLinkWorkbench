use clap::Args;
use iolink_workbench::shared_memory::Config;

#[derive(Args)]
pub struct Arguments {
    /// Opt-in Windows local shared-memory latest snapshot; suffix, not a full object path.
    #[arg(long)]
    shared_memory: Option<String>,
    /// Requested latest-state commit spacing, 1–1000 ms (default 10).
    #[arg(long, requires = "shared_memory")]
    shared_memory_interval_ms: Option<u64>,
    /// Independent handoff, 1–16384 events (default 256).
    #[arg(long, requires = "shared_memory")]
    shared_memory_queue_capacity: Option<usize>,
}
impl Arguments {
    pub fn config(self) -> Option<Config> {
        self.shared_memory.map(|name| Config {
            name,
            interval_ms: self.shared_memory_interval_ms.unwrap_or(10),
            queue_capacity: self.shared_memory_queue_capacity.unwrap_or(256),
        })
    }
}
