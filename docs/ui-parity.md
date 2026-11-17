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

P1 affects frequent editing or delivery tasks. P2 is important depth or usability work. IDs are
stable review references, not a complete inventory of the reference interface.

| ID | Priority | Missing capability | Evidence and current workaround | Acceptance check |
|---|---|---|---|---|
| UI-01 | P1 | Effect menu on the mask thumbnail | **UI + source.** Right-clicking the thumbnail did not open its own menu. The reference exposes mask operations and effect creation there. TextureCraft's layer-name context menu does add effects to the selected mask; adding a generator through that route worked | Right-click a mask thumbnail, add a generator/filter, and verify the effect belongs to that mask without changing the target first |
| UI-03 | P1 | Independent U/V tiling and a link/unlink control | **UI + source.** The fill panel has one Scale / tiling value. `projection_transform` reads the first UV scale component and submits a scalar; the model stores two components. The reference exposes two linked values | Set U = 2 and V = 5, switch projections/selections, and reopen the project without silently coupling the values |
| UI-04 | P1 | Fill sampling/filtering and UV wrap controls | **UI + source.** The reference fill panel exposes filtering and wrapping. TextureCraft's fill editor exposes projection, transforms and channel sources without those controls. No per-fill UI workaround was established | Set filtering and wrapping per fill, inspect the difference at a UV border, and retain the settings across save/reopen |
| UI-05 | P2 | Physical-size fill scaling | **UI + source.** The reference Scale menu offers tiling or physical size. TextureCraft exposes dimensionless scale only | Give a pattern a physical size and show that it keeps that size on meshes with different dimensions |
| UI-06 | P2 | Warp and per-UV-tile fill projection choices | **UI + source.** Both choices were visible in the reference projection menu. TextureCraft's ordinary fill picker has UV, tri-planar, planar, spherical and cylindrical choices; decals have a separate editor. A warp filter is a different operation | Exercise the two projection workflows on a multi-tile asset, with explicit controls, persistence and command inspection |
| UI-08 | P2 | Save a reusable user smart mask | **UI + source.** The reference mask menu has a smart-mask creation action. TextureCraft offers built-in smart masks and Create smart material, but no equivalent action for saving just the edited mask | Save a multi-effect mask, apply it to another layer/project, and retain its parameters and resource dependencies |
| UI-09 | P2 | Export/copy a mask bitmap from its context menu | **UI.** The reference exposes file and clipboard mask export; these actions are absent from TextureCraft's inspected menu. Previewing a mask is available, and is not a full-resolution bitmap-export check | Export the evaluated grayscale mask at its texture/tile resolution with explicit bit depth and orientation; test the file and clipboard paths |
| UI-10 | P1 | MaterialX output from the texture-export dialog | **UI + source + commands.** The dialog has no MaterialX option and `export_params` does not forward `materialX`. Command export with the OpenPBR preset successfully wrote `.mtlx` and maps | Enable MaterialX in the UI, preview the manifest and texture outputs, export, and validate the same result as the command route |
| UI-11 | P1 | Configurable project/display color-management workflow | **Source.** `DisplaySettings` provides fixed Linear/ACES-filmic tone mapping; export has fixed color spaces and metadata. There is no OCIO configuration/display/view model to expose in project or display settings | Select a configuration, working space and display/view transform; retain them; distinguish color channels from data maps through painting, preview and export |

UI-02 and UI-07 were findings at the assessed `62b2b13` revision and are no longer missing in
the integrated source. Effect rows now have a Rename action and editor, and anchors can record
intermediate mask values for higher layers. Engine and compositor regressions cover mask-anchor
consumption. The original UI task audit did not exercise these new controls; a live UI rename,
undo and save/reopen task remains to be repeated on the integrated build.

The relevant implementations are [layer rows, menus and fill/brush editors](../crates/ui-egui/src/dock.rs),
[generator properties](../crates/ui-egui/src/generator_panel.rs),
[export settings and parameters](../crates/ui-egui/src/windows.rs),
[layer/effect/projection types](../crates/core/src/layer.rs),
[display settings](../crates/core/src/project.rs), and
[smart-mask commands](../crates/engine/src/smart_masks.rs).

## Partial areas

| ID | Area | Observed limit | Next check |
|---|---|---|---|
| UI-12 | Generator depth and property coverage | **UI + source.** The picker contains seven kinds. Switching to Position, changing Global balance and duplicating the effect worked. Shared global adjustments, resolution and input slots already exist; that does not establish the richer generator/control inventory in [#374](https://github.com/storytold/texturecraft/issues/374) | Audit each generator's inputs, micro-detail behavior and controls against the reference; do not count seven shared implementations as equivalent to its whole catalog |
| UI-13 | Narrow workspace readability | **UI.** At 1280 × 800 the app rendered, both views stayed visible and the toolbar exposed scroll arrows. Layer names/types became heavily truncated, the layer toolbar wrapped, and Properties needed scrolling. This was not an input-failure or docking-persistence test | Repeat the editing tasks at smaller sizes, widen/rearrange panels, check full-name tooltips and keyboard access, and retain the layout after restart |

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
| F05 View switching | UI | Pass | F2 selected Only3D, F3 Only2D and F1 Split; the screenshots changed accordingly |
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

- Polygon fill already has triangle, polygon, connected-mesh and UV-island modes, a rectangle
  selection workflow and topology display. F11 exercised the UV rectangle route; a full trim-sheet
  authoring task is still unverified.
- The 2D paint cursor was visible, and direct 2D painting worked. This does not reproduce the older
  cursor/painting reports on this setup.
- Brush size/hardness and flow/angle gestures have a visible hint. Hardness is present under
  Flow and opacity; Angle has its own section. Their absence must not be inferred from the initial
  scrolled position. The broader precision/shortcut concerns in
  [#545](https://github.com/storytold/texturecraft/issues/545) still need an input-device task test.
- The shelf displayed four columns at the larger size, with type controls and search. Do not
  treat every older shelf-layout request as a currently missing implementation.
- A missing mask-thumbnail menu does not mean mask effects are unavailable: the layer-name menu
  worked. A missing MaterialX UI option does not mean the exporter is absent.

## Still unverified

Pen pressure/tilt and other input hardware; brush-feel matching; projection/stencil, clone,
smudge, path and symmetry task suites; a complete trim-sheet task; creating and editing output
templates; docking/floating/restart persistence; history-panel navigation; many-set/many-tile
projects; long sessions and recovery; matched performance budgets; and destination-app image
fidelity. The new `.spp` reader is a future goal, not a tested capability.

No implementation fixes were made in this assessment. The build from the earlier pass was
reused; the full Rust tests, clippy, wasm and other operating-system builds were not rerun.
