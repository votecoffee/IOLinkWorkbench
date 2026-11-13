use eframe::egui::Ui;
use iolink_workbench::{
    archive::{IdentityReview, RecordedStatus},
    profile::Identity,
};

pub(super) fn show(ui: &mut Ui, data: &IdentityReview) {
    ui.collapsing("Recorded master and device identity", |ui| {
        ui.small(data.basis);
        ui.label(format!(
            "Offline accepted-identity check · {}",
            data.accepted_recheck
        ));
        ui.label(format!(
            "Producer's saved check · {}",
            data.producer_check.as_deref().unwrap_or("unavailable")
        ));
        ui.label(format!("Recorded reconnects · {}", data.reconnects));
        ui.collapsing("Configured expectations", |ui| {
            fields(ui, &data.expected, "not pinned")
        });
        status(ui, "Last accepted connection", data.last_accepted.as_ref());
        status(
            ui,
            "Rejected connection — excluded from input",
            data.rejected.as_ref(),
        );
        for note in &data.notes {
            ui.small(*note);
        }
    });
}

fn fields(ui: &mut Ui, data: &Identity, missing: &str) {
    for (name, value) in [
        (
            "Vendor ID",
            data.vendor_id.map(|v| format!("{v} / 0x{v:04X}")),
        ),
        (
            "Device ID",
            data.device_id.map(|v| format!("{v} / 0x{v:06X}")),
        ),
        (
            "Product ID",
            data.product_id.as_ref().map(|v| format!("{v:?}")),
        ),
        (
            "Product name",
            data.product_name.as_ref().map(|v| format!("{v:?}")),
        ),
        (
            "Device serial",
            data.serial.as_ref().map(|v| format!("{v:?}")),
        ),
    ] {
        ui.label(format!("{name} · {}", value.as_deref().unwrap_or(missing)));
    }
}

fn status(ui: &mut Ui, heading: &str, data: Option<&RecordedStatus>) {
    ui.collapsing(heading, |ui| {
        let Some(data) = data else {
            ui.label("Not recorded");
            return;
        };
        for (name, value) in [
            ("Route", &data.route),
            ("Master model", &data.model),
            ("Firmware", &data.firmware),
            ("Master serial", &data.master_serial),
            ("Port mode", &data.port_mode),
            ("Device status", &data.device_status),
            ("Port diagnostic", &data.port_event),
            ("Device diagnostic", &data.device_event),
        ] {
            ui.label(format!(
                "{name} · {}",
                value
                    .as_ref()
                    .map(|v| format!("{v:?}"))
                    .unwrap_or_else(|| "unavailable".into())
            ));
        }
        ui.label(format!(
            "Port · {} · freshness {:?}",
            data.port
                .map(|p| p.to_string())
                .unwrap_or_else(|| "unavailable".into()),
            data.freshness
        ));
        match &data.identity {
            Some(identity) => fields(ui, identity, "unavailable"),
            None => {
                ui.label("Device identity unavailable");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_partial_and_rejected_saved_identity_render_without_input_or_io() {
        let ctx = eframe::egui::Context::default();
        ctx.memory_mut(|memory| memory.set_everything_is_visible(true));
        for actual in [
            None,
            Some(Identity {
                vendor_id: Some(0),
                device_id: Some(1),
                serial: Some(String::new()),
                ..Default::default()
            }),
        ] {
            let status = RecordedStatus {
                freshness: iolink_workbench::transport::Freshness::Snapshots,
                route: Some("synthetic render fixture".into()),
                model: None,
                firmware: Some(String::new()),
                master_serial: None,
                port: Some(1),
                port_mode: None,
                device_status: None,
                port_event: Some("saved diagnostic".into()),
                device_event: None,
                port_cycle_raw: None,
                port_cycle_us: None,
                identity: actual,
            };
            let data = IdentityReview {
                basis: "synthetic offline UI fixture",
                expected: Identity::default(),
                accepted_recheck: "recorded identity unavailable".into(),
                producer_check: None,
                last_accepted: Some(status.clone()),
                rejected: Some(status),
                reconnects: 2,
                notes: vec!["No per-generation identity history is implied"],
            };
            let input = eframe::egui::RawInput::default();
            assert!(input.events.is_empty());
            let output = ctx.run(input, |ctx| {
                eframe::egui::CentralPanel::default().show(ctx, |ui| show(ui, &data));
            });
            assert!(!output.shapes.is_empty());
        }
    }
}
