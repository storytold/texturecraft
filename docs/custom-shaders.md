# Custom GLSL shaders

Shader Settings can import a `.glsl` file, find it by name in the shader picker,
and select it for the 3D material viewport. The desktop and browser file pickers
use the same engine commands as the CLI and MCP server. Selecting `open-pbr`
returns to the built-in material shader.

Available shaders show sphere previews using the app's GPU device. One shared
preview renderer processes one sphere per frame and caches the bounded set of
results. Previews use a neutral sphere material and the shader's current
parameters and image assignments. A GPU rejection is shown as an unavailable
preview with the error in its tooltip.

Imported source, parameter values, image assignments and the instance name are
saved inside the `.tcraft` project. Import, selection, edits and restoring
defaults support undo and redo. Reopening recompiles the embedded source;
cached GPU code and control metadata are not trusted.
Image resource renames update shader assignments, and removing unused resources
keeps images assigned to any imported shader, including inactive shaders.

The adapter accepts annotated surface shaders with `void shade(V2F inputs)`.
Its interface follows the published
[surface shader API](https://adobedocs.github.io/painter-shader-api/), with
TextureCraft implementations of the supported library functions. It does not
load external library files or include shader assets from other products.

See [banded.glsl](../examples/shaders/banded.glsl) for an original example with
an integer slider, color, checkbox, material-channel input and image input.
Custom uniforms need a `//: param custom` JSON annotation with a `default`:

```glsl
//: param custom { "default": 3, "label": "Light bands", "min": 2, "max": 8 }
uniform int bands;
```

Supported parameter types are `bool`, `int`, `float`, `vec2`–`vec4` and
`ivec2`–`ivec4`. An integer `values` object supplies combobox choices. Optional
`label`, `group`, `description`, `widget`, `min` and `max` metadata describe
controls. Values are checked before editing the project. Custom `sampler2D`
inputs use their `default_color` until assigned an imported image; their `default`
must be the empty string (named default resources are unsupported). Automatic
`channel_user0`–`channel_user7` inputs are also assignable.

Material bindings include base color, roughness, metallic, ambient occlusion,
normal, height, emissive, opacity and specular level. Baked curvature, thickness
and ambient occlusion are available through their automatic texture bindings.
Camera, environment and key-light data come from the current viewport.

The recognized imports are `lib-defines`, `lib-sparse`, `lib-sampler`,
`lib-normal`, `lib-vectors`, `lib-utils`, `lib-pbr`, `lib-env`, `lib-emissive`
and `lib-alpha-test`, each with the `.glsl` extension. These names select an
adapter subset, not complete implementations of every function in those
libraries. Unsupported functions, imports, uniforms and GPU resources cause an
import error. Shader state annotations produce a warning because the viewport
uses its own render state.

Library lighting uses TextureCraft's environment and normal-map conventions.
Specular lighting and occlusion are approximations, and bent-normal specular
lighting is unavailable. Image inputs are resampled to at most 512 pixels per
side without mipmaps. Custom GLSL runs in the 3D material viewport; 2D and
channel views retain their existing display modes. Path tracing reports that
the selected custom shader is unavailable. The panel and automation results
explain these limits; compilation alone does not establish a visual match to
another renderer.

Imports are bounded to 1 MiB of UTF-8 source, 128 numeric parameters, 16 image
inputs and 16 shaders per project. Shader compilation performs no filesystem
access. GPU pipeline rejection is reported without installing the rejected
pipeline.

| Engine command | Parameters |
| --- | --- |
| `shader.import` | `{name, source}` or `{path}`; adds a validated shader |
| `shader.list` | `{}`; selected name, instance name, parameters, image inputs and support limits |
| `shader.select` | `{name}`; imported name or `open-pbr` |
| `shader.set` | `{parameters?: {uniform: value}, textures?: {input: resourceName \| null}, instanceName?, reset?: bool}` |

For MCP, use `shader_import {path}` to read a file below the configured
automation read root. Use `command_run` for selection and parameter changes;
`shader.import {name, source}` also works through `command_run`. Host paths
passed to `command_run` are refused. The JSON-lines RPC method is
`shader.import {path}`. Import does not select a shader automatically through
these programmatic interfaces; the GUI selects it after a successful import.

```sh
texturecraft-cli run ball.tcraft shader.import '{"path":"examples/shaders/banded.glsl"}' -o ball.tcraft --in-place
texturecraft-cli run ball.tcraft shader.select '{"name":"banded"}' -o ball.tcraft --in-place
texturecraft-cli run ball.tcraft shader.set '{"parameters":{"bands":5}}' -o ball.tcraft --in-place
texturecraft-cli render ball.tcraft banded.png --view 3d --size 640x480
```
