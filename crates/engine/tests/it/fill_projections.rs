//! The 3D projection settings and the projection kinds of a fill, through engine commands

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, DepthCulling, FillSymmetry, LayerId, LayerKind, Projection, Projector, Sampling, UvTransform};
use texturecraft_engine::geom::glam::Vec3;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

fn session() -> Session {
    let mut s = Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, 128).unwrap();
    s.update();
    s
}

/// A fill layer with a checker procedural in base color
fn checker_fill(s: &mut Session) -> LayerId {
    let layer = LayerId(execute(s, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    execute(s, "layer.fill.set", &json!({"layer": layer.0, "channel": "baseColor", "procedural": "Checker", "scale": 4, "low": [0, 0, 0], "high": [1, 1, 1]}))
        .unwrap();
    layer
}

fn set(s: &mut Session, layer: LayerId, mut p: Value) {
    p["layer"] = json!(layer.0);
    execute(s, "layer.fill.projection", &p).unwrap();
}

fn projection(s: &Session, layer: LayerId) -> Projection {
    match &s.doc.project.layer(layer).unwrap().kind {
        LayerKind::Fill(f) => f.projection.clone(),
        _ => panic!("a fill layer"),
    }
}

/// Base color red of every texel
fn snapshot(s: &mut Session) -> Vec<f32> {
    s.update();
    let cache = &s.caches[0];
    let (w, h) = cache.size;
    (0..w * h).map(|i| cache.composite.value(Channel::BaseColor, i % w, i / w)[0]).collect()
}

/// Rotation, per-axis scale and offset move every 3D projection; the defaults and older saves change nothing
#[test]
fn three_d_projection_settings_turn_stretch_and_move_every_3d_projection() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    for kind in ["triplanar", "planar", "spherical", "cylindrical"] {
        set(&mut s, layer, json!({"projection": kind}));
        let plain = snapshot(&mut s);

        set(&mut s, layer, json!({"projectorRotation": [30, 45, 60]}));

        assert_ne!(snapshot(&mut s), plain, "{kind}: rotation turns the pattern");

        set(&mut s, layer, json!({"projectorRotation": [0, 0, 0], "projectorScale": [2, 0.5, 1]}));

        assert_ne!(snapshot(&mut s), plain, "{kind}: per-axis scale stretches the pattern");

        set(&mut s, layer, json!({"projectorScale": 1, "offset": [0.3, 0.2, 0.1]}));

        assert_ne!(snapshot(&mut s), plain, "{kind}: offset moves the pattern");

        set(&mut s, layer, json!({"offset": 0}));

        assert_eq!(snapshot(&mut s), plain, "{kind}: the defaults change nothing");

        set(&mut s, layer, json!({"projectorRotation": [10, 20, 30], "projectorScale": [1, 2, 3]}));
        let mut saved = serde_json::to_value(projection(&s, layer)).unwrap();
        let fields = saved.as_object_mut().unwrap().values_mut().next().unwrap().as_object_mut().unwrap();
        fields.remove("projector");
        let loaded: Projection = serde_json::from_value(saved).unwrap();
        let settings = match loaded {
            Projection::Triplanar { projector, .. }
            | Projection::Planar { projector, .. }
            | Projection::Spherical { projector, .. }
            | Projection::Cylindrical { projector, .. } => projector,
            other => panic!("{kind}: loaded as {other:?}"),
        };
        assert_eq!(settings, Projector::default(), "{kind}: an older save has no turn or stretch");

        set(&mut s, layer, json!({"projectorRotation": 0, "projectorScale": 1}));
    }

    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    let row = inspect["layers"].as_array().unwrap().iter().find(|l| l["id"] == layer.0).unwrap().clone();
    let cylindrical = &row["fill"]["projectionSettings"]["Cylindrical"];
    assert!(
        cylindrical.is_object() && cylindrical.get("projector").is_none(),
        "a reset projector is the default, left out as older projects saved it: {cylindrical}"
    );
}

/// A negative projector scale mirrors that axis, as a project file's does: the command keeps its sign and limits only its size
#[test]
fn a_negative_projector_scale_keeps_its_mirror() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    set(&mut s, layer, json!({"projection": "spherical", "projectorScale": [-1.5, 0.0, 5000.0]}));
    let Projection::Spherical { projector, .. } = projection(&s, layer) else { panic!("a spherical projection") };

    assert_eq!(projector.scale, [-1.5, 0.001, 1000.0], "the mirror kept, each axis's size within the scale limits");

    set(&mut s, layer, json!({"projectorScale": -0.0001}));
    let Projection::Spherical { projector, .. } = projection(&s, layer) else { panic!("a spherical projection") };
    assert_eq!(projector.scale, [-0.001; 3], "a tiny mirrored scale keeps the smallest size, still mirrored");
}

/// Fill (Match per UV Tile) shows the same pattern on both UDIM tiles, where UV runs on across them
#[test]
fn match_per_uv_tile_repeats_the_source_once_per_tile() {
    let mut s = Session::new_project("t", primitives::udim_plane(), "builtin:udim_plane", None, 128).unwrap();
    let layer = LayerId(execute(&mut s, "layer.add.fill", &Value::Null).unwrap()["layer"].as_u64().unwrap());
    execute(
        &mut s,
        "layer.fill.set",
        &json!({"layer": layer.0, "channel": "baseColor", "procedural": "Clouds", "scale": 3, "low": [0, 0, 0], "high": [1, 1, 1]}),
    )
    .unwrap();
    let tiles = |s: &mut Session| {
        s.update();
        let composite = &s.caches[0].composite;
        let tile = |block: u32| (0..128 * 128).map(|i| composite.value(Channel::BaseColor, block * 128 + i % 128, i / 128)[0]).collect::<Vec<f32>>();
        (tile(0), tile(1))
    };
    let (left, right) = tiles(&mut s);

    assert_ne!(left, right, "a UV projection runs on into the next tile");

    set(&mut s, layer, json!({"projection": "uvTile"}));

    assert_eq!(projection(&s, layer), Projection::UvTile);

    let (left, right) = tiles(&mut s);
    let worst = left.iter().zip(&right).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);

    assert!(worst < 1e-4, "every tile shows the same pattern (worst difference {worst})");
}

