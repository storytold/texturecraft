//! Texture export: render the maps of an [`ExportPreset`] from a texture set's composite, baked
//! mesh maps and UV coverage, entirely in memory (so the web build can export too).
//!
//! Each [`MapSpec`] packs sources into the R, G, B, A (or L) channels of one file. Sources are
//! composite channels, virtual maps (final normal in OpenGL or DirectX convention, glossiness,
//! mixed AO, F0), baked mesh maps and constants. The final normal is the baked normal, the
//! composited normal channel and the normal derived from height combined with Reoriented Normal
//! Mapping. By default, color maps are sRGB-encoded in integer formats and linear in float
//! formats. Data maps are always linear. Height is stored with 0.5 = zero (`h * 0.5 + 0.5`) in every format.
//!
//! A UV-tiled (UDIM) texture set writes one file per map per tile: `$udim` in the file name
//! becomes the tile number, and names without it get `.$udim` appended. Padding never crosses
//! tiles.

use std::collections::{BTreeSet, HashMap, hash_map::Entry};
use std::io::Cursor;

use craft_image_codecs::{ChannelLayout, EncodeOptions, Format as CodecFormat, Image as CodecImage, SampleType};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use texturecraft_compose::{BakedMaps, Composite, SeamPaddingInput};
use texturecraft_core::{AoMixing, BakedMap, BlockScales, Channel, ChannelKind, NormalFormat, Project, TextureSet};
use texturecraft_geom::glam::Vec3;
use texturecraft_texel::{ColorSpace, linear_to_srgb};

use crate::{FormatError, MAX_IMAGE_SIZE, Result};

mod encoding;
mod layered_psd;
mod map_override;
mod map_preview;
mod map_renderer;
mod planned_layer;
mod psd_budget;
mod rendered_map;
use encoding::Encoding;
pub use map_override::MapOverride;
pub use map_preview::{MAX_PREVIEW_SIDE, MapPreview};
use map_renderer::MapRenderer;
pub use planned_layer::PlannedLayer;
use psd_budget::PsdBudget;

/// Height-to-normal slope: with strength 1, a height change of 1 across 1/16 of the texture
/// width tilts the normal 45°, at any resolution.
const HEIGHT_SPAN: f32 = 16.0;
const JPEG_QUALITY: u8 = 95;
/// Glossiness a set lacks: the KHR_materials_pbrSpecularGlossiness glossinessFactor default
const MISSING_GLOSSINESS: SourceRef = SourceRef::Constant(1.0);
const PSD_MAX_SIDE_PIXELS: u32 = 30_000;
const PSD_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Planes of a PSD image or layer: red, green, blue and alpha
const PSD_RGBA_PLANES: usize = 4;
/// Preset limits (user presets are untrusted JSON); maps leave room for one per channel plus packed maps
const MAX_MAPS: usize = 128;
const MAX_TEMPLATE: usize = 256;
const MAX_STEM: usize = 200;
/// Longest preset name.
pub const MAX_PRESET_NAME: usize = 64;

/// File name template variables with what they become (see [`list_outputs`]).
pub const FILE_TOKENS: [(&str, &str); 6] = [
    ("$mesh", "Mesh file name"),
    ("$textureSet", "Texture set name"),
    ("$project", "Project name"),
    ("$channel", "Channel or map id"),
    ("$colorSpace", "Colour maps: sRGB, linear, ACEScg or ACES2065-1; data maps: linear"),
    ("$udim", "UV tile number (1001 for sets without UV tiles)"),
];

/// Why `template` is not a valid file name template. `/` separates folders inside the output
/// directory; a backslash, `..`, a leading `/`, an empty folder name, control characters and
/// unknown `$` variables are refused.
pub fn template_error(template: &str) -> Option<String> {
    if template.trim().is_empty() || template.len() > MAX_TEMPLATE {
        return Some(format!("file name \"{template}\" is empty or longer than {MAX_TEMPLATE} bytes"));
    }
    if template.contains('\\') || template.contains("..") || template.chars().any(char::is_control) {
        return Some(format!("file name \"{template}\" contains a backslash, \"..\" or a control character"));
    }
    if template.split('/').any(|segment| segment.trim().is_empty()) {
        return Some(format!("file name \"{template}\" starts or ends with \"/\" or has an empty folder name"));
    }
    let mut rest = template;
    while let Some(i) = rest.find('$') {
        let tail = rest.get(i..).unwrap_or_default();
        match FILE_TOKENS.iter().find(|(t, _)| tail.starts_with(t)) {
            Some((t, _)) => rest = tail.get(t.len()..).unwrap_or_default(),
            None => return Some(format!("file name \"{template}\" has an unknown variable (use {})", FILE_TOKENS.map(|(t, _)| t).join(", "))),
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FileFormat {
    Png,
    Jpeg,
    Tga,
    Tiff,
    Bmp,
    Webp,
    Exr,
    Hdr,
    /// JPEG Network Graphics: JPEG colour with a lossless alpha.
    Jng,
    /// Portable graymap (binary).
    Pgm,
    /// Photoshop document: every output map of a texture set in this format as a layer of one file
    Psd,
}

impl FileFormat {
    /// Whether the texture can be referenced by the USD asset exporter.
    pub fn usable_by_usd_asset(self) -> bool {
        self != Self::Psd
    }

    pub const ALL: [FileFormat; 11] = [
        FileFormat::Png,
        FileFormat::Jpeg,
        FileFormat::Tga,
        FileFormat::Tiff,
        FileFormat::Bmp,
        FileFormat::Webp,
        FileFormat::Exr,
        FileFormat::Hdr,
        FileFormat::Jng,
        FileFormat::Pgm,
        FileFormat::Psd,
    ];

    pub fn extension(self) -> &'static str {
        match self {
            FileFormat::Png => "png",
            FileFormat::Jpeg => "jpg",
            FileFormat::Tga => "tga",
            FileFormat::Tiff => "tif",
            FileFormat::Bmp => "bmp",
            FileFormat::Webp => "webp",
            FileFormat::Exr => "exr",
            FileFormat::Hdr => "hdr",
            FileFormat::Jng => "jng",
            FileFormat::Pgm => "pgm",
            FileFormat::Psd => "psd",
        }
    }

    /// The name the export window lists (`from_extension` reads it back).
    pub fn name(self) -> &'static str {
        match self {
            FileFormat::Jpeg => "jpeg",
            FileFormat::Tga => "targa",
            FileFormat::Tiff => "tiff",
            other => other.extension(),
        }
    }

    pub fn from_extension(ext: &str) -> Option<FileFormat> {
        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
        match ext.as_str() {
            "jpeg" => Some(FileFormat::Jpeg),
            "tiff" => Some(FileFormat::Tiff),
            "targa" => Some(FileFormat::Tga),
            _ => FileFormat::ALL.into_iter().find(|f| f.extension() == ext),
        }
    }

    /// Bit depths this format is written with (16 and 32 are half and full float for EXR, 32 is
    /// float for TIFF and HDR).
    pub fn bit_depths(self) -> &'static [u8] {
        match self {
            FileFormat::Png | FileFormat::Pgm | FileFormat::Psd => &[8, 16],
            FileFormat::Tiff => &[8, 16, 32],
            FileFormat::Exr => &[16, 32],
            FileFormat::Hdr => &[32],
            _ => &[8],
        }
    }

    /// The bit depth a file of this format gets unless told otherwise.
    pub fn default_depth(self) -> u8 {
        if self.is_float() { 32 } else { 8 }
    }

    /// Floating-point formats hold linear values.
    pub fn is_float(self) -> bool {
        matches!(self, FileFormat::Exr | FileFormat::Hdr)
    }

    /// Whether a file of this format at `depth` holds floats (32-bit TIFF does).
    pub fn holds_float(self, depth: u8) -> bool {
        self.is_float() || (self == FileFormat::Tiff && depth == 32)
    }

    /// The supported depth closest to `want` (ties go to the deeper one).
    pub fn depth(self, want: u8) -> u8 {
        self.bit_depths().iter().copied().min_by_key(|d| (d.abs_diff(want), std::cmp::Reverse(*d))).unwrap_or(8)
    }
}

/// Destination channel of an output file. `L` alone writes grayscale; `L` + `A` gray + alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Dest {
    R,
    G,
    B,
    A,
    L,
}

/// Maps computed at export time from several channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VirtualMap {
    /// Final tangent-space normal, +Y up the image.
    NormalOpenGl,
    /// Final tangent-space normal, green flipped.
    NormalDirectX,
    /// 1 − roughness.
    Glossiness,
    /// Baked AO × AO channel (per the texture set's AO mixing).
    AoMixed,
    /// Dielectric reflectance at normal incidence: Specular Level scaled by the project's shader settings.
    F0,
    /// 1 − glossiness (specular/glossiness projects).
    Roughness,
    /// Tangent-space normal from the height channel alone (the export's normal convention).
    HeightNormal,
    /// Sheen color × sheen weight (glTF's sheen color, a color map).
    SheenTint,
}

impl VirtualMap {
    pub const ALL: [VirtualMap; 8] = [
        VirtualMap::NormalOpenGl,
        VirtualMap::NormalDirectX,
        VirtualMap::HeightNormal,
        VirtualMap::Glossiness,
        VirtualMap::Roughness,
        VirtualMap::AoMixed,
        VirtualMap::F0,
        VirtualMap::SheenTint,
    ];

    pub fn id(self) -> &'static str {
        match self {
            VirtualMap::NormalOpenGl | VirtualMap::NormalDirectX | VirtualMap::HeightNormal => "normal",
            VirtualMap::Glossiness => "glossiness",
            VirtualMap::Roughness => "roughness",
            VirtualMap::AoMixed => "ambientOcclusion",
            VirtualMap::F0 => "f0",
            VirtualMap::SheenTint => "sheenTint",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            VirtualMap::NormalOpenGl => "Normal OpenGL",
            VirtualMap::NormalDirectX => "Normal DirectX",
            VirtualMap::HeightNormal => "Normal from height",
            VirtualMap::Glossiness => "Glossiness from roughness",
            VirtualMap::Roughness => "Roughness from glossiness",
            VirtualMap::AoMixed => "Mixed AO",
            VirtualMap::F0 => "F0 from specular level",
            VirtualMap::SheenTint => "Sheen color × weight",
        }
    }

    /// 3 for normals and colors, 1 otherwise.
    pub fn components(self) -> usize {
        match self {
            VirtualMap::NormalOpenGl | VirtualMap::NormalDirectX | VirtualMap::HeightNormal | VirtualMap::SheenTint => 3,
            _ => 1,
        }
    }
}

