use std::sync::Arc;

use super::{CookReservation, CookedMap, MaterialInterface};

#[derive(Clone, Debug)]
pub struct CookedMaterial {
    pub interface: MaterialInterface,
    pub maps: Vec<CookedMap>,
    pub size: u32,
    // A queued result still owns its large buffers
    _reservation: Option<Arc<CookReservation>>,
}

impl CookedMaterial {
    /// Assemble decoded maps without holding a CLI worker slot.
    pub fn new(interface: MaterialInterface, maps: Vec<CookedMap>, size: u32) -> Self {
        Self { interface, maps, size, _reservation: None }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn with_reservation(mut self, reservation: CookReservation) -> Self {
        self._reservation = Some(Arc::new(reservation));
        self
    }
}
