//! Each `tool.variant.*` command sets its tool and setting, and `tool.variants` reports what the settings say

use serde_json::{Value, json};
use texturecraft_engine::core::Channel;
use texturecraft_engine::geom::glam::{Vec2, Vec3, Vec4};
use texturecraft_engine::geom::{Mesh, primitives};
use texturecraft_engine::tool_variant::ToolVariant;
use texturecraft_engine::{Session, Tool, execute};

const RES: f32 = 256.0;
const SQUARE_UV: [[f32; 2]; 4] = [[0.3, 0.3], [0.7, 0.3], [0.7, 0.7], [0.3, 0.7]];
// Wide enough that its middle stays clear of the line its first two points paint
const TRIANGLE_UV: [[f32; 2]; 3] = [[0.1, 0.1], [0.9, 0.1], [0.5, 0.9]];

fn session() -> Session {
    session_on(primitives::sphere(1.0, 48, 24))
}

/// A 2×2 wall facing the camera (+Z): particles run down it
fn wall() -> Mesh {
    let mut m = primitives::plane(2.0, 8);
    for p in &mut m.positions {
        *p = Vec3::new(p.x, -p.z, 0.0);
    }
    m.normals.iter_mut().for_each(|n| *n = Vec3::Z);
    m.tangents.iter_mut().for_each(|t| *t = Vec4::new(1.0, 0.0, 0.0, 1.0));
    m
}

fn session_on(mesh: Mesh) -> Session {
    let mut s = Session::new_project("t", mesh, "builtin:test", None, 256).unwrap();
    s.viewport = Vec2::new(400.0, 400.0);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;
    s.update();
    execute(&mut s, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut s, "tool.brush", &json!({"size": 6, "hardness": 90})).unwrap();
    s
}

fn red_uv(s: &Session, u: f32, v: f32) -> bool {
    let c = s.caches[0].composite.value(Channel::BaseColor, (u * RES) as u32, (v * RES) as u32);
    c[0] > 0.85 && c[1] < 0.15
}

fn red_texels(s: &Session) -> usize {
    (0..RES as u32).flat_map(|y| (0..RES as u32).map(move |x| (x, y))).filter(|&(x, y)| red_uv(s, x as f32 / RES, y as f32 / RES)).count()
}

fn variant_of(s: &mut Session, tool: &str) -> String {
    let variants = execute(s, "tool.variants", &Value::Null).unwrap();
    variants[tool].as_str().unwrap_or_default().to_string()
}

#[test]
fn each_variant_command_sets_its_tool_and_setting() {
    let mut s = session();
    for variant in ToolVariant::ALL {
        execute(&mut s, "tool.colorPicker", &Value::Null).unwrap();

        let result = execute(&mut s, variant.command_id(), &Value::Null).unwrap();

        assert_eq!(s.tool.tool, variant.tool(), "{} selects its tool", variant.id());
        assert_eq!(ToolVariant::current(&s.tool, variant.tool()), Some(variant), "{} sets its setting", variant.id());
        assert_eq!(result["variant"], variant.id());
    }

    // The settings the variants map onto, read directly
    execute(&mut s, "tool.variant.physicalEraser", &Value::Null).unwrap();
    assert!(s.tool.brush.particles.enabled);
    execute(&mut s, "tool.variant.cloneAbsolute", &Value::Null).unwrap();
    assert!(!s.tool.clone_aligned);
    execute(&mut s, "tool.variant.pathFilled", &Value::Null).unwrap();
    assert!(s.tool.path.closed && s.tool.path.fill);

    // A plain tool pick keeps the variant the tool is in
    execute(&mut s, "tool.variant.physicalPaint", &Value::Null).unwrap();
    execute(&mut s, "tool.eraser", &Value::Null).unwrap();
    execute(&mut s, "tool.paint", &Value::Null).unwrap();
    assert!(s.tool.brush.particles.enabled, "tool.paint keeps particles on");
}

