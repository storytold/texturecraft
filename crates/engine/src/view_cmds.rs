//! Edit, viewport and display commands.

use serde_json::{Value, json};
use texturecraft_core::project::ENVIRONMENTS;
use texturecraft_core::{BakedMap, Channel, EnvAlignment, ShadowMode, ToneMapping};
use texturecraft_geom::glam::Vec3;

use crate::commands::{CommandSpec, always, bad, boolp, can_redo, can_undo, color_param, f32p, strp, u64p};
use crate::frame_target::FrameTarget;
use crate::session::ViewMode;
use crate::{Result, Session, cmd};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "viewport.frame",
            "Show entire mesh",
            ["Viewport"],
            Some("F"),
            "{target?: \"all\" (default: the whole mesh, helper geometry included) | \"selection\" (the active texture set's geometry)} → the camera; the orbit turns around the framed center",
            always,
            frame
        ),
        cmd!(
            "viewport.frameSelection",
            "Frame texture set",
            ["Viewport"],
            Some("Shift+F"),
            "{} → the camera, framing the active texture set's geometry",
            always,
            |s, _| frame_target(s, FrameTarget::Selection)
        ),
        cmd!("viewport.displayMaterial", "Display material", ["Viewport"], Some("M"), "{}", always, |s, _| set_view(s, ViewMode::Material)),
        cmd!("viewport.nextChannel", "Display next channel", ["Viewport"], Some("C"), "{}", always, |s, _| cycle_channel(s, 1)),
        cmd!("viewport.previousChannel", "Display previous channel", ["Viewport"], Some("Shift+C"), "{}", always, |s, _| cycle_channel(s, -1)),
        cmd!("viewport.nextMeshMap", "Display next mesh map", ["Viewport"], Some("B"), "{}", always, |s, _| cycle_map(s, 1)),
        cmd!("viewport.previousMeshMap", "Display previous mesh map", ["Viewport"], Some("Shift+B"), "{}", always, |s, _| cycle_map(s, -1)),
        cmd!(
            "viewport.view",
            "Set view mode",
            [],
            None,
            "{mode: material|uvChecker|normalHeightMesh (the normal the shading uses: mesh normal, Height channel slope and Normal channel; 2D view: the tangent-space normal)|image (an image resource shown as it is on the mesh and in the 2D view)|mask|<channel id>|map:<mesh map id>, image? (the resource, for image), layer? (for mask)}",
            always,
            view
        ),
        cmd!(
            "camera.set",
            "Set camera",
            [],
            None,
            "{yaw?, pitch? (degrees), distance?, fov?, target?: [x,y,z], ortho?, rotation?: constrained|free (a free camera also rolls and turns over)}",
            always,
            camera_set
        ),
        cmd!("camera.get", "Get camera", [], None, "{}", always, |s, _| Ok(camera_json(s))),
        cmd!(
            "camera.orbit",
            "Orbit camera",
            [],
            None,
            "{dx?, dy? (degrees)} (turns the camera about the target; a free camera about its own axes)",
            always,
            orbit
        ),
        cmd!(
            "display.set",
            "Display settings",
            [],
            None,
            "{environment?: <built-in>|resource:<image resource>, exposure?, blur?, rotation?, opacity?, toneMapping?: linear|aces, fov?, shadows?, shadowMode? (intensive|average|lightweight: the shadow map resolution), shadowOpacity? (0..1: how dark the shadows are), alignment? (world|camera: what the environment turns with), soloUnlit? (single-channel views flat; off: lit), aoIntensity?, bloom?, vignette?, sharpen?, grain?, postEffects?, taa?, specGloss?, parallax?, displacement? (the viewport moves the surface by the height channel), scatterColor?: [r,g,b] linear or \"#rrggbb\" sRGB, scatterRadius? (0..1 of the mesh radius), ior? (1..3, transmission), dof?: {enabled?, focusDistance? (view depth, world units; 0 = the orbit target), aperture? (lens radius, 0..1 of the mesh radius)}}",
            always,
            display
        ),
    ]
}

