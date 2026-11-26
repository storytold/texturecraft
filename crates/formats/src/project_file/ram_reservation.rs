use std::borrow::Borrow;
use std::io::Cursor;
use std::sync::{Condvar, Mutex};

use super::{ProjectFile, ram_operation::RamOperation, write_project};
use crate::{FormatError, Result};

static ACTIVE_OPERATION: Mutex<Option<RamOperation>> = Mutex::new(None);
static RELEASED: Condvar = Condvar::new();

/// Shares the Settings allowance between linked cooks and project save workers.
#[derive(Debug)]
pub struct RamReservation(RamOperation);

impl RamReservation {
    /// Reserve the allowance through cooking, decoding and result delivery.
    pub fn for_cook() -> Result<Self> {
        let mut active = ACTIVE_OPERATION.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let message = match *active {
            None => {
                *active = Some(RamOperation::Cook);
                return Ok(Self(RamOperation::Cook));
            }
            Some(RamOperation::Cook) => "another MaterialCraft cook is still running",
            Some(RamOperation::Save) => "a project save is using the RAM allowance; retry the cook after saving",
        };

        Err(FormatError::Invalid(message.into()))
    }

    /// Serialize saves while giving the UI backpressure if a cook owns the allowance.
    pub fn for_save() -> Result<Self> {
        let mut active = ACTIVE_OPERATION.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        while *active == Some(RamOperation::Save) {
            active = RELEASED.wait(active).unwrap_or_else(std::sync::PoisonError::into_inner);
        }

        if active.is_some() {
            return Err(FormatError::Invalid("a MaterialCraft cook is using the RAM allowance; retry saving when it finishes".into()));
        }

        *active = Some(RamOperation::Save);
        Ok(Self(RamOperation::Save))
    }

    /// Reserve an autosave before cloning its snapshot without blocking the UI.
    pub fn try_for_save() -> Option<Self> {
        let mut active = ACTIVE_OPERATION.try_lock().ok()?;
        if active.is_some() {
            return None;
        }

        *active = Some(RamOperation::Save);
        Some(Self(RamOperation::Save))
    }

    pub fn save_bytes(&mut self, project_file: &ProjectFile<impl Borrow<Vec<u8>>>) -> Result<Vec<u8>> {
        self.check_save()?;
        let mut out = Cursor::new(Vec::new());
        write_project(&mut out, project_file)?;
        Ok(out.into_inner())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&mut self, path: &std::path::Path, project_file: &ProjectFile<impl Borrow<Vec<u8>>>) -> Result<()> {
        self.check_save()?;
        super::write_project_atomic(path, project_file)
    }

    fn check_save(&self) -> Result<()> {
        if self.0 == RamOperation::Save {
            return Ok(());
        }

        Err(FormatError::Invalid("saving requires a save reservation".into()))
    }
}

impl Drop for RamReservation {
    fn drop(&mut self) {
        *ACTIVE_OPERATION.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        RELEASED.notify_all();
    }
}