/// Cylindrical Angle is the arc one repeat spans; older saves go the whole way round
#[test]
fn cylindrical_angle_is_the_arc_one_repeat_spans() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    set(&mut s, layer, json!({"projection": "cylindrical"}));
    let plain = snapshot(&mut s);
    set(&mut s, layer, json!({"angle": 120}));

    assert!(matches!(projection(&s, layer), Projection::Cylindrical { angle: 120.0, .. }));
    assert_ne!(snapshot(&mut s), plain, "a third of the way round repeats three times");

    set(&mut s, layer, json!({"angle": 360}));

    assert_eq!(snapshot(&mut s), plain, "360 degrees is the whole way round");

    let mut saved = serde_json::to_value(projection(&s, layer)).unwrap();
    saved["Cylindrical"].as_object_mut().unwrap().remove("angle");

    assert!(matches!(serde_json::from_value(saved).unwrap(), Projection::Cylindrical { angle: 360.0, .. }));
}

/// Texel indices whose surface normal has a Z above `above` (and below `below`)
fn facing_z(s: &Session, above: f32, below: f32) -> Vec<usize> {
    let cache = &s.caches[0];
    let (w, h) = cache.size;
    (0..(w * h) as usize).filter(|&i| cache.gbuf.surface(&s.mesh, i).is_some_and(|surface| surface.nrm.z > above && surface.nrm.z < below)).collect()
}

/// Cropped to Shape stops a shrunk tri-planar box short of the mesh; the default box holds it all
#[test]
fn shape_crop_stops_a_projection_at_its_shape() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": false})).unwrap();
    let empty = snapshot(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": true})).unwrap();
    set(&mut s, layer, json!({"projection": "triplanar"}));
    let plain = snapshot(&mut s);
    set(&mut s, layer, json!({"shapeCrop": "cropped"}));

    assert_eq!(snapshot(&mut s), plain, "the default box holds the whole mesh");

    set(&mut s, layer, json!({"projectorScale": 0.5}));
    let cropped = snapshot(&mut s);
    let gone = cropped.iter().zip(&empty).filter(|(a, b)| a == b).count();
    set(&mut s, layer, json!({"shapeCrop": "extendsOutside"}));
    let extends = snapshot(&mut s);
    let gone_extending = extends.iter().zip(&empty).filter(|(a, b)| a == b).count();
    assert!(gone > gone_extending + 1000, "the half-size box leaves the rest of the mesh bare ({gone} vs {gone_extending} bare texels)");
    assert!(execute(&mut s, "layer.fill.projection", &json!({"layer": layer.0, "shapeCrop": "clipped"})).is_err());
}

/// Backface culling leaves the far side of the sphere bare and the near side unchanged
#[test]
fn backface_culling_keeps_a_planar_projection_off_faces_turned_away() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": false})).unwrap();
    let empty = snapshot(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": true})).unwrap();

    // Without depth culling the planar projection reaches the far side, so only backface culling can take it off
    set(&mut s, layer, json!({"projection": "planar", "axis": 2, "depthCulling": false}));
    let plain = snapshot(&mut s);
    let (far, near) = (facing_z(&s, -1.1, -0.05), facing_z(&s, 0.05, 1.1));

    assert!(!far.is_empty() && !near.is_empty());
    assert!(far.iter().any(|&i| plain[i] != empty[i]), "without culling the projection reaches the far side");

    set(&mut s, layer, json!({"backfaceCulling": true, "backfaceAngle": 90, "backfaceHardness": 1}));
    let culled = snapshot(&mut s);
    assert!(far.iter().all(|&i| culled[i] == empty[i]), "faces turned away get none of the projection");
    assert!(near.iter().all(|&i| culled[i] == plain[i]), "faces turned towards it keep all of it");

    set(&mut s, layer, json!({"backfaceAngle": 45}));
    let narrow = snapshot(&mut s);
    let sideways = facing_z(&s, 0.1, 0.6);
    assert!(sideways.iter().all(|&i| narrow[i] == empty[i]), "past 45 degrees nothing is left");
}

/// A decal saved before decals were planar projections loads as a planar projection placed on its frame, cropped to it and off faces turned away
#[test]
fn saved_decals_load_as_planar_projections_placed_on_the_surface() {
    use texturecraft_engine::core::{Backface, Decal, DecalFlip, DecalSymmetry, Fill, ShapeCrop};

    // A decal saved with a mirrored copy and a flipped image keeps both
    let frame = Decal {
        center: [0.0, 0.0, 1.0],
        normal: [0.0, 0.0, 1.0],
        up: [0.0, 1.0, 0.0],
        size: [0.5, 0.4],
        rotation: 10.0,
        offset: [0.1, 0.0],
        depth: 0.3,
        symmetry: Some(DecalSymmetry { axis: 0, flip: DecalFlip::U, offset: 0.25 }),
        flip: [true, false],
    };
    let mut saved = serde_json::to_value(Fill::default()).unwrap();
    saved["projection"] = json!({ "Decal": frame });
    let fill: Fill = serde_json::from_value(saved).unwrap();
    let Projection::Planar { placed: Some(placed), projector, .. } = fill.projection else { panic!("{:?}", fill.projection) };

    assert_eq!(placed, frame);
    assert_eq!(projector.crop, ShapeCrop::Cropped);
    assert_eq!(projector.backface, Backface { on: true, angle: 90.0, hardness: 0.5 });
}

