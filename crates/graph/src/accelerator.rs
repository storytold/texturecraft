//! Optional host accelerators for whole-image graph operations.

use crate::context::Cancel;
use crate::func::{FnEnv, Prog};
use crate::tex::{Grey, Grey16, Tex};

pub struct PixelRequest<'a> {
    pub program: &'a Prog,
    pub environment: &'a FnEnv<'a>,
    pub size: [usize; 2],
    pub cancel: &'a Cancel,
}

#[derive(Clone)]
pub enum PixelOutput {
    Grey(Grey),
    Rgba(Tex),
}

/// Whole-image operations on the compiled graph's 16-bit grayscale values.
pub enum GreyPass<'a> {
    /// A wrapped bilinear identity transform enlarging through successive doublings.
    Enlarge { source: &'a Grey16, side: usize },
    /// A full-frame unmasked blend that keeps the background alpha.
    Blend { foreground: &'a Grey16, background: &'a Grey16, mode: u32, opacity: f32 },
}

pub trait Accelerator: Send + Sync {
    /// Identifies changes to rendered pixels for the host's result caches.
    fn cache_tag(&self) -> &str;

    /// `None` leaves this operation to the reference evaluator.
    fn pixel(&self, _request: PixelRequest<'_>) -> Result<Option<PixelOutput>, String> {
        Ok(None)
    }

    /// Draw the ordered instances of a procedural pattern program.
    fn pattern(&self, _request: crate::patterns::RasterRequest<'_>) -> Result<Option<crate::patterns::RasterOutput>, String> {
        Ok(None)
    }

    fn grey_pass(&self, _pass: GreyPass<'_>, _cancel: &Cancel) -> Result<Option<Grey16>, String> {
        Ok(None)
    }
}
