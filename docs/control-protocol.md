# Control protocol, MCP and CLI

Everything TextureCraft does is an engine command with a stable id (`layer.add.fill`,
`bake.run`, `edit.undo`, …). Agents and scripts reach the same commands four ways:

| Surface | Start it with | Use it for |
|---|---|---|
| Control channel | `texturecraft --control <port>` | Drive the running desktop app: commands, inspect, menus, screenshots |
| MCP server | `texturecraft-cli mcp` | AI agents (Claude Code, Claude Desktop, any MCP client), headless or bridged to the app |
| JSON-lines server | `texturecraft-cli serve [--port <port>]` | Scripts that keep one headless session open |
| CLI subcommands | `texturecraft-cli info/run/batch/bake/export/render/new` | One-shot jobs in shells and pipelines |

Programmatic calls never open dialogs: a command runs with its defaults. Only `ui.menu.invoke`
behaves like a menu click. `session.inspect` shows every layer, mask, effect, fill source and
painted tile count, so an agent can verify its work without a screenshot.
`effect.histogram {effect, channel?, bins?}` counts the values an effect reads (a levels
effect's input), so an agent can pick its input range from the data. `effect.move {effect,
index}` reorders an effect within its stack (0 = bottom) and `effect.duplicate {effect}` copies
one just above itself. `effect.copy {effect?}` and `effect.cut {effect?}` put an effect on the
layer clipboard, and `effect.paste {layer?, target?}` adds it to another layer's content or mask.
`layer.select {layers: [id], layer?}` selects several layers of one texture set at once (`layer`
is the active one), and `{layer, range: true}` / `{layer, toggle: true}` do what Shift+click and
Ctrl+click do in the Layers panel. `layer.select {all: true}` (Ctrl+A with the pointer over the
Layers panel) selects every shown row of the active texture set: the folder rows and the layers
of open folders, not the layers of collapsed ones. `{layer, toggle: true}` on the only selected
layer leaves nothing selected. `layer.visible {layers, layer}` gives every listed layer the
opposite of `layer`'s visibility (the Layers panel's eye uses `{layer}` and acts on that layer
only). `layer.group` puts the new folder at the place and in the folder of the topmost selected
layer, the others in stack order; `layer.duplicate` puts the copies in one block above the
topmost selected layer. Without `layer`, `layer.remove`, `layer.duplicate`,
`layer.group` and `layer.visible` act on every selected layer; each of them and `layer.move
{layers, above | index}` also take an explicit `layers` list. `session.inspect` lists the
selection as `selectedLayers` (top row first).

`mask.export {layer?, path}` writes the layer's evaluated mask as a 16-bit grayscale PNG at the
texture set's resolution: black where the mesh does not cover the texture, with the orientation
of the exported textures (top row first). A UV-tiled set writes one file per tile, each at its
tile's size, with the tile number before the extension (`mask.1001.png`, `mask.1002.png`). The
result is `{files}`. A layer without an enabled mask is an error. The layer menu's Export mask to
file runs it; Copy mask to clipboard puts the same mask on the system clipboard as an 8-bit
grayscale image (the whole tile atlas for a UV-tiled set).

`effect.set {effect, filter: "blur", radius: 5}` replaces a filter in place, preserving its
stack position, blend, opacity and visibility. Replacement filters start from defaults plus
the supplied parameters; naming the current filter instead edits it without resetting omitted
parameters. Default effect names follow the new filter, custom names remain, and an explicit
`name` overrides either. Only filter effects can be replaced this way, and masks reject
content-only filters. `filter.list` lists the available IDs; levels and material finishes
must be added separately with `effect.add.filter`. A successful replacement is one undo step;
a rejected replacement changes nothing.

`edit.undo {steps?}` and `edit.redo {steps?}` go back or forward several steps at once (as far
as the history reaches); `edit.history` lists the steps done and the undone ones, next first. Step names are English; a stroke is named by tool and layer (`Paint on Rust`,
`Clone on Rust`, `Erase mask of Rust`).
Any command takes `merge: true` to fold its edit into the previous undo step when that step has
the same label: a drag that applies a value on every frame stays one step.

A colour param is `[r, g, b]` in linear 0..1, or `"#rrggbb"` in sRGB, which is converted to
linear. The same digits are two different colours: `"#c8102e"` is `[0.578, 0.005, 0.027]`, while
`[0.784, 0.063, 0.180]` (its hex digits over 255) paints the lighter `#e54776`. Pass a colour from
a design tool or a brand guide as hex. A single number is a linear grey.
For a colour selection, `effect.set` accepts only linear `[r, g, b]` arrays in its `colors` list.

