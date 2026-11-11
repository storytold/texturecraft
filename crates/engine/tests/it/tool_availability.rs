//! Which tools can act on each selection, and that every usable tool acts there in both views

use serde_json::{Value, json};
use texturecraft_engine::core::{Effect, EffectKind, LayerId};
use texturecraft_engine::geom::glam::{Vec2, Vec3};
use texturecraft_engine::{Session, StackTarget, Tool, execute};

/// Two triangles side by side, each a mesh part with its own corner of the UV square
const PARTS_OBJ: &str = "o Lid\nv 0 0 0\nv 2 0 0\nv 0 2 0\nvt 0 0\nvt 0.5 0\nvt 0 0.5\nf 1/1 2/2 3/3\n\
o Body\nv 2 2 0\nv 4 2 0\nv 2 4 0\nvt 0.5 0.5\nvt 1 0.5\nvt 0.5 1\nf 4/4 5/5 6/6\n";

const VIEWPORT_PIXELS: f32 = 400.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Selection {
    PaintContent,
    FillContent,
    PaintMask,
    FillMask,
    FolderMask,
    InstanceOwnMask,
    PaintSublayer,
    PaintSublayerInMask,
    PaintSublayerInFill,
    PathSublayer,
    FilterInPaint,
    FilterInFill,
    FilterInMask,
    FillEffectInPaint,
    GeneratorInFillMask,
    FolderContent,
    InstanceOfPaint,
    InstanceOfFill,
    Nothing,
    QuickMaskOverPaint,
    QuickMaskOverFill,
    QuickMaskOverNothing,
}

impl Selection {
    const ALL: [Selection; 22] = [
        Selection::PaintContent,
        Selection::FillContent,
        Selection::PaintMask,
        Selection::FillMask,
        Selection::FolderMask,
        Selection::InstanceOwnMask,
        Selection::PaintSublayer,
        Selection::PaintSublayerInMask,
        Selection::PaintSublayerInFill,
        Selection::PathSublayer,
        Selection::FilterInPaint,
        Selection::FilterInFill,
        Selection::FilterInMask,
        Selection::FillEffectInPaint,
        Selection::GeneratorInFillMask,
        Selection::FolderContent,
        Selection::InstanceOfPaint,
        Selection::InstanceOfFill,
        Selection::Nothing,
        Selection::QuickMaskOverPaint,
        Selection::QuickMaskOverFill,
        Selection::QuickMaskOverNothing,
    ];

    /// Whether the shelf offers `tool` for this selection
    fn offers(self, tool: Tool) -> bool {
        let reads_content = matches!(tool, Tool::Smudge | Tool::Clone);
        match tool {
            Tool::ColorPicker => true,
            Tool::GeometryMask => !matches!(self, Selection::Nothing | Selection::QuickMaskOverNothing),
            _ => match self {
                Selection::PaintContent | Selection::PaintSublayer => true,
                Selection::PaintSublayerInFill => tool != Tool::Path,
                Selection::PaintMask | Selection::FillMask | Selection::FolderMask | Selection::InstanceOwnMask | Selection::PaintSublayerInMask => {
                    !reads_content
                }
                Selection::PathSublayer => tool == Tool::Path,
                Selection::QuickMaskOverPaint | Selection::QuickMaskOverFill | Selection::QuickMaskOverNothing => tool != Tool::Path,
                _ => false,
            },
        }
    }

    /// Greyed in the shelf, but scripts still paint here: below the effect, or the instance's source
    fn scripts_still_paint(self) -> bool {
        matches!(
            self,
            Selection::FilterInPaint | Selection::FillEffectInPaint | Selection::FilterInMask | Selection::GeneratorInFillMask | Selection::InstanceOfPaint
        )
    }
}

