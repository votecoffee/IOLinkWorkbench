//! Offline host settings. These methods never connect or configure a master.
use super::{Connection, Gateway, Profile};
use anyhow::{Context, Result, ensure};
use reqwest::Url;

impl Connection {
    pub(crate) fn origin(&self) -> Result<Url> {
        let url = Url::parse(&self.endpoint).context("enter an explicit http(s) AL1350 origin")?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none(),
            "AL1350 endpoint must be an http(s) origin without path or credentials"
        );
        Ok(url)
    }
}

impl Profile {
    /// Build a validated candidate without mutating this profile or opening an endpoint.
    /// Blank endpoints remain permitted for unfinished bench templates.
    pub fn with_connection(&self, mut connection: Connection) -> Result<Self> {
        connection.endpoint = connection.endpoint.trim().to_owned();
        connection.ifm_firmware = connection.ifm_firmware.trim().to_owned();
        if connection.gateway == Gateway::IfmAl1350 && !connection.endpoint.is_empty() {
            connection.origin()?;
        }
        let mut candidate = self.clone();
        candidate.connection = connection;
        candidate.validate()?;
        Ok(candidate)
    }
}

#[cfg(test)]
mod tests;
