//! Colour swatches: named colours kept in the project or in the user's preferences, a short list
//! of recently used colours, and the two palette text formats (GIMP `.gpl` and a hex list).
//!
//! Colours are stored **linear** (like every colour in the document) and shown as sRGB; `#rrggbb`
//! is the sRGB 8-bit form. Everything read from a file is hostile: sizes, counts, numbers and
//! names are capped or rejected, never trusted.

use serde::{Deserialize, Serialize};

/// Most swatches in one list (project or global), and the most read from one palette file.
pub const MAX_SWATCHES: usize = 512;
/// Longest swatch name, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// Colours the recent row keeps.
pub const MAX_RECENT: usize = 8;
/// Largest palette text read (bytes).
pub const MAX_PALETTE_BYTES: usize = 1 << 20;

/// A named colour. `rgb` is linear.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    #[serde(default)]
    pub name: String,
    pub rgb: [f32; 3],
}

impl Swatch {
    /// A swatch for a linear colour; the name is cleaned (see [`clean_name`]) and the colour
    /// made finite and within 0..1.
    pub fn new(name: &str, rgb: [f32; 3]) -> Swatch {
        Swatch { name: clean_name(name), rgb: sanitize(rgb) }
    }

    /// The default name of a colour: its hex code.
    pub fn hex(&self) -> String {
        to_hex(self.rgb)
    }
}

/// sRGB (0..1) → linear (0..1). The same curve as `texturecraft_texel::srgb_to_linear`.
pub fn srgb_to_linear(v: f32) -> f32 {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Linear (0..1) → sRGB (0..1). The same curve as `texturecraft_texel::linear_to_srgb`.
pub fn linear_to_srgb(v: f32) -> f32 {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// An sRGB byte as a linear value.
pub fn srgb8_to_linear(v: u8) -> f32 {
    srgb_to_linear(f32::from(v) / 255.0)
}

/// A linear value as the nearest sRGB byte.
pub fn linear_to_srgb8(v: f32) -> u8 {
    (linear_to_srgb(v) * 255.0 + 0.5) as u8
}

/// The sRGB bytes of a linear colour.
pub fn to_srgb8(rgb: [f32; 3]) -> [u8; 3] {
    rgb.map(linear_to_srgb8)
}

/// The linear colour of sRGB bytes.
pub fn from_srgb8(rgb: [u8; 3]) -> [f32; 3] {
    rgb.map(srgb8_to_linear)
}

/// A colour with NaN and infinities turned to 0 and every channel within 0..1.
pub fn sanitize(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 })
}

/// `#rrggbb` for a linear colour.
pub fn to_hex(rgb: [f32; 3]) -> String {
    let b = to_srgb8(rgb);
    format!("#{:02x}{:02x}{:02x}", b[0], b[1], b[2])
}

/// A linear colour from `#rrggbb`, `rrggbb`, `#rgb` or `rgb` (any case).
pub fn parse_hex(text: &str) -> Option<[f32; 3]> {
    let t = text.trim();
    let t = t.strip_prefix('#').unwrap_or(t);
    if !t.is_ascii() || !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let digit = |i: usize, n: usize| u8::from_str_radix(t.get(i..i + n)?, 16).ok();
    let bytes = match t.len() {
        6 => [digit(0, 2)?, digit(2, 2)?, digit(4, 2)?],
        3 => [digit(0, 1)? * 17, digit(1, 1)? * 17, digit(2, 1)? * 17],
        _ => return None,
    };
    Some(from_srgb8(bytes))
}

/// A swatch name without control characters, trimmed and cut to [`MAX_NAME_CHARS`].
pub fn clean_name(name: &str) -> String {
    let kept: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    kept.trim().chars().take(MAX_NAME_CHARS).collect()
}

/// Whether two linear colours are the same sRGB bytes (what the eye and the hex code tell apart).
pub fn same_colour(a: [f32; 3], b: [f32; 3]) -> bool {
    to_srgb8(a) == to_srgb8(b)
}