#[test]
fn variants_query_follows_settings() {
    let mut s = session();
    assert_eq!(variant_of(&mut s, "paint"), "paint");
    assert_eq!(variant_of(&mut s, "clone"), "cloneRelative");

    execute(&mut s, "tool.brush", &json!({"particles": {"enabled": true}, "cloneAligned": false})).unwrap();
    execute(&mut s, "tool.path", &json!({"closed": true, "fill": true})).unwrap();

    for (tool, variant) in
        [("paint", "physicalPaint"), ("eraser", "physicalEraser"), ("projection", "physicalProjection"), ("clone", "cloneAbsolute"), ("path", "pathFilled")]
    {
        assert_eq!(variant_of(&mut s, tool), variant, "{tool}");
    }
    assert!(execute(&mut s, "tool.variants", &Value::Null).unwrap().get("smudge").is_none(), "a tool without variants is not listed");
}

#[test]
fn physical_paint_stroke_differs_from_plain() {
    let painted = |variant: &str| {
        let mut s = session_on(wall());
        execute(&mut s, "tool.brush", &json!({"size": 10, "particles": {"count": 6, "lifetime": 40, "gravity": 1.5}})).unwrap();
        execute(&mut s, variant, &Value::Null).unwrap();
        execute(&mut s, "tool.stroke", &json!({"points": [[200, 200]]})).unwrap();
        red_texels(&s)
    };

    let plain = painted("tool.variant.paint");
    let physical = painted("tool.variant.physicalPaint");

    assert!(plain > 0);
    assert!(physical > plain, "particles paint trails beyond the dab: {physical} vs {plain}");
}

#[test]
fn filled_path_is_one_point_undo_step() {
    let mut s = session();
    execute(&mut s, "tool.path", &json!({"uvPoints": SQUARE_UV})).unwrap();

    execute(&mut s, "tool.variant.pathFilled", &Value::Null).unwrap();
    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert!(!s.tool.path.closed && !s.tool.path.fill, "one undo restores both flags");
    assert_eq!(s.tool.path.points.len(), SQUARE_UV.len(), "the points stay");
}

#[test]
fn paint_along_path_keeps_a_closed_outline_closed() {
    let mut s = session();
    execute(&mut s, "tool.path", &json!({"uvPoints": SQUARE_UV, "closed": true})).unwrap();

    execute(&mut s, "tool.variant.pathFilled", &Value::Null).unwrap();
    execute(&mut s, "tool.variant.pathAlong", &Value::Null).unwrap();

    assert!(s.tool.path.closed && !s.tool.path.fill);
}

#[test]
fn path_variant_repaints_an_edited_stored_path() {
    let mut s = session();
    let report = execute(&mut s, "tool.path", &json!({"uvPoints": SQUARE_UV, "closed": true, "commit": true})).unwrap();
    assert!(!red_uv(&s, 0.5, 0.5), "a stroked outline leaves the middle unpainted");

    execute(&mut s, "path.edit", &json!({"effect": report["committed"]["effect"], "index": 0})).unwrap();
    execute(&mut s, "tool.variant.pathFilled", &Value::Null).unwrap();

    assert!(red_uv(&s, 0.5, 0.5), "the edited path fills at once");
}

#[test]
fn filled_path_survives_commit() {
    let mut s = session();
    execute(&mut s, "tool.variant.pathFilled", &Value::Null).unwrap();

    execute(&mut s, "tool.path", &json!({"uvPoints": SQUARE_UV, "commit": true})).unwrap();
    assert_eq!(variant_of(&mut s, "path"), "pathFilled", "Apply keeps the variant");
    execute(&mut s, "tool.path", &json!({"uvPoints": [[0.75, 0.75], [0.95, 0.75], [0.85, 0.95]], "commit": true})).unwrap();

    assert!(red_uv(&s, 0.85, 0.82), "the next path fills without picking Filled path again");
    execute(&mut s, "tool.path", &json!({"uvPoints": SQUARE_UV})).unwrap();
    execute(&mut s, "tool.path", &json!({"clear": true})).unwrap();
    assert_eq!(variant_of(&mut s, "path"), "pathFilled", "cancelling keeps it too");
}