/// Where one output component comes from. `component` picks r/g/b of 3-component sources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SourceRef {
    /// A composited channel. `Normal` means the final normal in the export normal format.
    Channel {
        channel: Channel,
        #[serde(default)]
        component: u8,
    },
    Virtual {
        map: VirtualMap,
        #[serde(default)]
        component: u8,
    },
    Baked {
        map: BakedMap,
        #[serde(default)]
        component: u8,
    },
    /// A constant (0 = black, 0.5 = gray, 1 = white).
    Constant(f32),
}

impl SourceRef {
    fn raw_component(&self) -> usize {
        match self {
            SourceRef::Channel { component, .. } | SourceRef::Virtual { component, .. } | SourceRef::Baked { component, .. } => *component as usize,
            SourceRef::Constant(_) => 0,
        }
    }

    fn component(&self) -> usize {
        self.raw_component().min(2)
    }

    /// Whether the source is a tangent-space normal, which pads in 2D whatever the set's UV padding
    fn pads_flat(&self) -> bool {
        match self {
            SourceRef::Channel { channel, .. } => channel.kind() == ChannelKind::NormalMap,
            SourceRef::Virtual { map, .. } => matches!(map, VirtualMap::NormalOpenGl | VirtualMap::NormalDirectX | VirtualMap::HeightNormal),
            SourceRef::Baked { map, .. } => matches!(map, BakedMap::Normal | BakedMap::BentNormal),
            SourceRef::Constant(_) => false,
        }
    }

    /// Components the source has (1 or 3; constants are 1).
    pub fn components(&self) -> usize {
        match self {
            SourceRef::Channel { channel, .. } => channel.components(),
            SourceRef::Virtual { map, .. } => map.components(),
            SourceRef::Baked { map, .. } => map.components(),
            SourceRef::Constant(_) => 1,
        }
    }
}

/// One exported file. `file` is a name template without extension (see [`list_outputs`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapSpec {
    pub file: String,
    pub format: FileFormat,
    pub bit_depth: u8,
    pub channels: Vec<(Dest, SourceRef)>,
    /// Destination channels written as `1 - value` (absent in old files: none). The source value
    /// is inverted before the colour-space encoding, so a colour map is inverted in linear light.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inverted: Vec<Dest>,
}

impl MapSpec {
    /// Whether this set supplies at least one source used by the output map.
    pub fn is_available_for(&self, set: &TextureSet) -> bool {
        self.channels.iter().any(|(_, source)| source_present(set, source))
    }

    /// Whether a non-alpha destination draws from a color source.
    pub fn is_color(&self) -> bool {
        self.channels.iter().any(|(dest, source)| {
            *dest != Dest::A
                && match source {
                    SourceRef::Channel { channel, .. } => channel.kind() == ChannelKind::Color,
                    SourceRef::Virtual { map, .. } => *map == VirtualMap::SheenTint,
                    _ => false,
                }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportPreset {
    pub name: String,
    pub maps: Vec<MapSpec>,
}

impl ExportPreset {
    /// Structural checks for (untrusted) presets.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(FormatError::Invalid(format!("export preset \"{}\": {m}", self.name)));
        if self.name.trim().is_empty() || self.name.chars().count() > MAX_PRESET_NAME || self.name.chars().any(char::is_control) {
            return bad(format!("the name must be 1 to {MAX_PRESET_NAME} characters without control characters"));
        }

        if self.maps.len() > MAX_MAPS {
            return bad(format!("more than {MAX_MAPS} maps"));
        }

        for m in &self.maps {
            if let Some(e) = template_error(&m.file) {
                return bad(e);
            }

            if !matches!(m.bit_depth, 8 | 16 | 32) {
                return bad(format!("map \"{}\" has bit depth {} (8, 16 or 32)", m.file, m.bit_depth));
            }

            if m.channels.iter().any(|(_, s)| s.component() != s.raw_component()) {
                return bad(format!("map \"{}\" picks a component other than 0, 1 or 2", m.file));
            }

            let dests: BTreeSet<Dest> = m.channels.iter().map(|(d, _)| *d).collect();
            if dests.len() != m.channels.len() || m.channels.is_empty() {
                return bad(format!("map \"{}\" needs distinct destination channels", m.file));
            }

            if dests.contains(&Dest::L) && dests.iter().any(|d| matches!(d, Dest::R | Dest::G | Dest::B)) {
                return bad(format!("map \"{}\" mixes L with R/G/B", m.file));
            }

            let inv: BTreeSet<Dest> = m.inverted.iter().copied().collect();
            if inv.len() != m.inverted.len() || !inv.is_subset(&dests) {
                return bad(format!("map \"{}\" inverts a channel it does not write, or one twice", m.file));
            }

            if m.channels.iter().any(|(_, s)| matches!(s, SourceRef::Constant(v) if !v.is_finite())) {
                return bad(format!("map \"{}\" has a non-finite constant", m.file));
            }
        }
        Ok(())
    }
}

/// UV padding of exported maps (texels outside the UV islands).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Padding {
    /// Uncovered texels are zero (transparent where there is alpha).
    None,
    /// Islands are dilated this many texels; beyond, texels are zero.
    Dilation(u32),
    /// Islands are dilated this many texels; beyond, texels hold the channel default.
    DilationDefault(u32),
    /// Islands are dilated this many texels; beyond, the texels are diffused (a smooth fill
    /// from the nearby island colours).
    DilationDiffusion(u32),
    /// Every texel takes its nearest island's value.
    Infinite,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportOptions {
    pub preset: ExportPreset,
    /// Write every map in this format instead of the preset's.
    pub format_override: Option<FileFormat>,
    pub bit_depth_override: Option<u8>,
    pub padding: Padding,
    /// Output size (power of two); None keeps the texture set resolution.
    pub size: Option<u32>,
    /// Normal convention for every normal output instead of the project's / preset's.
    pub normal_format: Option<NormalFormat>,
    /// UV-tiled sets: one image of the whole tile atlas per map instead of a file per tile
    /// (for formats without UDIM support, with the mesh's UVs in atlas space).
    pub atlas: bool,
    /// The colour space of the colour maps (data maps are never converted); None writes sRGB
    /// into integer files and linear Rec.709 into float files.
    #[serde(skip)]
    pub color_space: Option<ColorSpace>,
    /// Per map (by index into the preset's maps) settings.
    #[serde(default)]
    pub maps: std::collections::BTreeMap<usize, MapOverride>,
    /// Dither 8-bit integer files (an ordered pattern, the same on every export).
    #[serde(default)]
    pub dither: bool,
}

impl ExportOptions {
    /// The output size of the preset's map `map`: its own, else the export's (None: the texture set's)
    pub fn map_size(&self, map: usize) -> Option<u32> {
        self.maps.get(&map).and_then(|own| own.size).or(self.size)
    }

    /// The format, depth and dithering of a map before any PSD document-wide promotion.
    pub fn map_file_type(&self, map_index: usize, map: &MapSpec) -> (FileFormat, u8, bool) {
        let own = self.maps.get(&map_index);
        let format = own.and_then(|o| o.format).or(self.format_override).unwrap_or(map.format);
        let depth = format.depth(own.and_then(|o| o.bit_depth).or(self.bit_depth_override).unwrap_or(map.bit_depth));
        let dither = depth == 8 && !format.holds_float(depth) && own.and_then(|o| o.dither).unwrap_or(self.dither);
        (format, depth, dither)
    }

    /// The colour space colour maps use at this format's default depth.
    pub fn space_for(&self, format: FileFormat) -> ColorSpace {
        self.space_for_depth(format, format.default_depth())
    }

    /// The colour space colour maps are written in when `format` is used at `depth`.
    pub fn space_for_depth(&self, format: FileFormat, depth: u8) -> ColorSpace {
        let space = self.color_space.unwrap_or(if format.holds_float(depth) { ColorSpace::LinearRec709 } else { ColorSpace::Srgb });

        // PGM has one luminance channel and cannot represent ACES primaries
        if format == FileFormat::Pgm && space != ColorSpace::Srgb { ColorSpace::LinearRec709 } else { space }
    }

    /// The colour space of the written map samples after layout-specific fallbacks.
    pub fn space_for_map(&self, map: &MapSpec, format: FileFormat, depth: u8) -> ColorSpace {
        if !map.is_color() {
            return ColorSpace::LinearRec709;
        }

        let space = self.space_for_depth(format, depth);

        // L maps can use the integer sRGB transfer but have no RGB triplet to convert
        if map.channels.iter().any(|(dest, _)| *dest == Dest::L) && (space != ColorSpace::Srgb || format.holds_float(depth)) {
            ColorSpace::LinearRec709
        } else {
            space
        }
    }
}

impl ExportOptions {
    pub fn new(preset: ExportPreset) -> ExportOptions {
        ExportOptions {
            preset,
            format_override: None,
            bit_depth_override: None,
            padding: Padding::Infinite,
            size: None,
            normal_format: None,
            atlas: false,
            color_space: None,
            maps: Default::default(),
            dither: false,
        }
    }
}

/// What is exported: one texture set's composite with its baked maps and UV coverage (one flag
/// per composite texel, row-major). A preview can also borrow the UV rasterizer's triangle indices.
#[derive(Clone, Copy)]
pub struct ExportInput<'a, Coverage = bool> {
    pub project: &'a Project,
    pub ts: usize,
    pub composite: &'a Composite,
    pub baked: &'a BakedMaps,
    pub coverage: &'a [Coverage],
    /// The set's UV seams, which padding across seams (3D) crosses
    pub seams: Option<&'a SeamPaddingInput<'a>>,
    pub mesh_name: &'a str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportedFile {
    pub name: String,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub warnings: Vec<String>,
}

/// A file a preset produces for a texture set (see [`list_outputs`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedFile {
    pub name: String,
    /// Index into the preset's maps.
    pub map: usize,
    pub format: FileFormat,
    pub bit_depth: u8,
    /// 8-bit integer file written with dithering.
    pub dither: bool,
    /// UDIM number of the tile written (None: the whole set).
    pub tile: Option<u32>,
    /// For a tiled file, `name` with the tile number written as `<UDIM>` (what a material that
    /// covers every tile refers to); None when the file isn't tiled.
    pub udim_name: Option<String>,
    pub warnings: Vec<String>,
    /// A PSD's output maps, top layer first (`map` is the first); empty for a file of one map
    pub layers: Vec<PlannedLayer>,
}

impl PlannedFile {
    /// The output maps the file writes
    pub fn maps(&self) -> impl Iterator<Item = usize> + '_ {
        let own = self.layers.is_empty().then_some(self.map);
        own.into_iter().chain(self.layers.iter().map(|layer| layer.map))
    }
}

// ---------------------------------------------------------------------------------------------
// Planning: which maps exist for a texture set and what their files are called

