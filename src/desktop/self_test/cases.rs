use super::super::{App, Tab, configure_context};
use anyhow::{Result, ensure};
use eframe::egui::{self, Context};
use iolink_workbench::{
    iodd,
    profile::{Gateway, Profile},
};
use serde::Serialize;

pub(super) const SIZE: [u32; 2] = [1130, 3200];

#[derive(Serialize)]
pub(super) struct Case {
    pub profile: &'static str,
    pub tab: &'static str,
    pub width: u32,
    pub height: u32,
    pub passes: u8,
    pub shapes: usize,
    pub meshes: usize,
    pub triangles: usize,
}

fn profiles() -> Result<Vec<(&'static str, Profile)>> {
    let mut profiles = vec![(
        "bundled-simulator",
        Profile::parse(include_str!("../../../samples/simulator.toml"))?,
    )];
    for (name, xml) in [
        (
            "formats",
            include_str!("../../../samples/sensor-formats.xml"),
        ),
        (
            "records",
            include_str!("../../../samples/sensor-record.xml"),
        ),
        ("arrays", include_str!("../../../samples/sensor-array.xml")),
        (
            "strings",
            include_str!("../../../samples/sensor-strings.xml"),
        ),
        ("time", include_str!("../../../samples/sensor-time.xml")),
        ("values", include_str!("../../../samples/sensor-values.xml")),
        (
            "defaults",
            include_str!("../../../samples/sensor-defaults.xml"),
        ),
    ] {
        let catalog = iodd::parse(xml, format!("built-in synthetic GUI self-test {name}"))?;
        let (expected_identity, imported) = catalog.select(None, None)?;
        profiles.push((
            name,
            Profile {
                input_bytes: imported.input_bytes,
                expected_identity,
                iodd: Some(imported),
                ..Default::default()
            },
        ));
    }
    Ok(profiles)
}

pub(super) fn render(
    ctx: &Context,
    mut paint: impl FnMut([u32; 2], &egui::FullOutput, &[egui::ClippedPrimitive]) -> Result<()>,
) -> Result<Vec<Case>> {
    configure_context(ctx);
    // Exercise collapsed parameter/diagnostic/review controls without input events.
    ctx.memory_mut(|memory| memory.set_everything_is_visible(true));
    let mut cases = Vec::new();
    for (name, profile) in profiles()? {
        ensure!(
            profile.connection.gateway == Gateway::Simulator,
            "GUI self-test profile must stay offline"
        );
        for (tab, label) in [
            (Tab::Setup, "setup"),
            (Tab::Monitor, "monitor"),
            (Tab::Offline, "offline"),
        ] {
            let mut app = App::new();
            app.profile = profile.clone();
            app.setup.sync(&app.profile);
            if tab == Tab::Setup
                && let Some((field, origin)) = app.profile.effective_fields().first()
            {
                app.setup.edit_field((*field).clone(), *origin != "IODD");
            }
            app.tab = tab;
            // Render enabled API controls without starting acquisition or a listener.
            app.api_enabled = tab == Tab::Monitor;
            let before = app.profile.clone();
            let mut case = Case {
                profile: name,
                tab: label,
                width: SIZE[0],
                height: SIZE[1],
                passes: 0,
                shapes: 0,
                meshes: 0,
                triangles: 0,
            };
            for pass in 0..2 {
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
                    )),
                    time: Some(cases.len() as f64 + f64::from(pass) / 60.0),
                    focused: false,
                    ..Default::default()
                };
                input
                    .viewports
                    .entry(egui::ViewportId::ROOT)
                    .or_default()
                    .native_pixels_per_point = Some(1.0);
                ensure!(
                    input.events.is_empty(),
                    "self-test must not inject input events"
                );
                let output = ctx.run(input, |ctx| app.render(ctx));
                ensure!(
                    output.pixels_per_point == 1.0,
                    "self-test requires nominal one-pixel-per-point rendering"
                );
                let shapes = output.shapes.len();
                let primitives = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
                let mut meshes = 0;
                let mut triangles = 0;
                for primitive in &primitives {
                    match &primitive.primitive {
                        egui::epaint::Primitive::Mesh(mesh) => {
                            ensure!(mesh.is_valid(), "GUI produced an invalid mesh");
                            meshes += 1;
                            triangles += mesh.indices.len() / 3;
                        }
                        egui::epaint::Primitive::Callback(_) => {
                            anyhow::bail!("unexpected native callback in offscreen GUI self-test")
                        }
                    }
                }
                ensure!(
                    shapes > 0 && meshes > 0 && triangles > 0,
                    "GUI view produced no drawable geometry"
                );
                ensure!(
                    app.profile == before,
                    "rendering changed the applied profile"
                );
                ensure!(
                    app.worker.is_none() && app.isdu_worker.is_none() && !app.setup.busy(),
                    "rendering dispatched a worker"
                );
                paint(SIZE, &output, &primitives)?;
                case.passes += 1;
                case.shapes += shapes;
                case.meshes += meshes;
                case.triangles += triangles;
            }
            cases.push(case);
        }
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_built_in_views_render_without_input_or_profile_changes() -> Result<()> {
        let cases = render(&Context::default(), |_, _, _| Ok(()))?;
        assert_eq!(cases.len(), 24);
        assert!(
            cases
                .iter()
                .all(|case| case.passes == 2 && case.triangles > 0)
        );
        Ok(())
    }
}
