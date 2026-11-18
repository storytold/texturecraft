# UI parity and missing work

Reviewed 2026-11-16 against **main `62b2b13`, TextureCraft 0.6.1**, and reference app 12.2.0.
The [roadmap](../ROADMAP.md) carries the overall readiness estimate. This document records
specific UI gaps, their evidence, and the functional tasks actually exercised.

## Scope and method

The desktop apps were operated with mouse and keyboard input. TextureCraft's read-only
`session.inspect` and `ui.inspect` results confirmed document changes; whole-window screenshots
were inspected after the actions. Export-directory text was entered through the UI's accessibility
control. This is distinct from setting UI state or issuing an editing command directly.

The main UI case used the built-in cube at 1024 px, the default English interface, and a
1920 × 1032 client area. A 1280 × 800 layout was also inspected. The reference used an original
UV cube; its title showed “Read only”, but painting, layer/mask creation and baking worked.
These were qualitative workflow comparisons, not matched-camera, matched-brush or image-difference
tests. No reference artwork, presets or screenshots are bundled here.

**Evidence labels:** *UI* means exercised or inspected in the running interface; *source* means
confirmed in the assessed revision; *commands* means exercised through CLI/control commands.
Source-only findings are not presented as reproduced UI errors. Passing a small task does not
establish complete parity for its feature family.

## Missing controls and workflows

Texture Sets list (gaps H7, H9 and H10 of the round 2 audit, issue 646): a set renames by
double-click or from the row's right-click menu, the same menu hides, isolates and resizes the
set and offers Same size for all sets (also in Texture Set Settings), and Next / Previous
texture set are bindable commands without default keys. Covered by
`crates/ui-egui/tests/it/texture_set_list.rs` and engine tests in `set_cmds.rs`. Clicking a set
in the 3D view is not done.

P1 affects frequent editing or delivery tasks. P2 is important depth or usability work. IDs are
stable review references, not a complete inventory of the reference interface.

| ID | Priority | Missing capability | Evidence and current workaround | Acceptance check |
|---|---|---|---|---|
| UI-01 | P1 | Effect menu on the mask thumbnail | **UI + source.** Right-clicking the thumbnail did not open its own menu. The reference exposes mask operations and effect creation there. TextureCraft's layer-name context menu does add effects to the selected mask; adding a generator through that route worked | Right-click a mask thumbnail, add a generator/filter, and verify the effect belongs to that mask without changing the target first |
| UI-03 | P1 | Independent U/V tiling and a link/unlink control | **UI + source.** The fill panel has one Scale / tiling value. `projection_transform` reads the first UV scale component and submits a scalar; the model stores two components. The reference exposes two linked values | Set U = 2 and V = 5, switch projections/selections, and reopen the project without silently coupling the values |
| UI-04 | P1 | Fill sampling/filtering and UV wrap controls | **UI + source.** The reference fill panel exposes filtering and wrapping. TextureCraft's fill editor exposes projection, transforms and channel sources without those controls. No per-fill UI workaround was established | Set filtering and wrapping per fill, inspect the difference at a UV border, and retain the settings across save/reopen |
| UI-05 | P2 | Physical-size fill scaling | **UI + source.** The reference Scale menu offers tiling or physical size. TextureCraft exposes dimensionless scale only | Give a pattern a physical size and show that it keeps that size on meshes with different dimensions |
| UI-06 | P2 | Warp and per-UV-tile fill projection choices | **UI + source.** Both choices were visible in the reference projection menu. TextureCraft's ordinary fill picker has UV, tri-planar, planar, spherical and cylindrical choices; decals have a separate editor. A warp filter is a different operation | Exercise the two projection workflows on a multi-tile asset, with explicit controls, persistence and command inspection |
| UI-09 | P2 | Export/copy a mask bitmap from its context menu | **UI + source.** Both actions are now in the layer menu. File export writes a 16-bit grayscale PNG per tile; clipboard copy uses an 8-bit grayscale atlas. There is no bit-depth chooser | Repeat file and clipboard export in the native app, including a UV-tiled set; the engine and headless UI tests cover the values and menu paths |
| UI-11 | P1 | Configurable project/display color-management workflow | **Source.** `DisplaySettings` provides fixed Linear/ACES-filmic tone mapping; export has fixed color spaces and metadata. There is no OCIO configuration/display/view model to expose in project or display settings | Select a configuration, working space and display/view transform; retain them; distinguish color channels from data maps through painting, preview and export |