fn normal_present(set: &TextureSet) -> bool {
    set.has_channel(Channel::Normal) || set.has_channel(Channel::Height) || set.baked.contains(&BakedMap::Normal)
}

/// A source with real data in this texture set (constants never count).
pub(crate) fn source_present(set: &TextureSet, s: &SourceRef) -> bool {
    match s {
        SourceRef::Channel { channel: Channel::Normal, .. } => normal_present(set),
        SourceRef::Channel { channel, .. } => set.has_channel(*channel),
        SourceRef::Virtual { map: VirtualMap::NormalOpenGl | VirtualMap::NormalDirectX, .. } => normal_present(set),
        SourceRef::Virtual { map: VirtualMap::Glossiness, .. } => set.has_channel(Channel::Roughness),
        SourceRef::Virtual { map: VirtualMap::AoMixed, .. } => set.has_channel(Channel::AmbientOcclusion) || set.baked.contains(&BakedMap::AmbientOcclusion),
        SourceRef::Virtual { map: VirtualMap::F0, .. } => set.has_channel(Channel::SpecularLevel),
        SourceRef::Virtual { map: VirtualMap::Roughness, .. } => set.has_channel(Channel::Glossiness),
        SourceRef::Virtual { map: VirtualMap::HeightNormal, .. } => set.has_channel(Channel::Height),
        SourceRef::Virtual { map: VirtualMap::SheenTint, .. } => set.has_channel(Channel::SheenWeight),
        SourceRef::Baked { map, .. } => set.baked.contains(map),
        SourceRef::Constant(_) => false,
    }
}

/// The written components in order: L | L A | R G B | R G B A. Alpha from a channel the set
/// does not have is dropped.
fn layout<'m>(set: &TextureSet, map: &'m MapSpec) -> Vec<(Dest, &'m SourceRef)> {
    let get = |d: Dest| map.channels.iter().find(|(x, _)| *x == d).map(|(_, s)| s);

    // A glossiness alpha stays, fully glossy, so a specular/glossiness file is RGBA without the channel too
    let alpha = match get(Dest::A) {
        Some(SourceRef::Channel { channel: Channel::Glossiness, .. }) if !set.has_channel(Channel::Glossiness) => Some(&MISSING_GLOSSINESS),
        alpha => alpha.filter(|s| !matches!(s, SourceRef::Channel { channel, .. } if !set.has_channel(*channel))),
    };
    let black = &SourceRef::Constant(0.0);
    let mut out = match get(Dest::L) {
        Some(l) => vec![(Dest::L, l)],
        None => [Dest::R, Dest::G, Dest::B].into_iter().map(|d| (d, get(d).unwrap_or(black))).collect(),
    };
    if let Some(a) = alpha {
        out.push((Dest::A, a));
    }
    out
}

/// What `$channel` becomes in `map`'s file names: the id of its first non-constant source.
pub fn channel_label(map: &MapSpec) -> &'static str {
    map.channels
        .iter()
        .find_map(|(_, s)| match s {
            SourceRef::Channel { channel, .. } => Some(channel.id()),
            SourceRef::Virtual { map, .. } => Some(map.id()),
            SourceRef::Baked { map, .. } => Some(map.id()),
            SourceRef::Constant(_) => None,
        })
        .unwrap_or("constant")
}

/// Keep `[A-Za-z0-9_.-]`, replace everything else with `_`; never empty or a reserved device name.
pub fn sanitize_file_stem(s: &str) -> String {
    sanitize_stem_with_limit(s, MAX_STEM)
}

fn sanitize_stem_with_limit(s: &str, limit: usize) -> String {
    let mut out: String = s.chars().take(limit).map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' }).collect();
    if out.trim_matches('.').is_empty() {
        out = "texture".into();
    }
    let device = out.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device.len() == 4 && (device.starts_with("COM") || device.starts_with("LPT")) && device.ends_with(|c: char| c.is_ascii_digit()));
    if reserved {
        out.insert(0, '_');
    }

    out.truncate(limit);
    out
}

/// Leave room for the set's name, a set index if shortened, and the eventual tile suffix.
fn psd_stem(mesh_name: &str, set_name: &str, set_index: usize, max_len: usize) -> String {
    let full = sanitize_stem_with_limit(&format!("{mesh_name}_{set_name}"), max_len + 1);
    if full.len() <= max_len {
        return full;
    }

    let set_index = set_index.to_string();
    let set_budget = max_len / 2 - set_index.len() - 2;
    let set = sanitize_stem_with_limit(set_name, set_budget);
    let mesh = sanitize_stem_with_limit(mesh_name, max_len - set.len() - set_index.len() - 2);
    format!("{mesh}_{set}_{set_index}")
}

/// Compare against the whole project so a separate export and a batch agree on names.
fn untiled_psd_stem(project: &Project, mesh_name: &str, set_index: usize, set: &TextureSet, atlas: bool) -> String {
    let stem = psd_stem(mesh_name, &set.name, set_index, MAX_STEM);
    let taken = |candidate: &str| {
        let tile = psd_tile_suffix(candidate);
        project.texture_sets.iter().enumerate().filter(|(index, _)| *index != set_index).any(|(index, other)| {
            if atlas || other.uv_tiles.is_empty() {
                candidate.eq_ignore_ascii_case(&psd_stem(mesh_name, &other.name, index, MAX_STEM))
            } else {
                tile.is_some_and(|(base, number)| {
                    other.uv_tiles.contains(&number)
                        && base.eq_ignore_ascii_case(&psd_stem(mesh_name, &other.name, index, MAX_STEM - (candidate.len() - base.len())))
                })
            }
        })
    };
    if taken(&stem) { disambiguated_psd_stem(&stem, set_index, MAX_STEM, taken) } else { stem }
}

/// Only a canonical tile number can collide with a generated UDIM suffix.
fn psd_tile_suffix(stem: &str) -> Option<(&str, u32)> {
    let (base, suffix) = stem.rsplit_once('.')?;
    let number: u32 = suffix.parse().ok()?;
    (number.to_string() == suffix).then_some((base, number))
}

/// Keep one UDIM base for all tiles of a set when any tile's ordinary name collides.
fn tiled_psd_stem(project: &Project, mesh_name: &str, set_index: usize, set: &TextureSet, limit: usize) -> String {
    let stem = psd_stem(mesh_name, &set.name, set_index, limit);
    let other_sets = || project.texture_sets.iter().enumerate().filter(|(index, _)| *index != set_index);
    let collides = other_sets().any(|(index, other)| {
        !other.uv_tiles.is_empty()
            && stem.eq_ignore_ascii_case(&psd_stem(mesh_name, &other.name, index, limit))
            && other.uv_tiles.iter().any(|tile| set.uv_tiles.contains(tile))
    });
    if !collides {
        return stem;
    }

    // Reserve ordinary names so a generated suffix cannot take another document's name
    disambiguated_psd_stem(&stem, set_index, limit, |candidate| {
        other_sets().any(|(index, other)| {
            if other.uv_tiles.is_empty() {
                let untiled = psd_stem(mesh_name, &other.name, index, MAX_STEM);
                psd_tile_suffix(&untiled).is_some_and(|(base, tile)| candidate.eq_ignore_ascii_case(base) && set.uv_tiles.contains(&tile))
            } else {
                candidate.eq_ignore_ascii_case(&psd_stem(mesh_name, &other.name, index, limit))
            }
        })
    })
}

fn disambiguated_psd_stem(stem: &str, set_index: usize, limit: usize, taken: impl Fn(&str) -> bool) -> String {
    let mut attempt = 0usize;
    loop {
        let suffix = if attempt == 0 { format!("_{set_index}") } else { format!("-{attempt}_{set_index}") };
        let name = format!("{}{suffix}", &stem[..stem.len().min(limit - suffix.len())]);
        if !taken(&name) {
            return name;
        }

        attempt += 1;
    }
}

/// [`sanitize_file_stem`] for each `/`-separated folder name and the file stem.
pub fn sanitize_file_path(path: &str) -> String {
    if !path.contains('/') {
        return sanitize_file_stem(path);
    }

    path.split('/').map(sanitize_file_stem).collect::<Vec<_>>().join("/")
}

/// Expand tokens in the template while keeping the substituted values literal.
fn expand_file_template(template: &str, variables: &[(&str, &str)]) -> String {
    let mut expanded = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(index) = remaining.find('$') {
        let (literal, variable) = remaining.split_at(index);
        expanded.push_str(literal);
        if let Some((value, tail)) = variables.iter().find_map(|&(token, value)| variable.strip_prefix(token).map(|tail| (value, tail))) {
            expanded.push_str(value);
            remaining = tail;
        } else {
            expanded.push('$');
            remaining = &variable[1..];
        }
    }

    expanded.push_str(remaining);
    expanded
}

/// The files `opts` produces for texture set `ts` without rendering ([`FILE_TOKENS`] are the template variables); PSD maps are layers of one file
pub fn list_outputs(project: &Project, ts: usize, mesh_name: &str, opts: &ExportOptions) -> Result<Vec<PlannedFile>> {
    plan_outputs(project, ts, mesh_name, opts, PsdBudget::TARGET)
}

