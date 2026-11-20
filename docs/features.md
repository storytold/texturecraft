# TextureCraft features

Everything TextureCraft does today, by area. What's still missing is in [`ROADMAP.md`](../ROADMAP.md).

## Paint in 3D

- **Brushes** with size, flow, stroke opacity, spacing, hardness, angle and follow path; size,
  angle, flow and position jitter; tangent or camera alignment, angle fade, backface culling, lazy
  mouse, X/Y/Z mirror and radial symmetry (copies paint the far side too, and in the 2D view they land where the 3D
  copies would) and pen pressure in the 3D and 2D views (Windows Ink pens, and pens that send touch events in the browser, such as on
  tablets; on Linux, macOS and desktop browsers a pen works as a mouse for now).
  Ctrl+drag in a view sets them on the fly: right-drag the size and hardness, left-drag the flow
  and angle.
  **67 procedural brush alphas** in their own shelf tab, or any imported image as the alpha. Every alpha has Parameters (Properties › Alpha): hardness, size X and Y,
  roundness for the Round shape, contrast and invert, with Restore defaults.
  **18 brush presets** (basic, wear and grunge, details) in the shelf's Brushes tab set the
  stroke and keep the size. The Brush section's **Brush presets** popover (and the shelf) also keeps
  your own: save the current brush, click to use it, rename, replace or delete it.
- **Tools:** paint, eraser, **projection** with image stencils (hold S to move, turn and scale the
  stencil), polygon fill (triangle, UV island or the whole mesh), **path** (Shift+click places a corner point for hard edges) and **lasso fill**, a
  **particle brush**, **smudge**, **clone** (V + click sets the source, marked in the 3D and 2D views;
  an Aligned option keeps or restarts the offset per stroke), material picker, Shift+click straight
  lines (Ctrl+Shift+click snaps them to 15 degree steps), and painting directly in the **2D UV view**.
  Brush, symmetry and the other tool settings stay when you open another project and when you
  restart the app.
