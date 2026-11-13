use super::{IODD_NAMESPACE, child, label};
use anyhow::{Context, Result, ensure};
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    Notification,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventDefinition {
    pub code: u16,
    pub event_type: EventType,
    pub name: String,
    pub description: Option<String>,
    pub origin: Option<String>,
}

impl EventDefinition {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.name.trim().is_empty(), "event name is empty");
        Ok(())
    }
}

pub(super) fn parse(
    function: Node<'_, '_>,
    texts: &HashMap<&str, &str>,
) -> Result<(Vec<EventDefinition>, Vec<u16>)> {
    let mut events = Vec::new();
    let mut references = Vec::new();
    let mut codes = HashSet::new();
    for event in child(function, "EventCollection")
        .into_iter()
        .flat_map(|node| node.children())
    {
        if !event.has_tag_name((IODD_NAMESPACE, "Event"))
            && !event.has_tag_name((IODD_NAMESPACE, "StdEventRef"))
        {
            continue;
        }
        let code = event
            .attribute("code")
            .context("event has no code")?
            .parse()?;
        ensure!(codes.insert(code), "duplicate event code {code}");
        if event.has_tag_name((IODD_NAMESPACE, "StdEventRef")) {
            references.push(code);
            continue;
        }
        let event_type = match event.attribute("type") {
            Some("Notification") => EventType::Notification,
            Some("Warning") => EventType::Warning,
            Some("Error") => EventType::Error,
            _ => anyhow::bail!("event {code} has an invalid type"),
        };
        let definition = EventDefinition {
            code,
            event_type,
            name: label(event, texts, &format!("Event 0x{code:04X}")),
            description: child(event, "Description")
                .and_then(|node| node.attribute("textId"))
                .map(|id| texts.get(id).copied().unwrap_or(id).to_owned()),
            origin: None,
        };
        definition.validate()?;
        events.push(definition);
    }
    Ok((events, references))
}