#[test]
fn removing_a_point_keeps_filled() {
    let mut s = session();
    execute(&mut s, "tool.variant.pathFilled", &Value::Null).unwrap();
    execute(&mut s, "tool.path", &json!({"uvPoints": [[0.3, 0.3], [0.7, 0.3], [0.5, 0.7]]})).unwrap();

    s.remove_selected_path_point();

    assert_eq!(s.tool.path.points.len(), 2);
    assert_eq!(variant_of(&mut s, "path"), "pathFilled");
}

#[test]
fn a_filled_preview_goes_below_three_points() {
    let mut s = session();
    execute(&mut s, "tool.variant.pathFilled", &Value::Null).unwrap();
    execute(&mut s, "tool.path", &json!({"uvPoints": TRIANGLE_UV})).unwrap();
    assert!(red_uv(&s, 0.5, 0.5), "three points preview the fill");

    s.remove_selected_path_point();
    s.path_live().unwrap();
    s.path_live().unwrap();
    s.update();

    assert!(!red_uv(&s, 0.5, 0.5), "two points fill nothing");
    assert_eq!(s.tool.path.points.len(), 2, "the points stay");
    assert_eq!(variant_of(&mut s, "path"), "pathFilled");
}

#[test]
fn picking_filled_path_over_a_two_point_preview_keeps_the_points() {
    let mut s = session();
    execute(&mut s, "tool.path", &json!({"uvPoints": [[0.3, 0.3], [0.7, 0.3]]})).unwrap();

    execute(&mut s, "tool.variant.pathFilled", &Value::Null).unwrap();
    s.path_live().unwrap();
    s.path_live().unwrap();
    s.update();

    assert_eq!(s.tool.path.points.len(), 2);
    assert!(!red_uv(&s, 0.5, 0.3), "the open line's preview goes");
}

#[test]
fn removing_a_point_of_a_stored_filled_path_repaints_it_open() {
    let mut s = session();
    let report = execute(&mut s, "tool.path", &json!({"uvPoints": TRIANGLE_UV, "closed": true, "fill": true, "commit": true})).unwrap();
    execute(&mut s, "path.edit", &json!({"effect": report["committed"]["effect"], "index": 0})).unwrap();

    s.remove_selected_path_point();
    s.path_live().unwrap();
    s.update();

    assert!(!red_uv(&s, 0.5, 0.5), "the old fill is repainted away");
    assert!(red_uv(&s, 0.5, 0.1), "the two points paint as an open line");
}

#[test]
fn a_filled_path_opened_by_hand_stays_open() {
    let mut s = session();
    execute(&mut s, "tool.variant.pathFilled", &Value::Null).unwrap();
    execute(&mut s, "tool.path", &json!({"closed": false})).unwrap();

    execute(&mut s, "tool.path", &json!({"uvPoints": SQUARE_UV, "commit": true})).unwrap();

    assert_eq!(variant_of(&mut s, "path"), "pathAlong");
    assert!(!s.tool.path.closed);
}

#[test]
fn paint_along_path_without_points_starts_an_open_path() {
    let mut s = session();
    execute(&mut s, "tool.variant.pathFilled", &Value::Null).unwrap();

    execute(&mut s, "tool.variant.pathAlong", &Value::Null).unwrap();

    assert!(!s.tool.path.closed);
}

#[test]
fn finishing_a_stored_edit_keeps_that_paths_variant() {
    let mut s = session();
    let report = execute(&mut s, "tool.path", &json!({"uvPoints": SQUARE_UV, "closed": true, "fill": true, "commit": true})).unwrap();
    execute(&mut s, "tool.variant.pathAlong", &Value::Null).unwrap();

    execute(&mut s, "path.edit", &json!({"effect": report["committed"]["effect"], "index": 0})).unwrap();
    assert_eq!(variant_of(&mut s, "path"), "pathFilled", "editing loads the stored path's variant");
    execute(&mut s, "tool.path", &json!({"commit": true})).unwrap();

    assert_eq!(s.tool.tool, Tool::Path);
    assert_eq!(variant_of(&mut s, "path"), "pathFilled", "the tool stays in it after the edit");
}
