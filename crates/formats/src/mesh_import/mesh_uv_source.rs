use texturecraft_geom::glam::Vec2;

/// How to obtain triangle UVs when reading mesh geometry.
pub enum MeshUvSource<'a> {
    /// Keep complete authored UVs, or unwrap a mesh that needs them.
    FromFile,
    /// Generate a fresh layout even when the file contains UVs.
    Rebuild,
    /// Apply the project's saved UVs in triangle corner order.
    Saved(&'a [Vec2]),
}

impl MeshUvSource<'_> {
    pub fn from_rebuild_setting(rebuild_uvs: bool) -> Self {
        if rebuild_uvs { Self::Rebuild } else { Self::FromFile }
    }
}
