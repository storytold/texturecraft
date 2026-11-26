//! The projections a fill's Projection dropdown offers

use texturecraft_engine::core::Projection;

use crate::i18n::n_;

/// The projections a fill's Projection dropdown offers, with their command ids
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionKind {
    UvTile,
    Uv,
    Triplanar,
    Planar,
    Spherical,
    Cylindrical,
    Warp,
}

impl ProjectionKind {
    pub(crate) const ALL: [ProjectionKind; 7] = [
        ProjectionKind::UvTile,
        ProjectionKind::Uv,
        ProjectionKind::Triplanar,
        ProjectionKind::Planar,
        ProjectionKind::Warp,
        ProjectionKind::Spherical,
        ProjectionKind::Cylindrical,
    ];

    /// The `projection` param of `layer.fill.projection`
    pub(crate) fn id(self) -> &'static str {
        match self {
            ProjectionKind::UvTile => "uvTile",
            ProjectionKind::Uv => "uv",
            ProjectionKind::Triplanar => "triplanar",
            ProjectionKind::Planar => "planar",
            ProjectionKind::Spherical => "spherical",
            ProjectionKind::Cylindrical => "cylindrical",
            ProjectionKind::Warp => "warp",
        }
    }

    /// The kind `projection` is, as the Projection dropdown lists it
    pub(crate) fn of(projection: &Projection) -> ProjectionKind {
        match projection {
            Projection::UvTile => ProjectionKind::UvTile,
            Projection::Uv { .. } => ProjectionKind::Uv,
            Projection::Triplanar { .. } => ProjectionKind::Triplanar,
            Projection::Planar { .. } => ProjectionKind::Planar,
            Projection::Spherical { .. } => ProjectionKind::Spherical,
            Projection::Cylindrical { .. } => ProjectionKind::Cylindrical,
            Projection::Warp { .. } => ProjectionKind::Warp,
        }
    }

    /// The dropdown's name for it
    pub(crate) fn label(self) -> &'static str {
        match self {
            ProjectionKind::UvTile => n_("Fill (Match per UV Tile)"),
            ProjectionKind::Uv => n_("UV projection"),
            ProjectionKind::Triplanar => n_("Tri-planar projection"),
            ProjectionKind::Planar => n_("Planar projection"),
            ProjectionKind::Spherical => n_("Spherical projection"),
            ProjectionKind::Cylindrical => n_("Cylindrical projection"),
            ProjectionKind::Warp => n_("Warp projection"),
        }
    }
}
