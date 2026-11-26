# Project import

This page is for contributors and their agents: how the one-way import of the reference app's projects works, where
its measured rules live, and how imported graphs are evaluated. The shared reader, graph runtime and GPU backend
are `craft-spp-reader`, `craft-material-graph` and `craft-material-graph-wgpu` in craft-libs. `import` (L2) and
`import-verify` contain the TextureCraft conversion. `engine::import_cmds` and `engine::import_replay` connect it to the app.

## The reader

Reads projects of the reference app into plain Rust data: the file container, the reference app's records and its
layer model, and decompiles the compiled graphs a project embeds into graph text for TextureCraft's graph runtime. It
knows no application: the importer is built on it, and any other painting app can build its own importer on the same
reader.

Its dependencies are small pure-Rust decoders: `lz4_flex` (compressed records), `sevenz-rust2` (graph packages, which are 7z archives),
`hayro-jpeg2000` (version 12.2's image resources) and `zune-jpeg` (JPEGs inside compiled graphs). The crate contains no
code, data or resources of the reference app: everything is read from the user's own files at run time, from what those files
contain.

### What it reads

| Module | What |
|---|---|
| `container` | The generic formats: HDF5 (the project file itself), Qt QDataStream, byte cursors. Knows nothing of the reference app. |
| `decode` | The reference app's records: project settings, the resource table and images, the texture cache (the flattened channels the reference app saved), the mesh scene (`scene.assbin`, decoded into `SceneMesh`), mesh metadata and normalisation, cameras, and `document.bin` as a raw object tree. |
| `model` | The reference app's concepts built from `document.bin`: texture sets, channels, the layer tree (fill, paint, folder), per-channel blending, masks, effects (fills, filters, generators, levels, colour selection, anchors, polygon fills), sources (uniform, bitmap, procedural, reference), brushes and strokes, bake settings. Unknown classes are kept raw and counted, never dropped. |
| `project` | One opened project file with typed access to its parts. |
| `decompile` | A compiled graph to graph text (graph format 3 of the graph runtime): one node per compiled node, function subgraphs for the expressions, pattern programs for the fx-maps, stored images as embedded PNGs. Text only: the crate does not depend on the runtime, and the code-to-name tables of the compiled format live here and nowhere else. |
| `inspect` | Human-readable reports about a project, its document and its compiled graphs. |
| `blockers` | The shared vocabulary for what an importer can't evaluate yet (see below). |

### Example

Run this command in the craft-libs workspace.

```sh
cargo run -p craft-spp-reader --features dev-tools --example layers -- path/to/project-file
```

```rust
use craft_spp_reader::decode::assbin::AssbinGeometry;
use craft_spp_reader::project::{ProjectFile, paths};

let project = ProjectFile::open(path)?;          // or ProjectFile::from_bytes(bytes)?
let mesh = AssbinGeometry::parse(&project.read(paths::SCENE)?)?; // world-space parts, glTF UVs
let (document, report) = project.model()?;                        // texture sets and their layer trees
for set in &document.texture_sets {
    println!("{}: {} layers", set.name, set.layers.len());
}
```

See `examples/layers.rs` for the whole program. `examples/import_inspect` is the developer tool
(`tree`, `doc`, `model`, `compiled-graphs`, `decompile`).

### Reading from bytes

`ProjectFile::from_bytes(Vec<u8>)` opens a project already in memory, `ProjectFile::from_source` one served by any
`container::hdf5::ReadAt` (`MemorySource` is the in-memory one). Opening from a path (`ProjectFile::open`,
`ProjectReport::build`) exists only off wasm (`cfg(not(target_arch = "wasm32"))`); the web build reads bytes. Graph packages are read from bytes in any case (`decode::graph_package::GraphPackage::read`).

### Blockers: a shared vocabulary

Projects are full of compiled graphs (generators, procedural fills, filters), brush alphas and
brush types that an importer can't evaluate until someone writes an equivalent. `blockers` gives each one a stable
key, so every importer, report and corpus census names the same thing the same way:

| Key | What blocks |
|---|---|
| `graph:<graph>` | a compiled graph, e.g. `graph:mg_dirt` |
| `brush-alpha:<name>` | strokes painted with a brush alpha the importer doesn't have |
| `brush:<what>` | strokes the importer's brush can't paint (particles, 3D paths, stencils, projection …) |
| `strokes:<what>` | strokes that were not replayed |
| `feature:<id>` | a project feature the importer doesn't model yet |
| `class:<class>` | a layer, effect or source class the reader doesn't know |

What an importer can *do* about a key (replace it, replay it, or keep the layer locked) is the importer's own
business; our answer is its support catalog (`crates/import/src/translate/catalog.rs`).

### File formats

`document.bin` comes in two layouts, picked by the header: tagged (up to format 78, 2023) and compact (format 92, the reference app
12.2; see `decode/document/compact.rs`). Version 12.2 also stores image resources as tiled JPEG 2000, decoded with the
pure-Rust `hayro-jpeg2000`.

### Robustness

The reader parses untrusted files. Malformed input is reported as an error (`ProjectError`, `DecodeError`); unknown
classes, fields and newer formats are kept raw and reported rather than guessed. The never-panic tests
(`src/robustness_tests.rs`) feed truncated, corrupted and random bytes into every public entry point.

### Tests

From the craft-libs workspace, `cargo test -p craft-spp-reader` runs everything that needs no project files. The corpus tests run over real projects
when given them, and are ignored otherwise: `TC_IMPORT_CORPUS=<folders or project files> cargo test --release -p
craft-spp-reader -- --ignored`. Nothing read from a project is ever committed. `tests/fixtures/small.h5` is a
generic HDF5 file written by `tests/fixtures/make_fixture.c` (libhdf5) from a byte pattern; it holds no project data.

### Licence

MIT OR Apache-2.0, like the rest of the workspace.

## Compositing imported projects

Imported fills can carry a placed projection (`core::PlacedProjection`, evaluated by `compose::placed`):
a volume placed, turned and sized in the scene's bounding box, the image's placement in it, repetition per image
axis, the tri-planar blend curve, depth and facing falloffs, cylinder coverage, a Bézier warp grid, and a per-texel
anisotropic prefilter of graph outputs; TextureCraft's own projections are left as they are.

A project's compositing rules (`core::rules::CompositeRules`, `Project::rules`) default to TextureCraft's own and
are then left out of the project file. An imported project switches on the rules measured from the reference app's
renders: measured blend formulas, the blended value clamped to 0..1 before the opacity on channels that store 0..1, a
layer's mode applying only where something is below (the stack carries coverage), folders that do not pass through
composited on their own, and the exported normal derived from height with the document's filter (Sobel or backward
differences) at the measured slope, laid onto the baked normal before the normal channel. Such a project is
composited without the stack cache below the edited layer (it keeps no coverage).

