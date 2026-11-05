//! Recognising the maps of a PBR texture set from file names.
//!
//! Texture sites, scanners and game engines name the same maps in many ways (`Brick_Albedo.png`,
//! `brick_diff_4k.jpg`, `T_Brick_BC.tga`, `brick-NormalDX.exr`, `Brick_ARM.png`, `_BaseMap`...).
//! This module turns a list of file names into one [`PbrSet`]: for each [`MapKind`] the best
//! file, the other candidates, the normal-map convention and the files it ignored. It is pure
//! (names in, names out), so it works in the browser too and is tested with a table of
//! real-world styles.
//!
//! # Rules
//!
//! 1. Only image files count (`png jpg jpeg tga tif tiff bmp webp exr hdr`), matched
//!    case-insensitively. Dot files (`._x.png`, made by archivers on a Mac), files in
//!    `__MACOSX` and preview images (a token such as `preview`, `thumb`, `thumbnail`, `sphere`,
//!    `render`, `swatch`, `icon`, `screenshot`) are ignored.
//! 2. The file stem is split into lower-case tokens at separators (`_ - . space`), at
//!    lower→upper case changes (`BaseColor`) and between letters and digits (`diff4k`).
//! 3. The map is named by the **last** known word (or word pair); resolution (`1k`..`16k`,
//!    `2048`), bit depth (`16bit`), convention (`gl`, `dx`) and other tokens are skipped.
//!    Pairs (`base color`, `ambient occlusion`, `ao rough metal`) and glued names
//!    (`basecolor`, `normaldx`) are understood.
//! 4. Normal maps: `gl` / `ogl` / `opengl` means OpenGL, `dx` / `d3d` / `directx` means DirectX;
//!    with neither the convention is unknown and the caller's default applies.
//! 5. When several files name the same map the best wins: for normals the OpenGL one (no
//!    flip), then the highest resolution token, then the largest file, then the name. All of
//!    them stay in [`PbrSet::candidates`] so a caller can ask the user.
//!
//! Single letters are not accepted, except a trailing `n` (normal): `r`, `m`, `d` and `h` are
//! ambiguous with colour channels.

use std::collections::BTreeMap;

use texturecraft_core::NormalFormat;

/// The kinds of map a PBR folder holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MapKind {
    BaseColor,
    Roughness,
    /// Glossiness is the inverse of roughness; the importer inverts it.
    Glossiness,
    Metallic,
    Normal,
    Height,
    AmbientOcclusion,
    Emissive,
    Opacity,
    /// One image: occlusion in red, roughness in green, metallic in blue (`ORM`, `ARM`).
    PackedOrm,
    /// Recognised, but TextureCraft has no channel for it (cavity, specular, ...): reported,
    /// not imported.
    Unused,
}

impl MapKind {
    const IMPORTED: [MapKind; 10] = [
        MapKind::BaseColor,
        MapKind::Roughness,
        MapKind::Glossiness,
        MapKind::Metallic,
        MapKind::Normal,
        MapKind::Height,
        MapKind::AmbientOcclusion,
        MapKind::Emissive,
        MapKind::Opacity,
        MapKind::PackedOrm,
    ];

    /// Stable id for JSON.
    pub fn id(self) -> &'static str {
        match self {
            MapKind::BaseColor => "baseColor",
            MapKind::Roughness => "roughness",
            MapKind::Glossiness => "glossiness",
            MapKind::Metallic => "metallic",
            MapKind::Normal => "normal",
            MapKind::Height => "height",
            MapKind::AmbientOcclusion => "ambientOcclusion",
            MapKind::Emissive => "emissive",
            MapKind::Opacity => "opacity",
            MapKind::PackedOrm => "orm",
            MapKind::Unused => "unused",
        }
    }

    /// The kind with this JSON id.
    pub fn from_id(id: &str) -> Option<MapKind> {
        Self::IMPORTED.into_iter().find(|k| k.id() == id)
    }
}

/// File extensions read as images (lower case).
pub const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "tga", "tif", "tiff", "bmp", "webp", "exr", "hdr"];

/// What the name of one file says.
#[derive(Clone, Debug, PartialEq)]
pub struct Detected {
    pub kind: MapKind,
    /// The convention named for a normal map, if any.
    pub normal: Option<NormalFormat>,
    /// Resolution in pixels named by a `2k` / `2048` token, 0 if none.
    pub resolution: u32,
    /// Higher is a stronger match (a weak word such as `bump` loses to a strong one).
    pub strength: u8,
}

