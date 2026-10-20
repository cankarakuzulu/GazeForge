# GazeForge

A character system in Rust in the spirit of MakeHuman: one shared dense base mesh, linear targets on it, characters as
data, and a game-resolution mesh built once from the dense base. Meant to be driven by people and by AI agents alike.

**Status: 0.2.0.** Every milestone below has working code, and 0.2.0 adds facial animation as an experimental feature; the
gates that are still open are listed, with their last measurements, in [CHANGELOG.md](CHANGELOG.md). Interfaces may
change before 1.0.

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

The workspace is layered, and every edge points down: `gf-geom` (pure maths: no IO, no threads, no clocks; builds for
`wasm32-unknown-unknown`; parallel loops only behind its native-only `parallel` feature) <- `gf-core` (domain: placements,
catalog, character spec, evaluation, residual codec, macros, measurements; no IO) <- `gf-io` (formats and the store) <-
`gf-api` (the command registry, error model, jobs, and the argv commands' implementations) <- `gf-cli` (the argv parser and
the CLI, HTTP and MCP front doors).
`gf-wasm` (the designer's preview kernel) depends on `gf-geom` only. `gf-studio` (the native studio,
binary `gazeforge-studio`) is another front door: it links `gf-api` in-process and nothing depends on it.
`python tools/dep_gate/dep_gate.py [edges|clippy|wasm|front-doors|all] --cargo PATH` checks it: the dependency edges
against an allow-list, clippy bans on file, thread, process, network and clock calls in `gf-geom` and `gf-core` with
`-D warnings`, the wasm32 build, and a build of `gf-core`, `gf-io` and `gf-api` in a copy of the workspace without its
front doors.

## Usage

Every command prints one JSON object on stdout and exits non-zero on failure.

Build a game base from a dense base, optionally with a per-vertex importance weight and nested LODs from the same
collapse run:

```
gazeforge build-base --dense DIR --out DIR [--target 20000] [--shapes all|none|a,b] [--extra A.npy,B.npy]
                     [--weights W.npy] [--lods 8000,3000] [--min-angle 4] [--max-angle 170] ...
```

Export a shape (the base, a target, or any positions over the dense order) at game resolution:

```
gazeforge export-obj --dense DIR --base DIR --positions base|shape:NAME|FILE.npy --out FILE.obj
                     [--placement f5|f6|subset|optimal|fit|f2|f3|f4|NAME/VERSION] [--eyes EYES.json] [--tip-vertex N] ...
```