pub(crate) fn edit_specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "edit.undo",
            "Undo",
            ["Edit"],
            Some("Ctrl+Z"),
            "{steps?: 1 (as many as there are)} → {undone: the last step undone, steps}",
            can_undo,
            |s, p| {
                let (last, n) = repeat(s, p, Session::undo)?;
                Ok(json!({ "undone": last, "steps": n }))
            }
        ),
        cmd!(
            "edit.redo",
            "Redo",
            ["Edit"],
            Some("Ctrl+Y"),
            "{steps?: 1 (as many as there are)} → {redone: the last step redone, steps}",
            can_redo,
            |s, p| {
                let (last, n) = repeat(s, p, Session::redo)?;
                Ok(json!({ "redone": last, "steps": n }))
            }
        ),
        cmd!("edit.history", "History", [], None, "{} → {undo: steps, oldest first, redo: undone steps, next first}", always, |s, _| Ok(
            json!({ "undo": s.history_labels(), "redo": s.redo_labels() })
        )),
    ]
}

/// Run `step` (undo or redo) `steps` times (default 1) or until the history runs out: the last
/// step's label and how many ran (the History panel jumps to a step this way).
fn repeat(s: &mut Session, p: &Value, step: fn(&mut Session) -> Result<String>) -> Result<(String, u64)> {
    let steps = u64p(p, "steps").unwrap_or(1).max(1);
    let mut last = step(s)?;
    let mut n = 1;
    while n < steps
        && let Ok(l) = step(s)
    {
        last = l;
        n += 1;
    }
    Ok((last, n))
}

fn frame(s: &mut Session, p: &Value) -> Result<Value> {
    let target = match strp(p, "target") {
        None | Some("all") => FrameTarget::All,
        Some("selection") => FrameTarget::Selection,
        Some(other) => return Err(bad("viewport.frame", format!("unknown target `{other}` (all or selection)"))),
    };

    frame_target(s, target)
}

/// Frame the whole mesh, or only the triangles of the active texture set's material (the thing
/// being painted, without a floor plane or other helper geometry), and orbit around that center.
fn frame_target(s: &mut Session, target: FrameTarget) -> Result<Value> {
    let selection = match target {
        FrameTarget::Selection => s.doc.project.texture_sets.get(s.ts()).and_then(|set| s.mesh.bounds_of_material(set.material)),
        FrameTarget::All => None,
    };
    let (lo, hi) = selection.unwrap_or_else(|| s.mesh.bounds());
    s.camera.frame(lo, hi);
    s.revision = s.revision.wrapping_add(1);
    Ok(camera_json(s))
}

fn set_view(s: &mut Session, v: ViewMode) -> Result<Value> {
    if s.bake_preview.is_some() {
        // During a bake the view follows the preview: a mesh map pins it, another mode comes back afterwards
        s.bake_preview_user_view(v);
    } else {
        s.view = v;
    }
    let v = s.view;
    s.revision = s.revision.wrapping_add(1);
    Ok(json!({ "view": v.label() }))
}

/// C and Shift+C: the set's channels, then "Normal+height+mesh", then round again
fn cycle_channel(s: &mut Session, d: i64) -> Result<Value> {
    let mut views: Vec<ViewMode> = s.doc.project.texture_set(s.ts()).map(|t| t.channel_list()).unwrap_or_default().into_iter().map(ViewMode::Channel).collect();
    if views.is_empty() {
        return set_view(s, ViewMode::Material);
    }

    views.push(ViewMode::NormalHeightMesh);
    let n = views.len() as i64;
    let i = match views.iter().position(|view| *view == s.view) {
        Some(i) => (i as i64 + d).rem_euclid(n),
        None if d > 0 => 0,
        None => n - 1,
    };
    set_view(s, views[i as usize])
}

fn cycle_map(s: &mut Session, d: i64) -> Result<Value> {
    let maps = BakedMap::ALL;
    let n = maps.len() as i64;
    let i = match s.view {
        ViewMode::MeshMap(m) => maps.iter().position(|x| *x == m).map(|i| (i as i64 + d).rem_euclid(n)).unwrap_or(0),
        _ => {
            if d > 0 {
                0
            } else {
                n - 1
            }
        }
    };
    set_view(s, ViewMode::MeshMap(maps[i as usize]))
}