/// [`list_outputs`] with the PSD memory limits of `budget`
fn plan_outputs(project: &Project, ts: usize, mesh_name: &str, opts: &ExportOptions, budget: PsdBudget) -> Result<Vec<PlannedFile>> {
    opts.preset.validate()?;
    let set = project.texture_set(ts)?;
    for requested_size in opts.size.iter().chain(opts.maps.values().filter_map(|own| own.size.as_ref())) {
        if !requested_size.is_power_of_two() || *requested_size > MAX_IMAGE_SIZE {
            return Err(FormatError::Invalid(format!("export size {requested_size} must be a power of two up to {MAX_IMAGE_SIZE}")));
        }
    }

    let mut used = BTreeSet::new();
    let mut active_files = BTreeSet::new();
    let mut active_dirs = BTreeSet::new();
    let mut out = Vec::new();

    // Bucket layers in tile order to move their names into the corresponding PSD
    let mut psd_layers = Vec::new();

    // A PSD map scaled past the largest side fails only the set's PSDs, as their other planning failures do
    let mut psd_too_large = None;
    let project_stem = project.name.replace('/', "_");
    let mesh_stem = mesh_name.replace('/', "_");
    let set_stem = set.name.replace('/', "_");
    let tiles: Vec<Option<u32>> = if set.uv_tiles.is_empty() || opts.atlas { vec![None] } else { set.uv_tiles.iter().copied().map(Some).collect() };
    let (block_width, block_height) = set.block_size();
    for (map_index, map) in opts.preset.maps.iter().enumerate() {
        if !map.is_available_for(set) {
            continue;
        }

        let own = opts.maps.get(&map_index);
        let (format, bit_depth, dither) = opts.map_file_type(map_index, map);
        let enabled = own.is_none_or(|o| o.enabled);
        if enabled && let Err(error) = check_scaled_block(set, (block_width, block_height), opts.map_size(map_index)) {
            if format != FileFormat::Psd {
                return Err(error);
            }

            if psd_too_large.is_none() {
                psd_too_large = Some(match error {
                    FormatError::Invalid(reason) => reason,
                    other => other.to_string(),
                });
            }
        }

        let space = opts.space_for_map(map, format, bit_depth);
        let requested_space = opts.space_for_depth(format, bit_depth);
        let color_map = map.is_color();
        let color_space = match space {
            ColorSpace::LinearRec709 => "linear",
            ColorSpace::Srgb => "sRGB",
            other => other.name(),
        };

        // Integer files clamp wide-gamut linear color to 0–1
        let clamps = color_map && !format.holds_float(bit_depth) && matches!(space, ColorSpace::AcesCg | ColorSpace::Aces2065);

        // A PSD layer needs only explicit tile tokens because its document name already holds the tile number
        let template = if format != FileFormat::Psd && tiles[0].is_some() && !map.file.contains("$udim") {
            std::borrow::Cow::Owned(format!("{}.$udim", map.file))
        } else {
            std::borrow::Cow::Borrowed(map.file.as_str())
        };
        let channel = channel_label(map);
        let expand_stem = |udim: &str| {
            let expanded = expand_file_template(
                &template,
                &[
                    ("$udim", udim),
                    ("$project", &project_stem),
                    ("$mesh", &mesh_stem),
                    ("$textureSet", &set_stem),
                    ("$channel", channel),
                    ("$colorSpace", color_space),
                ],
            );
            sanitize_file_path(&expanded)
        };

        // A tiled raster's name with a placeholder for its tile, the same for every tile
        let udim_template = (format != FileFormat::Psd && tiles[0].is_some()).then(|| expand_stem("UDIMTOKEN"));
        for (tile_index, &tile) in tiles.iter().enumerate() {
            let tile_number = tile.unwrap_or(1001).to_string();
            let stem = expand_stem(&tile_number);
            let ext = format.extension();
            let mut warnings = Vec::new();
            if color_map
                && format == FileFormat::Pgm
                && let Some(requested) = opts.color_space
            {
                let message = if requested == space {
                    format!("PGM export writes luminance in {} without embedding a colour space", space.name())
                } else {
                    format!("PGM cannot write {} primaries; luminance is written in {}", requested.name(), space.name())
                };
                warnings.push(message);
            }
            if clamps {
                warnings.push(format!(
                    "{} colours outside 0-1 are clamped in {} files; use EXR or HDR to keep them",
                    space.name(),
                    format.extension().to_uppercase()
                ));
            }
            let packs_colour = |name: &str| {
                (color_map && requested_space != space)
                    .then(|| format!("\"{name}\" packs colour into one channel (L), so it is written in linear Rec.709, not {}", requested_space.name()))
            };

            if format == FileFormat::Psd {
                if enabled {
                    warnings.extend(packs_colour(&stem));
                    if psd_layers.is_empty() {
                        psd_layers.resize_with(tiles.len(), Vec::new);
                    }
                    psd_layers[tile_index].push((PlannedLayer { map: map_index, name: stem, dither }, bit_depth, warnings));
                }
                continue;
            }

            let (name, renamed) = unique_name(&mut used, &stem, ext, &mut warnings);
            if !enabled {
                continue;
            }

            claim_path(&mut active_files, &mut active_dirs, &name)?;
            warnings.extend(packs_colour(&name));

            // Use a UDIM name only when substituting its tile recovers the output name
            // Truncation or literal placeholder text can make that substitution ambiguous
            let udim_name = udim_template
                .as_deref()
                .filter(|t| t.contains("UDIMTOKEN") && t.replace("UDIMTOKEN", &tile_number) == stem)
                .map(|t| format!("{t}{renamed}.{ext}").replace("UDIMTOKEN", "<UDIM>"));
            out.push(PlannedFile { name, map: map_index, format, bit_depth, dither, tile, udim_name, warnings, layers: Vec::new() });
        }
    }

    // Reuse the base across tiles with the same suffix width
    let mut tiled_bases = HashMap::new();

    // Each tile's PSD maps are the layers of one file named after the texture set
    let mut psd_failures = Vec::new();
    for (&tile, in_tile) in tiles.iter().zip(psd_layers) {
        let Some((first_layer, ..)) = in_tile.first() else { continue };

        let first_map = first_layer.map;
        let bit_depth = in_tile.iter().map(|(_, depth, _)| *depth).max().unwrap_or(8);
        let mut warnings = Vec::new();
        let mut layers = Vec::with_capacity(in_tile.len());
        let mut dither_requested = false;
        for (mut layer, _, layer_warnings) in in_tile {
            dither_requested |= layer.dither;
            layer.dither &= bit_depth == 8;
            layers.push(layer);
            for warning in layer_warnings {
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }
        }
        if bit_depth == 16 && dither_requested {
            warnings.push("PSD layers use the 16-bit document depth; 8-bit dithering was skipped".into());
        }

        let ext = FileFormat::Psd.extension();
        let (stem, udim_stem) = if let Some(number) = tile {
            let suffix = format!(".{number}");
            let limit = MAX_STEM - suffix.len();
            let base = tiled_bases.entry(limit).or_insert_with(|| tiled_psd_stem(project, mesh_name, ts, set, limit));
            (format!("{base}{suffix}"), Some(format!("{base}.<UDIM>")))
        } else {
            (untiled_psd_stem(project, mesh_name, ts, set, opts.atlas), None)
        };
        let (name, renamed) = unique_name(&mut used, &stem, ext, &mut warnings);
        if let Some(reason) = &psd_too_large {
            psd_failures.push((name, FormatError::Invalid(reason.clone())));
            continue;
        }
        if let Err(error) = layered_psd::preflight(set, opts, tile, bit_depth, layers.iter(), budget.working_bytes) {
            psd_failures.push((name, error));
            continue;
        }

        claim_path(&mut active_files, &mut active_dirs, &name)?;
        let udim_name = udim_stem.map(|stem| format!("{stem}{renamed}.{ext}"));
        let dither = bit_depth == 8 && dither_requested;
        out.push(PlannedFile { name, map: first_map, format: FileFormat::Psd, bit_depth, dither, tile, udim_name, warnings, layers });
    }

    // A PSD that cannot be written leaves the set's other files to export, its reason a warning on the first of them
    for (name, error) in psd_failures {
        let Some(first) = out.first_mut() else { return Err(error) };
        let reason = match error {
            FormatError::Invalid(reason) | FormatError::TooLarge(reason) => reason,
            other => other.to_string(),
        };
        first.warnings.push(format!("\"{name}\" {PSD_NOT_WRITTEN}{reason}"));
    }
    Ok(out)
}

/// What a set's PSD planning failure says after the file name
const PSD_NOT_WRITTEN: &str = "was not written: ";

/// Whether `warning` is about a PSD of the set that was not planned rather than the file carrying it, so it outlives that file
pub fn is_set_warning(warning: &str) -> bool {
    warning.split_once(".psd\" ").is_some_and(|(name, rest)| name.starts_with('"') && rest.starts_with(PSD_NOT_WRITTEN))
}

/// `stem.ext`, or `stem_2.ext`… when an earlier file has the name ignoring case (as Windows and macOS file systems do): the name and the suffix added
fn unique_name(used: &mut BTreeSet<String>, stem: &str, ext: &str, warnings: &mut Vec<String>) -> (String, String) {
    let mut name = format!("{stem}.{ext}");
    let mut renamed = String::new();
    let mut k = 2;
    while used.contains(&name.to_ascii_lowercase()) {
        renamed = format!("_{k}");
        name = format!("{stem}{renamed}.{ext}");
        k += 1;
    }
    if !renamed.is_empty() {
        warnings.push(format!("\"{stem}.{ext}\" is produced by more than one map; this one was renamed to \"{name}\""));
    }

    used.insert(name.to_ascii_lowercase());
    (name, renamed)
}

/// Record exported file `name` and its folders, refusing a name that is already a folder or a folder that is already a file, ignoring case
fn claim_path(files: &mut BTreeSet<String>, dirs: &mut BTreeSet<String>, name: &str) -> Result<()> {
    let lower = name.to_ascii_lowercase();
    if dirs.contains(&lower) {
        return Err(FormatError::Invalid(format!("\"{name}\" cannot be both an exported file and a folder")));
    }

    let mut prefix = lower.as_str();
    while let Some((folder, _)) = prefix.rsplit_once('/') {
        if files.contains(folder) {
            return Err(FormatError::Invalid(format!("\"{name}\" cannot use \"{folder}\" as a folder because another output exports that file")));
        }

        dirs.insert(folder.to_owned());
        prefix = folder;
    }
    files.insert(lower);
    Ok(())
}

/// Reject file and folder conflicts across a selection's retained output paths, ignoring case.
pub fn validate_export_paths<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<()> {
    let mut files = BTreeSet::new();
    let mut folders = BTreeSet::new();
    for name in names {
        claim_path(&mut files, &mut folders, name)?;
    }

    Ok(())
}

/// The dimensions of a file or PSD layer, scaling relative to the width of one atlas block.
pub fn output_dimensions(set: &TextureSet, tile: Option<u32>, export_size: Option<u32>) -> (u32, u32) {
    let (block_width, block_height) = set.block_size();
    let (width, height) = match (tile, set.tile_grid()) {
        (Some(tile), Some(_)) if !set.tile_resolutions.is_empty() => {
            let side = tile_side(set, tile, block_width);
            (side, side)
        }
        (Some(_), Some(_)) | (_, None) => (block_width, block_height),
        (None, Some(grid)) => (block_width.saturating_mul(grid.cols), block_height.saturating_mul(grid.rows)),
    };

    (scale_export_side(width, block_width, export_size), scale_export_side(height, block_width, export_size))
}

/// Plan a selection in export order, keeping only the first case-insensitive use of a name.
pub fn list_set_outputs(project: &Project, sets: &[usize], mesh_name: &str, opts: &ExportOptions) -> Result<Vec<(usize, PlannedFile)>> {
    plan_set_outputs(project, sets, mesh_name, opts, PsdBudget::TARGET)
}