- **Geometry decals:** drop an image on the surface, then move, scale and turn its frame, keep the image's proportions
  while sizing it or flip it; it stays an editable fill layer. Mirror across X, Y or Z (optionally flipping the copy's image in U or V)
  to place a copy on the opposite side of the mesh (move the mirror plane with Axis position); it follows the decal.
- **Text** in any installed font (or the built-in ones), shaped for every script with kerning and
  ligatures, one or more lines: stamp it with the brush as its alpha, or place it as a decal in the
  tool's base color (Properties › Alpha › Text).
- **Quick mask** (Y): paint a temporary mask (U to edit, Ctrl+Shift+I to invert) that protects
  everything outside it while you paint.
- **Every channel at once:** base color, metallic, roughness, height, normal, emissive, opacity,
  ambient occlusion and specular level, plus clear coat, sheen, anisotropy, subsurface scattering
  and transmission, each toggled per stroke in the brush's material properties.

## Layers, masks and smart materials

- **Paint, fill, folder and instance layers** with **32 blend modes per channel** (Normal through
  Normal map detail), per-channel opacity, visibility, color tags, drag-and-drop reordering,
  duplicate, group, flatten, cut, copy and paste across texture sets, and **geometry masks** that
  show a layer only on the mesh parts you pick (right-click a layer, or click parts with the
  **Geometry mask** tool, 8). **Solo layer** (right-click a layer, or Alt+click its eye) shows
  one layer alone in the 3D and 2D views, optionally without its mask (Alt+Shift+click, or
  **Solo layer without its mask**), to judge a material hidden behind a mask. It is only a view:
  the document, the undo history, saving, exporting and baking never see it.
- **Several layers at once:** Shift+click a layer to select the rows from the last clicked one,
  Ctrl+click (Cmd+click on macOS) to add or take out one layer, Ctrl+A (Cmd+A) with the pointer
  over the Layers panel to select every shown row (folder rows and the layers of open folders).
  The eye of a row always acts on that layer only. Delete, the trash button, duplicate
  (Ctrl+D), group (Ctrl+G: the new folder takes the place and folder of the topmost selected layer; copies go in one block above it), the row menu and drag-and-drop then act on every selected layer, each
  as one undo step.
- **Filters and content effects** on any layer or mask: **31 filters** (HSL, levels,
  contrast/luminosity, color balance, gradient map, posterize, clamp, histogram scan, range and
  shift, invert, threshold, grayscale, replace color, edge detect, normal invert, baked lighting,
  blur, directional and slope blur, sharpen, high pass, bevel, glow, warp, directional warp,
  mosaic and four material finishes), fills and paint. **Levels** shows a histogram of what it
  reads (the mask below it, or a content channel you pick) with its input range marked.
  **Anchor points** record the stack at a layer (or, inside a mask, the mask value at that point)
  so masks, fills and generators higher up can react to it; a generator can take an anchor's
  painted height as **micro detail**. Double-click an effect to rename it.
- **Masks** with a white or black background and a stack of effects: painted strokes, fills,
  **mesh-map generators** (edge wear, dirt, thin areas, curvature, AO, position and direction
  gradients, all with world-space noise), filters, ID color selection and compare. Alt+click a
  mask to view it alone. Drag an effect to reorder its stack, or right-click it to duplicate it
  (painted strokes included) or copy it onto another layer or mask (Ctrl+C, Ctrl+V); with an
  effect selected, Ctrl+D and Delete duplicate or remove it.
- **Fill layers** take uniform values, **43 procedurals** (clouds, cells, grunge, scratches, wood
  grain, bricks, Tile Generator, fabric weave, twill, leather grain, marble veins, Voronoi, paint
  chips, hexagon tiles, wave scales, herringbone planks, halftone dots, diamond brocade, Truchet
  arcs, smooth fur, mackerel sky, soft billow, reptile scales, young bark, rotting flesh,
  camouflage, knit stitches, basket weave, goose-eye twill, slub silk, strip flooring, knotty
  wood…) or your own **images**. Patterns and images support UV, tri-planar, planar, spherical,
  cylindrical and decal projection. The fill's properties expose UV scale, rotation and offset,
  tri-planar blend sharpness and offset, planar axis and offset, and cylindrical axis. Set the
  pattern resolution separately for each source. Each channel takes its own image, such as a
  normal, roughness or height map. Import it with *File › Import resources...*, then select it
  in the channel's source picker or drop it from the Assets panel onto the channel. Mark
  DirectX normal maps (Y down) with **DirectX normal map**.
  For a color channel that reads a pattern or anchor point, choose **Gradient Map** from the
  three-dot menu beside its source slot. Up to 16 color stops map dark to light across the
  pattern or recolor the stack below by its lightness. The **Gradient map** filter does the
  same for a whole layer's content.
  Alt+click a material channel (of a fill or the brush) to keep that channel alone.
- **72 materials** (metals from measured reflectance, car paint, brushed anisotropic steel,
  velvet, skin, wax, glass, ice, woods, bamboo, bark, stone, cobblestone, gravel, plaster, sand,
  mud, snow, bone, moss, plastics, fabric, linen, wool, leather, ceramic, rust, cork, paper), **21 smart
  materials** (rusted iron, worn paint, oxidized copper, bronze statue, weathered wood, mossy
  brick and stone, sun-faded paint…) and **24 smart masks**, all procedural, so they work at any
  resolution. Drag them from the tabbed, searchable shelf onto the layer stack (below the row you
  drop on) or straight onto the model (a new fill layer on top). A click on a simple material makes it
  the brush's material: the next stroke paints its pattern (colour, metallic, roughness, height),
  fixed to the surface and sized with the brush. A click on a smart material, which is several
  layers, only highlights it; drag it to add its layers.
- **Import resources:** *File › Import resources...* opens a window: *Add resources* (images and
  materials), an optional name prefix and a role for each image. Environment images light the scene,
  and alpha images become the brush alpha. Filter by role or remove selected rows before importing.
  Failed rows stay in the dialog for retry. Color LUT, post effects and asset-library import are
  unavailable. The browser imports images straight from its file picker.
- **Your own library:** right-click a layer or folder, choose *Create smart material*, and it
  appears under *My smart materials*, pixels and images included. *Create smart mask* in the same
  menu saves just the layer's mask (all its effects, painted pixels and images); it appears under
  *My smart masks* and applies to any layer like a built-in smart mask.
- **MaterialCraft materials:** *File › Import material*, a drop on the window, or MaterialCraft's
  *File › Send to TextureCraft* (it lands under *My smart materials*) adds a `.mcmat` portable
  material, or a `.mcraft` package, as a fill layer. Its exposed graph inputs (numbers, vectors,
  toggles, options, colors) and presets show in the layer's properties; every change re-cooks
  the maps through MaterialCraft's command line at the texture set's resolution, on a worker
  thread, with the set's baked mesh maps (curvature, AO, thickness…) feeding graph inputs of the
  same name. *Open in MaterialCraft* edits the graph itself. In the browser the cooked maps paint,
  but the inputs are read-only.

## Files and texture sets

- **Open** glTF/GLB, FBX (binary and ASCII), COLLADA, **USD** (usda, usdc, usdz), **Alembic**,
  OBJ, PLY and STL, or a built-in mesh (the `project.new` command; *File › Open sample project...* opens the
  material ball); UVs are generated when a mesh has none. A `.gltf`'s buffers
  and images and a loose USD layer's textures are read from its folder and kept in the project.
  Reimport a changed mesh and your painted strokes stay on the surface. The
  `file.changeMesh` command (trusted local command API and the control channel) moves the project onto a different mesh file and keeps its layers.
  **Edit › Project Configuration...** (desktop) does the same from the 3D MESH section: *Select...* picks the
  file, *Reimport mesh* is ticked for you (or tick it alone to read the same file again), OK applies. *Import settings* decides whether painted
  strokes follow the surface (on by default), and PROJECT SETTINGS sets the project's normal map format and
  shows its UV tiles in the UV workflow tooltip. Edit the tiles in Texture Set Settings.
  Physical size and color management show the same settings as New project.
- **File › New project...** opens a setup form with BASIC SETTINGS (*Select...* a mesh file,
  the project template, the document resolution; Create waits for a mesh) and ADVANCED SETTINGS (normal
  map format, *Use UV Tile workflow*, *Import cameras*, *Import baked maps*, physical size, color
  management). The project is named after the mesh file. Settings TextureCraft does not have yet
  (tangent space per fragment, UV tiles as separate texture sets, turning auto-unwrap off and its
  options, a custom unit scale, OpenColorIO) are shown disabled with a hint; projects use Legacy color
  management. In the browser, *Select...* opens the browser's file picker.
- **UDIM / UV tiles:** detected on import or switched on later, with a resolution per tile (blurs,
  other filters and height-to-normal slopes keep their size in UV space at any tile resolution)
  and one texture per tile on export (`$udim`).
- **Texture sets** from 128 to 8192 px per material, resampled when you change them, with 21
  channel types to add or remove. Show, hide or invert them all at once, or **focus** on the
  active one. The project's **normal map format** (OpenGL or DirectX, in Texture Set Settings)
  says how a normal map image set as a mesh map is read, and is the default for export.
- **Projects** are a single `.tcraft` file: the mesh, every layer (sparse and lossless), baked maps
  and imported images. **Project templates** start new work from a saved setup. *File › More save options › Save as template...* saves the open project's setup (dialog "Save template", in the user templates folder, no confirmation); New project lists "Default (OpenPBR)", the other built-ins with " (starter_assets)", yours with " (your_assets)", and opens on the template of the last project. *More save options* also has *Save as copy...* and *Save and reduce file size* (greyed, "Not available yet"). Closing the app,
  or opening, starting or closing another project, with unsaved changes asks whether to save them
  first (in the browser too, where Save downloads the project and closing or reloading the tab
  asks before the changes are lost).

## Bake and export

- **Bake mesh maps** on the GPU (ray traced, about 12× the CPU path) or on every CPU core: ambient
  occlusion, curvature, thickness, world-space normal, position, ID, bent normals, opacity, color
  and direction maps, and normal and height maps from high-poly meshes, with a cage, ray distances
  and matching by mesh name. Or **use your own images** as mesh maps (baked in another app, or
  edited): drop one from the Assets panel on a slot of the Texture Set Settings, or pick it from
  the slot's menu. A UV-tiled set takes one image per tile, named with its tile number
  (`ao.1001.png`, `ao.1002.png`…): drop or pick any one of them.
- **Bake presets:** Mesh Map Bakers > Presets saves the active texture set's bake settings under a
  name, loads one back into the active set (its Include in Bake choice stays) and deletes it.
  Presets are kept between sessions. **Apply to All Texture Sets** copies the active set's settings,
  including its map list, to every set. When a bake ends, a toast shows the elapsed time and
  counts newly baked maps and failed sets. **Open Log** on a failed bake shows the General
  channel with the summary and the message for each failed set.
- **Live bake preview:** while the app bakes, each mesh map shows on the model as soon as it is
  finished; the bake bar follows the newest map, pins a finished one or shows the material.
- **Export presets** for glTF PBR Metal Roughness, Unreal Engine (packed), Unity HDRP (mask map),
  Unity Standard, Blender (Principled BSDF), Godot 4 (ORM), OpenPBR (with the coat, fuzz,
  anisotropy, transmission and subsurface maps), separate maps, mesh maps and document channels, in
  PNG, JPEG, JNG, PGM, TGA, TIFF, BMP, WEBP, PSD, EXR or HDR at 8 bits (optionally dithered), 16
  bits or 32-bit float (TIFF, EXR, HDR; 16-bit half float for EXR), with UV padding (dilation,
  infinite, or dilation + diffusion), size override and OpenGL or DirectX normals.
- **Output template editor:** build your own presets with per-channel packing, converted maps and per-channel inversion,
  and share them as JSON. Right-click a preset to rename, duplicate or remove it; F2 renames.
- **Export panel** on the right bar: *Export Mesh...* asks for the mesh export options, then for
  the file; *Export Textures...* opens the Export textures window (Settings, Output templates and
  List of exports, *Include all*, *Save settings* and the number of textures the export writes;
  *Cancel* goes back to the settings the window opened with, *Maximum size for all Texture Sets*
  exports every set at the largest set's size; a click on a texture set overrides its template,
  file type, size or padding, and each output map's file type and bit depth, or leaves a map out;
  Export shows the progress, a console of its steps and the written files, and can be canceled
  between texture sets).
- **Mesh export** to OBJ with an MTL material library and its maps (PBR extension keys for
  roughness, metallic, normal and coat; UV-tiled sets keep their UDIM UVs and export their maps
  through Export textures), textured GLB (with clear coat and sheen) and USD (with clear coat).

## The viewport

- Physically based shading (metallic/roughness or specular/glossiness) with clear coat, sheen,
  anisotropy, subsurface scattering, transmission, parallax occlusion and **displacement** (with
  its own shadows) from the height channel; image-based lighting from six procedural studio and
  outdoor environments or **your own HDR/EXR panoramas**; ACES or linear tone mapping, key-light
  shadows, bloom, vignette, sharpen, grain, **depth of field** and **saved cameras** (also imported
  from glTF, FBX, COLLADA and USD files).
- **Rendering mode** (F10): a progressive, **path-traced** view of your textures with multi-bounce
  light, environment importance sampling, displaced geometry, a denoiser and one-click PNG save,
  traced on the GPU
  (compute shaders, about 25× the CPU tracer, desktop and WebGPU) or on the CPU.
- View any channel, any baked mesh map, or a UV checker; Alt+drag to orbit (Alt+Shift+drag in 90°
  steps), middle-drag to pan, wheel to zoom, F to show the entire mesh, Shift+F to frame the active
  texture set (a floor plane or other helper geometry stays out of the view and the orbit pivot),
  M / C / B to cycle views (C and Shift+C also visit Normal+height+mesh). The view picker groups
  the modes under *Lighting*, *Single channel* and *Mesh maps*; *Other...* opens a search of the
  project's images and shows the chosen one on the mesh and in the 2D view; Display Settings has
  *Display solo view without lighting (unlit)* (on by default; off shades a channel view). Choose **free (trackball) or constrained rotation** in the top bar (free
  turns the model over; commands for stepped turns and for the next / previous saved camera can be
  bound in Settings), and the **Normal+height+mesh** view in the view picker. Orbit, pan and zoom buttons in the corner of each view do the same with a
  plain drag, so a pen and tablet can navigate without a keyboard or wheel (Viewport menu ▸
  Navigation buttons hides them).
- **Display Settings** pick the environment from a thumbnail grid (or drop an image on the slot),
  with opacity and blur in percent, an environment alignment to the world or the camera and a
  shadow quality and opacity; Settings > General > Background color sets the plain
  background (top and bottom color) that shows at environment opacity 0.
- **2D view navigation** uses the camera mouse bindings: Alt+left drag turns the view freely (the U/V gizmo turns with it) and Alt+Shift+left snaps the turn to 90° steps, Alt+right drag zooms, Alt+middle drag and Ctrl+Alt+left drag pan 1:1, the wheel zooms toward the cursor, and F resets pan, zoom and rotation (it frames tile 1001 in a UV tile set). The zoom reaches about ×220 in and about 2 px per tile out. Plain middle drag and a double click (reset) stay as extras.
- **Perspective (F5) or orthographic (F6)** projection, and **front, back, left, right, top and
  bottom views** that frame the mesh (Viewport menu; bind your own keys in Settings).
- **3D and UV wireframes** that show the mesh's own polygons: quads and n-gons from OBJ, FBX,
  COLLADA, USD, Alembic and PLY files keep their edges, without the triangulation's diagonals.
  One **Show mesh wireframe** switch in Display Settings (off by default, with **Wireframe opacity**, 40 to start) draws
  the edges in the 3D view and the UV edges, in red, in the 2D view.
- **UV tile grid** in the 2D view of a UV tile (UDIM) project: grid lines over the whole view, a red U and a green V axis
  line through the UV origin, each tile's number in a small box at its lower-left corner (dimmer for tiles the set does not
  paint) and a label at the bottom left with the set and the tile under the cursor ("UdimMat - 1001"). A button left of the
  view mode hides the numbers and the label.

