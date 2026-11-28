//! TextureCraft's colours handed to craft-ui, so the dock's frames, tab strips and splitters draw in our theme instead of the suite's default grey

use std::sync::Arc;

use craft_theme::{ColorToken, ModePins, Pins, Rgba, SUITE_DEFAULT, ThemeDef, ThemeId, builtins};
use egui::{Color32, Context, Id};

use crate::theme::Tokens;

/// The id of TextureCraft's theme, apart from every built-in one
const THEME_ID: &str = "texturecraft.app";

/// The theme craft-ui resolves its tokens from (`RootEnv::library_theme`), kept while the tokens stay the same so it resolves once per theme change
pub(crate) fn library_theme(ctx: &Context, tokens: &Tokens) -> Option<Arc<ThemeDef>> {
    let pins = app_pins(tokens);
    let memory_id = Id::unique(THEME_ID);
    let kept: Option<Arc<ThemeDef>> = ctx.data(|data| data.get_temp(memory_id));
    if let Some(def) = kept.filter(|def| def.pins.dark == pins) {
        return Some(def);
    }

    let def = Arc::new(app_def(pins)?);
    ctx.data_mut(|data| data.insert_temp(memory_id, Arc::clone(&def)));
    Some(def)
}

/// The suite default with TextureCraft's colours pinned in both modes (the root's mode picks one); `None` if the suite default is missing
fn app_def(pins: Pins) -> Option<ThemeDef> {
    let mut def = builtins().iter().find(|theme| theme.id.as_str() == SUITE_DEFAULT)?.clone();
    def.id = ThemeId::parse(THEME_ID).ok()?;
    def.pins = ModePins { light: pins.clone(), dark: pins };
    Some(def)
}

fn app_pins(tokens: &Tokens) -> Pins {
    [
        (ColorToken::SurfaceBackground, rgba(tokens.chrome)),
        (ColorToken::SurfacePanel, rgba(tokens.panel)),
        (ColorToken::LineBorder, rgba(tokens.separator)),
        (ColorToken::AccentSolid, rgba(tokens.accent)),
        // The shown tab's name is brighter than the others, as on the panel headers
        (ColorToken::TextNormal, rgba(tokens.text_strong)),
        (ColorToken::TextMuted, rgba(tokens.text_dim)),
    ]
    .into_iter()
    .collect()
}

fn rgba(color: Color32) -> Rgba {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    Rgba([r, g, b, a])
}