## Locked layers and imported projects

A layer can carry a `core::Lock` (a source and reasons): its content stands for something TextureCraft
cannot compute or edit yet, such as a layer converted from another application's project. Every
`Session::edit` compares the document before and after (`engine::lock_cmds`) and refuses an edit that
changed a locked layer's content (fill, pixels, mask, effects, height or geometry mask); painting is
refused before the stroke starts. Visibility, name, color tag, position, deletion, channel blending
and opacity and the mask switch stay editable, and `layer.unlock` removes the lock as an undoable step.
Placeholder effects (`EffectKind::Placeholder`) stand for imported effects that cannot be computed:
they composite as identity and cannot be edited. A converted project keeps a `core::ImportReport`
(fidelity per item and per layer, safety-net flags, layer roles such as the original's stored
result), shown by `project.inspect` with every lock and placeholder.

`file.importProject` (`engine::import_cmds`, CLI `texturecraft-cli import`) converts a project file of the
reference app one way: `craft-spp-reader` reads it, `import` maps it onto the model (layers, folders, fills,
masks, levels, blend modes, geometry masks, baked maps, bake settings) and composites every texture set
with `compose` to compare it with the file's stored result. The file's compiled graphs are decompiled
(`craft-spp-reader`) and run by the graph runtime (`craft-material-graph`): procedural and material fills become graph
sources (`Source::Graph`), mask generators and filters graph effects (`EffectKind::Graph`), each a `GraphRef` naming
its graph text by key with the file's values and input bindings. A use that cannot be fed stays a placeholder locked
with the key `graph:<name>` (or its input's blocker); `graphs: "keep"` keeps every use a placeholder. Brush strokes are replayed with TextureCraft's own brush: the translator (L2,
without `paint`) rasterises nothing but hands replay jobs (stroke records, the brush mapped with the measured rules,
tip or a dynamic alpha's stamps, stencil, camera, symmetry, seed, and the layer, content effect or mask effect painted) to a `StrokeReplayer`, which the
engine implements with `paint` (`engine::import_replay`) and passes in through the import's hooks; the import calls
it per texture set before the safety net measures. The brush draws each stroke's jitter from its seed as the
file's app does, and lays strokes through their stencil on the view; path strokes (curves on the mesh) are walked by their
length in the mesh's space (`paint::path::SpaceCurve`), and ribbon brushes lay their tip along them as a band
(`paint::ribbon`); particle brushes lay the dabs of a small simulation per stroke (`paint::particles::simulate`,
the receiver's numbers read by `import::replay::particles`). Brushes painting an image or a compiled graph paint it per texel (the graph a fill of it runs,
rendered before the replay with the import's cooks; `paint::commit::commit_layer_texels`), read along paths and ribbons
at each texel's place on the band. Replayed strokes become approximated pixels, not locked; strokes
the brush cannot replay yet (smudge or projection along path strokes, graph tips the project does not hold, an eraser above other content effects) stay
placeholders locked with a `brush:<what>` or `brush-alpha:<graph>` key, and with replay switched off with
`strokes:<tool>`. The
stored result itself becomes a locked "Original result" paint layer, hidden as a reference unless the
import is asked to show it; asked for a correction (`originalCorrection`), the import also puts a locked "Correction to
original" paint layer on top of a set's layers that holds the stored result only on the texels the layers miss. Polygon
fills in masks are kept by the file as hits (clicks and rectangles with their camera), not triangles: the import resolves
them on the set's mesh (`import::polygon_fill`) and lays the picked triangles into a mask paint effect, as TextureCraft's
polygon fill tool does. The mesh is kept as binary glTF; nothing else of the original file is kept. User channels 0–7 map
onto TextureCraft's user channels with their names (later ones, which the file numbers past the
eighth, are reported). A set the original paints per UV tile imports as one texture over UV 0..1 with a
note in the report: where the file keeps each tile's pixels has not been seen yet.

The import runs as an `engine::import_cmds::ImportTask` reporting to an `ImportProgress`: a fraction, the step (reading
the file; per texture set converting, replaying strokes, and cooking graphs; saving; and preparing the view) and a
cancel flag. The first cook prepares each stack's graph inputs before the viewport uses them. CPU sets are composited
on the worker and GPU sets go straight to the viewport. Offline import keeps the original-result comparison enabled
by default. The import window shows the latest graph or node activity. Native builds also record the phases and slow
nodes in the local performance log. Inside a long step the bar moves by the work done: layers
translated, strokes laid, graphs cooked (a running render by its nodes) against the graphs the stack runs, tiles
composited. The flag is checked
between steps, texture sets and strokes, and the imported project's graph store renders under it, so a graph render in
progress stops at its next node. The command runs the task in one go; the desktop app (File › Import project..., or
opening or dropping such a file) shows exactly two windows (`ui-egui::import_ui`): a notice that must be acknowledged,
then the progress (bar, step, elapsed time, Cancel) with the task on a worker thread, which opens the converted project
or turns into the error view (one sentence, "Copy details" with the version, step and error, the issue link, Close).
Called through the control channel or MCP, the command opens no window.

## Graph sources and cooked results

`compose::graphs` evaluates graph references. A document's `GraphStore` holds the graph texts by key and the cooks:
one render of a graph for one texture set, every output it keeps as 16-bit tiles. A cook's key covers the graph text,
the evaluator versions of the node kinds it uses, its values, seed and inputs (baked maps and images by content,
nested graphs the same way, a filter's input pixels) and the render side, but not the compiler (renders are the same
bits everywhere), so a stored cook stays valid across builds. Graphs reading nothing of the stack are cooked before the
tiles composite and sampled like images; graphs reading the stack below or an anchor run as whole-set neighbourhood
fields. Renders run on the graph runtime's own thread pool, and no compositor thread waits for a render held further
down its own stack (`compose::graphs::once`). The `.tcraft` file stores the texts (`graphs/<key>.txt`) and the cooks
the last composite read (`cooked/`), so a converted project opens without rendering its graphs; a use whose key no
longer matches is rendered again alone, and one that cannot be keeps its last pixels, flagged in `project.inspect`.

An import also keeps the stroke records behind the pixels it replayed (`core::strokes`, one `strokes/<id>.bin` entry
per paint layer or paint effect, written by `formats::project_file::strokes`): each replay job as the replayer reads it
(pen points, camera, seed, symmetry, the mapped brush and tip, a particle brush's simulation settings, the values laid), so `Session::replay_stroke_record`
lays the strokes again from the saved project alone, to the same pixels. Images a brush paints are not kept (that step
cannot be laid again); graphs it paints are kept by reference, their texts saved with the project. A record that cannot
be read stays as its bytes and is saved unchanged; older builds skip the entries.

Native interactive imports reuse graph renders from the user's cache directory. The command and CLI can use a chosen
folder with `resultCache` and `--result-cache`. The graph store looks up each cook by key before rendering and keeps the
completed result. Entries are checked by key, length and checksum. The default limit is 20 GiB, with least recently used
entries evicted first. A second import reuses renders when the graph, inputs and resolution match, including across
projects.

`import.library.list` and `import.library.add` (`engine::import_library_cmds`, over `import::library`) offer what
a project file brings for the user's asset library (brushes, bitmap alphas, stencils, textures, the environment
map) with its origin and whether it may be shared; brushes are added as `.tcbrush` presets in the data folder's
`brushes/`. The other kinds have no place in the library yet and are only listed.

## Commands and automation

Every action is a `CommandSpec` in `engine::commands` (id, label, menu path, shortcut, params doc,
enabled predicate, run). The menus, shortcuts, the desktop app's control channel
(`--control <port>`), the CLI and the MCP server all execute commands by id, so an agent can do
anything a user can and verify it through `project.inspect`. See
[`control-protocol.md`](control-protocol.md).

## Rule tables

### Graph engine (`crates/material-graph/tests/rules/` in craft-libs)

Measured rules of the compiled format's engine, as plain numbers: inputs and expected outputs, one table per rule. The
tables don't depend on our types, so another implementation can run the same data against its own code. Each table
names the measurement the rule comes from; the expected values are what our code computes, which that
section shows to match the reference app to the stated error. The reader and translator have their own tables in
`crates/import/tests/rules/`, in the same format.

#### Format

Tab-separated text. Lines starting with `#` are comments: the rule in words, its source, the tolerance and what each
column means. The first other line names the columns; each line after it is one case. Numbers are decimal (as f32 or
f64 prints them, read back as that type) or `0x` hex; a few columns hold words (`generator`, `none`) or lists of whole
numbers separated by spaces.

#### Tables

| Table | Rows | Rule | Measurement | Runner |
|---|---:|---|---|---|
| `node_hold.tsv` | 65 | What a node holding 8 bits, 16 bits or half floats keeps of a value | "Node formats"; "What nodes hold and how they read" | `rule_table_node_hold` |
| `bilinear_read.tsv` | 60 | A bilinear read with its weight in 1/256ths, rounded half up to a 16-bit step | "Transforms"; "What nodes hold and how they read" | `rule_table_bilinear_read` |
| `box_blur_plan.tsv` | 66 | A box along a tiling row on an 8- or 16-bit node: end reads, pyramid of held pairs, the node's rounding | "What nodes hold and how they read" ("Box blur along an axis on an integer node") | `rule_table_box_blur_plan` |
| `lcg_draws.tsv` | 118 | Draws of every 24-bit generator of the compiled format (pixel and value processors, parameters, fx-map sites, iterations, quadrant depths) | "Random numbers: solved, from rendered images only" | `rule_table_lcg_draws` |
| `pixel_state.tsv` | 24 | A pixel's start state: one-at-a-time hash of the seed and the bits of the pixel centre, and its first draw | same | `rule_table_pixel_state` |
| `fx_child_state.tsv` | 28 | A fx-map child's state from its parent's: child k takes k + 1 steps of its depth's generator | same | `rule_table_fx_child_state` |
| `pattern_shape.tsv` | 119 | The 17 built-in fx-map patterns at points of their box | "Fx-map drawing" ("Patterns (V)"); the gaussian: "One-effect projects" › "Node rules" | `rule_table_pattern_shape` |

From the craft-libs workspace, run `cargo test -p craft-material-graph --lib rule_table_`.

#### Where ours differs from the rule

`bilinear_read.tsv`: ours reads in single precision and rounds an exact tie (`(a (256 - w) + b w) mod 256 = 128`) down,
where the measured rule rounds it up; 3 of its 60 rows are such ties. The table holds both (`ours_16`, `rule_16`); the
runner checks that ours differs only there. Not changed here, since this work is a pure refactor.

### Importer (`crates/import/tests/rules/`)

Measured rules the importer relies on, as plain numbers: inputs and expected outputs, one table per rule. The tables
don't depend on our types, so another implementation can run the same data against its own code. Each table names the
measurement the rule comes from; the expected values are what our code computes, which that section shows to match the
reference app to the stated error. The graph engine has its own tables in `crates/material-graph/tests/rules/` in craft-libs, in the same
format.

#### Format

Tab-separated text. Lines starting with `#` are comments: the rule in words, its source, the tolerance and what each
column means. The first other line names the columns; each line after it is one case. Numbers are decimal (as f32
prints them); a few columns hold words (`name`, `fit`).

#### Tables

| Table | Rows | Rule | Runner |
|---|---:|---|---|
| `blend_gray.tsv` | 288 | Layer blend by stored blend id on a one-value channel: formula, strength, clamp on 0..1 channels, no clamp on float | `rule_table_blend_gray` |
| `blend_color.tsv` | 128 | The same on RGB (HSV modes with doubled value) and the normal channel (normal-map modes weaken the top first) | `rule_table_blend_color` |
| `blend_coverage.tsv` | 256 | Blend over partial coverage below, over an empty value ("mode only where something is below") | `rule_table_blend_coverage` |
| `mask_group.tsv` | 80 | A mask group's layer blended with the mask below: `b + (clamp(f(b, v)) - b) w a` | `rule_table_mask_group` |

The stored blend id goes through the import's blend table (`crates/import/src/blend.rs`) and is computed with
TextureCraft's measured blend rules (`texturecraft_core::rules`, which an imported project switches on). Runners are in
`crates/import/src/rule_tables.rs`. Run them all with `cargo test -p texturecraft-import --lib rule_table_`.

## Large projects: known limits and where to look

Imported projects are ordinary TextureCraft projects, but they are often far larger than what TextureCraft has been
used with (several texture sets at 4096², dozens of layers, many anchors and graph filters reading the stack). On such
projects editing works but is slow: toggling a layer takes close to a minute where the reference app needs seconds. This is a
property of TextureCraft's compositor, not of the import, and fixing it is general optimization work any large
project will need. What we found, for whoever picks it up:

- **The compositor recomputes the whole stack on most edits.** It runs on the CPU, tile by tile (`compose::eval`),
  and keeps no result per layer. Every edit marks the whole texture set dirty (`engine/src/session.rs`, the
  `update_for` path), so a visibility toggle composites every layer for every texel again.
- **The one stack cache only serves the selected layer.** `compose/src/below.rs` (`BelowCache`, `StackKey`) keeps the
  composite of the layers below the layer being edited; it speeds up painting and slider drags on that layer, not a
  toggle elsewhere (it is built around the selection in `engine/src/session.rs` and nothing fills it ahead).
- **Imported projects don't get that cache at all yet.** They switch on the measured compositing rules
  (`core::rules::CompositeRules`), and the cache keeps no coverage, which those rules need (a layer's mode applies
  only where something is below). The running stack already carries coverage (`Acc.cov` in `compose/src/eval.rs`);
  `store_below`/`load_below` would have to keep it, `eval_split` would have to call `group_start` when it enters a
  folder, `StackKey` would have to include the rules, and the gate in `below.rs` that turns the cache off for them
  would go. The bit-exact cached-vs-full test (`compose/tests/cache_and_instances.rs`, `check_cached`) should then run
  under both rule sets.
- **Anchors, effect layers and filters reading the stack turn the cache off by themselves**, in native projects
  too (`below.rs`, the checks next to the rules gate). A large imported project has many of them. The cache could
  stay on unless such a layer actually reads the edited layer.
- **Filters reading the stack run as whole-set fields** (`compose/src/field_cache.rs`, `compose::graphs`); painting
  bypasses the field cache, and the mask view and the Levels histogram recompute them on the UI thread.
- **Reopening recomposites everything.** A saved project stores its layers, graph texts and graph renders, but not
  the composited result, so the first view after opening composites the whole stack again (about 1.5 minutes on the
  largest test project; the UI blocks on it when clicked). Storing a composite per texture set in the project file and showing it
  until the first recomposite would make large projects open as fast as the reference app does (a file format change).
- **Graph results made while editing are never freed** (`compose/src/graphs/mod.rs`), and `GraphStore::prune` is
  never called, so results of deleted layers stay in the project file.

- **The graph runtime itself is slower than it could be.** Measured against the latest PiJellyPin/MaterialCraft
  (15 Nov) on the same graph text at 2048², ours is about 1.3–1.9× slower on the CPU, mostly thread-scheduling
  overhead and the deliberately exact maths library (`libm`, for bit-identical renders everywhere); its GPU path is
  about 10× faster again but not bit-identical. Our grey/16-bit storage needs about half the memory on large graphs.
  The runtime and wgpu backend are shared craft-libs crates that both apps can use.

The direction that would get closer to the reference app: keep each layer's result (and recompute only layers whose
inputs changed, following anchors and stack readers), so a toggle re-blends cached results instead of recompositing;
later, composite on the GPU. The second touches the "CPU compositor is the reference" principle and is a decision for
the maintainers.

Other gaps between imported and native editing, all in the UI: locks and placeholder reasons are not shown (an edit
fails with a toast; nothing in the UI unlocks), graph fills and effects have no parameter editor (`effect.set`
ignores them), placed projections show as an empty "UV projection" panel, and there is no window for the import
report. The engine already treats imported layers as ordinary layers; these need UI.

The viewport derives normal detail from height with the project's rule (filter and slope), as export does; two
smaller differences remain: the viewport combines height with the baked and painted normal in one step where export
lays height onto the baked normal first, and the CPU path tracer (`render/src/trace.rs`) still uses the native slope.

## Graph text format

The `craft-material-graph` crate documents the `.tcgraph` format in `crates/material-graph/docs/format.md`
in the craft-libs workspace. Run `texturecraft-cli graph <file> <out dir>` to render such a graph to PNGs.
