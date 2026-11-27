//! Whether a resource still matches the file it was imported from

/// How a resource compares with its file on disk (materials have none). The order is the Resources Updater's
/// list order: what needs attention first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceStatus {
    /// The file changed since it was read: Update reads it again.
    Outdated,
    /// The file is gone, or cannot be read.
    Missing,
    /// The file still holds the resource's bytes.
    UpToDate,
    /// Made in the app or imported without a file path: there is nothing to compare with.
    NoSource,
    /// A linked MaterialCraft package stored in the project, without a source file to compare.
    Material,
}

impl ResourceStatus {
    pub const ALL: [ResourceStatus; 5] =
        [ResourceStatus::Outdated, ResourceStatus::Missing, ResourceStatus::UpToDate, ResourceStatus::NoSource, ResourceStatus::Material];

    /// The id used by the control protocol
    pub fn id(self) -> &'static str {
        match self {
            ResourceStatus::Outdated => "outdated",
            ResourceStatus::Missing => "missing",
            ResourceStatus::UpToDate => "upToDate",
            ResourceStatus::NoSource => "noSource",
            ResourceStatus::Material => "material",
        }
    }

    /// The name shown in the Resources Updater (English; the shell translates it)
    pub fn label(self) -> &'static str {
        match self {
            ResourceStatus::Outdated => "Outdated",
            ResourceStatus::Missing => "Missing",
            ResourceStatus::UpToDate => "Up to date",
            ResourceStatus::NoSource => "No source file",
            ResourceStatus::Material => "Material",
        }
    }
}
