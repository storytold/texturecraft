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
use texturecraft_compose::{BakedMaps, Composite};
use texturecraft_core::{AoMixing, BakedMap, BlockScales, Channel, ChannelKind, NormalFormat, NormalMixing, Project, TextureSet};
use texturecraft_geom::glam::Vec3;
use texturecraft_texel::{ColorSpace, DilationMap, linear_to_srgb};

use crate::{FormatError, MAX_IMAGE_SIZE, Result};

mod encoding;
mod map_override;
use encoding::Encoding;
pub use map_override::MapOverride;

/// Height-to-normal slope: with strength 1, a height change of 1 across 1/16 of the texture
/// width tilts the normal 45°, at any resolution.
const HEIGHT_SPAN: f32 = 16.0;
const JPEG_QUALITY: u8 = 95;
const PSD_MAX_SIDE_PIXELS: u32 = 30_000;
const PSD_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Preset limits (user presets are untrusted JSON).
const MAX_MAPS: usize = 64;
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

/// Why `template` is not a valid file name template: path separators, `..`, control characters
/// and unknown `$` variables are refused.
pub fn template_error(template: &str) -> Option<String> {
    if template.trim().is_empty() || template.len() > MAX_TEMPLATE {
        return Some(format!("file name \"{template}\" is empty or longer than {MAX_TEMPLATE} bytes"));
    }
    if template.contains(['/', '\\']) || template.contains("..") || template.chars().any(char::is_control) {
        return Some(format!("file name \"{template}\" contains a path separator, \"..\" or a control character"));
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
    /// A PSD containing only its merged image.
    Psd,
}

impl FileFormat {
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
    /// Dielectric reflectance at normal incidence: 0.08 × specular level.
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
/// per composite texel, row-major).
#[derive(Clone, Copy)]
pub struct ExportInput<'a> {
    pub project: &'a Project,
    pub ts: usize,
    pub composite: &'a Composite,
    pub baked: &'a BakedMaps,
    pub coverage: &'a [bool],
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
}

// ---------------------------------------------------------------------------------------------
// Planning: which maps exist for a texture set and what their files are called

fn normal_present(set: &TextureSet) -> bool {
    set.has_channel(Channel::Normal) || set.has_channel(Channel::Height) || set.baked.contains(&BakedMap::Normal)
}

/// A source with real data in this texture set (constants never count).
fn source_present(set: &TextureSet, s: &SourceRef) -> bool {
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
    let alpha = get(Dest::A).filter(|s| !matches!(s, SourceRef::Channel { channel, .. } if !set.has_channel(*channel)));
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

fn channel_label(map: &MapSpec) -> &'static str {
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
    let mut out: String = s.chars().take(MAX_STEM).map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' }).collect();
    if out.trim_matches('.').is_empty() {
        out = "texture".into();
    }
    let device = out.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device.len() == 4 && (device.starts_with("COM") || device.starts_with("LPT")) && device.ends_with(|c: char| c.is_ascii_digit()));
    if reserved {
        out.insert(0, '_');
    }
    out
}

