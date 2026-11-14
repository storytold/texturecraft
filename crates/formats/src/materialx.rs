//! TextureCraft's MaterialX export adapter.
//!
//! The shared `craft-materialx` crate owns the MaterialX document writer. This module maps the
//! built-in preset's map names and TextureCraft color spaces to that format-level API.

use texturecraft_texel::ColorSpace;

/// How an OpenPBR input is typed in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `color3` from an sRGB texture.
    Color,
    /// `float` from a linear texture.
    Float,
    /// `vector3` through a `normalmap` node.
    Normal,
    /// `float` driving a `displacement` shader.
    Height,
}

/// The OpenPBR inputs the export writes, in document order. Names are the file-name suffixes of
/// the built-in OpenPBR preset's maps (`$mesh_$textureSet_<input>`).
pub const INPUTS: &[(&str, Kind)] = &[
    ("base_color", Kind::Color),
    ("base_metalness", Kind::Float),
    ("specular_roughness", Kind::Float),
    ("specular_roughness_anisotropy", Kind::Float),
    ("coat_weight", Kind::Float),
    ("coat_roughness", Kind::Float),
    ("fuzz_weight", Kind::Float),
    ("fuzz_color", Kind::Color),
    ("fuzz_roughness", Kind::Float),
    ("transmission_weight", Kind::Float),
    ("subsurface_weight", Kind::Float),
    ("emission_color", Kind::Color),
    ("geometry_opacity", Kind::Float),
    ("geometry_normal", Kind::Normal),
    ("height", Kind::Height),
];

const PREFIX: &str = "$mesh_$textureSet_";

/// One exported texture: the OpenPBR input it feeds and the file (relative to the `.mtlx`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Texture {
    pub input: String,
    pub file: String,
    /// The MaterialX colour space of the file's values; used for color inputs only.
    pub colorspace: &'static str,
}

/// The MaterialX colour space of colour maps written in `space`.
pub fn colorspace_name(space: ColorSpace) -> &'static str {
    match space {
        ColorSpace::Srgb => "srgb_texture",
        ColorSpace::LinearRec709 => "lin_rec709",
        ColorSpace::AcesCg => "acescg",
        ColorSpace::Aces2065 => "lin_ap0_scene",
    }
}

/// The OpenPBR input a preset map's file template feeds, or None for unsupported templates.
pub fn input_of_template(template: &str) -> Option<&'static str> {
    let suffix = template.strip_prefix(PREFIX)?;
    INPUTS.iter().find(|(name, _)| *name == suffix).map(|(name, _)| *name)
}

/// A MaterialX name: letters, digits and underscores, not starting with a digit.
pub fn node_name(name: &str) -> String {
    let mut out: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    out
}