fn stencil_png() -> Vec<u8> {
    let mut out = Vec::new();
    let green = image::RgbaImage::from_fn(8, 8, |_, _| image::Rgba([0, 255, 0, 255]));
    green.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn base_session() -> Session {
    let mut session = Session::from_mesh_bytes("parts.obj", PARTS_OBJ.as_bytes(), 256, "parts").unwrap();
    session.viewport = Vec2::splat(VIEWPORT_PIXELS);
    (session.camera.yaw, session.camera.pitch) = (0.0, 0.0);
    let (low, high) = session.mesh.bounds();
    session.camera.frame(low, high);
    session.update();

    execute(&mut session, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 0.0, 0.0]})).unwrap();
    execute(&mut session, "tool.brush", &json!({"size": 6})).unwrap();
    let stencil = session.import_resource_bytes("stencil.png", stencil_png()).unwrap();
    execute(&mut session, "tool.stencil", &json!({"image": stencil})).unwrap();
    session
}

fn screen(session: &Session, point: [f32; 3]) -> Vec2 {
    let ndc = session.camera.project(Vec3::from(point), 1.0).unwrap();
    Vec2::new((ndc.x + 1.0) * VIEWPORT_PIXELS / 2.0, (1.0 - ndc.y) * VIEWPORT_PIXELS / 2.0)
}

fn add_layer(session: &mut Session, command: &str) -> u64 {
    execute(session, command, &json!({})).unwrap()["layer"].as_u64().unwrap()
}

fn select(session: &mut Session, layer: u64, target: Value) {
    execute(session, "layer.select", &json!({"layer": layer, "target": target})).unwrap();
}

fn select_nothing(session: &mut Session) {
    let set = session.ts();
    session.selected.remove(&set);
    session.target = StackTarget::Content;
}

/// Adds an effect with `command` to the layer's content or mask and selects it
fn select_new_effect(session: &mut Session, layer: u64, mask: bool, command: &str, params: Value) {
    if mask {
        execute(session, "mask.add.black", &json!({"layer": layer})).unwrap();
    }

    let target = if mask { "mask" } else { "content" };
    let mut params = params;
    params["layer"] = json!(layer);
    params["target"] = json!(target);
    execute(session, command, &params).unwrap();

    let owner = session.doc.project.layer(LayerId(layer)).unwrap();
    let effects = if mask { &owner.mask.as_ref().unwrap().effects } else { &owner.content_effects };
    let effect = effects.last().unwrap().id.0;
    select(session, layer, json!(effect));
}

fn select_new_mask(session: &mut Session, layer: u64) {
    execute(session, "mask.add.black", &json!({"layer": layer})).unwrap();
    select(session, layer, json!("mask"));
}