/// [`list_set_outputs`] with the PSD memory limits of `budget`
fn plan_set_outputs(project: &Project, sets: &[usize], mesh_name: &str, opts: &ExportOptions, budget: PsdBudget) -> Result<Vec<(usize, PlannedFile)>> {
    let mut files = Vec::new();
    for &index in sets {
        let planned = plan_outputs(project, index, mesh_name, opts, budget)?;
        files.extend(planned.into_iter().map(|file| (index, file)));
    }

    let files = keep_first_planned_names(files);
    if sets.len() > 1 {
        validate_export_paths(files.iter().map(|(_, file)| file.name.as_str()))?;
    }

    let mut psd_bytes = 0;
    for (index, file) in files.iter().filter(|(_, file)| file.format == FileFormat::Psd) {
        check_psd_batch_within(project.texture_set(*index)?, opts, std::slice::from_ref(file), &mut psd_bytes, budget)?;
    }

    Ok(files)
}

// ---------------------------------------------------------------------------------------------
// Sampling

/// Which texel a value is taken from.
#[derive(Clone, Copy, PartialEq)]
enum At {
    Texel(u32, u32),
    /// The channel default (padding beyond the dilation distance).
    Default,
}

struct Sampler<'a, Coverage = bool> {
    set: &'a TextureSet,
    comp: &'a Composite,
    baked: &'a BakedMaps,
    coverage: &'a [Coverage],
    uncovered: Coverage,
    /// Composite width.
    res: u32,
    /// Texels across and down an atlas block (the composite for a classic set), which the height slope follows
    block: (u32, u32),
    /// The share of its block each tile fills: a smaller tile's height slope shrinks with it.
    scales: BlockScales,
    /// Normal convention overriding every normal output.
    normal_override: Option<NormalFormat>,
    project_normal: NormalFormat,
    dielectric_f0_scale: f32,
}

fn unpack(v: [f32; 3]) -> Vec3 {
    (Vec3::from(v) * 2.0 - Vec3::ONE).try_normalize().unwrap_or(Vec3::Z)
}

/// Reoriented Normal Mapping: `detail` re-expressed around `base` (unit tangent-space vectors).
pub fn rnm(base: Vec3, detail: Vec3) -> Vec3 {
    let t = base + Vec3::Z;
    let u = detail * Vec3::new(-1.0, -1.0, 1.0);
    (t * t.dot(u) / t.z.max(1e-4) - u).try_normalize().unwrap_or(base)
}

impl<'a, Coverage> Sampler<'a, Coverage> {
    /// The sampler of `input`'s texture set `set`, with the given atlas block dimensions
    fn new(input: &ExportInput<'a, Coverage>, options: &ExportOptions, set: &'a TextureSet, block: (u32, u32), uncovered: Coverage) -> Self {
        Sampler {
            set,
            comp: input.composite,
            baked: input.baked,
            coverage: input.coverage,
            uncovered,
            res: input.composite.resolution,
            block,
            scales: set.block_scales(),
            normal_override: options.normal_format,
            project_normal: input.project.normal_format,
            dielectric_f0_scale: input.project.display.dielectric_f0_scale(),
        }
    }
}

impl<Coverage: PartialEq> Sampler<'_, Coverage> {
    fn channel(&self, c: Channel, at: At) -> [f32; 3] {
        match at {
            At::Texel(x, y) if self.set.has_channel(c) => self.comp.value(c, x, y),
            _ => c.default_value(),
        }
    }

    fn baked(&self, m: BakedMap, at: At) -> [f32; 3] {
        match at {
            At::Texel(x, y) if self.set.baked.contains(&m) => {
                self.baked.sample_working(m, x, y, (self.res, self.comp.height), &self.scales).unwrap_or(m.neutral())
            }
            _ => m.neutral(),
        }
    }

    fn covered(&self, x: u32, y: u32) -> bool {
        self.coverage.get((y as usize) * self.res as usize + x as usize).is_some_and(|sample| sample != &self.uncovered)
    }

    /// Normal from the height channel (the set's height to normal method; uncovered neighbours repeat the centre)
    fn height_normal(&self, x: u32, y: u32) -> Vec3 {
        let center = self.channel(Channel::Height, At::Texel(x, y))[0];

        // Use the current tile's edge values instead of sampling its neighbors
        let (block_width, block_height) = (self.block.0.max(1), self.block.1.max(1));
        let (x0, y0) = ((x / block_width * block_width) as i64, (y / block_height * block_height) as i64);
        let h = |dx: i32, dy: i32| {
            let sx = (x as i64 + dx as i64).clamp(x0, x0 + block_width as i64 - 1) as u32;
            let sy = (y as i64 + dy as i64).clamp(y0, y0 + block_height as i64 - 1) as u32;
            if self.covered(sx, sy) { self.channel(Channel::Height, At::Texel(sx, sy))[0] } else { center }
        };
        let [gx, gy] = self.set.height_to_normal.slope(h);

        // Slopes per texel become slopes across the tile along each side
        let strength = self.set.height_strength * self.scales.at(x, y) / HEIGHT_SPAN;
        let (scale_x, scale_y) = (strength * block_width as f32, strength * block_height as f32);

        // OpenGL normal Y points up as image rows advance down
        Vec3::new(-gx * scale_x, gy * scale_y, 1.0).try_normalize().unwrap_or(Vec3::Z)
    }

    /// The final tangent-space normal (OpenGL): baked normal ⊕ (height ⊕ normal channel).
    fn final_normal(&self, at: At) -> Vec3 {
        let At::Texel(x, y) = at else { return Vec3::Z };

        let base = if self.set.baked.contains(&BakedMap::Normal) { unpack(self.baked(BakedMap::Normal, at)) } else { Vec3::Z };
        let mut detail = if self.set.has_channel(Channel::Normal) { unpack(self.channel(Channel::Normal, at)) } else { Vec3::Z };
        if self.set.height_in_normal() && self.set.has_channel(Channel::Height) && self.set.height_strength != 0.0 {
            detail = rnm(self.height_normal(x, y), detail);
        }
        rnm(base, detail)
    }

    fn normal(&self, at: At, format: NormalFormat, cache: &mut Option<Vec3>) -> [f32; 3] {
        let n = *cache.get_or_insert_with(|| self.final_normal(at));
        self.pack_normal(n, format)
    }

    /// A unit OpenGL normal packed into 0..1 in `format` (or the override).
    fn pack_normal(&self, n: Vec3, format: NormalFormat) -> [f32; 3] {
        let y = if self.normal_override.unwrap_or(format) == NormalFormat::DirectX { -n.y } else { n.y };
        [n.x * 0.5 + 0.5, y * 0.5 + 0.5, n.z * 0.5 + 0.5]
    }

    fn sample(&self, src: &SourceRef, at: At, cache: &mut Option<Vec3>) -> f32 {
        let k = src.component();
        let v = match src {
            SourceRef::Constant(v) => return *v,
            SourceRef::Channel { channel: Channel::Normal, .. } => self.normal(at, self.project_normal, cache),
            SourceRef::Channel { channel, .. } => {
                let v = self.channel(*channel, at);
                if channel.kind() == ChannelKind::Signed { v.map(|h| h * 0.5 + 0.5) } else { v }
            }
            SourceRef::Virtual { map, .. } => match map {
                VirtualMap::NormalOpenGl => self.normal(at, NormalFormat::OpenGl, cache),
                VirtualMap::NormalDirectX => self.normal(at, NormalFormat::DirectX, cache),
                VirtualMap::Glossiness => [1.0 - self.channel(Channel::Roughness, at)[0]; 3],
                VirtualMap::F0 => [self.dielectric_f0_scale * self.channel(Channel::SpecularLevel, at)[0]; 3],
                VirtualMap::Roughness => [1.0 - self.channel(Channel::Glossiness, at)[0]; 3],
                VirtualMap::SheenTint => self.channel(Channel::SheenColor, at).map(|c| c * self.channel(Channel::SheenWeight, at)[0]),
                VirtualMap::HeightNormal => {
                    let n = match at {
                        At::Texel(x, y) => self.height_normal(x, y),
                        At::Default => Vec3::Z,
                    };
                    self.pack_normal(n, self.project_normal)
                }
                VirtualMap::AoMixed => {
                    let ch = self.channel(Channel::AmbientOcclusion, at)[0];
                    let baked = self.baked(BakedMap::AmbientOcclusion, at)[0];
                    [if self.set.ao_mixing == AoMixing::Multiply { ch * baked } else { ch }; 3]
                }
            },
            SourceRef::Baked { map, .. } => {
                let v = self.baked(*map, at);

                // Tangent-space maps are stored with OpenGL green, like the normal map: write the format of the export
                if matches!(map, BakedMap::Normal | BakedMap::BentNormal) && self.normal_override.unwrap_or(self.project_normal) == NormalFormat::DirectX {
                    [v[0], 1.0 - v[1], v[2]]
                } else {
                    v
                }
            }
        };
        v.get(k).copied().unwrap_or(0.0)
    }
}

// ---------------------------------------------------------------------------------------------
// Rendering

/// Render every file of `opts` for `input`'s texture set.
pub fn render_maps(input: &ExportInput, options: &ExportOptions) -> Result<Vec<ExportedFile>> {
    let planned = list_outputs(input.project, input.ts, input.mesh_name, options)?;
    render_planned_maps(input, options, planned)
}

/// Add one set's PSD output estimate to a batch that holds encoded files in memory (limited only in a browser tab)
pub fn check_psd_batch(set: &TextureSet, options: &ExportOptions, planned: &[PlannedFile], total_bytes: &mut u64) -> Result<()> {
    check_psd_batch_within(set, options, planned, total_bytes, PsdBudget::TARGET)
}

fn check_psd_batch_within(set: &TextureSet, options: &ExportOptions, planned: &[PlannedFile], total_bytes: &mut u64, budget: PsdBudget) -> Result<()> {
    let mut estimate = *total_bytes;
    for file in planned.iter().filter(|file| file.format == FileFormat::Psd) {
        let (_, file_bytes) = layered_psd::preflight(set, options, file.tile, file.bit_depth, file.layers.iter(), budget.working_bytes)?;
        add_psd_batch_bytes(&mut estimate, file_bytes, budget.batch_bytes)?;
    }

    *total_bytes = estimate;
    Ok(())
}

fn add_psd_batch_bytes(total_bytes: &mut u64, file_bytes: u64, limit: Option<u64>) -> Result<()> {
    let estimate = total_bytes.saturating_add(file_bytes);
    if limit.is_some_and(|limit| estimate > limit) {
        return Err(FormatError::TooLarge("total PSD output would exceed 1 GiB; use a smaller export size, fewer PSD maps or separate exports".into()));
    }

    *total_bytes = estimate;
    Ok(())
}

/// Render the retained plans from `list_outputs` for this input and these options.
pub fn render_planned_maps(input: &ExportInput, options: &ExportOptions, planned: Vec<PlannedFile>) -> Result<Vec<ExportedFile>> {
    render_planned_maps_within(input, options, planned, PsdBudget::TARGET)
}

