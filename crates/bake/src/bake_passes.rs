use crate::cage::RayFrame;
use crate::map::BakeContext;
use crate::tracing::{CpuTracer, Tracer};
use crate::{BakeError, BakeOutput, BakeRequest, MeshExtent, Progress};
use std::sync::atomic::Ordering::Relaxed;
use texturecraft_core::{BakeSettings, BakedMap};

/// Map passes over one immutable request share the fitted cage and prepared skew.
pub struct BakePasses<'a> {
    request: BakeRequest<'a>,
    frame: Option<(MeshExtent, RayFrame<'a>)>,
}

impl<'a> BakePasses<'a> {
    pub fn new(request: BakeRequest<'a>) -> Self {
        Self { request, frame: None }
    }

    /// Bake one selected map, including the CPU fallback used by a normal bake.
    pub fn bake_map(
        &mut self,
        map: BakedMap,
        progress: &Progress,
        excluded_tiles: &[(u32, u32)],
        tracer: Option<&dyn Tracer>,
    ) -> Result<BakeOutput, BakeError> {
        let mut settings = self.request.settings.clone();
        settings.maps.select([map]);
        self.run(&settings, progress, excluded_tiles, tracer, super::STRIP_TEXELS)
    }

    pub(super) fn run(
        &mut self,
        settings: &BakeSettings,
        progress: &Progress,
        excluded_tiles: &[(u32, u32)],
        tracer: Option<&dyn Tracer>,
        strip_texels: usize,
    ) -> Result<BakeOutput, BakeError> {
        if progress.cancel.load(Relaxed) {
            return Err(BakeError::Cancelled);
        }

        let request = BakeRequest { settings, ..self.request };
        super::validate(&request)?;
        if excluded_tiles.iter().any(|(column, row)| *column >= request.tiles.0 || *row >= request.tiles.1) {
            return Err(BakeError::Invalid("excluded UV tile lies outside the bake atlas".into()));
        }

        // Validate each pass separately so earlier maps can finish before a later map fails
        let (extent, frame) = match &mut self.frame {
            Some(geometry) => geometry,
            None => {
                let extent = MeshExtent::of(request.mesh)?;
                self.frame.insert((extent, RayFrame::new(&self.request, extent)?))
            }
        };
        let context = BakeContext::with_frame(&request, *extent, frame);
        let bake = |tracer: &dyn Tracer| super::bake_prepared(&request, &context, progress, strip_texels, tracer, excluded_tiles);
        match tracer {
            Some(tracer) => match bake(tracer) {
                Err(BakeError::Tracer(error)) => {
                    let mut output = bake(&CpuTracer)?;
                    output.tracer = format!("CPU ({} unavailable: {error})", tracer.name());
                    Ok(output)
                }
                result => result,
            },
            None => bake(&CpuTracer),
        }
    }
}
