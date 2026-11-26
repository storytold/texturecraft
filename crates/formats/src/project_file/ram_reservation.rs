use std::borrow::Borrow;
use std::io::Cursor;

#[cfg(not(target_arch = "wasm32"))]
use super::SaveProgress;
use super::{ProjectFile, allowance::Allowance, ram_operation::RamOperation, write_project};
use crate::{FormatError, Result};

/// Shares the Settings allowance between linked cooks and project save workers.
#[derive(Debug)]
pub struct RamReservation {
    pub(super) operation: RamOperation,
    pub(super) allowance: Allowance,
}

impl RamReservation {
    /// Reserve the allowance through cooking, decoding and result delivery.
    pub fn for_cook() -> Result<Self> {
        Allowance::Global.for_cook()
    }

    /// Serialize saves while giving the UI backpressure if a cook owns the allowance.
    pub fn for_save() -> Result<Self> {
        Allowance::Global.for_save()
    }

    /// Reserve an autosave before cloning its snapshot without blocking the UI.
    pub fn try_for_save() -> Option<Self> {
        Allowance::Global.try_for_save()
    }

    pub fn save_bytes(&mut self, project_file: &ProjectFile<impl Borrow<Vec<u8>>, impl Borrow<Vec<u8>>>) -> Result<Vec<u8>> {
        self.check_save()?;
        let mut out = Cursor::new(Vec::new());
        write_project(&mut out, project_file, None)?;
        Ok(out.into_inner())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&mut self, path: &std::path::Path, project_file: &ProjectFile<impl Borrow<Vec<u8>>, impl Borrow<Vec<u8>>>) -> Result<()> {
        self.check_save()?;
        super::write_project_atomic(path, project_file, None)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn save_with_progress(
        &mut self,
        path: &std::path::Path,
        project_file: &ProjectFile<impl Borrow<Vec<u8>>, impl Borrow<Vec<u8>>>,
        progress: &SaveProgress,
    ) -> Result<()> {
        self.check_save()?;
        super::write_project_atomic(path, project_file, Some(progress))
    }

    fn check_save(&self) -> Result<()> {
        if self.operation == RamOperation::Save {
            return Ok(());
        }

        Err(FormatError::Invalid("saving requires a save reservation".into()))
    }
}

impl Drop for RamReservation {
    fn drop(&mut self) {
        let state = self.allowance.state();
        *state.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        state.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::RamAllowance;

    #[test]
    fn custom_allowances_do_not_block_each_other() {
        let a = Arc::new(RamAllowance::new());
        let b = Arc::new(RamAllowance::new());
        let cook = a.for_cook().unwrap();

        assert!(a.for_save().is_err());
        assert!(b.for_save().is_ok());

        drop(cook);
        assert!(a.for_save().is_ok());
    }
}
