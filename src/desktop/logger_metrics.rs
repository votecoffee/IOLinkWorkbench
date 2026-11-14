use eframe::egui::Ui;
use iolink_workbench::capture::{Latency, LoggerMetrics};

pub fn receive_tail(ui: &mut Ui, age_us: Option<u64>) {
    ui.label(match age_us {
        Some(age) => format!("Host receive age at acquisition end · {age} µs"),
        None => "Host receive age at acquisition end · unavailable".into(),
    });
    ui.small(
        "Terminal host silence ends before writer drain; source-update age and loss are separate.",
    );
}

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_receive_age_renders_known_zero_value_and_unavailable_without_io() {
        for age in [Some(0), Some(123456), None] {
            let context = eframe::egui::Context::default();
            let output = context.run(Default::default(), |ctx| {
                eframe::egui::CentralPanel::default().show(ctx, |ui| receive_tail(ui, age));
            });
            let text = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    eframe::egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            let expected = age
                .map(|v| format!("{v} µs"))
                .unwrap_or_else(|| "unavailable".into());
            assert!(text.contains(&expected));
            assert_eq!(text.contains("unavailable"), age.is_none());
        }
    }
}
