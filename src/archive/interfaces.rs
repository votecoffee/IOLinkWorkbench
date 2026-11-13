//! Interpretation of finalized saved interface counters; never access live interfaces.
use super::{Manifest, RecordedTiming, Summary};
use crate::profile::{Gateway, LogMode, Profile};
use anyhow::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Serialize)]
pub struct InterfaceFinding {
    pub component: &'static str,
    pub code: &'static str,
    pub category: &'static str,
    pub count: Option<u64>,
    pub meaning: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RecordedInterfaces {
    pub basis: &'static str,
    pub absence_meaning: &'static str,
    pub local_api_status: &'static str,
    pub mqtt_status: &'static str,
    pub shared_memory_status: &'static str,
    pub offer_counts_checked_against_acquisition: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_api: Option<crate::local_api::Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mqtt: Option<crate::mqtt::Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_memory: Option<crate::shared_memory::Summary>,
    pub findings: Vec<InterfaceFinding>,
}

#[derive(Debug, Serialize)]
pub struct InterfaceContext {
    pub configured_gateway: Gateway,
    pub recorded_route: Option<String>,
    pub simulation_faults: Option<bool>,
    pub simulated_writer_delay_ms: Option<u64>,
    pub elapsed_us: u64,
    pub log_mode: LogMode,
    pub requested_poll_us: Option<u64>,
    pub stale_threshold_us: u64,
    pub recorded_received: Option<u64>,
    pub recorded_disconnects: Option<u64>,
    pub recorded_reconnects: u64,
    pub recorded_logger_overruns: u64,
    pub recorded_timing: RecordedTiming,
    pub cycle_context: crate::timing::Context,
}

#[derive(Debug, Serialize)]
pub struct InterfaceReview {
    pub format_version: u32,
    pub basis: &'static str,
    pub capture_report_sha256: String,
    pub saved_profile_sha256: String,
    pub capture: Summary,
    pub acquisition: InterfaceContext,
    pub interfaces: RecordedInterfaces,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<super::IdentityReview>,
}

impl InterfaceReview {
    pub(super) fn new(
        capture: Summary,
        profile: &Profile,
        manifest: &Manifest,
        profile_bytes: &[u8],
        manifest_bytes: &[u8],
    ) -> Result<Self> {
        let status = manifest.last_status.as_ref();
        Ok(Self {
            format_version: 1,
            basis: "offline consistency review of saved report/profile and every CSV row; metrics are recorded claims; no gateway, broker, HTTP service or shared-memory object accessed; delivery and physical timing are not remeasured",
            capture_report_sha256: format!("{:x}", Sha256::digest(manifest_bytes)),
            saved_profile_sha256: format!("{:x}", Sha256::digest(profile_bytes)),
            capture,
            acquisition: InterfaceContext {
                configured_gateway: profile.connection.gateway,
                recorded_route: status.and_then(|s| s.route.clone()),
                simulation_faults: manifest.simulation_faults,
                simulated_writer_delay_ms: manifest.simulated_writer_delay_ms,
                elapsed_us: manifest.elapsed_us,
                log_mode: profile.logging.mode,
                requested_poll_us: manifest.requested_poll_us,
                stale_threshold_us: manifest.stale_threshold_us,
                recorded_received: manifest.counts.received,
                recorded_disconnects: manifest.counts.disconnects,
                recorded_reconnects: manifest.counts.reconnects,
                recorded_logger_overruns: manifest.counts.logger_overruns,
                recorded_timing: RecordedTiming {
                    port_cycle_raw: status.and_then(|s| s.port_cycle_raw.clone()),
                    port_cycle_us: status.and_then(|s| s.port_cycle_us),
                    host_poll_start_spacing: manifest.host_poll_start_spacing.clone(),
                    host_receive_spacing: manifest.host_receive_spacing.clone(),
                    host_new_update_spacing: manifest.host_new_update_spacing.clone(),
                    gateway_update_spacing: manifest.gateway_update_spacing.clone(),
                    logger_metrics: manifest.logger_metrics.clone(),
                },
                cycle_context: crate::timing::assess(
                    profile,
                    manifest.requested_poll_us,
                    crate::timing::Observation {
                        port_cycle_us: status.and_then(|s| s.port_cycle_us),
                        freshness: status.map(|s| s.freshness),
                        route: status.and_then(|s| s.route.as_deref()),
                    },
                    1000,
                )?,
            },
            interfaces: RecordedInterfaces::new(manifest),
            identity: super::IdentityReview::new(manifest, profile),
        })
    }
}

impl RecordedInterfaces {
    pub fn any_recorded(&self) -> bool {
        self.local_api.is_some() || self.mqtt.is_some() || self.shared_memory.is_some()
    }
    pub(super) fn new(manifest: &Manifest) -> Self {
        let status = |present| if present { "recorded" } else { "not_recorded" };
        let mut result = Self {
            basis: "final saved interface counters; separate from source gaps, CSV selection and logger overruns; no combined loss total or delivery confirmation",
            absence_meaning: "not_recorded means no saved metrics; disabled and older/absent producer data cannot be distinguished",
            local_api_status: status(manifest.local_api.is_some()),
            mqtt_status: status(manifest.mqtt.is_some()),
            shared_memory_status: status(manifest.shared_memory.is_some()),
            offer_counts_checked_against_acquisition: super::expected_interface_offers(manifest)
                .is_some()
                && (manifest.local_api.is_some()
                    || manifest.mqtt.is_some()
                    || manifest.shared_memory.is_some()),
            local_api: manifest.local_api.clone(),
            mqtt: manifest.mqtt.clone(),
            shared_memory: manifest.shared_memory.clone(),
            findings: Vec::new(),
        };
        let mut findings = Vec::new();
        let mut add = |component, code, category, count, meaning| {
            findings.push(InterfaceFinding {
                component,
                code,
                category,
                count,
                meaning,
            })
        };
        if let Some(api) = &result.local_api {
            if api.queue_dropped > 0 {
                add(
                    "local_api",
                    "api_queue_drops",
                    "interface_loss",
                    Some(api.queue_dropped),
                    "API handoff events dropped; not a source-update loss count",
                );
            }
            if api.history_evicted > 0 {
                add(
                    "local_api",
                    "api_history_evictions",
                    "retention",
                    Some(api.history_evicted),
                    "Old history events replaced; individual clients' missed observations are unknown",
                );
            }
            if api.failed_requests > 0 {
                add(
                    "local_api",
                    "api_request_failures",
                    "diagnostic",
                    Some(api.failed_requests),
                    "Rejected or failed HTTP requests; not source or CSV loss",
                );
            }
            if api.worker_failed {
                add(
                    "local_api",
                    "api_worker_failure",
                    "worker_failure",
                    None,
                    "API worker failure was recorded; internal worker accounting may be incomplete",
                );
            }
            add(
                "local_api",
                "api_client_delivery_unknown",
                "unknown",
                None,
                "Request counters do not prove which input events any client received",
            );
        }
        if let Some(mqtt) = &result.mqtt {
            for (count, code, category, meaning) in [
                (
                    mqtt.queue_dropped,
                    "mqtt_queue_drops",
                    "interface_loss",
                    "MQTT handoff events dropped; separate from source and CSV loss",
                ),
                (
                    mqtt.coalesced,
                    "mqtt_coalesced",
                    "intentional_selection",
                    "Pending MQTT states intentionally replaced by newer ingested state",
                ),
                (
                    mqtt.shutdown_discarded,
                    "mqtt_shutdown_discarded",
                    "interface_loss",
                    "A pending state was discarded at shutdown without a connection",
                ),
                (
                    mqtt.publish_write_failures,
                    "mqtt_write_uncertain",
                    "unconfirmed",
                    "Failed or partial socket writes have unknown delivery; not a confirmed subscriber loss count",
                ),
                (
                    mqtt.encoding_failures,
                    "mqtt_encoding_failures",
                    "interface_loss",
                    "MQTT payload encoding failed",
                ),
                (
                    mqtt.connect_failures,
                    "mqtt_connect_failures",
                    "diagnostic",
                    "Broker connection/handshake failures; independent of IO-Link reconnects",
                ),
                (
                    mqtt.connection_losses,
                    "mqtt_connection_losses",
                    "diagnostic",
                    "Established broker connections lost; source generations remain independent",
                ),
            ] {
                if count > 0 {
                    add("mqtt", code, category, Some(count), meaning);
                }
            }
            if mqtt.broker_connections == 0 {
                add(
                    "mqtt",
                    "mqtt_no_broker_connection",
                    "unconfirmed",
                    Some(0),
                    "No accepted broker connection was recorded",
                );
            }
            if mqtt.socket_writes_completed == 0 {
                add(
                    "mqtt",
                    "mqtt_no_complete_socket_writes",
                    "unconfirmed",
                    Some(0),
                    "No complete MQTT socket write was recorded; partial delivery may still be unknown",
                );
            }
            if mqtt.worker_failed {
                add(
                    "mqtt",
                    "mqtt_worker_failure",
                    "worker_failure",
                    None,
                    "MQTT worker failure was recorded; internal worker accounting may be incomplete",
                );
            }
            add(
                "mqtt",
                "mqtt_subscriber_delivery_unknown",
                "unknown",
                None,
                "QoS 0 socket completion confirms no broker/subscriber delivery; no replay or acknowledgment",
            );
        }
        if let Some(shm) = &result.shared_memory {
            for (count, code, category, meaning) in [
                (
                    shm.queue_dropped,
                    "shm_queue_drops",
                    "interface_loss",
                    "Shared-memory handoff events dropped; not a source-update loss count",
                ),
                (
                    shm.coalesced,
                    "shm_coalesced",
                    "intentional_selection",
                    "Pending shared-memory states intentionally replaced before a commit attempt",
                ),
                (
                    shm.busy_skips,
                    "shm_busy_skips",
                    "interface_loss",
                    "Commit attempts skipped because the named mutex was busy; acquisition did not wait for this mutex",
                ),
                (
                    shm.snapshots_replaced,
                    "shm_snapshot_replacements",
                    "retention",
                    "Commits after the first replaced the latest snapshot; this does not prove reader loss",
                ),
                (
                    shm.encoding_failures,
                    "shm_encoding_failures",
                    "interface_loss",
                    "Snapshot JSON encoding failed",
                ),
                (
                    shm.write_failures,
                    "shm_write_failures",
                    "unconfirmed",
                    "Shared-memory commit failures were recorded",
                ),
                (
                    shm.abandoned_recoveries,
                    "shm_abandoned_recoveries",
                    "diagnostic",
                    "Publisher recovered abandoned mutex ownership with a complete new commit",
                ),
            ] {
                if count > 0 {
                    add("shared_memory", code, category, Some(count), meaning);
                }
            }
            if shm.snapshots_written == 0 {
                add(
                    "shared_memory",
                    "shm_no_commits",
                    "unconfirmed",
                    Some(0),
                    "No successful shared-memory snapshot commit was recorded",
                );
            }
            if !shm.final_snapshot_written {
                add(
                    "shared_memory",
                    "shm_final_state_unconfirmed",
                    "unconfirmed",
                    None,
                    "An inactive final snapshot was not confirmed; retained bytes can still say active, so readers must check age and publisher identity/liveness",
                );
            }
            if shm.worker_failed {
                add(
                    "shared_memory",
                    "shm_worker_failure",
                    "worker_failure",
                    None,
                    "Shared-memory worker failure was recorded; internal worker accounting may be incomplete",
                );
            }
            add(
                "shared_memory",
                "shm_reader_delivery_unknown",
                "unknown",
                None,
                "Commit/overwrite counters do not prove which snapshots any reader observed",
            );
        }
        result.findings = findings;
        result
    }
}

#[cfg(test)]
mod tests;
