//! Validate recorded timing without deriving source timing from filtered CSV rows.
use super::Manifest;
use crate::transport::Freshness;
use anyhow::{Context, Result, ensure};

pub(super) fn validate(manifest: &Manifest) -> Result<()> {
    for (name, spacing, observations, host_clock) in [
        (
            "host_poll_start_spacing",
            &manifest.host_poll_start_spacing,
            manifest.counts.poll_attempts.map(|v| v.saturating_sub(1)),
            true,
        ),
        (
            "host_receive_spacing",
            &manifest.host_receive_spacing,
            manifest.counts.received.map(|v| v.saturating_sub(1)),
            true,
        ),
        (
            "host_new_update_spacing",
            &manifest.host_new_update_spacing,
            None,
            true,
        ),
        // A device clock is independent of host elapsed time. Never apply the host bound.
        (
            "gateway_update_spacing",
            &manifest.gateway_update_spacing,
            None,
            false,
        ),
    ] {
        let Some(spacing) = spacing else {
            continue;
        };
        spacing.validate().context(name)?;
        if let Some(expected) = observations {
            ensure!(
                spacing.count == expected,
                "{name} observation count differs from acquisition counters"
            );
        }
        if host_clock {
            ensure!(
                spacing.longest_us <= manifest.elapsed_us,
                "{name} exceeds the host acquisition window"
            );
        }
    }
    for (name, spacing) in [
        ("host_new_update_spacing", &manifest.host_new_update_spacing),
        ("gateway_update_spacing", &manifest.gateway_update_spacing),
    ] {
        let Some(spacing) = spacing else {
            continue;
        };
        ensure!(
            manifest
                .last_status
                .as_ref()
                .is_none_or(|s| s.freshness != Freshness::Snapshots),
            "{name} cannot describe source updates for snapshot acquisition"
        );
        if let Some(updates) = manifest.counts.new_updates_observed {
            ensure!(
                spacing.count <= updates.saturating_sub(1),
                "{name} has more intervals than recorded new source updates"
            );
        }
    }
    if let Some(gateway) = &manifest.gateway_update_spacing {
        ensure!(
            gateway.p50_us > 0,
            "gateway_update_spacing contains a nonpositive source-clock interval"
        );
        if let Some(host) = &manifest.host_new_update_spacing {
            ensure!(
                gateway.count <= host.count,
                "gateway_update_spacing has more intervals than observed new-update receives"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