fn view(s: &mut Session, p: &Value) -> Result<Value> {
    let m = strp(p, "mode").ok_or_else(|| bad("viewport.view", "`mode` is required"))?;
    let v = match m {
        "material" => ViewMode::Material,
        "uvChecker" | "uv" | "checker" => ViewMode::UvChecker,
        "normalHeightMesh" => ViewMode::NormalHeightMesh,
        "image" => {
            let name = strp(p, "image").ok_or_else(|| bad("viewport.view", "`image` (an image resource) is required"))?;
            if !s.images.contains_key(name) {
                return Err(bad("viewport.view", format!("no image resource `{name}` (import it with resource.import)")));
            }

            s.view_image = Some(name.to_owned());
            ViewMode::Image
        }
        "mask" => {
            let id = crate::commands::layer_param(s, "viewport.view", p)?;
            if s.doc.project.layer(id)?.mask.is_none() {
                return Err(bad("viewport.view", "the layer has no mask"));
            }

            ViewMode::Mask(id)
        }
        _ => match m.strip_prefix("map:") {
            Some(id) => ViewMode::MeshMap(BakedMap::parse(id).ok_or_else(|| bad("viewport.view", format!("unknown mesh map `{id}`")))?),
            None => ViewMode::Channel(Channel::parse(m).ok_or_else(|| bad("viewport.view", format!("unknown view mode `{m}`")))?),
        },
    };
    set_view(s, v)
}

pub(crate) fn camera_json(s: &Session) -> Value {
    let c = &s.camera;
    json!({
        "yaw": c.yaw.to_degrees(), "pitch": c.pitch.to_degrees(), "distance": c.distance, "fov": c.fov,
        "target": c.target.to_array(), "eye": c.eye().to_array(), "ortho": c.ortho,
        "rotation": if c.free { "free" } else { "constrained" },
    })
}

fn camera_set(s: &mut Session, p: &Value) -> Result<Value> {
    let c = &mut s.camera;
    match strp(p, "rotation") {
        Some("free") => c.set_free(true),
        Some("constrained") => c.set_free(false),
        Some(o) => return Err(bad("camera.set", format!("unknown rotation `{o}`: constrained or free"))),
        None => {}
    }
    if f32p(p, "yaw").is_some() || f32p(p, "pitch").is_some() {
        // A free camera is put level at the angles
        let yaw = f32p(p, "yaw").map_or(c.yaw, f32::to_radians);
        let pitch = f32p(p, "pitch").map_or(c.pitch, |v| v.clamp(-89.0, 89.0).to_radians());
        c.set_angles(yaw, pitch);
    }
    if let Some(v) = f32p(p, "distance") {
        c.distance = v.clamp(c.scene_radius * 0.05, c.scene_radius * 200.0);
    }
    if let Some(v) = f32p(p, "fov") {
        c.fov = v.clamp(5.0, 150.0);
    }
    if let Some(t) = p.get("target").and_then(Value::as_array) {
        let v: Vec<f32> = t.iter().filter_map(Value::as_f64).map(|x| x as f32).collect();
        if v.len() == 3 && v.iter().all(|x| x.is_finite()) {
            c.target = Vec3::new(v[0], v[1], v[2]);
        }
    }
    if let Some(o) = boolp(p, "ortho") {
        c.ortho = o;
    }
    s.revision = s.revision.wrapping_add(1);
    Ok(camera_json(s))
}

fn orbit(s: &mut Session, p: &Value) -> Result<Value> {
    s.camera.orbit(f32p(p, "dx").unwrap_or(0.0).to_radians(), f32p(p, "dy").unwrap_or(0.0).to_radians());
    s.revision = s.revision.wrapping_add(1);
    Ok(camera_json(s))
}