/// One candidate file.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    /// The file's path as given.
    pub name: String,
    pub detected: Detected,
    /// File size in bytes (a tie-breaker; 0 when unknown).
    pub bytes: u64,
}

/// The maps found among a list of files.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PbrSet {
    /// The best file per kind.
    pub chosen: BTreeMap<MapKind, Candidate>,
    /// Every file that names each kind, best first (the chosen one is first).
    pub candidates: BTreeMap<MapKind, Vec<Candidate>>,
    /// Files not used, with why (`not an image`, `preview image`, `unknown map name`,
    /// `unused map`, `another file is used for <kind>`).
    pub ignored: Vec<(String, String)>,
}

impl PbrSet {
    /// True when no map was recognised.
    pub fn is_empty(&self) -> bool {
        self.chosen.is_empty()
    }
}

// ---------------------------------------------------------------------------------------------
// Tokens.

/// Lower-case tokens of a file stem: split at separators, camelCase changes and letter/digit
/// changes; a number and its unit stay together (`4k`, `16bit`).
fn tokens(stem: &str) -> Vec<String> {
    let chars: Vec<char> = stem.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            // aB -> a|B; ABc -> A|Bc; a1 -> a|1; 1a -> 1|a.
            let split =
                (p.is_lowercase() && c.is_uppercase()) || (p.is_uppercase() && c.is_uppercase() && next_lower) || (p.is_alphabetic() != c.is_alphabetic());
            if split && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        }
        cur.extend(c.to_lowercase());
        prev = Some(c);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    let mut joined: Vec<String> = Vec::new();
    for t in out {
        if let Some(last) = joined.last_mut()
            && !last.is_empty()
            && last.chars().all(|c| c.is_ascii_digit())
            && matches!(t.as_str(), "k" | "bit" | "px")
        {
            last.push_str(&t);
            continue;
        }
        joined.push(t);
    }
    joined
}

/// Pixels named by a resolution token: `1k`..`16k`, `512px`, or a bare power of two ≥ 128.
fn resolution_of(t: &str) -> Option<u32> {
    if let Some(n) = t.strip_suffix('k').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=16).contains(&n).then_some(n * 1024);
    }
    if let Some(n) = t.strip_suffix("px").and_then(|n| n.parse::<u32>().ok()) {
        return (16..=16384).contains(&n).then_some(n);
    }
    let n: u32 = t.parse().ok()?;
    ((128..=16384).contains(&n) && n.is_power_of_two()).then_some(n)
}

fn normal_format_of(t: &str) -> Option<NormalFormat> {
    match t {
        "gl" | "ogl" | "opengl" | "normalgl" => Some(NormalFormat::OpenGl),
        "dx" | "d3d" | "directx" | "normaldx" => Some(NormalFormat::DirectX),
        _ => None,
    }
}

/// A single known word → (kind, strength).
fn word(t: &str) -> Option<(MapKind, u8)> {
    use MapKind::*;
    Some(match t {
        "basecolor" | "basecolour" | "albedo" | "diffuse" | "diff" | "dif" | "col" | "color" | "colour" | "bc" | "basemap" | "maintex" | "alb" => {
            (BaseColor, 3)
        }
        "roughness" | "rough" | "rgh" | "rou" => (Roughness, 3),
        "gloss" | "glossiness" | "glossy" | "smoothness" | "smooth" => (Glossiness, 3),
        "metallic" | "metalness" | "metal" | "mtl" | "met" => (Metallic, 3),
        "normal" | "nrm" | "nor" | "norm" | "nrml" | "normalmap" | "bumpmap" | "nm" | "n" | "normalgl" | "normaldx" => (Normal, 3),
        "height" | "disp" | "displacement" | "displace" | "heightmap" | "hgt" | "dsp" | "parallax" => (Height, 3),
        "bump" => (Height, 1),
        "ao" | "ambientocclusion" | "occlusion" | "occ" | "ambocc" | "occlusionmap" => (AmbientOcclusion, 3),
        "emissive" | "emission" | "emit" | "emis" | "glow" | "emissivecolor" | "emissionmap" | "illum" => (Emissive, 3),
        "opacity" | "alpha" | "opac" | "transparency" | "cutout" | "coverage" => (Opacity, 3),
        "orm" | "arm" | "aorm" => (PackedOrm, 4),
        "cavity" | "curvature" | "specular" | "spec" | "translucency" | "thickness" | "scatter" | "sss" | "fuzz" | "sheen" | "ior" | "mask" | "flow"
        | "reflection" => (Unused, 1),
        _ => return None,
    })
}

