# Roblox and Sketchfab export presets

Built-in presets in `crates/formats/src/presets.rs`: **Roblox (SurfaceAppearance)** and
**Sketchfab**. They show up in the export dialog, `export.preset.list` and
`texturecraft-cli export --preset`.

## Roblox (SurfaceAppearance)

| File (`$mesh_$textureSet_…`) | Content |
|---|---|
| `Color.png` | base color RGB, opacity in alpha |
| `Normal.png` | tangent-space normal, OpenGL (Y+) |
| `Metalness.png` | grayscale metallic |
| `Roughness.png` | grayscale roughness |

The file name endings are suffixes Roblox's importer recognises (color, normal, metalness, roughness),
so reimport assigns each map to its slot. This holds for an atlas export (UVs in 0 to 1). A UV-tiled
export names its files like `Normal.1001.png`; Roblox does not document stripping the tile number
before matching the suffix, so with tiles assign the maps by hand.

`ExportOptions::size` is per UV tile. To stay within the size limit with a tile grid, divide it by the
larger of the grid's columns and rows and round down to a power of two (three columns: 256, not 341).

Sources:
- Reimport and texture suffixes: <https://create.roblox.com/docs/art/modeling/reimport>
- SurfaceAppearance guide: <https://create.roblox.com/docs/art/modeling/surface-appearance>
- SurfaceAppearance API: <https://create.roblox.com/docs/reference/engine/classes/SurfaceAppearance>

## Sketchfab

The template is named exactly `Sketchfab`, so it fills the export window's first-group "Sketchfab" entry.

`BaseColor` (A = opacity), `Normal` (OpenGL), `Metallic`, `Roughness`, `AmbientOcclusion`,
`Emissive`, all 8-bit PNG. Sketchfab detects map types from file name suffixes (roughness, metalness
or metallic, normal, AO, base color/albedo/diffuse, emissive). For a packed ORM upload use the
existing "glTF PBR Metal Roughness" preset.

Sources:
- <https://help.sketchfab.com/en/articles/16152629-how-does-sketchfab-determine-if-a-3d-model-uses-pbr-materials>
- <https://sketchfab.com/blogs/community/automatic-pbr-texture-mapping/>

## NOT verified (read before relying on this)

The vendor pages could not be fetched while writing this (the build sandbox blocked
`create.roblox.com` and `support.fab.com`); only search-result summaries were available.
Please check against the official pages:
- Roblox normal map convention: OpenGL (Y+) is assumed, not confirmed.
- Roblox maximum texture size: 1024 (`preset_max_size`) comes from a 2023 DevForum reply, not
  official docs; the limit may be higher now. The size is passed through `ExportOptions::size`
  (power of two), it is not enforced.
- Sketchfab normal convention: no source found; OpenGL assumed (glTF convention).