/// A planar projection with a decal's look saves as a decal, so versions from before planar decals still read a decal; changed in any way it saves as a planar projection
#[test]
fn placed_planars_with_a_decals_look_save_as_decals() {
    use texturecraft_engine::core::{Decal, Fill};
    let frame = Decal {
        center: [0.0, 0.0, 1.0],
        normal: [0.0, 0.0, 1.0],
        up: [0.0, 1.0, 0.0],
        size: [0.5, 0.4],
        rotation: 10.0,
        offset: [0.1, 0.0],
        depth: 0.3,
        symmetry: None,
        flip: [false; 2],
    };
    let fill = Fill { projection: Projection::decal(frame), ..Fill::default() };

    let saved = serde_json::to_value(&fill).unwrap();

    assert_eq!(saved["projection"], json!({ "Decal": frame }));
    assert_eq!(serde_json::from_value::<Fill>(saved).unwrap(), fill, "and reads back the same");

    let mut turned = fill;
    let Projection::Planar { uv, .. } = &mut turned.projection else { panic!("{:?}", turned.projection) };
    uv.rotation = 30.0;
    let saved = serde_json::to_value(&turned).unwrap();

    assert_eq!(saved["projection"]["Planar"]["placed"], json!(frame));
    assert_eq!(serde_json::from_value::<Fill>(saved).unwrap(), turned);
}

/// A warp projects from a frame around the mesh: pulling a corner bends the image, Projection Depth bounds it when cropped, and Reset frame unpulls the corners
#[test]
fn warp_projection_bends_to_its_corners_reaches_its_depth_and_resets_its_frame() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": false})).unwrap();
    let empty = snapshot(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": true})).unwrap();
    set(&mut s, layer, json!({"projection": "warp"}));
    let Projection::Warp { frame, corners, depth, .. } = projection(&s, layer) else { panic!("{:?}", projection(&s, layer)) };

    assert_eq!((corners, depth), (Projection::FRAME_CORNERS, Projection::WARP_DEPTH));
    assert_eq!((frame.normal, frame.up), ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]), "seen from the front");
    assert!(frame.size.iter().all(|side| (side - 2.0).abs() < 1e-3) && frame.center.iter().all(|c| c.abs() < 1e-3), "around the unit sphere: {frame:?}");

    let unpulled = snapshot(&mut s);
    assert_ne!(unpulled, empty);

    let pulled = json!([[-0.5, 0.5], [0.9, 0.9], [0.5, -0.5], [-0.5, -0.5]]);
    set(&mut s, layer, json!({"warpCorners": pulled}));
    let bent = snapshot(&mut s);
    assert_ne!(bent, unpulled, "a pulled corner bends the image");

    set(&mut s, layer, json!({"projection": "warp", "rotation": 0}));

    assert_eq!(snapshot(&mut s), bent, "picking warp again keeps the corners");

    set(&mut s, layer, json!({"shapeCrop": "cropped", "warpCorners": Projection::FRAME_CORNERS}));
    let cropped = snapshot(&mut s);

    // Depth 0.1 of a frame 2 wide reaches 0.2 in front of and behind the frame through the sphere's middle
    let (beyond, within) = (facing_z(&s, 0.35, 1.1), facing_z(&s, -0.05, 0.05));
    assert!(!beyond.is_empty() && !within.is_empty());
    assert!(beyond.iter().all(|&i| cropped[i] == empty[i]), "past its Projection Depth the warp does not reach");
    assert!(within.iter().all(|&i| cropped[i] == unpulled[i]), "within it the warp covers as before");

    set(&mut s, layer, json!({"projectionDepth": 1}));
    let deep = snapshot(&mut s);
    assert!(beyond.iter().all(|&i| deep[i] == unpulled[i]), "a deeper warp reaches the whole sphere");

    set(&mut s, layer, json!({"warpCorners": pulled}));
    execute(&mut s, "layer.fill.resetFrame", &json!({"layer": layer.0})).unwrap();
    let Projection::Warp { frame: reset, corners, depth, .. } = projection(&s, layer) else { panic!("still a warp") };
    assert_eq!((reset, corners, depth), (frame, Projection::FRAME_CORNERS, 1.0), "the frame and corners go back; the depth stays");

    let steps = s.history_labels().len();
    execute(&mut s, "layer.fill.resetFrame", &json!({"layer": layer.0})).unwrap();

    assert_eq!(s.history_labels().len(), steps, "resetting a frame already reset adds no undo step");

    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    let row = inspect["layers"].as_array().unwrap().iter().find(|l| l["id"] == layer.0).unwrap().clone();

    assert_eq!(row["fill"]["projection"], "Warp projection");
    assert_eq!(row["fill"]["projectionSettings"]["Warp"]["corners"], json!(Projection::FRAME_CORNERS));

    set(&mut s, layer, json!({"projection": "planar"}));

    assert!(execute(&mut s, "layer.fill.resetFrame", &json!({"layer": layer.0})).is_err(), "only a warp has a frame to reset");
}

/// Depth culling keeps planar and warp projections off surfaces hidden behind others from the projector; new ones start with it on, older saves without it
#[test]
fn depth_culling_keeps_planar_and_warp_projections_off_hidden_surfaces() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": false})).unwrap();
    let empty = snapshot(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": true})).unwrap();
    set(&mut s, layer, json!({"projection": "planar", "axis": 2}));
    let Projection::Planar { projector, .. } = projection(&s, layer) else { panic!("{:?}", projection(&s, layer)) };

    assert_eq!(projector.depth, DepthCulling::NEW, "a new planar projection culls by depth");

    let culled = snapshot(&mut s);
    set(&mut s, layer, json!({"depthCulling": false}));
    let plain = snapshot(&mut s);

    // The sphere's far side sits behind its near side, seen from +Z
    let (far, near) = (facing_z(&s, -1.1, -0.3), facing_z(&s, 0.3, 1.1));
    assert!(!far.is_empty() && !near.is_empty());
    assert!(far.iter().any(|&i| plain[i] != empty[i]), "without depth culling the far side gets the projection");
    assert!(far.iter().all(|&i| culled[i] == empty[i]), "hidden behind the near side, the far side gets none of it");
    assert!(near.iter().all(|&i| culled[i] == plain[i]), "the near side keeps all of it");

    set(&mut s, layer, json!({"depthCulling": true, "depthCullingHardness": 0}));

    assert_ne!(snapshot(&mut s), culled, "a softer cull fades further behind the near side");

    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    let row = inspect["layers"].as_array().unwrap().iter().find(|l| l["id"] == layer.0).unwrap().clone();

    assert_eq!(row["fill"]["projectionSettings"]["Planar"]["projector"]["depth"], json!({"on": true, "hardness": 0.0}));

    set(&mut s, layer, json!({"projection": "warp"}));
    let Projection::Warp { projector, .. } = projection(&s, layer) else { panic!("{:?}", projection(&s, layer)) };
    assert_eq!(projector.depth, DepthCulling::NEW, "a new warp culls by depth");

    let warped = snapshot(&mut s);
    assert!(far.iter().all(|&i| warped[i] == empty[i]), "a warp from the front leaves the far side bare");

    let saved: Projector = serde_json::from_value(json!({"rotation": [0, 0, 0], "scale": [1, 1, 1]})).unwrap();

    assert_eq!(saved.depth, DepthCulling { on: false, hardness: 0.75 }, "an older save has no depth culling");
}

