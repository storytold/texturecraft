//! A brush stroke that starts on a texture set other than the active one paints nothing (only the active set takes paint): say why, once per stroke

use texturecraft_engine::geom::glam::Vec2;
use texturecraft_engine::{Session, Tool};

use crate::TextureCraftApp;
use crate::i18n::trf;

/// Tools that paint into the active texture set with the brush
fn paints_active_set(tool: Tool) -> bool {
    matches!(tool, Tool::Paint | Tool::Eraser | Tool::Projection | Tool::Smudge | Tool::Clone)
}

/// The name of the texture set under viewport pixel `screen` when a brush stroke started there would paint nothing because that set is not the active one
pub fn inactive_set_under(session: &Session, screen: Vec2) -> Option<String> {
    if !paints_active_set(session.tool.tool) {
        return None;
    }

    let set = session.texture_set_at(screen, session.viewport).filter(|set| *set != session.ts())?;
    session.doc.project.texture_sets.get(set).map(|texture_set| texture_set.name.clone())
}

/// Show the toast for a stroke that starts at viewport pixel `screen` (call once, when the stroke begins). Returns whether it did.
pub fn hint_on_stroke_start(app: &mut TextureCraftApp, screen: Vec2) -> bool {
    let Some(name) = inactive_set_under(&app.session, screen) else { return false };
    app.toast(trf("Texture set '{name}' is not active for painting. Select it in the Texture Sets list.", &[("name", &name)]), false);
    true
}
