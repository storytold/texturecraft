//! What each library filter does, in one sentence, and the words it is found by: the Attributes of its properties

/// A library filter's description and tags
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilterAbout {
    pub description: &'static str,
    pub tags: &'static [&'static str],
}

const fn about(description: &'static str, tags: &'static [&'static str]) -> FilterAbout {
    FilterAbout { description, tags }
}

/// One entry per library filter id, found by its id
const ABOUT: &[(&str, FilterAbout)] = &[
    ("hsl", about("Shifts the hue and changes the saturation and lightness of the colors.", &["color", "hue", "saturation", "lightness"])),
    ("levels", about("Remaps the values between new black and white points, with a midtone gamma.", &["levels", "contrast", "remap"])),
    ("contrastLuminosity", about("Raises or lowers the contrast and the overall brightness.", &["contrast", "brightness"])),
    ("colorBalance", about("Tints the shadows, midtones and highlights each towards a color.", &["color", "tint", "balance"])),
    (
        "colorCorrect",
        about("Corrects the contrast, luminosity and saturation of the shadows, midtones and highlights.", &["color", "correction", "saturation"]),
    ),
    ("gradientMap", about("Replaces each value by the color at its place on a gradient.", &["gradient", "color", "remap"])),
    ("gradientCurve", about("Bends the values through a curve preset.", &["gradient", "curve", "remap"])),
    ("gradientDynamic", about("Reads the colors along a gradient source at each value's position.", &["gradient", "color", "dynamic"])),
    ("posterize", about("Rounds the values to a few steps.", &["steps", "quantize", "flat"])),
    ("clamp", about("Keeps the values between a minimum and a maximum.", &["clamp", "range", "limit"])),
    ("smoothstep", about("Eases the values between a minimum and a maximum from black to white.", &["smoothstep", "range", "ease"])),
    ("heightAdjust", about("Flips, scales and raises heights around mid gray.", &["height", "offset", "multiply"])),
    ("histogramScan", about("Turns the values into a sharp or soft cut at a chosen position.", &["histogram", "contrast", "cut"])),
    ("histogramRange", about("Squeezes the values into a narrower range around a position.", &["histogram", "range"])),
    ("histogramShift", about("Slides the values along and wraps them around.", &["histogram", "shift", "wrap"])),
    ("invert", about("Turns dark into light and light into dark.", &["invert", "negative"])),
    ("threshold", about("Makes every value white past the threshold and black below it.", &["threshold", "black and white", "cut"])),
    ("grayscale", about("Turns the colors into grays.", &["grayscale", "desaturate"])),
    ("edgeDetect", about("Draws lines where the values change sharply.", &["edges", "outline", "lines"])),
    ("normalInvert", about("Flips the green component of a normal map.", &["normal", "invert", "green"])),
    ("bakedLighting", about("Paints sun, sky and light shading into the base color from the baked mesh maps.", &["lighting", "stylized", "shading"])),
    ("bakedLightingEnvironment", about("Lights the base color with the viewport environment.", &["lighting", "environment", "shading"])),
    ("replaceColor", about("Swaps one color for another, keeping the shading.", &["color", "replace", "swap"])),
    ("blur", about("Softens the input: each texel spreads into its neighbours.", &["blur", "soften"])),
    ("blurDirectional", about("Smears the input along one direction.", &["blur", "motion", "direction"])),
    ("blurSlope", about("Smears the values downhill along a noise height field.", &["blur", "slope", "flow"])),
    ("sharpen", about("Makes the details crisper.", &["sharpen", "detail"])),
    ("highPass", about("Keeps the fine details and drops the broad shapes.", &["detail", "high pass", "frequency"])),
    ("bevel", about("Slopes the shapes down to their edge, or raises a slope around them.", &["bevel", "edge", "slope"])),
    ("bevelSmooth", about("Draws a smooth ramp from the edge of the shapes.", &["bevel", "edge", "ramp"])),
    ("directionalDistance", about("Draws a trail fading away from the shapes towards an angle.", &["distance", "trail", "leak"])),
    ("glow", about("Lays a soft halo of color around the shapes.", &["glow", "halo", "light"])),
    ("dropShadow", about("Lays a blurred, offset shadow of the shapes under them.", &["shadow", "drop shadow"])),
    ("maskOutline", about("Draws a band along the edge of the shapes.", &["outline", "edge", "stroke"])),
    ("warp", about("Moves the texels by a noise or another source.", &["warp", "distort", "noise"])),
    ("mosaic", about("Breaks the input into square pixels, a set number across the UV tile.", &["pixelate", "mosaic", "cells"])),
    ("fxaa", about("Smooths stair-stepped edges by blending across them.", &["anti-aliasing", "edges", "smooth"])),
    ("anisotropicKuwahara", about("Smooths the input into painterly strokes that follow its direction.", &["painterly", "smooth", "stylize"])),
    ("pbrValidate", about("Shows in red the base colors outside the physically plausible ranges.", &["pbr", "validate", "albedo", "metal"])),
    ("colorMatch", about("Moves the colors so a source color, the texture's mean by default, lands on a target color.", &["color", "match", "recolor"])),
    ("heightToNormal", about("Turns the layer's height into its normal.", &["height", "normal", "convert"])),
    ("mirror", about("Copies one side of each UV tile over the other.", &["mirror", "symmetry", "flip"])),
    ("transform", about("Scales, turns, moves and flips the content of each UV tile.", &["transform", "scale", "rotate", "offset", "flip"])),
    ("matFinishGrind", about("Gives the material a ground finish: fine brushed lines in its roughness.", &["material", "metal", "scratches"])),
    ("matFinishRaw", about("Gives the material a raw finish: uneven, cloudy roughness.", &["material", "raw", "rough"])),
    ("matFinishSanded", about("Gives the material a sanded finish: a fine, even grain in its roughness.", &["material", "sanded", "rough"])),
    ("matFinishHammered", about("Gives the material a hammered finish: round dents in its roughness.", &["material", "metal", "hammered"])),
];

/// The description and tags of library filter `id` (its exact id)
pub fn of(id: &str) -> Option<FilterAbout> {
    ABOUT.iter().find(|(entry, _)| *entry == id).map(|(_, about)| *about)
}