/// Texels of `far` that `values` gives something other than the bare `empty`
fn filled(far: &[usize], values: &[f32], empty: &[f32]) -> usize {
    far.iter().filter(|&&i| values[i] != empty[i]).count()
}

/// The symmetry axis is read from an index or from the letter tool.brush takes; any other value is refused, never ignored
#[test]
fn fill_symmetry_axis_reads_an_index_or_a_letter() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    let axis = |s: &Session| match &s.doc.project.layer(layer).unwrap().kind {
        LayerKind::Fill(fill) => fill.sampling.symmetry.axis,
        _ => panic!("a fill layer"),
    };

    set(&mut s, layer, json!({"symmetryAxis": "Y"}));

    assert_eq!(axis(&s), 1);

    set(&mut s, layer, json!({"symmetryAxis": 2}));

    assert_eq!(axis(&s), 2);

    for refused in [json!("w"), json!(true), json!(-1), json!(3)] {
        assert!(execute(&mut s, "layer.fill.projection", &json!({"layer": layer.0, "symmetryAxis": refused})).is_err(), "{refused}");
    }

    assert_eq!(axis(&s), 2, "a refused axis changes nothing");
}

/// Mirror and radial symmetry copy a planar projection onto the sphere's far side and leave the near side as it was; Flip copy turns the copies' images over; UV projections and older saves have no copies
#[test]
fn fill_symmetry_mirrors_or_turns_copies_of_a_3d_projection() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": false})).unwrap();
    let empty = snapshot(&mut s);
    execute(&mut s, "layer.visible", &json!({"layer": layer.0, "visible": true})).unwrap();

    // Depth culled from +Z, the planar projection reaches only the near side
    set(&mut s, layer, json!({"projection": "planar", "axis": 2}));
    let plain = snapshot(&mut s);
    let (far, near) = (facing_z(&s, -1.1, -0.3), facing_z(&s, 0.3, 1.1));

    assert_eq!(filled(&far, &plain, &empty), 0);

    set(&mut s, layer, json!({"symmetry": true, "symmetryType": "mirror", "symmetryAxis": 2}));
    let mirrored = snapshot(&mut s);
    assert!(filled(&far, &mirrored, &empty) > far.len() / 2, "the mirror copy reaches the far side");
    assert!(near.iter().all(|&i| mirrored[i] == plain[i]), "the original keeps its own half");

    set(&mut s, layer, json!({"symmetryType": "radial", "symmetryAxis": 1, "symmetryCount": 2}));
    let turned = snapshot(&mut s);
    assert!(filled(&far, &turned, &empty) > far.len() / 2, "the copy turned half round reaches the far side");
    assert!(near.iter().all(|&i| turned[i] == plain[i]), "the original keeps its own wedge");
    assert_ne!(turned, mirrored, "a turned copy is not a mirrored one");

    set(&mut s, layer, json!({"symmetry": false}));

    assert_eq!(snapshot(&mut s), plain, "switched off, the copies go");

    // A left to right ramp shows a copy's image turned over
    let ramp = s.import_resource_bytes("ramp.png", ramp_png()).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"layer": layer.0, "channel": "baseColor", "image": ramp})).unwrap();
    set(&mut s, layer, json!({"symmetry": true}));
    let copied = snapshot(&mut s);
    set(&mut s, layer, json!({"flipCopy": [true, false]}));
    let flipped = snapshot(&mut s);
    assert!(far.iter().filter(|&&i| (flipped[i] - copied[i]).abs() > 0.1).count() > far.len() / 4, "Flip copy U turns the copy's ramp round");
    assert!(near.iter().all(|&i| flipped[i] == copied[i]), "the original is never flipped");

    set(&mut s, layer, json!({"projection": "uv"}));
    let uv = snapshot(&mut s);
    set(&mut s, layer, json!({"symmetry": false}));

    assert_eq!(snapshot(&mut s), uv, "a UV projection has no copies");

    assert!(execute(&mut s, "layer.fill.projection", &json!({"layer": layer.0, "symmetryType": "spiral"})).is_err());
    assert!(execute(&mut s, "layer.fill.projection", &json!({"layer": layer.0, "symmetryAxis": 3})).is_err());

    let saved: Sampling = serde_json::from_value(json!({"filtering": "BilinearHq", "wrap": "Repeat", "scale": "Tiling"})).unwrap();

    assert_eq!(saved.symmetry, FillSymmetry::OFF, "an older save has no symmetry");
}