fn session_with(selection: Selection) -> Session {
    let mut session = base_session();
    match selection {
        Selection::PaintContent => {
            let layer = add_layer(&mut session, "layer.add.paint");
            select(&mut session, layer, json!("content"));
        }
        Selection::FillContent => {
            let layer = add_layer(&mut session, "layer.add.fill");
            select(&mut session, layer, json!("content"));
        }
        Selection::PaintMask => {
            let layer = add_layer(&mut session, "layer.add.paint");
            select_new_mask(&mut session, layer);
        }
        Selection::FillMask => {
            let layer = add_layer(&mut session, "layer.add.fill");
            select_new_mask(&mut session, layer);
        }
        Selection::FolderMask => {
            let folder = add_layer(&mut session, "layer.add.group");
            select_new_mask(&mut session, folder);
        }
        Selection::InstanceOwnMask => {
            let source = add_layer(&mut session, "layer.add.paint");
            let instance = execute(&mut session, "layer.instance", &json!({"layer": source})).unwrap()["layer"].as_u64().unwrap();
            select_new_mask(&mut session, instance);
        }
        Selection::PaintSublayer => {
            let layer = add_layer(&mut session, "layer.add.paint");
            select_new_effect(&mut session, layer, false, "effect.add.paint", json!({}));
        }
        Selection::PaintSublayerInMask => {
            let layer = add_layer(&mut session, "layer.add.paint");
            select_new_effect(&mut session, layer, true, "effect.add.paint", json!({}));
        }
        Selection::PaintSublayerInFill => {
            let layer = add_layer(&mut session, "layer.add.fill");
            select_new_effect(&mut session, layer, false, "effect.add.paint", json!({}));
        }
        Selection::PathSublayer => {
            let layer = add_layer(&mut session, "layer.add.paint");
            select(&mut session, layer, json!("content"));
            let (start, end) = (screen(&session, [0.2, 0.2, 0.0]), screen(&session, [1.0, 0.4, 0.0]));
            execute(&mut session, "tool.path", &json!({"points": [[start.x, start.y], [end.x, end.y]]})).unwrap();
            execute(&mut session, "tool.path", &json!({"commit": true})).unwrap();
            let path = session.doc.project.layer(LayerId(layer)).unwrap().content_effects.last().unwrap().id.0;
            select(&mut session, layer, json!(path));
        }
        Selection::FilterInPaint | Selection::FilterInFill | Selection::FilterInMask => {
            let command = if selection == Selection::FilterInFill { "layer.add.fill" } else { "layer.add.paint" };
            let layer = add_layer(&mut session, command);
            select_new_effect(&mut session, layer, selection == Selection::FilterInMask, "effect.add.filter", json!({"filter": "invert"}));
        }
        Selection::FillEffectInPaint => {
            let layer = add_layer(&mut session, "layer.add.paint");
            select_new_effect(&mut session, layer, false, "effect.add.fill", json!({}));
        }
        Selection::GeneratorInFillMask => {
            let layer = add_layer(&mut session, "layer.add.fill");
            select_new_effect(&mut session, layer, true, "effect.add.generator", json!({}));
        }
        Selection::FolderContent => {
            let folder = add_layer(&mut session, "layer.add.group");
            select(&mut session, folder, json!("content"));
        }
        Selection::InstanceOfPaint | Selection::InstanceOfFill => {
            let command = if selection == Selection::InstanceOfFill { "layer.add.fill" } else { "layer.add.paint" };
            let source = add_layer(&mut session, command);
            let instance = execute(&mut session, "layer.instance", &json!({"layer": source})).unwrap()["layer"].as_u64().unwrap();
            select(&mut session, instance, json!("content"));
        }
        Selection::Nothing => select_nothing(&mut session),
        Selection::QuickMaskOverPaint | Selection::QuickMaskOverFill => {
            let command = if selection == Selection::QuickMaskOverFill { "layer.add.fill" } else { "layer.add.paint" };
            let layer = add_layer(&mut session, command);
            select(&mut session, layer, json!("content"));
            execute(&mut session, "viewport.quickMask", &json!({"enabled": true})).unwrap();
            execute(&mut session, "viewport.quickMask.edit", &json!({"editing": true})).unwrap();
        }
        Selection::QuickMaskOverNothing => {
            select_nothing(&mut session);
            execute(&mut session, "viewport.quickMask", &json!({"enabled": true})).unwrap();
            execute(&mut session, "viewport.quickMask.edit", &json!({"editing": true})).unwrap();
        }
    }
    session
}

/// Uses `tool` once the way the viewport does: a stroke, a click or a committed path
fn use_tool(session: &mut Session, tool: Tool, in_2d_view: bool) -> texturecraft_engine::Result<()> {
    session.tool.tool = tool;
    if tool == Tool::Clone {
        let source = screen(session, [1.2, 0.3, 0.0]);
        execute(session, "tool.cloneSource", &json!({"x": source.x, "y": source.y}))?;
    }

    let points = [screen(session, [0.3, 0.3, 0.0]), screen(session, [0.6, 0.4, 0.0]), screen(session, [0.9, 0.5, 0.0])];
    let uv_points = [Vec2::new(0.05, 0.95), Vec2::new(0.1, 0.92), Vec2::new(0.15, 0.9)];
    if tool == Tool::Path {
        let params = if in_2d_view {
            json!({"uvPoints": uv_points.iter().map(|uv| [uv.x, uv.y]).collect::<Vec<_>>()})
        } else {
            json!({"points": points.iter().map(|point| [point.x, point.y]).collect::<Vec<_>>()})
        };
        execute(session, "tool.path", &params)?;
        execute(session, "tool.path", &json!({"commit": true}))?;
        return Ok(());
    }

    let single_click = matches!(tool, Tool::PolygonFill | Tool::ColorPicker | Tool::GeometryMask);
    let viewport = session.viewport;
    if in_2d_view {
        session.begin_stroke_uv(uv_points[0], 1.0)?;
    } else {
        session.begin_stroke(points[0], viewport, 1.0)?;
    }

    if single_click {
        return Ok(());
    }

    for (point, uv) in points.iter().zip(&uv_points).skip(1) {
        if in_2d_view {
            session.continue_stroke_uv(*uv, 1.0);
        } else {
            session.continue_stroke(*point, 1.0);
        }
    }
    session.end_stroke()
}