/// The files `opts` produces for texture set `ts`, without rendering (for the export dialog).
/// Template variables: [`FILE_TOKENS`].
pub fn list_outputs(project: &Project, ts: usize, mesh_name: &str, opts: &ExportOptions) -> Result<Vec<PlannedFile>> {
    opts.preset.validate()?;
    let set = project.texture_set(ts)?;
    if let Some(s) = opts.size
        && (!s.is_power_of_two() || s > MAX_IMAGE_SIZE)
    {
        return Err(FormatError::Invalid(format!("export size {s} must be a power of two up to {MAX_IMAGE_SIZE}")));
    }

    let mut used = BTreeSet::new();
    let mut out = Vec::new();
    let tiles: Vec<Option<u32>> = if set.uv_tiles.is_empty() || opts.atlas { vec![None] } else { set.uv_tiles.iter().copied().map(Some).collect() };
    let grid = set.tile_grid();
    let block_resolution = set.block_resolution();
    for (i, map) in opts.preset.maps.iter().enumerate() {
        if !map.channels.iter().any(|(_, s)| source_present(set, s)) {
            continue;
        }

        let own = opts.maps.get(&i);
        let format = own.and_then(|o| o.format).or(opts.format_override).unwrap_or(map.format);
        let bit_depth = format.depth(own.and_then(|o| o.bit_depth).or(opts.bit_depth_override).unwrap_or(map.bit_depth));
        let enabled = own.is_none_or(|o| o.enabled);
        let psd_channels = (enabled && format == FileFormat::Psd).then(|| layout(set, map).len());
        let dither = bit_depth == 8 && !format.holds_float(bit_depth) && own.and_then(|o| o.dither).unwrap_or(opts.dither);
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

        // Append `$udim` to names for separate UV tiles
        let template = if tiles.iter().any(Option::is_some) && !map.file.contains("$udim") { format!("{}.$udim", map.file) } else { map.file.clone() };
        for &tile in &tiles {
            if let Some(channels) = psd_channels {
                let size = match (tile, grid) {
                    (Some(tile), Some(_)) => {
                        let side = tile_side(set, tile, block_resolution);
                        let side = scale_export_side(side, block_resolution, opts.size);
                        (side, side)
                    }
                    (_, Some(grid)) => {
                        let side = opts.size.unwrap_or(block_resolution);
                        (side.saturating_mul(grid.cols), side.saturating_mul(grid.rows))
                    }
                    (_, None) => {
                        let side = opts.size.unwrap_or(block_resolution);
                        (side, side)
                    }
                };
                check_psd_size(size, channels, bit_depth)?;
            }

            let stem = template
                .replace("$udim", &tile.unwrap_or(1001).to_string())
                .replace("$project", &project.name)
                .replace("$mesh", mesh_name)
                .replace("$textureSet", &set.name)
                .replace("$channel", channel_label(map))
                .replace("$colorSpace", color_space);

            // Replace the tile number with `<UDIM>` in the shared name
            let token_stem = tile.map(|_| {
                let t = template
                    .replace("$udim", "UDIMTOKEN")
                    .replace("$project", &project.name)
                    .replace("$mesh", mesh_name)
                    .replace("$textureSet", &set.name)
                    .replace("$channel", channel_label(map))
                    .replace("$colorSpace", color_space);
                sanitize_file_stem(&t)
            });
            let stem = sanitize_file_stem(&stem);
            let ext = format.extension();
            let mut name = format!("{stem}.{ext}");
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

            // Compare names case-insensitively for Windows and macOS
            let mut k = 2;
            while used.contains(&name.to_ascii_lowercase()) {
                name = format!("{stem}_{k}.{ext}");
                k += 1;
            }
            if k > 2 {
                warnings.push(format!("\"{stem}.{ext}\" is produced by more than one map; this one was renamed to \"{name}\""));
            }
            used.insert(name.to_ascii_lowercase());
            if !enabled {
                continue;
            }

            if color_map && requested_space != space {
                warnings.push(format!("\"{name}\" packs colour into one channel (L), so it is written in linear Rec.709, not {}", requested_space.name()));
            }

            // Use a UDIM name only when substituting its tile recovers the output name
            // Truncation or literal placeholder text can make that substitution ambiguous
            let udim_name = token_stem.filter(|t| tile.is_some_and(|n| t.contains("UDIMTOKEN") && t.replace("UDIMTOKEN", &n.to_string()) == stem)).map(|t| {
                let renamed = if k > 2 { format!("_{}", k - 1) } else { String::new() };
                format!("{t}{renamed}.{ext}").replace("UDIMTOKEN", "<UDIM>")
            });
            out.push(PlannedFile { name, map: i, format, bit_depth, dither, tile, udim_name, warnings });
        }
    }
    Ok(out)
}

