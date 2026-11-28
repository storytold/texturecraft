use std::collections::BTreeSet;
use std::sync::Arc;

use texturecraft_compose::{BakedMaps, ImageRes, ImageStore};
use texturecraft_core::LayerId;
use texturecraft_geom::{Mesh, UvGBuffer};

use super::micro_capture::MicroCapture;
use super::micro_request::MicroRequest;
use super::micro_slot::MicroSlot;

/// The micro details a program's generators read, and the micro curvatures their detail sizes blur
#[derive(Debug, Default)]
pub(crate) struct MicroPlan {
    pub(super) requests: Vec<MicroRequest>,
    /// The requests whose micro curvature a generator's detail sizes read
    pub(super) level_sets: BTreeSet<usize>,
    /// Each request's height and normal image, once the program's images are set
    pub(super) images: Vec<[Option<Arc<ImageRes>>; 2]>,
    /// Side-table words (indices into the table after its lookup words) that hold a generator
    /// source's local index plus one, made absolute by [`bind_sources`]
    pub(super) source_words: Vec<usize>,
    /// The programs that composite the anchor heights the micro details read
    pub(super) captures: Vec<MicroCapture>,
    /// This program is itself a capture program (`composite_capture` in WGSL)
    pub(super) capture: bool,
}

impl MicroPlan {
    /// Whether any generator reads micro maps
    pub(crate) fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Whether the micro details read anchor heights its capture programs composite
    pub(crate) fn captures_anchors(&self) -> bool {
        !self.captures.is_empty()
    }

    /// Whether the program is a capture program, whose Height display keeps its f32 value
    pub(crate) fn is_capture(&self) -> bool {
        self.capture
    }

    /// The anchor points whose height a micro details slot reads
    pub(super) fn anchors(&self) -> BTreeSet<LayerId> {
        self.requests
            .iter()
            .filter_map(|request| match request.height {
                Some(MicroSlot::Anchor(anchor)) => Some(anchor),
                _ => None,
            })
            .collect()
    }

    /// The capture programs' surface and bake inputs, as the program's own
    pub(crate) fn set_surface_inputs(&mut self, mesh: &Arc<Mesh>, gbuf: &Arc<UvGBuffer>, baked: Option<&BakedMaps>) -> Result<(), &'static str> {
        self.captures.iter_mut().try_for_each(|capture| capture.program.set_surface_inputs(mesh, gbuf, baked))
    }

    /// Resolve the requests' images (an image not loaded reads black, as the CPU's slot does), and
    /// the capture programs' images
    pub(crate) fn set_images(&mut self, images: &ImageStore) {
        for capture in &mut self.captures {
            capture.program.set_tile_images(images);
        }
        let image = |slot: &Option<MicroSlot>| match slot {
            Some(MicroSlot::Image { name, .. }) => images.get(name).cloned(),
            _ => None,
        };
        self.images = self.requests.iter().map(|request| [image(&request.height), image(&request.normal)]).collect();
    }
}
