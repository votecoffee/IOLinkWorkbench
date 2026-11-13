use super::*;

#[test]
fn drafts_gate_acquisition_and_discard_without_changing_the_applied_profile() {
    let profile = Profile::default();
    let mut editor = super::super::Editor::new(&profile);
    editor.mapping.new_field(&profile);
    assert!(editor.pending());
    assert!(profile.fields.is_empty());
    let candidate = profile
        .with_field(editor.mapping.draft.clone().unwrap())
        .unwrap();
    editor.sync(&candidate);
    assert!(!editor.pending());
    editor.mapping.draft.as_mut().unwrap().bits = 64;
    assert!(editor.pending());
    assert!(
        candidate
            .with_field(editor.mapping.draft.clone().unwrap())
            .is_err()
    );
    editor.sync(&candidate);
    assert!(!editor.pending());
}

#[test]
fn preview_requires_the_complete_payload_and_retains_signed_scaling() -> anyhow::Result<()> {
    let field = Field {
        name: "test".into(),
        bits: 12,
        kind: Kind::Signed,
        scale: 0.1,
        offset: 2.0,
        decimals: Some(3),
        ..Default::default()
    };
    let value = preview(&field, 2, "FFEF")?;
    assert_eq!((value.raw.as_str(), value.value.as_str()), ("-17", "0.300"));
    assert!(preview(&field, 2, "EF").is_err());
    assert!(preview(&field, 2, "zz").is_err());
    let little = Field {
        byte_order: ByteOrder::LittleEndian,
        ..field
    };
    assert_eq!(preview(&little, 2, "FFEF")?.raw, "-1");
    Ok(())
}

#[test]
fn new_field_and_invalid_type_controls_render_without_input_or_profile_edits() {
    let ctx = egui::Context::default();
    let mut profile = Profile::default();
    let before = profile.clone();
    let mut form = Form::default();
    form.new_field(&profile);
    form.draft.as_mut().unwrap().kind = Kind::Bool;
    form.preview = "0001".into();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1130.0, 1600.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(!form.show(ui, &mut profile, &mut String::new()));
            });
        },
    );
    assert!(!output.shapes.is_empty());
    assert!(form.pending());
    assert_eq!(profile, before);
}