`tool.cloneSource {uv: [u, v]}` (or `{x, y}` for a point on the mesh, or `{clear: true}`) sets the
Clone tool's source. The source belongs to the active texture set: selecting another texture set
(`textureSet.select`, or `layer.select` of a layer in another set) clears it. `tool.brush
{cloneAligned}` (default `true`) chooses how later strokes use it: aligned keeps the offset the
first stroke fixed, so the source moves along with the brush; `false` makes every stroke start
sampling at the source point. Changing `cloneAligned` forgets the offset of earlier strokes.
`tool.brush` takes the alpha's shared parameters next to `alpha`: `alphaSizeX` and `alphaSizeY`
(5..100, percent of the stamp the alpha fills, default 100), `alphaRoundness` (0..100, only for the
procedural Round alpha: 100 is a circle, 0 a square), `alphaContrast` (1..8, 1 leaves the values
alone) and `alphaInvert` (swaps paint and no paint inside the stamp square). They apply to
procedural, library and imported alphas alike and are returned by `tool.brush` with the other
values. The alpha's rotation is the brush `angle`.

Bake mode applies edits in Common Settings and Mesh Map Settings to every texture set. Each
set keeps its own enabled mesh-map checkboxes and Include in Bake flag. Edits update only the
changed fields; Restore Defaults resets the whole parameter section across all sets.

`bake.settings {ts: "all", patch: {maps: {ao: {parameters: {rays: {rays: 128}}}}}}` changes
only AO secondary rays. A `patch` uses the nested `BakeSettings` field names and named enum
values returned in the command's `model` result; omitted fields retain each set's values, and
`null` clears optional fields such as `common.resolution_height`. All targeted sets are validated
before the edit is applied as one undo step. `model` still replaces the complete settings;
when combined, `patch` follows `model`, then the flat command parameters apply. Omitting `ts`
still targets the active set for scripts that need individual control.

The bake texture-set list shows visibility eyes and expandable UV tile rows. Visibility affects
the viewport independently of bake inclusion. Its resolution menus edit the texture-set size
(`textureSet.resolution`) and individual tile overrides (`textureSet.tile.resolution`), independently
of Common Settings' bake Output Size. Overridden tiles display a resolution marked with an asterisk;
inherited tile resolutions appear only on hover or while their menu is open. Remove resolution
override hides the number and restores inheritance from the set, including when their current
sizes happen to match.

Baked maps use a uniform UV tile atlas: every selected tile receives the full bake Output Size,
including rectangular output sizes. Changing a texture-set or tile working resolution preserves
the stored bake pixels. Materials, mesh-map previews, and exports sample those pixels through
the working tile layout. Older project files load their baked pixels as-is without layout
conversion; rebaking restores correct mapping and detail.

`bake.tile.set {ts?, tile: 1002, enabled: false}`
excludes one UV tile; `textureSet.inspect` / `session.inspect` expose each tile's `bakeEnabled`.
Tile choices are saved and undoable. Unchecked tiles retain their previous baked pixels, resampled
if output size changes, or neutral values when no previous map exists. A running bake uses the
tile selection captured when its worker started. Sets with no checked tiles are skipped; their
geometry still contributes to occlusion when baking other tiles.

`view.solo {layer?, ignoreMask?}` shows one layer alone in the viewport, as if the other layers
of its texture set were hidden (`layer` defaults to the selected layer, `layer: null` ends the
solo; `ignoreMask: true` draws the layer, and the folders around it, at full coverage; other
texture sets are unchanged). It changes only the view: nothing enters the undo history, the
project file or an export (`file.exportTextures` and the CLI export the whole stack, then the solo comes
back), and removing the soloed layer ends it. `session.inspect` reports it as `solo`
(`{layer, ignoreMask}` or `null`).

## Control channel (desktop app)

```sh
texturecraft --control 7878
# stderr: texturecraft: control token written to <temp>/texturecraft-control-7878.token
```

Loopback TCP (`127.0.0.1` only), one JSON object per line each way. Requests are
`{"id", "method", "params"}`; replies are `{"id", "ok": true, "result"}` or
`{"id", "ok": false, "error"}`. Requests are limited to 8 MiB.

**Authentication.** The first request on every connection must be

```json
{"id": 0, "method": "auth", "params": {"token": "<token>"}}
```