Each placement is versioned (`subset/1`, `optimal/1`, `fit/1`, `f2/1`, `f3/1`, `f4/1`, `f5/1`, `f6/1`) and never changes once
released; a bare name means its latest version. `f5/1` is the default wherever no placement is named: a new character
spec (and so `export-glb`, which places with the spec's placement), `export-obj` (whose default was `optimal` before),
the designer, the image-to-character pipeline and `gates path`. A named placement runs exactly as released. With
`--eyes`, `f3`, `f5` and `f6` keep the placed eyelids outside the eyeballs. The output names the version it ran
(`placement_version`).

`f5` is the continuous placement: a small change to the dense shape makes a proportionally small change to the game mesh.
Each dense vertex is tied to a fixed game triangle and barycentric weights found in texture space (they do not depend on
the shape), and the game mesh is one linear least-squares solve: distance to the dense surface along normals taken from
the game-resolution subset, a weak pull toward the fixed correspondence, edge vectors kept close to the subset's, and a
tangential membrane that keeps triangles open. Seam vertices move along those normals only; the eyelid lift uses smooth
fades instead of cut-offs. There are no accept/reject steps, nearest-surface searches or data-dependent iteration counts
in the positions (the linear solver stops at a relative residual of 1e-13).

`f6` is `f5` with weaker edge and membrane terms (0.05 each) followed by a sliver nudge: wherever a triangle's height over
one of its edges is below sin(6 deg) times that edge, its vertices are pushed apart along that height, 32 damped rounds.
Each push grows continuously from zero at the threshold, so the output stays continuous in the input; boundary vertices
never move, seam vertices move at a quarter of the rate and keep their texture coordinates, and the push fades out near
the eyeballs when `--eyes` is given. The triangulation is unchanged. `f6` is opt-in (`--placement f6`); the default stays
`f5/1`.

Push a character's eyelid skin out of its eyeballs at dense resolution:

```
gazeforge collide-lids --dense DIR --positions base|shape:NAME|FILE.npy --eyes EYES.json --out FILE.npy [--clearance 0]
```

An option a command does not read is refused, and numbers and input positions must be finite. Run `gazeforge` without
arguments for the full option list of the argv commands; `gazeforge commands` lists the registry commands (below).

#### Store

A content-addressed store (sha256) holds dense packs, target catalogs and game bases. Each is a manifest whose id is
the sha256 of its canonical JSON bytes; arrays and target files are blobs under `blobs/`, verified on every read.
Identical files are stored once.

```
gazeforge store import-pack --store DIR --dense DIR [--macro-base SLIDERS_JSON|@FILE] [--pin-top Z_METRES]
gazeforge store import-catalog --store DIR --targets DIR --pack sha256:...
gazeforge store import-game --store DIR --base DIR --pack sha256:...
gazeforge store import-eyerig --store DIR --catalog sha256:... --makehuman MAKEHUMAN_DATA_DIR [--macro MACRO_JSON]
```

`import-catalog` reads `catalog.json` and its sparse targets (`.npz` with `indices` u32 and `deltas` f32 x 3, on the
dense order) as baked. `import-game` also imports `lod_<N>` subdirectories. `import-eyerig` builds a catalog's eye rig
(`gazeforge.eyerig/2`) from a MakeHuman data directory (CC0 assets): the helper eyeballs of `3dobjs/base.obj`
(`helper-l-eye`, `helper-r-eye`, 72 vertices each) and, for every catalog target, how its MakeHuman file
(`targets/<id>.target.gz`) moves them; a target whose file is missing is listed under `missing`. The helpers are stored
where they sit on the dense base: MakeHuman's base is put in the pack's `macro_base` state (`--macro`, the macro
definition, is required for a pack with one) and its skin is registered to the dense base by translation; the rig
records the shift and the registration error (`registration`), and the import refuses a registration whose median skin
distance exceeds 2 mm. An eye rig of an older format is refused with a request to import it again. `--macro-base` records the pack's own macro
slider values and `--pin-top` the height up to which head-only macros are pinned to the neck seam; both are needed for
characters with macros.

#### Characters

A character spec (`gazeforge.character/1`) is canonical JSON: the pack, catalog, optional game base and placement
version; target weights (each in [0, 1], rounded to 1e-6); optional macros; an optional residual. Its revision id is
the sha256 of those bytes, which is also the sha256 of the file `character create` writes.

```
gazeforge character create --store DIR --pack sha256:... --catalog sha256:... --out FILE.gfchar.json
                           [--game sha256:...] [--placement NAME/VERSION] [--name NAME] [--weights JSON|@FILE]
                           [--macro MACRO_JSON --sliders JSON|@FILE --macro-mode gf|mpfb --head-scale 1]
                           [--residual-from FACE.npy [--quantum-mm 0.005]]
gazeforge character validate --store DIR --spec FILE [--macro MACRO_JSON]
gazeforge character evaluate --store DIR --spec FILE --out DENSE.npy [--macro MACRO_JSON] [--precision f32|f64]
```

`evaluate` computes `base + sum(w * T) + head-only macros + residual` on the dense order; its output is a positions
file that `export-obj --positions` reads. `--residual-from` stores `FACE - (base + sum(w * T) + macros)` as an i16,
planar, delta-coded, zlib-compressed blob (`i16-planar-delta-zlib/1`); the default quantum of 0.005 mm keeps every
vertex within 0.0044 mm of the face.

**Eyeballs.** An optional `eyeballs` field makes the eyes of an eyes file follow the character:

```json
"eyeballs": {"follow": "makehuman-helper/1", "rig": "sha256:...",
             "left": {"offset_mm": [0, 0, 0], "rotation_deg": [0, 0, 0], "scale": 1},
             "right": {"offset_mm": [0, 0, 0], "rotation_deg": [0, 0, 0], "scale": 1}}
```

With `follow: makehuman-helper/1` each eye moves the way MakeHuman moves its helper eyeball: the rig's helper, where it
sits on the dense base, takes the weighted sum of every target's helper motion, macros through the same head-only
strip and pin as the skin, and the similarity (rotation, uniform scale, translation) fitted from the helper's rest to
its moved shape is applied to the eye as if the eye were rigidly attached to the helper: its mesh, its centre and
radius, and the surface table the eyelid clearance reads.
So the eye translation targets move the eye, while the targets that reshape the lids leave it where it is, as in
MakeHuman. `follow: none` keeps the eyes where the file puts them. The per-eye adjustments are applied before the
follow, about the eye's centre and in its own frame (offset along out from the midline, up and forward in mm; pitch up,
yaw outward, and roll turning the eye's up toward its out side, in degrees; a uniform scale), so they ride on the socket
and mirror between the sides; they are limited to 15 mm, 45 degrees and [0.5, 1.5], checked and applied after the
spec's rounding. A spec without the field places and exports exactly as before. The residual does not move the eyes.
`character validate` and `project.load` check that a following character's rig is in the store and belongs to its
catalog. Limits: the eyelid clearance of the placements lifts skin only within its clearance band (0.3 to 0.6 mm by
default), so where a target puts the skin deeper into an eye (static or moved) the skin stays inside it unless the
export runs with `--lids auto`; and MakeHuman moves its helper eyeball differently from the lids for some targets
(`head-scale-vert`, `head-angle`, the eye translation targets, `head-scale-horiz-decr`, old age), so a following eye can
sit up to about 5 mm deep in its lids there (measured with the eyes file used for testing, which was fitted to another
face; the per-eye adjustments correct a character).
`eyes.generate` writes an eyes file for a catalog's base head: a procedural eyeball (a sclera sphere, a cornea cap
meeting it at the limbus, a recessed iris and a pupil, as separate parts with UVs, from human proportions: a 12 mm
radius by default, the cornea's radius of curvature 0.65 of it, the limbus 0.96 of it across) centred on each of
MakeHuman's helper eyeballs where the eye rig puts them on the dense base; `helper_radius: true` uses MakeHuman's
helper radius instead, and `shape` sets the proportions. The eyelid clearance reads only the outer parts (sclera and
cornea) of an eyes file that names its parts, and only triangles facing forward, which gives the same surface table.
**Eye models.** `eyes.inventory` lists premade eye models (25 today, in families brown, hazel, amber, green, grey, blue
and special: heterochromia, a pale eye, dilated and constricted pupils), each with its proportions and its iris, sclera
and cornea looks. A character takes one through `eyeballs.model` (`{"id": "...", "radius_mm": 12}`, the radius in
[5, 20] mm; the model needs `eyeballs.rig`): the settle, `rig.face` and `export-glb` then build that model's eyes at the
eye rig's places instead of reading an eyes file (a model wins over `--eyes`), so the character carries its eyes.
`eyes.generate` takes `model` for the proportions too. `eyes.textures` writes each model's procedural textures as PNG:
the iris (1024 x 256: inner and outer colour fields meeting at a wavy collarette, radial fibres, crypts, a pupillary
ruff, a limbal ring, and the sector of a sectoral heterochromia) and the sclera (512 x 256: tint and veins, strongest at
the limbus), laid out on the UVs of the eyeball's iris and sclera parts. They are seeded per model, so they are the same
on every run. `export-glb` of a character with an eye model writes each eye as one mesh with four primitives and four
materials: the sclera and the iris textured (the PNGs embedded in the glb), a near-black pupil, and the cornea with
`KHR_materials_transmission` and `KHR_materials_ior` (the model's index of refraction, 1.376) and alpha blending at a low
base alpha, so a viewer without transmission still shows the iris through it. Without a rig the eyes are nodes
`Eye_L` / `Eye_R`; with `--rig head` they are children of the eye joints. Eyes from an eyes file export as before.
`eyes.look` returns one model's shading part by part (base colour, roughness, transmission and index of refraction,
blending, and the iris and sclera textures as RGBA pixels), the same values the glb writes. `designer.scene` and
`designer.eyes` (only the eyes, for a client that changes the model) take the spec's `eyeballs`: with a model they return
its eyes at their rest place with UVs and parts instead of an eyes file's.
`designer.sliders` returns the catalog's eye rig as `eyerig` and gives a new character `follow: makehuman-helper/1`;
`designer.settle` (and `export-glb` with `--eyes`) then report each eye's placement under `eyes`.

#### Macros

The macro definition (`macro.json`) is never shipped with GazeForge: every macro command reads it from the user's own
MakeHuman / MPFB install (`--macro PATH`), and a spec records its sha256 so a different file is refused.

```
gazeforge macro weights --macro MACRO_JSON --sliders JSON|@FILE [--targets TARGETS_ROOT] [--mode gf|mpfb] [--floor 0.01|none]
gazeforge macro labels --macro MACRO_JSON --slider NAME --values VALUES.npy --out TABLE.npy [--mode gf|mpfb]
```

`--sliders` is one object or a list of objects with all eleven sliders (`gender age muscle weight proportions height
cupsize firmness african asian caucasian`, each in [0, 1]). `gf` (the default) is continuous along every slider; `mpfb`
reproduces MPFB's values, including its 4-decimal rounding and, unless `--floor none`, its 0.01 key floor.

On a character, macros are head-only: the change against the pack's own sliders is summed over the catalog's macro
targets, its best linear similarity (translation, uniform scale, small rotation) over the whole head is removed, and the
rest is pinned to the neck seam by a smoothstep from the seam's top to `--pin-top`. Head size is its own control,
`--head-scale` in [0.9, 1.1], a scale about the seam centroid under the same pin.

#### glTF export

```
gazeforge export-glb --store DIR --spec FILE --out FILE.glb [--macro MACRO_JSON] [--morphs ID,ID|@FILE.json]
                     [--sparse auto|always|never] [--lod-screen 0.25,0.1] [the options of the spec's placement]
```

Writes one binary glTF 2.0 file of the character at game resolution: one node and mesh per LOD (`Head_LOD0` for the
full game base, `Head_LOD1`... for each nested coarser LOD of the spec's game base), positions in metres with +Y up and
the face looking along +Z, normals, one UV layer, 16-bit indices when a LOD has at most 65,535 vertices. Each catalog target named in `--morphs` becomes a morph
target on every LOD, `F(c + e) - F(c)`: the placed game positions of the character with the target at weight 1, minus
those without it. Morph targets are written as sparse accessors when that is smaller (`auto`), always, or never.
`extras.gazeforge` names the spec revision, pack, game base, placement version and tool. A sidecar `FILE.gazeforge.json`
lists the morph names and targets (with how many game vertices each moves), each LOD's node, vertex count and screen
size, the spec hash and the sha256 of the `.glb`.

Game-ready options (each is opt-in; without them the output is unchanged):

```
                     [--morph-dir DIR] [--morph-floor MM] [--facing keep|repair|repair-states] [--lod-screen measured
                     [--lod-pixels 1] [--lod-height 1080]] [--rig none|head|face --rig-jaw TARGET --eyes EYES.json [--rig-rounds 2]]
                     [--lids off|auto --eyes EYES.json [--lids-step-mm 0.05] [--lids-max-mm 1] [--lids-front-deg 75]
                     [--lids-tolerance-mm 0] [--lids-smooth 4]] [--lod0-out STEM]
```

- `--morph-dir DIR` resolves a morph id `ID` as `DIR/ID.npz` (sparse `indices` + `deltas` on the dense topology) before
  the catalog, so an expression set such as the ARKit-52 face units can be exported without being in the catalog. The
  morph name is the id's last path part.
- `--morph-floor MM` zeroes morph position deltas shorter than `MM` (and normal deltas under 1e-3), which lets sparse
  accessors pay where the placement moves far vertices by a negligible amount.
- `--facing repair` flips edges of triangles that face against their own vertex normals (folds a coarse LOD triangle
  picks up from the character's shape). A flip is taken only off UV seams, only when both new triangles face with their
  vertex normals in the neutral state, never adds a back-facing triangle in any morph state, and never makes a sliver
  thinner than the pair it replaces (or 5 degrees). Triangles the flips cannot fix are untangled: their vertices step
  toward their one-ring centroids in every LOD (at most 2 mm), and every morph state moves by the same amount, so morph
  deltas are unchanged. The LODs stay nested. The sidecar records the counts before and after per LOD.
- `--facing repair-states` does the same, then untangles each morph state at weight 1 that still has triangles facing
  against its own vertex normals, on its own (at most 4 mm). The neutral is not moved further, so at the moved vertices
  that morph's delta is no longer exactly `F(c + e) - F(c)`; the sidecar lists each state, its counts and its largest move.
- `--lids auto` keeps the eyelids outside the eyeballs of `--eyes`. After the placement, the finest LOD and every coarser
  LOD are checked: a vertex fails when it lies within 0.5 to 1.3 eye radii of an eye centre and within `--lids-front-deg`
  of its gaze, its normal points away from the centre, and it is closer to the centre than the larger of the eye radius
  and the eye mesh's outermost surface along that direction (by more than `--lids-tolerance-mm`). A state that passes is
  left untouched. A state that fails is collided at dense level (as `collide-lids`, with the failing vertices pushed even
  where the dense normal faces the eye) with clearance 0, then one `--lids-step-mm` more each time up to `--lids-max-mm`,
  re-placed each time, and the first clearance that passes is kept. Morph states are evaluated from the neutral's dense
  shape as exported, so the delta is `F(collided(c + e)) - F(c)`. After a facing repair, a vertex still inside an eye is
  moved along the eye's radius to its surface, alternating with the untangle (whose steps are kept outside the eyes) for
  up to 4 rounds. The sidecar's `lids` section records every search and the final check per LOD and state.
- `--lod0-out STEM` also writes the finest LOD as exported (neutral positions, its own triangulation and UVs) to
  `STEM.obj` and the neutral dense shape it was placed from to `STEM.dense.npy`, for `gates game --triangles obj`.
- `--lod-screen measured` chooses each LOD's screen size from the largest distance between the finest LOD's vertices
  and that LOD's surface: switch when that error projects to `--lod-pixels` on a screen `--lod-height` pixels tall,
  `s = 2 * pixels * radius / (error * height)`, with `s` the bounding sphere's diameter as a fraction of the screen
  height. The sidecar carries the errors, the radius and the formula, so an engine converts `s` to a distance for its
  own field of view: `d = radius / (s * tan(vfov / 2))`.
- `--rig head` adds a glTF skin with joints `neck` (root), `head`, `jaw`, `eye_L`, `eye_R`, translation-only, with
  inverse bind matrices that invert each joint chain exactly. Skin weights are computed per game vertex from the dense
  base: the jaw weight is the jaw target's displacement (`--rig-jaw`, e.g. a jaw-open unit) projected on the rotation
  about the hinge fitted to that same target; the neck weight is 1 on the neck seam (so a body's neck joint drives the
  seam exactly) and falls to 0 at half the chin-to-seam distance; the head takes the rest. Joint positions follow the
  character (the seam centroid, the fitted hinge moved with the face, the eye centres from `--eyes`), and two eyeballs
  `Eye_L` / `Eye_R` are children of the eye joints: spheres of the eyes file's radii, or, for a spec with `eyeballs`, the
  eyes file's own meshes as placed for the character. Without a rig, a spec with `eyeballs` exported with `--eyes` gets
  the placed eye meshes as two more nodes `Eye_L` / `Eye_R`; a spec without the field exports as before.

##### Face rig

`--rig face` (with `--morphs`, `--morph-dir`, `--rig-jaw` and `--eyes`, optionally `--rig-rounds N`, default 2) adds 24
facial joints to the head rig: `brow_inner/mid/outer_L/R`, `lid_upper/lower_L/R`, `cheek_L/R`, `nose`, `nostril_L/R`,
`lip_corner_L/R`, `lip_upper_L/C/R` (children of `head`) and `lip_lower_L/C/R`, `chin` (children of `jaw`). The head cage
has no tongue geometry, so there is no tongue joint.

- **Placement by index.** Each facial joint has a seed vertex on the dense base and sits at the mean of the character's
  dense positions over the base vertices within 2 mm of it; the lid joints sit at the eye centres and turn about them.
  Joint positions are rounded to 1/2^20 m so that the f32 joint chain and the inverse bind matrices cancel exactly.
- **Weights on the base.** Computed once from the base and the shapes, at the game vertices' dense indices, and carried
  to every LOD by index: distance falloffs around each joint (upper-lip joints share the head weight, lower-lip and chin
  joints the jaw weight, lids split at the lid line), then rounds of fit-the-poses / re-solve each vertex's weights
  (non-negative, summing to 1 - neck) / smooth over the mesh; the 4 largest are kept and quantised so the f32 weights sum
  to exactly 1. The same for every character.
- **Poses per shape.** For every morph, each posed joint (jaw and the facial joints) gets a bounded rotation about its
  rest position and a translation, solved by Gauss-Newton so that linear blend skinning reproduces F(c + e) - F(c) on the
  game LOD0 (vertices near the eyes count more). The sidecar's `rig.face.fit` lists, per shape, the error (mean / p95 /
  max mm) and the fraction of its squared displacement the joints explain; shapes below 0.8 or above 1 mm p95 are listed
  as poorly expressed.
- **Animations.** One glTF animation per morph, named as the morph, with LINEAR keys at t = 0 (rest) and t = 1 (the
  pose), so seeking to t gives roughly weight t. The blend shapes stay in the file: an engine uses either.

`gazeforge gates rig --glb X.glb [--eyes EYES.json]` checks a rigged glb: the rest pose lands exactly on the exported
positions, every vertex's f32 weights sum to exactly 1, and per animation and LOD the joint pose has no more folded
triangles (against normals recomputed on the posed mesh) than the blend shape of the same name, and with `--eyes` no
more vertices inside an eye (the `gates lids` rule).

The registry command `rig.face` returns the same rig for a designer character (settled positions, joints, weights,
fitted poses, errors, and the shapes' blend deltas), and the designer panel `web/panels/rig.js` drives it: expression
sliders and presets as blend shapes or through the joints, per-joint pose controls, a joint overlay, undo for every
change. Open the designer with `&face_morphs=<dir of the shapes' .npz>` (and `&face_prefix=` if their ids differ).

`--rig face2` is an opt-in second version of the face rig; `--rig face` is unchanged. It replaces the four lid joints
with twelve (`lid_upper/lower_in/mid/out_L/R`, three along each lid, all turning about the eye centre), starts their
weights from the eye geometry (the canthus landmarks and the lid seed vertices) instead of distance alone, gives the
mouth corners more translation reach, keeps up to 8 influences per vertex (`JOINTS_1` / `WEIGHTS_1` beside `JOINTS_0` /
`WEIGHTS_0`), and adds a barrier term to the pose fit that keeps vertices outside the eye surface. With
`--rig-correctives on` every shape also gets a corrective morph target (`<shape>.corrective`) that each animation drives
from 0 to 1 through a glTF `weights` channel, so the joint pose plus its corrective reproduces the blend shape exactly,
at the cost of one more morph target per shape on every LOD. `gates rig` reads both influence sets and applies
animated morph weights before skinning.

Morphs carry no UVs in glTF, so every LOD keeps the neutral character's UVs; the sidecar records, per morph, how far the
placement's own re-derived UVs at weight 1 sit from the neutral ones (`uv_slip_lod0`).

#### Bodies

A head joins a body at its neck seam. `body.import` builds a body pack (positions, triangles, UVs, the neck ring with a
seam parameter per ring vertex, and two coarser LODs whose collar keeps every ring vertex):

```
gazeforge call body.import --input '{"mode":"makehuman","obj":"base.obj","dense":"data/dense","out":"bodies/makehuman",
  "targets":"<makehuman targets>","macro":"<macro.json>","sliders":{...}}'
gazeforge call body.import --input '{"mode":"mesh","obj":"body.obj","dense":"data/dense","out":"bodies/other",
  "up":"y","unit_m":1.0,"cut_z_m":1.31}'
```

- **makehuman**: the MakeHuman base mesh (its `body` group). The dense head is the subdivided MakeHuman head, so the
  correspondence is found by topology: the dense base's subdivision patches give the head cage, and the cage embeds in
  the body's quads in exactly one orientation-preserving way. The neck ring is the cage boundary, vertex by vertex; no
  position matching. Optional macro sliders shape the body with the MakeHuman macro targets.
- **mesh**: any body. Its topmost open boundary loop is the neck (or `cut_z_m` removes every face above a plane and the
  cut becomes the loop); `align` is `translate` (default), `similarity` or `none`; ring parameters come from arc length
  with the best start offset, then each ring vertex's closest seam point nearby.

`character.stitch` (LOD0, unplaced head, for previews and the designer's Body panel) and `export-glb --body PACK` (every
LOD, placed head) join them: the body's ring vertices are replaced by copies of the head's seam vertices, and every body
triangle on the ring becomes a fan over the seam points between its two ring corners, so the seam has no cracks or
T-junctions by construction. The body is then bent onto the head over a band. Controls: `blend_mm` (band width, default
60), `neck_offset_mm`, `head_scale` (about the seam centre), `body_scale` (about the ring centre), `smooth` (Laplacian
passes over the band) and `tangent` (0..1, how far the first body row turns to continue the head surface; default 0.5);
on export they are `--body-blend-mm`, `--body-neck-offset-mm`, `--head-scale`, `--body-scale`, `--body-smooth`,
`--body-tangent`. `character.stitch` takes a spec file, a spec object or the designer's character; with `return_mesh` it
also returns the head positions it stitched to (`head_scale` applied, finest-level order).

In the glb the body is its own mesh per LOD (`Body_LODn` beside `Head_LODn`): the head carries the morphs and its own UV
layout, the body has none of the morphs and the body's UVs, so they need separate materials anyway. The seam vertices
of both meshes share position, normal (the two sides' average) and skin weights (1.0 on the neck joint), so they stay
closed in every pose. Gates, per LOD, fail the export: seam gap (< 1e-5 m for makehuman packs, < 1e-4 m for mesh packs),
open, non-manifold or mis-oriented seam edges, the angle between the head and body triangles on each seam edge (<= 40
deg), flipped or folded triangles in the band, and head or body vertices behind the other surface near the neck. The
`plant` input of `character.stitch` (`gap`, `crack`, `crease`, `flip`, `penetrate`) puts a defect in on purpose; each
makes the gates fail.

#### Gates

```
gazeforge gates shell --dense DIR --before base|shape:NAME|FILE.npy --after base|shape:NAME|FILE.npy [limits]
gazeforge gates game --dense DIR --base DIR --face base|shape:NAME|FILE.npy --obj GAME.obj [--gates triq,uv,census,seams]
gazeforge gates path --dense DIR --base DIR --start FILE.npy --delta FILE.npy [--steps 20] [--pop-limit 10]
                     [--placement f3 and its options] [--plant STEP,GAME_VERTEX,MM]
gazeforge gates facing --glb FILE.glb [--states neutral|all]
gazeforge gates lids --glb FILE.glb --eyes EYES.json [--states all|neutral] [--front-deg 75] [--tolerance-mm 0]
gazeforge gates rig  --glb FILE.glb [--eyes EYES.json] [--front-deg 75] [--tolerance-mm 0]
```

`facing` reads a `.glb` and counts, per mesh, triangles whose winding faces against the mean of their three exported
vertex normals (a back-facing triangle a renderer culls); with `--states all` it also counts every morph target at
weight 1 and fails on those too. `lids` applies the eyelid check of `export-glb --lids auto` to every head mesh of a
`.glb` (vertices sharing a position counted once), in the neutral and, with `--states all` (the default), every morph
target at weight 1.

`game --triangles obj` reads a game mesh whose triangulation differs from the game base's (for example the finest LOD
after `--facing repair`); the vertices must still be the base's. The `census` uses rule a-2: a visible triangle (outside
the eye and nostril box) more than 60 degrees from the mean of its corners' dense normals fails, unless a dense triangle
the decimation collapsed into it (all three of its owners among the triangle's corners) is itself more than 60 degrees
from the mean of its own vertex normals, i.e. the fold is already in the dense shape. `front_over60_raw` keeps the count
without that exception and `front_over60_dense_folded` the triangles it excused.

`shell` compares two dense shapes: quad folds that appear, inner surfaces that come to face the front, and move
roughness (vertices that move unlike their neighbours). `game` measures an exported game mesh against its dense face:
sliver and needle triangles outside the eye and nostril box (`triq`), flipped and zero-area UV triangles (`uv`), game
triangles facing away from the face's own dense normals (`census`), and seam vertices that disagree between UV charts
(`seams`). `path` places every step of a straight dense path and counts game vertices that jump while the dense surface
under them barely moves; `--plant` adds a forward jump to one vertex at one step to prove the gate fires. `shell` and
`game` assert the world frame first (-Y forward, +Z up); `path` measures distances only. A gate that fails prints its
report with `"ok": false` and exits non-zero.

#### Topology

```
gazeforge topology improve --store DIR --game sha256:... [--catalog sha256:...] [--shape-targets all|head|none]
                           [--samples 32] [--seed 1] [--dense DIR] [--below 30] [--min-gain 0.5] [--max-fold 20]
                           [--max-spread 40] [--rounds 32] [--levels all|finest] [--out DIR]
gazeforge topology inspect --store DIR --game sha256:...
gazeforge export-obj ... --quads on
```

`improve` (registry `topology.improve`, a job) re-cuts a game base's triangles by edge flips and stores the result as a
new game base; the old manifest is untouched. Only the triangulation changes: `kept` and `owner` stay byte-identical, so
every vertex is still an original dense vertex and every target, sculpt and residual maps exactly as before. Characters
name their game base by sha256, so they are unchanged until a spec names the new one.

A flip is scored over the shape range the base serves: the rest shape, every catalog target alone at weight 1
(identical target files once), `--samples` seeded combinations (per head slider of the designer's categories a random
side with probability 0.3 at a uniform weight, and with probability 0.5 one or two expression units), and with `--dense`
the dense pack's own manifest shapes. Each triangle's score is its smallest angle over all those shapes; a flip is made
only when it raises the smaller score of its two triangles by `--min-gain` degrees, and only on edges of triangles
scoring under `--below`. It is refused across a UV seam or the open boundary; where, in any scored shape, a new
triangle's normal leaves the old pair's normal by more than `--max-fold` degrees, more triangles would be turned over
(against their own rest orientation) than before, or a texture triangle would flip; and where the four corners' rest
surface normals (the dense base's, angle-weighted) differ by more than `--max-spread` degrees, a crease the game mesh
cannot follow. Rounds evaluate every candidate edge (in parallel when built with it) and apply the accepted flips in a
fixed order, so the result is the same on every run. With `--levels all` the levels of detail are improved the same way
and the new base names the new levels. The result reports, per level, the rest and whole-range statistics before and
after (smallest-angle percentiles, counts under 5, 10 and 20 degrees, largest angles, turned triangles) and a row per
scored shape; the manifest's `build.topology` keeps the summary and each triangle's worst angle before and after.
`--out` also writes the new base as a directory for `export-obj --base`.

`inspect` (registry `topology.inspect`) returns a base's rest-shape triangle quality, its triangles and the record of the
improve that made it. `export-obj --quads on` writes clean triangle pairs as quads: pairs are chosen on the base's rest
shape and texture (planar within 10 degrees, convex in 3D and in texture space, corners between 45 and 135 degrees, not
across a seam), so the characters of one base share them, and a pair that is not clean on the written mesh is written as
its two triangles. glTF has no quads: `export-glb` stays triangles, and `--quads` is off by default. `gates game` reads
triangle OBJs only, so gate the triangle export of the same mesh.

### One registry, three front doors

The commands an agent uses live in one registry (`gf-api`). Each has a name, a kind (query, mutation or job), an input
JSON schema, an output shape, its limits, and one handler. The CLI, the HTTP API and the MCP server all call that
handler and return the same envelope, byte for byte (`crates/gf-cli/tests/conformance.rs` checks this):

```
{"ok":true,"command":"character.measure","result":{...}}
{"ok":false,"command":"character.create","error":{"code":"invalid_input","message":"...","retryable":false,"details":{...},"hint":"..."}}
```

| code | CLI exit | HTTP |
|---|---|---|
| internal | 1 | 500 |
| invalid_input | 2 | 400 |
| not_found | 3 | 404 |
| gate_failed | 4 | 422 |
| worker_failed | 5 | 502 |
| cancelled | 6 | 499 |
| conflict | 7 | 409 |

Commands: `commands.list`, `store.list`, `catalog.list`, `character.create`, `character.validate`, `character.evaluate`,
`character.measure`, `jobs.submit`, `jobs.status`, `jobs.cancel`, `jobs.list`, and `debug.slow` (a deliberately slow job
that exists to prove cancel), among others; `commands.list` returns every command with its schema; the argv commands above
are unchanged. `store.list` gives each pack, catalog, game base and eye rig with the time it was imported (an eye rig of
an older format is listed as not usable rather than failing the listing); `jobs.list` gives the jobs folder's jobs, newest
first, with their state, times and progress. The browser designer's Store and Jobs panels and the studio's Tasks window
(Jobs, Store) are built on them: what the store holds by date, never a pasted sha256, and every job with its progress and
a cancel button.

**The argv commands in the registry.** Every argv command has a registry twin: `store.import_pack`, `store.import_catalog`,
`store.import_game`, `store.import_eyerig`, `base.build`, `export.obj`, `export.glb`, `lids.collide`, `gates.shell`,
`gates.game`, `gates.path`, `gates.facing`, `gates.lids` and `gates.rig` are jobs; `macro.weights` and `macro.labels` are
queries. The fields are the argv options with `_` for `-` (`min_angle` for `--min-angle`); numbers are JSON numbers, lists
and pairs are the same text the argv option takes (`"8000,3000"`), and a JSON option (`sliders`, `macro_base`) may be a
JSON object. The result is the argv command's own JSON, and a failing gate is the error `gate_failed` with that report in
`details`. One implementation serves both, so the argv command and its twin give the same output files and JSON on the
same input.

```
gazeforge call jobs.submit --input '{"command": "export.obj", "input": {"dense": "DENSE", "base": "BASE",
  "positions": "FACE.npy", "placement": "f3", "eyes": "EYES.json", "out": "FACE.obj"}}'
```

Every front door and the job records parse JSON with a correctly rounded float parser, so a number reaches the command
with the bits it was written with, and a job's result reads back unchanged (input nesting up to 128 levels, as before).
A twin's JSON option (`sliders`, `macro_base`) is handed to the command as text and parsed the way the argv option is,
so a 17-digit number there can still move by one ulp, exactly as it does on the command line. A field the chosen
placement does not read (or `lids_*` without `lids: auto`, `body_*` without `body`) passes the schema and is refused when
the job runs.

`character.measure` evaluates a spec and reports straight-line distances in millimetres between landmark vertices of
the dense base, with the base's value and the difference: nose width at the alae, nose length (nasion to tip), nose
height (nasion to subnasale), face width at the cheekbones, and eye spacing (inner and outer corners). The landmarks are
dense vertex indices chosen once on the base by a stated rule; the rules are in the command's input schema. A target
that slides the widest point along the surface is still measured at the base's vertex. `pairs` measures other vertex
pairs instead.

**CLI:**

```
gazeforge commands
gazeforge call character.measure --input '{"store": "STORE", "spec": "FACE.gfchar.json"}'
gazeforge call character.create --input @create.json
```

**HTTP** (127.0.0.1 only; requests with a non-loopback Host or Origin are refused):

```
gazeforge serve [--port 7878]          # prints {"ok":true,"command":"serve","listening":"127.0.0.1:7878",...}
GET  /v1/commands                      # the commands.list envelope
POST /v1/<command>                     # JSON body = the command's input; status from the table above
GET  /v1/jobs/<id>                     # jobs.status
```

**MCP** over stdio (`gazeforge mcp`: `initialize`, `tools/list`, `tools/call`, `ping`). Each command is a tool whose name
has its dots turned into underscores (`character_measure`); the tool result is the same envelope as text, with `isError`
set when `ok` is false. A client configuration:

```json
{
  "mcpServers": {
    "gazeforge": {"type": "stdio", "command": "/path/to/gazeforge", "args": ["mcp"]}
  }
}
```

**Jobs.** `jobs.submit {"command": NAME, "input": {...}}` runs a command in a worker process and returns at once;
`jobs.status {"id", "wait_s"}` reads its record (`queued`, `running`, `done`, `failed`, `cancelled`, with the result
when done); while a job runs, `progress` holds the stage, done, total and message its command last reported (export
stages, path steps, photo-pipeline stages; at most one write every 200 ms within a stage), and a failed or cancelled
job keeps it to show where it stopped; `jobs.cancel {"id"}` kills the worker and every process it started. On Windows each worker runs inside a Job
Object that is killed as a whole and closed on exit; the record's `worker_processes_left` is the job object's own count
after the kill. Elsewhere the worker gets its own process group, which is killed, and that count is null. Jobs live in
`GAZEFORGE_JOBS`, else `<temp>/gazeforge-jobs`, so a job submitted through one front door can be read or cancelled
through another that uses the same directory.

**Wizards.** A wizard is a guided task whose every step is one registry command, defined as data in `gf-api`:
`wizards.list` and `wizards.get` (each step with its command, kind, input schema, the fields a form shows, common or
advanced, and bindings: `$run.NAME`, `$STEP.input.FIELD`, `$STEP.result.PATH`, `=TEXT`). A run is a folder
`ROOT/runs/PROJECT/WIZARD-N` (`steps/STEP` holds each step's record, the rest the files its steps write, bound as `$dir`): `wizards.start` makes it, `wizards.step` checks a step's input
against its command's schema and runs it (or submits it as a job) and marks later steps stale, and `wizards.state` reads
the run back, resolves each step's proposed input and collects finished jobs, so a run can be left and resumed;
`wizards.runs` lists a project's runs, newest first, and both clients offer them under "recent runs". A step's input is
checked before anything of the earlier attempt is removed (a step without a command against its fields), re-running a
running job step cancels that job first, and a job whose record is gone fails only its own step. An agent
can drive a wizard with the same calls. The wizards: `setup` (a store from a dense base folder, its targets, a game
base and the eye rig), `build_base` (a game base built and imported), `create` (a character from target weights,
validated), `photos`, `export`, `body` (a body pack), and, behind a "developer tasks" toggle, `check` (shell gates),
`lids` (eyelids out of the eyeballs) and `macros` (macro weights and label tables). `photos` takes a front and a profile photo (and optionally the other profile)
through `photo.check`, `photo.views` (whose worker now also writes each photo's overlay: landmarks green, outline red,
profile key points yellow), `fit.photos` and `character.create`, and opens the saved spec in the designer. `export` takes
a character spec through `export.glb` (with an optional obj of the finest level) and `gates.facing`. Each writes the same
files its commands write from the CLI. `fit.photos` reports where its base landmark table came from (`base_landmarks`:
given, already in the run, or detected, which a re-run keeps reporting while the run still holds the detected table); with no table it now detects one on a render of the base, as `fit.register`
does, and warns that a table detected on the base without eyes is several millimetres off at the lids and the chin. Two
HTTP routes serve the browser's wizards, both loopback-only and confined to a run's folder: `POST /v1/upload?run=&name=`
(a file chosen in the browser, copied to the run's `uploads/`, up to 64 MB) and `GET /v1/wizard-file?run=&path=` (an
image a step wrote, such as an overlay). In the browser designer
the Tasks button (wand icon) lists the wizards and runs them: the current character is saved into the run, each step is
a form built from its command's schema (the named fields first, every other option under advanced), and a job step
shows its progress. The native studio has the same Tasks window (the wand in the toolbar), calling the registry
in-process; its job steps run through the `gazeforge` tool installed next to it, and the `--script` op `wizard` (`start`,
`set`, `run`, `next`, `back`, `wait`) drives it for checks.

### Designer

A browser page served by `gazeforge serve`: sliders for the 290 head targets (cheek, chin, ears, eyebrows, eyes,
forehead, head, mouth, neck, nose) with search, a category filter and reset (double-click resets one slider), the macro
sliders (head only: the similarity is stripped and the neck seam pinned) when a macro definition file is given, a 3D
viewer for topology inspection (below), undo / redo of every edit, extension panels, and saving the character as a spec
file.

```
gazeforge serve --port 7878 --web web       # --web: the folder holding index.html (default ./web, or GAZEFORGE_WEB)
http://127.0.0.1:7878/designer/?store=STORE[&spec=FACE.gfchar.json][&macro=MACRO_JSON][&eyes=EYES.json][&out=SAVE.gfchar.json][&fast=1]
```

While a slider is dragged the page shows a preview, labelled approximate: the last settled game mesh plus each changed
target's exact delta at the game vertices, computed in WebAssembly (`gf-wasm`, which also recomputes the vertex normals).
On release (150 ms debounce) the server settles the character (weights and macros -> dense -> the spec's placement ->
game mesh), and the settled mesh replaces the preview and becomes the base of the next one. Under the default placement
the settled mesh can differ from the preview, because the placement re-fits game vertices along the surface. Macro
sliders preview from samples: after each settle the page fetches, in the background, the settled game mesh at a few
values of the age and gender sliders (every part boundary of the slider plus a 0.25 grid, `designer.macro_delta` with
`sampled`), and a drag interpolates between them. Until a slider's samples exist its drag uses a one-sided slope, and the
status line says so.

Everything heavy stays on the server: the dense pack, the catalog and the placement never reach the browser. The page
downloads one preview pack per target set (game triangles plus each target's i16 delta at the game vertices and a scale,
a content-addressed store blob, cached by hash) and the settled positions. The commands, usable from every front door:
`designer.sliders`, `designer.pack`, `designer.settle`, `designer.macro_delta`, `designer.scene`, `designer.save`. The served process keeps
one pack, catalog and game base in memory between settles. `GET /v1/blob/<sha256>?store=STORE` serves a store blob.

With them it keeps, built on the first settle, everything the placement computes that does not depend on the character:
boundary and seam flags, rings, edges, the UV charts and each game vertex's UV search patch, the f5 footprint. Later
settles compute only what depends on the dense positions, and their results are byte-identical to a one-shot placement.
`designer.settle` places with the spec's placement, f5 by default. `&fast=1` asks for an opt-in draft when that
placement is f3/f4: 1 of its 4 fit rounds (input
`fast`, reply flagged `approximate`, labelled as a draft in the page). A draft is never saved or exported; after 3 s
without a change the page settles again with the full placement.

Building the page's kernel: `cargo build --release -p gf-wasm --target wasm32-unknown-unknown`, then copy
`target/wasm32-unknown-unknown/release/gf_wasm.wasm` to `web/gf_wasm.wasm` (`CARGO_PROFILE_RELEASE_DEBUG=0` keeps it
small). The page uses WebGL2 and no third-party scripts. On some Windows machines Chrome's default graphics backend gives
no WebGL context; `--use-angle=gl` does.

#### Viewer controls

The viewer draws the head by vertex pulling (positions, normals and indices live in textures), so every mode shares one
upload per frame. `designer.scene` returns the game base's coarser levels of detail as triangles over the finest level's
vertices, the base triangle UVs, and the eyes of an optional eyes file (centre, radius, side, gaze and mesh).

- **Camera:** orbit, pan and zoom with damping; double-click a surface point to orbit around it; perspective or
  orthographic with matched framing; named views; field of view (the framing at the pivot is kept).
- **Layouts:** single, side by side, quad (front, side and top orthographic plus perspective). Each viewport has its own
  camera; the active one is outlined and receives the camera controls. Render mode and level of detail are per viewport
  (linked by default).
- **Render modes:** clay, flat, Gouraud, Blinn-Phong, anisotropic (Kajiya-Kay along horizontal or vertical tangent rings),
  matcap, normals, hidden line, triangle quality (smallest angle), displacement from a reference in mm (the starting
  character, the neutral base or a snapshot; signed along the normal), mean curvature from the vertex normals, UV checker.
  Overlays on any mode: wireframe (constant pixel width, optionally faded where triangles are only a few pixels tall),
  hidden edges through the surface, vertex points, back-face
  highlight, UV seams and open boundaries. Levels of detail switch per viewport.
- **Lighting:** presets (studio three-point, flat, rim, top, single key), up to four lights with azimuth, elevation,
  intensity and colour temperature, a hemisphere ambient, exposure, lights fixed to the camera or to the world, a gizmo.
- **Clipping:** up to three planes (x, y, z or free), each optionally a slab of two parallel planes; the section cap is
  filled (stencil) and the back faces inside a cut are drawn in their own colour.
- **Hide:** show, hide or isolate the head, the eyes, extra meshes and overlays; hide regions (each vertex belongs to the
  category of the local target that moves it most relative to that target's largest move) or everything inside a box.
- **History:** sliders, macros, reset, visibility and panel actions undo and redo; camera moves are not recorded.
- **Screenshot:** a PNG of the active viewport or of the whole layout. View settings are remembered in the browser.

| key | action | key | action |
|---|---|---|---|
| Ctrl+Z, Ctrl+Y | undo, redo | F, Shift+F | frame the head, frame the selection |
| double-click | orbit pivot on the surface | drag, right / middle / Shift drag, wheel | orbit, pan, zoom |
| O or 5 | perspective / orthographic | 1, Shift+1 | front, back |
| 3, Shift+3 | left side, right side | 7, Shift+7, 9 | top, bottom, three-quarter |
| [ and ] | field of view | L, N | next layout, next viewport |
| M, Shift+M | next, previous render mode | `,` and `.` | coarser, finer level of detail |
| W, X, P | wireframe, hidden edges, points | B, E | back faces, seams and boundaries |
| C, Alt+wheel | clip plane 1, move it | G | light gizmo |
| H, Alt+H, I | hide the head, show everything, isolate the head | K, Shift+K | PNG of the view, of the layout |
| Ctrl+N, Ctrl+O | new project, open a project | Ctrl+S, Ctrl+Shift+S | save, save as |
| ? | shortcut help | Esc | close the help |

#### Panels

A panel is an ES module `web/panels/<id>.js` listed in `web/panels/index.json` (an array of ids; its order is the UI
order). The page imports every listed module at start; the module calls `window.GF.registerPanel({ id, title, order,
mount(rootEl, api) })`. The `api` given to `mount`:

```
call(command, input) -> Promise<result>     a registry command over HTTP (POST /v1/<command>)
character()                                 the current character as the page holds it (store, pack, catalog, game, weights, ...)
onCharacterChange(cb)                       cb(character, { kind: "settle" | "slider", positions?, measurements? })
setPose(id, positions | null)               temporary head positions (Float32Array, finest-level order); last non-null wins
setExtraMesh(id, { positions, indices, normals?, color? } | null)
poseExtraMesh(id, rotation3x3 | null, centre)   rigidly turns an extra mesh (e.g. "eyes") about centre; null restores it
setOverlay(id, { lines?, points?, color } | null)
pushUndo({ label, undo(), redo() })         a panel action joins the page's undo / redo history
requestRender()
```

Panels never reach the renderer directly. `web/panels/measurements.js` (the settled measurements, a bounding-box overlay
with undo, the level-of-detail sizes through `call`) is the example.

`mount` may return `{ getState(), setState(state) }` (or the definition may carry them; both optional): the page saves
`getState()` (plain JSON) with the project and calls `setState(state)` after mounting when a project is opened. A state
puts any file or folder path under `resources` (`{"body": "..."}`): the project keeps those as resources by identity and
hands `setState` the paths found on load; any other text that looks like a local path blocks a portable export. The Body
(`stitch.js`) and Face rig (`rig.js`) panels implement both. The Body panel runs `character.stitch` with its gates
enforced: a stitch that fails them is drawn in red with the failed gates listed, never as a usable body; a head scale
other than 1 shows the scaled head through `setPose`; while the character changes, the body is marked stale (or
re-stitched after the settle when "Re-stitch after every change" is on). The Face rig panel drops its pose while a slider
moves and refits after the settle.

The Topology panel (`topology.js`) shows the triangles whose smallest angle on the settled character is under a limit
(the toolbar's warning-triangle button toggles it), runs `topology.improve` as a job with its progress (an optional
"extra shapes" folder is passed as `dense`; the store does not keep a dense folder's shape keys, so a result scored
without them is a different base), shows the
numbers before and after, draws the improved base beside the head (settled on it), and reloads the page on the new base
(the `game` URL parameter, which `designer.sliders` takes as `game`). Its default base, when no spec names one, is the
store's largest base that is not a topology result.

`api.grid` (the face chart, below) gives a panel `ensure()`, `lastProbe()`, `onProbe(cb)`, `onPin(cb)`,
`describe(triangle, bary)`, `pointOf(triangle, bary)`, `normalOf(triangle)`, `goTo(address)`, `setMarkers(id, list | fn)`,
`character()` and `definition()`. The Pins panel (`web/panels/pins.js`) is built on it.

`api.eyes` gives `get()` and `set(eyeballs)` (the character's `eyeballs`, see Characters; `set` settles), `rig()` (the
catalog's eye rig), `placed()` (the last settle's eye report) and `hasEyes()`. The Eyes panel (`web/panels/eyes.js`) is
built on it: an eye model picker (the inventory by family, or the eyes file) with its radius, follow the sockets, link
left and right (default), and per eye the out, up and forward offsets, pitch, yaw, roll and scale relative to the
socket; each change joins undo and redo. The page draws the eye model's eyes (or the eyes file's meshes) and places them
from each settle's eye report, so an eye moves when the settle lands (no live preview between settles in the browser; the
native studio previews it). In the lit render modes (clay, flat, Blinn-Phong, anisotropic) a model's eyes are shaded
from `eyes.look`: the textured sclera and iris, a near-black pupil, and a glossy cornea drawn over them with blending
(Fresnel reflection and a sharp highlight); the other modes draw them like any mesh.

### Grid and addresses

A way to say *where* on the face something is, the same way for a person looking at the designer and for a tool reading a
gate report.

**The face chart.** Every vertex of the dense base gets a chart coordinate once, computed from the **base** positions
(never from a character) and carried by vertex identity: because the dense vertex order is the identity and game vertices
are dense vertices, a cell names the same vertices on every character, at every level of detail. The chart is a cylinder
about a vertical (z) axis through a fixed centre: the midpoint of the x and y extent of the base vertices at or above the
nose tip (dense vertex 185894). `u` is the angle about that axis, 0 through the nose tip and positive towards +x (the
viewer's right in the front view), in (-180, 180]; `v` is millimetres below the highest base vertex.

**Cell names.** Columns are 15 degrees of `u`, lettered A to X, with **M** centred on the nose tip (`u` in [-7.5, 7.5))
and A at the back of the head; rows are 10 mm of `v`, numbered from 1 at the top of the head down. At the nose tip's
distance from the axis a cell is about 26 x 10 mm. A subcell `.1` to `.9` splits a cell 3 x 3 in numeric-keypad
layout as seen from outside the head: 7 8 9 the top third, 4 5 6 the middle, 1 2 3 the bottom. `M14.5` is the middle of
the cell holding the nose tip. Where the surface turns away from the axis (behind the ears, under the chin and jaw,
inside the lips, nostrils and eye openings) a cell covers two layers of surface; the vertex index of an address tells
them apart. `grid.chart` reports these folded triangles per cell.

**The seam and the axis.** The seam (`u` = +-180) runs through the middle of column A, so no cell boundary lies on it;
a triangle that crosses it is interpolated after unwrapping its corners' `u`. Close to the axis itself (the crown,
and inside the mouth and throat) the angle stops meaning anything: a triangle whose corners' unwrapped `u` spread over
45 degrees or more (`axis_span_deg` in the chart definition) is treated as lying on the axis. A point on such a
triangle takes the cell of its largest-barycentric corner. The cells touched by such triangles, or by a triangle that
crosses the seam, never get an in-cell label (the address of a point there is still exact).

**Addresses.** A probe (Q toggles probe mode, Alt+click probes in any mode, also through the address card and other
overlays) shows an address card with one line to copy:

```
GF@ M14.5 | region nose | game v1060 = dense 446284 | LOD0 t13322 b(1.000,0.000) | -1.7,-151.1,1473.9 mm | train_aged fe2b89a9e097
GF@ L8.2 (v in L8.3) | region eyebrows | game v4833 = dense 302901 | LOD0 t7537 b(0.040,0.559) | -18.2,-148.9,1516.9 mm | reference_face f738077f5812
```

The fields, in order: the cell and subcell of the surface point; when the named vertex lies in another subcell,
`(v in ...)` gives that vertex's cell, so the line never disagrees with itself; the region of the vertex; the vertex,
which is the corner of the hit triangle nearest to the point in space, as game vertex and dense vertex; the level of
detail, triangle and barycentrics of corners 1 and 2 in that level; the point in millimetres; and the character. The
card's go-to box (J) takes a whole line, a cell (`M14`), a subcell (`M14.5`), `dense N` or `game vN`, frames the spot in
the active view and flashes a marker; a pasted line whose parts disagree still goes to its surface point, with a warning
naming the disagreement. **T** draws the face grid on the surface (cell lines, subcell lines when close, the front
midline in red) in every shading mode, layout and level of detail, and labels it like an atlas: column letters along the
top and bottom of each viewport and row numbers along its sides. A top letter sits over its column where the column
crosses the upper half of the head; a bottom letter sits under the point where its column leaves the head at the bottom
(it is only placed when the column reaches the bottom outline), so following a strip down to the bottom edge reads its
letter; a row number sits level with its row. Hovering shows the cell under the cursor, and once a cell is large enough
on screen (zoomed in) it carries its own label. Labels are tested against the surface depth, so none shows through the
head. The **eyes** option sets what the eyes do while the grid is on: the grid drawn over them (from their own
positions in the chart frame), hidden, or plain. **R** draws a millimetre grid: in an axis-aligned orthographic view
(front, side, top) lines every 10 mm labelled at the edges, with 5, 2 or 1 mm lines as the view zooms in; in perspective
a 10 mm back plane behind the head. No label is drawn over a viewport caption, the toolbar or the address card.

**Pins.** The Pins panel keeps numbered markers placed from the address card. A pin is stored by the finest level's
triangle and barycentrics (plus its dense vertex), so it follows sliders, characters and levels of detail; each has a
note; pins join undo and redo and are saved with the project. A pin placed on a coarser level of detail goes to the
finest-level surface point closest to the clicked point (an exact closest-point search over the finest triangles); the
card and the pin record give that distance (`lod0_error_mm`), which is how far the coarser level's surface lies from the
finest one there. *Export pins* writes `gazeforge.pins/1` JSON: the character, the chart definition, and per pin its
address, cell, region, game and dense vertex, triangle, barycentrics, position and note.

**Commands.** `grid.chart` (per game vertex: chart coordinate, packed cell, region, dense index; one label anchor per
cell; the chart definition and its sanity figures), `grid.locate` (dense vertices, game vertices, triangles,
triangle + barycentrics points, or a whole gate report -> cell, region, chart coordinate) and `grid.resolve` (a cell,
subcell, `dense N`, `game vN` or an address line -> the game vertices, the dense count, the base centroid and the game
vertex nearest it). `grid.resolve` checks an address line against itself before answering: the game index must be the
game base's index of the dense vertex, the cell (or the `(v in ...)` cell) must be the vertex's cell, and for a LOD0
line the cell must be the cell of the stated triangle point and the vertex one of its corners. A line that fails any of
these is refused with the disagreements named; `accept_inconsistent: true` resolves it by its dense index anyway and
lists them under `warnings` (`consistent` is false, `checked` lists what was checked). A dense vertex the game base does
not keep comes back with `kept: false`. In `grid.locate`, a point's vertex is the corner nearest to it on the base,
reported with its own `vertex_cell`. The gates speak the grid when asked: `gates game ... --cells on` adds a `cell` beside every
reported triangle and vertex and a `*_cells` list beside `worst_visible`, `lost_vertices` and `added_vertices`;
`gates lids ... --cells on --dense DIR --base DIR` adds the cells of the vertices inside an eye (read for the finest
level, whose triangles follow the game base's order). Without `--cells` both commands print exactly what they did
before.

### Native client

`gazeforge-studio` is a native designer (Rust, wgpu and egui; Windows and macOS) that runs the core in the same process:
loading a store and settling a character call the registry's typed cores directly (`designer::scene_mesh`,
`designer::settle_mesh`), so no mesh crosses HTTP, JSON or base64. The browser designer stays as the other client.

```
cargo build --release -p gf-studio
gazeforge-studio --store DIR [--spec FILE] [--macro FILE] [--eyes FILE] [--keymap FILE] [--input sliders] [--godot-config FILE]
gazeforge-studio --script STEPS.json --out DIR
gazeforge-studio --write-keymap FILE
```

The viewport ports the browser viewer: the same twelve render modes (clay by default) and overlays (wireframe, hidden
edges, vertex points, back faces, seams and boundaries, clip planes with capped sections, a clip box, light gizmo),
the same orbit, pan, zoom, pivot, focus, perspective and orthographic cameras, named views, the single, side-by-side and
quad layouts and the LOD switch. Each viewport slot takes its pixels from a frame source: the built-in renderer, or an
external source that hands back RGBA frames for the slot's camera and size.

The side panel carries the browser's Face rig, Body and Timeline panels and its projects, ported command for command:
the face rig (`rig.face`; blend shapes or joints, presets, the 51 shapes, a manual joint pose, the joint overlay), the body
stitch (`character.stitch`; the body as an extra mesh, the seam as overlay lines, a head scale shown as a pose), the
facial animation timeline (clips, capture from video or images, CSV import, playback with the head rotation turning the
eyes too) and projects (open with relinking, save, save as, characters, notes, autosave and recovery, portable export).
A project saved by either client opens in the other: the studio writes the face rig, body and timeline states in the
browser's formats and keeps the panel states and viewer state it does not use. `GAZEFORGE_FACE_MORPHS` and
`GAZEFORGE_FACE_PREFIX` play the role of the browser's `face_morphs` and `face_prefix`. The script ops `rig`, `stitch`,
`timeline` and `project` drive them headlessly; each port's pose, body and project equal the browser's on the same input.

`--eyes` takes the same eyes file as the browser designer: its eye meshes are drawn in the built-in renderer (the `eyes`
toggle hides them) and every settle gets it, so `f3`, `f5` and `f6` keep the eyelids outside the eyeballs. With the
character's `eyeballs` following (a new character starts that way when the catalog has an eye rig), each settle places
the eyes and the drawn meshes move with them. The **Eyes** tab turns following on and off and sets each eye's offset,
rotation and scale relative to its socket, linked between the sides by default; while a slider moves, the eye moves on
the last settled socket, and the settle that follows clears the lids against it. The tab also picks an eye model from
the inventory and its radius (needs the catalog's eye rig): the studio rebuilds the eyes at once and draws them in the
lit modes like the browser does (textured sclera and iris, near-black pupil, a blended glossy cornea); other modes draw
the plain geometry. Eye edits join undo and redo and are saved with the spec.

Shape input sits behind an input layer (`--input`, default `sliders`): an input turns what the user does into target
weights and macro values, as preview changes while a gesture runs and one commit when it ends. The slider panel is the
first input: a tab for the macro sliders (when a macro definition is given), one per target category and one with every
target; search across all targets; reset; right click for a slider's start value. Targets that are opposites of each
other (`decr`/`incr`, `down`/`up`, `in`/`out`, `backward`/`forward`, `concave`/`convex`, `compress`/`uncompress` on the
same name) share one slider from -1 (full first) through 0 to 1 (full second), which sets one of the two and clears the
other; a character that has both set shows the difference and names both in the slider's tooltip until the slider is
moved. Another input can replace it without touching the preview, the settle or the history.

While a slider moves, the preview is the last settled mesh plus the weight changes times each target's sparse deltas
on the game vertices (macro sliders interpolate settled samples taken in the background, or use the macro's local slope
until those exist, which the overlay reports), with normals recomputed and uploaded every frame. A
commit settles in-process on a worker thread with the spec's placement; the settled mesh replaces the preview. A settle
still queued when a newer one arrives is dropped before it starts; one already running stops at the placement's next
safe point (between solver iterations and fit rounds) when a newer commit arrives or a slider drag begins, and never
publishes a partial mesh. A settle stopped by a drag runs again once the drag ends, so the preview has the cores while a
slider moves and the settle has all of them otherwise; a settle that completes is identical to `designer.settle`. Every
shape change can be undone and redone, and the spec can be saved (`Ctrl+S`). A timings overlay (`Alt+H`) shows the
present rate, frame times, input-to-preview latency, settle latency, the macro slope and sample costs.
`GAZEFORGE_SETTLE_THREADS` (default: all cores) and `GAZEFORGE_PREVIEW_THREADS` (default 1) size the two.

The side panel's Topology section highlights the triangles whose smallest angle is under its limit (`Alt+Q`, or the
warning-triangle toolbar button), runs `topology.improve` as a gf-api job (the studio binary runs the job's supervisor
and worker itself) with a progress bar, shows the numbers before and after, switches the viewports between the loaded
base and the improved one with the character settled on each, and reloads the studio on the improved base (the Store
section's `game` field, an override of the spec's base). Scripts drive it with the `topology` step (`do`: `show`,
`improve`, `wait`, `after`, `before`, `summary`).

`--bench-window SECONDS` orbits and previews continuously and prints the frame rate; `--bench-idle SECONDS` waits for
the app to go idle and counts the frames it draws with nothing changing (none expected). When the operating system
reports the window hidden (a minimized window, or a desktop with no display attached), the windowing layer repaints at
most every 100 ms and the bench says so in `paced_by`. `GAZEFORGE_LOG=warn` (or `info`, `debug`) prints the GPU
driver's and the windowing layer's log to stderr; a window whose GPU setup fails exits with a JSON error naming the
backend.

The key map is a JSON file (`--write-keymap` writes the defaults; edit and pass it with `--keymap`, or place it at the
default location printed by `--help`): every action takes a list of bindings such as `"Alt+W"`, `"Ctrl+Shift+Z"` or
`"Ctrl+Alt+1"`, and an empty list unbinds it. No default is a bare letter or digit: view toggles use `Alt`, editing
uses `Ctrl` (`Ctrl+Z` undo, `Ctrl+Y` or `Ctrl+Shift+Z` redo, `Ctrl+S` save) and `F1` lists the keys.

`--script` runs a list of steps against an offscreen GPU target with no window and prints one JSON report (also
written to `DIR/report.json`): `load`, `layout`, `active`, `mode`, `lod`, `view`, `ortho`, `camera`, `orbit`, `pan`,
`zoom`, `fov`, `focus`, `action` (any key-map action, including `undo` and `redo`), `option` (viewer options by name,
such as `wire` or `clip.0.offset`), `weights` (target weights and macro values committed through the input layer, then
the settle awaited), `edit` (the same as a preview change, optionally committed without waiting), `drag` (a slider moved
over a number of rendered frames, then released: input-to-preview latency per frame, frame times during the drag and
during the settle, release-to-settled latency), `settle`, `wait`, `save`, `dump_settle` (the settled positions and the
exact settle input), `hud`, `source`, `screenshot` (PNG of the layout or the active viewport), `frames` (frame timings),
`dump_positions`, `keymap` and `summary`, plus the Godot render mode's steps below. A step that fails stops the run
with a non-zero exit code.

#### Godot render mode

Any viewport slot can be drawn by Godot's renderer instead of the built-in one (the toolbar's renderer choice, or the
`source` step with `"kind": "godot"`), so a character can be looked at under a game's own materials, environment and
lights without leaving the client. The frames are shown in the viewport like any other; the client's camera drives
Godot's. The mode is off until it is chosen.

How it works: choosing the mode starts Godot 4.7 as a child process on the render host project in `clients/godot-render`
(GDScript only). The host keeps its main window minimized with the render loop off and draws on request into one
offscreen viewport per slot. The two talk over one localhost TCP connection with a small binary protocol (protocol
version 2, little endian; every message has a 16-byte header: magic `GFRH`, version, type, payload length; a host that
speaks another version is refused with the reason):

| Message | Direction | Payload |
|---|---|---|
| `HELLO` / `HELLO_ACK` | app -> host -> app | protocol version; the host answers with its Godot version and GPU |
| `CONFIG` / `CONFIG_ACK` | app -> host -> app | JSON: material, environment, scene, background; the host answers what loaded and what did not |
| `MESH` / `MESH_ACK` | app -> host | once per topology: positions, normals, UVs (game vertices split at UV seams, one vertex set shared by every LOD) and each LOD's triangle indices |
| `POSITIONS` / `POSITIONS_ACK` | app -> host | every new head revision (each settle, and each preview frame while a slider moves, when the host is free): positions and normals |
| `RENDER` / `FRAME` | app -> host -> app | request number, slot, size, perspective field of view or orthographic height, near, far, camera transform, LOD; the host answers with the request number, RGBA8 pixels and its render and readback times, always in request order |
| `READBACK` / `VERTS` | app -> host -> app | the positions the host's mesh holds, the LOD it draws and that LOD's index count (for checks) |
| `ERROR`, `BYE` | either | a message, or the end of the session |

The host reads each frame back asynchronously (`RenderingDevice.texture_get_data_async`): the copy is queued behind
the frame's draw and delivered a couple of draws later, so the host can draw the next request while the previous frame
is still on its way. The client keeps at most two requests in flight and pairs every frame with the request number it
answers, so a frame is always shown with the camera, size, LOD and head revision it was drawn for. TCP and not shared
memory: GDScript has no shared-memory API (it would need a native extension), and loopback TCP is not the bottleneck
(a 1280x800 frame, 4 MB, arrives in about 2 to 3 ms). On an RTX 4090 with the client pacing 144 frames a second, a
full-size orbit shows about 106 to 114 Godot frames a second at 1280x800 and at 1920x1080, where the synchronous
readback this replaced reached 30 to 66. `--sync-readback` in `host_args` switches the host back to the synchronous
`get_image` path; both give byte-identical frames. While the camera moves the host renders at a reduced size
(`orbit_scale`, default 0.5; 1.0 keeps full size) and the client scales the frame up; once the camera has been still
for `still_ms` (default 120) the slot is rendered at full size. Only the newest camera of each slot is rendered, so a
slow host shows fewer frames but never a backlog. A frame that has not changed is not uploaded to the GPU again.

The client never waits for Godot: the connection runs on its own thread, and a slot without a current Godot frame is
drawn by the built-in renderer. A missing binary or project, a host that does not connect within `start_timeout_ms`,
does not answer a frame within `frame_timeout_ms`, exits or is killed, puts the mode into a failed state with the
reason shown in the slot's label, and the slot falls back to the built-in renderer until the mode is chosen again.

Settings live in a JSON file (`--godot-config FILE`, or `studio-godot.json` next to the key map; `--help` prints the
default path); `GAZEFORGE_GODOT` and `GAZEFORGE_GODOT_PROJECT` give the binary and project when the file does not:

```json
{
  "godot": "path/to/Godot_v4.7-stable_win64_console.exe",
  "project": "path/to/clients/godot-render",
  "script": "res://host.gd",
  "material": "res://materials/skin.tres",
  "environment": "res://environments/studio.tres",
  "scene": "res://lighting/key_fill_rim.tscn",
  "orbit_scale": 0.5,
  "still_ms": 120,
  "start_timeout_ms": 30000,
  "frame_timeout_ms": 3000,
  "visible": false
}
```

Without `material`, `environment` and `scene` the host uses a neutral studio clay that matches the client's own clay
mode with its default studio lights: a warm grey two-sided material with wrapped diffuse and no specular, the three
studio lights (key, fill, back) following the camera, a sky-to-ground hemisphere ambient and the viewport's background
colour (mean difference to the client's clay under 1 of 255 per channel on the head). To use your own look, point `project` at
your own Godot project, set `script` to the absolute path of `clients/godot-render/host.gd`, and name `res://` paths in
that project: `material` (any `Material`, applied to the character), `environment` (an `Environment`) and `scene` (a
`PackedScene`, instanced in place of the default lights; put your lights, probes or props in it). The project must have
been opened in the Godot editor once so its resources are imported. Paths that do not load are reported and the
defaults stay.

Limits: the character is the game mesh with its UVs at the slot's LOD; the slot's render mode and overlays (wireframe,
clip planes, points and the rest) do not apply to Godot frames, and the default look does not follow edits to the
client's lights; morph targets, skeletons and eyes are not sent; the host
renders one slot at a time, so in the quad layout the four slots share its frame rate. `visible: true` shows the host's
window, for debugging only.

Scripted steps for the mode: `source` with `"kind": "godot"` (optional `godot` settings object or `config` file,
`restart`), `godot_wait` (until every Godot slot shows a full-size frame of the current camera and head, or the mode
failed), `godot_status` (state, host info and the frame, latency and transfer timings; `reset` clears them),
`godot_readback` (writes the positions Godot holds, the client's positions and the vertex map), `godot_marker` (a marker
sphere at a vertex, with the vertex's projected pixel in every slot), `godot_track` (orbits and checks every new
Godot frame's marker against the camera that frame says it was drawn for), `godot_kill` and `godot_stop`.

### Projects

A project file `<name>.gfproj.json` keeps an ongoing piece of work: its characters (each a spec with a revision history;
a new revision is recorded when a save changes the spec), the panel states (body choice and stitch settings, expression
and joint settings), the viewer state (cameras per viewport, layout, render modes, overlays, lights, clip planes, the hide
box, regions and visibility), designer options and notes. The shared base (pack, catalog, game base) is referenced by
sha256 only, with a fingerprint of its blob sizes.

Everything outside the project file is a **resource**, kept as a role, an identity and a hint: `store` (identity: the
base fingerprint), `macro` and `eyes` (files: sha256 of the bytes), `face_morphs` (a folder: sha256 over every file's
relative name, size and content, whatever its size) and panel resources such as `panels.stitch.body` (a panel's
`getState()` puts its paths under `resources`). Files are hashed as they stream, on every save and load; nothing is
cached, so an edit is seen whatever it keeps of the size or the modification time. A folder's identity follows the
symbolic links and junctions inside it, as the loaders do, and covers what they lead to under the link's name (a folder
reached through a junction has the same identity as a copy of it); a link that leads back into the folder (a cycle)
cannot be identified and refuses the save, and a dangling link is recorded by name. The hint is relative to the projects folder when the resource lies inside
it, else absolute; it lives in the local project file only.

```
gazeforge call project.new    --input '{"name": "hero", "store": "STORE", "spec": "FACE.gfchar.json"}'
gazeforge call project.save   --input '{"name": "hero", "characters": [{"spec": {...}}], "notes": "..."}'
gazeforge call project.load   --input '{"name": "hero", "evaluate": true}'
gazeforge call project.list   --input '{}'
gazeforge call project.export --input '{"name": "hero", "out": "hero.gfproj.zip"}'
gazeforge call project.load   --input '{"zip": "hero.gfproj.zip", "store": "STORE", "eyes": "EYES.json", "body": "BODY_DIR"}'
gazeforge call project.discard --input '{"name": "hero"}'
```

- **Projects folder:** `root` in the input, else `serve --projects DIR`, else `GAZEFORGE_PROJECTS`, else
  `GazeForge/projects` under the user's data folder (the roaming application data folder on Windows, `~/Library/Application Support` on macOS,
  `$XDG_DATA_HOME` or `~/.local/share` elsewhere). Names are 1 to 64 letters, digits, spaces, `_`, `-`, `.`, so a project
  stays inside its folder.
- **Saving** writes a temporary file and renames it over the project, so a reader never sees a partial file and the
  last of several concurrent saves wins whole. `recovery: true` writes the autosave slot (`.recovery/<name>.gfproj.json`)
  instead; a normal save removes it. A save refuses a resource path that does not exist; the autosave keeps it as typed
  instead, reports it under `unresolved`, and loading that slot shows it again unresolved. With `evaluate: true` every
  character is evaluated before anything is written.
- **Loading** finds each resource, in order, from an explicit option (`store`, `macro`, `eyes`, `face_morphs`, `body`,
  or `resources: {"role": "path"}` for any role), then by its file name in the `search` folders, in
  `GAZEFORGE_RESOURCES` and in the projects folder, then from its hint; the first candidate whose identity matches wins.
  An explicit option is the only candidate for its role. Every role that stays unresolved is listed in one refusal
  (`not_found`, or `conflict` when a candidate exists with another identity); `skip: ["role"]` opens without it (never
  the store). Then the project is checked: base manifests by hash, the base blob sizes against the saved fingerprint
  (`verify: "full"` hashes every base blob), character blobs by hash, every character's revision, and the macro
  definition when a character uses macros. `evaluate: true` reports each character's evaluated dense sha256. Every blob
  that evaluation (or the designer's blob route) reads is checked against its hash on each read; the quick check only
  fingerprints sizes, so an unread same-size corruption is found by `verify: "full"` or by the first read.
- **Portable file:** `project.export` writes one `.gfproj.zip` with the project file, an index and the
  character-specific blobs (residuals), not the shared base. Its structured fields never carry paths: each resource is
  its role, identity and file name only, never a folder. Free text (notes, names, panel and viewer values, keys) gets a
  best-effort scan: the export is refused when any string or key, anywhere inside it, looks like a local path (drive,
  drive-relative, network, `//host/share`, `file:` URL, any other `scheme://host` URL except `http://` and `https://`,
  `~/`, an absolute path such as `/mnt/...`, `/Volumes/...` or `\data\...`, also right after punctuation such as `→` or
  `«`, an environment variable such as `%NAME%` or `$NAME/`), a user or home folder, the projects, temp or working
  folder, or holds the user name or the host name as a whole word (any case; `_` and digits separate words, so
  `backup_name_2026` counts). Each string is scanned as written and again normalised: percent-decoding repeated, Unicode
  NFKC, look-alike slashes, backslashes and colons made ASCII, every kind of space made a plain space, invisible
  characters removed. The scan reads one string at a time: a path split across two fields (the drive in one,
  the rest in another) is not seen. The refusal names each JSON pointer and the fragment (from the normalised text
  when only that matched). The notes are free text: `allow_paths_in_notes: true` exports them as they are; no other
  field has an opt-out.
  On another machine, `project.load` with `zip` takes the resources as options or finds them by file name in the
  `search` folders (never the projects folder, and the hints are not used), imports the project into a projects folder
  with hints for where they were found, and the blobs into the store; the receiving store must hold the same base.
  Each resource in the file must be a known role with a kind, a sha256 identity and a bare file name (no separators,
  `..`, drive, `:` or reserved device name); anything else refuses the file before a path is looked up. The loader
  always identifies folders by content, whatever the file says. Re-loading a zip into an existing project of the same
  name compares the whole project and refuses a different one unless `overwrite: true`. A load validates, resolves and
  (with `evaluate`) evaluates everything first and writes last: a refused load writes nothing.
- **Import limits:** an entry name that is absolute, holds `..`, a drive letter, `:` or a backslash, appears twice, or is
  a symbolic link or a folder refuses the whole file. Each entry is decompressed into a temporary folder that is removed
  afterwards, and its bytes are counted as they stream, never trusting the sizes in the headers: one entry may not pass
  `max_entry_bytes` (default 256 MiB) and all entries together `max_total_bytes` (default 2 GiB); the project entry is
  capped at `max_project_bytes` (default 16 MiB) before it is parsed, and parsing bounds nesting (64 levels) and the
  number of values. The project entry is read and parsed first; before any other entry is decompressed, the file is
  refused when it holds more entries than the project can reference (the project, its index and one blob per residual
  its characters name) or any entry the project does not name, so such entries cost no temporary space. A refused file
  writes nothing into the projects folder or the store. An import whose process was killed leaves its temporary folder behind; the next import
  removes such folders (named `gf-import-<process>-<n>` and older than an hour) and nothing else.

In the designer: New, Open, Recent, Save and Save As in the toolbar (Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+Shift+S; some browsers
keep Ctrl+N for themselves, the button always works), the project name with an unsaved marker, a warning before the page
is closed with unsaved changes, and a Project tab (characters, notes, portable export). `?project=NAME` opens a project
(`&root=` for another folder). When a resource has moved, opening the project lists each unresolved role with a path
field (and "open without it" for the optional ones) and retries with them; the project stays unsaved until it is saved
with the new locations. Every settle and every 60 s the page autosaves into the recovery slot while there are unsaved
changes (a path that does not exist is kept as typed and shown in the Project tab); opening the project after a session
ended without saving offers to restore or discard it. The undo history belongs to the session and is not saved.

### Image to character

A front photo and a profile photo of one face, plus any number of extra views, become a character spec and a glb, as
registry commands (so the CLI, HTTP and MCP all have them); the long ones are jobs.

**Direct fit (the default, `"fit": "photos"`).** The targets are fitted to the photos themselves; no 3D mesh is generated.

| stage | command | what it does and measures |
|---|---|---|
| 0 | `photo.ingest` | the reference-photo gate and the facing check on the front and the profile (below) |
| 1 | `photo.views` | per photo: 478 landmarks, the subject's outline (plain background) with outward normals, the profile keys of a side view; an extra view (`extras`: three-quarters, the other side, slightly up or down) gets the gate's quality checks (one face, size, sharpness, clipping, eyes open, mouth closed, plain background), never its pose limits |
| 4 | `fit.photos` | bounded least squares ([0, 1]) over the head targets to every photo at once (in `character.from_photos` a result that fails the shell gates is refused, `gate_failed`, and nothing is exported): landmark reprojection (`landmarks`: every landmark of the base table except the face oval, or the 30 stable ones) and outline distance (each fitted outline point of the model, its rim through the photo's camera, to the nearest photo outline point along that point's normal) through one perspective camera per photo; the cameras start from landmarks (front, extras) or the profile keys and the declared side (profile) and are re-solved every round, the gated front and profile held near straight (`pose_prior_deg`, default 3); stay-at-base on the outline regions not fitted (`prior_weight` 0.1), left/right symmetry, a ridge and an L1 term (`sparsity` 0.0007); per photo the landmark rms in px and per mille of the face height and the outline rms / p95 in mm; the shell gates on the result; `leave_out` names a photo to fit without and then predict |
| 8a | `export-glb` | the spec (weights; the residual is the fitted dense shape and is near zero) and the glb |
| 8b | `review.views` | one row per photo: the photo, a clay render at its solved camera, and both at 50 % with both outlines |

`other_profile` adds a profile of the other side. The front and the profile gate strictly; an extra view or the other
profile that fails its checks (no face, quality, no profile keys, no outline) is dropped, named in the report's `dropped`
and `warnings`, and the job goes on with the rest.

`fit_options` passes `fit.photos` options through `character.from_photos`. The outline of the hair-covered skull and of the
neck (clothing) is not fitted by default (`outline_regions`); the focal length is assumed (70 mm full-frame equivalent)
unless `focal_px` is given; scale and placement are not observable from photos. No texture is projected on this path yet.

**Dense shell (optional, `"dense_shell": true`).** The front and profile also go to an image-to-mesh server (`server` +
`graph`, or `glb` to reuse a mesh, as on the mesh path); the mesh's outer shell (`mesh.multiview`; an untextured mesh is
rendered as shaded clay for its landmarks) becomes an extra term of `fit.photos` (`shell`: the directory holding
`shell_points.npy`, `shell_normals.npy`, `shell_landmarks.json`). The shell is placed by its landmarks, re-registered
(similarity, trimmed ICP plus landmarks) every round, and pulls every vertex of `shell_regions` (default all but eyes and
neck) that a photo sees (`shell_coverage`: `seen`, or `all` for a complete surface) and that has the shell within
`shell_radius_mm` (default 8), its normal within 60 degrees of the vertex's, along the shell's normal, however far it
is from the shell (`shell_trim` false: the old 3x-median trim dropped exactly the vertices that differ most), with a dead band (`shell_tolerance_mm`: a vertex closer than that is not pulled, one further away is
pulled to the band's edge; default the shell's own `tolerance_mm` in `shell_landmarks.json`, which `mesh.multiview` sets
to 2 for a generated surface, else 0) and `shell_weight` (default 3); the stay-at-base prior leaves `shell_regions` to
the shell. Any surface in the base's frame up to a similarity can be a shell (a scan, a photogrammetry cloud with
normals). The generator is whatever the graph
runs; a generator whose licence restricts territories or users can only be used as a bring-your-own graph, never as a
default. Limits: a generated surface is a few millimetres off the true one, so the term helps a face far from the base
and hurts one close to it; the shell's landmarks come from MediaPipe on a clay render of the mesh and can fail on a
noisy surface (the run then stops with `worker_failed`); results that fail the shell gates are refused as before.

**Mesh path (`"fit": "mesh"`).** A generated multi-view mesh is the fit target:

| stage | command | what it does and measures |
|---|---|---|
| 0 | `photo.ingest` | copies the photos into the run directory; facing check: the front looks at the camera (nose offset within 0.08 of the eye span), the profile shows the declared side (the nose bump of the silhouette). A failure is `gate_failed` and stops the run. The proportion measure (upper lip over lower face, front vs profile) is reported in `consistency` and refuses only when `max_ratio_gap` is given: renders of one face differ by 0.16-0.28 on it, as much as two different faces do. |
| 1 | `photo.landmarks` | 478 MediaPipe landmarks on the front photo (re-run agreement in px), the profile silhouette, its profile line and key points (nasion, nose tip, subnasale, lips, chin); an overlay image for each photo, to be looked at before any number is trusted |
| 2 | `mesh.multiview` | sends both photos to an external image-to-mesh server, then keeps the outer shell and lifts the landmarks onto it from a front render. `shell_mode`: `outer_triangles` (default: triangles front-most from 26 directions at a fine depth tolerance, normals oriented toward the directions that see them) or `visible_vertices` (vertices visible from those directions, normals as the file gives them) |
| 3 | `fit.register` | similarity transform of the shell onto the dense base from 30 stable landmarks, refined by trimmed ICP on the face; landmark residual in mm. `base_landmarks`: the base's landmark table (`{"vertices": {landmark index: dense vertex}}`), else the landmarks are detected on a render of the base |
| 4 | `fit.targets` | bounded least squares ([0, 1]) over the head targets, with terms for distance to the shell (face and neck), front-photo landmark reprojection through a perspective camera (`camera`: `perspective`, the default, or `orthographic`; `focal_px` fixes the focal length, else it is solved), the profile silhouette (`profile_weight`, default 0; `profile_upright`, `profile_align`), a stay-at-base prior where the photos see nothing (hair-covered skull, back), left/right symmetry and a ridge; the shell is rigidly re-aligned each round; per-region residual in mm |
| 5 | `fit.residual` | a residual toward the shell (trust radius, graph smoothing, holds at the eyes, the lip seam, existing folds and back-facing surfaces), backed off until the shell gates pass; otherwise refused, and nothing is written |
| 7 | `texture.project` | projects the photos onto the game UVs through the fitted cameras, blended by facing and visibility; camera rms in px |
| 8 | `review.sheet` | the photos beside textured and clay renders of the result at the fitted cameras |
| 0-8 | `character.from_photos` | runs everything (with `export-glb` for the spec and glb) and logs per-stage times in `<run>/stages.json` |

```
gazeforge call jobs.submit --input '{"command": "character.from_photos", "input": {"run": "RUN", "front": "FRONT.png",
  "profile": "PROFILE.png", "profile_side": "right", "store": "STORE", "pack": "PACK", "catalog": "CATALOG", "game": "GAME",
  "name": "NAME", "eyes": "EYES.json", "sheet": "SHEET.png"}}'
```

External tools, found through options or environment variables:
- `python` / `GAZEFORGE_PYTHON`: a Python with numpy, opencv-python and mediapipe; `workers` / `GAZEFORGE_WORKERS`: the
  `workers/` folder of this repository; `landmarker` / `GAZEFORGE_FACE_LANDMARKER`: MediaPipe's `face_landmarker.task`.
- `server` / `GAZEFORGE_MESH_SERVER`: an image-to-mesh server with a ComfyUI-compatible HTTP API, and `graph` /
  `GAZEFORGE_MESH_GRAPH`: its API-format graph template taking a front and a profile image (placeholders are listed in
  `workers/mesh_worker.py`). `glb` reuses an existing mesh instead. Check the licences of the models the server runs.

Limits: from 30 front landmarks the focal length is barely observable (the reprojection error changes by a fraction of a
pixel across a tenfold range of focal lengths), so pass `focal_px` when it is known; the profile silhouette term is off by
default because its alignment drifts with the fit; the photos carry no absolute size, so scale comes from the base's landmark spans; the
mesh's hair is not separated from the skull, which is why the skull is held to the base; the texture is not delit unless
albedo images are passed to `texture.project` (`front_albedo`, `profile_albedo`); eyes, teeth and hair are not built.

#### Reference photos

The fit assumes two photos taken like a passport pair: a straight front view and a true side view, with the head neither
turned (yaw), nodded (pitch) nor tilted toward a shoulder (roll). A photo that breaks this is rejected before any work
starts, because a turned or tilted head is read as a different face shape. What makes a photo acceptable:

- one person, face filling a good part of the frame, in focus, evenly lit (no blown highlights or crushed shadows);
- a neutral expression: eyes open, mouth closed;
- a plain background that the head stands out from;
- the front photo looks straight into the lens; the profile shows the declared side at 90 degrees, the camera at head height.

`photo.check` (a query, so the CLI, HTTP and MCP all have it) measures this and returns a report per photo: every check
with its measured value, its limit and, on failure, a plain reason. `photo.ingest` and `character.from_photos` run the same
checks first and stop with `gate_failed`, naming every failed check; `thresholds` overrides a limit by name and `allow`
records a named failure as allowed instead of stopping.

The report's `calibration` block holds the measured error of every estimator and the limit chosen from it; where an axis
cannot reach the 3 degree target (front pitch, profile yaw, profile roll), it says so there. The mouth check measures the gap
between the inner lips against the face height. Two estimators are built but switched off in `workers/photo_gate_model.json`
because they measured worse on the calibration renders: `fit_head` (fit this person's head to the front landmarks and the
profile's keys, then measure the front pose against it) and `profile_out_of_plane` (read the profile's turn and tilt from
that head's silhouette over a grid of yaw and roll).

```
gazeforge call photo.check --input '{"front": "FRONT.png", "profile": "PROFILE.png", "profile_side": "right"}'
```

| check | how it is measured | default limit |
|---|---|---|
| front yaw | MediaPipe's facial transformation matrix, referred to the base mesh rendered straight on | 3 deg |
| front roll | PnP of 30 stable landmarks against the base mesh's own landmark positions (a 70 mm full-frame-equivalent lens is assumed) | 3 deg |
| front pitch | the mean of the transformation matrix's pitch and that PnP's pitch | 5.5 deg |
| profile pitch | trimmed ICP similarity fit of the silhouette's front edge (forehead to chin) to the base's straight profile; a poor fit counts as unmeasured and fails | 3 deg |
| profile yaw | the landmarker finds no face in a true side view; a face it does find means the view is turned toward the camera, and its reading is mapped to degrees | 15 deg |
| one face | face count from the landmarker (front); no face outside the subject's head (profile) | exactly one |
| face size | forehead to chin (front), nasion to chin (profile), in pixels | 300 px, 170 px |
| sharpness | Laplacian variance on the face region scaled to 256 px height | 81.9 front, 31.3 profile |
| exposure | share of face pixels at or above 250 (highlights) and at or below 5 (shadows) | 2 % each |
| expression (front) | MediaPipe blendshapes: the larger eye-blink score; the jaw-open score | 0.162; 0.4 |
| background | 95th percentile colour distance (Lab) of the background from its median; face-to-background contrast | 20; 26.6 front, 22.7 profile |
| profile side | the nose bump of the silhouette is on the declared side (the facing check) | must match |

The limits come from renders of the base and four training faces at known rotations (0, 2, 4, 8 and 15 degrees on each
axis and in combinations, front and right profile, eyes in place); `workers/photo_gate_model.json` holds the calibration.
A pose limit is never tighter than its estimator's 95th-percentile error there plus 1 degree, with 3 degrees as the target.
What the method cannot do, measured on those renders:

- front pitch: face shape alone moves single-photo pitch estimates by up to about 5 degrees (95th-percentile error 4.2), so
  its limit is 5.5 degrees and a nod of 8 degrees can pass on some faces;
- profile yaw: a view turned away from the camera, or turned toward it by less than about 10 to 15 degrees, is not
  detected; the profile's silhouette barely changes with yaw;
- profile roll (a tilt toward the shoulder, seen from the side) is not observable and is not gated; the front photo's roll is;
- expression is measured on the front photo only; the jaw-open score also rises on a long chin with the mouth closed;
- the facing check (unchanged) can reject a straight profile of a narrow face.

### Facial animation

A video of a face, or a capture file, becomes a glTF animation of a character's expression blend shapes and head
joint. Two registry commands (so the CLI, HTTP and MCP all have them):

```
gazeforge call jobs.submit --input '{"command": "animation.from_video", "input": {"video": "TAKE.mp4", "out": "OUT/take",
  "glb": "CHARACTER.glb", "landmarker": "face_landmarker.task", "neutral": {"start_s": 0, "end_s": 1}}}'
gazeforge call animation.from_csv --input '{"csv": "TAKE.csv", "fps": 60, "out": "OUT/take", "glb": "CHARACTER.glb"}'
```

- **Inputs.** `animation.from_video` (a job) reads a video file (`video`, decoded with OpenCV) or a folder of images in
  name order (`frames` with `fps`). A worker runs MediaPipe's Face Landmarker in video mode on every frame: 52 blendshape
  scores, read by category name (`_neutral` is dropped; the other 51 are the ARKit names), the facial transformation
  matrix (head pose) and 478 landmarks. `take` reuses the frames of an earlier run without the landmarker.
- **Per-frame gate.** A frame is rejected when no face is found (under `min_confidence`, default 0.5), when more than one
  face is found (`max_faces`, default 2, is how many it looks for), when the head is turned past the pose limits against
  the camera (`gate.max_yaw_deg` 40, `max_pitch_deg` 30, `max_roll_deg` 30, each less `gate.margin_deg`, default 3, for
  the landmarker's own angle error; `gate.relative_to_neutral` measures them against the neutral pose instead), when the face is smaller than
  `gate.min_face_px` (60) or when fewer than `gate.min_in_frame` (98 %) of its landmarks are inside the image. Rejected
  frames are filled by linear interpolation between the nearest accepted frames (slerp for the head) and held at the
  ends; every rejected frame and its reasons is in the report. More than `gate.max_rejected_fraction` (50 %) rejected,
  or none accepted, is `gate_failed`.
- **Calibration.** The rest face of a person is not zero on every channel. A neutral segment (`neutral` in seconds, else
  the first `neutral_frames` accepted frames, default 10) gives each channel's rest value (the median), and each weight
  becomes `clamp((score - rest) / (1 - rest) * gain, 0, 1)`; `gains` sets a channel's gain, `calibrate: false` turns
  it off. The head rotation is taken relative to the mean rotation over the same segment (`head_reference`: `camera`,
  the default, removes it as q0^-1 q; `face` as q q0^-1).
- **Range-of-motion calibration (optional).** `rom` lists windows where the performer, head still, takes named channels
  to their extreme. Each named channel's gain becomes 1 / its peak there (clamped to [1 / `rom_gain_max`, `rom_gain_max`],
  default 16); the other channels' leak from it is fitted over the window and subtracted; and the head's expression-driven
  motion is fitted over the windows and removed (`head_bias`). The report's `calibration_profile` holds the result and can
  be passed back as `calibration_profile` for later takes of the same performer. A gain is linear: a landmarker whose
  response is not proportional (on rendered faces jawOpen read high at half opening and low at full) keeps that error.
- **Couplings.** `couplings.squint_smile` (default 0.6) removes that fraction of a side's mouthSmile from its eyeSquint (a
  smile's cheek raise lifts the lower lid); `couplings.look_lid` (`{lo, hi}`, off by default) fades eyeLook* as the lid
  of that side closes. Both are fixed rules, not fitted to the performer.
- **Lid guard (opt-in).** With `glb`, `eyes` (the character's eyes JSON) and `lid_guard.enabled: true`, every frame's
  evaluated mix is checked per level of detail: it may not put more lid vertices inside an eye or turn more triangles
  against their normals than the worst of the neutral and the single shapes at weight 1; a failing frame's weaker
  channels on the failing vertices are scaled down until it passes. It is off by default because a single blink shape at
  a weight between 0 and 1 already fails that rule on the characters measured (its straight-line path crosses the
  eyeball), so the guard cuts blinks.
- **Smoothing.** A One Euro filter (Casiez, Roussel and Vogel, CHI 2012) per weight channel and per component of the head
  rotation vector: `smoothing.min_cutoff` (Hz, default 1), `beta` (20), `d_cutoff` (1), `head_min_cutoff` (4),
  `head_beta` (8); `smoothing.enabled: false` turns it off. The defaults were chosen on rendered test faces.
- **Retarget.** By name: the channel order is the character's morph target order (`glb`: the mesh's
  `extras.targetNames`), else `morphs`, else the source's names. A character morph with no source channel stays at 0
  and is listed (`tongueOut` has none from the landmarker); a source channel the character lacks is listed as dropped.
  `sides: "swap"` exchanges every Left/Right pair; `mirror: true` is for a mirrored (selfie) video and also mirrors the
  head rotation. On unmirrored synthetic renders the landmarker's Left/Right were face-relative, like ARKit's.
- **Outputs.** `<out>.clip.json` (`gazeforge.clip/1`: fps, times, channel names, weights per frame, head rotation per
  frame as a quaternion in the character frame), `<out>.report.json` (the gate and every rejected frame, the calibration,
  the smoothing settings, per channel the rest value, raw and calibrated range, mean, p95 frame-to-frame change and the
  same on the neutral segment, the head's largest angles) and, with `glb`, `<out>.glb`: a copy of the character with
  one animation (`name`, default `capture`): a `weights` channel on every head mesh node (LINEAR keys at the source
  frame times) and, when the character has a rig, a rotation channel on the `head` joint (`neck_share` gives part of it
  to `neck`). Head translation is not exported. `return_clip` also puts the clip in the result.
- **Joint tracks (approximate).** With a face-rigged character (`export-glb --rig face`), `joint_tracks: true` adds a
  second animation `<name>_joints` driving the facial joints instead of the blend shapes: per joint, the rest plus each
  shape's fitted pose difference times its weight (rotations summed as quaternion differences and normalised). This is an
  approximation of the blend shapes, not a solve; an engine plays one animation or the other, not both. On a character
  exported with `--rig face2 --rig-correctives on`, the joint animation does not drive the corrective morphs (they stay
  at 0), so it plays at the quality of the joints alone.
- **Live Link Face CSV (EXPERIMENTAL).** `animation.from_csv` reads a CSV whose header names ARKit blendshapes
  (PascalCase as the iOS app is believed to write them, or camelCase), with an optional `Timecode` column
  (`HH:MM:SS:FF`, frames at `fps`; otherwise row index / `fps`). The app's column list, timecode format and head rotation
  units are not documented by its maker and have not been checked against a real recording, so head rotation
  (`HeadYaw`, `HeadPitch`, `HeadRoll`) is ignored unless `head_units` (`rad` or `deg`) is given, with `head_signs` to flip
  axes; eye rotation columns and `TongueOut` are not used. The output carries an `experimental` note. Calibration,
  smoothing, retarget and outputs are the same as for video.
- **Designer.** The `Timeline` panel (`web/panels/timeline.js`) loads a clip from a file or runs either command, plays,
  pauses, scrubs, loops and changes speed, and shows the weights on the head (from the shapes' deltas, through `rig.face`;
  open the designer with `&face_morphs=`) and the head rotation (a rigid turn about the head joint in the preview, with
  the angles shown; the designer's eyes turn with it). It offers `getState()` / `setState(state)` for a host
  that saves panel state; the state includes the clip.
- **Limits.** The landmarker reads one face per frame and its scores are estimates relative to an average face, not
  measurements; channels it barely moves (brows lowering, eye look on a face whose eyeballs do not turn) come out weak
  however the gain is set. Head pose limits are against the camera. The model file is supplied by the user.

Licences: MediaPipe (the Tasks library and the Face Landmarker model cards) is Apache-2.0; the model file
`face_landmarker.task` is not part of this repository and is passed as `landmarker` (or `GAZEFORGE_FACE_LANDMARKER`).
The One Euro filter is written here from the paper.

### Data layout

A dense base directory holds `manifest.json`, `positions.npy`, `face_sizes.npy`, `corner_vertices.npy`,
`corner_uvs.npy`, and one `shape_<name>.npy` per shape listed in the manifest. A game base directory holds `kept.npy`
(the dense indices it keeps), `owner.npy`, `triangles.npy`, `triangle_uvs.npy` and `report.json`.

The dense base and the targets are not in this repository. `tools/blender/export_dense.py` and
`tools/blender/export_targets.py` produce them in Blender from MakeHuman's assets; a documented, self-contained data
recipe is on the roadmap.

### Macros (Python reference)

`tools/macros/macro_weights.py` turns MakeHuman macro slider values (gender, age, muscle, weight, proportions, height,
...) into weights over MakeHuman's macro targets:

```
python macro_weights.py MACRO_JSON TARGETS_ROOT SLIDERS_JSON [--mode mpfb|gf]
```

`mpfb` reproduces the weights MakeHuman's Blender add-on produces; `gf` is a variant with no jumps along any slider.

## Roadmap

Each milestone ends on a measured gate, not a declaration.

| | What | Gate | State in 0.2.0 |
|---|---|---|---|
| M0 | Pilot: the Rust core reads the dense base and targets and builds the shared ~20k-vertex game base; a sculpt maps onto it by index | per-region error against the dense sculpt within budget; no sliver triangles outside the eye and nostril areas; no UV flips; a painted look-development render passes review | open: slivers, canthus folds and the clay difference on held-out faces; review pending |
| M1 | Core: the character spec (JSON: target weights, head-only macros, residual), evaluation and placement onto the game base, shape and game-mesh gates (folds, inner surfaces, move roughness, triangle quality, UV flips, orientation, seams, path continuity), glTF export with blend shapes and the nested LOD chain | a spec round-trips to glTF and into a game engine; every gate fires on a planted defect | met; the shell gate's fold rule is under review |
| M2 | Interfaces: CLI, local HTTP API and an MCP server, all over one core | an AI agent completes "make a character with a wide nose" through each of the three | met through all three front doors |
| M3 | Browser designer: sliders and a live 3D view, a WASM preview kernel over the pure maths crate, heavy jobs on the server | slider response under 16 ms on the 20k base | met (frame p95 under 10 ms); server settle under 300 ms with `f5` on an idle machine |
| M4 | Image to character: facial landmarks and a multi-view reconstruction, fitted to the linear targets, then a residual and a photo projection | a character rebuilt from photos in under an hour of compute | open: under half of the gap to the truth closed |
| M5 | Game-ready output: LODs, a rig, expression blend shapes (ARKit-52) | a GazeForge head running in a game | partly: imports and plays in Godot 4.7; the closed eyelid of the blink shapes is a known defect |
| 0.2.0 | Facial animation: a video or a face-capture take to an expression clip on the character (per-frame gate, calibration, smoothing, retarget to the blend shapes and the face rig) | jitter on a still segment, reprojection onto the source landmarks, eyelid and facing gates per frame | experimental in 0.2.0: works end to end; reprojection, eyelid and head-pitch gates open |
| Next | Native studio: a native client over the in-process core, with a scripted offscreen mode for every check | the viewport matches the browser viewer's pixels; settles equal the registry's byte for byte | in progress: viewport, render modes, layouts, in-process loading, the shape input layer, live preview, worker-thread settles, undo, save and a Godot render mode done |

## Development

The repository follows gitflow: `main` holds releases, each tagged `vX.Y.Z`; work happens on `develop`, through
`feature/*` and `bugfix/*` branches; releases go through `release/*` and urgent fixes to a release through `hotfix/*`.
Releases and the gates each one leaves open: [CHANGELOG.md](CHANGELOG.md).

**Interface text and translations.** Every string either client shows comes from one table per language,
`web/locales/<lang>.json` (flat keys, `{name}` placeholders, `en.json` complete, other languages any subset of its keys).
The browser picks the language from `?lang=` or the browser's own, falling back to English per key; the studio embeds
`en.json` and overlays the file named by `GAZEFORGE_LOCALE`. Labels, buttons, tabs and menu items are in sentence case.
What agents read stays English on purpose: command summaries, limits and errors, job stages, and the wizard
definitions, which both clients look up as `wizard.<id>...` keys with the server's text as the fallback, so a
translation can cover them without a second copy in English. `tools/i18n/check.py` (also the `ui-text` gate of
`tools/dep_gate/dep_gate.py`) refuses a literal UI string in either client, a key the table lacks, an unused key, a
value in lower case and a translation whose placeholders differ from English; its docstring lists what it cannot see.

## Licensing and provenance

GazeForge is released under the [MIT License](LICENSE).

MakeHuman's assets (the base mesh and the targets) are CC0. MakeHuman's Blender add-on, MPFB, is GPLv3; no MPFB code is
included or adapted here. The Blender scripts only call it at run time, and the macro system is a clean-room
implementation written from `macro.json`, the target file names and observed output.

## Authors

Can Karakuzulu, with Sol (an AI assistant) as co-author.
