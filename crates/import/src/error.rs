//! [`ImportError`]: why a whole import failed, and [`PartError`]: why one part (an image, a cache entry) could not be
//! read; parts that fail are reported and left out, they never fail the import.

use std::fmt;

use texturecraft_import_reader::decode::DecodeError;
use texturecraft_import_reader::project::ProjectError;

#[derive(Debug)]
pub enum ImportError {
    /// The caller asked to stop.
    Cancelled,
    /// The file is not a project or its container cannot be read.
    Project(ProjectError),
    /// A part the import cannot do without (the mesh) could not be decoded.
    Decode { part: &'static str, source: DecodeError },
    /// The file reads but holds nothing to import (no mesh, no texture set).
    Invalid(String),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ImportError::Cancelled => f.write_str("import cancelled"),
            ImportError::Project(e) => write!(f, "cannot read the project: {e}"),
            ImportError::Decode { part, source } => write!(f, "cannot read the {part}: {source}"),
            ImportError::Invalid(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<ProjectError> for ImportError {
    fn from(e: ProjectError) -> Self {
        ImportError::Project(e)
    }
}

/// Why one part of the file could not be read.
#[derive(Debug)]
pub enum PartError {
    Missing(String),
    Project(ProjectError),
    Decode(DecodeError),
    UnknownFormat(String),
    Image(String),
}

impl fmt::Display for PartError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PartError::Missing(what) => write!(f, "{what} is not stored in the file"),
            PartError::Project(e) => write!(f, "{e}"),
            PartError::Decode(e) => write!(f, "{e}"),
            PartError::UnknownFormat(name) => write!(f, "unknown pixel format {name}"),
            PartError::Image(why) => f.write_str(why),
        }
    }
}

impl From<ProjectError> for PartError {
    fn from(e: ProjectError) -> Self {
        PartError::Project(e)
    }
}

impl From<DecodeError> for PartError {
    fn from(e: DecodeError) -> Self {
        PartError::Decode(e)
    }
}

impl From<crate::image::ImageError> for PartError {
    fn from(e: crate::image::ImageError) -> Self {
        PartError::Image(e.0)
    }
}