## Interface

- **Swap 3D/2D** (F4, Viewport menu and the layout dropdown, rebindable) puts the 2D view on the left and the 3D view on the right in the split layout. Painting, the divider and the views' headers work on both sides, and the choice is kept with the UI state.
- A modern look in **four brightness levels** (dark, medium dark, medium light and light) with
  **seven accent colors**, set in Settings › General › Appearance.
- **Its own title bar** with the app icon, menus and window buttons on Windows (and on Linux
  desktops that leave the frame to the app, such as GNOME), the macOS traffic lights over the same
  strip, and Settings › General › Appearance › System title bar for the system's frame instead.
- **Dockable, tabbed and floating panels**, saved layouts, and panels that reopen where you closed
  them. The right bar's panels (Export, Log, Display Settings, Shader Settings, History) pop out
  beside it; *Undock* makes one a floating window, and its *Dock* button puts it back on the bar.
- **Window menu** as in the reference: Views (with *Renderer Settings*, a small panel for Rendering
  mode, and *Properties - Paint*, which names the tool), Toolbars (*Docks*, the right bar, and *Tools*, the
  tool shelf, switch off and on; *Plugins* is greyed), *Save UI Layout* for the Painting, Rendering or
  Baking mode, *Load UI Layout* ("Select a UI layout file.") and *Hide UI* with its check mark. Views
  are greyed per mode as measured: Rendering keeps Display Settings, Log, Renderer Settings, Shader
  Settings and Texture Sets; Baking greys Assets, Export, Layers, Properties, Renderer Settings, Shader
  Settings and Texture Set Settings. *Python Console* and *Online assets* are greyed ("Not available yet").
