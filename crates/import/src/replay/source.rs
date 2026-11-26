//! What a brush paints when its colour source is not a uniform: an image or a compiled graph ([`ReplaySource`]).
//!
//! - A **bitmap** source is laid on the texture's UV space once, unscaled and repeating, converted as a bitmap fill
//!   converts its image (sRGB integer images made linear in colour channels, a one-value channel reads the grey, height
//!   signed); its alpha scales the coverage. The reference app's placement of a brush's image is not measured: it is the
//!   placement a brush's graph gets too, and the one a bitmap fill gets without a transform.
//! - A **graph** source runs the graph a fill of it runs ([`crate::graph_uses::fill_sources`] per channel, a mask its
//!   grey output), rendered at the texture's side before the strokes are laid ([`render`]); stroke by stroke its outputs
//!   are laid on texel for texel (its placement is not read; the coverage output is not used there).
//! - Along **path strokes and ribbons** either source is read at the texel's place on the band or in the tip's square of
//!   the last dab reaching it (a material laid along a path), with the source's coverage (a graph's coverage output, a
//!   bitmap's alpha). Measured replay rules of an earlier importer; the place itself is the brush's.

use std::sync::Arc;

use texturecraft_compose::graphs::{RenderEnv, at_texel, graph_to_channel, sample};
use texturecraft_compose::{Cooked, GraphStore, ImageRes};
use texturecraft_core::{ChannelKind, GraphRef, lum};

use super::ReplayJob;

/// What a brush paints per texel instead of a uniform value.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplaySource {
    /// A compiled graph as a fill of it runs, not rendered yet: the import renders it before the strokes are laid
    /// ([`render`]).
    Graph(Box<GraphRef>),
    /// Ready to sample.
    Texels(Arc<ReplayTexels>),
    /// A graph that did not render, and why: the job cannot be laid.
    Failed(String),
}

/// An image a brush paints, ready to sample as a channel holds it.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplayTexels {
    /// An image resource laid on the texture once, repeating (values in its file's encoding; `srgb` integer images).
    Image(Arc<ImageRes>),
    /// A graph's render covering the texture: the output painted and the output that is its coverage.
    Graph { cooked: Arc<Cooked>, output: Option<String>, alpha: Option<String> },
}

impl ReplayTexels {
    /// The value painted at texel `(x, y)` of a texture of `size`, as a channel of `kind` holds it, and the coverage
    /// factor there (a graph's is 1: laid texel for texel, its coverage output is not read).
    pub fn at_texel(&self, kind: ChannelKind, x: u32, y: u32, size: (u32, u32)) -> Option<([f32; 3], f32)> {
        match self {
            ReplayTexels::Image(image) => {
                let (w, h) = (size.0.max(1) as f32, size.1.max(1) as f32);
                Some(image_value(image, kind, image.sample((x as f32 + 0.5) / w, (y as f32 + 0.5) / h)))
            }
            ReplayTexels::Graph { cooked, output, .. } => {
                let v = at_texel(cooked.output(output.as_deref())?, x, y, size);
                Some((graph_to_channel(kind, v, cooked.is_linear(output.as_deref())), 1.0))
            }
        }
    }

    /// The value at place `uv` of the source's own image (0..1 repeating, rows down), as a channel of `kind` holds it,
    /// with the source's coverage there (a graph's coverage output, an image's alpha).
    pub fn at_uv(&self, kind: ChannelKind, uv: [f32; 2]) -> Option<([f32; 3], f32)> {
        match self {
            ReplayTexels::Image(image) => Some(image_value(image, kind, image.sample(uv[0], uv[1]))),
            ReplayTexels::Graph { cooked, output, alpha } => {
                let v = sample(cooked.output(output.as_deref())?, uv[0], uv[1]);
                let a = alpha.as_deref().and_then(|a| cooked.output(Some(a))).map_or(1.0, |img| sample(img, uv[0], uv[1])[0]);
                Some((graph_to_channel(kind, v, cooked.is_linear(output.as_deref())), if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 }))
            }
        }
    }
}

/// An image's sample as a channel of `kind` holds it (as a bitmap fill reads its image), with its alpha.
fn image_value(image: &ImageRes, kind: ChannelKind, s: [f32; 4]) -> ([f32; 3], f32) {
    let rgb = [s[0], s[1], s[2]];
    let v = match kind {
        ChannelKind::Color if image.srgb => rgb.map(texturecraft_texel::srgb_to_linear),
        ChannelKind::Color | ChannelKind::NormalMap | ChannelKind::Data => rgb,
        ChannelKind::Scalar => [lum(rgb); 3],
        ChannelKind::Signed => [lum(rgb) * 2.0 - 1.0; 3],
    };
    let a = if s[3].is_finite() { s[3].clamp(0.0, 1.0) } else { 0.0 };
    (v, a)
}

/// Renders every graph source of `jobs` with `store` in `env` (the texture set's baked maps and images, the texture's
/// side), in place: each becomes [`ReplaySource::Texels`], or [`ReplaySource::Failed`] with why it did not render.
pub fn render(jobs: &mut [ReplayJob], store: &GraphStore, env: &RenderEnv) {
    let cook = |source: &mut ReplaySource| {
        let ReplaySource::Graph(graph) = source else { return };
        *source = match store.cook(graph, env, &[]) {
            Ok((_, cooked)) => ReplaySource::Texels(Arc::new(ReplayTexels::Graph { cooked, output: graph.output.clone(), alpha: graph.alpha_output.clone() })),
            Err(why) => ReplaySource::Failed(format!("brush source \"{}\" did not render: {why}", graph.name)),
        };
    };
    for step in jobs.iter_mut().flat_map(|j| j.steps.iter_mut()) {
        for (_, source, _) in &mut step.sources {
            cook(source);
        }
        if let Some(source) = &mut step.mask_source {
            cook(source);
        }
    }
}

/// The graph texts every graph source of `jobs` runs, by key.
pub fn text_keys(jobs: &[ReplayJob]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let steps = jobs.iter().flat_map(|j| &j.steps);
    for source in steps.flat_map(|s| s.sources.iter().map(|(_, src, _)| src).chain(&s.mask_source)) {
        if let ReplaySource::Graph(g) = source {
            for k in g.text_keys() {
                if !out.iter().any(|o| o == k) {
                    out.push(k.to_string());
                }
            }
        }
    }
    out
}