fn display(s: &mut Session, p: &Value) -> Result<Value> {
    let tm = match strp(p, "toneMapping") {
        Some("linear") => Some(ToneMapping::Linear),
        Some("aces") | Some("acesFilmic") => Some(ToneMapping::AcesFilmic),
        Some(o) => return Err(bad("display.set", format!("unknown tone mapping `{o}`"))),
        None => None,
    };
    let shadow_mode = match strp(p, "shadowMode") {
        Some(id) => Some(ShadowMode::parse(id).ok_or_else(|| bad("display.set", format!("unknown shadow mode `{id}` (intensive, average or lightweight)")))?),
        None => None,
    };
    let alignment = match strp(p, "alignment") {
        Some(id) => Some(EnvAlignment::parse(id).ok_or_else(|| bad("display.set", format!("unknown environment alignment `{id}` (world or camera)")))?),
        None => None,
    };
    let env = strp(p, "environment").map(str::to_string);
    if let Some(name) = env.as_deref().and_then(|e| e.strip_prefix(texturecraft_core::project::ENV_RESOURCE_PREFIX))
        && !s.images.contains_key(name)
    {
        return Err(bad("display.set", format!("no image resource `{name}` (import it with resource.import)")));
    }

    if let Some(e) = env.as_deref().filter(|e| !e.starts_with(texturecraft_core::project::ENV_RESOURCE_PREFIX) && !ENVIRONMENTS.contains(e)) {
        return Err(bad("display.set", format!("unknown environment `{e}`: one of {}, or resource:<image>", ENVIRONMENTS.join(", "))));
    }

    let scatter_color = color_param("display.set", p, "scatterColor")?;
    let d = &mut s.doc.project.display;
    if let Some(e) = env {
        d.environment = e;
    }
    for (k, slot, lo, hi) in [
        ("exposure", &mut d.env_exposure, -10.0, 10.0),
        ("blur", &mut d.env_blur, 0.0, 1.0),
        ("rotation", &mut d.env_rotation, -360.0, 360.0),
        ("opacity", &mut d.env_opacity, 0.0, 1.0),
        ("fov", &mut d.fov, 5.0, 150.0),
        ("aoIntensity", &mut d.ao_intensity, 0.0, 1.0),
        ("bloom", &mut d.bloom, 0.0, 1.0),
        ("vignette", &mut d.vignette, 0.0, 1.0),
        ("sharpen", &mut d.sharpen, 0.0, 1.0),
        ("grain", &mut d.grain, 0.0, 1.0),
        ("scatterRadius", &mut d.scatter_radius, 0.0, 1.0),
        ("ior", &mut d.ior, 1.0, 3.0),
        ("shadowOpacity", &mut d.shadow_opacity, 0.0, 1.0),
    ] {
        if let Some(v) = f32p(p, k) {
            *slot = v.clamp(lo, hi);
        }
    }
    if let Some(t) = tm {
        d.tone_mapping = t;
    }
    if let Some(c) = scatter_color {
        d.scatter_color = c.map(|v| v.clamp(0.0, 1.0));
    }
    if let Some(b) = boolp(p, "shadows") {
        d.shadows = b;
    }
    if let Some(mode) = shadow_mode {
        d.shadow_mode = mode;
    }
    if let Some(alignment) = alignment {
        d.env_alignment = alignment;
    }
    if let Some(dof) = p.get("dof") {
        if let Some(b) = boolp(dof, "enabled") {
            d.dof.enabled = b;
        }
        if let Some(v) = f32p(dof, "focusDistance") {
            d.dof.focus_distance = v.clamp(0.0, 1e6);
        }
        if let Some(v) = f32p(dof, "aperture") {
            d.dof.aperture = v.clamp(0.0, 1.0);
        }
    }
    for (k, slot) in [
        ("postEffects", &mut d.post_effects),
        ("taa", &mut d.taa),
        ("specGloss", &mut d.spec_gloss),
        ("parallax", &mut d.parallax),
        ("displacement", &mut d.displacement),
        ("soloUnlit", &mut d.solo_unlit),
    ] {
        if let Some(b) = boolp(p, k) {
            *slot = b;
        }
    }

    // The field of view applies to the view only when set (a saved camera keeps its own)
    if f32p(p, "fov").is_some() {
        s.camera.fov = s.doc.project.display.fov;
    }
    s.revision = s.revision.wrapping_add(1);
    Ok(serde_json::to_value(&s.doc.project.display).unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execute;

    fn sphere_session() -> Session {
        Session::new_project("t", texturecraft_geom::primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap()
    }

    fn pressed(s: &mut Session, command: &str) -> ViewMode {
        execute(s, command, &json!({})).unwrap();
        s.view
    }

    #[test]
    fn c_visits_the_channels_then_normal_height_mesh_and_wraps() {
        let mut s = sphere_session();
        let channels: Vec<ViewMode> = s.doc.project.texture_set(s.ts()).unwrap().channel_list().into_iter().map(ViewMode::Channel).collect();
        assert!(channels.len() > 2);
        assert_eq!(s.view, ViewMode::Material);

        // From the material C starts at the first channel
        let mut seen = Vec::new();
        for _ in 0..channels.len() + 2 {
            seen.push(pressed(&mut s, "viewport.nextChannel"));
        }
        let mut expected = channels.clone();
        expected.push(ViewMode::NormalHeightMesh);
        expected.push(channels[0]);
        assert_eq!(seen, expected, "channels, then Normal+height+mesh, then round again");

        // Shift+C walks it backwards, and from the material starts at the last entry
        assert_eq!(pressed(&mut s, "viewport.previousChannel"), ViewMode::NormalHeightMesh);
        assert_eq!(pressed(&mut s, "viewport.previousChannel"), *channels.last().unwrap());
        execute(&mut s, "viewport.displayMaterial", &json!({})).unwrap();
        assert_eq!(pressed(&mut s, "viewport.previousChannel"), ViewMode::NormalHeightMesh);
    }

    #[test]
    fn an_image_resource_can_be_shown_as_the_view() {
        let mut s = sphere_session();
        assert!(execute(&mut s, "viewport.view", &json!({"mode": "image"})).is_err(), "the image is required");
        assert!(execute(&mut s, "viewport.view", &json!({"mode": "image", "image": "stone.png"})).is_err(), "unknown resources are rejected");
        assert_eq!(s.view, ViewMode::Material);
        assert!(s.image_view_rgba(8).is_none(), "no image is chosen");

        let mut png = Vec::new();
        let img = image::RgbaImage::from_fn(2, 2, |x, y| image::Rgba([if x == 0 { 255 } else { 0 }, if y == 0 { 255 } else { 0 }, 0, 255]));
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let name = s.import_resource_bytes("stone.png", png).unwrap();
        execute(&mut s, "viewport.view", &json!({"mode": "image", "image": name})).unwrap();
        assert_eq!((s.view, s.view_image.as_deref()), (ViewMode::Image, Some(name.as_str())));

        // The 2x2 image is stretched over the UV square: each quadrant keeps its texel's color
        let rgba = s.image_view_rgba(8).unwrap();
        assert_eq!(rgba.len(), 8 * 8 * 4);
        let at = |x: usize, y: usize| rgba[(y * 8 + x) * 4..(y * 8 + x) * 4 + 3].to_vec();
        assert_eq!(at(0, 0), [255, 255, 0], "top left: red and green");
        assert_eq!(at(7, 0), [0, 255, 0], "top right: green");
        assert_eq!(at(0, 7), [255, 0, 0], "bottom left: red");
        assert_eq!(at(7, 7), [0, 0, 0], "bottom right: black");
        assert!(s.image_view_rgba(u32::MAX).is_none(), "reject an oversized raster before allocating it");
    }

    #[test]
    fn the_solo_view_is_unlit_by_default_and_the_switch_is_a_display_setting() {
        let mut s = sphere_session();
        assert!(s.doc.project.display.solo_unlit);
        let r = execute(&mut s, "display.set", &json!({"soloUnlit": false})).unwrap();
        assert_eq!(r["solo_unlit"], json!(false));
        assert!(!s.doc.project.display.solo_unlit);
    }

    #[test]
    fn alignment_and_shadow_quality_are_display_settings_with_the_reference_defaults() {
        let mut s = sphere_session();
        let d = &s.doc.project.display;
        assert_eq!((d.env_alignment, d.shadow_mode, d.shadow_opacity), (EnvAlignment::World, ShadowMode::Lightweight, 1.0));
        assert_eq!(ShadowMode::ALL.map(ShadowMode::label), ["Intensive", "Average", "Lightweight"]);
        assert_eq!(EnvAlignment::ALL.map(EnvAlignment::label), ["World", "Camera"]);
        assert!(ShadowMode::Intensive.map_size() > ShadowMode::Average.map_size() && ShadowMode::Average.map_size() > ShadowMode::Lightweight.map_size());

        let r = execute(&mut s, "display.set", &json!({"alignment": "camera", "shadowMode": "intensive", "shadowOpacity": 0.4})).unwrap();
        let d = &s.doc.project.display;
        assert_eq!((d.env_alignment, d.shadow_mode, d.shadow_opacity), (EnvAlignment::Camera, ShadowMode::Intensive, 0.4));
        assert_eq!((r["env_alignment"].as_str(), r["shadow_mode"].as_str()), (Some("Camera"), Some("Intensive")));
        execute(&mut s, "display.set", &json!({"shadowOpacity": 3.0})).unwrap();
        assert_eq!(s.doc.project.display.shadow_opacity, 1.0, "the opacity is clamped to 0..1");
        for bad in [json!({"alignment": "screen"}), json!({"shadowMode": "ultra"})] {
            assert!(execute(&mut s, "display.set", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.doc.project.display.env_alignment, EnvAlignment::Camera, "a rejected call changes nothing");

        // Projects saved before these settings open with the defaults
        let old: texturecraft_core::DisplaySettings = serde_json::from_value(json!({"environment": "Night", "shadows": true})).unwrap();
        assert_eq!((old.env_alignment, old.shadow_mode, old.shadow_opacity), (EnvAlignment::World, ShadowMode::Lightweight, 1.0));
        let saved: texturecraft_core::DisplaySettings = serde_json::from_value(serde_json::to_value(&s.doc.project.display).unwrap()).unwrap();
        assert_eq!(saved, s.doc.project.display, "the settings survive a save");
    }

    #[test]
    fn the_image_view_falls_back_to_the_material_when_the_image_goes() {
        let mut s = sphere_session();
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([9, 9, 9, 255]));
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let name = s.import_resource_bytes("stone.png", png).unwrap();
        execute(&mut s, "viewport.view", &json!({"mode": "image", "image": name})).unwrap();
        assert_eq!(s.view, ViewMode::Image);
        execute(&mut s, "resource.remove", &json!({"name": name})).unwrap();
        assert_eq!((s.view, s.view_image.as_deref()), (ViewMode::Material, None), "a removed image is no longer shown");
    }

    #[test]
    fn display_set_accepts_the_names_it_reports_in_any_case() {
        let mut s = sphere_session();
        let r = execute(&mut s, "display.set", &json!({"alignment": "camera", "shadowMode": "Intensive"})).unwrap();
        let (alignment, mode) = (r["env_alignment"].as_str().unwrap().to_owned(), r["shadow_mode"].as_str().unwrap().to_owned());
        execute(&mut s, "display.set", &json!({"alignment": "World"})).unwrap();
        execute(&mut s, "display.set", &json!({"alignment": alignment, "shadowMode": mode})).unwrap();
        let d = &s.doc.project.display;
        assert_eq!((d.env_alignment, d.shadow_mode), (EnvAlignment::Camera, ShadowMode::Intensive));
    }

    #[test]
    fn hdr_resources_become_the_environment() {
        let mut s = Session::new_project("t", texturecraft_geom::primitives::sphere(1.0, 16, 8), "builtin:sphere", None, 128).unwrap();
        assert!(execute(&mut s, "display.set", &json!({"environment": "resource:sky.hdr"})).is_err(), "unknown resources are rejected");
        let e = execute(&mut s, "display.set", &json!({"environment": "nonexistent"})).unwrap_err().to_string();
        assert!(e.contains("Studio Soft"), "unknown environments are rejected with the valid names: {e}");
        assert_eq!(s.doc.project.display.environment, "Studio Soft");
        execute(&mut s, "display.set", &json!({"environment": "Night"})).unwrap();
        let img = image::Rgb32FImage::from_fn(64, 32, |_, y| image::Rgb([if y < 8 { 40.0 } else { 0.5 }; 3]));
        let mut hdr = Vec::new();
        image::DynamicImage::ImageRgb32F(img).write_to(&mut std::io::Cursor::new(&mut hdr), image::ImageFormat::Hdr).unwrap();
        let name = s.import_resource_bytes("sky.hdr", hdr).unwrap();
        let res = &s.images[&name];
        assert!(!res.srgb && res.rgba[0][0] > 30.0, "HDR keeps values above 1");
        let env = format!("resource:{name}");
        let r = execute(
            &mut s,
            "display.set",
            &json!({"environment": env, "specGloss": true, "parallax": true, "displacement": true, "taa": false, "scatterColor": [2.0, 0.5, 0.25], "scatterRadius": 0.3}),
        )
        .unwrap();
        assert_eq!(r["environment"], json!(env));
        let d = &s.doc.project.display;
        assert!(d.spec_gloss && d.parallax && d.displacement && !d.taa);
        assert_eq!((d.scatter_color, d.scatter_radius), ([1.0, 0.5, 0.25], 0.3), "scatter settings (clamped)");
        assert_eq!(d.environment_resource(), Some(name.as_str()));
        execute(&mut s, "file.removeUnused", &Value::Null).unwrap();
        assert!(s.doc.resources.contains_key(&name), "the environment's image counts as used");
    }
}
