//! [`GeometryMask`]: limits a layer to parts of the geometry, by mesh part name or by UV tile, so it survives a mesh
//! reimport. The compositor zeroes the layer's mask on texels outside it; a layer without one (`None`) shows everywhere.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GeometryMask {
    /// The layer shows only on these mesh parts (submesh names). Names no part of the current mesh carries are kept,
    /// so a part missing from one mesh import comes back masked as before.
    Parts(Vec<String>),
    /// The layer shows only on these UV tiles (UDIM numbers, 1001 = the first tile). On a texture set without UV
    /// tiles every texel is in tile 1001.
    UvTiles(Vec<u32>),
}

impl GeometryMask {
    /// True if the mask lets the part named `name` through (a UV-tile mask does not restrict parts).
    pub fn includes_part(&self, name: &str) -> bool {
        match self {
            GeometryMask::Parts(parts) => parts.iter().any(|p| p == name),
            GeometryMask::UvTiles(_) => true,
        }
    }

    /// True if the mask lets UV tile `udim` through (a part mask does not restrict tiles).
    pub fn includes_tile(&self, udim: u32) -> bool {
        match self {
            GeometryMask::Parts(_) => true,
            GeometryMask::UvTiles(tiles) => tiles.contains(&udim),
        }
    }

    /// The part names of a part mask.
    pub fn parts(&self) -> Option<&[String]> {
        match self {
            GeometryMask::Parts(parts) => Some(parts),
            GeometryMask::UvTiles(_) => None,
        }
    }

    /// The UDIM numbers of a UV-tile mask.
    pub fn uv_tiles(&self) -> Option<&[u32]> {
        match self {
            GeometryMask::Parts(_) => None,
            GeometryMask::UvTiles(tiles) => Some(tiles),
        }
    }
}

/// The stored forms. Part masks keep the plain list of names that format version 2 stored for every geometry mask,
/// so files of both versions read the same; other kinds are tagged.
#[derive(Serialize, Deserialize)]
enum Tagged {
    Parts(Vec<String>),
    UvTiles(Vec<u32>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Stored {
    Names(Vec<String>),
    Tagged(Tagged),
}

impl Serialize for GeometryMask {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            GeometryMask::Parts(parts) => parts.serialize(s),
            GeometryMask::UvTiles(tiles) => Tagged::UvTiles(tiles.clone()).serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for GeometryMask {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<GeometryMask, D::Error> {
        Ok(match Stored::deserialize(d)? {
            Stored::Names(parts) | Stored::Tagged(Tagged::Parts(parts)) => GeometryMask::Parts(parts),
            Stored::Tagged(Tagged::UvTiles(tiles)) => GeometryMask::UvTiles(tiles),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn part_masks_keep_the_version_2_form_and_tile_masks_are_tagged() {
        let parts = GeometryMask::Parts(vec!["Body".into()]);
        assert_eq!(serde_json::to_value(&parts).unwrap(), json!(["Body"]));
        let tiles = GeometryMask::UvTiles(vec![1001, 1012]);
        assert_eq!(serde_json::to_value(&tiles).unwrap(), json!({"UvTiles": [1001, 1012]}));
        for m in [parts, tiles, GeometryMask::Parts(Vec::new())] {
            assert_eq!(serde_json::from_value::<GeometryMask>(serde_json::to_value(&m).unwrap()).unwrap(), m);
        }
        assert_eq!(serde_json::from_value::<GeometryMask>(json!({"Parts": ["Lid"]})).unwrap(), GeometryMask::Parts(vec!["Lid".into()]));
        for bad in [json!(3), json!({"UvTiles": ["x"]}), json!({"Other": []}), json!([1, 2])] {
            assert!(serde_json::from_value::<GeometryMask>(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn each_kind_restricts_only_its_own_dimension() {
        let parts = GeometryMask::Parts(vec!["Body".into()]);
        assert!(parts.includes_part("Body") && !parts.includes_part("Lid") && parts.includes_tile(1002));
        let tiles = GeometryMask::UvTiles(vec![1002]);
        assert!(tiles.includes_tile(1002) && !tiles.includes_tile(1001) && tiles.includes_part("Lid"));
        assert_eq!(parts.parts(), Some(&["Body".to_string()][..]));
        assert_eq!(tiles.uv_tiles(), Some(&[1002][..]));
        assert_eq!((parts.uv_tiles(), tiles.parts()), (None, None));
    }
}
