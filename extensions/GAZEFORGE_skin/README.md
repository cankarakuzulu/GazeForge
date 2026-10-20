# GAZEFORGE_skin

A glTF 2.0 material extension for layered character skin: the material's core properties are the finished, baked skin, so
any glTF viewer shows the character as exported; the extension lists the layers a renderer can change at run time (grime,
blood, paint, ...) and the masks that say where they appear.

Status: version 1. Written by GazeForge's `export-glb`; read by the Godot addon in `clients/godot-addon`.

## Placement

On a material, under `extensions.GAZEFORGE_skin`. The writer lists `GAZEFORGE_skin` in `extensionsUsed` and never in
`extensionsRequired`: a reader that does not know it simply shows the core material.

```json
{
  "name": "head_skin",
  "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "metallicRoughnessTexture": {"index": 2}, ...},
  "normalTexture": {"index": 1},
  "occlusionTexture": {"index": 2},
  "extensions": {
    "GAZEFORGE_skin": {
      "version": 1,
      "look": "medium-warm",
      "baked": [{"name": "dust", "kind": "grime"}],
      "live": [
        {"name": "blood", "kind": "blood", "parameter": "blood", "default": 0.0,
         "mask": {"index": 3, "channel": 0}, "spread": 0.1,
         "colour": [0.22, 0.012, 0.012], "opacity": 0.95, "roughness": 0.25,
         "variant": {"parameter": "blood_dry", "default": 0.0, "colour": [0.06, 0.008, 0.006], "roughness": 0.75}},
        {"name": "mud", "kind": "grime", "parameter": "mud", "default": 0.0,
         "mask": {"index": 3, "channel": 1}, "spread": 0.2,
         "colour": [0.12, 0.08, 0.05], "opacity": 0.9, "roughness": 0.9,
         "variant": {"parameter": "mud_wet", "default": 0.0, "colour": [0.05, 0.035, 0.022], "roughness": 0.3}}
      ],
      "expressions": []
    }
  }
}
```

## Fields

| field | type | meaning |
|---|---|---|
| `version` | integer | 1 |
| `look` | string | the skin look the base was generated from; information only |
| `baked` | array of `{name, kind}` | layers already composited into the core material; information only |
| `live` | array of live layers | applied by the renderer, in array order (a later layer is drawn over an earlier one) |
| `expressions` | array | reserved for expression maps; empty in version 1 |

A live layer:

| field | type | meaning |
|---|---|---|
| `name` | string | unique within the material |
| `kind` | string | `grime`, `blood` or `paint` in version 1; a reader treats an unknown kind like any other layer |
| `parameter` | string | the name the application drives the layer by, matching `[A-Za-z_][A-Za-z0-9_]*`, unique within the material |
| `default` | number in [0, 1] | the parameter's value until the application sets it |
| `mask` | `{index, channel}` | a glTF texture index and a channel, 0..3 = R, G, B, A; the texture holds non-colour data (no sRGB decoding) |
| `spread` | number in (0, 1] | how soft the layer's growing edge is |
| `colour` | three numbers in [0, 1] | linear RGB the layer paints |
| `opacity` | number in [0, 1] | how much of the colour and roughness the fully covered layer applies |
| `roughness` | number in [0, 1] | the layer's roughness |
| `variant` | optional object | a second state: `{parameter, default, colour, roughness}`, e.g. fresh to dried blood, dry to wet mud |

The variant's `parameter` follows the same rules as the layer's and is unique among all parameters of the material.

## Masks are reach orders

A mask texel `m` in [0, 1] says how early the layer reaches that point: `m = 0` is never covered, larger values are covered
first. With the layer parameter `a` in [0, 1]:

```
coverage = clamp((m + a - 1) / spread, 0, 1)
```

At `a = 0` nothing shows; as `a` rises the layer grows outward from the texels with the highest `m`; at `a = 1` every
texel with `m >= spread` is fully covered. So one 8-bit channel both places a layer and animates its build-up.

## Shading

With the variant parameter `v` (0 when the layer has no variant):

```
layer_colour    = mix(colour,    variant.colour,    v)
layer_roughness = mix(roughness, variant.roughness, v)
k               = coverage * opacity
base_colour     = mix(base_colour, layer_colour,    k)     # after baseColorTexture x baseColorFactor, linear
roughness       = mix(roughness,   layer_roughness, k)     # after metallicRoughnessTexture x roughnessFactor
```

Layers are applied in array order. Normal, occlusion and metallic are unchanged in version 1.

## Readers

- A material whose extension breaks this format is refused as a whole: the reader keeps the core material and reports why.
- A parameter becomes a shader uniform in most readers, so a writer should not use shading-language keywords, types or
  built-in function names (`float`, `mix`, `texture`, ...). GazeForge writes lowercase `[a-z][a-z0-9_]*` names and refuses
  those words and the `gf_` prefix.
- The mask may be filtered (bilinear, mipmapped) before the coverage formula is applied per fragment.
- `COLOR_0`, when the core material uses it, multiplies the base colour before the layers, as in core glTF.
- Texture transforms do not apply to masks in version 1.

## Masks in the file

Masks are PNG images, four layers per RGBA image, each referenced through an ordinary glTF `texture`. Several layers can
share one texture through different channels. Mask textures use the same `TEXCOORD_0` as the base colour.
