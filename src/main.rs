#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod desktop;

fn main() -> eframe::Result {
    desktop::run()
}
