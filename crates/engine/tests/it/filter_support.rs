//! What the filter tests share: their sessions, adding, setting and refusing a filter, and reading the composite

use serde_json::{Value, json};
use texturecraft_engine::core::{Channel, EffectKind, Filter, LayerId};
use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::geom::primitives;
use texturecraft_engine::{Session, execute};

/// The texture set side the filter tests paint on
pub(crate) const SIZE: u32 = 128;

/// The viewport's side in pixels when a test reads the texel under its center
const VIEWPORT_SIDE: f32 = 400.0;

/// A sphere on a `SIZE` texture set, rendered once
pub(crate) fn session() -> Session {
    let mut s = sized_session(SIZE);
    s.update();
    s
}

/// A sphere on a texture set of side `size`, not yet rendered
pub(crate) fn sized_session(size: u32) -> Session {
    Session::new_project("t", primitives::sphere(1.0, 48, 24), "builtin:sphere", None, size).unwrap()
}

/// A session whose camera looks straight at the sphere, so the viewport center reads one texel (see [`center`])
pub(crate) fn facing_session() -> Session {
    let mut s = session();
    s.viewport = Vec2::splat(VIEWPORT_SIDE);
    s.camera.yaw = 0.0;
    s.camera.pitch = 0.0;

    s.update();
    s
}

/// The UV under the viewport center
pub(crate) fn center_uv(s: &mut Session) -> Vec2 {
    s.update();
    s.pick(Vec2::splat(VIEWPORT_SIDE / 2.0), s.viewport).unwrap().uv
}

/// Composited value of `channel` under the viewport center
pub(crate) fn center(s: &mut Session, channel: Channel) -> [f32; 3] {
    let uv = center_uv(s);
    s.caches[0].composite.value(channel, (uv.x * SIZE as f32) as u32, (uv.y * SIZE as f32) as u32)
}

/// The composited roughness at texel `(x, y)`
pub(crate) fn roughness(s: &mut Session, (x, y): (u32, u32)) -> f32 {
    s.update();
    s.caches[0].composite.value(Channel::Roughness, x, y)[0]
}

/// Content filter `params` on the selected layer: the effect
pub(crate) fn add_filter(s: &mut Session, params: Value) -> LayerId {
    let mut params = params;
    params["target"] = json!("content");

    LayerId(execute(s, "effect.add.filter", &params).unwrap()["effect"].as_u64().unwrap())
}

/// A fill layer of base color `color` and roughness 0.3 under content filter `params`: the effect
pub(crate) fn filtered(s: &mut Session, color: [f32; 3], params: Value) -> LayerId {
    execute(s, "layer.add.fill", &json!({"color": color, "roughness": 0.3})).unwrap();
    add_filter(s, params)
}

/// The filter `effect` holds
pub(crate) fn filter_of(s: &Session, effect: LayerId) -> Filter {
    match &s.doc.project.effect(effect).unwrap().1.kind {
        EffectKind::Filter(filter) => filter.clone(),
        other => panic!("not a filter: {other:?}"),
    }
}

/// `effect.set` on `effect` with `params`, which must succeed
pub(crate) fn set(s: &mut Session, effect: LayerId, params: Value) {
    let mut params = params;
    params["effect"] = json!(effect.0);

    execute(s, "effect.set", &params).unwrap();
}

/// Each of `bad` is refused by `effect.set` on `effect`
pub(crate) fn refused(s: &mut Session, effect: LayerId, bad: &[Value]) {
    for bad in bad {
        let mut params = bad.clone();
        params["effect"] = json!(effect.0);

        assert!(execute(s, "effect.set", &params).is_err(), "{bad}");
    }
}
