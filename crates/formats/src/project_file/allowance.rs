use std::sync::Arc;

use super::{RamAllowance, RamReservation, ram_operation::RamOperation};
use crate::{FormatError, Result};

static DEFAULT_ALLOWANCE: RamAllowance = RamAllowance::new();

#[derive(Clone, Debug)]
pub(super) enum Allowance {
    Global,
    Custom(Arc<RamAllowance>),
}

impl Allowance {
    pub(super) fn state(&self) -> &RamAllowance {
        match self {
            Allowance::Global => &DEFAULT_ALLOWANCE,
            Allowance::Custom(allowance) => allowance,
        }
    }

    pub(super) fn for_cook(self) -> Result<RamReservation> {
        let mut active = self.state().active.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let message = match *active {
            None => {
                *active = Some(RamOperation::Cook);
                drop(active);
                return Ok(RamReservation { operation: RamOperation::Cook, allowance: self });
            }
            Some(RamOperation::Cook) => "another MaterialCraft cook is still running",
            Some(RamOperation::Save) => "a project save is using the RAM allowance; retry the cook after saving",
        };

        Err(FormatError::Invalid(message.into()))
    }

    pub(super) fn for_save(self) -> Result<RamReservation> {
        let state = self.state();
        let mut active = state.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while *active == Some(RamOperation::Save) {
            active = state.released.wait(active).unwrap_or_else(std::sync::PoisonError::into_inner);
        }

        if active.is_some() {
            return Err(FormatError::Invalid("a MaterialCraft cook is using the RAM allowance; retry saving when it finishes".into()));
        }

        *active = Some(RamOperation::Save);
        drop(active);
        Ok(RamReservation { operation: RamOperation::Save, allowance: self })
    }

    pub(super) fn try_for_save(self) -> Option<RamReservation> {
        let mut active = self.state().active.try_lock().ok()?;
        if active.is_some() {
            return None;
        }

        *active = Some(RamOperation::Save);
        drop(active);
        Some(RamReservation { operation: RamOperation::Save, allowance: self })
    }
}
