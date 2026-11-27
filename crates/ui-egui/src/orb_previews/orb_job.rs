use std::hash::{DefaultHasher, Hash, Hasher};

#[cfg(not(target_arch = "wasm32"))]
use egui::Id;
use egui::{Color32, ColorImage};

use super::{QUICK_BELOW_PX, QUICK_DIVISOR};
use crate::material_orb::render;
use crate::orb_material::OrbMaterial;

/// One orb to render
#[derive(Clone, Debug)]
pub(crate) struct OrbJob {
    pub(super) key: u64,
    pub(super) request: u64,
    pub(super) material: OrbMaterial,
    pub(super) side: usize,
    pub(super) backdrop: Color32,
    /// Where it is drawn: a newer job for the same place replaces an older one still waiting (the web renders each job at once)
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) place: Id,
    /// The quick picture is still to come
    pub(super) quick: bool,
}

impl OrbJob {
    pub(crate) fn new(material: OrbMaterial, side: usize, backdrop: Color32) -> OrbJob {
        OrbJob {
            request: 0,
            key: OrbJob::key_of(&material, side, backdrop),
            material,
            side,
            backdrop,
            #[cfg(not(target_arch = "wasm32"))]
            place: Id::NULL,
            quick: side >= QUICK_BELOW_PX,
        }
    }

    /// The cache key of `material`'s orb `side` pixels square on `backdrop`, without building the job
    pub(crate) fn key_of(material: &OrbMaterial, side: usize, backdrop: Color32) -> u64 {
        let mut hasher = DefaultHasher::new();
        (material.key(), side, backdrop.to_array()).hash(&mut hasher);
        hasher.finish()
    }

    pub(super) fn picture(&self) -> ColorImage {
        let side = if self.quick { self.side / QUICK_DIVISOR } else { self.side };
        render(&self.material, side, self.backdrop)
    }
}