/// The text of a MaterialX OpenPBR document for `name` and the given texture files.
pub fn write(name: &str, textures: &[Texture], physical_size_cm: Option<[f32; 2]>) -> String {
    let textures: Vec<craft_materialx::Texture> = textures
        .iter()
        .map(|texture| craft_materialx::Texture { input: texture.input.clone(), file: texture.file.clone(), colorspace: texture.colorspace.to_owned() })
        .collect();
    craft_materialx::write_open_pbr(name, &textures, physical_size_cm)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex(input: &str, file: &str) -> Texture {
        Texture { input: input.into(), file: file.into(), colorspace: "srgb_texture" }
    }

    /// Every start tag is closed or self-closed, in order: enough to catch a malformed writer.
    fn balanced(xml: &str) -> bool {
        let mut stack: Vec<String> = Vec::new();
        let mut rest = xml;
        while let Some(i) = rest.find('<') {
            let Some(j) = rest.get(i..).and_then(|r| r.find('>')) else { return false };
            let Some(tag) = rest.get(i + 1..i + j) else { return false };
            rest = rest.get(i + j + 1..).unwrap_or("");
            if tag.starts_with('?') || tag.starts_with('!') {
                continue;
            }
            if let Some(name) = tag.strip_prefix('/') {
                if stack.pop().as_deref() != Some(name.trim()) {
                    return false;
                }
            } else if !tag.ends_with('/') {
                stack.push(tag.split_whitespace().next().unwrap_or("").to_owned());
            }
        }
        stack.is_empty()
    }

    #[test]
    fn a_full_material_maps_every_input() {
        let t: Vec<Texture> = INPUTS.iter().map(|(n, _)| tex(n, &format!("stone_{n}.png"))).collect();
        let x = write("Stone", &t, Some([100.0, 50.0]));
        assert!(balanced(&x), "{x}");
        for want in [
            "<materialx version=\"1.39\"",
            "<open_pbr_surface name=\"OPBR_Stone\" type=\"surfaceshader\">",
            "name=\"base_color\" type=\"color3\" nodename=\"image_base_color\"",
            "name=\"base_metalness\" type=\"float\" nodename=\"image_base_metalness\"",
            "name=\"specular_roughness\" type=\"float\" nodename=\"image_specular_roughness\"",
            "name=\"coat_weight\" type=\"float\" nodename=\"image_coat_weight\"",
            "name=\"fuzz_color\" type=\"color3\" nodename=\"image_fuzz_color\"",
            "name=\"transmission_weight\" type=\"float\"",
            "name=\"subsurface_weight\" type=\"float\"",
            "name=\"geometry_opacity\" type=\"float\" nodename=\"image_geometry_opacity\"",
            "name=\"geometry_normal\" type=\"vector3\" nodename=\"normalmap\"",
            "name=\"emission_color\" type=\"color3\" nodename=\"image_emission_color\"",
            "name=\"emission_luminance\" type=\"float\" value=\"1\"",
            "value=\"stone_base_color.png\" colorspace=\"srgb_texture\"",
            "value=\"stone_emission_color.png\" colorspace=\"srgb_texture\"",
            "<normalmap name=\"normalmap\" type=\"vector3\">",
            "<displacement name=\"DISP_Stone\" type=\"displacementshader\">",
            "name=\"displacementshader\"",
            "<surfacematerial name=\"Stone\" type=\"material\" doc=\"Tiles every 100 x 50 cm\">",
        ] {
            assert!(x.contains(want), "missing {want}\n{x}");
        }
        // Data textures carry no colour space, and height is not an OpenPBR input.
        assert!(x.contains("value=\"stone_specular_roughness.png\"/>"));
        assert!(!x.contains("name=\"height\""));
    }

    #[test]
    fn only_what_is_there_is_written() {
        let x = write("m", &[tex("specular_roughness", "r.png")], None);
        assert!(balanced(&x));
        assert!(!x.contains("base_color") && !x.contains("normalmap") && !x.contains("displacement") && !x.contains("emission_luminance"));
        assert!(x.contains("<surfacematerial name=\"m\" type=\"material\">"));
        let none = write("m", &[], None);
        assert!(balanced(&none) && none.contains("<open_pbr_surface") && !none.contains("tiledimage"));
    }

    #[test]
    fn colour_inputs_carry_their_files_space_and_data_inputs_none() {
        let lin = |input: &str, file: &str| Texture { colorspace: "lin_rec709", ..tex(input, file) };
        let x = write("m", &[lin("base_color", "c.exr"), lin("specular_roughness", "r.exr")], None);
        assert!(x.contains("value=\"c.exr\" colorspace=\"lin_rec709\"") && x.contains("value=\"r.exr\"/>"), "{x}");
    }

    #[test]
    fn unknown_and_repeated_inputs_are_skipped() {
        let x = write("m", &[tex("ambient_occlusion", "ao.png"), tex("base_color", "a.png"), tex("base_color", "b.png")], None);
        assert!(!x.contains("ao.png") && x.contains("a.png") && !x.contains("b.png"));
    }

    #[test]
    fn templates_map_to_inputs() {
        assert_eq!(input_of_template("$mesh_$textureSet_base_color"), Some("base_color"));
        assert_eq!(input_of_template("$mesh_$textureSet_height"), Some("height"));
        assert_eq!(input_of_template("$mesh_$textureSet_orm"), None);
        assert_eq!(input_of_template("base_color"), None);
        for (n, _) in INPUTS {
            assert_eq!(input_of_template(&format!("{PREFIX}{n}")), Some(*n));
        }
    }

    #[test]
    fn udim_tokens_and_hostile_names_are_escaped() {
        assert_eq!(node_name("Rough Stone #2"), "Rough_Stone__2");
        assert_eq!(node_name("9lives"), "_9lives");
        assert_eq!(node_name(""), "_");
        let x = write("a<b>&\"c", &[tex("base_color", "x\"&<'>.<UDIM>.png")], Some([f32::NAN, 1.0]));
        assert!(balanced(&x.replace("&lt;UDIM&gt;", "UDIM")), "{x}");
        let doc = craft_materialx::Document::parse(x.as_bytes()).unwrap();
        let material = doc.read_material(None).unwrap();
        assert!(material.inputs.iter().any(|(input, value)| input == "base_color" && value == &craft_materialx::Value::Image("x\"&<'>.<UDIM>.png".into())));
        assert!(!x.contains("doc="), "a non-finite size is left out");
        assert!(x.contains("OPBR_a_b___c"));
    }
}
