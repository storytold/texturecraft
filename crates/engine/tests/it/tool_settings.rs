//! Brush, symmetry and other tool settings outlive the project; what belongs to a project does not

use serde_json::json;
use texturecraft_engine::geom::glam::Vec3;
use texturecraft_engine::paint::BrushAlpha;
use texturecraft_engine::{CloneFrom, CloneSource, Session, ToolSettings, execute, sample_session};

fn another_project() -> Session {
    sample_session(128, 64).unwrap()
}

fn import_image(session: &mut Session, file: &str) -> String {
    let mut png_bytes = Vec::new();
    let image = image::RgbaImage::from_fn(4, 4, |_, _| image::Rgba([200, 100, 50, 255]));
    image.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png).unwrap();
    session.import_resource_bytes(file, png_bytes).unwrap()
}

/// A session with every kind of tool setting changed from its default
fn tuned() -> Session {
    let mut session = another_project();
    execute(
        &mut session,
        "tool.brush",
        &json!({
            "size": 33, "hardness": 80, "flow": 40, "angle": 15, "spacing": 30, "alpha": "star",
            "symmetryOn": true, "symmetryX": true, "symmetryY": false, "symmetryZ": true,
            "symmetryRadial": 8, "symmetryAxis": "x", "symmetryOffset": [0.1, 0.0, 0.2],
            "smudgeStrength": 80, "cloneSource": "layer", "cloneAligned": false,
        }),
    )
    .unwrap();
    session.tool.path.closed = true;
    session.tool.path.fill = true;
    session
}

#[test]
fn opening_another_project_keeps_the_brush_symmetry_and_tool_settings() {
    let mut session = tuned();
    let before = session.tool.clone();

    session.switch_to(another_project());

    assert_eq!(session.tool.brush, before.brush);
    assert_eq!(session.tool.brush.alpha, BrushAlpha::Star);
    assert!(session.tool.brush.symmetry.x && session.tool.brush.symmetry.z && !session.tool.brush.symmetry.y);
    assert_eq!(session.tool.smudge_strength, 0.8);
    assert_eq!(session.tool.clone_from, CloneFrom::Layer);
    assert!(!session.tool.clone_aligned, "the Clone tool's Aligned switch is kept too");
    assert!(session.tool.path.closed && session.tool.path.fill);
}

#[test]
fn a_project_loaded_from_a_file_gets_the_tool_settings_too() {
    let mut session = tuned();
    let before_size = session.tool.brush.size;
    let bytes = another_project().to_project_bytes().unwrap();

    session.switch_to(Session::from_project_bytes(&bytes).unwrap());

    assert_ne!(session.tool.brush.size, another_project().tool.brush.size);
    assert_eq!(session.tool.brush.size, before_size);
    assert_eq!(session.tool.smudge_strength, 0.8);
}

#[test]
fn what_belongs_to_the_old_project_is_reset() {
    let mut session = tuned();
    let before_size = session.tool.brush.size;
    let image = import_image(&mut session, "stamp.png");
    execute(&mut session, "tool.brush", &json!({"alpha": {"image": image.clone()}})).unwrap();
    execute(&mut session, "tool.stencil", &json!({"image": image, "scale": 2.0, "rotation": 30, "tile": true, "opacity": 0.9})).unwrap();
    session.tool.clone_source = Some(CloneSource { uv: [0.3, 0.4], offset: Some([0.1, 0.1]) });
    session.tool.path.push(Vec3::new(1.0, 2.0, 3.0), false);

    session.switch_to(another_project());

    assert_eq!(session.tool.clone_source, None);
    assert_eq!(session.tool.stencil, None);
    assert_eq!(session.tool.brush.alpha_image, None);
    assert!(session.tool.path.points.is_empty());

    // The rest of the brush came along
    assert_ne!(session.tool.brush.size, another_project().tool.brush.size);
    assert_eq!(session.tool.brush.size, before_size);
}

#[test]
fn the_stencil_placement_carries_over_to_the_next_stencil() {
    let mut session = another_project();
    let image = import_image(&mut session, "stamp.png");
    execute(&mut session, "tool.stencil", &json!({"image": image, "scale": 2.0, "rotation": 30, "tile": true, "opacity": 0.9})).unwrap();

    session.switch_to(another_project());
    let image = import_image(&mut session, "other.png");
    execute(&mut session, "tool.stencil", &json!({"image": image})).unwrap();

    let stencil = session.tool.stencil.as_ref().unwrap();
    assert_eq!((stencil.scale, stencil.rotation, stencil.tile, stencil.opacity), (2.0, 30.0, true, 0.9));
}

#[test]
fn clearing_the_stencil_keeps_its_placement_for_the_next_one() {
    let mut session = another_project();
    let image = import_image(&mut session, "stamp.png");
    execute(&mut session, "tool.stencil", &json!({"image": image.clone(), "scale": 2.0})).unwrap();
    execute(&mut session, "tool.stencil", &json!({"clear": true})).unwrap();

    execute(&mut session, "tool.stencil", &json!({"image": image})).unwrap();

    assert_eq!(session.tool.stencil.as_ref().unwrap().scale, 2.0);
}

#[test]
fn settings_survive_json_and_apply_to_a_fresh_tool() {
    let session = tuned();
    let saved = ToolSettings::of(&session.tool);
    let loaded: ToolSettings = serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();

    let mut fresh = another_project();
    loaded.apply(&mut fresh.tool);

    assert_eq!(ToolSettings::of(&fresh.tool), saved);
    assert_eq!(fresh.tool.brush.size, session.tool.brush.size);
}

#[test]
fn settings_saved_before_the_clone_aligned_switch_load_with_it_on() {
    let mut saved = serde_json::to_value(ToolSettings::of(&tuned().tool)).unwrap();
    saved.as_object_mut().unwrap().remove("clone_aligned");
    let loaded: ToolSettings = serde_json::from_value(saved).unwrap();
    assert!(loaded.clone_aligned);
}

#[test]
fn restored_stencil_and_smudge_settings_use_the_tool_limits() {
    let mut session = another_project();
    let mut settings = ToolSettings::default();
    settings.smudge_strength = 2.0;
    settings.stencil.center = [-20.0, 30.0];
    settings.stencil.opacity = -1.0;
    settings.stencil.scale = 0.0;
    settings.stencil.rotation = -15.0;
    settings.apply(&mut session.tool);

    assert_eq!(session.tool.smudge_strength, 1.0);
    let look = session.tool.stencil_look;
    assert_eq!((look.center, look.scale, look.opacity, look.rotation), ([-4.0, 4.0], 0.01, 0.0, 345.0));
}
