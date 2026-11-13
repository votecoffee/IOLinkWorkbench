use super::Parameter;
use crate::{
    decode::Field,
    iodd::{IODD_NAMESPACE, child},
};
use roxmltree::Node;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    pub id: String,
    pub field: Field,
}

#[derive(Debug, Serialize)]
pub struct DisplayValue {
    pub id: String,
    pub value: String,
    pub unit: String,
}

pub(super) fn apply(function: Node<'_, '_>, parameters: &mut [Parameter]) {
    let Some(menus) = child(function, "UserInterface").and_then(|ui| child(ui, "MenuCollection"))
    else {
        return;
    };
    for menu in menus
        .children()
        .filter(|node| node.has_tag_name((IODD_NAMESPACE, "Menu")))
    {
        let menu_id = menu.attribute("id").unwrap_or("unnamed-menu");
        for (ordinal, reference) in menu
            .children()
            .filter(|node| {
                node.has_tag_name((IODD_NAMESPACE, "VariableRef"))
                    || node.has_tag_name((IODD_NAMESPACE, "RecordItemRef"))
            })
            .enumerate()
        {
            let Some(parameter) = parameters
                .iter_mut()
                .find(|p| Some(p.id.as_str()) == reference.attribute("variableId"))
            else {
                continue;
            };
            let parameter = if reference.has_tag_name((IODD_NAMESPACE, "RecordItemRef")) {
                let Some(subindex) = reference
                    .attribute("subindex")
                    .and_then(|value| value.parse::<u8>().ok())
                else {
                    parameter
                        .needs_input
                        .push("Menu record reference has no valid subindex".into());
                    continue;
                };
                let Some(item) = parameter.record.as_mut().and_then(|record| {
                    record
                        .items
                        .iter_mut()
                        .find(|item| item.subindex == subindex)
                }) else {
                    parameter.needs_input.push(format!(
                        "Menu reference to unresolved record subindex {subindex}"
                    ));
                    continue;
                };
                &mut item.parameter
            } else {
                parameter
            };
            if !["gradient", "offset", "unitCode", "displayFormat"]
                .iter()
                .any(|attr| reference.attribute(*attr).is_some())
            {
                continue;
            }
            let id = format!("{menu_id}/{}", ordinal + 1);
            let display = (|| -> anyhow::Result<Field> {
                let mut field = parameter.scalar_field()?;
                field.scale = reference.attribute("gradient").unwrap_or("1").parse()?;
                field.offset = reference.attribute("offset").unwrap_or("0").parse()?;
                if let Some(format) = reference.attribute("displayFormat") {
                    field.decimals = Some(
                        format
                            .strip_prefix("Dec.")
                            .ok_or_else(|| anyhow::anyhow!("unsupported display format {format}"))?
                            .parse()?,
                    );
                }
                field.unit = crate::iodd::unit(reference.attribute("unitCode"));
                field.validate(usize::from(field.bits).div_ceil(8))?;
                Ok(field)
            })();
            match display {
                Ok(field) => {
                    if reference.attribute("unitCode").is_none()
                        || field.unit.starts_with("IODD unit ")
                    {
                        parameter
                            .needs_input
                            .push(format!("Display {id} needs a confirmed unit"));
                    }
                    parameter.displays.push(Display { id, field });
                }
                Err(error) => parameter.needs_input.push(format!(
                    "Display {id} requires manual interpretation: {error}"
                )),
            }
        }
    }
    for parameter in parameters {
        mark_multiple(parameter);
        if let Some(record) = &mut parameter.record {
            for item in &mut record.items {
                mark_multiple(&mut item.parameter);
            }
        }
    }
}

fn mark_multiple(parameter: &mut Parameter) {
    if parameter.displays.len() > 1 {
        parameter.needs_input.push(
            "Multiple menu displays are retained separately; confirm which display applies".into(),
        );
    }
}
