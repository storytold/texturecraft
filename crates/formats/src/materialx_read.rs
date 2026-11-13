//! MaterialX material import adapted to TextureCraft's channel model.
//!
//! XML parsing, nodegraph traversal and shader input resolution live in `craft-materialx`.
//! This module maps those format-level values to TextureCraft's channels and color handling.

use texturecraft_core::{Channel, ChannelKind};
use texturecraft_texel::srgb_to_linear;

use crate::{FormatError, Result, malformed};

const KIND: &str = "MaterialX document";
/// Largest document read, in bytes.
pub const MAX_BYTES: usize = craft_materialx::MAX_BYTES;

/// What one channel is set from.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// A constant: linear RGB for colours, `[v, v, v]` for scalars.
    Constant([f32; 3]),
    /// An image file, as written in the document (`fileprefix` applied), relative to it.
    Image(String),
}

/// A material read from a document.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    /// The channels the material sets, in a fixed order.
    pub channels: Vec<(Channel, Value)>,
    /// The first image's `uvtiling` (1, 1 when none).
    pub uv_tiling: [f32; 2],
    /// What was skipped and why.
    pub warnings: Vec<String>,
}

fn channel(shader: craft_materialx::Shader, input: &str) -> Option<Channel> {
    use craft_materialx::Shader::{GltfPbr, OpenPbr, StandardSurface};
    Some(match (shader, input) {
        (OpenPbr, "base_color") | (StandardSurface, "base_color") | (GltfPbr, "base_color") => Channel::BaseColor,
        (OpenPbr, "base_metalness") | (StandardSurface, "metalness") | (GltfPbr, "metallic") => Channel::Metallic,
        (OpenPbr, "specular_roughness") | (StandardSurface, "specular_roughness") | (GltfPbr, "roughness") => Channel::Roughness,
        (OpenPbr, "geometry_normal") | (StandardSurface, "normal") | (GltfPbr, "normal") => Channel::Normal,
        (OpenPbr, "geometry_opacity") | (StandardSurface, "opacity") | (GltfPbr, "alpha") => Channel::Opacity,
        (OpenPbr, "emission_color") | (StandardSurface, "emission_color") | (GltfPbr, "emission") => Channel::Emissive,
        (OpenPbr, "coat_weight") | (StandardSurface, "coat") | (GltfPbr, "clearcoat") => Channel::CoatWeight,
        (OpenPbr, "coat_roughness") | (StandardSurface, "coat_roughness") | (GltfPbr, "clearcoat_roughness") => Channel::CoatRoughness,
        (OpenPbr, "geometry_coat_normal") | (StandardSurface, "coat_normal") | (GltfPbr, "clearcoat_normal") => Channel::CoatNormal,
        (OpenPbr, "fuzz_weight") | (StandardSurface, "sheen") => Channel::SheenWeight,
        (OpenPbr, "fuzz_color") | (StandardSurface, "sheen_color") | (GltfPbr, "sheen_color") => Channel::SheenColor,
        (OpenPbr, "fuzz_roughness") | (StandardSurface, "sheen_roughness") | (GltfPbr, "sheen_roughness") => Channel::SheenRoughness,
        (OpenPbr, "specular_roughness_anisotropy") | (StandardSurface, "specular_anisotropy") => Channel::AnisotropyLevel,
        (OpenPbr, "transmission_weight") | (StandardSurface, "transmission") | (GltfPbr, "transmission") => Channel::Transmission,
        (OpenPbr, "subsurface_weight") | (StandardSurface, "subsurface") => Channel::ScatterWeight,
        (GltfPbr, "occlusion") => Channel::AmbientOcclusion,
        _ => return None,
    })
}

fn constant(channel: Channel, values: &[f32], colorspace: Option<&str>) -> [f32; 3] {
    let mut values = values.to_vec();
    if let Some(space) = colorspace
        && (space.starts_with("srgb") || space.starts_with("g22") || space.starts_with("g18"))
        && values.len() >= 3
    {
        values.iter_mut().take(3).for_each(|c| *c = srgb_to_linear(*c));
    }
    match (channel.kind(), values.as_slice()) {
        (_, [a]) => [*a; 3],
        (ChannelKind::Color, [r, g, b, ..]) => [*r, *g, *b],
        (_, [r, g, b, ..]) => {
            let mean = (r + g + b) / 3.0;
            [mean; 3]
        }
        _ => Channel::default_value(channel),
    }
}

