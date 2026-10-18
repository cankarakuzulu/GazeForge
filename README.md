# GazeForge

A character system in Rust in the spirit of MakeHuman: one shared dense base mesh, linear targets on it, characters as
data, and a game-resolution mesh built once from the dense base. Meant to be driven by people and by AI agents alike.

**Status: early.** The Rust core and the pilot (M0) are in progress; the interfaces below will change.

## How it works

- **One dense base.** The MakeHuman head, subdivided to 1,151,889 vertices. Its vertex order is the identity of every
  vertex: targets, sculpts and characters are arrays over that order, never matched by position.
- **Linear targets.** MakeHuman's targets, re-expressed on the dense topology as sparse per-vertex offsets. A character
  is a set of target weights plus a sculpt residual.
- **The game base is a vertex subset of the dense base.** It is built by half-edge collapse only, so every kept vertex
  is an original dense vertex. Every target and every sculpt therefore maps to game resolution exactly, by index. The
  collapse cost is summed over several shapes (the base plus extreme targets), so one game mesh serves the whole shape
  range rather than one face.
- **Quality guards** during the collapse: minimum and maximum triangle angles, no normal flips, UV seams and the open
  boundary kept.

## Build

```
cargo build --release
```

The binary is `target/release/gazeforge`.

## Usage

Every command prints one JSON object on stdout and exits non-zero on failure.

Build a game base from a dense base:

```
gazeforge build-base --dense DIR --out DIR [--target 20000] [--shapes all|none|a,b] [--extra A.npy,B.npy]
                     [--min-angle 4] [--max-angle 170] [--max-turn 45] [--boundary-weight 100]
                     [--improve-below 20] [--flip-flat 15]
```

Export a shape (the base, a target, or any positions over the dense order) at game resolution:

```
gazeforge export-obj --dense DIR --base DIR --positions base|shape:NAME|FILE.npy --out FILE.obj
                     [--placement optimal|fit|f2|subset] [--max-move 3]
```

Run `gazeforge` without arguments for the full option list.

### Data layout

A dense base directory holds `manifest.json`, `positions.npy`, `face_sizes.npy`, `corner_vertices.npy`,
`corner_uvs.npy`, and one `shape_<name>.npy` per shape listed in the manifest. A game base directory holds `kept.npy`
(the dense indices it keeps), `owner.npy`, `triangles.npy`, `triangle_uvs.npy` and `report.json`.

The dense base and the targets are not in this repository. `tools/blender/export_dense.py` and
`tools/blender/export_targets.py` produce them in Blender from MakeHuman's assets; a documented, self-contained data
recipe is on the roadmap.

### Macros

`tools/macros/macro_weights.py` turns MakeHuman macro slider values (gender, age, muscle, weight, proportions, height,
...) into weights over MakeHuman's macro targets:

```
python macro_weights.py MACRO_JSON TARGETS_ROOT SLIDERS_JSON [--mode mpfb|gf]
```

`mpfb` reproduces the weights MakeHuman's Blender add-on produces; `gf` is a variant with no jumps along any slider.

## Roadmap

Each milestone ends on a measured gate, not a declaration.

| | What | Gate |
|---|---|---|
| M0 | Pilot: the Rust core reads the dense base and targets and builds the shared ~20k-vertex game base; a sculpt maps onto it by index | per-region error against the dense sculpt within budget; no sliver triangles outside the eye and nostril areas; no UV flips; a painted look-development render passes review |
| M1 | Core: the character spec (JSON: target weights + residual), apply and transfer, shape-quality gates (folds, inner surfaces, move roughness), glTF export with blend shapes | a spec round-trips to glTF and into a game engine; every gate fires on a planted defect |
| M2 | Interfaces: CLI, local HTTP API and an MCP server, all over one core | an AI agent completes "make a character with a wide nose" through each of the three |
| M3 | Browser designer: sliders and a live 3D view, the core compiled to WASM, heavy jobs on the server | slider response under 16 ms on the 20k base |
| M4 | Image to character: facial landmarks and a multi-view reconstruction, fitted to the linear targets, then a residual and a photo projection | a character rebuilt from photos in under an hour of compute |
| M5 | Game-ready output: LODs, a rig, expression blend shapes (ARKit-52) | a GazeForge head running in a game |

## Development

The repository follows gitflow: `main` holds releases, each tagged `vX.Y.Z`; work happens on `develop`, through
`feature/*` and `bugfix/*` branches; releases go through `release/*` and urgent fixes to a release through `hotfix/*`.

## Licensing and provenance

GazeForge is released under the [MIT License](LICENSE).

MakeHuman's assets (the base mesh and the targets) are CC0. MakeHuman's Blender add-on, MPFB, is GPLv3; no MPFB code is
included or adapted here. The Blender scripts only call it at run time, and the macro system is a clean-room
implementation written from `macro.json`, the target file names and observed output.

## Authors

Can Karakuzulu, with Sol (an AI assistant) as co-author.