/// Plan a selection in export order, keeping only the first case-insensitive use of a name.
pub fn list_set_outputs(project: &Project, sets: &[usize], mesh_name: &str, opts: &ExportOptions) -> Result<Vec<(usize, PlannedFile)>> {
    let mut files = Vec::new();
    for &set in sets {
        files.extend(list_outputs(project, set, mesh_name, opts)?.into_iter().map(|file| (set, file)));
    }

    Ok(keep_first_names(files, |(_, file)| &file.name, |(_, file)| &mut file.warnings))
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

struct Sampler<'a> {
    set: &'a TextureSet,
    comp: &'a Composite,
    baked: &'a BakedMaps,
    coverage: &'a [bool],
    /// Composite width.
    res: u32,
    /// Texels per atlas block (`res` for a classic set): the height slope and the Sobel window
    /// follow the block.
    tile_res: u32,
    /// The share of its block each tile fills: a smaller tile's height slope shrinks with it.
    scales: BlockScales,
    /// Normal convention overriding every normal output.
    normal_override: Option<NormalFormat>,
    project_normal: NormalFormat,
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

impl Sampler<'_> {
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
        self.coverage.get((y as usize) * self.res as usize + x as usize).copied().unwrap_or(false)
    }

    /// Normal from the height channel (3×3 Sobel; uncovered neighbours repeat the centre).
    fn height_normal(&self, x: u32, y: u32) -> Vec3 {
        let center = self.channel(Channel::Height, At::Texel(x, y))[0];
        // Clamped to the texel's UV tile.
        let t = self.tile_res.max(1);
        let (x0, y0, last) = ((x / t * t) as i64, (y / t * t) as i64, t as i64 - 1);
        let h = |dx: i64, dy: i64| {
            let (sx, sy) = ((x as i64 + dx).clamp(x0, x0 + last) as u32, (y as i64 + dy).clamp(y0, y0 + last) as u32);
            if self.covered(sx, sy) { self.channel(Channel::Height, At::Texel(sx, sy))[0] } else { center }
        };
        let gx = (h(1, -1) + 2.0 * h(1, 0) + h(1, 1) - h(-1, -1) - 2.0 * h(-1, 0) - h(-1, 1)) / 8.0;
        let gy = (h(-1, 1) + 2.0 * h(0, 1) + h(1, 1) - h(-1, -1) - 2.0 * h(0, -1) - h(1, -1)) / 8.0;
        let s = self.set.height_strength * self.tile_res as f32 * self.scales.at(x, y) / HEIGHT_SPAN;
        // Image y points down; OpenGL normals have +Y up the image.
        Vec3::new(-gx * s, gy * s, 1.0).try_normalize().unwrap_or(Vec3::Z)
    }

    /// The final tangent-space normal (OpenGL): baked normal ⊕ (height ⊕ normal channel).
    fn final_normal(&self, at: At) -> Vec3 {
        let At::Texel(x, y) = at else { return Vec3::Z };
        let base = if self.set.baked.contains(&BakedMap::Normal) { unpack(self.baked(BakedMap::Normal, at)) } else { Vec3::Z };
        let mut detail = if self.set.has_channel(Channel::Normal) { unpack(self.channel(Channel::Normal, at)) } else { Vec3::Z };
        if self.set.normal_mixing == NormalMixing::Combine && self.set.has_channel(Channel::Height) && self.set.height_strength != 0.0 {
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
                VirtualMap::F0 => [0.08 * self.channel(Channel::SpecularLevel, at)[0]; 3],
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
    let set = input.project.texture_set(input.ts)?;
    let (width, height) = (input.composite.resolution, input.composite.height);
    let grid = set.tile_grid();
    let tile_res = width / grid.map_or(1, |g| g.cols.max(1));
    if tile_res == 0 || tile_res > MAX_IMAGE_SIZE || height != tile_res * grid.map_or(1, |g| g.rows) {
        return Err(FormatError::Invalid(format!("composite size {width}×{height} does not fit texture set \"{}\"", set.name)));
    }

    if input.coverage.len() != width as usize * height as usize {
        return Err(FormatError::Invalid(format!("coverage has {} texels, expected {width}×{height}", input.coverage.len())));
    }

    if planned.is_empty() {
        return Ok(Vec::new());
    }

    // Padding stays inside each UV tile
    let dilation_map = match options.padding {
        Padding::None => None,
        Padding::Dilation(d) | Padding::DilationDefault(d) | Padding::DilationDiffusion(d) => {
            Some(DilationMap::in_blocks(input.coverage, width, height, Some(d), tile_res))
        }
        Padding::Infinite => Some(DilationMap::in_blocks(input.coverage, width, height, None, tile_res)),
    };
    let beyond_default = matches!(options.padding, Padding::DilationDefault(_) | Padding::Infinite);
    let diffuse = matches!(options.padding, Padding::DilationDiffusion(_));
    let sampler = Sampler {
        set,
        comp: input.composite,
        baked: input.baked,
        coverage: input.coverage,
        res: width,
        tile_res,
        scales: set.block_scales(),
        normal_override: options.normal_format,
        project_normal: input.project.normal_format,
    };
    planned
        .into_iter()
        .map(|planned_file| {
            let map = options.preset.maps.get(planned_file.map).ok_or_else(|| FormatError::Invalid("planned map out of range".into()))?;
            let channel_layout = layout(set, map);
            let component_count = channel_layout.len();
            let inverted: Vec<bool> = channel_layout.iter().map(|(d, _)| map.inverted.contains(d)).collect();

            // The texels written: one tile, or the whole set
            let (x0, y0, w, h) = match (planned_file.tile, grid) {
                (Some(t), Some(g)) => {
                    let (bx, by) = g.block(t).ok_or_else(|| FormatError::Invalid(format!("UV tile {t} is not in the set's grid")))?;

                    // A tile smaller than its block fills the block's top-left corner
                    let side = tile_side(set, t, tile_res);
                    (bx * tile_res, by * tile_res, side, side)
                }
                _ => (0, 0, width, height),
            };

            // An atlas scales by the requested size per tile
            let scale = |value| scale_export_side(value, tile_res, options.size);
            let size = (scale(w), scale(h));
            if planned_file.format == FileFormat::Psd {
                check_psd_size(size, component_count, planned_file.bit_depth)?;
            }

            let mut data = vec![0.0f32; w as usize * h as usize * component_count];
            data.par_chunks_mut(w as usize * component_count).enumerate().for_each(|(row_y, row)| {
                let y = y0 + row_y as u32;
                for (column, x) in (x0..x0 + w).enumerate() {
                    let at = if sampler.covered(x, y) {
                        Some(At::Texel(x, y))
                    } else {
                        match dilation_map.as_ref().and_then(|d| d.source(x, y)) {
                            Some((sx, sy)) => Some(At::Texel(sx, sy)),
                            None => beyond_default.then_some(At::Default),
                        }
                    };
                    let Some(at) = at else { continue };
                    let mut cache = None;
                    for (component, (_, source)) in channel_layout.iter().enumerate() {
                        if let Some(value) = row.get_mut(column * component_count + component) {
                            let sample = sampler.sample(source, at, &mut cache);
                            *value = if inverted.get(component).copied().unwrap_or(false) { 1.0 - sample } else { sample };
                        }
                    }
                }
            });

            // Diffuse into the texels the dilation did not reach
            if diffuse {
                let known: Vec<bool> = (y0..y0 + h)
                    .flat_map(|y| (x0..x0 + w).map(move |x| (x, y)))
                    .map(|(x, y)| sampler.covered(x, y) || dilation_map.as_ref().is_some_and(|d| d.source(x, y).is_some()))
                    .collect();

                // Keep diffusion within each UV tile when rendering an atlas
                let block = if planned_file.tile.is_none() && grid.is_some() { tile_res } else { w.max(h) };
                diffuse_blocks(&mut data, (w as usize, h as usize), component_count, &known, block as usize);
            }

            let mut data = resize(data, (w, h), component_count, size);

            // Integer sRGB converts in `encode`; other color spaces convert here
            let space = options.space_for_map(map, planned_file.format, planned_file.bit_depth);
            let float = planned_file.format.holds_float(planned_file.bit_depth);
            let rgb = component_count >= 3;
            let mut warnings = planned_file.warnings;
            let color_map = map.is_color();
            let convert = (space != ColorSpace::Srgb || float) && space != ColorSpace::LinearRec709;

            if convert {
                data.par_chunks_mut(component_count).for_each(|px| {
                    if let [r, g, b, ..] = px {
                        [*r, *g, *b] = space.from_linear_rec709([*r, *g, *b]);
                    }
                });
            }
            let srgb: Vec<bool> = channel_layout.iter().map(|(d, _)| *d != Dest::A && !float && space == ColorSpace::Srgb).collect();

            // A colour space the user chose is written into the colour maps (data maps have none)
            let embed = (options.color_space.is_some() && color_map && rgb).then_some(space);
            let bytes = encode(
                Encoding { format: planned_file.format, depth: planned_file.bit_depth, dither: planned_file.dither },
                size,
                component_count,
                data,
                &srgb,
                embed,
                &mut warnings,
            )?;
            Ok(ExportedFile { name: planned_file.name, bytes, width: size.0, height: size.1, warnings })
        })
        .collect()
}

fn tile_side(set: &TextureSet, tile: u32, block_resolution: u32) -> u32 {
    ((block_resolution as f32 * set.tile_scale(tile)).round() as u32).clamp(1, block_resolution.max(1))
}

/// Scale an image side relative to the source block, keeping a pixel for any nonempty source.
pub fn scale_export_side(source_side: u32, block_resolution: u32, export_size: Option<u32>) -> u32 {
    let export_size = export_size.unwrap_or(block_resolution);
    let scaled = u64::from(source_side) * u64::from(export_size) / u64::from(block_resolution.max(1));
    u32::try_from(scaled).unwrap_or(u32::MAX).max(u32::from(source_side > 0))
}

fn diffuse_blocks(data: &mut Vec<f32>, (width, height): (usize, usize), channels: usize, known: &[bool], block: usize) {
    // A whole image or a separately exported tile already occupies a contiguous buffer
    if block >= width.max(height) {
        diffuse_fill(data, (width, height), channels, known);
        return;
    }

    for block_y in (0..height).step_by(block.max(1)) {
        for block_x in (0..width).step_by(block.max(1)) {
            let (block_width, block_height) = (block.min(width - block_x), block.min(height - block_y));
            let rows = || (block_y..block_y + block_height).map(|y| (y * width + block_x)..(y * width + block_x + block_width));
            let mut block_data: Vec<f32> =
                rows().flat_map(|row| data.get(row.start * channels..row.end * channels).unwrap_or_default().iter().copied()).collect();
            let block_known: Vec<bool> = rows().flat_map(|row| known.get(row).unwrap_or_default().iter().copied()).collect();
            diffuse_fill(&mut block_data, (block_width, block_height), channels, &block_known);
            for (row, chunk) in rows().zip(block_data.chunks(block_width * channels)) {
                if let Some(destination) = data.get_mut(row.start * channels..row.end * channels) {
                    destination.copy_from_slice(chunk);
                }
            }
        }
    }
}

/// Fill the texels `known` leaves out of a `size` image of `n` components by pull-push
/// diffusion: known texels are averaged down a pyramid, then every level fills its gaps from the
/// smoothly upsampled level below, so a gap takes a soft blend of the nearby known colours.
fn diffuse_fill(data: &mut Vec<f32>, (w, h): (usize, usize), channels: usize, known: &[bool]) {
    if known.iter().all(|channel| *channel) || !known.iter().any(|channel| *channel) {
        return;
    }

    // Keep coverage weights alongside the colour samples at each level
    let mut levels: Vec<(usize, usize, Vec<f32>, Vec<f32>)> =
        vec![(w, h, std::mem::take(data), known.iter().map(|channel| if *channel { 1.0 } else { 0.0 }).collect())];
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
        && matches!(format, FileFormat::Tga | FileFormat::Bmp | FileFormat::Hdr | FileFormat::Jng | FileFormat::Psd)
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

    // Rec.709 luminance weights convert linear RGB to PGM's single gray channel
    const REC709_LUMINANCE_WEIGHTS: [f32; 3] = [0.2126, 0.7152, 0.0722];
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
                [red, green, blue, ..] => {
                    let [red_weight, green_weight, blue_weight] = REC709_LUMINANCE_WEIGHTS;
                    red_weight * red + green_weight * green + blue_weight * blue
                }
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
        .iter()
        .enumerate()
        .map(|(index, value)| if srgb.get(index % component_count).copied().unwrap_or(false) { linear_to_srgb(*value) } else { value.clamp(0.0, 1.0) });
    let (bytes, component_count) = if let Some(indices) = channel_indices {
        // Transfer luminance once before TIFF repeats it across RGB
        let transferred: Vec<f32> = transferred.collect();
        drop(integer_data);
        let remapped = remap(&transferred, component_count, indices);
        drop(transferred);
        (quantize_integer(encoding, size.0, indices.len(), remapped.into_iter())?, indices.len())
    } else {
        // Stream native-order samples directly into the integer buffer
        let bytes = quantize_integer(encoding, size.0, component_count, transferred)?;
        drop(integer_data);
        (bytes, component_count)
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
        FileFormat::Psd => return psd(bytes, size, component_count, depth).map_err(|error| encoding_error(&error)),
        FileFormat::Jng => return jng(&bytes, size, component_count).map_err(|error| encoding_error(&error)),
        FileFormat::Exr | FileFormat::Hdr => return Err(encoding_error(&"unexpected float raster format")),
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

fn quantize_integer(encoding: Encoding, width: u32, component_count: usize, samples: impl ExactSizeIterator<Item = f32>) -> Result<Vec<u8>> {
    let Encoding { format, depth, dither } = encoding;
    let byte_count = samples
        .len()
        .checked_mul(if depth == 16 { 2 } else { 1 })
        .ok_or_else(|| FormatError::Invalid(format!("{} sample buffer size overflows", format.extension())))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(byte_count).map_err(|error| FormatError::Invalid(format!("{} sample buffer allocation failed: {error}", format.extension())))?;

    if depth == 16 {
        bytes.extend(samples.flat_map(|value| ((value * f32::from(u16::MAX) + 0.5) as u16).to_ne_bytes()));
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
            bytes.extend(samples.map(|value| (value * max_sample + 0.5) as u8));
        }
    }

    Ok(bytes)
}

fn check_psd_size((width, height): (u32, u32), channels: usize, depth: u8) -> Result<()> {
    if width == 0 || height == 0 || width > PSD_MAX_SIDE_PIXELS || height > PSD_MAX_SIDE_PIXELS {
        return Err(FormatError::Invalid(format!("PSD size {width}×{height} must be between 1 and {PSD_MAX_SIDE_PIXELS} pixels per side")));
    }

    // Measure the flat file's overhead without allocating any pixel buffers
    let overhead = flat_psd((width, height), channels, depth).encoded_len().map_err(|error| FormatError::Invalid(format!("PSD sizing failed: {error}")))?;
    let sample_bytes = u64::from(width) * u64::from(height) * channels as u64 * u64::from(depth / 8);
    if overhead + sample_bytes >= PSD_MAX_FILE_BYTES {
        return Err(FormatError::Invalid(format!("PSD size {width}×{height} would reach the 2 GiB file limit; use a smaller export size or another format")));
    }

    Ok(())
}

fn flat_psd((width, height): (u32, u32), channels: usize, depth: u8) -> craft_psd::PsdFile {
    use craft_psd::{ColorMode, Compression, Header, ImageData, ImageResource, PsdFile, Version};

    const PSD_ALPHA_NAMES_RESOURCE_ID: u16 = 1006;
    let color_mode = if channels <= 2 { ColorMode::Grayscale } else { ColorMode::Rgb };
    let header = Header::new(Version::Psd, width, height, channels as u16, u16::from(depth), color_mode);

    // A Pascal name marks the extra plane as a channel separate from merged transparency
    let resources = if matches!(channels, 2 | 4) { vec![ImageResource::new(PSD_ALPHA_NAMES_RESOURCE_ID, b"\x05Alpha".to_vec())] } else { Vec::new() };

    PsdFile {
        header,
        color_mode_data: Vec::new(),
        resources,
        layer_info: None,
        layer_info_placement: Default::default(),
        global_layer_mask: None,
        global_blocks: Vec::new(),
        layer_mask_trailing: Vec::new(),
        image_data: ImageData::new(Compression::Raw, Vec::new()),
        write_as_psb: false,
    }
}

/// Write a merged PSD image with raw planes, including an extra alpha plane when present.
fn psd(bytes: Vec<u8>, size: (u32, u32), channels: usize, depth: u8) -> craft_psd::Result<Vec<u8>> {
    let mut file = flat_psd(size, channels, depth);
    let planes = &mut file.image_data.data;
    planes.try_reserve_exact(bytes.len()).map_err(|_| craft_psd::PsdError::OutOfMemory("cannot allocate PSD planes"))?;
    if depth == 16 {
        let samples = bytes.as_chunks::<2>().0;
        for channel in 0..channels {
            planes.extend(samples.iter().skip(channel).step_by(channels).flat_map(|sample| u16::from_ne_bytes(*sample).to_be_bytes()));
        }
    } else {
        for channel in 0..channels {
            planes.extend(bytes.iter().skip(channel).step_by(channels).copied());
        }
    }
    drop(bytes);

    file.to_bytes()
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

fn keep_first_names<T>(files: Vec<T>, name: impl Fn(&T) -> &str, warnings: impl Fn(&mut T) -> &mut Vec<String>) -> Vec<T> {
    let mut first = HashMap::new();
    let mut kept = Vec::with_capacity(files.len());
    for file in files {
        let filename = name(&file);
        match first.entry(filename.to_ascii_lowercase()) {
            Entry::Occupied(previous) => {
                if let Some(winner) = kept.get_mut(*previous.get()) {
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

/// Write exported files into `dir` (created if missing), each atomically (`name.tmp`, then
/// rename). Names must be plain file names.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_files(files: &[ExportedFile], dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    std::fs::create_dir_all(dir)?;
    files
        .iter()
        .map(|f| {
            // Cap the sanitized stem before adding the extension
            let (stem, ext) = f.name.rsplit_once('.').unwrap_or((&f.name, ""));
            if f.name.is_empty() || stem != sanitize_file_stem(stem) || !ext.chars().all(|c| c.is_ascii_alphanumeric()) || f.name.trim_matches('.').is_empty() {
                return Err(FormatError::Invalid(format!("\"{}\" is not a plain file name", f.name)));
            }

            let path = dir.join(&f.name);
            crate::write_atomic(&path, &f.bytes)?;
            Ok(path)
        })
        .collect()
}

#[cfg(test)]
mod tests;
