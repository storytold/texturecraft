use serde::{Deserialize, Serialize};

/// What the environment is fixed to (Display settings > Environment Alignment)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum EnvAlignment {
    /// The environment stays with the scene: orbiting the camera looks around it
    #[default]
    World,
    /// The environment turns with the camera (about the vertical axis): the lighting stays the same while orbiting
    Camera,
}

impl EnvAlignment {
    pub const ALL: [EnvAlignment; 2] = [EnvAlignment::World, EnvAlignment::Camera];

    pub fn label(self) -> &'static str {
        match self {
            EnvAlignment::World => "World",
            EnvAlignment::Camera => "Camera",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            EnvAlignment::World => "world",
            EnvAlignment::Camera => "camera",
        }
    }

    pub fn parse(id: &str) -> Option<EnvAlignment> {
        Self::ALL.into_iter().find(|alignment| alignment.id().eq_ignore_ascii_case(id))
    }
}
