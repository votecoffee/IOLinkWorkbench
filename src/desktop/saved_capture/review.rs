use crate::desktop::{ACCENT, WARNING};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Sense, Stroke, Ui};
use iolink_workbench::{archive::Review, capture::Spacing};

fn number(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.6e}"))
        .unwrap_or_else(|| "unavailable".into())
}

pub fn show(ui: &mut Ui, review: &Review) {
    let signal = &review.signal;
    ui.separator();
    ui.heading(format!("{} · {}", signal.field.name, signal.field.unit));
    ui.label(format!(
        "Configured transport · {:?} · recorded route {} · logging {:?}",
        signal.configured_gateway,
        signal.recorded_route.as_deref().unwrap_or("unavailable"),
        signal.log_mode
    ));
    ui.small("This is an offline file review. Physical hardware qualification is separate.");
    ui.small(format!(
        "Recorded simulation settings · faults {:?} · writer delay {:?} ms",
        signal.simulation_faults, signal.simulated_writer_delay_ms
    ));
    ui.label(format!(
        "{} finite values · {} blank cells · {} non-finite cells · {} recorded logger overruns",
        signal.numeric.finite_values,
        signal.numeric.blank_values,
        signal.numeric.non_finite_values,
        signal.recorded_logger_overruns
    ));
    ui.label(format!(
        "Approximate minimum {} · maximum {} · mean {}",
        number(signal.numeric.minimum),
        number(signal.numeric.maximum),
        number(signal.numeric.mean)
    ));
    ui.label(format!(
        "First CSV value {} · last CSV value {}",
        signal
            .numeric
            .first_csv_value
            .as_deref()
            .unwrap_or("unavailable"),
        signal
            .numeric
            .last_csv_value
            .as_deref()
            .unwrap_or("unavailable")
    ));
    ui.small(signal.numeric_basis);
    plot(ui, review);
    ui.small("Each independent time bin shows finite minimum–maximum and the last finite value. Amber marks bins containing blank/non-finite cells or multiple generations. No lines join bins; gaps and event ordering inside a bin require the original CSV. Hover for bin details.");
    ui.collapsing("Recorded acquisition timing", |ui| {
        let timing = &signal.recorded_timing;
        ui.label(format!("Requested host poll interval · {} µs · stale threshold {} µs", signal.requested_poll_us.map(|value| value.to_string()).unwrap_or_else(|| "unavailable".into()), signal.stale_threshold_us));
        ui.label(format!("IO-Link port cycle · {} µs · raw {}", timing.port_cycle_us.map(|value| value.to_string()).unwrap_or_else(|| "unknown".into()), timing.port_cycle_raw.as_deref().unwrap_or("unavailable")));
        spacing(ui, "Host poll starts", timing.host_poll_start_spacing.as_ref());
        spacing(ui, "Host receive completions", timing.host_receive_spacing.as_ref());
        spacing(ui, "Observed new-update receive spacing", timing.host_new_update_spacing.as_ref());
        spacing(ui, "Gateway update timestamps", timing.gateway_update_spacing.as_ref());
        ui.small("These are the acquisition report's measurements, not timing derived from the selected CSV rows. Change-only logging and dropped rows reduce the recorded row set. A 1 ms target needs the eventual sensor and gateway qualification.");
    });
    crate::desktop::logger_metrics::show(ui, signal.recorded_timing.logger_metrics.as_ref());
}

fn spacing(ui: &mut Ui, name: &str, spacing: Option<&Spacing>) {
    if let Some(value) = spacing {
        ui.label(format!(
            "{name} · p50 {} · p95 {} · p99 {} · longest {} µs · {:.2} Hz",
            value.p50_us, value.p95_us, value.p99_us, value.longest_us, value.mean_hz
        ));
    } else {
        ui.label(format!("{name} · unavailable"));
    }
}

fn plot(ui: &mut Ui, review: &Review) {
    let signal = &review.signal;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width().max(120.0), 230.0),
        Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(15, 21, 27));
    let plot = rect.shrink2(egui::vec2(12.0, 26.0));
    painter.text(
        Pos2::new(plot.left(), rect.top() + 4.0),
        Align2::LEFT_TOP,
        number(signal.numeric.maximum),
        FontId::monospace(11.0),
        ACCENT,
    );
    painter.text(
        Pos2::new(plot.left(), rect.bottom() - 4.0),
        Align2::LEFT_BOTTOM,
        number(signal.numeric.minimum),
        FontId::monospace(11.0),
        ACCENT,
    );
    painter.text(
        Pos2::new(plot.right(), rect.bottom() - 4.0),
        Align2::RIGHT_BOTTOM,
        format!(
            "0 … {:.6} s host elapsed",
            signal.elapsed_us as f64 / 1_000_000.0
        ),
        FontId::monospace(11.0),
        Color32::GRAY,
    );
    let width = plot.width() / signal.buckets_requested as f32;
    for bucket in &signal.buckets {
        let left = plot.left() + bucket.bin as f32 * width;
        if bucket.blank_values != 0
            || bucket.non_finite_values != 0
            || bucket.first_generation != bucket.last_generation
        {
            painter.rect_filled(
                egui::Rect::from_min_max(
                    Pos2::new(left, plot.top()),
                    Pos2::new(left + width, plot.bottom()),
                ),
                0.0,
                Color32::from_rgba_unmultiplied(253, 196, 111, 32),
            );
        }
        if let (Some(low), Some(high), Some(last), Some(minimum), Some(maximum)) = (
            bucket.minimum,
            bucket.maximum,
            bucket.last,
            signal.numeric.minimum,
            signal.numeric.maximum,
        ) {
            // Normalize before subtracting so a finite range spanning ±f64::MAX stays finite.
            let scale = minimum.abs().max(maximum.abs()).max(1.0);
            let lower = minimum / scale;
            let span = maximum / scale - lower;
            let y = |value: f64| {
                plot.bottom()
                    - (if span == 0.0 {
                        0.5
                    } else {
                        (value / scale - lower) / span
                    } as f32)
                        * plot.height()
            };
            let x = left + width / 2.0;
            painter.line_segment(
                [Pos2::new(x, y(low)), Pos2::new(x, y(high))],
                Stroke::new(width.clamp(1.0, 3.0), ACCENT),
            );
            painter.circle_filled(Pos2::new(x, y(last)), 1.6, ACCENT);
        }
    }
    if signal.numeric.finite_values == 0 {
        painter.text(
            plot.center(),
            Align2::CENTER_CENTER,
            "No finite typed values",
            FontId::proportional(15.0),
            WARNING,
        );
    }
    if let Some(position) = response
        .hover_pos()
        .filter(|position| plot.contains(*position))
    {
        let bin = (((position.x - plot.left()) / plot.width()) * signal.buckets_requested as f32)
            as usize;
        if let Some(bucket) = signal.buckets.get(bin) {
            response.on_hover_ui(|ui| {
                ui.label(format!(
                    "Time bin {} · {} recorded rows",
                    bucket.bin + 1,
                    bucket.rows
                ));
                ui.label(format!(
                    "Host elapsed {:?} … {:?} µs · generations {:?} … {:?}",
                    bucket.first_host_us,
                    bucket.last_host_us,
                    bucket.first_generation,
                    bucket.last_generation
                ));
                ui.label(format!(
                    "{} finite · {} blank · {} non-finite",
                    bucket.finite_values, bucket.blank_values, bucket.non_finite_values
                ));
                ui.label(format!(
                    "Minimum {} · maximum {} · last {}",
                    number(bucket.minimum),
                    number(bucket.maximum),
                    number(bucket.last)
                ));
            });
        }
    }
}
