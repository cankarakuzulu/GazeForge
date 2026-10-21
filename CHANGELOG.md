# Changelog

All notable changes to GazeForge. The project follows [Semantic Versioning](https://semver.org); before 1.0 any minor
version may change interfaces. Every milestone ends on a measured gate, and a gate that is still open is listed here as
open, with its last measurement, rather than left out.

## [Unreleased]

### Added

- **Bodies shaped by the macros.** A spec's `body` (a MakeHuman body pack and its targets in the store) is shaped live
  by the macro sliders in the designer, the studio and `export-glb`; the head follows it (built in its canonical frame,
  the character mapped to the world by one `Character` root). Measured: a live body equals a fresh MakeHuman import to
  0.0002 mm; heights within 0.4 % of MakeHuman's own.
- **Regional body sliders and compound parameters.** `body.weights` (MakeHuman's regional targets, 117 sliders by family,
  `body.sliders`) and `body.compounds` (15 named mixes: leg and arm length, shoulder and hip width, torso length, waist,
  chest, athletic, curvy, belly, hand and foot size, arm and leg muscle, breasts natural / implanted); a Body shape panel
  in the studio and the browser, one undo step per change. A target the head's catalog also has drives both sides.
- **A finer body.** The body is shaped on its cage and refined by Catmull-Clark (level 2 by default, equal to Blender's
  own subdivision to 4e-7 m) before it is stitched; its game levels of detail are decimations of the refined body.
- **Rigs.** `export-glb --body-rig game_engine` (or `game_engine_with_breast`, `default`, `mixamo`, `mixamo_unity`,
  `cmu_mb`, `openpose`; MakeHuman's rigs and weights bundled, CC0) skins the whole character; `body.joints` returns a
  rig's joints on any shaped body. Measured: joints equal MakeHuman's rule to 0.001 mm on all eight rigs; in Godot 4.7
  the rest pose reproduces to 0.07 mm and the neck seam stays closed with the spine, neck and head rotated.
- **Genitals (male).** `body.genitals: "male"` exports MakeHuman's genital patch per level of detail, shaped by the macros
  and three sliders, skinned with the body.
- `export-glb --body-gates report` exports a body whose stitch gates fail, listing the failures in the sidecar.
- **Levels of detail on demand.** `derive-lods` / `base.derive_lods` (and the "Add levels of detail" task) makes a new
  game base from a stored one: its finest level unchanged byte for byte, any number of coarser levels decimated from
  it, so characters keep their head and a ladder can grow later; `--seam-weight` keeps more neck seam points on coarse
  levels. The same inputs give the same reference. A body import takes one level of detail per head level
  (`fine_lods`), each refined and decimated to its size. Recommended ladder: head 20000 / 10000 / 5000 / 2500 / 1200 /
  800, body 24000 / 12000 / 6000 / 3000 / 1500 / 800; 800 vertices is the floor.
- **Genitals, rig and gates in the clients.** The Body shape panel (studio and browser) has a Genitals choice when the
  body can wear one, undone and redone like every edit; the stitch preview draws it. The body import task takes the
  targets, the macro definition, the subdivision and the body levels of detail, with a Targets step; the export task
  takes the body skeleton and the gates choice.

- **Wearables (MakeHuman clothes, shoes, hats, hair, beards).** `gazeforge wearable import` / `wearable.import` (and the
  "Import a garment" task) puts a MakeHuman clothes file with its mesh and maps into the store; a spec wears it with
  `body.wearables: [{asset, hide_body}]`. Each garment's binding fits it to the shaped character (head items on the
  head, the rest on the body, through every macro, slider and compound), it is pushed out of the body (opaque items),
  layered over lower garments (MakeHuman's `z_depth`), the body it covers is left out (per garment, `hide_body`), and
  `export-glb` writes it textured (colour, normal and occlusion maps; alpha cards as a mask) on every level of detail - decimated per level to the body's
  ratio, cards kept whole - and skinned with the body rig. The Body shape panel (studio and browser) wears, hides and
  takes off wearables, with undo; the stitch preview draws them. `wearable fit` writes a lookdev OBJ with fit and
  cover measures; `wearable check` compares a binding with its own mesh. Measured: the binding reading is exact (to
  0.00015 MakeHuman units) for the items authored on a known body; most shipped garment meshes were authored on other
  bodies, so the binding, not the mesh, is the asset.
- **Any garment, and rigid items.** `wearable bind` / `wearable.bind` (the "Fit a garment made on a character" task)
  turns any OBJ or glb modelled over one of your characters into a wearable: each vertex bound to the nearest point of that
  character's body and head, the body it covers listed for hide-body; from then on it behaves like MakeHuman clothes.
  Measured on MakeHuman's own casual suit, cut out and bound again: on a heavy, muscular second character it lands a
  median 14 mm from MakeHuman's binding with less skin showing through, and its hidden list matches MakeHuman's on the
  body (Jaccard 0.92; shoes 0.98). `wearable attach` / `wearable.attach` (the "Attach a solid to a bone" task) keeps a
  rigid item - a sword, a helmet - in one bone's frame and places it on any character, at its modelled size, weighted
  wholly to that bone.
- **Physics data for wearables.** A worn item can ask for `"physics": "cloth"` or `"physics": "springbone"`;
  `export-glb --body-rig` then writes engine-agnostic data under the root extension `GAZEFORGE_physics`
  (`extensions/GAZEFORGE_physics/README.md`): capsule colliders per bone measured on the character, a `_PIN` weight per
  cloth vertex, and for spring bones chains of extra joints built from the item's own mesh (rooted where it rests on the
  anchor bone, the head for hair) with the item skinned along them. Measured: MakeHuman's ponytail becomes one chain
  that bends as one piece, its long hair five strands that bend as a sheet. No engine reader ships yet.

### Changed

- **Stitch gates on coarse levels of detail.** On LOD1 and coarser, the seam's normal-continuity limit is the larger of
  40 degrees and the sharpest angle inside either side at that level (the body's blend band, the head's seam row); LOD0
  and the stitch preview keep 40 degrees. The sidecar reports each level's limit.

### Fixed

- The Stitch, Body shape and Eyes panels of the browser designer laid every row into the next one; they now line up
  label, slider and value.

### Open (bodies)

- The neck seam with six levels of detail (ladder above, 800-vertex floor): 119 of the 128 characters of the macro grid
  pass the stitch gates at every level (116 with the earlier three levels); the 9 left carry 10 failures, 5 on band
  folds or flips and 5 on the seam angle (one character at two levels). The relative limit can rise far above 40
  degrees where a side has one sharp edge (up to 117 at LOD5 in the grid) - the seams that pass stay at or under 53
  degrees.
- Wearables: inward-facing layers (a collar's underside, cuffs) stay where the binding puts them and can sit inside a
  heavier neck or wrist; at the coarsest levels a trouser hem can rise above the shoe opening; a card beard keeps only
  a few cards at the coarsest level; a bound garment follows the body's local shape more closely than MakeHuman's
  hand-chosen bindings (loose cloth can hug muscles); a rigid hat does not follow a head's shape (bind hats instead);
  a glb's JPEG colour texture is not read yet (give a PNG).
- Body and genital UVs reach glTF in the OBJ convention (V up), so a body texture would map upside down; garments are
  flipped to glTF's convention.
- Head, jaw and eye joints follow the body's shaping, not the head's own sliders or sculpt.
- With the thighs posed, a small ridge can show at the top of the genital patch's rim. There is no female option yet
  (MakeHuman ships no geometry for one).
- The face rig cannot yet be combined with a body rig; no humanoid retargeting presets yet.

- **Topology improvement.** `topology improve` / `topology.improve` (a job) re-cuts a game base's triangles by edge
  flips over the same vertices, scored over the shape range the base serves (the rest shape, every catalog target, seeded
  combinations, optionally the dense pack's shapes), and stores the result as a new game base (its levels of detail too);
  `topology inspect` / `topology.inspect` reports a base's triangle quality. On the test store's 20,000-vertex base:
  triangles under 10 degrees at rest 777 -> 544, under 5 degrees 90 -> 66, over the whole range 5,468 -> 3,947; no scored
  shape gains a triangle under 5 or 10 degrees or a turned triangle.
- **Skin.** `skin.generate` writes a procedural skin for a game base (albedo, tangent-space normal, occlusion-roughness-
  metallic), evaluated on the neutral base's surface in millimetres so it has no UV seams; 12 presets in `skin.models`.
  `skin.project` projects a photos-path run's photos onto the texels through the fitted cameras, delit and gain-matched,
  and fills what no photo sees from the mirrored side and a tone-fitted procedural skin.
- **Skin layers and the `GAZEFORGE_skin` glTF extension.** A character spec can carry `skin`: a look, the base's landmark
  table (`skin.landmarks` puts it in the store), a texture size and layers - grime (dust, sand, dirt, mud), blood (drips,
  splatter), paint (eye-band, cheek-stripes, chin) - each baked into the textures or live. `export-glb` then gives every
  head level of detail a textured material: the baked skin in the core glTF material, so any viewer shows it, and the
  live layers in the [`GAZEFORGE_skin`](extensions/GAZEFORGE_skin/README.md) extension as masks a renderer drives by
  name (amount, and a second state such as wet or dried). `skin.layers` lists the kinds and styles. Measured: with no
  baked layers the exported textures equal `skin.generate`'s exactly; a layer baked at an amount equals the same layer
  live at that amount (max difference 0 in 8 bits). Not yet: relief (layers change colour and roughness only), painted or
  stamped masks, a textured body.
- **Godot addon** (`clients/godot-addon`). Imports `GAZEFORGE_skin` materials, in the editor and at run time, as shader
  materials with one uniform per layer parameter; masks arrive lossless; invalid extensions are refused with the core
  material kept. Measured on Godot 4.7.2 (Forward+): the rendered layers match the format's math within 1 in 8 bits.
- **Quads for OBJ.** `export-obj --quads on` writes clean triangle pairs as quads (off by default).
- **Designers.** A Topology tool in the browser and the native studio: bad-triangle highlight, improve as a job with
  progress, the numbers and the look before and after, and designing on the improved base. `designer.sliders` takes an
  optional `game`; the browser takes it as a URL parameter, the studio in its Store section.

### Changed

- `designer.sliders` without a spec defaults to the store's largest game base that is not a topology result (before:
  the largest, ties by reference), so importing an improved base does not move the default.
- `export-obj` and `export.obj` accept `quads`; the unknown-option message lists it. Existing OBJ output is unchanged
  byte for byte.

### Open (measured)

- On the held-out sculpted face the f5 placement on the improved base has 30 triangles under 5 degrees (41 on the old
  base) but 10 under 2 degrees (7): placed vertices move with the triangulation (median 0.24 mm, up to 2.9 mm on the
  neck and shoulders), and three unchanged eye-region triangles got thinner. Census over 90 degrees 26 -> 27.

## [0.2.0] - 2026-10-19

Facial animation, released as **experimental**: it works end to end and is deterministic, but its quality gates are
open, and every number below was measured on rendered test faces - no recording of a real person has been processed yet.

### Added

- **Facial animation from video.** `animation.from_video` (a job): MediaPipe's Face Landmarker on every frame
  (blendshape scores and head pose), a per-frame gate with rejected frames interpolated and listed, neutral and optional
  range-of-motion calibration, a squint/smile coupling, One Euro smoothing, retarget by name to the character's morphs,
  head rotation, and a glTF animation written into a copy of the character (a weights channel per head mesh, a head-joint
  rotation, and optional approximate joint tracks for the face rig). An opt-in lid guard.
- **Face-capture CSV (EXPERIMENTAL).** `animation.from_csv` reads a CSV of ARKit blendshape columns, as a Live Link Face
  recording is believed to have; the app's columns and units have not been checked against a real recording.
- **Designer.** A Timeline panel: load or run a clip, play, scrub, loop, speed, with the head rotation shown; it offers
  its state, clip included, through the panel API's getState / setState.

### Changed

- The tool version written into glTF exports is now 0.2.0. Every other byte of existing exports is unchanged: the glb
  files differ from 0.1.0's only in the three version characters, and the placement and OBJ suites are identical.

### Open gates (measured, not met)

- **Exact truth on a rendered clip with known weights:** the 8 driven channels pass correlation, error and lag; head
  pitch r 0.921 (bar 0.95). Brow-down reaches 0.04-0.05 of its true peak (bar 0.6-1.4): the landmarker barely moves it on
  these renders. Channels the clip never drives leak up to 0.28 (p95; bar 0.15), mostly while the head turns.
- **Reprojection** of the animated character onto the source landmarks: median 7.2 px (bar 4 px).
- **Eyelids and facing on mixed expressions:** fail. A single blink shape at a weight between 0 and 1 already puts the
  lid inside the eyeball, because its straight-line path crosses the ball - the same root cause as the eyelid defect
  listed under 0.1.0. The opt-in lid guard passes this gate only by cutting blinks.
- **Godot:** blend weights match exactly; skinned positions differ by 7.2e-5 m against a 1e-5 m bar, which is the
  engine's own rest-pose error with its 16-bit skin weights (7.1e-5 m).
- **A held-out face** fails blink and brow correlation, the wink check, the brow and blink amplitude bars and jitter.
- On a character with face-rig correctives, the joint-track animation does not drive the correctives.

Every open gate of 0.1.0 is unchanged.

## [0.1.0] - 2026-10-19

The first edition: every milestone of the roadmap has working code, a CLI, HTTP and MCP front door over one registry, and a
browser designer. Several of the milestone gates are still open; they are listed below with what was measured.

### Added

- **Core (M0, M1).** The dense base (the MakeHuman head subdivided to 1,151,889 vertices) and its 845 sparse targets,
  addressed by vertex index. The shared ~20k game base built by multi-shape half-edge collapse with region weights and
  quality guards, nested LODs 20k / 8k / 3k. The character spec (target weights, head-only macros, residual), evaluation,
  and placement onto the game base: `subset`, `optimal`, `fit`, `f2`, `f3`, `f4`, `f5` (default) and `f6` (opt-in). A
  released placement version never changes its output. Shape and game-mesh gates: folds, inner surfaces, move roughness,
  triangle quality, UV flips, orientation (fold rule a-2), seams, path continuity, eyelid clearance. glTF export with
  blend shapes and the nested LOD chain.
- **Macros.** A clean-room macro system written from `macro.json`, the target file names and observed output; `gf` mode
  (no jumps along any slider) and `mpfb` mode.
- **Interfaces (M2).** One command registry behind the argv CLI, a local HTTP API (loopback only) and an MCP server, with
  a shared error model and cancellable jobs.
- **Designer (M3).** A dependency-free browser designer (WebGL2, own shaders) with a WebAssembly preview kernel and
  server-side settles: sliders, undo / redo, hide, planar clipping, light controls, wireframe / Gouraud / Blinn-Phong /
  anisotropic and topology-inspection shaders, orbit camera with focus, perspective and orthographic views, multiple
  viewports, and a panel API for extensions.
- **Projects.** Save and load ongoing work as `<name>.gfproj.json`: characters with their history, body and stitch
  settings, rig settings, the viewer state and notes. Outside resources (the store, eye and macro files, morph folders)
  are recorded by identity and resolved on load from options, search folders or a local hint; a missing or changed
  resource is refused with one message naming every role. A portable `.gfproj.zip` carries no local paths; zip import
  checks names, caps sizes as it streams and writes nothing unless the whole load succeeds. The designer has New / Open /
  Save / Save As / Recent, an unsaved marker, autosave with crash recovery and a relink dialog; registry commands
  `project.new / save / load / list / export / discard`.
- **Image to character (M4).** A reference-photo gate (straight front and true side views, rejected when the head is
  turned, tilted or rolled beyond the limits, or the image fails quality checks); any number of extra views, each gated
  and posed; a direct fit of the linear targets to landmarks and outlines across the views; an optional dense-shell term
  for a mesh from an image-to-3D server or a photogrammetry cloud; a residual with the shell gates enforced.
- **Game-ready output (M5).** ARKit-52 expression blend shapes (51 shapes; no tongue geometry), morph-state facing
  repair, eyelid collision applied only when the lid gate fails, measured LOD switch distances, a head and neck rig.
- **Body stitching.** An exact seam onto the CC0 MakeHuman body and a seam-fitting blend onto any user-supplied body (neck
  ring found and matched; controls for blend width, neck height and scale), with seam gates; the designer panel runs with
  the gates enforced and draws a stitch that fails them in red.
- **Face rig (opt-in).** 29 joints (24 facial) skinned from the dense base and fitted to reproduce the ARKit shapes,
  exported beside the blend shapes as glTF animations; designer controls to pose expressions and joints. A second opt-in
  variant, `--rig face2` (37 joints, 12 of them on the eyelids, up to 8 influences), with optional per-shape correctives
  (`--rig-correctives on`).

### Open gates (measured, not met)

- **M0, default placement `f5`, five held-out test faces:** per-region error within 2x of the per-face reference on 4 of
  5 (one face's ears at 2.1x); triangle quality fails on all 5 (2-7 slivers under 5 degrees outside the eye and nostril
  areas; `f6` removes them on 4 of 5); orientation census (a-2) fails on all 5 (1-25 visible triangles over 60 degrees,
  mostly at the lateral canthus, where the dense shapes themselves fold); the clay-render difference gate fails on all 5;
  the eyelid gate fails on 1 of 5 after placement (0.178 mm2 of lid inside the eye; the glTF export's `--lids auto`
  collision clears it); UV flips 0, seams and path continuity pass on all 5. The painted look-development review is
  pending.
- **M1 shell gate:** a legitimate training shape exported without any fit fails the fold count on its own, so the gate
  refuses some valid shapes; its definition is under review.
- **M3:** the slider gate passes (frame time p95 7.3-8.6 ms, target 16 ms). The server settle meets its 300 ms target
  only with `f5` on an idle machine (p50 / p95 164 / 179 ms over 12 drags; `f3` 380 / 408 ms); under heavy load `f5`
  measured 254 / 482 ms.
- **M4:** the pass bar (every region better than the base and at least half of the base-to-truth gap closed, on renders of
  known faces) is not met. Gap closed: photos only 0.169; with a generated multi-view shell 0.285; with a photogrammetry
  cloud 0.255; more views saturate near 0.23. The photo path's regularisation does not scale with the evidence (root
  cause found, not yet changed).
- **M5:** the morph-state facing gate still fails on some expression states; a head running in a shipped game is not yet
  shown (imports and plays in Godot 4.7).
- **Known defect - the closed eyelid.** The eyeBlink targets close the lid inside the eyeball on every face. With
  `--lids auto` the closed lid is pushed out vertex by vertex onto the eye mesh, which leaves it crumpled, and the eye can
  still show through on the coarser LODs; without it the eyeball shows through. The vertex-based lid gate does not see
  either. A fix that bakes a corrected closure before placement, with a coverage gate, is in progress.
- **Face rig:** the pose gates fail for the joint rigs. `face` folds more triangles and pushes lid vertices into the eyes
  where the blend shapes do not, and blinks do not close through the joints; `face2` folds less but explains the shapes
  less well and also leaves blinks open. `face2` with correctives passes the pose gates, but by construction (each
  corrective restores its blend shape at weight 1), at about 2.7x the file size. The blend shapes remain the reference.
- **Body stitching:** some in-range settings of smoothing and tangent fail the seam gates (the designer shows them as
  failed); the any-body mode has no universal blend width.
- **Projects:** the free-text scan that keeps local paths out of a portable export checks each string on its own, so a
  path split across two fields is not caught.

### Licensing

MIT. MakeHuman's assets are CC0; no MPFB code is included or adapted. Dependencies keep their own licences (see
`Cargo.lock`); `unicode-normalization` carries Unicode-3.0-licensed data tables, whose notice must accompany any binary
distribution.