The token is `TEXTURECRAFT_CONTROL_TOKEN` when that is set (16 to 1024 printable ASCII
characters; surrounding whitespace is ignored), otherwise a fresh random 256-bit token the app
writes to `<temp>/texturecraft-control-<port>.token` (`<temp>` is `%TEMP%` on Windows, `$TMPDIR`
or `/tmp` elsewhere). On Unix the file is created new and readable by its owner only; when an
existing file there cannot be replaced (another user's, in a shared `/tmp`), the control server
stays off: set `TEXTURECRAFT_CONTROL_TOKEN` instead. Clients (`texturecraft-cli mcp --bridge`)
only read a token file that is a regular file owned by them with no group or other permissions.

A connection must authenticate within 10 seconds. Any other first request, or a wrong token, is
refused and the connection closed. At most 16 connections are served at once.

Scripted runs (agents, recordings, tests) can set `TEXTURECRAFT_NO_PREFS` (any value) so the app
neither reads nor overwrites the user's saved layout and settings: it starts from the defaults and
keeps its state in a throwaway file.

| Method | Params | Result |
|---|---|---|
| `ping` | | `"pong"` |
| `command.run` | `{id, params?}` | the command's JSON result |
| `command.list` | `{filter?}` | `[{id, label, menu, shortcut, params, enabled}]` |
| `session.inspect` | | project, mesh, texture sets, layer stack, tool, camera, display, history |
| `ui.inspect` | | UI state, viewport rect, fps, bake progress (`baking`, 0..1), the running bake's stage (`bakeStatus`, e.g. "Baking Ambient Occlusion (2/4)") and live preview (`bakePreview`, see *Live bake preview*), toasts, `titleBar` (who draws the title bar this run: `app`, `integrated` on macOS, or `system`) |
| `ui.menu.list` | | the menu catalog: `{menu, submenu, label, id, shortcut, implemented, enabled}` |
| `ui.menu.invoke` | `{id}` | behaves like a menu click (may open a window; `ui.resetUi` opens its question). `{"id": "ui.layout.apply", "name": "<layout>"}` switches to a panel layout saved under Window > Layouts (panels only; mode, zoom and wireframes stay) and errors when no layout has that name |
| `ui.reset` | | resets the panels, view layout, zoom and every other view setting at once, without the question the Reset UI menu item asks; settings and shortcuts stay |
| `ui.bake.start` | `{ts?: index \| [indices]}` (default: the sets ticked in Bake mode) | Opens Bake mode and bakes in the background like the Bake button: the 3D view shows the maps landing on the model as they finish (see *Live bake preview*). → the `bake.preview` state. Errors when a bake is already running |
| `ui.bake.cancel` | | Stops the running bake; the set being baked keeps the maps it had, sets already finished stay applied, and the previous view mode comes back |
| `ui.set` | fields of the UI state, e.g. `{"layout": "Only3D"}`; `dock` is the panel layout, a craft-ui docking `Workspace` (`{version, layout: {root, floating}, hidden}`; `root` is a tree of `Split`, `Tabs` and `Stack` nodes over the panels `Viewport`, `Assets`, `TextureSetList`, `Layers`, `TextureSetSettings`, `Properties`, `Display`, `Shader`, `History` and `Log`; `hidden` lists where each closed panel was, so reopening it puts it back; a layout that does not read or has no docked `Viewport` becomes the default), `prefs` the settings (`{"language": "ja", "viewport_texture": 1, "brightness": "light", "accent": "blue"}`; missing fields take their defaults; `brightness` is `dark`, `mediumDark` (default), `mediumLight` or `light`, `accent` `copper` (default), `coral`, `blue`, `violet`, `green`, `amber` or `pink`, applied on the next frame; `system_title_bar` `true` has the system draw the title bar from the next start), `keymap` the custom shortcuts (`{"file.save": "F5"}`, `""` unbinds) | `null` |
| `viewport.screenshot` | `{view?: "3d" \| "2d"}` | `{width, height, png}` (PNG, base64) |
| `ui.screenshot` | | `{width, height, png}` of the whole window, after the next frame |
| `ui.resize` | `{width, height}` | Sets the window's inner size in points (640–7680), leaving maximized state; for recordings and screenshots at a fixed size |
| `debug.paintTest` | `{from: [x, y], to: [x, y], frames?: 120}` | Paints a straight stroke through the real frame loop (viewport pixels, one point per frame) for performance work |
| `debug.frames` | | Frame timing since the last call (then cleared): `{frames, intervalMs, uiMs, syncMs, logicMs}` each `{avg, p95, max}`; `syncMs` is compositing plus display tile conversion, `logicMs` input, control requests and scripted strokes before the UI is built; `worst` breaks down the slowest frame; `strokeEnd` lists the frame that committed a `debug.paintTest` stroke and the one after it (null without one) |

Example session:

```text
> {"id":0,"method":"auth","params":{"token":"3f9c…"}}
< {"id":0,"ok":true}
> {"id":1,"method":"command.run","params":{"id":"layer.add.fill","params":{"color":"#336699","roughness":0.2}}}
< {"id":1,"ok":true,"result":{"layer":7}}
> {"id":2,"method":"command.run","params":{"id":"mask.add.black"}}
< {"id":2,"ok":true,"result":null}
> {"id":3,"method":"viewport.screenshot","params":{"view":"3d"}}
< {"id":3,"ok":true,"result":{"width":1210,"height":788,"png":"iVBORw0KGgo…"}}
```

### Live bake preview

A bake started in the app (`ui.bake.start`, or the Bake button) makes the mesh maps one after
another, in the `BakedMap` order of the set's enabled maps, and shows each one on the model as
soon as it is finished and padded: the 3D view displays the newest finished map raw (normals as
RGB, scalar maps as gray, ID maps as their colors), maps and texture sets not baked yet show their
neutral value, and the bake bar names the stage (`Baking Ambient Occlusion (2/4)`). Each map is a
bake of its own, so with a high poly the transfer rays are cast once per map; the pixels are the
same as a single bake's. `bake.run` and the CLI bake all maps at once and never show a preview.

The streamed maps are held by the session, not the document: nothing enters the undo history
until a texture set's bake is applied (one undo step); a cancelled bake leaves the set it was baking
as it was, while sets that finished before the cancel stay applied. When
the bake ends, is cancelled or Bake mode is left, the view mode that was in use comes back.
Bake setup overlays are hidden during the preview so they cannot cover the maps; their visibility
settings take effect again when the preview ends.
**Painting is paused while a bake is shown**: strokes and polygon fills fail with "Painting is paused while a bake
is shown. Cancel the bake or wait for it to finish."

`bake.preview` `{show?: "follow" \| "material" \| "map:<id>"}` reads and sets what the view shows
(also the control in the bake bar): `follow` (default) the newest finished map, `material` the lit
material, `map:<id>` one map pinned (neutral until it is baked). Picking a mesh map with
`viewport.view` during a bake pins it; any other view mode is kept for when the bake ends. Result,
also `session.inspect`'s `bakePreview`:

```json
{"active": true, "show": "follow", "shown": "ambientOcclusion", "newest": "ambientOcclusion",
 "restore": "Material", "stage": {"set": 0, "map": "curvature", "n": 3, "total": 4},
 "sets": [{"set": 0, "order": ["id", "ambientOcclusion", "curvature", "height"], "done": ["id", "ambientOcclusion"]}]}
```

`{"active": false}` when no bake is shown; setting `show` then is an error. Drive a bake with
`ui.bake.start` and poll `ui.inspect`'s `bakeStatus` / `bakePreview` (and `viewport.screenshot`)
until `baking` is `null`.

A minimal client (Python, for illustration):

```python
import json, os, socket, tempfile
port = 7878
token = os.environ.get("TEXTURECRAFT_CONTROL_TOKEN") or open(
    os.path.join(tempfile.gettempdir(), f"texturecraft-control-{port}.token")).read().strip()
s = socket.create_connection(("127.0.0.1", port)); f = s.makefile("rw")
def call(method, params=None, _id=[0]):
    _id[0] += 1
    f.write(json.dumps({"id": _id[0], "method": method, "params": params or {}}) + "\n"); f.flush()
    reply = json.loads(f.readline())
    if not reply["ok"]: raise RuntimeError(reply["error"])
    return reply.get("result")
call("auth", {"token": token})
print(call("command.run", {"id": "layer.add.fill", "params": {"color": [0.8, 0.1, 0.1]}}))
```

The UI's `prefs.project_open_limit_gib` sets the maximum decompressed project data accepted on open
(default 8 GiB, minimum 1 GiB). Set it with `ui.set {"prefs":{"project_open_limit_gib":16}}`;
it takes effect before the next control request, without waiting for a frame, and persists with
the UI preferences. Individual entry limits still apply. This controls opening only, independently
of `prefs.reserved_cores` and `prefs.save_ram_percent` for saving.
Settings bounds the reserved-core input to the available logical CPU count and shows the estimated
save budget in GiB beside the RAM percentage. The readout refreshes once a second from currently
available RAM; a save calculates its budget when it starts.

## MCP server

`texturecraft-cli mcp` speaks MCP on stdin/stdout (stdout carries only protocol messages).

- **Headless** (default): an in-process session that starts on the material ball (1024).
  Files are reachable only below the roots given at launch, as forward-slash paths relative to
  them: `--read-root <dir>` for `project_open` / `mesh_import` / `mesh_reimport` / `bake_mesh_add`,
  `--write-root <dir>` for `project_save` / `material_save` / `textures_export` / `mesh_export`
  (`--automation-read-root` / `--automation-write-root`, as in the other crafting apps, are the
  same flags). Without roots there is no file access. `..`, absolute
  paths, drive letters, device names and symlink or junction escapes are refused.
- **Bridge**: `--bridge <port>` forwards to a running `texturecraft --control <port>` (token from
  `--token`, `TEXTURECRAFT_CONTROL_TOKEN` or the app's token file). The app reads and writes its
  own files, so the path tools are unavailable there; `project_save` without a path invokes
  File › Save.

Through `command_run`, every `file.*` command but `file.close` and `file.removeUnused` (and
`file.reimportMesh` without a `path`) is refused, and so is any command given a `path` or `dir`, at the top level or in
a `files` entry (`resource.import`, `material.save`, `material.load`, `textureSet.meshMap.import`, …), in both modes: use the workspace tools below. `resource.text`
takes a font family, not a path: it reads only the system's font folders.

`file.close` leaves no project open: `session.inspect` then reports `projectOpen: false`, `command.list` marks every command but the project-independent ones as not enabled: the ones that open or create a project (`file.open`, `file.new`, `project.new`, `project.sample`), `project.inspect`, `viewport.frame`, `project.templates`, the `export.preset*` commands (`export.presets`, `export.preset.get`, `export.preset.save`, `export.preset.remove`) and the library and preset listings (`brush.preset.list`, `alpha.list`, `filter.list`, `material.list`, `mask.smart.list`, `text.fonts`), each still subject to its own enabled check. The rest are refused with "no project is open" until a project is opened or created. A `texturecraft --control` run starts with the sample project (an empty material ball with `--empty`); the app without `--control` starts with no project.

| Tool | Params | Notes |
|---|---|---|
| `session_inspect` | | The whole session as JSON |
| `command_list` | `{filter?}` | Command ids, labels, params docs, enabled now |
| `command_run` | `{id, params?}` | Any command by id |
| `command_batch` | `{steps: [{id, params?}]}` | Stops at the first error: `{completed, results, failed: {index, id, error}}` (a tool error) |
| `project_new` | `{mesh?, resolution?, name?, template?}` | Built-in mesh: material_ball, sphere, cube, rounded_cube, cylinder, torus, plane, crate; `template` names a project template (`project.templates`) |
| `project_open` | `{path}` | `.tcraft` or a mesh file, relative to the read root |
| `project_save` | `{path?}` | `.tcraft` (appended if missing), relative to the write root |
| `mesh_import` | `{path, resolution?}` | New project from .glb/.gltf/.fbx/.dae/.usd/.usda/.usdc/.usdz/.abc/.obj/.ply/.stl; a .gltf's buffers and images and a loose USD layer's textures are read from its folder |
| `mesh_reimport` | `{path?, preserveStrokes?}` | Reimport the mesh from a file, or (no path) from the project's own copy; bridge mode supports only the latter |
| `mesh_export` | `{path, format?}` | The mesh as obj, glb, usda, usdc or usdz below the write root (obj: its .mtl and textures, usda / usdc: textures next to it) → `{path, textures?}` |
| `bake_mesh_add` | `{path, cage?}` | Add a high-definition mesh for baking, or set the cage → `{name, meshes}` |
| `resource_import` | `{path}` | Image resource (png, jpg, tga, tif, bmp, webp, exr, hdr) → its name, for `layer.add.image` / `tool.stencil`, or as a mesh map instead of baking it: `textureSet.meshMap.set {ts?, map, image}` (`textureSet.meshMap.clear {ts?, map}` empties the slot); call it once per baked map file (the desktop app's `textureSet.meshMap.import`, which takes host paths, is refused through `command_run`) |
| `material_save` | `{path, layer?, name?}` | A layer or folder as a `.tcmat` smart material below the write root |
| `material_load` | `{path}` | Add a `.tcmat` smart material, a MaterialCraft `.mcmat` material or a `.mcraft` package above the selection |
| `textures_export` | `{dir, preset?, ts?, format?, padding?, size?}` | Writes files below the write root |
| `bake` | `{ts?, resolution?, maps?, cpu?}` | Mesh maps for generators and smart materials; rays on the GPU when there is one (`cpu: true`: on the CPU). Per-map ray settings (`ao`, `thickness`, `bentNormal`: rays, maxDistance, spread, distribution; AO ignoreBackface, selfOcclusion) are `bake.settings` parameters |
| `paint_stroke` | `{points, viewport?, color?, roughness?, metallic?, height?, size?, hardness?}` | Paint tool + brush material + `tool.stroke`; points in viewport pixels |
| `viewport_render` | `{view?: "3d" \| "2d", width?, height?, yaw?, pitch?, samples?, bounces?, cpu?}` | Headless: PNG image (≤ 2048 px a side); yaw/pitch in degrees for this render only; `samples` (1–4096) path traces the 3D view instead, on the GPU when it has compute shaders, else (or with `cpu: true`) on the CPU (no GPU needed) |
| `viewport_screenshot` | `{view?}` | Bridge: the app's 3D or 2D view |
| `ui_screenshot` | | Bridge: the whole window (downscaled to 2048 px) |
| `ui_inspect` | | Bridge: UI state |
| `ui_menu_invoke` | `{id}` | Bridge: a menu click |

Replies are capped: 8 MiB of JSON, 5 MiB per PNG, 256 batch steps. Stroke points and render
sizes share one space: the default render size is the session viewport (1280×800), so
`paint_stroke` points can be read straight off a `viewport_render` image.

A typical agent run: `project_new {"mesh": "sphere"}` → `command_run layer.add.fill
{"color": "#336699", "roughness": 0.2}` → `bake {"resolution": 512}` → `command_run
mask.add.black` → `command_run effect.add.generator {"generator": "edgeWear"}` →
`session_inspect` → `viewport_render` → `project_save {"path": "ball.tcraft"}` →
`textures_export {"dir": "textures", "preset": "glTF PBR Metal Roughness"}`.

### Claude Code

```sh
claude mcp add texturecraft -- texturecraft-cli mcp --read-root . --write-root .
# bridged to the running app:
claude mcp add texturecraft-app -- texturecraft-cli mcp --bridge 7878
```

or a project `.mcp.json`:

```json
{
  "mcpServers": {
    "texturecraft": { "command": "texturecraft-cli", "args": ["mcp", "--read-root", ".", "--write-root", "."] }
  }
}
```

### Claude Desktop

In `claude_desktop_config.json` (Settings › Developer › Edit Config), with absolute paths:

```json
{
  "mcpServers": {
    "texturecraft": {
      "command": "C:\\Program Files\\TextureCraft\\texturecraft-cli.exe",
      "args": ["mcp", "--read-root", "C:\\Users\\me\\Texturing", "--write-root", "C:\\Users\\me\\Texturing"]
    }
  }
}
```

## Headless JSON-lines server

`texturecraft-cli serve` keeps one headless session open and answers the control channel's
envelope on stdin/stdout (no authentication: the caller started the process). With
`--port <port>` it listens on `127.0.0.1:<port>` instead, with the same `auth` handshake and
token rules as the app (a generated token goes to `<temp>/texturecraft-control-<port>.token`).
File access follows `--read-root` / `--write-root` as for MCP.

| Method | Params |
|---|---|
| `ping`, `methods`, `session.inspect` | |
| `command.run` | `{id, params?}` |
| `command.list` | `{filter?}` |
| `command.batch` | `{steps: [{id, params?}]}` |
| `project.open` | `{path}` |
| `project.save` | `{path?}` |
| `mesh.import` | `{path, resolution?, name?}` |
| `mesh.reimport` | `{path?, preserveStrokes?}` |
| `mesh.export` | `{path, format?}` |
| `bake.mesh.add` | `{path, cage?}` |
| `resource.import` | `{path}` |
| `material.save` | `{path, layer?, name?}` |
| `material.load` | `{path}` |
| `textures.export` | `{dir, preset?, ts?, format?, padding?, size?, bitDepth?, dilation?, normalFormat?}` |
| `render` | `{view?, width?, height?, yaw?, pitch?}` → `{width, height, png}` |

Brush preset operations are engine commands. Send each through `command.run` with the shown
`id` and `params`, for example `{"id":1,"method":"command.run","params":{"id":"brush.preset.list","params":{}}}`.

| `command.run` id | `params` and result |
|---|---|
| `brush.preset.list` | `{}` → `[{id, label, category, builtin}]`: the built-in brush presets, then the user's |
| `brush.preset.apply` | `{preset}`: a built-in sets the stroke and keeps the size; a user preset sets the whole brush (size too) and the painted channels |
| `brush.preset.store` | `{name, overwrite?}`: saves the current brush and painted channels as a user preset (desktop and trusted local CLI: a `.tcbrush` file in the data folder's `brushes` folder; browser and untrusted headless MCP/RPC: current session only); an existing name needs `overwrite: true`, built-in names are read-only |
| `brush.preset.rename` | `{name, to}` |
| `brush.preset.delete` | `{name}` |
| `brush.preset.save` / `brush.preset.load` | `{path}`: save the brush as a loose `.tcbrush` file; load a loose or named preset's stroke, keeping the current size and painted channels |

```sh
printf '%s\n' '{"id":1,"method":"command.run","params":{"id":"project.new","params":{"mesh":"torus"}}}' \
              '{"id":2,"method":"render","params":{"width":512,"height":512}}' | texturecraft-cli serve
```

## PBR texture folders

`file.importPbrFolder` turns a folder, a `.zip` or a list of image files of PBR maps into one fill layer
(one undo step), the way `file.importMaterialX` does for a `.mtlx`. `file.scanPbrFolder` takes the same
`path` / `files` and reports what would be used without changing anything. Both are `file.*` commands, so
they run over the control channel and the CLI, not through MCP `command_run` (which refuses `file.*`
commands and host paths).

| Command | Params |
|---|---|
| `file.importPbrFolder` | `{path \| files, name?, normalFormat?: auto\|openGl\|directX, projection?: uv\|triplanar\|planar..., scale?, choose?: {map: file}}` → `{layer, name, maps, candidates, normalFormat, channels, resources, ignored, warnings}` |
| `file.scanPbrFolder` | `{path \| files, name?, normalFormat?, choose?, projection?, scale?}` → `{name, maps, candidates, normalFormat, ignored, channelsAvailable}` |

Maps are found by file name, case-insensitively, from the last known word in the name; resolution
(`2k`, `4096`), bit depth and other tokens are skipped:

| Map (`maps` key) | Words |
|---|---|
| `baseColor` | `basecolor`, `base_color`, `albedo`, `diffuse`, `diff`, `col`, `color`, `bc`, `basemap`, `maintex` |
| `roughness` | `roughness`, `rough`, `rgh` |
| `glossiness` (inverted into roughness; a roughness file wins) | `gloss`, `glossiness`, `smoothness` |
| `metallic` | `metallic`, `metalness`, `metal`, `mtl` |
| `normal` | `normal`, `nrm`, `nor`, `norm`, `normalmap`, `bumpmap`, a trailing `n` |
| `height` | `height`, `disp`, `displacement`, `hgt`, `bump` (weakest) |
| `ambientOcclusion` | `ao`, `ambientocclusion`, `ambient_occlusion`, `occlusion`, `occ` |
| `emissive` | `emissive`, `emission`, `emit`, `glow` |
| `opacity` | `opacity`, `alpha`, `transparency` |
| `orm` (split: R occlusion, G roughness, B metallic; separate files win) | `orm`, `arm`, `ao_rough_metal` |

- **Normal convention.** `gl` / `opengl` in the name is OpenGL, `dx` / `directx` is DirectX (the layer's
  normal source is flagged DirectX, so its green is flipped where it is sampled). With neither the map is
  OpenGL. `normalFormat: openGl|directX` forces every normal map; `auto` (the default) reads the name.
- **Ignored**: other files, dot files and `__MACOSX`, preview images (`preview`, `thumb`, `thumbnail`,
  `sphere`, `render`, `swatch`, `icon`), and maps with no channel here (cavity, curvature, specular...).
  Each is listed in `ignored` with the reason.
- **Several files for one map** (resolutions, formats, GL and DX): the OpenGL normal, then the highest
  resolution token, then the largest file wins. A programmatic call never asks; `candidates` lists the
  alternatives and `choose: {"baseColor": "file.png"}` picks one (the app's dialog shows a drop-down).
- **Where.** A folder's own files if they hold maps; otherwise the one sub-folder that does (several are
  an error that names them). The layer is named after the folder, archive or shared file-name prefix.
  The import reply's `name` is the created layer's unique name, including its numeric suffix.
  A blank `name` uses the same source-derived default as an omitted name.
- **Channels** the texture set doesn't have are skipped with a warning. Projection defaults to UV
  (`scale` 1); `projection: triplanar` is the option for models without good UVs.
- **Hostile input.** Links, pipes and devices are rejected as sources. A scan admits at most 4096
  directory entries (including subdirectories), selected files, or ZIP entries. Folder traversal is
  limited to four levels below the root; exceeding a scan limit returns an error instead of a partial list.
  A file or ZIP entry is at most 256 MiB, an archive 512 MiB, an image 64 Mi texels (320 Mi per import).
  Actual reads are capped even if a file grows. Decoded sources and outputs also share an estimated
  memory budget: one quarter of available memory, capped at 4 GiB (1 GiB for the byte API on the web).
  Compressed inputs already held by the caller and existing project/GPU memory are outside that estimate.
  An image that exceeds the budget or cannot decode is skipped with a warning; if none decodes the
  import is an error that changes nothing. `maps` describes detection; `channels` and `resources`
  describe the successful import. Scan validates import options without decoding pixels.

In the app: File › Import PBR texture folder... or drop a folder, a `.zip` or several map files on the
window. The dialog shows the maps found, lets you pick between candidates, set the normal convention and
projection, and keep the material in the library (shelf › Materials).

## Smart masks

`mask.smart.list` lists the built-in smart masks and `mask.smart.apply {name, layer?}` replaces a
layer's mask with one. A user smart mask is a layer's mask saved as a `.tcmask` file (the `.tcraft`
container holding one layer with the mask): all effects with their parameters, painted pixels and the
images they read.

| Command | Params |
|---|---|
| `mask.smart.create` | `{layer?, path, name?}`: save the layer's mask (an instance without its own mask saves its source's) to `path` (`.tcmask` is appended if missing, folders are made); `name` defaults to the layer's name. Returns `{path}` |
| `mask.smart.apply` | `{path, layer?}`: replace the layer's mask with the one in a `.tcmask` file (one undo step; images it needs are added to the project, painted pixels are fitted to the texture set) |

In the app: right-click a layer > *Create smart mask* saves into the library folder next to the smart
materials (never over an earlier file), and the shelf lists it under *My smart masks* (Smart masks tab,
*Your assets*): click it or drop it on a layer. Like `material.save`, both forms with `path` are not
available through `command_run` on the MCP server, which rejects host paths.

## Geometry decals

`layer.add.decal {image, ...}` places an image as a decal and `layer.decal.set {layer? | effect?, ...}` edits
one (`command.list` shows every param). Three of them keep the image's look:

- `keepProportions: true` keeps the image's aspect (the decal's current ratio when it shows no image). A
  height (`size: 0.2`) sets the width; a `[w, h]` pair or no `size` sets the height from the width. It acts
  in that call only and is not saved: the desktop Properties panel's "Keep proportions" box (on for every
  decal) sends it with each Width or Height edit.
- `flipU` / `flipV`: mirror the image left to right / top to bottom inside the frame. They are saved with
  the decal (`flip: [u, v]`, left out while both are false, so older projects load and save unchanged).

`layer.add.decal {image, position?, normal?, up?, size?, rotation?, depth?, symmetry?, symmetryFlip?}`
places an image on the surface; `layer.decal.set` moves, resizes or turns it later. `symmetry`
(`x`, `y`, `z` or `off`) adds a mirrored copy on the other side of the mesh: the decal reflected
through the plane across the mesh center, perpendicular to that axis. The copy is part of the
decal, not a second layer, so it follows every later edit and is saved with the project.
`symmetryFlip` (`none`, `u`, `v`) additionally flips the copy's image (U swaps left and right,
V swaps top and bottom). `symmetryOffset` moves the mirror plane along the axis, in world units from the mesh center (default 0, kept within the mesh, as the brush symmetry's offset). With a copy on, each of the three params may be sent alone and keeps the others.
`session.inspect` shows the setting as `decal.symmetry` (`{axis: 0|1|2, flip, offset?}`, absent without
a copy).

## Camera bookmarks

Saved cameras (`Project.cameras`, stored in the `.tcraft` file) are the camera bookmarks.
`camera.add {name?}` saves the current view, `camera.list` lists them (`current` is the one the view
is at), `camera.select {index? | name?}` restores one, `camera.rename {index? | name?, to}` and
`camera.remove {index? | name?}` edit them. `camera.bookmark1` … `camera.bookmark9` (Ctrl+1 … Ctrl+9)
go to the first nine. Commands jump straight to the view; the desktop UI glides there over about a third
of a second.

## Framing the view

`viewport.frame {target?: "all" | "selection"}` points the camera at the bounding box of the whole mesh
(`"all"`, the default, F) or of the active texture set's geometry only (`"selection"`), keeping the
current angles. The orbit pivot (`camera.get` `target`) is the framed center, so `camera.orbit` and the
Alt+drag orbit turn around the object being painted even with a floor plane or other helper geometry in
the mesh. `viewport.frameSelection` (Shift+F) is the same as `target: "selection"`; a set with no triangles
frames the whole mesh. Both return the camera like `camera.get`.

## Camera views and projection

`camera.view {view}` looks at the mesh from one side (`front` from +Z, `back`, `left` from -X,
`right`, `top` and `bottom` at the orbit's ±89° pitch limit) and frames it; the projection stays.
`camera.view.front` … `camera.view.bottom` do the same without params (Viewport menu, no default
shortcut, bindable in Settings ▸ Keyboard). `camera.perspective` (F5) and `camera.orthographic` (F6)
change only the projection; position and rotation stay. None of them is an undo step. In the 3D
view, Alt+Shift+left drag orbits in 90° steps.

## MaterialCraft materials

A `.mcmat` portable material (MaterialCraft's format, read here by `formats::mcmat`; the manifest is
documented in MaterialCraft's `docs/control-protocol.md` § Portable materials) becomes a fill layer
whose channel sources are its cooked maps and whose `link` (shown by `project.inspect`) holds the
graph's exposed inputs, presets and image inputs. Both apps use the shared `craft-mcmat` container.
Versions 1 and 2 are accepted; version 1 contains a legacy JSON `.mcraft` source that current
MaterialCraft cannot reopen, so its cooked maps remain usable but re-cooking needs a compatible older version.

| Command | Params |
|---|---|
| `material.load` | `{path, above?, inside?, index?}`: `.tcmat`, `.mcmat` or `.mcraft` (cooked first) |
| `material.link.set` | `{layer?, id, value, cook?: true}`: a number, `[..]`, a bool, `"#rrggbb"` or a typed `{"t", "v"}`; re-cooks unless `cook: false` |
| `material.link.preset` | `{layer?, name, cook?: true}` |
| `material.link.cook` | `{layer?}`: re-cook at the texture set's resolution with its baked mesh maps (after a bake or a resize) |
| `material.link.saveSource` | `{layer?, path}`: the `.mcraft` or `.sbs` source, to open in MaterialCraft |

Re-cooks use `materialcraft-cli batch` to open the saved graph by name and run `file.exportMaterial` with its size, values and inputs:
`$MATERIALCRAFT_CLI`, else `materialcraft-cli` next to the TextureCraft executable, else on `PATH`
(the app is `$MATERIALCRAFT` / `materialcraft`). Baked maps feed graph input nodes whose identifier
matches the map's shared id (`ambientOcclusion`, `curvature`, `thickness`, `position`,
`worldNormal`, `normal`, `height`, `id`, `bentNormal`, `opacity`, `colorMap`, `worldDirection`;
case and underscores ignored; `normal` and `bentNormal` are tangent space, OpenGL green). Usages
map to channels of the same id (`baseColor`, `normal` in OpenGL convention, `roughness`, `metallic`,
`height`, `ambientOcclusion`, `emissive`, `opacity`, `specular`); missing channels join the texture
set.

## CLI

```sh
texturecraft-cli --version                                   # TextureCraft 0.1.0 (abc1234 2026-10-14)
texturecraft-cli info model.glb                              # triangles, materials, UV report
texturecraft-cli info project.tcraft                         # texture sets, layers, baked maps
texturecraft-cli commands --filter mask
texturecraft-cli new builtin:material_ball ball.tcraft --resolution 1024 --sample   # mesh maps baked at ≤ 1024 px
texturecraft-cli new model.glb model.tcraft --resolution 2048
texturecraft-cli run ball.tcraft layer.add.fill '{"color":"#c08040","metallic":1}' -o ball.tcraft
texturecraft-cli batch script.json
texturecraft-cli bake model.tcraft --resolution 2048 --maps ambientOcclusion,curvature,thickness
texturecraft-cli export model.tcraft out/ --preset "Unreal Engine (Packed)" --format png --size 2048
texturecraft-cli render ball.tcraft ball.png --size 1280x800 --view 3d --yaw 30 --pitch 15
texturecraft-cli render scene.tcraft hero.png --size 1200x900 --camera Hero --samples 2048   # a saved camera (camera.add), path traced
```

`<input>` is a `.tcraft` project, a mesh file or `builtin:<id>`. A batch script is a list of
steps or an object with an optional input and output (paths relative to the current directory):

```json
{
  "open": "builtin:sphere",
  "steps": [
    {"id": "layer.add.fill", "params": {"color": [0.6, 0.6, 0.62], "metallic": 1, "roughness": 0.3}},
    {"id": "bake.run", "params": {"resolution": 512}},
    {"id": "material.apply", "params": {"material": "iron_rusted"}}
  ],
  "save": "rusty.tcraft"
}
```

Errors print `texturecraft-cli <command>: error: …` and exit 1; usage errors exit 2. The CLI is a
console program and never opens a window; `render` needs a GPU or software adapter.
