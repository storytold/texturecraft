use super::ImageInput;
use texturecraft_core::{LinkParam, LinkPreset};

#[derive(Clone, Debug, serde::Deserialize)]
pub struct MaterialInterface {
    pub name: String,
    pub graph: String,
    pub parameters: Vec<LinkParam>,
    pub inputs: Vec<ImageInput>,
    pub presets: Vec<LinkPreset>,
}
