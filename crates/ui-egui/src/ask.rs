use crate::list_action::ListAction;

/// A question waiting for an answer in the Output templates tab
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    /// Delete this template (its file goes with it)
    Delete(String),
    /// Leave the selected template, whose last edit does not validate and is not saved, for this
    Leave(ListAction),
}
