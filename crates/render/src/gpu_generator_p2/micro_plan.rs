use std::sync::Arc;

use texturecraft_compose::{ImageRes, ImageStore};

use super::micro_request::MicroRequest;
use super::micro_slot::MicroSlot;

/// The micro details a program's generators read, and the curvature its detail sizes blur
#[derive(Debug, Default)]
pub(crate) struct MicroPlan {
    pub(super) requests: Vec<MicroRequest>,
    /// The curvature every detail size reader reads: None unclaimed, Some(None) the bake's,
    /// Some(Some(i)) the bake with request `i`'s micro curvature
    pub(super) levels: Option<Option<usize>>,
    /// Each request's height and normal image, once the program's images are set
    pub(super) images: Vec<[Option<Arc<ImageRes>>; 2]>,
    /// Side-table words (indices into the table after its lookup words) that hold a generator
    /// source's local index plus one, made absolute by [`bind_sources`]
    pub(super) source_words: Vec<usize>,
}

impl MicroPlan {
    /// Whether any generator reads micro maps
    pub(crate) fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    /// Detail sizes read from `source` (None: the bake's own curvature, else a request's micro
    /// curvature): None when another reader already took another curvature, whose sizes the one
    /// curvature levels binding cannot hold at once (that program composes on the CPU)
    pub(crate) fn claim_levels(&mut self, source: Option<usize>) -> Option<()> {
        match self.levels {
            None => {
                self.levels = Some(source);
                Some(())
            }
            Some(claimed) => (claimed == source).then_some(()),
        }
    }

    /// Resolve the requests' images (an image not loaded reads black, as the CPU's slot does)
    pub(crate) fn set_images(&mut self, images: &ImageStore) {
        let image = |slot: &Option<MicroSlot>| match slot {
            Some(MicroSlot::Image { name, .. }) => images.get(name).cloned(),
            _ => None,
        };
        self.images = self.requests.iter().map(|request| [image(&request.height), image(&request.normal)]).collect();
    }
}
