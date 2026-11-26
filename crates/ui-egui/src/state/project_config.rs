/// Edit > Project Configuration: choices waiting for OK
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ProjectConfig {
    /// A mesh file picked with Select..., waiting for OK
    pub mesh: Option<String>,
    /// The user's Reimport mesh selection
    pub reimport: bool,
    /// Whether painted strokes follow the surface of a newly imported mesh
    pub preserve: bool,
    /// The selected normal map format when it differs from the project's
    pub normal_format: Option<texturecraft_engine::core::NormalFormat>,
    /// Color management as chosen in the window; nothing is applied (the project stays Legacy)
    pub color_management: crate::project_form::ColorManagement,
    /// Whether Enter requested confirmation in the next frame
    pub confirm: bool,
    /// The project that was open when this window opened
    pub project: u64,
    /// This opening's identity, used to reject late file picker answers
    pub(crate) opening_id: u64,
}
