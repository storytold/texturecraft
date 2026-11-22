# Project import

This page is for contributors and their agents: how the one-way import of the reference app's projects works, where
its measured rules live, and the graph text format the graph runtime reads. The crates are `import-reader` (L0),
`graph` (L0, `texturecraft-graph`), `import` (L2) and `import-verify`; the engine side is `engine::import_cmds` and
`engine::import_replay`.

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

```sh
cargo run -p texturecraft-import-reader --features dev-tools --example layers -- path/to/project-file
```

```rust
use texturecraft_import_reader::decode::assbin::AssbinGeometry;
use texturecraft_import_reader::project::{ProjectFile, paths};

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

`cargo test -p texturecraft-import-reader` runs everything that needs no project files. The corpus tests run over real projects
when given them, and are ignored otherwise: `TC_IMPORT_CORPUS=<folders or project files> cargo test --release -p
texturecraft-import-reader -- --ignored`. Nothing read from a project is ever committed. `tests/fixtures/small.h5` is a
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
reference app one way: `import-reader` reads it, `import` maps it onto the model (layers, folders, fills,
masks, levels, blend modes, geometry masks, baked maps, bake settings) and composites every texture set
with `compose` to compare it with the file's stored result. The file's compiled graphs are decompiled
(`import-reader`) and run by the graph runtime (`texturecraft-graph`): procedural and material fills become graph
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
the file; per texture set converting and replaying strokes; cooking graphs or comparing with the original when
requested; saving; and preparing the view) and a cancel flag. An interactive import evaluates procedural materials
when it prepares the first view. CPU sets are composited on the worker and GPU sets go straight to the viewport.
Offline import keeps the original-result comparison enabled by default. Inside a long step the bar moves by the work done: layers
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

Across documents, an import can also keep its cooks in a result cache folder (`resultCache`, CLI
`--result-cache`; native builds only): the graph store looks there by cook key before rendering and puts each
finished render there (`compose::graphs::kept`, a `KeptCooks` the engine backs with `texturecraft-result-cache`,
L0: entries checked by key, length and checksum, the folder kept under a limit, least recently used out first).
A second import of the same project, or of another one using the same graph the same way, reads the render.

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

### Graph engine (`crates/graph/tests/rules/`)

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

Runners are in `crates/graph/src/rule_tables.rs`. Run them all with `cargo test -p texturecraft-graph --lib rule_table_`.

#### Where ours differs from the rule

`bilinear_read.tsv`: ours reads in single precision and rounds an exact tie (`(a (256 - w) + b w) mod 256 = 128`) down,
where the measured rule rounds it up; 3 of its 60 rows are such ties. The table holds both (`ours_16`, `rule_16`); the
runner checks that ours differs only there. Not changed here, since this work is a pure refactor.

### Importer (`crates/import/tests/rules/`)

Measured rules the importer relies on, as plain numbers: inputs and expected outputs, one table per rule. The tables
don't depend on our types, so another implementation can run the same data against its own code. Each table names the
measurement the rule comes from; the expected values are what our code computes, which that section shows to match the
reference app to the stated error. The graph engine has its own tables in `crates/graph/tests/rules/`, in the same
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
  In the long run a narrow shared graph crate with storytold/materialcraft (for example in `storytold/craft-libs`)
  would let both apps share this work; that needs the maintainers' agreement (`AGENTS.md` keeps the apps' crates apart).

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

The graph runtime (`crates/graph`, package `texturecraft-graph`) reads node graphs from a plain text format and
evaluates them to float RGBA images on the CPU. As a loose file a graph text uses the extension `.tcgraph`;
`texturecraft-cli graph <file> <out dir>` renders one to PNGs. This page describes format 3, the version the runtime
writes (`graph::FORMAT_VERSION`).

### Lines

One directive per line. `#` starts a comment that runs to the end of the line; blank lines are ignored. Values are
whitespace-separated tokens; a value with spaces is written in double quotes.

| Directive | Meaning |
|---|---|
| `format <n>` | The format version the text was written with. Texts without it are version 1. A version newer than the runtime's is refused with an error. |
| `size <w> [<h>]` | The graph's nominal size in pixels (the height defaults to the width). An evaluation may ask for another size (1 to 8192); node results scale with it. |
| `seed_offset <n>` | Added to every random seed in the graph (`texturecraft-cli graph --seed`). |
| `meta <key> "<value>"` | Free key/value data. `meta kind "function"`, `"fxmap"` or `"patterns"` marks a subgraph as a function graph, an FX-map program or a pattern program; `meta label` names the graph. |
| `node <id> <kind> [key=value ...]` | A node. `id` is a whole number, unique in its graph; `kind` is one of the node kinds below. `key=@<id>` connects input slot `key` to node `id`, `key=@<id>.<output>` to a named output of a node with several. A number value is a parameter; anything else is a text parameter. `_x=` and `_y=` are the node's position in an editor. |
| `output <name> <id>[.<output>]` | A graph output named `name`, taken from node `id`. Graphs may instead carry `output` nodes (`usage`, `identifier`). |
| `subgraph <name>` ... `end` | A graph defined inside this one, in the same format, used by `subgraph` nodes (`graph="<name>"`), by `pixel_processor`/`value_processor` nodes (`fn="<name>"`), by `fx_map` (`program="<name>"`) or by `pattern_program` (`program="<name>"`). |
| `resource <name> <base64>` | Bytes carried by the text (an image a `bitmap` node reads by `file="<name>"`). |
| `expose <label> <node> <param> <min> <max>` | A parameter shown to users under `label`, clamped to the range (`texturecraft-cli graph --param <label>=<v>`). |
| `link <name> "<path>" ["<graph>"]` | A live link: the graph is read from another file (relative to this one) whenever the text is loaded, and used through `subgraph` nodes named `name`. Native builds only. |
| `comment <x> <y> <w> <h> <color> <text>` | An editor note; ignored by evaluation. |

