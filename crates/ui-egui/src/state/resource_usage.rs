/// What an imported resource is for
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ResourceUsage {
    /// Imported as an image resource, nothing more
    #[default]
    Undefined,
    /// An image resource that also becomes the brush alpha
    Alpha,
    /// Color lookup tables (not available yet)
    ColorLut,
    /// An image resource that also lights the scene
    Environment,
    /// Post effect resources (not available yet)
    PostFx,
    /// An image resource for fills and stencils
    Texture,
}

impl ResourceUsage {
    /// The usages in the dropdown's order
    pub const ALL: [ResourceUsage; 6] = [Self::Undefined, Self::Alpha, Self::ColorLut, Self::Environment, Self::PostFx, Self::Texture];

    /// The name shown in the dropdowns (the same in every language, as in the reference)
    pub fn name(self) -> &'static str {
        match self {
            Self::Undefined => "undefined",
            Self::Alpha => "alpha",
            Self::ColorLut => "colorlut",
            Self::Environment => "environment",
            Self::PostFx => "postfx",
            Self::Texture => "texture",
        }
    }

    /// Whether the app can do something with resources of this usage
    pub fn available(self) -> bool {
        !matches!(self, Self::ColorLut | Self::PostFx)
    }
}