/// Record `colour` as the most recently used: first in the list, any earlier copy removed, at
/// most [`MAX_RECENT`] kept.
pub fn push_recent(recent: &mut Vec<[f32; 3]>, colour: [f32; 3]) {
    let colour = sanitize(colour);
    recent.retain(|c| !same_colour(*c, colour));
    recent.insert(0, colour);
    recent.truncate(MAX_RECENT);
}

/// Move the swatch at `from` so it sits at `to` (both clamped into the list). Returns false for
/// an empty list or no change.
pub fn move_swatch(list: &mut Vec<Swatch>, from: usize, to: usize) -> bool {
    if list.is_empty() || from >= list.len() {
        return false;
    }
    let to = to.min(list.len() - 1);
    if from == to {
        return false;
    }
    let s = list.remove(from);
    list.insert(to, s);
    true
}

/// Parse a GIMP palette (`.gpl`): the `GIMP Palette` line, optional `Name:` and `Columns:`
/// lines, `#` comments, then `R G B [name]` lines with 0–255 integers. Returns the palette's
/// name (may be empty) and its swatches. Bad lines are skipped (counted in the third value);
/// text that is not a palette is an error.
pub fn parse_gpl(text: &str) -> Result<(String, Vec<Swatch>, usize), String> {
    if text.len() > MAX_PALETTE_BYTES {
        return Err("the palette file is too large".into());
    }
    let mut lines = text.lines();
    let first = lines.next().map(|l| l.trim_start_matches('\u{feff}').trim());
    if first != Some("GIMP Palette") {
        return Err("not a GIMP palette (the first line must be `GIMP Palette`)".into());
    }
    let (mut name, mut swatches, mut skipped) = (String::new(), Vec::new(), 0usize);
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(v) = line.strip_prefix("Name:") {
            name = clean_name(v);
            continue;
        }
        if line.starts_with("Columns:") {
            continue;
        }
        if swatches.len() >= MAX_SWATCHES {
            skipped += 1;
            continue;
        }
        let mut parts = line.split_whitespace();
        let mut ch = [0u8; 3];
        let mut ok = true;
        for c in &mut ch {
            match parts.next().and_then(|p| p.parse::<u8>().ok()) {
                Some(v) => *c = v,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            skipped += 1;
            continue;
        }
        let rest = parts.collect::<Vec<_>>().join(" ");
        swatches.push(Swatch::new(&rest, from_srgb8(ch)));
    }
    Ok((name, swatches, skipped))
}

/// A GIMP palette for `swatches` (unnamed ones are written with their hex code).
pub fn write_gpl(name: &str, swatches: &[Swatch]) -> String {
    let mut out = String::from("GIMP Palette\n");
    let name = clean_name(name);
    out.push_str(&format!("Name: {}\nColumns: 8\n#\n", if name.is_empty() { "TextureCraft" } else { &name }));
    for s in swatches.iter().take(MAX_SWATCHES) {
        let b = to_srgb8(s.rgb);
        let n = clean_name(&s.name);
        out.push_str(&format!("{:3} {:3} {:3} {}\n", b[0], b[1], b[2], if n.is_empty() { s.hex() } else { n }));
    }
    out
}

/// Parse a hex list: one `#rrggbb` (or `rrggbb`, `#rgb`) per line, optionally followed by a name.
/// Blank lines and lines starting with `//` or `;` are comments. Returns the swatches and the
/// number of lines skipped as unreadable.
pub fn parse_hex_list(text: &str) -> Result<(Vec<Swatch>, usize), String> {
    if text.len() > MAX_PALETTE_BYTES {
        return Err("the palette file is too large".into());
    }
    let (mut swatches, mut skipped) = (Vec::new(), 0usize);
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with(';') {
            continue;
        }
        let (code, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match parse_hex(code) {
            Some(rgb) if swatches.len() < MAX_SWATCHES => swatches.push(Swatch::new(rest, rgb)),
            _ => skipped += 1,
        }
    }
    Ok((swatches, skipped))
}