Example:

```text
format 3
size 256
node 1 perlin scale=8 seed=3
node 2 levels in=@1 low=0.2 high=0.8
node 3 blend bg=@1 fg=@2 mode="multiply" opacity=0.5
output height 3
```

### Named values

A node keeps these as numbers in memory; the text writes a name where one exists. Reading accepts the name or the
number. The tables are in `crates/graph/src/names.rs`.

| Node kind | Key | Names |
|---|---|---|
| every kind | `format` (output format) | `graph` (the graph's own), `u8`, `u16`, `f16`, `f32`, and `u8_compiled`, `u16_compiled`, `f16_compiled`, which quantise the way compiled graphs do (their own rounding, and blurs and levels that keep the format on the way) |
| `blend` | `mode` | `copy`, `add`, `subtract`, `multiply`, `screen`, `overlay`, `max`, `min`, `difference`, `soft_light`, `color_dodge`, `color_burn`, `linear_burn`, `hard_light`, `exclusion`, `divide`, `linear_light`, `color`, `luminosity`, `dissolve`, `add_sub`, `switch`, and `add_compiled`, `subtract_compiled`, `add_sub_compiled`, `divide_compiled`, `soft_light_compiled` (opacity acts before the result is limited) |
| `pp_quadrant` | `pattern` | `square`, `disc`, `paraboloid`, `bell`, `gaussian`, `thorn`, `pyramid`, `brick`, `gradation`, `waves`, `half_bell`, `ridged_bell`, `crescent`, `capsule`, `cone`, `gradation_offset`, `hemisphere` |
| `f_random` | `lcg` | which 24-bit generator a draw comes from, by where the function runs: `pixel`, `value`, `parameter`, `size`, `fx_luminance`, `fx_branch_offset`, `fx_pattern_offset`, `fx_size`, `fx_rotation`, `fx_variation`, `fx_switch_condition`, `fx_iterate_count`, `fx_image_index` |

Older spellings still read the same: the key `ofmt=<n>` for `format`, numbers in place of the names, and seven function
ops under their format-2 names (`f_fsin`, `f_fcos`, `f_flog`, `f_fdiv`, `f_fatan2`, `f_gsin`, `f_gcos`, now
`f_sin_poly`, `f_cos_poly`, `f_log_floor`, `f_div_large`, `f_atan2_approx`, `f_sin_f32turns`, `f_cos_f32turns`).

### Evaluation

- Results are float RGBA. The same text, size and parameters give the same bits on every platform and thread count:
  transcendental functions come from one implementation (`crate::math`), parallel loops split work at fixed indices,
  and results never depend on which thread computed them.
- Evaluation never panics on a hostile text: sizes are limited to 8192 per side, subgraphs nest at most 8 deep, function
  calls at most 8 deep, a function graph runs at most 2048 nodes deep per pixel, FX-maps draw at most 500,000 markers,
  and loops are bounded.
- The host stops an evaluation with a cancel flag (`context::Cancel`); the runtime never reads a clock. Files a graph
  names but does not carry are read through the host's `context::FileSource` (none on the web).
- `versions::of_kind` gives a version per node kind, derived at build time from the source each kind reaches, for
  caches of results.

### Node kinds

Function graphs (`meta kind "function"`) carry values (float, float2-4, integer, integer2-4, boolean, string) instead of
images and never produce NaN or infinity. FX-map programs (`meta kind "fxmap"`) are run by the `fx_map` node; pattern
programs (`meta kind "patterns"`) by `pattern_program`. The tables below are the runtime's own node definitions
(`graph::SPECS`, `compiled_ops::SPECS`, `atomic::SPECS`, `func::FUNC_SPECS`, `fxmap::FXM_SPECS`, `patterns::SPECS`).

#### Image nodes

| Node | Category | Inputs | Description and parameters |
|---|---|---|---|
| `color` | Generators |  | constant colour: r g b a |
| `white_noise` | Generators |  | per-pixel random: seed distribution(0 uniform,1 gaussian,2 triangle) |
| `gaussian_noise` | Generators |  | gaussian-distributed per-pixel noise: seed scale contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `perlin` | Generators |  | perlin noise: scale scale_x scale_y seed disorder disorder_speed tile_x tile_y |
| `fbm` | Generators |  | fractal noise/clouds: scale octaves gain seed mode(0 fbm,1 ridged,2 turbulence) disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `voronoi` | Generators |  | cellular: scale jitter seed mode(0 f1,1 cell id,2 edge,3 f2,4 f2-f1,5 f1*f2,6 f1/f2) aspect offset_x offset_y distortion distortion_scale distance_mode(0 euclid,1 manhattan,2 chebyshev,3 minkowski) minkowski edge_thickness rounded distance_scale color_seed_mode color_seed invert |
| `voronoi_fractal` | Generators |  | layered voronoi: scale jitter seed mode aspect offset_x offset_y distortion distortion_scale distance_mode minkowski edge_thickness rounded distance_scale min_level max_level roughness lacunarity opacity blend(0 add,1 max,2 min) color_seed_mode color_seed invert |
| `gradient` | Generators |  | gradient: mode(0 linear,1 radial) angle |
| `circle` | Generators |  | radius softness |
| `rect` | Generators |  | width height softness |
| `polygon` | Generators |  | sides radius softness rotation |
| `checker` | Generators |  | tiles |
| `bricks` | Generators |  | rows cols mortar bevel offset lum_var seed |
| `fx_map` | Generators | in (optional), in2 (optional), in3 (optional), in4 (optional) | run a pattern program: program="name" depth seed blend (0 max 1 add 2 over 3 min 4 multiply) color bg bg_r bg_g bg_b |
| `curve` | Filters | in | tone curves: points (master), points_r points_g points_b points_a as "x:y,x:y" lists |
| `svg` | Generators |  | vector image: file="art.svg" fit (0 contain 1 stretch 2 cover) scale x y |
| `text` | Generators |  | text: text="..." font="face.ttf" (optional) size x y align (0 left 1 centre 2 right) spacing line_spacing rotation value |
| `channel_shuffle` | Filters | in, in2 (optional) | rebuild channels: src_r src_g src_b src_a (0-3 channels of in, 4-7 channels of in2, 8 zero, 9 one), inv_r inv_g inv_b inv_a |
| `input_value` | Utility |  | graph parameter read by functions: name, type (0 float, 1 int, 2 bool, 3 float2, 4 float3, 5 float4, 6 int2, 7 int3, 8 int4, 9 string), v / x y z w, min max, label |
| `pixel_processor` | Generators | in (optional), in2 (optional), in3 (optional), in4 (optional), in5 (optional), in6 (optional), in7 (optional), in8 (optional), in9 (optional), in10 (optional), in11 (optional), in12 (optional), in13 (optional), in14 (optional), in15 (optional), in16 (optional), in17 (optional), in18 (optional), in19 (optional), in20 (optional), in21 (optional), in22 (optional), in23 (optional), in24 (optional), in25 (optional), in26 (optional), in27 (optional), in28 (optional), in29 (optional), in30 (optional), in31 (optional), in32 (optional) | per-pixel expr / expr_g / expr_b / expr_a (string params), or fn="function" to run a function graph per pixel |
| `value_processor` | Generators | in (optional), in2 (optional), in3 (optional), in4 (optional) | run fn="function" once; the result (float, float3 or float4) fills the image. The function can read image statistics |
| `sdf_shape` | SDF |  | 2D signed distance shape: shape(0 circle,1 box,2 polygon,3 ring) size aspect sides rotation round x y range |
| `sdf_op` | SDF | a, b | combine SDFs: op(0 union,1 subtract,2 intersect) smooth range |
| `sdf_mask` | SDF | in | SDF -> mask: offset softness |
| `sdf_form` | SDF |  | procedural 3D SDF form as height: shape(0 sphere,1 box,2 torus,3 cylinder) size repeat round scatter seed height |
| `bitmap` | Utility |  | flip_x flip_y gray; imported PNG bitmap: file="path.png" |
| `input` | Utility |  | subgraph input: idx |
| `subgraph` | Utility | in0 (optional), in1 (optional), in2 (optional), in3 (optional) | instance of a reusable subgraph: graph="name" |
| `shape` | Generators |  | procedural shape: square, disc, paraboloid, gaussian, diamond, pyramid, half sphere, cone |
| `tile_sampler` | Generators | pattern (optional), pattern2 (optional), pattern3 (optional), pattern4 (optional), pattern5 (optional), pattern6 (optional), mask (optional), scale_map (optional), rotation_map (optional), displacement_map (optional), color_map (optional), distribution_map (optional), background (optional) | grid of pattern instances: x_amount y_amount size aspect size_mode scale scale_random size_random rotation rotation_random pattern_rotation symmetry_random position_random offset offset_mode offset_random global_offset_x global_offset_y color color_random lum_by_number lum_by_scale mask_threshold mask_invert mask_random mask_checker invert_mask blending opacity background reverse_order pattern_type pattern_specific pattern_distribution |
| `tile_generator` | Generators | pattern (optional), pattern2 (optional), pattern3 (optional), pattern4 (optional), pattern5 (optional), pattern6 (optional), mask (optional), scale_map (optional), rotation_map (optional), displacement_map (optional), color_map (optional), distribution_map (optional), background (optional) | brick-style tiles: x_amount y_amount size aspect size_mode scale scale_random size_random rotation rotation_random pattern_rotation symmetry_random position_random offset offset_mode offset_random global_offset_x global_offset_y color color_random lum_by_number lum_by_scale mask_checker mask_horizontal mask_vertical mask_random invert_mask blending opacity background reverse_order pattern_type pattern_specific pattern_distribution |
| `splatter` | Generators | pattern (optional), pattern2 (optional), pattern3 (optional), pattern4 (optional), pattern5 (optional), pattern6 (optional) | random scatter of a pattern: count grid_number scale scale_var aspect rotation rot_var pattern_rotation lum_var disorder pan_x pan_y symmetry_random gain output_min output_max background blending opacity pattern_type pattern_specific pattern_distribution |
| `splatter_circular` | Generators | pattern (optional), pattern2 (optional), pattern3 (optional), pattern4 (optional), pattern5 (optional), pattern6 (optional) | instances on one or more circles: number amount_random rings radius radius_random ring_radius_multiplier angle_random spiral_factor spread size size_random scale_by_number scale_by_ring rotation rotation_random center_orientation ring_rotation ring_rotation_random position_random color luminance_random lum_by_number lum_by_ring mask_random background blending opacity pattern_type pattern_specific pattern_distribution |
| `clouds` | Generators |  | soft cloud noise: scale seed contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `bnw_spots` | Generators |  | black and white spots: scale seed contrast invert variant disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `crystal` | Generators |  | faceted crystal cells: scale disorder seed contrast invert variant |
| `directional_noise` | Generators |  | streak noise, seamless at any angle: scale stretch angle seed contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `grunge` | Generators |  | grunge masks: variant(0 concrete,1 scratches,2 spots,3 smudges) balance large_variation base_noisiness dirt_specks_opacity invert_dirt scratches_opacity sharpen quantity scratch_width scratch_length details coverage coverage_contrast disorder disorder_speed seed contrast invert |
| `plasma` | Generators |  | smooth plasma waves |
| `stripes` | Generators |  | stripes: count angle softness width tile(1 = snap to seamless angle, 0 = exact) |
| `weave` | Generators |  | woven threads: count width |
| `star` | Generators |  | star shape: points inner size softness rotation |
| `gradient_axial` | Generators |  | symmetric gradient: angle |
| `gradient_circular` | Generators |  | angular gradient: offset |
| `switch` | Filters | a (optional), b (optional), c (optional), d (optional) | pass one of up to four inputs: index |
| `flood_fill` | Filters | in | label connected regions of a mask |
| `flood_fill_random` | Filters | in | random grayscale per flood-fill region |
| `flood_fill_color` | Filters | in | random color per flood-fill region |
| `flood_fill_to_index` | Filters | in | region number of each flood-fill region as a gray level |
| `flood_fill_to_position` | Filters | in | centre of each flood-fill region (outputs: position, x, y) |
| `flood_fill_to_bbox_size` | Filters | in | bounding-box size of each flood-fill region (outputs: size, width, height) |
| `flood_fill_gradient` | Filters | in | gradient across each flood-fill region |
| `vector_warp` | Filters | in, vector | displace by a vector map: intensity directx |
| `slope_blur` | Filters | in, slope | blur along a slope map: samples intensity mode |
| `blur_hq` | Filters | in | high quality blur: intensity quality |
| `histogram_range` | Filters | in | select a range of values: position range contrast invert |
| `bevel` | Filters | in | bevel a shape: distance smoothing threshold use_as_normal normal_intensity corner(0 round,1 angular) directx |
| `distance` | Filters | in | distance field of a mask: distance mode |
| `color_match` | Filters | in, reference (optional), src_color (optional), target_color (optional) | recolour: method(0 statistics transfer from reference,1 colour swap) strength source_mode(0 average,1 colour,2 input) src_r src_g src_b target_mode(0 colour,1 input) target_r target_g target_b custom_variation variation_hue variation_chroma variation_luma use_mask mask_hue_range mask_chroma_range mask_luma_range mask_smoothness |
| `curvature_smooth` | Material | in | smooth curvature from a normal map |
| `height_blend` | Material | bg, fg, mask (optional) | blend two height maps: opacity contrast output |
| `make_it_tile_photo` | Filters | in | make a photo tile: width width_v |
| `make_it_tile_patch` | Filters | in | make an image tile with random patches |
| `clamp` | Filters | in | clamp values: low high apply_alpha |
| `highpass` | Filters | in | high-pass filter: radius |
| `swirl` | Filters | in | swirl distortion: amount radius |
| `edge_wear` | Material | in, noise (optional) | edge wear mask from a height map: amount width contrast |
| `dirt` | Material | in, noise (optional) | dirt mask from an ambient occlusion map: amount contrast |
| `dust` | Material | in, noise (optional) | dust mask from a normal map: amount contrast |
| `output` | Utility | in (optional) | graph output: usage identifier |
| `dot` | Utility | in | re-route a link |
| `blur` | Filters | in | gaussian blur: radius |
| `directional_blur` | Filters | in | angle length |
| `sharpen` | Filters | in | radius amount |
| `levels` | Filters | in | low high gamma out_low out_high |
| `adjust` | Filters | in | brightness contrast |
| `invert` | Filters | in | invert colours: invert |
| `threshold` | Filters | in | binary threshold: threshold softness mode(0 greater,1 greater or equal,2 lower,3 lower or equal) |
| `histogram_scan` | Filters | in | position width |
| `quantize` | Filters | in | steps |
| `to_gray` | Filters | in | luminance |
| `channel` | Filters | in | ch(0 r,1 g,2 b,3 a) |
| `merge` | Filters | r (optional), g (optional), b (optional), a (optional) | combine channels |
| `tint` | Filters | in | multiply by colour: r g b |
| `colorize` | Filters | in | gradient map: interp(0 linear,1 smooth,2 step,3 smoother) smoothness addressing(0 clamp,1 repeat) use_alpha  stops p0 r0 g0 b0 a0 ... p15 r15 g15 b15 a15 |
| `hsl` | Filters | in | hue sat val |
| `transform` | Filters | in | offset_x offset_y scale_x scale_y rotation(turns) |
| `mirror` | Filters | in | mirror an image: axis(0 x,1 y,2 both,3 corner) offset_x offset_y invert_x invert_y corner(0 top left,1 top right,2 bottom left,3 bottom right) |
| `warp` | Filters | in, warp | slope warp: intensity filtering(0 bilinear,1 nearest,2 bicubic) |
| `directional_warp` | Filters | in, warp | warp in one direction: angle intensity offset filtering(0 bilinear,1 nearest,2 bicubic) |
| `blend` | Filters | fg, bg, mask (optional) | blend two images: mode (copy, add, subtract, multiply, screen, overlay, max, min, difference, soft_light, color_dodge, color_burn, linear_burn, hard_light, exclusion, divide, linear_light, color, luminosity, dissolve, add_sub, switch, and as compiled graphs blend: add_compiled, subtract_compiled, add_sub_compiled, divide_compiled, soft_light_compiled) opacity alpha_mode(0 source alpha,1 ignore alpha,2 straight,3 premultiplied) crop_x0 crop_y0 crop_x1 crop_y1 |
| `normal` | Material | in | height->normal (OpenGL +Y): strength directx alpha_from_input |
| `normal_blend` | Material | a, b, mask (optional) | blend normal maps (a = background, b = foreground): opacity |
| `ao` | Material | in | ambient occlusion: mode(0 blur,1 horizon) radius intensity hbao_radius hbao_intensity quality(0 4,1 8,2 16 samples) use_world_units height_depth surface_size height_depth_cm |
| `quantize_color` | Filters | in | reduce to few colours (k-means): color_amount contour_smoothing dithering dithering_pattern(0 noise,1 bayer) ignore_alpha distance_color_space(0 lab,1 rgb) |
| `curvature` | Material | in | curvature of a normal map: mode(0 normal map,1 height map) intensity directx radius |
| `edge_detect` | Filters | in | edge lines: mode(0 lines,1 sobel) edge_width edge_roundness invert tolerance intensity |
| `emboss` | Filters | in | angle intensity |
| `non_uniform_blur` | Filters | in, mask | blur by a mask: intensity |
| `radial_blur` | Filters | in | spin blur around a centre: mode(0 spin,1 zoom) samples angle x y amount radius |
| `anisotropic_blur` | Filters | in | blur stretched in one direction: intensity anisotropy angle |
| `symmetry` | Filters | in | mirror the image around a line: axis(0 x,1 y,2 both,3 diagonal,4 anti-diagonal,5 eight-fold) offset flip transfer(0 copy,1 add,2 sub,3 mul,4 add sub,5 max,6 min) |
| `skew` | Filters | in | shear that keeps tiling: skew_x skew_y (whole numbers) align(0 centre,1 top left,2 bottom right) |
| `auto_levels` | Filters | in | stretch values to the full range: per_channel |
| `channel_mixer` | Filters | in | mix colour channels: rr rg rb gr gg gb br bg bb mono |
| `alpha_merge` | Filters | rgb, alpha (optional) | put a grayscale image into the alpha channel |
| `color_to_mask` | Filters | in | mask of pixels close to a colour: r g b range softness |
| `replace_color` | Filters | in | replace one colour with another: src_r src_g src_b dst_r dst_g dst_b range softness |
| `pow` | Filters | in | raise values to a power: exponent |
| `histogram_shift` | Filters | in | shift all values: position |
| `grayscale_conversion` | Filters | in | weighted grayscale: r g b |
| `cells` | Generators | color (optional) | cellular noise: scale jitter seed variant(0 cells1,1 cells2,2 cells3,3 cells4) luminance_random color_source(0 random,1 pseudorandom,2 image) pseudorandom_seed edge_width hardness contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `fractal_sum` | Generators |  | sum of noise octaves: scale min_level max_level roughness seed mode(0 base,1 turbulence,2 creased) contrast invert disorder disorder_speed |
| `waveform` | Generators |  | tileable wave pattern: count angle warp warp_scale seed shape(0 sine,1 triangle,2 saw,3 square) contrast invert disorder disorder_speed |
| `gaussian_spots` | Generators |  | soft gaussian spots: scale size size_variation seed contrast invert disorder disorder_speed anisotropy anisotropy_angle tile_x tile_y |
| `normal_invert` | Material | in | flip normal map channels: invert_x invert_y invert_z invert_a |
| `normal_normalize` | Material | in | renormalise a normal map |
| `normal_vector_rotation` | Material | in, rotation (optional) | rotate normal vectors: angle directx |
| `normal_to_height` | Material | in | integrate a normal map into a height map: directx relief_balance opacity |
| `sobel` | Filters | in | sobel edge magnitude: intensity |
| `glow` | Filters | in | glow around bright areas: width intensity clear glow_r glow_g glow_b |
| `mosaic` | Filters | in, mosaic_map (optional) | mosaic of the map regions (or a pixel grid without a map): count intensity |
| `non_uniform_directional_warp` | Filters | in, intensity, angle (optional) | warp along a direction chosen per pixel: intensity angle |
| `multi_directional_warp` | Filters | in, warp | warp in several directions at once: intensity directions(0 one,1 two,2 four) angle mode(0 average,1 max,2 min,3 chain) |
| `normal_combine` | Material | base, detail | combine two normal maps: technique(0 whiteout,1 channel mixer,2 detail oriented) |
| `triangle_grid` | Generators |  | grid of triangles: style(0 squares,1 jittered mesh) count border variation x_amount y_amount random_position_multiplier quincunx_offset_x quincunx_offset_y triangulation(0 fixed,1 alternate,2 random) output(0 smooth,1 flat,2 edge distance) height_random thin_lines distance_to_edge_thickness thin_line_color thin_line_background seed contrast invert |
| `grunge_map` | Generators |  | grunge map 1-15: map balance contrast brush_pattern cracks disorder coverage seed invert |
| `rgba_split` | Filters | in | split into R, G, B and A grayscale outputs |
| `alpha_split` | Filters | in | split into an RGB output and an alpha output |
| `weave_generator` | Generators |  | woven threads with gaps: tile_x tile_y weave shape gap_x gap_y contrast invert |
| `starburst` | Generators |  | star-like rays: count rays sharpness size rotation rotation_random seed contrast invert |
| `gradient_reflected` | Generators |  | gradient that goes 0-1-0: count angle contrast invert |
| `shape_stroke` | Filters | in | outline of a shape: width offset softness threshold opacity color_r color_g color_b |
| `shape_glow` | Filters | in | glow around a shape: width softness intensity threshold opacity spread color_r color_g color_b |
| `chrominance_extract` | Filters | in | colour without brightness |
| `shape_drop_shadow` | Filters | in | drop shadow of a mask: angle distance blur opacity spread |

#### Compiled-graph image nodes

| Node | Inputs | Description and parameters |
|---|---|---|
| `curve_keys` | in | tone curve from Bezier keys: n, then per key k xk yk (point) lxk lyk (left handle) rxk ryk (right handle); ramp=1 draws the curve left to right instead of applying it |
| `triangle_strips` |  | white triangle strips on black: strips="...", each strip hex digits, 8 per vertex (yyyyxxxx, 0..ffff across the image), strips apart by a space |
| `distance_map` | in, source (optional) | distance from the mask's bright pixels: distance (reach in 1/256 of the image, default 10), aspect_x aspect_y, tile_x tile_y (1, the default: across the image's edge); with a source, the colour of the nearest mask pixel, fade=1 fading it out with distance |
| `normal_map` | in | tangent-space normals from a height: intensity (default 1, scaled by width/256), flip_y, alpha=1 keeps the height in alpha, aspect_x aspect_y |
| `emboss_map` | in, map | adds the slope of map along a direction: intensity (default 1, scaled by width/256), angle (turns), aspect_x aspect_y |
| `gradient_dynamic` | in, grad | look up the grad image with the input: in picks the column (vertical=1 the row), position the other coordinate |

