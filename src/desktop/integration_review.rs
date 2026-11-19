//! Build and check the next session's feeds before starting any services.
use super::{App, Tab, WARNING, workspace::IntegrationPage};
use anyhow::{Context, Result};
use eframe::egui::Ui;
use iolink_workbench::{capture::Options, local_api, mqtt, shared_memory};
use std::net::{Ipv4Addr, SocketAddrV4};

pub(super) struct FeedReview {
    http: Result<Option<local_api::Config>>,
    mqtt: Result<Option<mqtt::Config>>,
    shared: Result<Option<shared_memory::Config>>,
}

impl FeedReview {
    pub fn into_options(self) -> Result<Options> {
        Ok(Options {
            local_api: self.http.context("Local HTTP API settings")?,
            mqtt: self.mqtt.context("MQTT settings")?,
            shared_memory: self.shared.context("Shared memory settings")?,
            ..Default::default()
        })
    }

    pub fn valid(&self) -> bool {
        self.http.is_ok() && self.mqtt.is_ok() && self.shared.is_ok()
    }

    pub fn enabled(&self) -> bool {
        !(matches!(self.http, Ok(None))
            && matches!(self.mqtt, Ok(None))
            && matches!(self.shared, Ok(None)))
    }

    pub fn issue(&self, page: IntegrationPage) -> Option<&anyhow::Error> {
        match page {
            IntegrationPage::Api => self.http.as_ref().err(),
            IntegrationPage::Mqtt => self.mqtt.as_ref().err(),
            IntegrationPage::SharedMemory => self.shared.as_ref().err(),
        }
    }

    pub fn show_destinations(&self, ui: &mut Ui) {
        if !self.enabled() {
            ui.small("No software feeds enabled for this session.");
            return;
        }
        if let Ok(Some(config)) = &self.http {
            ui.label(format!(
                "Local HTTP API · http://{}/v1/status",
                config.listen
            ));
        }
        if let Ok(Some(config)) = &self.mqtt {
            ui.label(format!(
                "MQTT · {} · topic {:?} · {} ms",
                config.broker, config.topic, config.interval_ms
            ));
        }
        if let Ok(Some(config)) = &self.shared {
            ui.label(format!(
                "Shared memory · {} · {} ms",
                config.mapping_name(),
                config.interval_ms
            ));
        }
    }
}

impl App {
    pub(super) fn review_feeds(&self) -> FeedReview {
        let http = self.api_enabled.then(|| {
            let config = local_api::Config {
                listen: SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.api_port).into(),
                ..Default::default()
            };
            config.validate()?;
            Ok(config)
        });
        let mqtt = self.mqtt_enabled.then(|| {
            let config = mqtt::Config {
                broker: self
                    .mqtt_broker
                    .trim()
                    .parse()
                    .map_err(|_| anyhow::anyhow!("Enter a numeric MQTT broker IP:PORT"))?,
                topic: self.mqtt_topic.clone(),
                interval_ms: self.mqtt_interval_ms,
                ..Default::default()
            };
            config.validate()?;
            Ok(config)
        });
        let shared = self.shared_memory_enabled.then(|| {
            let config = shared_memory::Config {
                name: self.shared_memory_name.clone(),
                interval_ms: self.shared_memory_interval_ms,
                ..Default::default()
            };
            config.validate()?;
            Ok(config)
        });
        FeedReview {
            http: http.transpose(),
            mqtt: mqtt.transpose(),
            shared: shared.transpose(),
        }
    }

    pub(super) fn show_feed_issues(&mut self, ui: &mut Ui, review: &FeedReview) {
        for (page, button) in [
            (IntegrationPage::Api, "Edit local HTTP API settings"),
            (IntegrationPage::Mqtt, "Edit MQTT settings"),
            (IntegrationPage::SharedMemory, "Edit shared memory settings"),
        ] {
            if let Some(error) = review.issue(page) {
                ui.colored_label(WARNING, format!("{error:#}"));
                if ui.small_button(button).clicked() {
                    self.tab = Tab::Integration;
                    self.workspace.integration = page;
                    self.workspace.connect_dialog = false;
                }
            }
        }
    }
}