fn app_value(channel: Channel, value: craft_materialx::Value) -> Value {
    match value {
        craft_materialx::Value::Constant { values, colorspace } => Value::Constant(constant(channel, &values, colorspace.as_deref())),
        craft_materialx::Value::Image(file) => Value::Image(file),
    }
}

/// Read the material called `material` (default: the first) from a `.mtlx` document.
pub fn read_materialx(bytes: &[u8], material: Option<&str>) -> Result<Material> {
    crate::guard(KIND, || read_inner(bytes, material))
}

fn read_inner(bytes: &[u8], material: Option<&str>) -> Result<Material> {
    let document = craft_materialx::Document::parse(bytes).map_err(map_error)?;
    let shared = document.read_material(material).map_err(map_error)?;
    let mut channels = Vec::new();
    for (input, value) in shared.inputs {
        if let Some(channel) = channel(shared.shader, &input) {
            channels.push((channel, app_value(channel, value)));
        }
    }
    if let Some(value) = shared.displacement {
        channels.push((Channel::Height, app_value(Channel::Height, value)));
    }
    Ok(Material { name: shared.name, channels, uv_tiling: shared.uv_tiling, warnings: shared.warnings })
}

fn map_error(error: craft_materialx::Error) -> FormatError {
    use craft_materialx::Error;
    match error {
        Error::Xml(xml) if xml.is_limit() => FormatError::TooLarge(xml.to_string()),
        Error::MissingMaterial(name) => FormatError::Invalid(format!("no material named {name:?}")),
        Error::Unsupported(message) => FormatError::Unsupported(message),
        Error::Malformed(message) => malformed(KIND, message),
        Error::Root => malformed(KIND, "no <materialx> element"),
        Error::Xml(other) => malformed(KIND, other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPENPBR: &str = r#"<?xml version="1.0"?>
<materialx version="1.39" colorspace="lin_rec709">
  <open_pbr_surface name="S" type="surfaceshader">
    <input name="base_color" type="color3" nodename="img_base" />
    <input name="base_metalness" type="float" value="1" />
    <input name="specular_roughness" type="float" nodename="img_rough" />
    <input name="geometry_normal" type="vector3" nodename="nm" />
    <input name="emission_color" type="color3" value="1, 0.5, 0" />
    <input name="emission_luminance" type="float" value="2" />
    <input name="coat_weight" type="float" value="0.25" />
  </open_pbr_surface>
  <tiledimage name="img_base" type="color3">
    <input name="file" type="filename" value="stone_base.png" colorspace="srgb_texture" />
    <input name="uvtiling" type="vector2" value="4, 2" />
  </tiledimage>
  <tiledimage name="img_rough" type="float"><input name="file" type="filename" value="stone_r.png" /></tiledimage>
  <tiledimage name="img_n" type="vector3"><input name="file" type="filename" value="stone_n.png" /></tiledimage>
  <normalmap name="nm" type="vector3"><input name="in" type="vector3" nodename="img_n" /></normalmap>
  <tiledimage name="img_h" type="float"><input name="file" type="filename" value="stone_h.png" /></tiledimage>
  <displacement name="D" type="displacementshader"><input name="displacement" type="float" nodename="img_h" /></displacement>
  <surfacematerial name="Stone" type="material">
    <input name="surfaceshader" type="surfaceshader" nodename="S" />
    <input name="displacementshader" type="displacementshader" nodename="D" />
  </surfacematerial>
</materialx>"#;

    fn get(m: &Material, c: Channel) -> Option<&Value> {
        m.channels.iter().find(|(ch, _)| *ch == c).map(|(_, v)| v)
    }

    #[test]
    fn open_pbr_inputs_become_channels() {
        let m = read_materialx(OPENPBR.as_bytes(), None).unwrap();
        assert_eq!(m.name, "Stone");
        assert_eq!(get(&m, Channel::BaseColor), Some(&Value::Image("stone_base.png".into())));
        assert_eq!(get(&m, Channel::Roughness), Some(&Value::Image("stone_r.png".into())));
        assert_eq!(get(&m, Channel::Normal), Some(&Value::Image("stone_n.png".into())), "through the normalmap node");
        assert_eq!(get(&m, Channel::Height), Some(&Value::Image("stone_h.png".into())), "from the displacement shader");
        assert_eq!(get(&m, Channel::Metallic), Some(&Value::Constant([1.0; 3])));
        assert_eq!(get(&m, Channel::CoatWeight), Some(&Value::Constant([0.25; 3])));
        assert_eq!(get(&m, Channel::Emissive), Some(&Value::Constant([1.0, 0.5, 0.0])));
        assert_eq!(m.uv_tiling, [4.0, 2.0]);
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        let extreme = OPENPBR.replace(r#"value="4, 2""#, r#"value="1e9, 1e-9""#);
        assert_eq!(read_materialx(extreme.as_bytes(), None).unwrap().uv_tiling, [1000.0, 0.001], "clamped like a fill's UV scale");
    }

    #[test]
    fn nodegraph_input_without_output_uses_the_first_output() {
        let xml = r#"<materialx version="1.38">
  <nodegraph name="NG">
    <tiledimage name="a" type="color3"><input name="file" type="filename" value="first.png" /></tiledimage>
    <tiledimage name="b" type="color3"><input name="file" type="filename" value="second.png" /></tiledimage>
    <output name="o1" type="color3" nodename="a" />
    <output name="o2" type="color3" nodename="b" />
  </nodegraph>
  <standard_surface name="S" type="surfaceshader"><input name="base_color" type="color3" nodegraph="NG" /></standard_surface>
  <surfacematerial name="M" type="material"><input name="surfaceshader" type="surfaceshader" nodename="S" /></surfacematerial>
</materialx>"#;
        for _ in 0..50 {
            let material = read_materialx(xml.as_bytes(), None).unwrap();
            assert_eq!(get(&material, Channel::BaseColor), Some(&Value::Image("first.png".into())));
        }
    }

    #[test]
    fn emission_is_off_unless_the_shader_emits() {
        let x = OPENPBR.replace(r#"<input name="emission_luminance" type="float" value="2" />"#, "");
        let m = read_materialx(x.as_bytes(), None).unwrap();
        assert!(get(&m, Channel::Emissive).is_none());
        assert!(m.warnings.iter().any(|w| w.contains("emission")), "{:?}", m.warnings);
        let zero = OPENPBR.replace(r#"value="2" />"#, r#"value="0" />"#);
        assert!(get(&read_materialx(zero.as_bytes(), None).unwrap(), Channel::Emissive).is_none());
    }

    #[test]
    fn standard_surface_nodegraphs_prefixes_and_colour_spaces() {
        let x = r#"<materialx version="1.38" fileprefix="tex/">
  <nodegraph name="NG">
    <tiledimage name="t" type="color3"><input name="file" type="filename" value="a.png" /></tiledimage>
    <output name="out" type="color3" nodename="t" />
  </nodegraph>
  <standard_surface name="SS" type="surfaceshader">
    <input name="base_color" type="color3" nodegraph="NG" output="out" />
    <input name="specular_roughness" type="float" value="0.4" />
    <input name="opacity" type="color3" value="0.5, 0.5, 0.5" />
    <input name="sheen_color" type="color3" value="0.5, 0.5, 0.5" colorspace="srgb_texture" />
    <input name="base" type="float" value="0.5" />
  </standard_surface>
  <surfacematerial name="M1" type="material"><input name="surfaceshader" type="surfaceshader" nodename="SS" /></surfacematerial>
  <surfacematerial name="M2" type="material"><input name="surfaceshader" type="surfaceshader" nodename="SS" /></surfacematerial>
</materialx>"#;
        let m = read_materialx(x.as_bytes(), Some("M2")).unwrap();
        assert_eq!(m.name, "M2");
        assert_eq!(get(&m, Channel::BaseColor), Some(&Value::Image("tex/a.png".into())), "through the nodegraph, with the prefix");
        assert_eq!(get(&m, Channel::Roughness), Some(&Value::Constant([0.4; 3])));
        assert_eq!(get(&m, Channel::Opacity), Some(&Value::Constant([0.5; 3])), "a colour on a scalar channel is averaged");
        let Some(Value::Constant(c)) = get(&m, Channel::SheenColor) else { panic!("sheen colour") };
        assert!((c[0] - 0.214).abs() < 1e-3, "sRGB 0.5 is linear 0.214: {c:?}");
        assert!(m.warnings.iter().any(|w| w.contains("base weight")), "{:?}", m.warnings);
        assert!(read_materialx(x.as_bytes(), Some("nope")).is_err());
    }

    #[test]
    fn unsupported_nodes_and_shaders_are_reported() {
        let x = OPENPBR.replace(r#"<tiledimage name="img_rough""#, r#"<multiply name="img_rough" type="float"/><tiledimage name="unused""#);
        let m = read_materialx(x.as_bytes(), None).unwrap();
        assert!(get(&m, Channel::Roughness).is_none());
        assert!(m.warnings.iter().any(|w| w.contains("multiply")), "{:?}", m.warnings);
        let other = r#"<materialx><UsdPreviewSurface name="U" type="surfaceshader"/><surfacematerial name="M" type="material"><input name="surfaceshader" type="surfaceshader" nodename="U"/></surfacematerial></materialx>"#;
        assert!(matches!(read_materialx(other.as_bytes(), None), Err(FormatError::Unsupported(_))));
    }

    #[test]
    fn hostile_documents_are_errors() {
        for bad in ["", "not xml", "<materialx>", "<a><b></a>", "<materialx/>", "<materialx><surfacematerial name=\"M\"/></materialx>"] {
            assert!(read_materialx(bad.as_bytes(), None).is_err(), "{bad:?}");
        }
        // A wiring loop and a deep tree stop instead of running forever or overflowing.
        let loop_doc = r#"<materialx><open_pbr_surface name="S"><input name="base_color" nodename="c"/></open_pbr_surface>
            <convert name="c"><input name="in" nodename="c"/></convert>
            <surfacematerial name="M"><input name="surfaceshader" nodename="S"/></surfacematerial></materialx>"#;
        let m = read_materialx(loop_doc.as_bytes(), None).unwrap();
        assert!(m.channels.is_empty() && m.warnings.iter().any(|w| w.contains("loops")), "{:?}", m.warnings);
        let deep = format!("<materialx>{}{}</materialx>", "<a>".repeat(200), "</a>".repeat(200));
        assert!(matches!(read_materialx(deep.as_bytes(), None), Err(FormatError::TooLarge(_))));
        // Numbers: NaN, inf and over-long lists are not values.
        let nan = OPENPBR.replace(r#"value="0.25""#, r#"value="NaN""#);
        assert!(get(&read_materialx(nan.as_bytes(), None).unwrap(), Channel::CoatWeight).is_none());
    }

    #[test]
    fn xml_entities_in_file_names_are_decoded() {
        let x = OPENPBR.replace("stone_base.png", "a&amp;b &lt;1&gt;.png");
        let m = read_materialx(x.as_bytes(), None).unwrap();
        assert_eq!(get(&m, Channel::BaseColor), Some(&Value::Image("a&b <1>.png".into())));
    }

    #[test]
    fn mutated_documents_never_panic() {
        // Truncations and byte flips of a real document: errors or materials, never a panic.
        let bytes = OPENPBR.as_bytes();
        for cut in (0..bytes.len()).step_by(7) {
            let _ = read_materialx(&bytes[..cut], None);
        }
        for i in (0..bytes.len()).step_by(5) {
            let mut b = bytes.to_vec();
            b[i] ^= 0x5a;
            let _ = read_materialx(&b, None);
        }
    }
}