fn is_preview(t: &str) -> bool {
    matches!(t, "preview" | "previews" | "thumb" | "thumbs" | "thumbnail" | "sphere" | "render" | "swatch" | "icon" | "screenshot" | "prev" | "overview")
}

fn is_junk_path(path: &str) -> bool {
    path.split(['/', '\\']).any(|part| part == "__MACOSX" || part.starts_with("._") || part == ".DS_Store")
}

/// What the file `name` (a path or a bare name) is, or why it is not a map.
pub fn classify(name: &str) -> std::result::Result<Detected, &'static str> {
    if is_junk_path(name) {
        return Err("hidden or archive metadata file");
    }
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let (stem, ext) = leaf.rsplit_once('.').unwrap_or((leaf, ""));
    if !IMAGE_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
        return Err("not an image");
    }
    let toks = tokens(stem);
    if toks.iter().any(|t| is_preview(t)) {
        return Err("preview image");
    }
    let resolution = toks.iter().filter_map(|t| resolution_of(t)).max().unwrap_or(0);
    let normal = toks.iter().find_map(|t| normal_format_of(t));
    // (token index, kind, strength) of every known word.
    let words: Vec<(usize, MapKind, u8)> = toks.iter().enumerate().filter_map(|(i, t)| word(t).map(|(k, s)| (i, k, s))).collect();

    let done = |kind: MapKind, strength: u8| Ok(Detected { kind, normal: if kind == MapKind::Normal { normal } else { None }, resolution, strength });

    // `ao rough metal` written as three words.
    for w in words.windows(3) {
        if let [(a, MapKind::AmbientOcclusion, _), (b, MapKind::Roughness, _), (c, MapKind::Metallic, _)] = w
            && *b == a + 1
            && *c == b + 1
        {
            return done(MapKind::PackedOrm, 4);
        }
    }
    // Word pairs and the position they end at.
    let mut pair: Option<(usize, MapKind)> = None;
    for (i, t) in toks.iter().enumerate() {
        let next = toks.get(i + 1).map(String::as_str);
        let kind = match (t.as_str(), next) {
            ("base", Some("color" | "colour" | "map")) | ("main", Some("tex")) => Some(MapKind::BaseColor),
            ("ambient", Some("occlusion")) | ("mixed", Some("ao")) => Some(MapKind::AmbientOcclusion),
            ("bump", Some("map")) => Some(MapKind::Normal),
            _ => None,
        };
        if let Some(kind) = kind {
            pair = Some((i + 1, kind));
        }
    }
    // The last known word wins; a pair counts if it ends at or after it.
    let single = words.iter().rev().find(|(i, kind, _)| {
        // A lone `n` only names a normal map as the final meaningful token.
        let lone_n = toks.get(*i).is_some_and(|t| t == "n");
        !lone_n
            || (*kind == MapKind::Normal && toks.iter().skip(i + 1).all(|t| resolution_of(t).is_some() || normal_format_of(t).is_some() || t.ends_with("bit")))
    });
    match (pair, single) {
        (Some((pi, kind)), Some((si, _, _))) if pi >= *si => done(kind, 3),
        (Some((_, kind)), None) => done(kind, 3),
        (_, Some((_, MapKind::Height, 1))) => {
            // `bump` is weak: any stronger word in the name wins over it.
            match words.iter().rev().find(|(_, k, s)| *s >= 3 && *k != MapKind::Unused) {
                Some((_, k, s)) => done(*k, *s),
                None => done(MapKind::Height, 1),
            }
        }
        (_, Some((_, kind, strength))) => done(*kind, *strength),
        (None, None) => Err("unknown map name"),
    }
}

/// Sort key for files naming the same map: smaller is better.
type Rank = (u8, std::cmp::Reverse<u8>, std::cmp::Reverse<u32>, std::cmp::Reverse<u64>, String);

fn rank(c: &Candidate) -> Rank {
    // OpenGL first (no flip), then unlabelled, then DirectX.
    let fmt = match c.detected.normal {
        Some(NormalFormat::OpenGl) => 0,
        None => 1,
        Some(NormalFormat::DirectX) => 2,
    };
    (fmt, std::cmp::Reverse(c.detected.strength), std::cmp::Reverse(c.detected.resolution), std::cmp::Reverse(c.bytes), c.name.to_ascii_lowercase())
}

