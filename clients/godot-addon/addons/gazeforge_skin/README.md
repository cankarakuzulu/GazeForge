# gazeforge_skin (Godot 4 addon)

Reads the `GAZEFORGE_skin` glTF material extension: layered character skin whose layers (grime, blood, paint, ...) a game
drives at run time. Format: [GAZEFORGE_skin](../../../../extensions/GAZEFORGE_skin/README.md).

For every glTF material that carries the extension, the addon builds a `ShaderMaterial` and puts it on every mesh surface
that used that material. The shader is generated per material: each live layer becomes a `float` uniform named by the
layer's `parameter` (and one for its variant's `parameter`), starting at the layer's `default`. The layer math is the
format's "Masks are reach orders" and "Shading" sections, layers in array order. The core material (base colour,
metallic-roughness, normal, occlusion, emissive, their factors, normal scale, occlusion strength, double-sided, alpha
mode) is carried into the same shader; with every layer parameter at 0 no layer covers anything.

Masks are decoded straight from the PNG bytes in the file, so their 8-bit values arrive exactly as stored: no sRGB
decoding, no compression, no alpha-border fix.

Tested with Godot 4.7.2 (Forward+).

## Install

1. Copy `addons/gazeforge_skin` into your project's `addons/` folder.
2. Project > Project Settings > Plugins: enable "GazeForge skin".
3. Import (or reimport) your `.glb` files. Materials with `GAZEFORGE_skin` become `ShaderMaterial`s in the imported scene.

## Driving a layer

```gdscript
var character := preload("res://characters/hero.glb").instantiate()
add_child(character)

GazeForgeSkin.make_unique(character)          # this character gets its own copies of the skin materials
GazeForgeSkin.set_parameter(character, "blood", 0.6)
GazeForgeSkin.set_parameter(character, "blood_dry", 0.3)
GazeForgeSkin.set_parameter(character, "mud", 1.0)
```

| function | does |
|---|---|
| `GazeForgeSkin.set_parameter(root, parameter, value)` | sets `parameter` (clamped to 0..1) on every skin material under `root` that has it; returns how many it set |
| `GazeForgeSkin.make_unique(root)` | gives the meshes under `root` their own copies of the skin materials, in surface materials and in `material_override`; returns how many surfaces now draw with a copy |
| `GazeForgeSkin.materials(root)` | every skin `ShaderMaterial` under `root` |
| `GazeForgeSkin.parameters(material)` | the parameter names of one skin material, layers in order, each variant after its layer |

An imported scene shares its materials between all of its instances. Without `make_unique`, setting a parameter on one
instance changes every instance of that scene.

A tween works as well as a plain call, since the parameter is an ordinary shader uniform:

```gdscript
for m in GazeForgeSkin.materials(character):
    create_tween().tween_property(m, "shader_parameter/blood", 1.0, 2.0)
```

## Loading a glb at run time

The editor plugin only acts on editor imports. To load a `.glb` with `GLTFDocument` in a running game, register the
extension yourself:

```gdscript
var extension := GazeForgeSkinExtension.new()
GLTFDocument.register_gltf_document_extension(extension)

var doc := GLTFDocument.new()
var state := GLTFState.new()
if doc.append_from_file("user://hero.glb", state) == OK:
    add_child(doc.generate_scene(state))
```

## Inspecting the layers

`GazeForgeSkinExtension.debug_mode` set to `"albedo"` or `"roughness"` before loading generates unshaded shaders that
output the layered base colour, or the layered roughness as grey, so a render can be compared with the format's math.
Leave it at `""` for the real material.

## Invalid materials

A material whose extension breaks the format is refused: the addon reports a `push_error` naming the material and the
reason, and the surface keeps the core `StandardMaterial3D`. Refused: a `version` other than 1, a missing `live` array,
a missing or duplicate `name`, a `parameter` (layer or variant) that is malformed or not unique within the material, a
mask `channel` outside 0..3, a mask `index` that is not a texture with a readable PNG image, `spread` outside (0, 1],
`default`, `opacity`, `roughness` or a `colour` component outside [0, 1], and more than four distinct mask textures in
one material. The other materials of the file are still built.

## Limits

- At most four mask textures per material (each holds four layers; layers may also share a channel).
- A `parameter` must also be usable as a Godot shader uniform name: shading-language keywords and built-ins (`float`,
  `ALBEDO`, ...) and the addon's own `gf_`-prefixed names are refused.
- Masks are kept uncompressed (RGBA8 with mipmaps) so they stay exact: a 2048 x 2048 mask texture costs about 21 MiB of
  video memory, a 4096 x 4096 one about 85 MiB.
- The editor import still extracts every embedded image, the masks included, as separate texture files beside the
  `.glb` (Godot's default image handling). The skin materials do not use those copies.
- The core textures are the ones Godot imported for the core material, with Godot's import settings.
- All textures are sampled with linear mipmapped filtering and repeat wrapping; glTF samplers and
  `KHR_texture_transform` are not applied.
- `GLTFDocument` imports with a copy of each registered extension, so a subclass that adds settings must `@export` them
  for them to reach the import.
- Godot 4.7.2 logs four `Parameter "material" is null` errors when a mesh instance that has surface override materials on
  an imported mesh is freed - which `make_unique` sets up. The same happens with any surface override material on an
  imported mesh, a plain `StandardMaterial3D` included; it is logged by the engine, nothing leaks or breaks.
- Tested on Forward+ only; Mobile and Compatibility are untested.

## Tests

`tests/` runs against the fixtures in `tests/assets`:

```
godot --headless --path clients/godot-addon --script res://tests/runtime_test.gd
godot --headless --path clients/godot-addon --import
godot --headless --path clients/godot-addon --script res://tests/import_test.gd
```

The first loads every fixture at run time, the third checks the editor-imported scenes; each prints one PASS or FAIL
line per file and exits non-zero on a failure.