/// Moving an axis planar on the model first places it as the frame it projects from: nothing moves until the frame does, and undo puts the axis planar back
#[test]
fn moving_an_axis_planar_places_it_as_its_frame_first() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    let ramp = s.import_resource_bytes("ramp.png", ramp_png()).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"layer": layer.0, "channel": "baseColor", "image": ramp})).unwrap();
    set(&mut s, layer, json!({"projection": "planar", "axis": 0, "scale": 1.5, "offset": [0.2, -0.1, 0.0], "projectorRotation": [0, 30, 10]}));
    let axis = projection(&s, layer);
    let before = snapshot(&mut s);

    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0})).unwrap();
    let placed = projection(&s, layer);
    let Projection::Planar { placed: Some(frame), projector, .. } = &placed else { panic!("placed: {placed:?}") };

    assert_eq!((projector.rotation, projector.scale), ([0.0; 3], [1.0; 3]), "the turn is in the frame now");

    let after = snapshot(&mut s);
    let moved = before.iter().zip(&after).filter(|(a, b)| (*a - *b).abs() > 2.0 / 255.0).count();
    assert!(moved * 200 < before.len(), "the image stays where it was: {moved} of {} texels changed", before.len());

    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "offset": [0.4, 0.0], "merge": true})).unwrap();

    assert_ne!(snapshot(&mut s), after, "then the frame moves the image");
    let Projection::Planar { placed: Some(slid), .. } = projection(&s, layer) else { panic!("still placed") };
    assert_eq!((slid.offset, slid.size), ([0.4, 0.0], frame.size));

    s.undo().unwrap();

    assert_eq!(projection(&s, layer), axis, "placing and moving is one step");

    set(&mut s, layer, json!({"projection": "warp", "warpCorners": [[-0.5, 0.5], [0.9, 0.9], [0.5, -0.5], [-0.5, -0.5]]}));
    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "rotation": 30})).unwrap();
    let Projection::Warp { frame, corners, .. } = projection(&s, layer) else { panic!("still a warp") };
    assert_eq!((frame.rotation, corners[1]), (30.0, [0.9, 0.9]), "a warp's frame turns and keeps its corners");

    set(&mut s, layer, json!({"projection": "spherical"}));

    assert!(execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "rotation": 30})).is_err(), "a spherical projection has no frame");
}

#[test]
fn placing_a_cropped_axis_planar_preserves_coverage_through_save_and_reload() {
    let mesh = primitives::builtin("sphere").unwrap();
    let mut session = Session::new_project("Axis planar", mesh, "builtin:sphere", None, 128).unwrap();
    let layer = checker_fill(&mut session);
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 128])).write_to(&mut png, image::ImageFormat::Png).unwrap();
    let image = session.import_resource_bytes("translucent.png", png.into_inner()).unwrap();
    execute(&mut session, "layer.fill.set", &json!({"layer": layer.0, "channel": "baseColor", "image": image})).unwrap();
    set(&mut session, layer, json!({"projection": "planar", "axis": 2, "scale": 1.8, "shapeCrop": "cropped", "depthCulling": false}));
    let before = snapshot(&mut session);

    assert!(before.iter().any(|&red| red > 0.99), "the axis fill ignores source alpha in its crop");
    assert!(before.iter().any(|&red| red < 0.6), "the shape still crops coverage");

    execute(&mut session, "layer.decal.set", &json!({"layer": layer.0})).unwrap();
    let placed = projection(&session, layer);
    let after = snapshot(&mut session);

    assert!(placed.placed().is_some());
    let changed = before.iter().zip(&after).filter(|(before, after)| (*before - *after).abs() > 2.0 / 255.0).count();
    assert!(changed * 200 < before.len(), "placement changed visible coverage at {changed} texels");

    let bytes = session.to_project_bytes().unwrap();
    let mut reopened = Session::from_project_bytes(&bytes).unwrap();

    assert_eq!(projection(&reopened, layer), placed);
    let reloaded = snapshot(&mut reopened);
    let changed = reloaded.iter().zip(&after).filter(|(reloaded, after)| reloaded != after).count();
    assert_eq!(changed, 0, "reload changed visible coverage");
}

/// An axis planar reading a procedural keeps its axis: a placed frame reads the pattern in its own uv, so placing it would move the pattern; a mask fill's pattern stays the same way
#[test]
fn an_axis_planar_reading_a_pattern_is_not_placed() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    set(&mut s, layer, json!({"projection": "planar", "axis": 0, "scale": 1.5}));
    let axis = projection(&s, layer);
    let before = snapshot(&mut s);

    assert!(execute(&mut s, "layer.decal.set", &json!({"layer": layer.0})).is_err(), "placing would move the pattern");
    assert_eq!(projection(&s, layer), axis, "the planar keeps its axis");
    assert_eq!(snapshot(&mut s), before, "and the pattern stays where it was");

    let added = execute(&mut s, "effect.add.fill", &json!({"layer": layer.0, "target": "mask", "projection": "planar"})).unwrap();
    let effect = added["effect"].as_u64().unwrap();
    execute(&mut s, "layer.fill.set", &json!({"effect": effect, "procedural": "Checker"})).unwrap();

    assert!(execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "effect": effect})).is_err(), "a mask fill's pattern keeps its axis too");
}

/// Backface culling under an uneven or mirrored projector scale cannot keep its facing in a frame: moving it is refused with the way out, and nothing changes
#[test]
fn a_frame_under_uneven_culling_is_refused_with_the_way_out_and_changes_nothing() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    let ramp = s.import_resource_bytes("ramp.png", ramp_png()).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"layer": layer.0, "channel": "baseColor", "image": ramp})).unwrap();
    set(&mut s, layer, json!({"projection": "planar", "axis": 2, "backfaceCulling": true}));

    // An axis planar with an uneven then a mirrored scale, then a placed frame with an uneven one
    for (scale, placed) in [(json!([2, 1, 1]), false), (json!([1.5, -1.5, 1.5]), false), (json!([2, 1, 1]), true)] {
        let edit = if placed { json!({"layer": layer.0, "frameOffset": [0.1, 0, 0]}) } else { json!({"layer": layer.0}) };
        if placed {
            set(&mut s, layer, json!({"projectorScale": 1}));
            execute(&mut s, "layer.decal.set", &json!({"layer": layer.0})).unwrap();

            assert!(projection(&s, layer).placed().is_some(), "an even scale places the frame");
        }

        set(&mut s, layer, json!({"projectorScale": scale}));
        let before = projection(&s, layer);
        let steps = s.history_labels().len();

        let refused = execute(&mut s, "layer.decal.set", &edit).unwrap_err().to_string();

        assert!(refused.contains("even out the projector scale or turn backface culling off"), "{scale} placed {placed}: {refused}");
        assert_eq!(projection(&s, layer), before, "{scale} placed {placed}: the projection is unchanged");
        assert_eq!(s.history_labels().len(), steps, "{scale} placed {placed}: no undo step");
    }

    // A placed frame's edit that moves no frame still lands
    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "flipU": true})).unwrap();

    assert!(projection(&s, layer).frame().is_some_and(|frame| frame.flip[0]), "the flip lands on the placed frame");
}

