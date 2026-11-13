use clap::{Args, ValueEnum};
use iolink_workbench::profile::{LogMode, LogSelection, Profile};

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    EveryUpdate,
    OnChange,
}

#[derive(Args)]
#[group(required = true, multiple = true)]
pub struct Arguments {
    #[arg(long, value_enum)]
    mode: Option<Mode>,
    #[arg(long)]
    queue_capacity: Option<usize>,
    #[arg(long)]
    rotate_rows: Option<u64>,
    #[arg(long)]
    rotate_seconds: Option<u64>,
    #[arg(long, conflicts_with_all = ["all_fields", "fields"])]
    raw_only: bool,
    #[arg(long, conflicts_with_all = ["raw_only", "fields"])]
    all_fields: bool,
    /// Exact mapped field name; repeat to choose saved CSV value order.
    #[arg(long = "field", conflicts_with_all = ["raw_only", "all_fields"])]
    fields: Vec<String>,
}

impl Arguments {
    pub fn apply(self, profile: &Profile) -> anyhow::Result<Profile> {
        let mut logging = profile.logging.clone();
        if let Some(mode) = self.mode {
            logging.mode = match mode {
                Mode::EveryUpdate => LogMode::EveryUpdate,
                Mode::OnChange => LogMode::OnChange,
            };
        }
        if let Some(capacity) = self.queue_capacity {
            logging.queue_capacity = capacity;
        }
        if let Some(rows) = self.rotate_rows {
            logging.rotate_rows = rows;
        }
        if let Some(seconds) = self.rotate_seconds {
            logging.rotate_seconds = seconds;
        }
        let mut candidate = profile.with_logging(logging)?;
        if self.raw_only {
            candidate.set_log_selection(LogSelection::RawOnly)?;
        } else if self.all_fields {
            candidate.set_log_selection(LogSelection::All)?;
        } else if !self.fields.is_empty() {
            candidate.set_log_selection(LogSelection::Fields(self.fields))?;
        }
        Ok(candidate)
    }
}
