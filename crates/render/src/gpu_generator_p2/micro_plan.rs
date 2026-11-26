use std::collections::BTreeSet;
use std::sync::Arc;

use texturecraft_compose::{ImageRes, ImageStore};

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
}

impl MicroPlan {
    /// Whether any generator reads micro maps
    pub(crate) fn is_empty(&self) -> bool {
        self.requests.is_empty()
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