#[test]
fn every_selection_offers_the_tools_that_can_act_on_it() {
    let mut wrong = Vec::new();
    for selection in Selection::ALL {
        let session = session_with(selection);
        for tool in Tool::ALL {
            let usable = session.tool_usable(tool);
            if usable != selection.offers(tool) {
                wrong.push(format!("{selection:?} {tool:?}: usable {usable}"));
            }
        }
    }

    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn every_usable_tool_acts_in_both_views() {
    let mut wrong = Vec::new();
    for in_2d_view in [false, true] {
        for selection in Selection::ALL {
            for tool in Tool::ALL {
                // The picker and the geometry mask read the 3D view only, whatever is selected
                if in_2d_view && matches!(tool, Tool::ColorPicker | Tool::GeometryMask) {
                    continue;
                }

                let mut session = session_with(selection);
                let usable = session.tool_usable(tool);

                let result = use_tool(&mut session, tool, in_2d_view);

                let view = if in_2d_view { "2D" } else { "3D" };
                match result {
                    Err(error) if usable => wrong.push(format!("{view} {selection:?} {tool:?}: usable but {error}")),
                    Ok(()) if !usable && !selection.scripts_still_paint() => wrong.push(format!("{view} {selection:?} {tool:?}: greyed but acts")),
                    _ => {}
                }
            }
        }
    }

    assert!(wrong.is_empty(), "{wrong:#?}");
}

fn commit_path(session: &mut Session) -> texturecraft_engine::Result<Value> {
    execute(session, "tool.path", &json!({"uvPoints": [[0.05, 0.95], [0.15, 0.9]], "commit": true}))
}

fn path_count(session: &Session, path_sublayer: LayerId) -> usize {
    match session.doc.project.effect(path_sublayer).map(|(_, effect)| &effect.kind) {
        Some(EffectKind::Path(paths)) => paths.len(),
        _ => 0,
    }
}

#[test]
fn paths_are_refused_on_fill_layers_and_the_quick_mask_but_saved_ones_stay_editable() {
    let mut session = session_with(Selection::FillContent);
    let fill = session.selected_layer().unwrap();

    assert!(commit_path(&mut session).is_err(), "no new paths in a fill layer");
    assert!(session.doc.project.layer(fill).unwrap().content_effects.is_empty());

    let mut session = session_with(Selection::QuickMaskOverPaint);
    assert!(commit_path(&mut session).is_err(), "paths can't edit the quick mask");

    // A project saved while paths could go into fill layers
    let mut session = session_with(Selection::FillContent);
    let fill = session.selected_layer().unwrap();
    let saved_path = session.doc.project.alloc_id();
    session.doc.project.layer_mut(fill).unwrap().content_effects.push(Effect::new(saved_path, EffectKind::Path(Vec::new())));
    select(&mut session, fill.0, json!(saved_path.0));

    assert!(session.tool_usable(Tool::Path));

    commit_path(&mut session).unwrap();

    assert_eq!(path_count(&session, saved_path), 1, "the saved path sublayer takes the new path");
}

#[test]
fn quick_mask_edit_and_invert_need_the_mask_on() {
    let mut session = base_session();

    assert!(execute(&mut session, "viewport.quickMask.edit", &json!({"editing": true})).is_err());
    assert!(execute(&mut session, "viewport.quickMask.invert", &json!({})).is_err());

    execute(&mut session, "viewport.quickMask", &json!({"enabled": true})).unwrap();
    execute(&mut session, "viewport.quickMask.edit", &json!({"editing": true})).unwrap();
    execute(&mut session, "viewport.quickMask", &json!({"enabled": false})).unwrap();

    assert!(!session.quick_mask.editing, "turning the mask off stops editing it");
}
