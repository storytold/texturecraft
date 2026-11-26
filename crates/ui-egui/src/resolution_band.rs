//! The Resolution of a cooked source, Relative (follows the texture set) or Fixed, each size pictured against the texture set's: a material's band, a procedural slot's row

use egui::{CornerRadius, Painter, Rect, Stroke, StrokeKind, vec2};
use texturecraft_engine::core::{CookResolution, MAX_RESOLUTION};

use crate::i18n::tr;
use crate::property_sheet::PropertySheet;
use crate::theme::Tokens;

/// Side of the grey square (this source) for factors 1/8 … ×8, as a share of the picture's side
const RATIO_SIDES: [f32; 7] = [0.12, 0.22, 0.32, 0.46, 0.66, 0.84, 1.0];

/// Side of the accent outline (the texture set) as a share of the picture's side
const TEXTURE_SET_SIDE: f32 = 0.42;

/// The smallest fixed size the list offers
const SMALLEST_FIXED: u32 = 256;

/// The Relative side of the Relative | Fixed switch
const RELATIVE_MODE: usize = 0;

/// The Fixed side of the Relative | Fixed switch
const FIXED_MODE: usize = 1;

/// The Resolution band (folded: picture and size); returns the resolution picked
pub(crate) fn resolution_band(sheet: &mut PropertySheet<'_>, current: CookResolution, texture_set: u32) -> Option<CookResolution> {
    let tokens = Tokens::get(sheet.ui.ctx());
    let size = current.size(texture_set);
    let relative = matches!(current, CookResolution::Relative(_));
    let picture = |painter: &Painter, rect: Rect| ratio_picture(painter, rect, factor_of(size, texture_set), relative, &tokens);
    if !sheet.band_pictured(tr("Resolution"), false, &format!("{size} × {size}"), &picture).open {
        return None;
    }

    let modes = [tr("Relative"), tr("Fixed")];
    let mode_picture = |painter: &Painter, rect: Rect, index: usize| ratio_picture(painter, rect, 0, index == RELATIVE_MODE, &tokens);
    let switched = sheet.picture_switch(&modes, if relative { RELATIVE_MODE } else { FIXED_MODE }, &mode_picture).map(|mode| {
        if mode == RELATIVE_MODE {
            // Fixed → Relative keeps the nearest factor the texture set can cook at
            let factor = factor_of(size, texture_set);
            if fits(texture_set, factor) { CookResolution::Relative(factor) } else { CookResolution::TEXTURE_SET }
        } else {
            CookResolution::Fixed(size)
        }
    });

    let picked = if let CookResolution::Relative(factor) = current {
        let factors = || fitting_factors(texture_set);
        let name = |factor: i8| format!("{}  ·  {}", factor_name(factor), CookResolution::Relative(factor).size(texture_set));
        let draw = |painter: &Painter, rect: Rect, factor: i8| ratio_picture(painter, rect, factor, true, &tokens);
        sheet.dropdown_built(tr("Size"), factor, factors, name, Some(&draw)).0.map(CookResolution::Relative)
    } else {
        let draw = |painter: &Painter, rect: Rect, size: u32| ratio_picture(painter, rect, factor_of(size, texture_set), false, &tokens);
        sheet.dropdown_built(tr("Size"), size, fixed_sizes, |size| format!("{size} × {size}"), Some(&draw)).0.map(CookResolution::Fixed)
    };
    switched.or(picked)
}

/// The Resolution row of a procedural slot's card: one list with the texture set's factors under Relative and fixed sizes under Fixed; returns the resolution picked
pub(crate) fn resolution_row(sheet: &mut PropertySheet<'_>, current: CookResolution, texture_set: u32) -> Option<CookResolution> {
    let tokens = Tokens::get(sheet.ui.ctx());
    let name = |resolution: CookResolution| match resolution {
        CookResolution::Relative(factor) => format!("{}  ·  {}", factor_name(factor), resolution.size(texture_set)),
        CookResolution::Fixed(size) => format!("{size} × {size}"),
    };
    let draw = |painter: &Painter, rect: Rect, resolution: CookResolution| {
        let related = matches!(resolution, CookResolution::Relative(_));
        ratio_picture(painter, rect, factor_of(resolution.size(texture_set), texture_set), related, &tokens);
    };
    let groups = || {
        let relative: Vec<CookResolution> = fitting_factors(texture_set).map(CookResolution::Relative).collect();
        let fixed: Vec<CookResolution> = fixed_sizes().map(CookResolution::Fixed).collect();
        [(tr("Relative"), relative), (tr("Fixed"), fixed)]
    };
    sheet.grouped_dropdown(tr("Resolution"), current, groups, name, &draw)
}

