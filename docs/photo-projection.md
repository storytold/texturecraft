# Painting a photo onto the model from the camera

What TextureCraft offers, and how to use it (all steps are commands, so agents can do the same).

## What the audit found

| Need | Already there |
|---|---|
| Load a photo | `resource.import` (File > Import resources, or Import... in the tool properties) |
| Place it over the viewport | Projection tool (`tool.projection`, key 3) + `tool.stencil`: center, scale, rotation, tile; hold S to move/turn/scale with the mouse; overlay opacity preview |
| Paint it through the brush | Projection strokes (`tool.stroke`): brush size, hardness (falloff), flow, opacity, symmetry, camera visibility (hidden surfaces stay clean), backface culling, angle fade, layer masks and the quick mask |
| Other tools | Geometry decals (`layer.add.decal`: an image fixed on the surface, as a layer), fill projections (`layer.fill.projection`: uv, triplanar, planar, spherical, cylindrical) and Clone (copies paint inside the texture, not an image) |

## What was missing, and now added

- `tool.stencil.project` and the **Project whole image** button: bakes the whole stencil from the
  current camera in one step, with the brush's culling and angle fade (`paint::view_weight`), so
  surfaces seen at a grazing angle fade out instead of stretching the photo.
- **Base color only** (`baseColorOnly`): the photo paints the base color alone; off, the tool's
  other enabled channels are painted too, with the photo's alpha as their coverage. It applies to
  brush strokes and to the one-click projection.

## Still open

- The stencil is a flat 2D image. There is no perspective-corrected "camera match" and no
  seam-aware blending when you project from several cameras.
- Angle fade uses the interpolated surface normal; very coarse meshes fade per triangle.
