//! Wired communication declarations; these are not negotiated or measured values.
use super::IODD_NAMESPACE;
use anyhow::{Context, Result, ensure};
use roxmltree::Node;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Bitrate {
    #[serde(rename = "COM1")]
    Com1,
    #[serde(rename = "COM2")]
    Com2,
    #[serde(rename = "COM3")]
    Com3,
}

impl Bitrate {
    pub fn label(self) -> &'static str {
        match self {
            Self::Com1 => "COM1",
            Self::Com2 => "COM2",
            Self::Com3 => "COM3",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Communication {
    pub iolink_revision: String,
    pub compatible_with: Option<String>,
    pub bitrate: Bitrate,
    pub minimum_cycle_us: u32,
    pub sio_supported: bool,
    pub m_sequence_capability: u8,
}

impl Communication {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.iolink_revision == "V1.1",
            "only wired IO-Link V1.1 communication declarations are supported"
        );
        ensure!(
            self.compatible_with
                .as_deref()
                .is_none_or(|value| value == "V1.0"),
            "invalid IO-Link compatibility declaration"
        );
        let us = self.minimum_cycle_us;
        ensure!(
            (us <= 6300 && us.is_multiple_of(100))
                || ((6400..=31600).contains(&us) && (us - 6400).is_multiple_of(400))
                || ((32000..=132800).contains(&us) && (us - 32000).is_multiple_of(1600)),
            "IODD minCycleTime is outside the specified microsecond ranges/steps"
        );
        Ok(())
    }

    pub fn summary(&self) -> String {
        format!(
            "IODD declaration · {} · {} · minimum cycle {} µs · SIO {} · M-sequence {}",
            self.iolink_revision,
            self.bitrate.label(),
            self.minimum_cycle_us,
            self.sio_supported,
            self.m_sequence_capability
        )
    }
}

fn unique<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Result<Option<Node<'a, 'i>>> {
    let mut matching = node
        .children()
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, name)));
    let first = matching.next();
    ensure!(
        matching.next().is_none(),
        "duplicate IODD communication element {name}"
    );
    Ok(first)
}

pub(super) fn parse(root: Node<'_, '_>) -> Result<Option<Communication>> {
    let Some(network) = unique(root, "CommNetworkProfile")? else {
        return Ok(None);
    };
    let Some(layers) = unique(network, "TransportLayers")? else {
        return Ok(None);
    };
    let Some(physical) = unique(layers, "PhysicalLayer")? else {
        return Ok(None);
    };
    // Wireless/other transport representations are not interpreted as wired declarations.
    if network.attribute("iolinkRevision").is_none() {
        return Ok(None);
    }
    let bitrate = match physical.attribute("bitrate") {
        Some("COM1") => Bitrate::Com1,
        Some("COM2") => Bitrate::Com2,
        Some("COM3") => Bitrate::Com3,
        _ => anyhow::bail!("missing or unsupported IODD wired bitrate"),
    };
    let sio_supported = match physical.attribute("sioSupported").map(str::trim) {
        Some("true" | "1") => true,
        Some("false" | "0") => false,
        _ => anyhow::bail!("missing or invalid IODD sioSupported"),
    };
    let declaration = Communication {
        iolink_revision: network.attribute("iolinkRevision").unwrap().into(),
        compatible_with: network.attribute("compatibleWith").map(str::to_owned),
        bitrate,
        minimum_cycle_us: physical
            .attribute("minCycleTime")
            .context("missing IODD minCycleTime")?
            .trim()
            .parse()?,
        sio_supported,
        m_sequence_capability: physical
            .attribute("mSequenceCapability")
            .context("missing IODD mSequenceCapability")?
            .trim()
            .parse()?,
    };
    declaration.validate()?;
    Ok(Some(declaration))
}
