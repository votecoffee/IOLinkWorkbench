#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod desktop;

#[derive(clap::Parser)]
#[command(version, about = "Read-only IO-Link desktop workbench")]
struct Arguments {
    /// Render built-in views offscreen with a hidden native OpenGL context; save a new JSON receipt.
    #[arg(long, value_name = "NEW_RECEIPT_JSON")]
    self_test: Option<std::path::PathBuf>,
}

fn main() -> anyhow::Result<()> {
    use clap::Parser;
    match Arguments::parse().self_test {
        Some(path) => desktop::self_test::run(&path),
        None => desktop::run().map_err(|error| anyhow::anyhow!("{error}")),
    }
}
