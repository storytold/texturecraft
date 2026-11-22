//! Optional host accelerators for whole-image graph operations.

use std::{any::Any, fmt::Debug, sync::Arc};

use crate::context::Cancel;
use crate::func::{FnEnv, PixelShader, Prog};
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

/// Packed, quantized grayscale pixels owned by an accelerator.
pub trait ResidentGreyPixels: Debug + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn read(&self, cancel: &Cancel) -> Result<Vec<u16>, String>;
}

#[derive(Debug)]
pub struct ResidentGrey16 {
    pub w: usize,
    pub h: usize,
    pub div: f32,
    pub alpha: f32,
    pub pixels: Arc<dyn ResidentGreyPixels>,
}

#[derive(Clone, Copy)]
pub enum GreyInput<'a> {
    Host(&'a Grey16),
    Device(&'a ResidentGrey16),
}

impl GreyInput<'_> {
    pub fn info(self) -> (usize, usize, f32, f32) {
        match self {
            Self::Host(t) => (t.w, t.h, t.div, t.alpha),
            Self::Device(t) => (t.w, t.h, t.div, t.alpha),
        }
    }
}

pub enum ResidentGreyPass<'a> {
    Enlarge { source: GreyInput<'a>, side: usize },
    Blend { foreground: GreyInput<'a>, background: GreyInput<'a>, mode: u32, opacity: f32 },
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

    /// Execute a grayscale operation without fetching its result back to the host.
    fn resident_grey(&self, _pass: ResidentGreyPass<'_>, _cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        Ok(None)
    }

    /// Execute a grayscale pixel program whose image inputs and output use packed integer samples.
    fn resident_pixel(&self, _shader: PixelShader, _images: &[GreyInput<'_>], _size: [usize; 2], _cancel: &Cancel) -> Result<Option<ResidentGrey16>, String> {
        Ok(None)
    }
}