/// A frame's 3D projection settings (Offset, Rotation, Scale as the panel and the gizmo share them) move it in one undo step, and a turned or scaled projector folds into the frame first, landing the same
#[test]
fn a_frame_takes_its_offset_rotation_and_scale_and_folds_its_projector_in_first() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    let ramp = s.import_resource_bytes("ramp.png", ramp_png()).unwrap();
    execute(&mut s, "layer.fill.set", &json!({"layer": layer.0, "channel": "baseColor", "image": ramp})).unwrap();
    set(&mut s, layer, json!({"projection": "planar", "axis": 2}));
    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0})).unwrap();

    // Disable depth culling here because rerasterizing a turned frame can change silhouette texels
    set(&mut s, layer, json!({"projectorRotation": [0, 0, 30], "projectorScale": 1.25, "depthCulling": false}));
    let turned = snapshot(&mut s);
    let steps = s.history_labels().len();

    // Moving the frame as shown: the turn and scale are in the frame now, the image where it was
    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "frameOffset": [0.0, 0.0, 0.0]})).unwrap();
    let Projection::Planar { placed: Some(frame), projector, .. } = projection(&s, layer) else { panic!("still placed") };

    assert_eq!((projector.rotation, projector.scale), ([0.0; 3], [1.0; 3]), "folded into the frame");

    let (lo, hi) = s.mesh.bounds();
    assert!((Vec3::from(frame.center) - (lo + hi) * 0.5).length() < 1e-4, "offset 0 = the mesh's centre: {frame:?}");

    let shown = texturecraft_engine::compose::frame_placement::placement(&projection(&s, layer), (lo, hi)).unwrap();

    assert!((shown.rotation[2] - 30.0).abs() < 1e-3 && shown.rotation[0].abs() < 1e-3, "the projector's turn shows as the frame's: {shown:?}");

    let folded = snapshot(&mut s);
    let changed = turned.iter().zip(&folded).filter(|(a, b)| (*a - *b).abs() > 2.0 / 255.0).count();
    assert!(changed * 200 < turned.len(), "the frame was already at the centre: the image stays, {changed} of {} texels changed", turned.len());

    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "frameScale": [shown.scale[0] * 0.5, shown.scale[1], shown.scale[2]], "merge": true}))
        .unwrap();
    let Projection::Planar { placed: Some(narrow), .. } = projection(&s, layer) else { panic!("still placed") };
    assert!((narrow.size[0] - frame.size[0] * 0.5).abs() < 1e-4 && (narrow.size[1] - frame.size[1]).abs() < 1e-4, "Scale X halves the width");

    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "frameRotation": [0, 0, 90], "merge": true})).unwrap();
    let again = texturecraft_engine::compose::frame_placement::placement(&projection(&s, layer), (lo, hi)).unwrap();

    assert!((again.rotation[2] - 90.0).abs() < 1e-3 && (again.scale[0] - shown.scale[0] * 0.5).abs() < 1e-4, "{again:?}");
    assert_eq!(s.history_labels().len(), steps + 1, "a drag is one undo step");

    s.undo().unwrap();

    assert_eq!(snapshot(&mut s), turned, "undo puts the turned projector back");

    let before_invalid_edit = projection(&s, layer);
    let undo_steps = s.history_labels().len();
    for bad in [
        json!({"frameScale": [1, 2]}),
        json!({"frameOffset": "x"}),
        json!({"frameScale": [0, -1, "deep"]}),
        json!({"offset": [0.2, "x", 0.1]}),
        json!({"offset": 0.1}),
        json!({"size": [0.1, 0.2, 0.3]}),
        json!({"size": [0.1, "x"]}),
    ] {
        let mut params = bad.clone();
        params["layer"] = json!(layer.0);

        assert!(execute(&mut s, "layer.decal.set", &params).is_err(), "{bad}");
    }

    assert_eq!(projection(&s, layer), before_invalid_edit);
    assert_eq!(s.history_labels().len(), undo_steps);

    // A collapsed or turned-over scale is clamped to the smallest frame, not refused
    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "frameScale": [0, -1, 0]})).unwrap();
    let Projection::Planar { placed: Some(smallest), .. } = projection(&s, layer) else { panic!("still placed") };
    assert!(smallest.size.iter().chain([&smallest.depth]).all(|side| *side > 0.0 && side.is_finite()), "{smallest:?}");

    // A warp's Scale Z is its reach: Projection Depth times the longer side
    set(&mut s, layer, json!({"projection": "warp"}));
    let Projection::Warp { frame, .. } = projection(&s, layer) else { panic!("a warp") };
    let diagonal = (hi - lo).length();
    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "frameScale": [frame.size[0] / diagonal, frame.size[1] / diagonal, 0.1]})).unwrap();
    let Projection::Warp { frame, depth, .. } = projection(&s, layer) else { panic!("a warp") };
    assert!((depth * frame.size[0].max(frame.size[1]) - 0.1 * diagonal).abs() < 1e-4, "reach {depth}");
}

/// A decal edit that moves nothing and keeps its image adds no undo step and leaves the project unmodified; one that moves it adds one
#[test]
fn a_decal_edit_that_changes_nothing_adds_no_undo_step() {
    let mut s = session();
    let ramp = s.import_resource_bytes("ramp.png", ramp_png()).unwrap();
    let added = execute(&mut s, "layer.add.decal", &json!({"image": ramp, "position": [0, 0, 1], "normal": [0, 0, 1], "rotation": 30})).unwrap();
    let layer = added["layer"].as_u64().unwrap();
    let mask = execute(&mut s, "layer.add.decal", &json!({"image": ramp, "channel": "mask", "position": [0, 0, 1], "normal": [0, 0, 1]})).unwrap();
    let mask_effect = mask["maskEffect"].as_u64().unwrap();
    let steps = s.history_labels().len();

    for unchanged in [
        json!({"layer": layer}),
        json!({"layer": layer, "rotation": 30}),
        json!({"layer": layer, "image": ramp}),
        json!({"layer": layer, "offset": [0, 0]}),
        json!({"effect": mask_effect}),
        json!({"effect": mask_effect, "image": ramp}),
    ] {
        execute(&mut s, "layer.decal.set", &unchanged).unwrap();

        assert_eq!(s.history_labels().len(), steps, "{unchanged} changes nothing");
    }

    execute(&mut s, "layer.decal.set", &json!({"layer": layer, "rotation": 45})).unwrap();

    assert_eq!(s.history_labels().len(), steps + 1, "a turn is an edit");
}

