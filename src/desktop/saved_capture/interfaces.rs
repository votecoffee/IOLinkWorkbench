use crate::desktop::WARNING;
use eframe::egui::Ui;
use iolink_workbench::archive::{InterfaceReview, RecordedInterfaces};

pub(super) fn show(ui: &mut Ui, data: &RecordedInterfaces) {
    ui.collapsing("Recorded interface diagnostics",|ui| {
        ui.small(data.basis);ui.small(data.absence_meaning);
        ui.label(format!("HTTP API · {} · MQTT · {} · shared memory · {}",data.local_api_status.replace('_'," "),data.mqtt_status.replace('_'," "),data.shared_memory_status.replace('_'," ")));
        ui.label(if data.offer_counts_checked_against_acquisition {"Interface event-offer totals match recorded acquisition events."} else {"Offer-total comparison unavailable or no interface counters recorded."});
        if let Some(api)=&data.local_api {ui.label(format!("HTTP API · {} offered · {} ingested · {} queue drops · {} history evictions · {} failed requests",api.events_offered,api.events_ingested,api.queue_dropped,api.history_evicted,api.failed_requests));}
        if let Some(mqtt)=&data.mqtt {ui.label(format!("MQTT · {} offered · {} queue drops · {} coalesced · {} complete socket writes · {} failed writes · {} shutdown discards",mqtt.events_offered,mqtt.queue_dropped,mqtt.coalesced,mqtt.socket_writes_completed,mqtt.publish_write_failures,mqtt.shutdown_discarded));}
        if let Some(shm)=&data.shared_memory {ui.label(format!("Shared memory · {} offered · {} queue drops · {} coalesced · {} busy skips · {} commits · {} replacements · final inactive commit {}",shm.events_offered,shm.queue_dropped,shm.coalesced,shm.busy_skips,shm.snapshots_written,shm.snapshots_replaced,shm.final_snapshot_written));}
        for finding in &data.findings {
            let line=format!("{} · {}{} · {}",finding.component,finding.category.replace('_'," "),finding.count.map(|count|format!(" · {count}")).unwrap_or_default(),finding.meaning);
            if matches!(finding.category,"interface_loss"|"worker_failure"|"unconfirmed") {ui.colored_label(WARNING,line);} else {ui.small(line);}
        }
        ui.collapsing("Exact saved interface counters",|ui| {match toml::to_string_pretty(data) {Ok(text)=>{ui.monospace(text);},Err(error)=>{ui.colored_label(WARNING,error.to_string());}}});
    });
}

pub(super) fn show_review(ui: &mut Ui, review: &InterfaceReview) {
    ui.small(review.basis);
    let context = &review.acquisition;
    ui.label(format!(
        "Configured {:?} · recorded route {} · received {} · {} logger overruns",
        context.configured_gateway,
        context.recorded_route.as_deref().unwrap_or("unavailable"),
        context
            .recorded_received
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unavailable".into()),
        context.recorded_logger_overruns
    ));
    ui.collapsing("Recorded sampling timing", |ui| {
        ui.label(context.cycle_context.text());
        let timing = &context.recorded_timing;
        ui.label(format!(
            "Port cycle {} µs · raw {}",
            timing
                .port_cycle_us
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".into()),
            timing.port_cycle_raw.as_deref().unwrap_or("unavailable")
        ));
        for (name, spacing) in [
            ("Host poll starts", timing.host_poll_start_spacing.as_ref()),
            (
                "Host receive completions",
                timing.host_receive_spacing.as_ref(),
            ),
            (
                "Observed new-update receives",
                timing.host_new_update_spacing.as_ref(),
            ),
            (
                "Gateway update timestamps",
                timing.gateway_update_spacing.as_ref(),
            ),
        ] {
            super::review::spacing(ui, name, spacing);
        }
        ui.small(
            "These are saved acquisition measurements, not a timing test performed by this review.",
        );
    });
    crate::desktop::logger_metrics::show(ui, context.recorded_timing.logger_metrics.as_ref());
    show(ui, &review.interfaces);
    ui.collapsing("Reviewed metadata hashes",|ui| {
        ui.monospace(format!("capture.toml SHA256 {}\nprofile.toml SHA256 {}",review.capture_report_sha256,review.saved_profile_sha256));
        ui.small("These identify reviewed metadata bytes; they do not attest acquisition origin or consumer delivery.");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_and_recorded_interface_diagnostics_render_without_input_or_io() {
        let context = eframe::egui::Context::default();
        context.memory_mut(|memory| memory.set_everything_is_visible(true));
        for recorded in [false, true] {
            let data = RecordedInterfaces {
                basis: "synthetic render fixture; no capture or interface opened",
                absence_meaning: "absence remains unknown",
                local_api_status: if recorded { "recorded" } else { "not_recorded" },
                mqtt_status: "not_recorded",
                shared_memory_status: "not_recorded",
                offer_counts_checked_against_acquisition: recorded,
                local_api: recorded.then(|| iolink_workbench::local_api::Summary {
                    format_version: 1,
                    url: "http://127.0.0.1:9".into(),
                    session_id: "render-fixture".into(),
                    queue_capacity: 2,
                    history_capacity: 2,
                    events_offered: 4,
                    events_ingested: 2,
                    queue_dropped: 2,
                    history_evicted: 0,
                    requests: 1,
                    failed_requests: 0,
                    worker_failed: false,
                }),
                mqtt: None,
                shared_memory: None,
                findings: Vec::new(),
            };
            let input = eframe::egui::RawInput::default();
            assert!(input.events.is_empty());
            let output = context.run(input, |ctx| {
                eframe::egui::CentralPanel::default().show(ctx, |ui| show(ui, &data));
            });
            assert!(!output.shapes.is_empty());
            assert_eq!(data.local_api.is_some(), recorded);
        }
    }
}
