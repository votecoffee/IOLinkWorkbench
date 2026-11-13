use clap::Args;
use iolink_workbench::local_api::Config;
use std::net::SocketAddr;

#[derive(Args)]
pub struct Arguments {
    /// Enable a read-only local API on 127.0.0.1:PORT; port 0 selects a free port.
    #[arg(long)]
    api_listen: Option<SocketAddr>,
    /// Independent API event queue, 1–16384 entries (default 1024).
    #[arg(long, requires = "api_listen")]
    api_queue_capacity: Option<usize>,
    /// Retained API events, 1–16384 entries (default 4096).
    #[arg(long, requires = "api_listen")]
    api_history_capacity: Option<usize>,
}

impl Arguments {
    pub fn config(self) -> Option<Config> {
        self.api_listen.map(|listen| Config {
            listen,
            queue_capacity: self.api_queue_capacity.unwrap_or(1024),
            history_capacity: self.api_history_capacity.unwrap_or(4096),
        })
    }
}
