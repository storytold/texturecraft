use std::sync::{Arc, Condvar, Mutex};

use super::{RamReservation, allowance::Allowance, ram_operation::RamOperation};
use crate::Result;

/// Coordinates linked cooks and project saves that share one RAM allowance
#[derive(Debug, Default)]
pub struct RamAllowance {
    pub(super) active: Mutex<Option<RamOperation>>,
    pub(super) released: Condvar,
}

impl RamAllowance {
    pub const fn new() -> Self {
        Self { active: Mutex::new(None), released: Condvar::new() }
    }

    pub fn for_cook(self: &Arc<Self>) -> Result<RamReservation> {
        Allowance::Custom(Arc::clone(self)).for_cook()
    }

    pub fn for_save(self: &Arc<Self>) -> Result<RamReservation> {
        Allowance::Custom(Arc::clone(self)).for_save()
    }

    pub fn try_for_save(self: &Arc<Self>) -> Option<RamReservation> {
        Allowance::Custom(Arc::clone(self)).try_for_save()
    }
}