/// Inspect shows the frame a decal was placed on, for a decal placed as a mask as for one placed as a layer
#[test]
fn inspect_shows_a_mask_decals_frame_as_a_layer_decals() {
    let mut s = session();
    let ramp = s.import_resource_bytes("ramp.png", ramp_png()).unwrap();
    let placement = json!({"image": ramp, "position": [0, 0, 1], "normal": [0, 0, 1], "size": 0.5});
    let layer_decal = execute(&mut s, "layer.add.decal", &placement).unwrap();
    let mut mask_placement = placement.clone();
    mask_placement["channel"] = json!("mask");
    let mask_decal = execute(&mut s, "layer.add.decal", &mask_placement).unwrap();

    let inspect = execute(&mut s, "project.inspect", &Value::Null).unwrap();
    let layers = inspect["layers"].as_array().unwrap();
    let row = |id: &Value| layers.iter().find(|layer| &layer["id"] == id).unwrap().clone();
    let layer_frame = row(&layer_decal["layer"])["fill"]["decal"].clone();
    let mask_effects = row(&mask_decal["layer"])["mask"]["effects"].clone();
    let mask_effect = mask_effects.as_array().unwrap().iter().find(|effect| effect["id"] == mask_decal["maskEffect"]).unwrap().clone();

    // Each side holds a frame on the requested point and normal, so two missing frames cannot match as nulls
    let on_request = |frame: &Value| {
        let near = |key: &str, want: [f64; 3]| {
            frame[key]
                .as_array()
                .is_some_and(|got| got.len() == 3 && got.iter().zip(want).all(|(got, want)| got.as_f64().is_some_and(|got| (got - want).abs() < 1e-3)))
        };
        near("center", [0.0, 0.0, 1.0]) && near("normal", [0.0, 0.0, 1.0])
    };
    for (name, frame) in [
        ("layer decal", &layer_frame),
        ("layer decal reply", &layer_decal["decal"]),
        ("mask decal", &mask_effect["decal"]),
        ("mask decal reply", &mask_decal["decal"]),
    ] {
        assert!(on_request(frame), "the {name} shows its frame on the requested placement: {frame}");
    }

    assert_eq!(layer_frame, layer_decal["decal"], "a layer decal's frame");
    assert_eq!(mask_effect["decal"], mask_decal["decal"], "a mask decal's frame");
}

/// `depth` on a warp sets how far it reaches: its Projection Depth becomes the depth over the frame's longer side
#[test]
fn a_warps_depth_sets_its_projection_depth() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    set(&mut s, layer, json!({"projection": "warp"}));

    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "depth": 0.5})).unwrap();
    let Projection::Warp { frame, depth, .. } = projection(&s, layer) else { panic!("a warp") };

    assert!((depth * frame.size[0].max(frame.size[1]) - 0.5).abs() < 1e-5, "reach {depth} × {:?}", frame.size);
}

#[test]
fn a_zero_depth_warp_can_move_its_frame() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    set(&mut s, layer, json!({"projection": "warp", "projectionDepth": 0}));
    let before = projection(&s, layer);

    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "frameOffset": [0.2, 0, 0], "frameRotation": [0, 10, 0]})).unwrap();

    let Projection::Warp { frame, depth, .. } = projection(&s, layer) else { panic!("a warp") };
    let original = before.frame().unwrap();

    assert_ne!(frame.center, original.center);
    assert_ne!(frame.normal, original.normal);
    assert_eq!(depth, 0.0);
}

#[test]
fn a_procedural_mask_keeps_array_placement_scale_separate_from_pattern_scale() {
    use texturecraft_engine::core::{EffectKind, Source};
    let mut s = session();
    let layer = checker_fill(&mut s);
    let added = execute(&mut s, "effect.add.fill", &json!({"layer": layer.0, "target": "mask", "procedural": "Checker", "scale": 4})).unwrap();
    let effect = added["effect"].as_u64().unwrap();

    execute(&mut s, "effect.set", &json!({"effect": effect, "scale": [2, 3]})).unwrap();
    execute(&mut s, "effect.set", &json!({"effect": effect, "scale": 5})).unwrap();

    let (_, effect) = s.doc.project.effect(LayerId(effect)).unwrap();
    let EffectKind::Fill { source: Source::Procedural { pattern, .. }, projection: Projection::Uv { scale, .. }, .. } = &effect.kind else {
        panic!("a procedural UV mask fill");
    };

    assert_eq!(*scale, [2.0, 3.0]);
    assert_eq!(pattern.scale, 5.0);
}

/// `depth` at a warp's current reach changes nothing, so it adds no undo step (its frame's unused depth is left alone)
#[test]
fn a_warps_current_reach_as_depth_adds_no_undo_step() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    set(&mut s, layer, json!({"projection": "warp", "projectionDepth": 0.5}));
    let Projection::Warp { frame, depth, .. } = projection(&s, layer) else { panic!("a warp") };
    let steps = s.history_labels().len();

    let reach = depth * frame.size[0].max(frame.size[1]);
    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "depth": reach})).unwrap();

    assert_eq!(s.history_labels().len(), steps, "reach {reach} is the one it has");
}

