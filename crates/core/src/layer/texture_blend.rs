use serde::{Deserialize, Serialize};

/// How a generator's texture combines with its signal
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextureBlend {
    /// The texture replaces the signal
    Normal,
    /// The texture adds to the signal
    Add,
    /// The texture is taken from the signal
    Subtract,
    /// The texture multiplies the signal
    Multiply,
    /// The texture adds above its middle gray and subtracts below it
    AddSub,
    /// The brighter of the texture and the signal
    Max,
    /// The darker of the texture and the signal
    Min,
    /// The texture darkens the signal's darks and lightens its lights
    #[default]
    Overlay,
    /// The inverse of multiplying the inverses: always lighter
    Screen,
    /// A gentler overlay led by the texture
    SoftLight,
}

impl TextureBlend {
    pub const ALL: [TextureBlend; 10] = [
        TextureBlend::Normal,
        TextureBlend::Add,
        TextureBlend::Subtract,
        TextureBlend::Multiply,
        TextureBlend::AddSub,
        TextureBlend::Max,
        TextureBlend::Min,
        TextureBlend::Overlay,
        TextureBlend::Screen,
        TextureBlend::SoftLight,
    ];

    /// The name the panel shows
    pub fn label(self) -> &'static str {
        match self {
            TextureBlend::Normal => "Normal",
            TextureBlend::Add => "Linear Dodge (Add)",
            TextureBlend::Subtract => "Subtract",
            TextureBlend::Multiply => "Multiply",
            TextureBlend::AddSub => "Add/Sub",
            TextureBlend::Max => "Max (Lighten)",
            TextureBlend::Min => "Min (Darken)",
            TextureBlend::Overlay => "Overlay",
            TextureBlend::Screen => "Screen",
            TextureBlend::SoftLight => "Soft light",
        }
    }

    /// The name commands and saved projects use
    pub fn id(self) -> &'static str {
        match self {
            TextureBlend::Normal => "normal",
            TextureBlend::Add => "add",
            TextureBlend::Subtract => "subtract",
            TextureBlend::Multiply => "multiply",
            TextureBlend::AddSub => "addSub",
            TextureBlend::Max => "max",
            TextureBlend::Min => "min",
            TextureBlend::Overlay => "overlay",
            TextureBlend::Screen => "screen",
            TextureBlend::SoftLight => "softLight",
        }
    }

    /// The blend named by its id or its label, in any case
    pub fn parse(name: &str) -> Option<TextureBlend> {
        TextureBlend::ALL.into_iter().find(|blend| blend.id().eq_ignore_ascii_case(name) || blend.label().eq_ignore_ascii_case(name))
    }
}