UI-02, UI-07 and UI-08 were findings at the assessed `62b2b13` revision and are no longer missing in
the integrated source. Effect rows now have a Rename action and editor, and anchors can record
intermediate mask values for higher layers. Engine and compositor regressions cover mask-anchor
consumption. Right-click a layer > *Create smart mask* saves its mask (all effects, parameters, painted pixels and images) as a user smart mask that the shelf lists under *My smart masks*; engine tests cover applying it to another layer and a fresh project. The original UI task audit did not exercise these new controls. The rename and anchor controls are now covered
by interface tests (`crates/ui-egui/tests/it/rename_anchor.rs`): renaming an effect row from its
context menu, Undo and Redo of the rename, Escape, and Save as and Open keeping the name; adding an
anchor point to a mask from the Add effect menu, picking it as the Source of a fill in a higher
layer's mask, the resulting composite matching the engine-only result, and Save as and Open keeping
the link. These run in the headless test harness; a live run in the native app on the integrated
build is still not done.

The layer menu also has the reference's **Add bitmap mask** (a submenu of the imported images),
**Copy mask** and **Paste into mask** (Ctrl+Alt+C and Ctrl+Alt+V, both rebindable), and
**Flatten all instances** (the layer and every instance of it in one undo step). Engine tests
(`crates/engine/tests/it/mask_menu.rs`) and interface tests (`crates/ui-egui/tests/it/mask_menu.rs`)
cover the commands, their undo, the menu entries and the keys; a live run in the native app is
not done. Unlike the reference, which copies a mask as internal data only, the copy is kept
in the session and is not put on the system clipboard.

The layer menu also has the reference's **Gamma corrected alpha/mask** (checkable, saved with the
project) and **Export channels to files** (a save dialog; one image per channel the layer holds, the
layer alone). Unlike the reference, the file name is `<name>_<channel>.<ext>` beside the chosen
name and there is no name pattern with tokens. **Lock layer** (a lock icon on the row, a click on it
unlocks) was not measured in the reference: it is not part of the reference's layer menu in the
batch 1 results, and its menu text is ours. Engine tests (`crates/engine/tests/it/layer_menu.rs`) and
interface tests (`crates/ui-egui/tests/it/layer_menu.rs`) cover the commands, their undo and saving, the
menu entries and the lock icon; a live run in the native app is not done.

The relevant implementations are [layer rows, menus and fill/brush editors](../crates/ui-egui/src/dock.rs),
[generator properties](../crates/ui-egui/src/generator_panel.rs),
[export settings and parameters](../crates/ui-egui/src/windows.rs),
[layer/effect/projection types](../crates/core/src/layer.rs),
[display settings](../crates/core/src/project.rs), and
[smart-mask commands](../crates/engine/src/smart_masks.rs).

## Side panels ([#642](https://github.com/storytold/texturecraft/issues/642))

**UI + tests** (`crates/ui-egui/tests/it/side_panels.rs`). The right bar lists Export, Log,
Display Settings, Shader Settings and History in the reference order (Texture Set Settings follows
when it is not docked). Each opens a popup beside the bar with its title in capitals, an *Undock*
button that turns it into a floating window, and *Close*. A floating rail panel has a *Dock* button
that closes it back onto the bar; the float is saved with the UI state.