/// [`render_planned_maps`] with the PSD memory limits of `budget`
fn render_planned_maps_within(input: &ExportInput, options: &ExportOptions, planned: Vec<PlannedFile>, budget: PsdBudget) -> Result<Vec<ExportedFile>> {
    if planned.is_empty() {
        return Ok(Vec::new());
    }

    let mut psd_sizes = Vec::new();
    if planned.iter().any(|file| file.format == FileFormat::Psd) {
        let set = input.project.texture_set(input.ts)?;
        let expected = set.size();
        let actual = (input.composite.resolution, input.composite.height);
        if actual != expected {
            return Err(FormatError::Invalid(format!(
                "PSD composite size {}×{} does not match the planned texture set size {}×{}",
                actual.0, actual.1, expected.0, expected.1
            )));
        }

        let mut psd_bytes = 0;
        for file in &planned {
            if file.format == FileFormat::Psd {
                let (size, file_bytes) = layered_psd::preflight(set, options, file.tile, file.bit_depth, file.layers.iter(), budget.working_bytes)?;
                add_psd_batch_bytes(&mut psd_bytes, file_bytes, budget.batch_bytes)?;
                psd_sizes.push(Some(size));
            } else {
                psd_sizes.push(None);
            }
        }
    }

    let renderer = MapRenderer::new(input, options, &planned)?;
    planned
        .into_iter()
        .zip(psd_sizes.into_iter().chain(std::iter::repeat(None)))
        .map(|(planned_file, psd_size)| {
            if let Some(size) = psd_size {
                return layered_psd::render_psd(&renderer, planned_file, size);
            }

            let map = renderer.render(planned_file.map, planned_file.tile, planned_file.format, planned_file.bit_depth)?;
            let mut warnings = planned_file.warnings;
            let encoding = Encoding { format: planned_file.format, depth: planned_file.bit_depth, dither: planned_file.dither };
            let bytes = encode(encoding, map.size, map.component_count, map.data, &map.srgb, map.embed, &mut warnings)?;
            Ok(ExportedFile { name: planned_file.name, bytes, width: map.size.0, height: map.size.1, warnings })
        })
        .collect()
}

fn tile_side(set: &TextureSet, tile: u32, block_resolution: u32) -> u32 {
    ((block_resolution as f32 * set.tile_scale(tile)).round() as u32).clamp(1, block_resolution.max(1))
}

/// Refuse an export size that scales a non-square set's UV tile past the largest image side
fn check_scaled_block(set: &TextureSet, (block_width, block_height): (u32, u32), export_size: Option<u32>) -> Result<()> {
    let width = export_size.unwrap_or(block_width);
    let height = scale_export_side(block_height, block_width, export_size);

    if width.max(height) > MAX_IMAGE_SIZE {
        return Err(FormatError::Invalid(format!("export size makes texture set \"{}\" {width}×{height}, over {MAX_IMAGE_SIZE} texels a side", set.name)));
    }

    Ok(())
}

/// Scale an image side relative to the source block, keeping a pixel for any nonempty source.
pub fn scale_export_side(source_side: u32, block_resolution: u32, export_size: Option<u32>) -> u32 {
    let export_size = export_size.unwrap_or(block_resolution);
    let scaled = u64::from(source_side) * u64::from(export_size) / u64::from(block_resolution.max(1));
    u32::try_from(scaled).unwrap_or(u32::MAX).max(u32::from(source_side > 0))
}

fn diffuse_blocks(
    data: &mut Vec<f32>,
    (width, height): (usize, usize),
    channels: usize,
    dilation: &texturecraft_texel::DilationMap,
    (origin_x, origin_y): (usize, usize),
    (step_x, step_y): (usize, usize),
) {
    use texturecraft_texel::dilate::NONE;

    for block_y in (0..height).step_by(step_y) {
        for block_x in (0..width).step_by(step_x) {
            let (block_width, block_height) = (step_x.min(width - block_x), step_y.min(height - block_y));
            let source_rows = (origin_y + block_y..origin_y + block_y + block_height).map(|y| {
                let start = y * dilation.width as usize + origin_x + block_x;
                &dilation.src[start..start + block_width]
            });
            let mut coverage = source_rows.flatten().map(|source| *source != NONE);
            let Some(first) = coverage.next() else { continue };
            if coverage.clone().all(|covered| covered == first) {
                continue;
            }

            // The dilation map already identifies every known texel, including original UV coverage
            let weights = std::iter::once(first).chain(coverage).map(f32::from).collect();
            if block_width == width && block_height == height {
                diffuse_fill(data, (width, height), channels, weights);
                return;
            }

            let rows = || (block_y..block_y + block_height).map(|y| (y * width + block_x)..(y * width + block_x + block_width));
            let mut block_data: Vec<f32> =
                rows().flat_map(|row| data.get(row.start * channels..row.end * channels).unwrap_or_default().iter().copied()).collect();
            diffuse_fill(&mut block_data, (block_width, block_height), channels, weights);
            for (row, chunk) in rows().zip(block_data.chunks(block_width * channels)) {
                if let Some(destination) = data.get_mut(row.start * channels..row.end * channels) {
                    destination.copy_from_slice(chunk);
                }
            }
        }
    }
}

/// Fill the zero-weight texels of a `size` image of `n` components by pull-push
/// diffusion: known texels are averaged down a pyramid, then every level fills its gaps from the
/// smoothly upsampled level below, so a gap takes a soft blend of the nearby known colours.
fn diffuse_fill(data: &mut Vec<f32>, (w, h): (usize, usize), channels: usize, weights: Vec<f32>) {
    // Keep coverage weights alongside the colour samples at each level
    let mut levels = vec![(w, h, std::mem::take(data), weights)];
    let mut accumulator = vec![0.0f32; channels];
    while let Some((width, height, colour, weight)) = levels.last()
        && (*width > 1 || *height > 1)
    {
        let (next_width, next_height) = (width.div_ceil(2), height.div_ceil(2));
        let mut coarse_colour = vec![0.0f32; next_width * next_height * channels];
        let mut coarse_coverage = vec![0.0f32; next_width * next_height];
        for y in 0..next_height {
            for x in 0..next_width {
                let mut total_weight = 0.0;
                accumulator.fill(0.0);
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (2 * x + dx, 2 * y + dy);
                    if sx >= *width || sy >= *height {
                        continue;
                    }

                    let i = sy * width + sx;
                    let sample_weight = weight.get(i).copied().unwrap_or(0.0);
                    total_weight += sample_weight;
                    for (channel, v) in accumulator.iter_mut().enumerate() {
                        *v += sample_weight * colour.get(i * channels + channel).copied().unwrap_or(0.0);
                    }
                }

                let output_index = y * next_width + x;
                if total_weight > 0.0 {
                    for (channel, v) in accumulator.iter().enumerate() {
                        if let Some(t) = coarse_colour.get_mut(output_index * channels + channel) {
                            *t = v / total_weight;
                        }
                    }
                }
                if let Some(t) = coarse_coverage.get_mut(output_index) {
                    *t = total_weight.min(1.0);
                }
            }
        }
        levels.push((next_width, next_height, coarse_colour, coarse_coverage));
    }

    // Release each coarse level after using it to fill the finer level
    while levels.len() > 1
        && let Some((coarse_width, coarse_height, coarse_colour, _)) = levels.pop()
        && let Some(fine) = levels.last_mut()
    {
        let (fine_width, fine_height, fine_colour, fine_coverage) = (fine.0, fine.1, &mut fine.2, &mut fine.3);
        let sample = |x: f32, y: f32, channel: usize| {
            let (x, y) = (x.clamp(0.0, (coarse_width - 1) as f32), y.clamp(0.0, (coarse_height - 1) as f32));
            let (x0, y0) = (x as usize, y as usize);
            let (x1, y1) = ((x0 + 1).min(coarse_width - 1), (y0 + 1).min(coarse_height - 1));
            let (ax, ay) = (x - x0 as f32, y - y0 as f32);
            let at = |px: usize, py: usize| coarse_colour.get((py * coarse_width + px) * channels + channel).copied().unwrap_or(0.0);
            (at(x0, y0) * (1.0 - ax) + at(x1, y0) * ax) * (1.0 - ay) + (at(x0, y1) * (1.0 - ax) + at(x1, y1) * ax) * ay
        };
        for y in 0..fine_height {
            for x in 0..fine_width {
                let i = y * fine_width + x;
                let coverage = fine_coverage.get(i).copied().unwrap_or(0.0);
                if coverage >= 1.0 {
                    continue;
                }

                let (sx, sy) = ((x as f32 + 0.5) / 2.0 - 0.5, (y as f32 + 0.5) / 2.0 - 0.5);
                for channel in 0..channels {
                    if let Some(v) = fine_colour.get_mut(i * channels + channel) {
                        *v = coverage * *v + (1.0 - coverage) * sample(sx, sy, channel);
                    }
                }
                if let Some(t) = fine_coverage.get_mut(i) {
                    *t = 1.0;
                }
            }
        }
    }

    if let Some((_, _, filled, _)) = levels.pop() {
        *data = filled;
    }
}

/// Resample a `from` (width, height) image of `n` components to `to` (same aspect): box filter
/// when shrinking by an integer factor, bilinear otherwise.
fn resize(data: Vec<f32>, from: (u32, u32), n: usize, to: (u32, u32)) -> Vec<f32> {
    if to == from || from.0 == 0 || from.1 == 0 || to.0 == 0 || to.1 == 0 {
        return data;
    }
    let ((r, rh), (t, th)) = ((from.0 as usize, from.1 as usize), (to.0 as usize, to.1 as usize));
    let mut out = vec![0.0f32; t * th * n];
    let px = |x: usize, y: usize, k: usize| data.get((y * r + x) * n + k).copied().unwrap_or(0.0);
    let boxed = r > t && r % t == 0 && rh % th == 0 && r / t == rh / th;
    out.par_chunks_mut(t * n).enumerate().for_each(|(y, row)| {
        for x in 0..t {
            for k in 0..n {
                let v = if boxed {
                    let f = r / t;
                    let sum: f32 = (0..f).flat_map(|dy| (0..f).map(move |dx| (dx, dy))).map(|(dx, dy)| px(x * f + dx, y * f + dy, k)).sum();
                    sum / (f * f) as f32
                } else {
                    let (sx, sy) = (r as f32 / t as f32, rh as f32 / th as f32);
                    let (fx, fy) = (((x as f32 + 0.5) * sx - 0.5).max(0.0), ((y as f32 + 0.5) * sy - 0.5).max(0.0));
                    let (x0, y0) = ((fx as usize).min(r - 1), (fy as usize).min(rh - 1));
                    let (x1, y1) = ((x0 + 1).min(r - 1), (y0 + 1).min(rh - 1));
                    let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
                    let top = px(x0, y0, k) * (1.0 - ax) + px(x1, y0, k) * ax;
                    let bottom = px(x0, y1, k) * (1.0 - ax) + px(x1, y1, k) * ax;
                    top * (1.0 - ay) + bottom * ay
                };
                if let Some(o) = row.get_mut(x * n + k) {
                    *o = v;
                }
            }
        }
    });
    out
}