/// The factors the texture set can cook at, smallest first
fn fitting_factors(texture_set: u32) -> impl Iterator<Item = i8> {
    CookResolution::FACTORS.filter(move |factor| fits(texture_set, *factor))
}

/// The sizes a fixed resolution offers, smallest first
fn fixed_sizes() -> impl Iterator<Item = u32> {
    std::iter::successors(Some(SMALLEST_FIXED), |size| size.checked_mul(2)).take_while(|size| *size <= MAX_RESOLUTION)
}

/// Whether the texture set's size times 2^`factor` can be cooked
fn fits(texture_set: u32, factor: i8) -> bool {
    let shift = u32::from(factor.unsigned_abs());
    let size = if factor >= 0 { 1u32.checked_shl(shift).and_then(|scale| texture_set.checked_mul(scale)) } else { texture_set.checked_shr(shift) };
    size.is_some_and(CookResolution::cookable)
}

/// The factor (a power of two, within the list's) from the texture set's size to `size`
fn factor_of(size: u32, texture_set: u32) -> i8 {
    let steps = i64::from(size.max(1).ilog2()) - i64::from(texture_set.max(1).ilog2());
    let (low, high) = (CookResolution::FACTORS.start(), CookResolution::FACTORS.end());
    i8::try_from(steps.clamp(i64::from(*low), i64::from(*high))).unwrap_or_default()
}

/// A factor's name in the list: 1/8 … Texture set … ×8 (a factor outside the list, from a loaded project, is named as its nearest end)
fn factor_name(factor: i8) -> String {
    match factor.clamp(*CookResolution::FACTORS.start(), *CookResolution::FACTORS.end()) {
        0 => tr("Texture set").to_owned(),
        negative if negative < 0 => format!("1/{}", 1u32 << negative.unsigned_abs()),
        positive => format!("×{}", 1u32 << positive),
    }
}

/// This source (grey square) against the texture set (accent outline, when the size follows it)
fn ratio_picture(painter: &Painter, rect: Rect, factor: i8, related: bool, tokens: &Tokens) {
    painter.rect_filled(rect, CornerRadius::ZERO, tokens.field);
    let step = usize::try_from(i32::from(factor) - i32::from(*CookResolution::FACTORS.start())).unwrap_or_default();
    let side = RATIO_SIDES.get(step).copied().unwrap_or(1.0) * rect.width();
    painter.rect_filled(Rect::from_center_size(rect.center(), vec2(side, side)), CornerRadius::ZERO, tokens.text_dim);
    if related {
        let side = TEXTURE_SET_SIDE * rect.width();
        painter.rect_stroke(Rect::from_center_size(rect.center(), vec2(side, side)), CornerRadius::ZERO, Stroke::new(1.0, tokens.accent), StrokeKind::Inside);
    }
}

#[cfg(test)]
mod tests {
    use super::{factor_name, factor_of, fits};

    #[test]
    fn factors_name_and_fit_the_texture_set() {
        assert_eq!([-3, -1, 1, 3].map(factor_name), ["1/8", "1/2", "×2", "×8"]);
        assert_eq!((factor_of(1024, 2048), factor_of(8192, 128), factor_of(0, 2048)), (-1, 3, -3));
        assert!(fits(2048, 2) && !fits(2048, 3) && !fits(256, -2));
    }

    #[test]
    fn a_factor_outside_the_list_is_named_as_its_nearest_end() {
        assert_eq!([i8::MIN, i8::MAX].map(factor_name), ["1/8", "×8"]);
    }
}