/// A frameScale reach on a warp keeps its Projection Depth within the range `projectionDepth` allows
#[test]
fn a_warps_frame_scale_keeps_its_projection_depth_in_range() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    set(&mut s, layer, json!({"projection": "warp"}));

    let deepest = checker_fill(&mut s);
    set(&mut s, deepest, json!({"projection": "warp", "projectionDepth": 1e9}));
    let Projection::Warp { depth: max_depth, .. } = projection(&s, deepest) else { panic!("a warp") };

    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "frameScale": [0.0001, 0.0001, 10]})).unwrap();
    let Projection::Warp { depth, .. } = projection(&s, layer) else { panic!("a warp") };

    assert!(depth <= max_depth, "Projection Depth {depth} past {max_depth}");
}

/// A 64 × 8 PNG whose red climbs from left to right
fn ramp_png() -> Vec<u8> {
    let mut out = Vec::new();
    let ramp = image::RgbaImage::from_fn(64, 8, |x, _| image::Rgba([(x * 4) as u8, 0, 0, 255]));
    ramp.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// Reset fill settings puts the Fill section back to the defaults of the projection it uses, in one undo step, and leaves the projection kind, the symmetry and sources alone
#[test]
fn reset_fill_settings_keeps_the_projection_and_restores_its_defaults_in_one_undo_step() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    let fresh = snapshot(&mut s);
    set(
        &mut s,
        layer,
        json!({"projection": "planar", "tiling": [3, 2], "rotation": 30, "axis": 0, "filtering": "nearest", "wrap": "none", "scaleMode": "physicalSize", "shapeCrop": "cropped", "symmetry": true, "symmetryAxis": 1}),
    );
    let fill = |s: &Session| match &s.doc.project.layer(layer).unwrap().kind {
        LayerKind::Fill(f) => f.clone(),
        _ => panic!("a fill layer"),
    };
    let changed = fill(&s);

    assert!(!texturecraft_engine::fill_reset_cmds::at_defaults(&changed.projection, changed.sampling));

    execute(&mut s, "layer.fill.resetProjection", &json!({"layer": layer.0})).unwrap();
    let reset = fill(&s);
    let Projection::Planar { scale, axis, offset, uv, projector, placed } = reset.projection else { panic!("still planar, got {:?}", reset.projection) };
    assert_eq!((scale, axis, offset, uv, placed), (1.0, 2, [0.0; 3], UvTransform::default(), None), "a new planar's placement");
    assert_eq!(projector, Projector::depth_culled(), "a new planar's 3D settings and culling");
    assert_eq!(reset.sampling, Sampling { symmetry: changed.sampling.symmetry, ..Sampling::default() }, "the sampling is a new fill's; the symmetry stays");
    assert_eq!(reset.sources, changed.sources, "the material stays");
    assert!(texturecraft_engine::fill_reset_cmds::at_defaults(&reset.projection, reset.sampling));

    let steps = s.history_labels().len();
    execute(&mut s, "layer.fill.resetProjection", &json!({"layer": layer.0})).unwrap();

    assert_eq!(s.history_labels().len(), steps, "resetting a Fill section at its defaults adds no undo step");

    execute(&mut s, "edit.undo", &Value::Null).unwrap();

    assert_eq!(fill(&s), changed, "one undo brings every setting back");
    assert!(execute(&mut s, "layer.fill.resetProjection", &json!({"layer": 9999})).is_err(), "no such layer");

    // A UV fill comes back to a UV projection at tiling 1 and renders like a new fill
    set(&mut s, layer, json!({"projection": "uv", "tiling": [3, 2], "rotation": 30}));
    execute(&mut s, "layer.fill.resetProjection", &json!({"layer": layer.0})).unwrap();

    assert_eq!(fill(&s).projection, Projection::default());
    assert_eq!(snapshot(&mut s), fresh, "it renders like the new fill");
}

/// A warp's reset keeps its frame where it is placed and restores its corners, Projection Depth, UV transform and projector settings
#[test]
fn reset_fill_settings_keeps_a_warp_frame_and_restores_its_settings() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    set(&mut s, layer, json!({"projection": "warp"}));
    let Projection::Warp { frame: fitted, .. } = projection(&s, layer) else { panic!("a warp") };
    execute(&mut s, "layer.decal.set", &json!({"layer": layer.0, "offset": [0.2, 0.1]})).unwrap();
    set(
        &mut s,
        layer,
        json!({"warpCorners": [[-0.6, 0.5], [0.5, 0.6], [0.4, -0.5], [-0.5, -0.4]], "projectionDepth": 0.5, "tiling": [2, 2], "rotation": 45, "shapeCrop": "extendsOutside"}),
    );
    let Projection::Warp { frame: moved, .. } = projection(&s, layer) else { panic!("a warp") };

    assert_ne!(moved, fitted, "the frame was moved");

    execute(&mut s, "layer.fill.resetProjection", &json!({"layer": layer.0})).unwrap();

    assert_eq!(projection(&s, layer), Projection::warp(moved), "still a warp: its frame stays, every setting is a new warp's");
}

#[test]
fn malformed_warp_corners_copy_flips_and_a_warps_own_placement_are_rejected_and_change_nothing() {
    let mut s = session();
    let layer = checker_fill(&mut s);
    set(&mut s, layer, json!({"projection": "warp"}));
    let before = projection(&s, layer);
    let steps = s.history_labels().len();

    for bad in [
        json!({"warpCorners": [[1, 2]]}),
        json!({"warpCorners": [[0, 0], [0, 0], [0, 0], ["x", 0]]}),
        json!({"warpCorners": "pulled"}),
        json!({"flipCopy": [true, "x"]}),
        // A warp is placed by its frame (layer.decal.set): a scale or offset of its own would change nothing
        json!({"scale": 2}),
        json!({"offset": [1, 0, 0]}),
        json!({"projection": "warp", "scale": 2, "offset": [1, 0, 0]}),
    ] {
        let mut p = bad.clone();
        p["layer"] = json!(layer.0);

        assert!(execute(&mut s, "layer.fill.projection", &p).is_err(), "{bad}");
    }

    assert_eq!(projection(&s, layer), before);
    assert_eq!(s.history_labels().len(), steps, "no undo step for a rejected edit");
}
