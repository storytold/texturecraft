use crate::i18n::n_;

/// How a project manages colour. Only Legacy is implemented; OpenColorIO can be chosen to see its
/// settings, but they stay disabled and a project is always created and kept as Legacy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorManagement {
    #[default]
    Legacy,
    OpenColorIo,
}

impl ColorManagement {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ColorManagement::Legacy => n_("Legacy"),
            ColorManagement::OpenColorIo => "OpenColorIO",
        }
    }
}
