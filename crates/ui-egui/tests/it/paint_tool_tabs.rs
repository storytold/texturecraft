//! A paint tool's Properties: one page of sections under jump tabs, Brush · Alpha · (Stencil) · Material, below a brush preview that stays put

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use texturecraft_engine::execute;
use texturecraft_ui_egui::TextureCraftApp;

use crate::{BAKE_RESOLUTION, FRAME_SECONDS, STARTUP_FRAMES, TEXTURE_RESOLUTION};

/// Short enough that the tool's sections scroll
const WINDOW: egui::Vec2 = egui::vec2(1400.0, 800.0);

/// A second at 60 frames: a jump scrolls with egui's scroll animation
const SCROLL_FRAMES: usize = 60;

fn paint_harness(tool: &str) -> Harness<'static, TextureCraftApp> {
    let mut session = texturecraft_engine::sample_session(TEXTURE_RESOLUTION, BAKE_RESOLUTION).unwrap();
    execute(&mut session, "tool.select", &json!({"tool": tool})).unwrap();
    let mut harness = Harness::builder()
        .with_size(WINDOW)
        .with_step_dt(FRAME_SECONDS)
        .with_context_setup(texturecraft_ui_egui::framework::install_headless)
        .build_eframe(|cc| TextureCraftApp::new(cc, session));
    harness.state_mut().ui.prefs.language = "en".into();
    harness.run_steps(STARTUP_FRAMES);
    harness
}

/// egui reports a tab's selection as its toggled state
fn tab_selected(harness: &Harness<'_, TextureCraftApp>, tab: &str) -> bool {
    harness.get_by_role_and_label(Role::Tab, tab).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

#[test]
fn the_tabs_jump_within_one_page_of_the_tools_sections() {
    let mut harness = paint_harness("paint");
    for tab in ["Brush", "Alpha", "Stencil", "Material"] {
        assert!(harness.query_by_role_and_label(Role::Tab, tab).is_some(), "the {tab} tab");
    }
    assert!(tab_selected(&harness, "Brush"), "the page opens on its first section");
    let flow_before = harness.get_by_label("Flow").rect().top();

    harness.get_by_role_and_label(Role::Tab, "Material").click();
    harness.run_steps(SCROLL_FRAMES);

    assert!(tab_selected(&harness, "Material"), "the clicked section is the one in view");
    assert!(!tab_selected(&harness, "Brush"));
    assert!(harness.get_by_label("Flow").rect().top() < flow_before, "the Brush rows scrolled up out of the way, still on the page");
}

#[test]
fn every_brush_tool_has_a_stencil_tab_and_polygon_fill_none() {
    for tool in ["paint", "eraser", "projection", "smudge", "clone"] {
        let harness = paint_harness(tool);

        assert!(harness.query_by_role_and_label(Role::Tab, "Stencil").is_some(), "{tool}");
    }

    let harness = paint_harness("polygonFill");
    assert!(harness.query_by_role_and_label(Role::Tab, "Stencil").is_none(), "polygon fill paints no strokes");
}

#[test]
fn the_brush_preview_stays_under_the_tabs_while_the_sections_scroll() {
    let mut harness = paint_harness("paint");
    let preview_before = harness.get_by_label("Brush preview").rect();
    let flow_before = harness.get_by_label("Flow").rect().top();

    harness.get_by_role_and_label(Role::Tab, "Material").click();
    harness.run_steps(SCROLL_FRAMES);

    assert!(harness.get_by_label("Flow").rect().top() < flow_before, "the sections scrolled");
    assert_eq!(harness.get_by_label("Brush preview").rect(), preview_before, "the preview stayed where it was");
    assert!(preview_before.top() >= harness.get_by_role_and_label(Role::Tab, "Material").rect().bottom(), "under the tabs");
}

#[test]
fn the_brush_preview_keeps_its_picture_until_the_paint_colour_changes() {
    let mut harness = paint_harness("paint");
    let pictures = |harness: &Harness<'_, TextureCraftApp>| -> Vec<egui::TextureId> {
        harness.ctx.tex_manager().read().allocated().filter(|(_, texture)| texture.name == "brush_stroke_preview").map(|(id, _)| *id).collect()
    };
    let first = pictures(&harness);

    harness.run_steps(SCROLL_FRAMES);

    assert_eq!(pictures(&harness), first, "an unchanged brush reuses its picture");

    execute(&mut harness.state_mut().session, "tool.material", &json!({"channel": "baseColor", "color": [0.0, 0.0, 1.0]})).unwrap();
    harness.run_steps(SCROLL_FRAMES);
    let recoloured = pictures(&harness);
    assert_eq!(recoloured.len(), 1, "the old picture is released");
    assert_ne!(recoloured, first, "a new colour draws a new picture");
}