#### Atomic image nodes

| Node | Inputs | Description and parameters |
|---|---|---|
| `levels_mid` | in | levels with a midpoint: low high mid out_low out_high; per channel with suffix _r _g _b _a (e.g. low_r) |
| `hsl_centered` | in | hue, saturation and lightness shifts, 0.5 = unchanged, twice the distance from 0.5 is added: hue (turns) sat lum |
| `transform_matrix` | in | transform by a matrix around the centre, reading the input at (m00 x + m10 y + offset_x, m01 x + m11 y + offset_y) from the centre: tile_x tile_y (1 repeat, 0 bg_r bg_g bg_b bg_a outside, default none) filter (1 bilinear, 0 nearest) mip (level, 31 from the scale) |
| `dir_warp` | in, map | move each pixel along a direction by the brightness of map: intensity (1/256 of the image per unit of map), angle (turns), map_offset (brightness that moves nothing), tile_x tile_y (1 repeat, 0 repeat the edge) |
| `grad_warp` | in, map | move each pixel along the slope of map: intensity (default 1; 1/256 of the image per unit of slope in uv), aspect_x, aspect_y (default 1), tile_x tile_y (1 repeat, 0 keep the edge: no slope across it, edge value read) |
| `box_blur` | in | blur with a box in both axes reaching intensity pixels of a 256 pixel image each way: intensity aspect_x aspect_y (scale it per axis) tile_x tile_y (1 repeat, 0 repeat the edge) |
| `line_blur` | in | average along a line reaching intensity pixels of a 256 pixel image each way, by shearing the image, a box along the longer axis and shearing back: intensity aspect_x aspect_y (scale it per axis) angle (turns) tile_x tile_y (1 repeat, 0 repeat the edge) |
| `laplacian_sharpen` | in | add intensity times the difference to the four neighbours, limited to 0..1: intensity aspect_x aspect_y (scale it per axis) tile_x tile_y (1 repeat, 0 repeat the edge) |
| `pattern_program` | in (optional), in2 (optional), in3 (optional), in4 (optional), in5 (optional), in6 (optional), in7 (optional), in8 (optional), in9 (optional), in10 (optional), in11 (optional), in12 (optional), in13 (optional), in14 (optional), in15 (optional), in16 (optional), in17 (optional), in18 (optional), in19 (optional), in20 (optional), in21 (optional), in22 (optional), in23 (optional), in24 (optional), in25 (optional), in26 (optional), in27 (optional), in28 (optional), in29 (optional), in30 (optional), in31 (optional), in32 (optional), bg (optional) | run a pattern program: program="name" seed color(0/1); bg is an image the canvas starts from |