- **Export:** SEND TO lists three targets and a Configure button, all greyed with "Not available
  yet". The reference names other products there; TextureCraft says *Export to image editor*,
  *Send to compositing app* and *Send to 3D staging app*. EXPORT has *Export Mesh...* (the
  *Mesh Export Options* prompt, then the file choice) and *Export Textures...* (the Export textures
  window: *Global settings* above the texture sets, *General Export Parameters*, *Export USD asset*
  and *Export shaders parameters* greyed, *Include all*, *Save settings* and "N textures selected
  for export"). The prompt's *With displacement/tessellation* choice and both option checkboxes are
  greyed; TextureCraft exports the mesh as it is.
- **Log:** *General* and *Assets* tabs. Messages are sorted by how they begin (see
  `log_panel::ASSET_PREFIXES`); the status bar still shows the latest message of either.
- **Display Settings:** *View mode* above the section tabs; Environment settings (Environment Map,
  Opacity, Exposure (EV), Blur, Rotation; *Override environment map color space* with *Auto*,
  *Environment Alignment* *World*), Shadows (*Shadows*; *Computation mode* *Lightweight* and
  *Shadows opacity* 90 greyed) and Camera settings (*Preset* and *Restore* work on the saved
  cameras). Post effects and Viewer settings keep TextureCraft's own controls. The reference's
  fifth tab is not shown in the screenshot and is left out. Colour management stays greyed.
- **Shader Settings:** greyed *Undo*, *Redo* and *Instance name* (*Main shader*); the *open-pbr*
  button opens the shader choice with the reference's search (*u:shader*) and its 17 entries as
  the screenshot shows them, names cut short included. Only *open-pbr* works. Shader parameters
  (Base Surface, Specular, Transmission, Subsurface) and Displacement & tessellation (Enable,
  Displacement, Tessellation, *Restore defaults*) follow the reference; rows TextureCraft cannot
  drive are greyed. Its own Metallic Roughness / Specular Glossiness model choice, Scatter color and
  Parallax occlusion stay in the fitting sections. *Quality*, *Horizon fade* and the cut-off
  "Enable d..." row are hidden in the reference screenshot and are left out.

## Partial areas

| ID | Area | Observed limit | Next check |
|---|---|---|---|
| UI-12 | Generator depth and property coverage | **UI + source.** The picker contains seven kinds. Switching to Position, changing Global balance and duplicating the effect worked. Shared global adjustments, resolution and input slots already exist; that does not establish the richer generator/control inventory in [#374](https://github.com/storytold/texturecraft/issues/374) | Audit each generator's inputs, micro-detail behavior and controls against the reference; do not count seven shared implementations as equivalent to its whole catalog |
| UI-13 | Narrow workspace readability | **UI.** At 1280 × 800 the app rendered, both views stayed visible and the toolbar exposed scroll arrows. Layer names/types became heavily truncated, the layer toolbar wrapped, and Properties needed scrolling. This was not an input-failure or docking-persistence test | Repeat the editing tasks at smaller sizes, widen/rearrange panels, check full-name tooltips and keyboard access, and retain the layout after restart. Since then: the Layers toolbar stays on one row at 1280 × 800 (its channel box shrinks), and a cut-short layer name or type shows in full on hover. Properties scrolling and restart persistence are unchanged |
| UI-14 | Assets panel filters | **UI + source.** The type dropdown lists the reference entries in its order, Fonts and Show all included. Saved searches and Filter by path are separate scrollable panes with a divider that supports pointer and keyboard resizing and saves its position with the UI state ([#640](https://github.com/storytold/texturecraft/issues/640)). Fonts, the built-in saved searches Favorites, 3D Assets and Flattened layers, and the System fonts and User fonts libraries are listed but greyed with "Not available yet" | Give fonts, favorites, 3D assets and flattened layers real assets, then enable those rows and check that each one filters the grid |
| UI-15 | Edit > Settings ([#637](https://github.com/storytold/texturecraft/issues/637)) | **Source + headless UI tests.** The window has the reference's General, Shortcuts and Libraries pages with OK and Cancel, and the General sections in its order (no Privacy section: TextureCraft sends no usage data). Language (with Default (System Language)), the camera rotation, zoom and wheel speeds with Inverse direction, Enable GPU raytracing, the Shortcuts page and adding or removing library locations work. Show keyboard helper, Show world axes, Background color, Only display the selected material when painting, Viewport Scaling, Default UV scale for materials, simplified thumbnails, the other baking options, the preview cache and shader, the temporary cache directory, sparse virtual texture and the IRAY HARDWARE choices are shown disabled with "Not available yet". A location's Default choice is fixed to your_assets. TextureCraft's own settings (appearance, viewport textures, resources, autosave, folders) follow the reference sections | Run the window in the native app; wire each disabled control as its function lands |

Bake workflow (issue [#651](https://github.com/storytold/texturecraft/issues/651)): Mesh Map Bakers
has a Presets menu (save, load, delete named bake settings; Apply to All Texture Sets) and a finished
bake shows a summary toast (maps baked, time, failed sets, pointing to the Log on failure).

Broader properties/filter work remains tracked in
[#433](https://github.com/storytold/texturecraft/issues/433). Its individual rows need checking
against current code; an open issue is not proof that every control it lists is still missing.

## Functional task audit

These twenty check groups include UI, mixed-surface and command-only cases. They are an evidence
list, not a feature-completion denominator.

| Check | Surface | Result | Observable evidence / limit |
|---|---|---|---|
| F01 New project | UI | Pass | Ctrl+N, built-in Cube, 1024 px, Create produced a new project with one empty paint layer. The native mesh-file selection path was not completed in this deeper pass |
| F02 Paint in 3D | UI | Pass | A mouse drag produced a visible stroke in 3D and its UV representation; inspection showed a Paint stroke undo step and painted tiles |
| F03 Paint in 2D | UI | Pass | A separate UV-view drag increased painted tiles from 7 to 21 per active channel and changed the visible image |
| F04 Undo and redo | UI | Pass | Ctrl+Z restored 7 tiles and history depth 2; Ctrl+Y restored 21 tiles and depth 3 |
| F05 View switching | UI | Pass | F2 selected Only3D, F3 Only2D and F1 Split; the screenshots changed accordingly. F4 (Swap 3D/2D) puts 2D on the left and 3D on the right |
| F06 Brush gesture | UI | Pass | Ctrl+right-drag changed size from 8 to about 12.94 and hardness from 50 to about 38. This also demonstrates that diagonal movement changes both parameters; pen precision was not tested |
| F07 Shelf search and material application | UI | Pass | Searching for car and dragging Car Paint onto the mesh added a red fill layer and an Add Car Paint history step |
| F08 Fill layer and black mask | UI | Pass | Layer toolbar added a fill; the layer-name context menu added a black mask and selected the mask target |
| F09 Generator editing | UI | Pass | Layer context menu added a generator to that mask. Selecting Position and dragging Global balance changed the mask/viewport; inspection recorded balance about 0.279 |
| F10 Duplicate an effect | UI | Pass | Ctrl+D produced a separate Generator copy with the same parameters and a Duplicate effect history step |
| F11 UV rectangle fill | UI | Pass | Tool 4, UV-island mode and a rectangle across two UV islands wrote 88 tiles per painted channel on a new paint layer. Undo removed that fill. Topology was visible in the tool |
| F12 Bake from the UI | UI | Pass | F8, 256 px, low mesh used as high mesh, Check All and Bake completed all ten map types. The UI log reported CPU execution; this is separate from the GPU command check |
| F13 Texture-export flow | UI + accessibility text entry | Pass | Inspected Settings, Output templates and List of exports, then clicked Export. Eight 1024 px PNGs decoded successfully; all eight were byte-identical to command exports from the same saved project/options. The base-color output was inspected visually. Template creation/editing was not exercised |
| F14 Save and reopen | Mixed | Pass within scope | Ctrl+S and the native Save As dialog saved four layers, two generator effects, 21 painted tiles per channel and ten 256 px baked maps. Reopening through `file.open` matched inspected layers, texture sets and mesh. This was not a complete keyboard-only open-dialog test |
| F15 Smaller window | Scripted resize + UI inspection | Partial | 1280 × 800 rendered with working layout structure; the readability limits are UI-13. No full task suite at that size |
| F16 Reference workflow | Reference UI | Pass within scope | Pointer painting, paint/fill-layer creation, a black mask and an empty generator slot worked. A seven-map self-bake at 1024 px completed with map thumbnails and completion entries. Reference save/export and output equivalence were not tested |
| F17 GPU bake | Commands, earlier pass | Pass | All ten maps at 128 px / 16 rays completed; result identified the GTX 1080 GPU tracer. This does not prove that the UI bake used that path |
| F18 High-to-low transfer | CLI | Pass within scope | A two-triangle plane received normal and height from a separate 1,152-triangle analytic surface at 128 px on the CPU. Both maps varied; this was not a reference comparison or absolute-height calibration |
| F19 UDIM sizes and packed channels | CLI | Pass | Tiles 1001/1002 exported at 128/256 px respectively. Both packed ORM maps sampled RGB (255,153,51), matching AO 1, roughness 0.6 and metallic 0.2 |
| F20 Scene/material interchange | Commands, earlier pass | Pass within scope | Textured GLB and USDA reopened in TextureCraft with the original geometry/material; OpenPBR MaterialX XML parsed and referenced exported maps. External consumers/renderers remain unchecked |

F18 used an original surface over x,z ∈ [-1,1], with height
`y = 0.06 + 0.1 * exp(-6 * (x*x + z*z))`, a 24 × 24 quad grid, matching UVs and frontal/rear
distance settings of 0.3. Normal and height were exported as 16-bit PNG. Interior samples read
through an 8-bit display decoder ranged 103–152 in normal R/G and 176–255 in height. These ranges
establish nonconstant transfer only; they are not full-precision error bounds. F19 used the
built-in `udim_plane`, a uniform fill and a tile-1002 resolution override.

## Avoid stale missing-feature claims

- **Paste as instance** has the default key Ctrl+Shift+V, shown in the layer menu and the shortcut
  overview, greyed while no layer is copied, and rebindable.
- Polygon fill already has triangle, polygon, connected-mesh and UV-island modes, a rectangle
  selection workflow and topology display. F11 exercised the UV rectangle route; a full trim-sheet
  authoring task is still unverified.
- The 2D paint cursor was visible, and direct 2D painting worked. This does not reproduce the older
  cursor/painting reports on this setup.
- Brush size/hardness and flow/angle gestures have a visible hint. Hardness is present under
  Alpha > Parameters; Angle has its own section. Their absence must not be inferred from the initial
  scrolled position. The shortcut overview lists Shift+wheel and Ctrl+wheel (brush size, 3D and 2D
  view), Ctrl+right-drag, Ctrl+left-drag and the fine mode (Ctrl+Shift+right-drag changes size and
  hardness by a tenth). Pen precision in [#545](https://github.com/storytold/texturecraft/issues/545)
  still needs an input-device task test.
- The Jitters section has "Position Jitter Axis" (Both / Single) and "Distribution" (Gaussian /
  Uniform), labelled as in the reference. The reference behaviour was not measured: Both and Uniform
  keep the previous behaviour, Single offsets only across the stamp, Gaussian clusters the offsets
  near the centre. Old brushes load as Both and Uniform.
- Hide/ignore excluded geometry is there: the top-bar eye-slash toggle (Alt+H, `viewport.hideExcluded`)
  hides the parts the selected layer's geometry mask leaves out, and strokes and picking skip them.
  Engine tests cover picking and painting through the hidden part; `crates/ui-egui/tests/it/hide_excluded.rs`
  covers the button, the key and the no-project state. The headless tests do not check the drawn pixels.
- The shelf displayed four columns at the larger size, with type controls and search. Do not
  treat every older shelf-layout request as a currently missing implementation.
- A missing mask-thumbnail menu does not mean mask effects are unavailable: the layer-name menu
  worked. A missing MaterialX UI option does not mean the exporter is absent.
- The top bar right group has Pause Engine Computation (Shift+Esc), enabled without a project, with the
  reference tooltip text. While paused no texture is composited or shown; export still uses up-to-date
  results.

- Painting toggles have default keys: L symmetry, D lazy mouse, A resume previous stroke, N ignore stencil and
  X invert greyscale (the mask value swap). All five are in the shortcut overview and rebindable. The Projection
  tool has a Hide stencil when painting option (on by default); the overlay opacity slider stays in the same panel.

## Still unverified

Pen pressure/tilt and other input hardware; brush-feel matching; projection/stencil, clone,
smudge, path and symmetry task suites; a complete trim-sheet task; creating and editing output
templates; docking/floating/restart persistence; history-panel navigation; many-set/many-tile
projects; long sessions and recovery; matched performance budgets; and destination-app image
fidelity. The new `.spp` reader is a future goal, not a tested capability.

No implementation fixes were made in this assessment. The build from the earlier pass was
reused; the full Rust tests, clippy, wasm and other operating-system builds were not rerun.

## Camera and view (2026-11-18)

Closes the camera gaps of [#649](https://github.com/storytold/texturecraft/issues/649):
constrained / free (trackball) rotation, the "Camera snap rotate" commands next to the
Alt+Shift+left drag (which already existed), bindable next / previous saved camera (the
Ctrl+1 ... Ctrl+9 slots stay), and the "Normal+height+mesh" entry of the view picker.
Free rotation uses the same Alt+left drag (and right drag) as the constrained one.