/// 8×8 ordered (Bayer) dither thresholds, 0..64.
const BAYER8: [u8; 64] = [
    0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60, 28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41, 51,
    19, 59, 27, 49, 17, 57, 25, 15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21,
];

fn rec709_luminance(red: f32, green: f32, blue: f32) -> f32 {
    // Rec.709 luminance weights apply before the display transfer
    const WEIGHTS: [f32; 3] = [0.2126, 0.7152, 0.0722];
    let [red_weight, green_weight, blue_weight] = WEIGHTS;
    red_weight * red + green_weight * green + blue_weight * blue
}

/// Encode a linear image as L, LA, RGB or RGBA at the requested width and height.
fn encode(
    encoding: Encoding,
    size: (u32, u32),
    component_count: usize,
    data: Vec<f32>,
    srgb: &[bool],
    embed: Option<ColorSpace>,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>> {
    let Encoding { format, depth, .. } = encoding;
    let encoding_error = |error: &dyn std::fmt::Display| FormatError::Invalid(format!("{} encoding failed: {error}", format.extension()));

    // Embed ICC or EXR chromaticities only where the format supports them
    if let Some(space) = embed
        && matches!(format, FileFormat::Tga | FileFormat::Bmp | FileFormat::Hdr | FileFormat::Jng)
    {
        warnings.push(format!("{} export does not embed a colour space; the colours are in {}", format.extension().to_ascii_uppercase(), space.name()));
    }
    match format {
        FileFormat::Exr => return write_exr(size, component_count, &data, depth == 16, embed).map_err(|error| encoding_error(&error)),
        FileFormat::Hdr => {
            if matches!(component_count, 2 | 4) {
                warnings.push("HDR has no alpha channel; alpha was dropped".into());
            }

            let samples = if component_count == 3 {
                data
            } else {
                let expanded = remap(&data, component_count, if component_count <= 2 { &[0, 0, 0] } else { &[0, 1, 2] });
                drop(data);
                expanded
            };

            return write_float_raster(CodecFormat::Hdr, size, ChannelLayout::Rgb, samples, None).map_err(|error| encoding_error(&error));
        }

        // Widen float gray for readers that only support float RGB and RGBA
        FileFormat::Tiff if depth == 32 => {
            let samples = if component_count <= 2 {
                let components: &[usize] = if component_count == 1 { &[0, 0, 0] } else { &[0, 0, 0, 1] };
                let expanded = remap(&data, component_count, components);
                drop(data);
                expanded
            } else {
                data
            };
            let layout = if matches!(component_count, 2 | 4) { ChannelLayout::Rgba } else { ChannelLayout::Rgb };
            let profile = embed.and_then(crate::icc::profile);

            return write_float_raster(CodecFormat::Tiff, size, layout, samples, profile).map_err(|error| encoding_error(&error));
        }
        _ => {}
    }

    let gray_srgb: [bool; 1];
    let (integer_data, srgb, component_count): (Vec<f32>, &[bool], usize) = if format == FileFormat::Pgm && component_count > 1 {
        if component_count != 3 {
            warnings.push("PGM has no alpha channel; alpha was dropped".into());
        }
        if component_count >= 3 {
            warnings.push("PGM holds one gray channel; colour became its luminance".into());
        }

        let gray = data
            .chunks_exact(component_count)
            .map(|pixel| match pixel {
                [l, _] => *l,
                [red, green, blue, ..] => rec709_luminance(*red, *green, *blue),
                _ => 0.0,
            })
            .collect();
        drop(data);
        gray_srgb = [srgb.first().copied().unwrap_or(false)];
        (gray, &gray_srgb, 1)
    } else {
        (data, srgb, component_count)
    };

    let channel_indices: Option<&[usize]> = match (format, component_count) {
        (FileFormat::Jpeg, 2 | 4) => {
            warnings.push("JPEG has no alpha channel; alpha was dropped".into());
            Some(if component_count == 2 { &[0] } else { &[0, 1, 2] })
        }
        (FileFormat::Tiff, 2) => Some(&[0, 0, 0, 1]),
        _ => None,
    };

    let transferred = integer_data
        .into_iter()
        .enumerate()
        .map(|(index, value)| if srgb.get(index % component_count).copied().unwrap_or(false) { linear_to_srgb(value) } else { value.clamp(0.0, 1.0) });
    let (bytes, component_count) = if let Some(indices) = channel_indices {
        // Transfer each source sample once, then quantize directly in destination order
        let transferred: Vec<f32> = transferred.collect();
        let output_channels = indices.len();
        let sample_count =
            (transferred.len() / component_count).checked_mul(output_channels).ok_or_else(|| encoding_error(&"remapped sample count overflows"))?;
        let remapped = (0..sample_count).map(|sample| {
            let pixel_start = sample / output_channels * component_count;
            transferred[pixel_start + indices[sample % output_channels]]
        });
        (quantize_integer(encoding, size.0, output_channels, remapped)?, output_channels)
    } else {
        (quantize_integer(encoding, size.0, component_count, transferred)?, component_count)
    };
    let sample_type = if depth == 16 { SampleType::U16 } else { SampleType::U8 };

    let codec_format = match format {
        FileFormat::Png => CodecFormat::Png,
        FileFormat::Jpeg => CodecFormat::Jpeg,
        FileFormat::Tga => CodecFormat::Tga,
        FileFormat::Tiff => CodecFormat::Tiff,
        FileFormat::Bmp => CodecFormat::Bmp,
        FileFormat::Webp => CodecFormat::WebP,
        FileFormat::Pgm => CodecFormat::Pnm,
        FileFormat::Jng => return jng(&bytes, size, component_count).map_err(|error| encoding_error(&error)),
        FileFormat::Exr | FileFormat::Hdr => return Err(encoding_error(&"unexpected float raster format")),
        FileFormat::Psd => return Err(encoding_error(&"a PSD holds the layers of several maps (`layered_psd`)")),
    };
    let layout = match component_count {
        1 => ChannelLayout::Gray,
        2 => ChannelLayout::GrayA,
        3 => ChannelLayout::Rgb,
        4 => ChannelLayout::Rgba,
        _ => return Err(encoding_error(&"expected one to four raster channels")),
    };

    // PGM colour space is reported in the export plan since PGM has no ICC field
    let profile = if format == FileFormat::Pgm { None } else { embed.and_then(crate::icc::profile) };
    let image = CodecImage::from_raw(size.0, size.1, layout, sample_type, bytes).map_err(|error| encoding_error(&error))?.with_icc(profile);
    write_raster(&image, codec_format).map_err(|error| encoding_error(&error))
}

fn quantize_u8(value: f32) -> u8 {
    (value * f32::from(u8::MAX) + 0.5) as u8
}

fn quantize_u16(value: f32) -> u16 {
    (value * f32::from(u16::MAX) + 0.5) as u16
}

fn quantize_integer(encoding: Encoding, width: u32, component_count: usize, samples: impl ExactSizeIterator<Item = f32>) -> Result<Vec<u8>> {
    let Encoding { format, depth, dither } = encoding;
    let byte_count = samples
        .len()
        .checked_mul(if depth == 16 { 2 } else { 1 })
        .ok_or_else(|| FormatError::Invalid(format!("{} sample buffer size overflows", format.extension())))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(byte_count).map_err(|error| FormatError::Invalid(format!("{} sample buffer allocation failed: {error}", format.extension())))?;

    if depth == 16 {
        bytes.extend(samples.flat_map(|value| quantize_u16(value).to_ne_bytes()));
    } else {
        let width = width.max(1) as usize;
        let threshold = |sample_index: usize| {
            const BAYER_SIDE: usize = 8;
            let pixel = sample_index / component_count.max(1);
            let index = (pixel / width % BAYER_SIDE) * BAYER_SIDE + pixel % width % BAYER_SIDE;
            let midpoint = (BAYER8.len() / 2) as u8;
            (f32::from(BAYER8.get(index).copied().unwrap_or(midpoint)) + 0.5) / BAYER8.len() as f32
        };
        let max_sample = f32::from(u8::MAX);
        if dither {
            bytes.extend(samples.enumerate().map(|(index, value)| (value * max_sample + threshold(index)).floor().clamp(0.0, max_sample) as u8));
        } else {
            bytes.extend(samples.map(quantize_u8));
        }
    }

    Ok(bytes)
}

/// Return the canvas size and conservative file size after checking PSD format limits.
fn check_psd_size(sizes: impl IntoIterator<Item = (u32, u32)>, folder_count: usize, depth: u8) -> Result<((u32, u32), u64)> {
    // A layer record with its names, and each of a group's two records, stays below this
    const PSD_RECORD_BYTES: u64 = 2048;

    let (mut width, mut height) = (1, 1);
    let mut layer_texels = 0;
    let mut layer_count = 0;
    for (layer_width, layer_height) in sizes {
        if !(1..=PSD_MAX_SIDE_PIXELS).contains(&layer_width) || !(1..=PSD_MAX_SIDE_PIXELS).contains(&layer_height) {
            return Err(FormatError::Invalid(format!("PSD size {layer_width}×{layer_height} must be between 1 and {PSD_MAX_SIDE_PIXELS} pixels per side")));
        }

        width = width.max(layer_width);
        height = height.max(layer_height);
        layer_texels += u64::from(layer_width) * u64::from(layer_height);
        layer_count += 1;
    }

    // Measure the merged image's overhead without allocating any pixel buffers
    let merged = flat_psd((width, height), depth).encoded_len().map_err(|error| FormatError::Invalid(format!("PSD sizing failed: {error}")))?;
    let records = PSD_RECORD_BYTES * (layer_count + 2 * folder_count as u64);
    let texels = u64::from(width) * u64::from(height) + layer_texels;
    let sample_bytes = texels * PSD_RGBA_PLANES as u64 * u64::from(depth / 8);
    let file_bytes = merged + records + sample_bytes;
    if file_bytes >= PSD_MAX_FILE_BYTES {
        return Err(FormatError::Invalid(format!(
            "a PSD of {layer_count} maps at {width}×{height} would reach the 2 GiB file limit; use a smaller export size, fewer PSD maps or another format"
        )));
    }

    Ok(((width, height), file_bytes))
}

/// A PSD of only an RGBA merged image with no pixels yet, to measure
fn flat_psd((width, height): (u32, u32), depth: u8) -> craft_psd::PsdFile {
    use craft_psd::{ColorMode, Compression, Header, ImageData, PsdFile, Version};

    let header = Header::new(Version::Psd, width, height, PSD_RGBA_PLANES as u16, u16::from(depth), ColorMode::Rgb);
    PsdFile {
        header,
        color_mode_data: Vec::new(),
        resources: Vec::new(),
        layer_info: None,
        layer_info_placement: Default::default(),
        global_layer_mask: None,
        global_blocks: Vec::new(),
        layer_mask_trailing: Vec::new(),
        image_data: ImageData::new(Compression::Raw, Vec::new()),
        write_as_psb: false,
    }
}

/// A JNG file: the colour as JPEG, alpha (when present) as lossless 8-bit deflate data.
fn jng(bytes: &[u8], (w, h): (u32, u32), n: usize) -> std::result::Result<Vec<u8>, String> {
    use std::io::Write;
    let alpha = n == 2 || n == 4;
    let colour = remap8(bytes, n, if n <= 2 { &[0] } else { &[0, 1, 2] });
    let mut jpeg = Cursor::new(Vec::new());
    let kind = if n <= 2 { image::ExtendedColorType::L8 } else { image::ExtendedColorType::Rgb8 };
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY).encode(&colour, w, h, kind).map_err(|e| e.to_string())?;

    let mut out = vec![0x8B, b'J', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let chunk = |out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(data);
        out.extend_from_slice(&crc.sum().to_be_bytes());
    };

    // JHDR: size, colour type (8 gray, 10 colour, +4 with alpha), 8-bit baseline JPEG, alpha as PNG data
    let mut jhdr = Vec::with_capacity(16);
    jhdr.extend_from_slice(&w.to_be_bytes());
    jhdr.extend_from_slice(&h.to_be_bytes());
    jhdr.extend_from_slice(&[if n <= 2 { 8 } else { 10 } + if alpha { 4 } else { 0 }, 8, 8, 0, if alpha { 8 } else { 0 }, 0, 0, 0]);
    chunk(&mut out, b"JHDR", &jhdr);
    if alpha {
        let a = remap8(bytes, n, &[n - 1]);
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        for row in a.chunks_exact(w.max(1) as usize) {
            z.write_all(&[0]).and_then(|()| z.write_all(row)).map_err(|e| e.to_string())?;
        }
        chunk(&mut out, b"IDAT", &z.finish().map_err(|e| e.to_string())?);
    }
    chunk(&mut out, b"JDAT", jpeg.get_ref());
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// Pick components `pick` from every texel of an `n`-component 8-bit image.
fn remap8(data: &[u8], n: usize, pick: &[usize]) -> Vec<u8> {
    data.chunks_exact(n.max(1)).flat_map(|px| pick.iter().map(|k| px.get(*k).copied().unwrap_or(0))).collect()
}

/// Pick components `pick` from every texel of an `n`-component image.
fn remap(data: &[f32], n: usize, pick: &[usize]) -> Vec<f32> {
    data.chunks_exact(n.max(1)).flat_map(|px| pick.iter().map(|k| px.get(*k).copied().unwrap_or(0.0))).collect()
}

fn write_float_raster(
    format: CodecFormat,
    (width, height): (u32, u32),
    layout: ChannelLayout,
    samples: Vec<f32>,
    profile: Option<Vec<u8>>,
) -> std::result::Result<Vec<u8>, craft_image_codecs::CodecError> {
    // The shared codec owns native-endian bytes and stages its own typed samples
    let source = bytemuck::cast_slice(&samples);
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(source.len()).map_err(|error| craft_image_codecs::CodecError::LimitExceeded(error.to_string()))?;
    bytes.extend_from_slice(source);
    drop(samples);

    let image = CodecImage::from_raw(width, height, layout, SampleType::F32, bytes)?.with_icc(profile);
    write_raster(&image, format)
}

fn write_raster(image: &CodecImage, format: CodecFormat) -> std::result::Result<Vec<u8>, craft_image_codecs::CodecError> {
    let options = EncodeOptions {
        jpeg_quality: JPEG_QUALITY,
        jpeg_chroma_subsampling: false, // Keep the 4:4:4 sampling used before the cutover.
        webp_lossless: true,
        embed_icc: image.icc.is_some(),
        embed_metadata: false,
        ..EncodeOptions::default()
    };
    craft_image_codecs::encode(image, format, &options)
}

/// OpenEXR with Y / YA / RGB / RGBA channels, half or full float, ZIP16 compression.
fn write_exr(
    (w, h): (u32, u32),
    n: usize,
    data: &[f32],
    half: bool,
    space: Option<ColorSpace>,
) -> std::result::Result<Vec<u8>, craft_image_codecs::CodecError> {
    let layout = match n {
        1 => craft_image_codecs::ChannelLayout::Gray,
        2 => craft_image_codecs::ChannelLayout::GrayA,
        3 => craft_image_codecs::ChannelLayout::Rgb,
        _ => craft_image_codecs::ChannelLayout::Rgba,
    };
    let img = if half {
        let samples: Vec<half::f16> = data.iter().map(|v| half::f16::from_f32(*v)).collect();
        craft_image_codecs::Image::from_f16(w, h, layout, &samples)?
    } else {
        craft_image_codecs::Image::from_f32(w, h, layout, data)?
    };
    let opts = craft_image_codecs::EncodeOptions {
        exr_compression: craft_image_codecs::ExrCompression::Zip16,
        exr_chromaticities: space.map(|s| s.chromaticities().map(|xy| [xy[0] as f32, xy[1] as f32])),
        ..Default::default()
    };
    craft_image_codecs::encode(&img, craft_image_codecs::Format::OpenExr, &opts)
}

/// Leave out each file named like an earlier one of `files` (ignoring case, as Windows and macOS
/// file systems do): written into the same folder it would replace that file. One texture set's
/// names never repeat, so a repeat comes from a later set. The kept file's warnings say so; the
/// left-out file's own warnings go with it.
pub fn skip_repeated_names(files: &mut Vec<ExportedFile>) {
    *files = keep_first_names(std::mem::take(files), |file| &file.name, |file| &mut file.warnings);
}

/// Keep the first case-insensitive use of each filename and attach duplicate warnings to its plan.
pub fn keep_first_planned_names<T>(files: impl IntoIterator<Item = (T, PlannedFile)>) -> Vec<(T, PlannedFile)> {
    keep_first_names(files, |(_, file)| &file.name, |(_, file)| &mut file.warnings)
}

fn keep_first_names<T>(files: impl IntoIterator<Item = T>, name: impl Fn(&T) -> &str, warnings: impl Fn(&mut T) -> &mut Vec<String>) -> Vec<T> {
    let files = files.into_iter();
    let mut first = HashMap::new();
    let mut kept = Vec::with_capacity(files.size_hint().0);
    for mut file in files {
        let filename = name(&file).to_owned();
        match first.entry(filename.to_ascii_lowercase()) {
            Entry::Occupied(previous) => {
                // The later file's set warnings stay with the export
                let moved: Vec<_> = warnings(&mut file).extract_if(.., |warning| is_set_warning(warning)).collect();
                if let Some(winner) = kept.get_mut(*previous.get()) {
                    warnings(winner).extend(moved);
                    warnings(winner).push(format!(
                        "\"{}\" of a later texture set has the name of a file an earlier texture set exports, so it was not written; add $textureSet to the template's file names or rename a texture set",
                        filename
                    ));
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(kept.len());
                kept.push(file);
            }
        }
    }

    kept
}

/// Write exported files into `dir` (created if missing) via [`crate::write_batch_atomic`]:
/// all filenames are validated and staged before any rename, and a later rename failure rolls
/// back earlier publications (concurrent readers may briefly observe a mixed generation during
/// the rename loop). Names are file names, or relative paths with `/` between sanitized folder
/// names; the folders are created inside `dir`.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_files(files: &[ExportedFile], dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    let mut seen_files = BTreeSet::new();
    let mut seen_dirs = BTreeSet::new();
    let mut rel_targets = Vec::with_capacity(files.len());
    for f in files {
        // Cap the sanitized stem before adding the extension
        let (stem, ext) = f.name.rsplit_once('.').unwrap_or((&f.name, ""));
        if f.name.is_empty() || stem != sanitize_file_path(stem) || !ext.chars().all(|c| c.is_ascii_alphanumeric()) || f.name.trim_matches('.').is_empty() {
            return Err(FormatError::Invalid(format!("\"{}\" is not a file name or a relative path", f.name)));
        }

        let lower = f.name.to_ascii_lowercase();
        if seen_dirs.contains(&lower) {
            return Err(FormatError::Invalid(format!("\"{}\" cannot be both an exported file and a folder", f.name)));
        }
        let mut prefix = lower.as_str();
        while let Some((folder, _)) = prefix.rsplit_once('/') {
            if seen_files.contains(folder) {
                return Err(FormatError::Invalid(format!("\"{}\" cannot use \"{folder}\" as a folder because another exported file has that name", f.name)));
            }
            seen_dirs.insert(folder.to_owned());
            prefix = folder;
        }
        seen_files.insert(lower);
        rel_targets.push((std::path::Path::new(&f.name), f.bytes.as_slice()));
    }

    // Open the output directory once and keep every subfolder creation, staging temp file and rename inside that handle
    std::fs::create_dir_all(dir)?;
    let root_dir = cap_std::fs::Dir::open_ambient_dir(dir, cap_std::ambient_authority())?;
    for (rel, _) in &rel_targets {
        create_export_folders(&root_dir, rel)?;
    }
    crate::write_batch_atomic_in_dir(&root_dir, rel_targets)?;
    Ok(files.iter().map(|f| dir.join(&f.name)).collect())
}

/// Create each folder in `rel` beneath `root`, refusing a component that is a symbolic link so two
/// export names cannot resolve to the same file through an in-tree alias.
#[cfg(not(target_arch = "wasm32"))]
fn create_export_folders(root: &cap_std::fs::Dir, rel: &std::path::Path) -> Result<()> {
    let Some(parent) = rel.parent().filter(|parent| !parent.as_os_str().is_empty()) else {
        return Ok(());
    };

    let mut prefix = std::path::PathBuf::new();
    for component in parent.components() {
        prefix.push(component);
        match root.symlink_metadata(&prefix) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(FormatError::Invalid(format!("\"{}\" cannot use a symbolic link as a folder", rel.display())));
            }
            Ok(meta) if meta.file_type().is_dir() => {}
            Ok(_) => return Err(FormatError::Invalid(format!("\"{}\" cannot use an existing file as a folder", rel.display()))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => root.create_dir(&prefix)?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