- **Display and Shader Settings** are one scrolling page each: their section tabs jump to a
  section, and scrolling highlights the tab of the section in view.
- A **Log** panel with a *GENERAL* and an *ASSETS* tab: asset messages (resources that do not
  decode, skipped templates and presets, PBR folder and MaterialX import warnings) go under ASSETS.
  Lines read `[Source] message`, coloured by level (info, warning, error), and can be
  multi-selected; the right-click menu copies them (`Ctrl+C`) or clears them.
- **No project at launch**: empty views, panels and toolbars until you create or open one (File ›
  New project, Open, Recent projects, Open sample project); File › Close returns to that state.
- A **data folder** of your choice for autosaves, smart materials, presets, templates and UI
  layouts, on any drive (Settings › General › Folders; `Documents/TextureCraft` by default).
- A **Settings window** (Edit › Settings) with General, Shortcuts and Libraries pages; changes
  apply on OK and Cancel drops them. General holds the language (or the system's), the 3D view's
  rotation, zoom and wheel speeds with inverse directions, GPU or CPU bake rays, resource limits
  (worker threads, cores kept for the UI, RAM while saving, project size on open) and the autosave
  interval. Shortcuts rebinds any command in the sections Actions, Mouse shortcuts and Modifiers shortcuts (Delete clears a binding, a shared shortcut warns and keeps OK disabled, Reset all to default restores every key); the window reopens on the page it was closed on. Libraries adds or removes folders whose saved smart
  materials and masks the shelf lists.
- The **shortcut list** (Settings › Shortcuts and Help › Shortcuts list) names the actions as the reference does and starts with its default keys (Switch to baking mode Ctrl+Shift+B, Select next and previous camera `>` and `<`, Viewers layout F1 to F4); Show mesh wireframe is a bindable command.
- A **shortcut editor** with conflict detection, listing every menu item and every command a key can run (commands that need a path, name or similar are left out), and every shortcut shown in its tooltip.
- **Help menu:** Release notes... opens the project's releases page. Export log... writes every
  channel to a text file with each entry's recorded UTC time, severity and source. The desktop app asks
  for a destination, and the browser downloads the file.
- A **History** panel: click any step to go back to it, or forward again to an undone one.
- **10 languages:** English, French, German, Spanish, Portuguese, Italian, Japanese, Chinese,
  Korean and Arabic, switchable live.
- **In the browser:** the same app compiled to WebAssembly, on WebGPU or WebGL2, with touch
  navigation and project open, save and download.

## Built for agents

Everything you can do in the window, an agent can do too:

- **CLI:** `texturecraft-cli new builtin:material_ball out.tcraft`, `bake`, `export`, `render`,
  `run <command> <params>`, `batch`, `commands`, `info`.
- **MCP server:** `texturecraft-cli mcp` gives Claude and other agents tools to create projects,
  add layers and masks, paint strokes, add high polys and a cage, bake, export textures and the
  mesh, reimport a changed mesh, render previews and inspect everything they changed, with file
  access limited to the folders you give it.
- **Control channel:** launch the app with `--control <port>` to drive the running UI: run any
  command, invoke menu items, paint, and take screenshots.

See [`control-protocol.md`](control-protocol.md) for the full protocol and examples.
