use serde::{Deserialize, Serialize};

/// How an axis of a World Space Normals or Position generator combines with the axes before it
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AxisBlend {
    /// The axis replaces what is below it
    Normal,
    /// The axis adds to what is below it
    Add,
    /// The axis is taken from what is below it
    Subtract,
    /// The axis multiplies what is below it
    Multiply,
    /// The axis adds above its middle gray and subtracts below it
    AddSub,
    /// The brighter of the axis and what is below it
    Max,
}

impl AxisBlend {
    pub const ALL: [AxisBlend; 6] = [AxisBlend::Normal, AxisBlend::Add, AxisBlend::Subtract, AxisBlend::Multiply, AxisBlend::AddSub, AxisBlend::Max];

    /// The name the panel shows
    pub fn label(self) -> &'static str {
        match self {
            AxisBlend::Normal => "Normal",
            AxisBlend::Add => "Linear Dodge (Add)",
            AxisBlend::Subtract => "Subtract",
            AxisBlend::Multiply => "Multiply",
            AxisBlend::AddSub => "Add/Sub",
            AxisBlend::Max => "Max (Lighten)",
        }
    }

    /// The name commands and saved projects use
    pub fn id(self) -> &'static str {
        match self {
            AxisBlend::Normal => "normal",
            AxisBlend::Add => "add",
            AxisBlend::Subtract => "subtract",
            AxisBlend::Multiply => "multiply",
            AxisBlend::AddSub => "addSub",
            AxisBlend::Max => "max",
        }
    }

    /// The blend named by its id or its label, in any case
    pub fn parse(name: &str) -> Option<AxisBlend> {
        AxisBlend::ALL.into_iter().find(|blend| blend.id().eq_ignore_ascii_case(name) || blend.label().eq_ignore_ascii_case(name))
    }
}
