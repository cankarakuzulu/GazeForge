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

The workspace is layered, and every edge points down: `gf-geom` (pure maths: no IO, no threads, no clocks; builds for
`wasm32-unknown-unknown`; parallel loops only behind its native-only `parallel` feature) <- `gf-core` (domain: placements,
catalog, character spec, evaluation, residual codec, macros, measurements; no IO) <- `gf-io` (formats and the store) <-
`gf-api` (the command registry, error model and jobs) <- `gf-cli` (the argv commands and the CLI, HTTP and MCP front doors).
`gf-wasm` (the designer's preview kernel) depends on `gf-geom` only.
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
                     [--placement optimal|fit|f2|f3|f4|f5|subset|NAME/VERSION] [--eyes EYES.json] [--tip-vertex N] ...
```

Each placement is versioned (`subset/1`, `optimal/1`, `fit/1`, `f2/1`, `f3/1`, `f4/1`, `f5/1`) and never changes once
released; a bare name means its latest version, and `f3/1` is the default of a character spec. With `--eyes`, `f3` and `f5`
keep the placed eyelids outside the eyeballs. The output names the version it ran (`placement_version`).

`f5` is the continuous placement: a small change to the dense shape makes a proportionally small change to the game mesh.
Each dense vertex is tied to a fixed game triangle and barycentric weights found in texture space (they do not depend on
the shape), and the game mesh is one linear least-squares solve: distance to the dense surface along normals taken from
the game-resolution subset, a weak pull toward the fixed correspondence, edge vectors kept close to the subset's, and a
tangential membrane that keeps triangles open. Seam vertices move along those normals only; the eyelid lift uses smooth
fades instead of cut-offs. There are no accept/reject steps, nearest-surface searches or data-dependent iteration counts
in the positions (the linear solver stops at a relative residual of 1e-13).

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
```

`import-catalog` reads `catalog.json` and its sparse targets (`.npz` with `indices` u32 and `deltas` f32 x 3, on the
dense order) as baked. `import-game` also imports `lod_<N>` subdirectories. `--macro-base` records the pack's own macro
slider values and `--pin-top` the height up to which head-only macros are pinned to the neck seam; both are needed for
characters with macros.

#### Characters

A character spec (`gazeforge.character/1`) is canonical JSON: the pack, catalog, optional game base and placement
version; target weights (each in [0, 1], rounded to 1e-6); optional macros; an optional residual. Its revision id is
the sha256 of those bytes, which is also the sha256 of the file `character create` writes.

```
gazeforge character create --store DIR --pack sha256:... --catalog sha256:... --out FILE.gfchar.json
                           [--game sha256:...] [--placement f3/1] [--name NAME] [--weights JSON|@FILE]
                           [--macro MACRO_JSON --sliders JSON|@FILE --macro-mode gf|mpfb --head-scale 1]
                           [--residual-from FACE.npy [--quantum-mm 0.005]]
gazeforge character validate --store DIR --spec FILE [--macro MACRO_JSON]
gazeforge character evaluate --store DIR --spec FILE --out DENSE.npy [--macro MACRO_JSON] [--precision f32|f64]
```

`evaluate` computes `base + sum(w * T) + head-only macros + residual` on the dense order; its output is a positions
file that `export-obj --positions` reads. `--residual-from` stores `FACE - (base + sum(w * T) + macros)` as an i16,
planar, delta-coded, zlib-compressed blob (`i16-planar-delta-zlib/1`); the default quantum of 0.005 mm keeps every
vertex within 0.0044 mm of the face.

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
                     [--lod-pixels 1] [--lod-height 1080]] [--rig none|head --rig-jaw TARGET --eyes EYES.json]
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
  character (the seam centroid, the fitted hinge moved with the face, the eye centres from `--eyes`), and two eyeball
  spheres `Eye_L` / `Eye_R` are children of the eye joints.

Morphs carry no UVs in glTF, so every LOD keeps the neutral character's UVs; the sidecar records, per morph, how far the
placement's own re-derived UVs at weight 1 sit from the neutral ones (`uv_slip_lod0`).

#### Gates