#### Function-graph nodes

| Node | Inputs | Description and parameters |
|---|---|---|
| `f_out` | in | result of the function |
| `f_pin` | a | re-route a value link |
| `f_float` |  | constant float: v |
| `f_int` |  | constant integer: v |
| `f_bool` |  | constant boolean: v (0 or 1) |
| `f_string` |  | constant string: v |
| `f_vec2` | x (optional), y (optional) | float2 from parts (x y, linked parts win over the typed values) |
| `f_vec3` | x (optional), y (optional), z (optional) | float3 from parts |
| `f_vec4` | x (optional), y (optional), z (optional), w (optional) | float4 from parts |
| `f_ivec2` | x (optional), y (optional) | integer2 from parts |
| `f_ivec3` | x (optional), y (optional), z (optional) | integer3 from parts |
| `f_ivec4` | x (optional), y (optional), z (optional), w (optional) | integer4 from parts |
| `f_swizzle` | a | pick or reorder components: mask (xyzw or rgba, e.g. "zyx") |
| `f_neg` | a | negate |
| `f_abs` | a | absolute value |
| `f_sqrt` | a | square root (0 for negative input) |
| `f_exp` | a | e to the power a |
| `f_log` | a | natural logarithm (0 for input <= 0) |
| `f_log_floor` | a | natural logarithm, very low for input <= 0 (the engine of compiled graphs) |
| `f_log2` | a | base-2 logarithm (0 for input <= 0) |
| `f_sin` | a | sine (radians) |
| `f_cos` | a | cosine (radians) |
| `f_sin_poly` | a | sine by the polynomial of the engine of compiled graphs' fx-maps and node parameters |
| `f_cos_poly` | a | cosine by the polynomial of the engine of compiled graphs' fx-maps and node parameters |
| `f_sin_f32turns` | a | sine as a compiled graph's pixel processor has it on a GPU: the turn count truncated in single precision |
| `f_cos_f32turns` | a | cosine as a compiled graph's pixel processor has it on a GPU: the turn count truncated in single precision |
| `f_tan` | a | tangent (radians) |
| `f_asin` | a | inverse sine (input clamped to -1..1) |
| `f_acos` | a | inverse cosine (input clamped to -1..1) |
| `f_atan` | a | inverse tangent |
| `f_radians` | a | degrees to radians |
| `f_degrees` | a | radians to degrees |
| `f_floor` | a | round down |
| `f_ceil` | a | round up |
| `f_round` | a | round to nearest |
| `f_fract` | a | fractional part |
| `f_sign` | a | -1, 0 or 1 |
| `f_saturate` | a | clamp to 0..1 |
| `f_not` | a | boolean not |
| `f_length` | a | length of a vector |
| `f_normalize` | a | vector of length 1 (zero stays zero) |
| `f_add` | a, b | a + b |
| `f_sub` | a, b | a - b |
| `f_mul` | a, b | a * b |
| `f_div` | a, b | a / b (0 when b is 0; integer division truncates) |
| `f_div_large` | a, b | a / b, very large when b is 0 and a is not (the engine of compiled graphs) |
| `f_mod` | a, b | remainder of a / b with the sign of b (0 when b is 0) |
| `f_pow` | a, b | a to the power b |
| `f_min` | a, b | smaller value |
| `f_max` | a, b | larger value |
| `f_atan2` | a, b | angle of the point (b, a) |
| `f_atan2_approx` | a, b | angle of the point (b, a), by the approximation of the engine of compiled graphs |
| `f_step` | a, b | 0 when b < a, else 1 |
| `f_dot` | a, b | dot product |
| `f_cross` | a, b | cross product of two float3 |
| `f_distance` | a, b | distance between two points |
| `f_clamp` | a, b, c | clamp a between b (low) and c (high) |
| `f_lerp` | a, b, c | a + (b - a) * c |
| `f_smoothstep` | a, b, c | smooth 0..1 ramp of c between edges a and b |
| `f_eq` | a, b | a == b |
| `f_ne` | a, b | a != b |
| `f_lt` | a, b | a < b |
| `f_le` | a, b | a <= b |
| `f_gt` | a, b | a > b |
| `f_ge` | a, b | a >= b |
| `f_and` | a, b | boolean and |
| `f_or` | a, b | boolean or |
| `f_xor` | a, b | boolean exclusive or |
| `f_if` | cond, a, b | a when cond is true, else b (only the chosen branch runs) |
| `f_sequence` | a, b | run a, then b; returns b |
| `f_loop` | a, c, b (optional) | run a, then c again and again: until b is true, or `passes` times; returns 0 |
| `f_set` | a | store a in the variable `name`; returns a |
| `f_get` |  | read the variable `name` (0 until it is set); type: what it holds, for type checks (default float) |
| `f_random` | a (optional), b (optional), s (optional) | random float between a (default 0) and b (default 1); `seed` shifts the sequence; `lcg` draws from a 24-bit generator as compiled graphs do, by where the function runs (pixel, value, parameter, size, fx_luminance ... fx_image_index; see names::LCG_SITES and lcg_rand), with seed s and draw number k |
| `f_sample_gray` | a | brightness of input image `slot` (default in) at the position a (float2, 0..1); filter: bit 0 bilinear, else nearest, bit 1 keep the edge, else repeat (default bilinear, repeat) |
| `f_sample_color` | a | colour (float4) of input image `slot` at the position a; filter as for f_sample_gray |
| `f_image_avg` |  | average colour (float4) of input image `slot` |
| `f_image_min` |  | smallest channel values (float4) of input image `slot` |
| `f_image_max` |  | largest channel values (float4) of input image `slot` |
| `f_pos` |  | position being evaluated (float2, 0..1) |
| `f_size` |  | output size in pixels (float2) |
| `f_own_size` |  | size in pixels (float2) of the node whose parameter this computes; the output size elsewhere |
| `f_cell_size` |  | size of the cell a pattern program is walking, in uv (float; 1 elsewhere) |
| `f_cell_depth` |  | how many quadrants deep the cell of a pattern program is (float; 0 elsewhere) |
| `f_index` |  | instance / iteration number (integer) |
| `f_count` |  | number of instances / iterations (integer) |
| `f_seed` |  | random seed of the caller (integer) |
| `f_input` |  | input of this function: name, type (0 float, 1 int, 2 bool, 3 float2, 4 float3, 5 float4, 6 int2, 7 int3, 8 int4, 9 string), default v / x y z w |
| `f_graph_input` |  | value of an Input Value node of the graph the caller lives in: name, type, default v / x y z w |
| `f_to_float` | a | convert to float |
| `f_to_int` | a | convert to integer (drops the fraction) |
| `f_to_bool` | a | non-zero is true |
| `f_to_string` | a | text of a value |
| `f_concat` | a, b | join two strings |
| `f_call` | a (optional), b (optional), c (optional), d (optional) | run another function (`fn`); a b c d fill its inputs in the order of their node numbers |