#[test]
fn the_brush_preview_ignores_values_of_disabled_channels() {
    let mut harness = paint_harness("paint");
    let preview_meshes = |harness: &Harness<'_, TextureCraftApp>| {
        let preview = harness.get_by_role_and_label(Role::Image, "Brush preview").rect();
        let mut meshes = harness.output().shapes.iter().filter_map(|clipped| match &clipped.shape {
            egui::Shape::Mesh(mesh) if preview.contains_rect(mesh.calc_bounds()) => Some(mesh),
            _ => None,
        });
        let stroke = meshes.find(|mesh| mesh.calc_bounds().right() <= preview.center().x).unwrap();
        let sphere = meshes.find(|mesh| mesh.calc_bounds().left() >= preview.center().x).unwrap();
        (stroke.texture_id, sphere.vertices.iter().map(|vertex| vertex.color).collect::<Vec<_>>())
    };

    for (channel, material) in [
        ("baseColor", json!({"channel": "baseColor", "color": [0.0, 0.0, 1.0]})),
        ("metallic", json!({"channel": "metallic", "value": 1.0})),
        ("roughness", json!({"channel": "roughness", "value": 0.0})),
    ] {
        execute(&mut harness.state_mut().session, "tool.material", &material).unwrap();
        harness.run_steps(1);
        let enabled = preview_meshes(&harness);

        execute(&mut harness.state_mut().session, "tool.material", &json!({"channel": channel, "enabled": false})).unwrap();
        harness.run_steps(1);
        let disabled = preview_meshes(&harness);

        assert!(disabled.1 != enabled.1, "disabling {channel} changes the sphere shading");
        if channel == "baseColor" {
            assert_ne!(disabled.0, enabled.0, "disabling base color redraws the tinted stroke");
        }

        execute(&mut harness.state_mut().session, "tool.material", &json!({"channel": channel, "enabled": true})).unwrap();
        harness.run_steps(1);

        assert!(preview_meshes(&harness).1 == enabled.1, "re-enabling {channel} restores its stored value in the sphere");
    }
}

#[test]
fn the_brush_preview_uses_neutral_coverage_while_editing_the_quick_mask() {
    let mut harness = paint_harness("paint");
    let stroke_texture = |harness: &Harness<'_, TextureCraftApp>| {
        let preview = harness.get_by_role_and_label(Role::Image, "Brush preview").rect();
        harness
            .output()
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Mesh(mesh) if preview.contains_rect(mesh.calc_bounds()) && mesh.calc_bounds().right() <= preview.center().x => {
                    Some(mesh.texture_id)
                }
                _ => None,
            })
            .unwrap()
    };
    execute(&mut harness.state_mut().session, "tool.material", &json!({"channel": "baseColor", "color": [0.0, 0.0, 1.0]})).unwrap();
    harness.step();
    let blue = stroke_texture(&harness);

    execute(&mut harness.state_mut().session, "viewport.quickMask", &json!({"enabled": true})).unwrap();
    execute(&mut harness.state_mut().session, "viewport.quickMask.edit", &json!({"editing": true})).unwrap();
    harness.step();

    let coverage = stroke_texture(&harness);
    assert_ne!(coverage, blue, "editing the quick mask redraws the blue stroke as coverage");

    execute(&mut harness.state_mut().session, "tool.material", &json!({"channel": "baseColor", "color": [1.0, 1.0, 1.0]})).unwrap();
    harness.step();

    assert_eq!(stroke_texture(&harness), coverage, "material changes do not tint the quick mask stroke");

    execute(&mut harness.state_mut().session, "viewport.quickMask.edit", &json!({"editing": false})).unwrap();
    harness.step();

    assert_eq!(stroke_texture(&harness), coverage, "the quick mask preview uses the same neutral stroke as white paint");
}