/// A hex list for `swatches`: `#rrggbb name` per line.
pub fn write_hex_list(swatches: &[Swatch]) -> String {
    let mut out = String::new();
    for s in swatches.iter().take(MAX_SWATCHES) {
        let n = clean_name(&s.name);
        out.push_str(&s.hex());
        if !n.is_empty() {
            out.push(' ');
            out.push_str(&n);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_srgb_byte_survives_the_linear_round_trip() {
        for v in 0..=255u8 {
            assert_eq!(linear_to_srgb8(srgb8_to_linear(v)), v, "byte {v}");
        }
    }

    #[test]
    fn known_values_are_exact() {
        assert_eq!(srgb8_to_linear(0), 0.0);
        assert_eq!(srgb8_to_linear(255), 1.0);
        // Mid-gray 188 is about 0.5 linear; 128 is about 0.2158.
        assert!((srgb8_to_linear(188) - 0.5).abs() < 0.003);
        assert!((srgb8_to_linear(128) - 0.215_86).abs() < 1e-4);
        assert_eq!(to_hex([1.0, 0.0, 0.0]), "#ff0000");
        assert_eq!(to_hex(from_srgb8([0x12, 0xab, 0xef])), "#12abef");
    }

    #[test]
    fn hostile_numbers_are_made_safe() {
        assert_eq!(sanitize([f32::NAN, f32::INFINITY, -3.0]), [0.0, 0.0, 0.0]);
        assert_eq!(to_hex([f32::NAN, 2.0, f32::NEG_INFINITY]), "#00ff00");
        let s = Swatch::new("a\nb\u{0}c", [9.0, -1.0, f32::NAN]);
        assert_eq!(s.name, "a b c");
        assert_eq!(s.rgb, [1.0, 0.0, 0.0]);
        assert_eq!(clean_name(&"é".repeat(500)).chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn hex_parsing_accepts_the_usual_forms_only() {
        assert_eq!(to_hex(parse_hex("#FFaa00").unwrap()), "#ffaa00");
        assert_eq!(to_hex(parse_hex("f80").unwrap()), "#ff8800");
        for bad in ["", "#", "#12", "#12345", "#1234567", "#gg0000", "#ff00 00", "ｆｆ0000", "#ff00é0", "0x112233"] {
            assert!(parse_hex(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn recent_colours_keep_eight_most_recent_first_without_repeats() {
        let mut r = Vec::new();
        for i in 0..12u8 {
            push_recent(&mut r, from_srgb8([i * 20, 0, 0]));
        }
        assert_eq!(r.len(), MAX_RECENT);
        assert_eq!(to_srgb8(r[0]), [220, 0, 0]);
        assert_eq!(to_srgb8(r[7]), [80, 0, 0]);
        // Using an older one again moves it to the front instead of duplicating it.
        push_recent(&mut r, from_srgb8([100, 0, 0]));
        assert_eq!(r.len(), MAX_RECENT);
        assert_eq!(to_srgb8(r[0]), [100, 0, 0]);
        assert_eq!(r.iter().filter(|c| to_srgb8(**c) == [100, 0, 0]).count(), 1);
        // The same colour twice in a row changes nothing.
        let before = r.clone();
        push_recent(&mut r, from_srgb8([100, 0, 0]));
        assert_eq!(r, before);
        // Nothing breaks on NaN.
        push_recent(&mut r, [f32::NAN; 3]);
        assert_eq!(r[0], [0.0; 3]);
    }

    #[test]
    fn moving_swatches_clamps() {
        let mut l: Vec<Swatch> = (0..4).map(|i| Swatch::new(&i.to_string(), [0.0; 3])).collect();
        assert!(move_swatch(&mut l, 0, 99));
        assert_eq!(l.iter().map(|s| s.name.as_str()).collect::<String>(), "1230");
        assert!(!move_swatch(&mut l, 9, 0));
        assert!(!move_swatch(&mut l, 2, 2));
        assert!(!move_swatch(&mut Vec::new(), 0, 0));
    }

    #[test]
    fn gpl_round_trips() {
        let list = vec![Swatch::new("Sky", from_srgb8([10, 20, 250])), Swatch::new("", from_srgb8([255, 255, 0]))];
        let text = write_gpl("My colours", &list);
        let (name, back, skipped) = parse_gpl(&text).unwrap();
        assert_eq!((name.as_str(), skipped), ("My colours", 0));
        assert_eq!(back[0].name, "Sky");
        assert_eq!(to_srgb8(back[0].rgb), [10, 20, 250]);
        assert_eq!(back[1].name, "#ffff00", "an unnamed swatch is written with its code");
        assert_eq!(to_srgb8(back[1].rgb), [255, 255, 0]);
    }

    #[test]
    fn gpl_reads_real_files() {
        let text = "\u{feff}GIMP Palette\r\nName: Warm\r\nColumns: 4\r\n# a comment\r\n255   0   0\tRed Brick\r\n  0 128 0 \r\n";
        let (name, l, skipped) = parse_gpl(text).unwrap();
        assert_eq!((name.as_str(), l.len(), skipped), ("Warm", 2, 0));
        assert_eq!(l[0].name, "Red Brick");
        assert_eq!(l[1].name, "");
    }

    #[test]
    fn gpl_hostile_input() {
        assert!(parse_gpl("").is_err());
        assert!(parse_gpl("PNG\u{0}\u{1}binary").is_err());
        assert!(
            parse_gpl("GIMP Palette\nName: x\n1 2\n300 0 0 big\n-1 0 0\n1.5 0 0\nabc def ghi\n0 0 0 ok\n").map(|(_, l, s)| (l.len(), s)).unwrap() == (1, 5)
        );
        // Names are cleaned, and a huge name is cut.
        let long = "x".repeat(10_000);
        let (_, l, _) = parse_gpl(&format!("GIMP Palette\n1 2 3 {long}\n")).unwrap();
        assert_eq!(l[0].name.chars().count(), MAX_NAME_CHARS);
        // Too many colours: the rest is skipped, not allocated.
        let many = "0 0 0 c\n".repeat(MAX_SWATCHES + 50);
        let (_, l, skipped) = parse_gpl(&format!("GIMP Palette\n{many}")).unwrap();
        assert_eq!((l.len(), skipped), (MAX_SWATCHES, 50));
        // Too large a file is refused.
        assert!(parse_gpl(&format!("GIMP Palette\n{}", "#".repeat(MAX_PALETTE_BYTES))).is_err());
        // A name can never carry a second line into a written file.
        let out = write_gpl("a\nb", &[Swatch { name: "x\n255 0 0 evil".into(), rgb: [f32::NAN; 3] }]);
        assert_eq!(out.lines().count(), 5, "{out}");
        let (_, back, _) = parse_gpl(&out).unwrap();
        assert_eq!(back.len(), 1);
    }

    #[test]
    fn hex_list_round_trips_and_survives_junk() {
        let list = vec![Swatch::new("Moss", from_srgb8([1, 2, 3])), Swatch::new("", from_srgb8([250, 251, 252]))];
        let text = write_hex_list(&list);
        assert_eq!(text, "#010203 Moss\n#fafbfc\n");
        let (back, skipped) = parse_hex_list(&text).unwrap();
        assert_eq!((back.len(), skipped), (2, 0));
        assert_eq!(back[0].name, "Moss");
        let (l, skipped) = parse_hex_list("// c\n; c\n\n#fff White\nnope\n#12\n\u{0}\u{1}\n").unwrap();
        assert_eq!((l.len(), skipped), (1, 3));
        let many = "#000000\n".repeat(MAX_SWATCHES + 7);
        let (l, skipped) = parse_hex_list(&many).unwrap();
        assert_eq!((l.len(), skipped), (MAX_SWATCHES, 7));
        assert!(parse_hex_list(&"a".repeat(MAX_PALETTE_BYTES + 1)).is_err());
    }
}