#### FX-map program nodes

| Node | Inputs | Description and parameters |
|---|---|---|
| `fxm_out` | in | start of a pattern program |
| `fxm_marker` |  | draw a shape: shape (0 circle 1 square 2 diamond 3 image) src x y w h rot value r g b softness rnd_pos rnd_scale rnd_rot rnd_val rnd_color |
| `fxm_quadrant` | q0 (optional), q1 (optional), q2 (optional), q3 (optional) | split the cell in four: scale off_x off_y p0..p3 (chance) v0..v3 (value) r0..r3 (rotation) rnd_pos rnd_scale rnd_rot rnd_val |
| `fxm_iterate` | body (optional) | repeat body: count dx dy dscale drot dval, or grid_x grid_y stagger; rnd_x rnd_y rnd_scale rnd_rot rnd_val |
| `fxm_switch` | a (optional), b (optional), c (optional), d (optional) | run one branch: mode (0 index, 1 random) index w0..w3 |

#### Pattern-program nodes

| Node | Inputs | Description and parameters |
|---|---|---|
| `pp_out` | in | start of a pattern program; fn.init, then fn.background, fn.roughness, fn.opacity, fn.frame and fn.frame_offset run first |
| `pp_iterate` | first (optional), body (optional) | run first once, then body count times (count or fn.count); f_index is the iteration |
| `pp_switch` | a (optional), b (optional) | run b when cond (or fn.cond) is true, else a |
| `pp_quadrant` | q0 (optional), q1 (optional), q2 (optional), q3 (optional) | one cell, split in four for q0..q3: draws pattern (square, disc, paraboloid, bell, gaussian, thorn, pyramid, brick, gradation, waves, half_bell, ridged_bell, crescent, capsule, cone, gradation_offset, hemisphere), or with image=1 the input image_index, with luminance, branch_offset_x/y, offset_x/y, size_x/y, rotation, variation, max (each also fn.<name>) |
