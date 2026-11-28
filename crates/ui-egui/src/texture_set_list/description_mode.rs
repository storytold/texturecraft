/// Which rows show their description line under the name (the list menu's choice)
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum DescriptionMode {
    #[default]
    HideEmpty,
    ShowAll,
    HideAll,
}

impl DescriptionMode {
    /// Whether a row with `description` shows the line
    pub(crate) fn shows(self, description: &str) -> bool {
        match self {
            Self::HideEmpty => !description.trim().is_empty(),
            Self::ShowAll => true,
            Self::HideAll => false,
        }
    }
}
