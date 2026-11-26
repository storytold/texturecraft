use crate::session::Session;

/// An archive that was written successfully. Only the same project can adopt its path.
pub struct SaveOutput {
    pub path: String,
    pub(super) project_id: u64,
    pub(super) saved_revision: Option<u64>,
    pub(super) adopt: bool,
}

impl SaveOutput {
    /// Apply the saved path without clearing edits made since the snapshot.
    pub fn apply(&self, session: &mut Session) {
        if session.project_id != self.project_id {
            return;
        }

        if self.adopt {
            session.path = Some(self.path.clone());

            // A stroke started after capture changes the revision only when committed
            if self.saved_revision == Some(session.save_revision) {
                session.modified = false;
            }
        }

        session.log.info("Project", format!("Saved {}", self.path));
    }
}
