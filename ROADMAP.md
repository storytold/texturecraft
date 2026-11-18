# TextureCraft roadmap

Stage: alpha (readiness assessment) · next: beta, ~75% readiness plus scoped compatibility and integration gates; remaining effort unestimated

> Last reviewed: 2026-11-16 · Last updated: 2026-11-16 · Change: major (UI/functionality audit, possible format/package reworks, unfinished `.mcmat` integration and incoming ArtCraft UI/shared-library work) · Target: reference app 12.2.0

TextureCraft is a clean-room implementation of the professional texture-painting workflow. This
assessment covers **main at [62b2b13](https://github.com/storytold/texturecraft/commit/62b2b13fdd32740b6e118cd00c5e0d0c12b30815),
version 0.6.1**, compared with the installed reference app on 2026-11-16. It follows the
[PhotoCraft roadmap format](https://github.com/storytold/photocraft/blob/main/ROADMAP.md).
Detail: [UI gaps and functional task audit](docs/ui-parity.md), [features](docs/features.md), [menu checklist](docs/parity.md),
[architecture](docs/architecture.md), [automation](docs/control-protocol.md),
[localization](docs/localization-parity.md), [releasing](docs/releasing.md).

**Format direction:** `.tcraft` remains the native editable project format. Read-only opening/import
of `.spp` is a future compatibility goal: leave the source untouched and save converted work as
`.tcraft`. Supported versions, recoverable layers/resources and handling of unsupported content
still need investigation; no `.spp` reader is implemented. Writing `.spp`, round-tripping edits
back into it and compatibility with other proprietary asset/preset formats are outside this goal.
Pipeline exchange uses USD, glTF/GLB, MaterialX and texture maps.

The current `.tcraft` schema/container and output-package contracts may be reworked; their
present implementations are not a frozen design. MaterialCraft `.mcmat` import remains a goal
whose next format is unfinished upstream. ArtCraft UI refactoring and further shared-library
adoption also add integration and regression work. These dependencies are part of the remaining
roadmap, even where a current task already passes.

## Headline numbers

| | Value | Kind |
|---|---|---|
| Feature breadth | ~80% (75–85%) | Estimated across feature families; no exhaustive target inventory |
| Ready for real work | ~60% (50–70%) | Estimated readiness of the assessed revision; no completion credit for incoming reworks/integrations |
| Mainstream practitioner | ~60% (50–70%) | Estimated for regular asset texturing, baking, masking and pipeline delivery |
| Essentials user | ~75% (65–85%) | Estimated for a simple mesh, layered materials/paint and texture export |
| Desktop menu catalog | 59/59 live | Observed through the running app; this is TextureCraft's own catalog |
| Engine commands | 178 registered | Observed through the running app; registration does not prove every parameter path |
| Functional review | 20 scoped check groups | UI, mixed-surface and command cases [recorded separately](docs/ui-parity.md#functional-task-audit); not an exhaustive coverage score |
| Remaining to beta | Unestimated | Requires acceptance tasks, performance budgets and a decision on required format/package/UI migrations |
| Remaining to full scoped parity | Unestimated | Includes possible `.tcraft`/output reworks, upstream `.mcmat`, ArtCraft UI/shared-library adoption, `.spp` feasibility and workflow/fidelity work |

The former ~94% figure mostly counted implementation paths. This review also asks whether an
artist gets equivalent controls, predictable output, responsive interaction and dependable
delivery. The change is a measurement rebaseline and an expanded import goal, not a measured
loss of functionality. Percentages are rounded judgments; ranges express uncertainty, not
statistical confidence intervals. The old “~6 h + baking TBD” remainder is withdrawn: this
review provides no calibrated engineering or agent-hour forecast.

The percentages describe behavior at `62b2b13`, not how much of a settled delivery plan is
finished. Potential rework can revisit working code and require repeating the checks below.
Its scope and cost are not yet measured, so remaining effort cannot be calculated as
100 minus the readiness score. Incoming upstream work is not counted as landed functionality.

### Readiness by audience

| Audience | Ready % | Hours to ~95% | Work that dominates |
|---|---|---|---|
| Full scoped target | ~60% | Unestimated | Workflow depth, format/output-package decisions, `.mcmat`, ArtCraft UI/shared libraries, bake/display fidelity, color management, read-only `.spp` and platform validation |
| Mainstream practitioner | ~60% | Unestimated | Reliable masks/painting, MaterialCraft exchange once its contract is ready, project/package compatibility, larger projects and destination-app checks |
| Essentials user | ~75% | Unestimated | Discoverable controls, predictable startup/GPU selection and save/reopen reliability retained through format/UI migrations |

Why alpha. Mouse/keyboard tasks verified 3D/2D painting, undo/redo, material drag and drop, mask
and generator editing, UV rectangle fill, UI baking, native saving and texture export. Separate
command checks covered high-to-low transfer, UDIM sizes/packing and scene/material exports.
The interface is recognizably close in its painting, bake and export workspaces, but still lacks
mask-thumbnail effect menus, several fill controls and a MaterialX export option.
Generator depth is limited, bake equivalence is open,
and hardware/performance coverage is incomplete. There were **96 open issues** on the review date;
these include requests and older reports, not 96 reproduced defects. The published
[v0.6.1 release](https://github.com/storytold/texturecraft/releases/tag/v0.6.1) already exists;
“first release v0.1.0” is no longer an upcoming milestone.

Native/output contracts and the future MaterialCraft, ArtCraft UI and shared-library integrations
also remain unsettled. Passing a legacy reader or a standalone export check does not complete
those future workflows.

A proposed beta gate is ~75% readiness plus repeatable artist tasks on representative assets:
high/low baking and smart masks, precise 3D/2D painting, a multi-set UDIM project, save/reopen
without lost work, and exports checked in destination tools. Startup failures and data-loss
blockers on the supported test matrix must be resolved. This audit does not establish that gate.
Beta scope must identify which reworks/integrations it requires and which remain later goals.
Every adopted project/package version needs a compatibility policy and regression checks;
MaterialCraft exchange must be validated against a named producer/consumer version pair before
it is advertised as ready.

## By dimension

All percentages below are estimates. The weights sum to 100; their additive result is 59.1%,
rounded to ~60%. Hardware, performance and stability scores include confidence discounts for
incomplete validation; they are not measured success rates or speed comparisons.

| Dimension | % ready | Weight | Hours to full | Evidence / remaining work |
|---|---:|---:|---|---|
| Features (depth) | ~65% | 30 | TBD | Core editing tasks and high/low/UDIM outputs exercised; generators, properties and specialized workflows need depth. Mask anchors landed after this assessment; live UI validation remains |
| UI/UX fidelity | ~65% | 20 | TBD | Current editing/bake/export UI checked; [gaps](docs/ui-parity.md), ArtCraft UI adoption and revalidation after refactoring remain |
| File formats / interchange | ~60% | 10 | TBD | Current native save and basic exports exercised; possible `.tcraft`/output-package reworks, unfinished upstream `.mcmat`, `.spp` and consumer validation remain |
| Hardware (GPU, pen, displays) | ~50% | 8 | TBD | One Windows GPU exercised; pen tilt, pressure across platforms and display coverage remain incomplete |
| Localization | ~35% | 4 | TBD | Ten picker languages in source; optional shared catalogs/fonts and native-language review need current measurement |
| Performance | ~40% | 10 | TBD | Dirty-tile/budgeted architecture and benchmark code exist; no matched performance budgets or large-project results from this review |
| Stability | ~55% | 10 | TBD | Small save/reopen and undo/redo passed; recovery, large projects and long sessions need coverage, including after format/library migrations |
| Platforms | ~65% | 3 | TBD | Desktop/web build and packaging paths exist; this review ran Windows only |
| Automation / ecosystem | ~75% | 5 | TBD | 178 commands and CLI/control/MCP architecture; shared-library adoption, command/UI compatibility and the future MaterialCraft contract need integration checks |

## Features

These are approximate depth/readiness estimates, not percentages of matching pixels or completed
menu entries. Feature rows overlap and are not an additional additive score.

| Area | % ready | Hours | What works / main gaps |
|---|---:|---|---|
| Projects and files | ~80% | TBD | Current `.tcraft` new/save/reopen passed with layers, effects and baked maps retained. Schema/container rework remains possible; versioning/migration policy and renewed persistence/recovery tests are needed if adopted. Large cases, Alembic HDF5 and animation remain open; PBR folder/ZIP/selected-file import landed after this assessment: `file.importPbrFolder` creates one fill layer from named maps, including packed ORM and OpenGL/DirectX normals, with a UI dialog and optional native library copy; it is not credited in this score |
| Texture sets and UDIM | ~80% | TBD | Two tiles exported at independent 128/256 px sizes with expected packed-channel values; 21 channel kinds in source. Many-set/many-tile production coverage remains untested |
| Layers, masks and effects | ~75% | TBD | UI fill/black-mask creation, generator editing/duplication and native persistence passed at the assessed revision. Mask-thumbnail context actions remain missing; effect renaming and mask-stack anchors landed afterward, with live UI validation still open |
| Painting and precise 2D work | ~70% | TBD | Mouse painting in both views, keyboard undo/redo, view shortcuts, brush gesture and UV-island rectangle fill passed. Nine tool kinds exist; brush feel, pressure/tilt, other tool task suites and complete trim sheets need validation |
| Generators and smart masks | ~45% | TBD | Seven generator kinds and 24 smart-mask presets; Position/global-balance editing worked in the UI. Shared adjustments, resolution and input slots exist; richer generators, micro-detail workflows and saving user smart masks remain incomplete |
| Materials and asset library | ~60% | TBD | 72 procedural materials and 21 smart materials; shelf search/drop worked. Legacy MaterialCraft paths exist, but the planned `.mcmat` contract is unfinished upstream. Library variety, previews and cross-app integration remain work |
| MaterialCraft `.mcmat` integration | Incomplete; upstream-dependent | TBD after contract | Current reader accepts legacy v1/v2. The proposed MaterialX-based v3 still needs upstream completion, shared-library support, TextureCraft mapping/UI and tests with real producer outputs; this is not a completed material exchange workflow |
| Baking | ~60% | TBD | All ten maps completed through the UI on CPU at 256 px and through commands on GPU at 128 px. Separate high/low normal/height transfer produced varying maps; cross-app pixel equivalence, larger assets and resolution-stable wear remain unproven |
| Viewport, shading and rendering | ~70% | TBD | PBR/IBL, extra shading lobes, 3D/2D views, displacement and CPU/GPU path-tracing paths. Matching material/display behavior, larger scenes and hardware ray tracing remain open |
| Color management | ~35% | TBD | Linear/sRGB handling, fixed export spaces and profile metadata; display model has Linear/ACES-filmic tone mapping. Configurable OCIO project/display workflows are absent |
| Texture and scene export | ~75% | TBD | Current PNG/UDIM/USD/GLB/MaterialX checks passed within scope. Output-package layout/content may be reworked; package contracts and consumers need separate validation. MaterialX UI output, complex materials and larger UDIM assets remain incomplete |
| Read-only `.spp` compatibility | 0% | TBD after feasibility study | Planned; no reader, version matrix or import-fidelity corpus. `.tcraft` remains the save format |
| Workspace, panels and preferences | ~65% | TBD | Current views/layouts checked; narrow labels truncate. Incoming ArtCraft UI refactoring/shared components can change these surfaces; task, shortcut, accessibility and layout/preferences persistence checks must be repeated |
| Automation | ~80% | TBD | Command inspection and scripted editing/export work; UI, CLI and MCP surfaces share engine commands. This review exercised the control channel and CLI, not every MCP/tool path |

Source checks matter here: [generator kinds](crates/core/src/layer.rs) number seven even though
their shared controls have grown; [polygon fill](crates/engine/src/fill_mode.rs) already includes
triangle, polygon, UV-island and mesh modes. Current [MaterialCraft import](crates/formats/src/mcmat.rs)
accepts v2 and recognizes the newer source archive layout. That is legacy compatibility, not
completion of the next `.mcmat` integration. The [upstream v3 proposal](https://github.com/storytold/materialcraft/issues/190)
describes MaterialX plus resources in equivalent folder/ZIP deliveries and excludes native source.
Its proposed exposed controls differ from re-cooking an embedded authoring graph. The final
supported subset, version policy and authoring/re-cook workflow must be agreed and tested;
the legacy promise must not be carried forward automatically.

The [UI gap inventory](docs/ui-parity.md#missing-controls-and-workflows) records reproduction
routes and acceptance checks: mask context/save/export actions; independent U/V tiling,
filtering/wrap, physical-size and warp/per-tile projections; MaterialX UI output;
and configurable color-management settings. Generator depth and narrow-layout readability are
tracked as partial areas. Direct hardness/angle controls and polygon/UV rectangle fill already
exist; older reports must not turn them into fresh missing-feature claims.

### Evidence from this review

| Check | Result and limit |
|---|---|
| Baseline/build | Fetched `origin/main`; HEAD matched `62b2b13`. `cargo build -p texturecraft -p texturecraft-cli --profile verify` passed; binary reported 0.6.1 and that revision |
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

## Languages

The [picker](crates/ui-egui/src/i18n.rs) contains ten languages. Shared catalogs are optional at
build time; missing entries fall back to English, and builds without catalogs offer English only.
The older [coverage table](docs/localization-parity.md) predates catalog migration, so its “100%”
rows are not a current measurement. This audit ran an English build without external catalogs.

| Language | Code | UI strings | Status |
|---|---|---|---|
| English | `en` | Source language | Exercised in this audit |
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
| 1 | Reliable startup and project safety across the supported GPU/platform matrix; reproduce startup/GPU reports and exercise recovery/large saves ([#541](https://github.com/storytold/texturecraft/issues/541), [#489](https://github.com/storytold/texturecraft/issues/489)) | TBD after reproduction |
| 2 | Define `.tcraft` and output-package direction, scope any reworks, and establish versioning, migration and producer/consumer acceptance cases | Unestimated; design scope open |
| 3 | Plan/adopt the incoming ArtCraft UI refactor and shared-library changes; preserve command reachability and repeat UI/persistence/platform checks after adoption | TBD after upstream contracts and consumer scope |
| 4 | Bring in the completed MaterialCraft `.mcmat` contract: agree the supported MaterialX/resources/controls subset, implement the consumer through shared APIs and validate folder/archive outputs from named versions | Upstream-dependent; format unfinished |
| 5 | Bake fidelity and dependable smart-material wear: matched assets/settings at multiple resolutions, all ten maps, high/low pairs and documented differences ([#536](https://github.com/storytold/texturecraft/issues/536), [#538](https://github.com/storytold/texturecraft/issues/538)) | TBD after map corpus |
| 6 | Generator, anchor and property depth; finish the artist-facing control inventory while preserving existing shared controls ([#374](https://github.com/storytold/texturecraft/issues/374), [#389](https://github.com/storytold/texturecraft/issues/389), [#433](https://github.com/storytold/texturecraft/issues/433)) | TBD after control audit |
| 7 | Brush/pen precision, feedback and discoverability; complete 3D/2D/trim-sheet tasks and set responsiveness budgets ([#545](https://github.com/storytold/texturecraft/issues/545), [#485](https://github.com/storytold/texturecraft/issues/485), [#306](https://github.com/storytold/texturecraft/issues/306)) | TBD after task tests |
| 8 | Color-managed pipeline and destination checks for USD, glTF/GLB, MaterialX and texture maps, including UDIM, complex materials and the adopted output packages | TBD after consumer matrix |
| 9 | Read-only `.spp` discovery: clean-room feasibility, recoverable data and unsupported-content reporting; convert into the adopted native format without changing the source | Unestimated; feasibility unknown |
| 10 | Release/platform regression checks, web polish, current translation coverage and native review; expand the original asset library | TBD after validation |

### Dependencies and possible rework

These tracks add discovery, migration, integration and repeated validation work. A possible
rework is not an approved replacement design, and an upstream proposal/merge is not proof of
TextureCraft adoption.

| Track | Current state | Completion / revalidation gate |
|---|---|---|
| Native `.tcraft` | Current save/load works in the audit; schema/container may change | Decide the native contract and compatibility policy. For any adopted rework, check existing projects and conversion explicitly: layers, masks, effects, resources, painted tiles, UDIM layouts and baked data; retain bounded reads, atomic saves and recovery behavior |
| Output packages | Current individual exports work within the recorded scope; future package format/content/layout is not settled | Define affected artifacts, manifests, naming, resource references and versioning. Test package relocation, folder/archive delivery where applicable, color/normal/displacement metadata and real consumers; repeat install/update checks if distribution artifacts change |
| MaterialCraft `.mcmat` | Legacy v1/v2 support exists; [v3](https://github.com/storytold/materialcraft/issues/190) is unfinished upstream | Agree the published contract and supported MaterialX subset, folder/ZIP equivalence, resources and exposed inputs. Adopt it through shared libraries and test actual MaterialCraft output in TextureCraft, including unsupported versions/features. Do not assume native-source recovery or the legacy re-cook behavior |
| ArtCraft UI refactor | Incoming suite/UI work; shared palette/theme proposals such as [craft-libs #102](https://github.com/storytold/craft-libs/pull/102) and [#397](https://github.com/storytold/craft-libs/pull/397) are examples, not the whole refactor or an adoption claim | Scope TextureCraft's adapters/components and design changes. Keep document behavior command-driven; repeat the [UI task audit](docs/ui-parity.md), input/accessibility checks, screenshots and layout/preferences persistence tests against the adopted design |
| Shared libraries | Several shared crates are already pinned in [Cargo.toml](Cargo.toml); [format migration work](https://github.com/storytold/craft-libs/issues/778) continues | Use reviewed public APIs and keep app-specific mapping/policy local. Each adoption needs consumer integration plus compatibility, security-limit, layering, wasm and regression checks; a library change alone does not complete the app's work |

### Current focus

Keep the current functional baseline while establishing project-safety and bake-fidelity checks.
Scope `.tcraft`/output-package changes and coordinate ArtCraft UI/shared-library adoption alongside
that work, so feature work budgets include possible migration and revalidation. Bring `.mcmat`
forward against the finished upstream contract rather than treating the legacy reader as completion.
Run `.spp` feasibility separately. Hardware ray tracing is a later enhancement; beta depends on
the scoped acceptance/integration gates above. [Release validation](docs/releasing.md) continues
for existing releases; there is no outstanding “first release” gate.

## Progress log

| Date | Entry |
|---|---|
| 2026-11-16 | Added possible native/output-package reworks, unfinished MaterialCraft `.mcmat` integration and incoming ArtCraft UI/shared-library work as explicit roadmap tracks. Current audit scores stay tied to the tested revision; migration scope and effort remain unestimated |
| 2026-11-16 | Deeper UI/functionality audit added in [docs/ui-parity.md](docs/ui-parity.md): twenty check groups, specific missing controls, reference editing/self-bake, UI export/save, analytic high/low transfer and UDIM packing. Overall percentages remain provisional estimates |
| 2026-11-16 | Approximate live review against main `62b2b13` and reference 12.2.0. Basic edit/save/reopen, GPU bake and interchange smoke checks passed. Rebaselined reporting to ~80% breadth / ~60% readiness and recorded validation limits |
| 2026-11-16 | Scoped reference-project compatibility to future read-only `.spp`; retained `.tcraft` for editing/saving and open pipeline exports. No importer or other implementation change in this review |
| 2026-11-12 | v0.6.1 published; supersedes the former first-release v0.1.0 focus |
| 2026-11-04 | Previous roadmap reported ~94% weighted implementation/workflow completion; retained here as historical context, not a comparable readiness measurement |

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-11-16 | major | Added dependency/rework scope and adoption gates; distinguished legacy `.mcmat` support from the unfinished MaterialX-based proposal; expanded upcoming work without crediting unlanded integrations |
| 2026-11-16 | major | Expanded live UI tasks and output validation; added a separate UI gap inventory with acceptance checks; distinguished UI, mixed and command evidence and CPU/GPU bake paths |
| 2026-11-16 | major | Adopted headline/audience/dimension/feature/language/upcoming tables; added main-revision evidence and uncertainty, read-only `.spp` scope, current release status and acceptance gates; removed the unsupported six-hour remainder |