```
gazeforge gates shell --dense DIR --before base|shape:NAME|FILE.npy --after base|shape:NAME|FILE.npy [limits]
gazeforge gates game --dense DIR --base DIR --face base|shape:NAME|FILE.npy --obj GAME.obj [--gates triq,uv,census,seams]
gazeforge gates path --dense DIR --base DIR --start FILE.npy --delta FILE.npy [--steps 20] [--pop-limit 10]
                     [--placement f3 and its options] [--plant STEP,GAME_VERTEX,MM]
gazeforge gates facing --glb FILE.glb [--states neutral|all]
gazeforge gates lids --glb FILE.glb --eyes EYES.json [--states all|neutral] [--front-deg 75] [--tolerance-mm 0]
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
`character.measure`, `jobs.submit`, `jobs.status`, `jobs.cancel`, and `debug.slow` (a deliberately slow job that exists
to prove cancel). `commands.list` returns every schema; the argv commands above are unchanged.

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
when done); `jobs.cancel {"id"}` kills the worker and every process it started. On Windows each worker runs inside a Job
Object that is killed as a whole and closed on exit; the record's `worker_processes_left` is the job object's own count
after the kill. Elsewhere the worker gets its own process group, which is killed, and that count is null. Jobs live in
`GAZEFORGE_JOBS`, else `<temp>/gazeforge-jobs`, so a job submitted through one front door can be read or cancelled
through another that uses the same directory.

### Designer

A browser page served by `gazeforge serve`: sliders for the 290 head targets (cheek, chin, ears, eyebrows, eyes,
forehead, head, mouth, neck, nose) with search, a category filter and reset (double-click resets one slider), the macro
sliders (head only: the similarity is stripped and the neck seam pinned) when a macro definition file is given, a 3D view
(orbit, wheel zoom, front / 30 / 85 degree cameras, clay or normal shading), the `character.measure` measurements of the
settled character, and saving the character as a spec file.

```
gazeforge serve --port 7878 --web web       # --web: the folder holding index.html (default ./web, or GAZEFORGE_WEB)
http://127.0.0.1:7878/designer/?store=STORE[&spec=FACE.gfchar.json][&macro=MACRO_JSON][&eyes=EYES.json][&out=SAVE.gfchar.json][&fast=1]
```

While a slider is dragged the page shows a preview, labelled approximate: the last settled game mesh plus each changed
target's exact delta at the game vertices, computed in WebAssembly (`gf-wasm`, which also recomputes the vertex normals).
On release (150 ms debounce) the server settles the character (weights and macros -> dense -> the spec's placement ->
game mesh), and the settled mesh replaces the preview and becomes the base of the next one. Under the default placement
the settled mesh can differ from the preview, because the placement re-fits game vertices along the surface. Macro
sliders preview with a one-sided slope fetched when the drag starts (`designer.macro_delta`).

Everything heavy stays on the server: the dense pack, the catalog and the placement never reach the browser. The page
downloads one preview pack per target set (game triangles plus each target's i16 delta at the game vertices and a scale,
a content-addressed store blob, cached by hash) and the settled positions. The commands, usable from every front door:
`designer.sliders`, `designer.pack`, `designer.settle`, `designer.macro_delta`, `designer.save`. The served process keeps
one pack, catalog and game base in memory between settles. `GET /v1/blob/<sha256>?store=STORE` serves a store blob.

With them it keeps, built on the first settle, everything the placement computes that does not depend on the character:
boundary and seam flags, rings, edges, the UV charts and each game vertex's UV search patch, the f5 footprint. Later
settles compute only what depends on the dense positions, and their results are byte-identical to a one-shot placement.
`designer.settle` also places with f5. `&fast=1` asks for an opt-in draft: f3/f4 with 1 of its 4 fit rounds (input
`fast`, reply flagged `approximate`, labelled as a draft in the page). A draft is never saved or exported; after 3 s
without a change the page settles again with the full placement.

Building the page's kernel: `cargo build --release -p gf-wasm --target wasm32-unknown-unknown`, then copy
`target/wasm32-unknown-unknown/release/gf_wasm.wasm` to `web/gf_wasm.wasm` (`CARGO_PROFILE_RELEASE_DEBUG=0` keeps it
small). The page uses WebGL2 and no third-party scripts. On some Windows machines Chrome's default graphics backend gives
no WebGL context; `--use-angle=gl` does.

### Image to character

A front photo and a profile photo of one face become a character spec, a glb and a photo-projected texture, as registry
commands (so the CLI, HTTP and MCP all have them); the long ones are jobs.

| stage | command | what it does and measures |
|---|---|---|
| 0 | `photo.ingest` | copies the photos into the run directory; facing check: the front looks at the camera (nose offset within 0.08 of the eye span), the profile shows the declared side (the nose bump of the silhouette), and both photos agree on proportions (upper lip over lower face within 12 %). A failure is `gate_failed` and stops the run. |
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

| | What | Gate |
|---|---|---|
| M0 | Pilot: the Rust core reads the dense base and targets and builds the shared ~20k-vertex game base; a sculpt maps onto it by index | per-region error against the dense sculpt within budget; no sliver triangles outside the eye and nostril areas; no UV flips; a painted look-development render passes review |
| M1 | Core: the character spec (JSON: target weights, head-only macros, residual), evaluation and placement onto the game base, shape and game-mesh gates (folds, inner surfaces, move roughness, triangle quality, UV flips, orientation, seams, path continuity), glTF export with blend shapes and the nested LOD chain | a spec round-trips to glTF and into a game engine; every gate fires on a planted defect |
| M2 | Interfaces: CLI, local HTTP API and an MCP server, all over one core | an AI agent completes "make a character with a wide nose" through each of the three |
| M3 | Browser designer: sliders and a live 3D view, a WASM preview kernel over the pure maths crate, heavy jobs on the server | slider response under 16 ms on the 20k base |
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
