//! Resolve the selected IODD condition offline, then optionally read its selector.
use super::{ParameterReading, Status, parameter_target, read_parameter_observed};
use crate::profile::Profile;
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub layout: String,
    pub declaration: String,
    pub parameter_id: String,
    pub parameter_name: String,
    pub index: u16,
    pub subindex: u8,
    pub expected: u8,
}

impl Plan {
    pub fn from_profile(profile: &Profile) -> Result<Self> {
        let imported = profile
            .iodd
            .as_ref()
            .context("select an IODD layout first")?;
        let declaration = imported
            .condition
            .as_deref()
            .context("the selected IODD layout is unconditional")?;
        // Import already stores this grammar in older profiles.
        let (selector, value) = declaration
            .split_once(" = ")
            .context("expected a saved IODD condition in the form ID[subindex] = value")?;
        let (id, subindex) = if let Some((id, index)) = selector
            .strip_suffix(']')
            .and_then(|selector| selector.rsplit_once('['))
        {
            (
                id,
                index
                    .trim()
                    .parse::<u8>()
                    .context("invalid IODD condition subindex")?,
            )
        } else {
            (selector, 0)
        };
        ensure!(
            !id.is_empty() && !id.contains(['[', ']']) && !id.chars().any(char::is_whitespace),
            "invalid IODD condition parameter ID"
        );
        let expected = value
            .trim()
            .parse::<u8>()
            .context("IODD condition value must be 0–255")?;
        let parameter = parameter_target(profile, id, subindex)
            .with_context(|| format!("cannot resolve the condition selector {selector}"))?;
        let limit = match (parameter.datatype.as_str(), parameter.bits) {
            ("BooleanT", Some(1)) => 2,
            ("UIntegerT", Some(bits @ 2..=64)) => 1u128 << bits,
            ("IntegerT", Some(bits @ 2..=64)) => 1u128 << (bits - 1),
            _ => bail!("layout checks require a readable Boolean or integer selector"),
        };
        ensure!(
            u128::from(expected) < limit,
            "IODD condition value exceeds the selector width"
        );
        Ok(Self {
            layout: imported.layout.clone(),
            declaration: declaration.into(),
            parameter_id: id.into(),
            parameter_name: parameter.name.clone(),
            index: parameter.index,
            subindex,
            expected,
        })
    }

    fn compare(&self, raw_value: Option<&str>) -> Outcome {
        let value = match raw_value {
            Some("true") => Some(1),
            Some("false") => Some(0),
            Some(value) => value.parse::<i128>().ok(),
            None => None,
        };
        match value {
            Some(value) if value == i128::from(self.expected) => Outcome::Matched,
            Some(_) => Outcome::Mismatch,
            None => Outcome::Unavailable,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Matched,
    Mismatch,
    Unavailable,
}

#[derive(Debug, Serialize)]
pub struct Observation {
    pub outcome: Outcome,
    pub plan: Plan,
    pub observed: Status,
    pub reading: ParameterReading,
}

impl Observation {
    pub fn text(&self) -> String {
        let mut text = format!(
            "{} · expected {} · observed {}\nRaw bytes {}\n{} · port {} · {}",
            match self.outcome {
                Outcome::Matched => "Condition matched",
                Outcome::Mismatch => "Condition mismatch",
                Outcome::Unavailable => "Selector value unavailable",
            },
            self.plan.expected,
            self.reading.raw_value.as_deref().unwrap_or("unavailable"),
            self.reading.raw_hex,
            self.observed.model,
            self.observed.port,
            self.reading.host_read_utc,
        );
        if let Some(error) = &self.reading.decode_error {
            text.push_str(&format!("\n{error}"));
        }
        text
    }
}

/// Perform one explicit acyclic read using the current profile and its identity checks.
pub fn read(profile: &Profile) -> Result<Observation> {
    let plan = Plan::from_profile(profile)?;
    let (reading, observed) = read_parameter_observed(profile, &plan.parameter_id, plan.subindex)?;
    Ok(Observation {
        outcome: plan.compare(reading.raw_value.as_deref()),
        plan,
        observed,
        reading,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{iodd, profile::Gateway};

    fn with_condition(xml: &str, rule: &str) -> Result<Profile> {
        let source = xml.replace(
            r#"<ProcessData id="PD">"#,
            &format!(r#"<ProcessData id="PD"><Condition {rule}/>"#,),
        );
        let catalog = iodd::parse(&source, "synthetic conditional IODD".into())?;
        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), Some("IN"))?;
        let mut profile = Profile::default();
        profile.apply_iodd(identity, imported);
        Profile::parse(&toml::to_string(&profile)?)
    }

    #[test]
    fn plans_use_saved_conditions_and_compare_unscaled_values() -> Result<()> {
        let profile = with_condition(
            include_str!("../../samples/sensor.xml"),
            r#"variableId="V_SIM_Threshold" value="1""#,
        )?;
        let before = profile.clone();
        let plan = Plan::from_profile(&profile)?;
        assert_eq!((plan.index, plan.subindex, plan.expected), (64, 0, 1));
        assert_eq!(plan.compare(Some("1")), Outcome::Matched);
        assert_eq!(plan.compare(Some("2.1")), Outcome::Unavailable);
        assert_eq!(plan.compare(Some("-1")), Outcome::Mismatch);
        let observed = read(&profile)?;
        assert_eq!(observed.outcome, Outcome::Mismatch);
        assert_eq!(observed.reading.raw_value.as_deref(), Some("500"));
        assert_eq!(observed.observed.port, profile.connection.port);
        assert_eq!(profile, before);
        assert!(!profile.confirm_iodd_condition);
        Ok(())
    }

    #[test]
    fn record_member_reads_respect_access_and_boolean_domain() -> Result<()> {
        let mut profile = with_condition(
            include_str!("../../samples/sensor-record.xml"),
            r#"variableId="V_SIM_Record" subindex="2" value="1""#,
        )?;
        let observed = read(&profile)?;
        assert_eq!(observed.outcome, Outcome::Matched);
        assert_eq!(observed.reading.raw_value.as_deref(), Some("true"));
        assert_eq!((observed.plan.index, observed.plan.subindex), (66, 2));

        profile.connection.gateway = Gateway::IfmAl1350;
        profile.connection.endpoint = "unavailable".into();
        let imported = profile.iodd.as_mut().unwrap();
        imported.condition = Some("V_SIM_Record[2] = 2".into());
        assert!(
            Plan::from_profile(&profile)
                .unwrap_err()
                .to_string()
                .contains("width")
        );
        let imported = profile.iodd.as_mut().unwrap();
        imported.condition = Some("V_SIM_Record[256] = 1".into());
        assert!(format!("{:#}", read(&profile).unwrap_err()).contains("subindex"));
        let imported = profile.iodd.as_mut().unwrap();
        imported.condition = Some("V_SIM_Record[2] = 1".into());
        imported
            .parameters
            .iter_mut()
            .find(|parameter| parameter.id == "V_SIM_Record")
            .unwrap()
            .record
            .as_mut()
            .unwrap()
            .subindex_access = false;
        assert!(format!("{:#}", read(&profile).unwrap_err()).contains("does not permit"));
        Ok(())
    }
}