/// Group `files` (`(path, size in bytes)`) into a [`PbrSet`].
pub fn detect<'a>(files: impl IntoIterator<Item = (&'a str, u64)>) -> PbrSet {
    let mut set = PbrSet::default();
    for (name, bytes) in files {
        match classify(name) {
            Err(why) => set.ignored.push((name.to_string(), why.to_string())),
            Ok(d) if d.kind == MapKind::Unused => set.ignored.push((name.to_string(), "unused map".to_string())),
            Ok(detected) => set.candidates.entry(detected.kind).or_default().push(Candidate { name: name.to_string(), detected, bytes }),
        }
    }
    for (kind, list) in &mut set.candidates {
        list.sort_by_key(rank);
        if let Some(best) = list.first() {
            set.chosen.insert(*kind, best.clone());
        }
        for other in list.iter().skip(1) {
            set.ignored.push((other.name.clone(), format!("another file is used for {}", kind.id())));
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;
    use MapKind::*;

    fn kind(name: &str) -> Option<MapKind> {
        classify(name).ok().map(|d| d.kind)
    }

    #[test]
    fn real_world_naming_styles() {
        let table: &[(&str, MapKind)] = &[
            // Plain suffixes, any case and separator.
            ("brick_albedo.png", BaseColor),
            ("brick_basecolor.png", BaseColor),
            ("brick_base_color.png", BaseColor),
            ("Brick_BaseColor.png", BaseColor),
            ("Brick-Base-Color.PNG", BaseColor),
            ("brick_diffuse.jpg", BaseColor),
            ("brick_diff_4k.jpg", BaseColor),
            ("brick_col.tga", BaseColor),
            ("brick_color.png", BaseColor),
            ("Brick_Colour.png", BaseColor),
            ("brick_roughness.png", Roughness),
            ("brick_rough.png", Roughness),
            ("brick_Rough_2K.jpg", Roughness),
            ("brick_metallic.png", Metallic),
            ("brick_metalness.png", Metallic),
            ("brick_metal.png", Metallic),
            ("brick_normal.png", Normal),
            ("brick_nrm.png", Normal),
            ("brick_nor_gl_4k.exr", Normal),
            ("brick_nor_dx_4k.exr", Normal),
            ("brick_NormalGL.jpg", Normal),
            ("brick_NormalDX.jpg", Normal),
            ("brick_normal_opengl.png", Normal),
            ("brick_n.png", Normal),
            ("brick_n_2k.png", Normal),
            ("brick_height.png", Height),
            ("brick_disp_4k.png", Height),
            ("brick_displacement.png", Height),
            ("brick_Displacement.jpg", Height),
            ("brick_ao.png", AmbientOcclusion),
            ("brick_AO.png", AmbientOcclusion),
            ("brick_ambientocclusion.png", AmbientOcclusion),
            ("brick_AmbientOcclusion.jpg", AmbientOcclusion),
            ("brick_ambient_occlusion.png", AmbientOcclusion),
            ("brick_occlusion.png", AmbientOcclusion),
            ("brick_mixed_ao.png", AmbientOcclusion),
            ("brick_emissive.png", Emissive),
            ("brick_emission.png", Emissive),
            ("brick_opacity.png", Opacity),
            ("brick_alpha.png", Opacity),
            ("brick_orm.png", PackedOrm),
            ("brick_arm_2k.jpg", PackedOrm),
            ("brick_ao_rough_metal.png", PackedOrm),
            // Prefix styles and engine conventions.
            ("Albedo_brick.png", BaseColor),
            ("T_Brick_BC.tga", BaseColor),
            ("T_Brick_N.tga", Normal),
            ("T_Brick_ORM.tga", PackedOrm),
            ("brick_BaseMap.png", BaseColor),
            ("brick_MainTex.png", BaseColor),
            ("brick_BumpMap.png", Normal),
            ("brick_OcclusionMap.png", AmbientOcclusion),
            ("brick_EmissionMap.png", Emissive),
            // Scan sites: ids, resolution and format tokens in the name.
            ("Bricks059_4K-JPG_Color.jpg", BaseColor),
            ("Bricks059_4K-JPG_NormalGL.jpg", Normal),
            ("Bricks059_4K-JPG_Roughness.jpg", Roughness),
            ("Bricks059_1K_AmbientOcclusion.png", AmbientOcclusion),
            ("wall_1234_8k_Albedo.png", BaseColor),
            ("wood16bit_roughness.png", Roughness),
            // Weak and inverted maps.
            ("brick_bump.png", Height),
            ("WoodFloor_Gloss.png", Glossiness),
            ("wood_smoothness.png", Glossiness),
        ];
        let wrong: Vec<String> =
            table.iter().filter(|(name, want)| kind(name) != Some(*want)).map(|(name, want)| format!("{name}: got {:?}, want {want:?}", kind(name))).collect();
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn non_maps_are_ignored_with_a_reason() {
        for (name, why) in [
            ("readme.txt", "not an image"),
            ("brick.mtlx", "not an image"),
            ("brick_preview.png", "preview image"),
            ("Brick_Thumbnail.jpg", "preview image"),
            ("sphere_render.png", "preview image"),
            ("._brick_albedo.png", "hidden or archive metadata file"),
            ("__MACOSX/brick_albedo.png", "hidden or archive metadata file"),
            ("brick.png", "unknown map name"),
            ("", "not an image"),
        ] {
            assert_eq!(classify(name).unwrap_err(), why, "{name}");
        }
        // Recognised words with no channel here are reported as unused, not as maps.
        assert_eq!(kind("brick_cavity.png"), Some(Unused));
        assert!(detect([("brick_cavity.png", 1)]).ignored.iter().any(|(_, w)| w == "unused map"));
    }

    #[test]
    fn a_weak_word_loses_to_a_strong_one() {
        // `bump` alone is a height map; next to a stronger word that word names the file.
        assert_eq!(kind("brick_bump.png"), Some(Height));
        assert_eq!(kind("bump_brick_normal.png"), Some(Normal));
        // A pair at the end beats an earlier single word.
        assert_eq!(kind("diffuse_wall_ambient_occlusion.png"), Some(AmbientOcclusion));
    }

    #[test]
    fn normal_convention_comes_from_the_name() {
        let n = |name: &str| classify(name).unwrap().normal;
        assert_eq!(n("a_normal_gl.png"), Some(NormalFormat::OpenGl));
        assert_eq!(n("a_normal_opengl.png"), Some(NormalFormat::OpenGl));
        assert_eq!(n("a_normal_dx.png"), Some(NormalFormat::DirectX));
        assert_eq!(n("a_NormalDX.png"), Some(NormalFormat::DirectX));
        assert_eq!(n("a_nor_directx_2k.png"), Some(NormalFormat::DirectX));
        assert_eq!(n("a_normal.png"), None);
        // `dx` on a map that is not a normal map says nothing.
        assert_eq!(n("a_roughness_dx.png"), None);
    }

    #[test]
    fn several_candidates_pick_gl_then_resolution_then_size() {
        let set = detect([
            ("m/n_dx.png", 10),
            ("m/n_gl.png", 5),
            ("m/albedo_1k.png", 900),
            ("m/albedo_4k.png", 100),
            ("m/albedo_2k.png", 500),
            ("m/rough.png", 7),
            ("m/rough.jpg", 9),
            ("m/notes.txt", 1),
            ("m/preview.png", 1),
            ("m/mystery.png", 1),
        ]);
        let pick = |k| set.chosen.get(&k).map(|c| c.name.as_str());
        // The convention that needs no flip wins even when the DirectX file is bigger.
        assert_eq!(pick(Normal), Some("m/n_gl.png"));
        // The higher resolution token wins over file size.
        assert_eq!(pick(BaseColor), Some("m/albedo_4k.png"));
        // No resolution token: the larger file.
        assert_eq!(pick(Roughness), Some("m/rough.jpg"));
        assert_eq!(set.candidates[&BaseColor].len(), 3);
        let why = |name: &str| set.ignored.iter().find(|(n, _)| n == name).map(|(_, w)| w.as_str());
        assert_eq!(why("m/notes.txt"), Some("not an image"));
        assert_eq!(why("m/preview.png"), Some("preview image"));
        assert_eq!(why("m/mystery.png"), Some("unknown map name"));
        assert_eq!(why("m/albedo_1k.png"), Some("another file is used for baseColor"));
    }

    #[test]
    fn empty_and_hostile_names_do_not_panic() {
        assert!(detect([]).is_empty());
        let long = "a".repeat(100_000);
        let weird = [
            "\u{0}.png",
            "....png",
            "ÄÖÜ_Ålbedo.png",
            "🙂.png",
            "_.png",
            "-.exr",
            long.as_str(),
            "x/../../y_albedo.png",
            "C:\\a\\b_albedo.png",
            "99999999999999_albedo.png",
        ];
        let set = detect(weird.iter().map(|n| (*n, 0)));
        assert!(set.chosen.contains_key(&BaseColor));
    }
}
