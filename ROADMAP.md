# TextureCraft roadmap

Stage: alpha (readiness assessment) · next: beta, ~75% readiness plus scoped compatibility and integration gates; remaining effort unestimated

> Last reviewed: 2026-11-25 · Last updated: 2026-11-25 · Change: major (implementation audit and revised readiness estimates) · Target: reference app 12.2.0

The implementation audit covers main at
[`3c026860`](https://github.com/storytold/texturecraft/commit/3c026860e36e5fbe2147c6c78ca0bbc617575887),
with workspace version 0.6.1. Current estimates incorporate landed workflows, Linux regression
tests and the MaterialCraft round trip. The November 16 live comparison with reference app 12.2.0
at [`62b2b13`](https://github.com/storytold/texturecraft/commit/62b2b13fdd32740b6e118cd00c5e0d0c12b30815)
provides the starting scores. The live artist tasks still need a repeat on current main.
See [UI gaps and functional tasks](docs/ui-parity.md), [features](docs/features.md), [menu checklist](docs/parity.md),
[architecture](docs/architecture.md), [automation](docs/control-protocol.md),
[localization](docs/localization-parity.md), [releasing](docs/releasing.md).

**Format direction:** `.tcraft` remains the native editable project format. Read-only opening/import
of `.spp` is a future compatibility goal: leave the source untouched and save converted work as
`.tcraft`. Supported versions, recoverable layers/resources and handling of unsupported content
still need investigation; no `.spp` reader is implemented. Writing `.spp`, round-tripping edits
back into it and compatibility with other proprietary asset/preset formats are outside this goal.
Pipeline exchange uses USD, glTF/GLB, MaterialX and texture maps.

The current `.tcraft` schema/container and output-package contracts remain open to revision.
MaterialCraft and TextureCraft have landed procedural `.mcmat` v3 support. The source-build round trip passed at the revisions below. The exchange gate still needs
a supported release pair and artist workflow checks. ArtCraft UI refactoring and
further shared-library adoption also require integration and regression checks.

## Headline numbers

| | Value | Kind |
|---|---|---|
| Feature breadth | ~90% | Estimated across feature families. The November 16 baseline was ~80% |
| Ready for real work | ~68% | Weighted estimate, up from ~60% on November 16. The largest gains are in feature depth, UI and interchange |
| Mainstream practitioner | ~70% | Estimated for regular asset texturing, baking, masking and pipeline delivery. The November 16 baseline was ~60% |
| Essentials user | ~85% | Estimated for a simple mesh, layered materials and paint, and texture export. The November 16 baseline was ~75% |
| Desktop menu catalog | 59/59 live | Observed on November 16 in TextureCraft's desktop catalog |
| Engine commands | 178 registered | Observed on November 16 through the running app |
| Functional review | 20 scoped check groups | November 16 UI, mixed-surface and command cases. [Functional task audit](docs/ui-parity.md#functional-task-audit) |
| Remaining to beta | Unestimated | Requires acceptance tasks, performance budgets and a decision on required format, package and UI migrations |
| Remaining to full scoped parity | Unestimated | Includes possible `.tcraft` and output-package reworks, `.mcmat` release validation, ArtCraft UI and shared libraries, `.spp` feasibility, and workflow and fidelity checks |

Feature breadth estimates how much of the scoped feature set is present. Readiness also weighs
control depth, output fidelity, responsiveness, project safety and delivery. The earlier ~94%
figure mostly counted implementation paths and used a narrower scope. The current figures are
audit judgments based on the evidence below. Potential format and UI rework can require repeating
earlier checks, and its scope and cost remain unestimated.

### Readiness by audience

| Audience | Ready % | Hours to ~95% | Work that dominates |
|---|---|---|---|
| Full scoped target | ~68% | Unestimated | Workflow depth, format and package decisions, ArtCraft UI and shared libraries, bake and display fidelity, color management, read-only `.spp` and platform validation |
| Mainstream practitioner | ~70% | Unestimated | Production-scale baking and masking, project and package compatibility, and destination-tool checks |
| Essentials user | ~85% | Unestimated | Startup and GPU coverage, discoverable controls, and dependable save/reopen for simple mesh, paint, material and texture-export tasks |

The live baseline verified mouse and keyboard painting in 3D and 2D, undo and redo, material
drag and drop, mask and generator edits, UV rectangle fill, baking, native saving and texture
export. Separate command checks covered high-to-low transfer, UDIM packing and scene/material
export. Since then, the mask and fill controls, generator coverage, Bake Mode and export workflows
have expanded. Beta readiness still depends on representative projects, hardware and performance
coverage, project recovery and destination-tool checks. The latest published stable release is
[v0.6.1](https://github.com/storytold/texturecraft/releases/tag/v0.6.1).

A proposed beta gate is ~75% readiness plus repeatable artist tasks on representative assets:
high/low baking and smart masks, precise 3D/2D painting, a multi-set UDIM project, save/reopen
without lost work, and exports checked in destination tools. Startup failures and data-loss
blockers on the supported test matrix must be resolved. The gate remains open.
Beta scope must identify which reworks/integrations it requires and which remain later goals.
Every adopted project/package version needs a compatibility policy and regression checks;
MaterialCraft exchange must be validated against a named producer/consumer version pair before
it is advertised as ready.

## By dimension

With the same weights, readiness rises from 59.1% to 68.0%. The table shows the November 25 scores.
The evidence column includes the November 16 scores for comparison.

| Dimension | % ready | Weight | Hours to full | Evidence / remaining work |
|---|---:|---:|---|---|
| Features (depth) | ~80% | 30 | TBD | Generators, mask actions, fill controls and bake workflows expanded. Specialized artist tasks and output fidelity still need repeatable checks. November 16 estimate ~65% |
| UI/UX fidelity | ~75% | 20 | TBD | Layer targeting, the workspace, Bake Mode and export dialogs changed. Live task, layout, shortcut and accessibility checks remain. November 16 estimate ~65% |
| File formats / interchange | ~70% | 10 | TBD | Shared import/export codecs, rebuilt UV persistence and linked `.mcmat` v3 have landed. Project/output contracts, `.spp` and destination-tool coverage remain open. November 16 estimate ~60% |
| Hardware (GPU, pen, displays) | ~50% | 8 | TBD | One Windows GPU exercised; pen tilt, pressure across platforms and display coverage remain incomplete. November 16 estimate ~50% |
| Localization | ~35% | 4 | TBD | Eleven picker languages in source; optional shared catalogs/fonts and native-language review need current measurement. November 16 estimate ~35% |
| Performance | ~45% | 10 | TBD | Main added dirty-region uploads, cheaper undo snapshots and faster glTF lookups. Establish response-time and memory budgets with representative large projects. November 16 estimate ~40% |
| Stability | ~60% | 10 | TBD | The live baseline covered small save/reopen and undo/redo. Rebuilt UV layouts gained persistence checks. Recovery, large projects and long sessions remain open. November 16 estimate ~55% |
| Platforms | ~70% | 3 | TBD | Desktop and web build paths exist. Nightly packaging and supervised signing workflows landed for Windows, Linux and macOS. Install, update and hardware checks remain. November 16 estimate ~65% |
| Automation / ecosystem | ~80% | 5 | TBD | CLI, control and MCP share engine commands. Bake settings sync, UV import and generator controls have command paths. The MaterialCraft source-build round trip passed. Surface-specific integration and release-pair checks remain. November 16 estimate ~75% |

## Features

These depth estimates overlap. The overall readiness estimate uses the weighted dimensions above.

| Area | % ready | Hours | What works / main gaps |
|---|---:|---|---|
| Projects and files | ~85% | TBD | November 16 estimate ~80%. PBR folders, ZIPs and selected files import into one fill layer, with packed ORM and OpenGL/DirectX normal support. New Project can rebuild mesh UVs and persist the rebuilt layout. The shipped Material Ball project is available as a starting point. Large saves, recovery and Alembic HDF5/animation remain open |
| Texture sets and UDIM | ~85% | TBD | November 16 estimate ~80%. Baseline command checks covered two tiles at different sizes and packed maps. Bake settings can sync across sets, export settings can vary by set and map, and hovering a set highlights it in the viewport. Validate many-set, many-tile projects at production sizes |
| Layers, masks and effects | ~85% | TBD | November 16 estimate ~75%. Mask thumbnails have their own effect menu and a Shift-click toggle. Effect renaming, mask anchors and user smart-mask saving/application are implemented. Repeat the workflow in a live desktop session with undo and reopen |
| Painting and precise 2D work | ~75% | TBD | November 16 estimate ~70%. The live baseline covered mouse painting, undo/redo and UV-island rectangle fill. Main added brush presets and fixes for steep-angle strokes and intermittent pen-pressure spikes. Brush feel, pressure and tilt across devices, and trim-sheet tasks still need live checks. [Presets #259](https://github.com/storytold/texturecraft/pull/259), [strokes #442](https://github.com/storytold/texturecraft/pull/442), [pressure #399](https://github.com/storytold/texturecraft/pull/399) |
| Generators and smart masks | ~75% | TBD | November 16 estimate ~45%. 16 selectable generator kinds. Added 3D distance and gradient, UV border and diagnostic generators, grayscale conversion, rust runoff, per-kind controls, custom grunge, anchored micro-details and an enlarged preview. Validate control coverage, fidelity and performance on production assets |
| Materials and asset library | ~70% | TBD | November 16 estimate ~60%. Procedural and smart materials, user smart masks, refreshed previews and resource import with usage assignment. Fill properties support independent U/V tiling, filtering, wrapping, physical-size controls, warp and per-UV-tile projection. Validate physical scale across meshes and fill persistence in artist tasks |
| MaterialCraft `.mcmat` integration | v3 round trip passed with source builds | TBD after release-pair validation | Shared reader and the desktop MaterialCraft CLI bring in a published graph as a linked fill. Exposed controls and mesh-map inputs can drive re-cooks at the texture set resolution. v1/v2 imports require republishing. Saved baked maps can be used in the browser |
| Baking | ~75% | TBD | November 16 estimate ~60%. Bake Mode adds texture-set synchronization, automatic fitted cages, background high-poly loading and a settings-driven selected-map preview. CPU/GPU map fidelity, high/low pairs on real assets and resolution-stable wear still need validation |
| Viewport, shading and rendering | ~75% | TBD | November 16 estimate ~70%. PBR/IBL, displacement and CPU/GPU path tracing are available. Main improved camera controls and 2D navigation, including the UV tile grid, and highlights hovered texture sets. Matching material/display behavior, larger scenes and hardware ray tracing remain open |
| Color management | ~35% | TBD | Linear/sRGB handling, fixed export spaces, profile metadata and Linear/ACES-filmic tone mapping are available. The project form displays disabled OpenColorIO settings. Configurable project and display transforms remain open |
| Texture and scene export | ~85% | TBD | November 16 estimate ~75%. Desktop texture export can write companion OpenPBR MaterialX and a textured USD asset. Template editing, subfolder names, per-map overrides and URP/USD presets expanded. Standard Surface and browser companion outputs remain open. Validate complex materials, UDIM and destination tools |
| Read-only `.spp` compatibility | 0% | TBD after feasibility study | Planned; no reader, version matrix or import-fidelity corpus. `.tcraft` remains the save format |
| Workspace, panels and preferences | ~75% | TBD | November 16 estimate ~65%. Menu and layout work, camera and 2D navigation updates, and full-name tooltips for truncated layer labels. Verify small windows, keyboard navigation, docking and restart persistence on the integrated desktop build |
| Automation | ~85% | TBD | November 16 estimate ~80%. Commands cover UV import, bake sync, fill and generator controls and export. The November 16 review exercised control and CLI paths. Recheck UI reachability and direct/bridged MCP coverage on current main |

The [generator catalog](crates/core/src/layer.rs) defines 16 kinds, and
[polygon fill](crates/engine/src/fill_mode.rs) includes triangle, polygon, UV-island and mesh modes.
The [fill properties](crates/ui-egui/src/fill_panel.rs) and
[layer-panel tests](crates/ui-egui/tests/it/layers_panel.rs) supersede the November 16 missing-control
findings UI-01 and UI-03 through UI-06 in the [UI gap inventory](docs/ui-parity.md#missing-controls-and-workflows).
Use those findings' acceptance checks for a new live run. Color management (UI-11), generator
control and output validation (UI-12) and small-window work (UI-13) still need coverage.

### Evidence from this review

| Check | Result and limit |
|---|---|
| Baseline/build | On November 16, fetched `origin/main`; HEAD matched `62b2b13`. `cargo build -p texturecraft -p texturecraft-cli --profile verify` passed; binary reported 0.6.1 and that revision |
| Reference UI/functionality | On an original UV cube, pointer painting, paint/fill/black-mask creation and an empty generator slot worked; seven-map self-bake at 1024 px completed. Inspected fill/projection/scale menus, mask menu and export dialog. The “Read only” title did not prevent these edits; reference save/export and output equivalence remain unchecked |
| TextureCraft UI tasks | Mouse/keyboard new project, 3D/2D strokes, Ctrl+Z/Y, F1/F2/F3, brush gesture, shelf search/drop, black mask, Position generator/global balance and effect duplication worked. UV rectangle fill wrote 88 tiles per channel and undo removed it |
| Persistence | Native UI save of the deeper four-layer case followed by command reopen preserved layers, effects, 21 painted tiles per channel and ten baked maps. Earlier command save/reopen also passed. UI-only open-dialog completion was not established |
| Bake | GPU command check: ten maps at 128 px / 16 rays. UI check: ten maps at 256 px, logged CPU. Separate CPU high/low check: normal and height varied on a two-triangle low plane / 1,152-triangle analytic high surface. No matched reference quality or timing comparison |
| UI texture export | Settings/template/list tabs inspected and Export clicked; eight 1024 px PNGs decoded successfully, and base color was inspected visually. All eight matched command export byte for byte from the same project/options. Creating/editing templates was not exercised |
| UDIM and channel packing | Two tiles exported at 128 and 256 px. Both ORM maps sampled RGB (255,153,51), matching AO 1, roughness 0.6 and metallic 0.2; this is a small command-only case |
| Interchange | Five 256 px OpenPBR PNGs plus parsed MaterialX XML; textured GLB and USDA reopened through TextureCraft's CLI with the original 12 triangles, 24 vertices and material. No external renderer/material equivalence checked |
| Layout | Whole-window PNGs inspected at 1920 × 1032 and 1280 × 800. Narrow layout retained both views and toolbar scroll controls, with severe layer-label truncation; no full narrow-window task suite or pixel-by-pixel UI score |
| Rework/upstream scope | Possible `.tcraft`/output-package reworks and incoming ArtCraft UI/shared-library work are planning inputs, not completed changes. The `.mcmat` v3 proposal and current legacy reader were reviewed; no future contract or current MaterialCraft producer/consumer pair was validated |
| Not run | Full Rust tests, clippy, wasm, other operating systems, pen/display hardware matrix, large UDIM projects, long sessions, matched performance benchmarks and reference image-difference corpus |

The November 25 audit verified the following on main at `3c026860`.

- **Masks, fills and generators.** Mask target menus and toggles, linked or independent tiling, filtering and wrap, physical-size controls, warp and per-tile projection, and 16 generator kinds with per-kind controls and previews. [Layer targeting #771](https://github.com/storytold/texturecraft/pull/771), [mask toggle #841](https://github.com/storytold/texturecraft/pull/841), [fill controls #762](https://github.com/storytold/texturecraft/pull/762), [generators #826](https://github.com/storytold/texturecraft/pull/826)
- **Bake and UV import.** Per-group bake settings sync, fitted cages, background import of high-poly meshes and a settings-driven map preview. Rebuilt project UVs survive reopen. [Bake Mode #799](https://github.com/storytold/texturecraft/pull/799), [UV import #748](https://github.com/storytold/texturecraft/pull/748), [reopen regression](crates/engine/tests/it/rebuilt_uv_layout.rs)
- **Interchange and shared formats.** Merged transitions to shared glTF/GLB, COLLADA, Alembic, USD and raster I/O, with project mapping in TextureCraft. Desktop texture-export dialogs gained MaterialX and USD options, and browser downloads use shared ZIP handling. [Migration tracker](https://github.com/storytold/craft-libs/issues/778), [MaterialX #784](https://github.com/storytold/texturecraft/pull/784), [USD asset #788](https://github.com/storytold/texturecraft/pull/788), [browser ZIP #792](https://github.com/storytold/texturecraft/pull/792)
- **Procedural material exchange.** MaterialCraft publishes a ZIP containing a selected SBS graph, reachable dependencies and resources. TextureCraft inspects and cooks it through the CLI. MaterialX is an optional bake output. [Producer #270](https://github.com/storytold/materialcraft/pull/270), [consumer #644](https://github.com/storytold/texturecraft/pull/644), [commands](docs/control-protocol.md#materialcraft-materials)
- **MaterialCraft compatibility test.** With MaterialCraft CLI 0.4.1 at `998d9d8f`, the Linux integration test published a selected graph, imported its v3 archive, supplied a mesh-map input and re-cooked at the target texture set size. CLI 0.4.1 at `f9174cb` failed earlier in the test when editable `.mcraft` import passed `--graph-path`. [Producer revision](https://github.com/storytold/materialcraft/commit/998d9d8f74454df047b6fb131f57b01d00a4983b), [integration test](crates/engine/tests/it/materialcraft_link.rs)
- **Desktop startup.** Interactive launches without a project now open an empty workspace. Control launches can still request the sample. Measure startup on the supported GPU matrix. [Startup #634](https://github.com/storytold/texturecraft/pull/634), [native entry point](apps/texturecraft/src/main.rs)
- **Release automation.** Isolated nightly launch profiles and a supervised Windows x86-64, Linux x86-64 and macOS arm64 workflow, with a separate signing ref. Scheduled publication remains disabled in this revision. Publication and installation still need end-to-end evidence. [Nightly preparation](docs/nightly-release-preparation.md), [producer](.github/workflows/nightly.yml)
- **Build checks.** `cargo xtask layers` passed for all 15 workspace packages. `cargo xtask wasm` passed for ten library crates and the web app.
- **Linux regression suites.** Locked release unit and integration tests for bake, compositing, formats, engine and headless UI reported 2,978 passing, zero failing and 101 ignored. The engine session suite included the hostile-input command sweep. The real MaterialCraft test was run separately with the CLI configured.
- **Packaging checks.** The local nightly workflow and policy suite passed 28 tests. The Linux tarball launcher and Arch package handoff tests passed using a stub application.

Live desktop artist tasks, browser interaction, destination-tool fidelity, hardware coverage and
signed release installation remain outside the November 25 test evidence.

## Languages

The [picker](crates/ui-egui/src/i18n.rs) contains eleven languages. Shared catalogs are optional at
build time; missing entries fall back to English, and builds without catalogs offer English only.
The older [coverage table](docs/localization-parity.md) predates catalog migration, so its “100%”
rows are not a current measurement. The November 16 live audit ran an English build without external catalogs.

| Language | Code | UI strings | Status |
|---|---|---|---|
| English | `en` | Source language | Exercised in the November 16 live audit |
| Simplified Chinese | `zh-Hans` | Not remeasured | Catalog/font dependent; native review pending |
| Spanish | `es` | Not remeasured | Catalog dependent; native review pending |
| Hindi | `hi` | No picker entry | Not offered |
| Arabic | `ar` | Not remeasured | Catalog/font dependent; shaping/layout needs current review |
| French | `fr` | Not remeasured | Catalog dependent; native review pending |
| Portuguese (Brazil) | `pt-BR` | Not remeasured | Catalog dependent; native review pending |
| Indonesian | `id` | No picker entry | Not offered |
| Japanese | `ja` | Not remeasured | Catalog/font dependent; native review pending |
| German | `de` | Not remeasured | Catalog dependent; native review pending |
| Korean | `ko` | Not remeasured | Catalog/font dependent; native review pending |
| Vietnamese | `vi` | No picker entry | Not offered |

Also in the picker: Italian (`it`), with catalog coverage and native review not remeasured.
Engine error details remain English; UI-key coverage alone would not establish full localization.

## Upcoming

These are acceptance-oriented work areas, not claims that their linked reports were reproduced
here. Open pull requests are not credited until merged into the assessed main revision.

| Rank | Milestone | Estimate |
|---|---|---|
| 1 | Verify empty-workspace startup, reproduce the DirectX startup report on affected hardware, and exercise large saves and recovery across supported platforms. [#489](https://github.com/storytold/texturecraft/issues/489), [#541](https://github.com/storytold/texturecraft/issues/541) | TBD after reproduction |
| 2 | Define `.tcraft` and output-package direction, scope any reworks, and establish versioning, migration and producer/consumer acceptance cases | Unestimated; design scope open |
| 3 | Scope ArtCraft UI adoption and remaining shared-library cutovers. Consolidate the current multiple craft-libs pins in a dedicated update, preserve command reachability and rerun acceptance checks | TBD after adoption scope |
| 4 | Set the supported MaterialCraft and TextureCraft release pair and verify its installed CLI. Extend the source-build check to bundled resources, presets, resolution changes, reopen, missing CLI behavior and v1/v2 republish guidance | TBD after producer/consumer matrix |
| 5 | Measure bake fidelity and wear across resolutions, high/low pairs, all ten maps and CPU/GPU paths. Include automatic cages, name matching, settings sync and final output versus live preview | TBD after map corpus |
| 6 | Exercise the expanded generator, anchor and property controls on production assets. Finish uncovered inputs and workflows after rechecking [#374](https://github.com/storytold/texturecraft/issues/374) and [#433](https://github.com/storytold/texturecraft/issues/433) against main | TBD after control audit |
| 7 | Brush/pen precision, feedback and discoverability; complete 3D/2D/trim-sheet tasks and set responsiveness budgets ([#545](https://github.com/storytold/texturecraft/issues/545), [#485](https://github.com/storytold/texturecraft/issues/485), [#306](https://github.com/storytold/texturecraft/issues/306)) | TBD after task tests |
| 8 | Color-managed pipeline and destination checks for USD, glTF/GLB, MaterialX and texture maps, including UDIM, complex materials and the adopted output packages | TBD after consumer matrix |
| 9 | Read-only `.spp` discovery: clean-room feasibility, recoverable data and unsupported-content reporting; convert into the adopted native format without changing the source | Unestimated; feasibility unknown |
| 10 | Verify supervised nightly signing, install, co-installation and update on the target platforms. Repeat web and localization checks and expand the original asset library | TBD after validation |

### Dependencies and possible rework

These tracks add discovery, migration, integration and repeated validation work. A possible
rework is not an approved replacement design, and an upstream proposal/merge is not proof of
TextureCraft adoption.

| Track | Current state | Completion / revalidation gate |
|---|---|---|
| Native `.tcraft` | The live baseline covered save/load. Main persists rebuilt mesh UV layouts. The future container/schema remains open | Decide the contract and compatibility policy. For any adopted migration, check layers, masks, effects, resources, painted tiles, UV layouts and baked data. Exercise bounded reads, atomic saves and recovery |
| Output packages | Export templates now support subfolders, and desktop USD/MaterialX companions reference exported textures | Define affected artifacts, manifests, naming and versioning. Check package relocation, applicable folder/archive delivery, color/normal/displacement metadata and destination tools |
| MaterialCraft `.mcmat` | The [v3 procedural ZIP contract](https://github.com/storytold/materialcraft/issues/190) and shared reader have landed. TextureCraft uses the desktop CLI for cooking and rejects v1/v2 with republish guidance | Validate supported producer and consumer versions with bundled assets, exposed controls and mesh-map inputs. Test archived sources after moving the authoring files, and saved baked maps in the browser |
| ArtCraft UI refactor | Incoming suite/UI work; shared palette/theme proposals such as [craft-libs #102](https://github.com/storytold/craft-libs/pull/102) and [#397](https://github.com/storytold/craft-libs/pull/397) are examples, not the whole refactor or an adoption claim | Scope TextureCraft's adapters/components and design changes. Keep document behavior command-driven; repeat the [UI task audit](docs/ui-parity.md), input/accessibility checks, screenshots and layout/preferences persistence tests against the adopted design |
| Shared libraries | Shared format adoption now includes glTF/GLB, COLLADA, Alembic, USD, runtime raster codecs and ZIP. [Cargo.toml](Cargo.toml) declares direct dependencies at six craft-libs revisions | Consolidate the pins under the repository's single-revision convention and rerun consumer compatibility, format limit, layering, wasm and regression checks. [Migration tracker](https://github.com/storytold/craft-libs/issues/778) |

### Current focus

Establish project-recovery, startup and bake-fidelity checks on representative assets. Scope
`.tcraft` and output-package decisions and ArtCraft UI adoption alongside those checks. Repeat
the passing source-build `.mcmat` handoff with supported release builds. Read-only `.spp` remains a separate
feasibility track. Hardware ray tracing for Rendering mode remains a later enhancement.

## Progress log

| Date | Entry |
|---|---|
| 2026-11-25 | Audited main at `3c026860`. Recorded the expanded generator catalog, mask and fill controls, Bake Mode, UV rebuilds, export migrations, the v3 material handoff and supervised nightlies. Raised estimated breadth from ~80% to ~90% and weighted readiness from ~60% to ~68% |
| 2026-11-16 | Added possible native/output-package reworks, unfinished MaterialCraft `.mcmat` integration and incoming ArtCraft UI/shared-library work as explicit roadmap tracks. Current audit scores stay tied to the tested revision; migration scope and effort remain unestimated |
| 2026-11-16 | Deeper UI/functionality audit added in [docs/ui-parity.md](docs/ui-parity.md): twenty check groups, specific missing controls, reference editing/self-bake, UI export/save, analytic high/low transfer and UDIM packing. Overall percentages remain provisional estimates |
| 2026-11-16 | Approximate live review against main `62b2b13` and reference 12.2.0. Basic edit/save/reopen, GPU bake and interchange smoke checks passed. Rebaselined reporting to ~80% breadth / ~60% readiness and recorded validation limits |
| 2026-11-16 | Scoped reference-project compatibility to future read-only `.spp`; retained `.tcraft` for editing/saving and open pipeline exports. No importer or other implementation change in this review |
| 2026-11-12 | v0.6.1 published; supersedes the former first-release v0.1.0 focus |
| 2026-11-04 | Previous roadmap reported ~94% weighted implementation/workflow completion; retained here as historical context, not a comparable readiness measurement |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-25 | major | Updated implementation status and provisional readiness for current main. Retained the November 16 scores for comparison and kept the beta acceptance gates |
| 2026-11-16 | major | Added dependency/rework scope and adoption gates; distinguished legacy `.mcmat` support from the unfinished MaterialX-based proposal; expanded upcoming work without crediting unlanded integrations |
| 2026-11-16 | major | Expanded live UI tasks and output validation; added a separate UI gap inventory with acceptance checks; distinguished UI, mixed and command evidence and CPU/GPU bake paths |
| 2026-11-16 | major | Adopted headline/audience/dimension/feature/language/upcoming tables; added main-revision evidence and uncertainty, read-only `.spp` scope, current release status and acceptance gates; removed the unsupported six-hour remainder |
