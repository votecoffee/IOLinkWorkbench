use eframe::egui::Ui;
use iolink_workbench::capture::{Latency, LoggerMetrics};

pub fn show(ui: &mut Ui, metrics: Option<&LoggerMetrics>) {
    ui.collapsing("Recorded logger pressure and timing", |ui| {
        let Some(metrics) = metrics else {
            ui.label("Logger measurements are unavailable for this run.");
            return;
        };
        ui.label(format!("Observed queue high water · {} of {} rows · {} rows dequeued", metrics.queue_high_water_observed, metrics.queue_capacity, metrics.rows_dequeued));
        ui.label(format!("Final stop and drain · {} µs", metrics.final_drain_us));
        latency(ui, "Queue wait", metrics.queue_wait.as_ref());
        latency(ui, "Row service", metrics.row_service.as_ref());
        latency(ui, "CSV flush attempts", metrics.flush.as_ref());
        latency(ui, "File sync attempts", metrics.sync.as_ref());
        ui.small("Queue occupancy is an observed lower bound. Queue wait ends at dequeue; row service includes decoding, buffering, rotation and simulated writer delay. Flush and sync are separate operation timings; rotation can include them inside row service. Stop and drain follows the acquisition window. These host measurements do not measure source cadence or prove a 1 ms capability.");
    });
}

fn latency(ui: &mut Ui, label: &str, value: Option<&Latency>) {
    match value {
        Some(value) => {
            ui.label(format!(
                "{label} · {} attempts · p50 {} · p95 {} · p99 {} · longest {} µs · mean {:.1} µs",
                value.count,
                value.p50_us,
                value.p95_us,
                value.p99_us,
                value.longest_us,
                value.mean_us
            ));
        }
        None => {
            ui.label(format!("{label} · no measured operations"));
        }
    }
}
