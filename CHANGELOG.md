# Changelog

All notable changes to GazeForge. The project follows [Semantic Versioning](https://semver.org); before 1.0 any minor
version may change interfaces. Every milestone ends on a measured gate, and a gate that is still open is listed here as
open, with its last measurement, rather than left out.

## [Unreleased]

### Added

- **Strand hair (first step).** `export-glb --strands HAIR.usda [--strands-count 20000] [--strands-points 16]
  [--strands-seed 1]` (also in the export task): the worn hair as USD `BasisCurves`, traced from its cards (texture
  columns followed along each card where opaque, every opaque run of 1 cm or more a strand, the direction per card
  piece chosen by which way its runs are longer, rooted at the end nearer the scalp over the card, coloured from the
  texture in linear colour, shared out by opaque card area in 3D). Verified with Blender 5.2's USD importer
  (`tools/diag/strands_blender_check.py`, not shipped): counts identical, every point on the glb's cards within 0.37 mm
  (ponytail) and 0.03 mm (long hair), renders beside the cards. Limits: layered like the cards; thinner than the cards
  over a long hair's crown at 20000 strands; fixed widths. A **Strands** switch per hair item in both Body shape
  panels (spec `body.wearables[].strands`, undo/redo, studio script op `body do=strands`): the stitch preview
  (`character.stitch` with `return_mesh`, new `strands_preview` count) draws that item as strand ribbons in its hair
  colour, and `--strands` traces only the items it is on. Gates: the studio undo gate with the switch (19 edits),
  the browser wearables gate (off, on, undo, redo; absent on opaque items), pseudo-locale scan clean.
- **Baked spring-bone physics.** `export-glb --bake sway|ANIMATION [--bake-format bones|vertex] [--bake-fps 30]`, the
  same as `wearable bake --glb IN --out OUT` on an exported file, and the "Bake hair physics into a glb" task: the
  chains of wearables worn with spring bones are simulated offline (VRM's spring bone against the body's capsules, 60 Hz,
  after a one-second settle) against a built-in four-second `sway` or an animation already in the file, and written as
  an animation `CLIP_baked`, as chain-joint rotation keys or as one morph target per frame (the chain joints taken out
  of those meshes' skin). Listed under `GAZEFORGE_physics.baked`. Verified: Godot 4.7's `SpringBoneSimulator3D` settles
  the same file's chain tips where the bake does (0.001 mm without capsules; with them 0.03 mm on the ponytail,
  0.4-1.6 mm on the long hair); three.js plays the bones and the vertex bake to the same
  vertices within 0.0001 mm at every key and within 0.22 mm between keys (slerped rotations against blended morphs),
  `tools/diag/bake_reader_check.mjs`, not shipped, proven to fail on a bake with a different stiffness (8.3 mm). Cloth
  is baked too (below); a bake does not loop exactly (about 4 degrees).
- **Baked cloth.** The same `--bake` simulates cloth items (worn with `"physics": "cloth"`): position-based cloth on
  each level's own mesh (edges hard, bending across edges, gravity, 240 Hz, 12 iterations), pushed out of the body's
  capsules, each vertex pulled to its skinned place by its `_PIN`; written as one morph target per frame in either
  format. The sway clip also drives the bone a cloth item hangs from. Measured on a test cape: no vertex inside a
  capsule at any level, the most-pinned vertices within 0.01 mm of the skin, stretch at most 11%, the hem swinging 21-23 cm;
  Blender plays it back with those vertices 0.002 mm from the skin (`tools/diag/cloth_bake_blender.py`, not
  shipped). Bakes are deterministic: constraint order no longer follows a hash map (three exports, one hash).

- **VRM 1.0 export.** `export-glb --format vrm --vrm-author NAME` (needs `--body-rig` with a humanoid map; optional
  `--vrm-name`, `--vrm-license`; also in the export task): the finest level only, the arms re-posed to a T-pose by the
  character's own skinning (the rest of the body as exported), `VRMC_vrm` with the meta, the humanoid bones and
  expression presets from the ARKit-named face morphs. Verified with three.js + `@pixiv/three-vrm` 3.3.3 in Chrome
  (`tools/diag/vrm_reader_check.mjs`, not shipped): VRM 1.0, all required bones, arms level to 0.00001 degrees in the
  world, `aa` drives its morph to 1, no metallic mesh. The coarser levels and cloth are left out.
- **Spring bones in VRM.** `--format vrm` writes the spring-bone chains as `VRMC_springBone` 1.0 (body capsules moved
  into the T-pose with the arms and fitted per chain, one spring and collider group per chain) and leaves
  `GAZEFORGE_physics` out of the `.vrm`. The `.vrm` now has scale 1 everywhere: the Character root's scale is folded
  into meshes, morphs, joints, eyes and animation translations (three-vrm 3.3.3 takes collider radii unscaled, Godot
  scales them). Verified with three-vrm in Chrome (`tools/diag/vrm_spring_check.mjs`, `vrm_world_compare.mjs`, not
  shipped): settled tips equal to the bake's (0.001 mm without contact, within 0.2 mm with), the capsules hold the hair
  in a 60-degree head turn (10.9 mm against 49.7 mm without), world positions unchanged by the fold (0.0001 mm, 13
  morphs), animation translations scaled exactly. `--strands` and `--bake` are refused with `--format vrm`.
- **The face rig on the body.** `--rig head|face|face2` now works together with `--body-rig`: one skeleton, the body's
  bones first (indices unchanged), the face joints under the body's head bone; the preset's jaw and eyes are taken over
  by the face rig, its other bones under the head are left out with their weights folded into their nearest kept
  ancestor (56 on `default`); per head vertex the body's head, jaw and eye shares are re-split by the face rig, neck and
  spine shares kept, up to 8 influences; face animations re-keyed to their new parents, their neck and head channels
  dropped; correctives computed on the merged weights. Measured on `default` and `game_engine`: rest identity 2e-8, seam
  gap 0 under a body pose (the seam moving 68-83 mm) and under each of four face shapes; face displacement against a
  head-only rig within 0.03 / 0.2 / 0.2 / 4.4 mm (brow, smile, blink, jaw open - the jaw's under-chin vertices keep the
  body's neck share); Godot 4.7 resolves all 259 face tracks, opens the jaw 23.7 degrees and carries it with a neck turn
  (`clients/godot-addon/tests/face_on_body_test.gd`).
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
- **Head joints follow the head.** On export with a body rig, the joints under the head bone move with the head's own
  sliders, head scale and sculpt (each by the inverse-square mean displacement of its 8 nearest head cage vertices,
  from the head the body targets alone would give to the head as exported). Measured (`default` rig, eyes moved by
  head sliders, head scale 1.1): eye joints 27.3 mm from the eyeball centres before, 5.3-5.6 mm after; on an unchanged
  head 3.1 mm before and 3.4 mm after (0.4 mm worse: the reference head and the body's helpers disagree by up to 6 mm at
  the neck ring, where the head keeps its own neck shape). B3 skin gate unchanged: rest identity 2e-8, seam gap 0 under
  a pose. `body.joints` applies the same fit (its joints equal the export's to 0.001 mm). With eye bones in the rig (`default`, `mixamo_unity`) and eyeballs on the character, each eye joint is placed at its eyeball's centre and the eyeball hangs from it: turning `eye.L` 30 degrees turns the left eyeball about its centre (vertices move 7.9 mm, their distance to the joint changes 0.0001 mm) where before it moved nothing (the eyes hung from the head bone).
- **Humanoid maps.** Each rig preset but `openpose` maps to Godot's humanoid profile and Unity's humanoid, written into
  the glb as the root extension `GAZEFORGE_humanoid` (`extensions/GAZEFORGE_humanoid/README.md`). Tested in the core
  for every preset: all required bones mapped, each under the bone its profile parent maps to, every Left bone on the
  character's left (a swapped side and a misplaced chest were planted and caught). Godot addon `gazeforge_humanoid`:
  the map attached to the skeleton on import, and a ready retarget bone map per preset; through Godot 4.7.2's own
  retargeter the bones take the humanoid names and an animation of `LeftUpperArm` moves the left hand 364 mm and the
  right hand 0 mm (`mixamo`, `default`); a wrong map fails the test.
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
  that bends as one piece, its long hair five strands that bend as a sheet. **A Godot reader:** the addon
  `gazeforge_physics` builds a `SpringBoneSimulator3D` with the colliders as capsules, and a `SoftBody3D` for each cloth
  item whose pinned vertices (`_PIN` >= 0.5) follow the skin (up to four bones each) while the free cloth collides with
  the capsules as moving bodies (editor import and run time). Measured in Godot 4.7.2 with movements ramped over half a
  second: long hair 0 mm into the body with the capsules, up to 49 mm without; a test cape's collar stays on the skin to
  0.05 mm and its free cloth 0 mm into the body with the spine bent 35 degrees either way, up to 78 mm without.
  Collider radii are the 10th-percentile distance (were the median: a round torso capsule stood out of the back by about
  10 cm). Open: the ponytail's 4 cm tie chain still reaches 13 mm into the neck capsule with the head tipped back 60
  degrees (29 mm counting the joints within 3 cm of its root); a few edges near the cape's hem stretch to about twice
  their length where it meets the leg capsules; the reader drives pins from a script, about 4 bone transforms per pinned
  vertex per frame. (The cape first settled 11 cm: a soft body ignores the scale in its transform, so it started 9%
  small; the cloth's mesh is now baked into world space at the character's size.)

### Changed

- **Equal layer depths: the later item is outer.** Two worn items with the same `layer` / `z_depth` were never layered
  against each other (only a strictly lower depth counted as underneath); now the item listed later in
  `body.wearables` goes over the earlier one. A cape worn after a suit (both 50) started 24.6 mm inside the suit;
  now 2.7 mm. Characters without tied overlapping items export unchanged (a five-item outfit: identical hash).
- **`wearable bind` turns inside-out garments the right way.** When most of a garment's normals point at the body
  it is bound to, it is stored with every face reversed (and `vn` negated) and the report says so (`faces`). The
  test cape was inside out (403 of 403 vertices): layering skipped it as an "inner face" and Godot culled it.
- **Spring chains start at rest.** Every export fits each spring chain's colliders to its modelled shape
  (`GAZEFORGE_physics.springs[].collider_radii` and `hit_radius_m`): a capsule a chain starts inside is shrunk to the
  chain's rest distance for that chain only. Before, a short tuft at the ponytail's nape started 11.6 mm inside the head
  capsule and was thrown 40 mm off its shape at the first frame in any solver (Godot included). The Godot addon gives
  each such chain its own thinner capsule (listed in child order: Godot ignores a deny-list entry out of that order). The default spring gravity is now 0.1 (was 1.0): the ponytail's tip settled
  65 mm off its modelled place under its own weight, now 26 mm.
- **Up to 8 joints per body vertex.** With `--body-rig` the body, head, genitals and worn items now keep up to 8
  joints per vertex (`JOINTS_1` / `WEIGHTS_1`); `--body-influences 4` (and the export task's field) keeps the old single
  set, byte-identical to before. Why: MakeHuman's weights name more than four bones on many shoulder vertices (`default`:
  926 of them, losing 7.8 percent of their weight on average and up to 33 percent; `game_engine`: 305, up to 24
  percent), and cutting them made the `default` preset's shoulders tear with the arms over the head
  (`tools/diag/arms_up.py`); with 8 they bend smoothly. The posed-garment results are unchanged; the skin gate (which
  now reads both sets), the humanoid maps and Godot 4.7's import (8 weights per vertex) pass. A natural arms-up pose
  also lifts the clavicles: turning the upper arms alone creases the top of the shoulder on any weights.
- **A painted hair cap at far levels.** At LOD4 and LOD5, where a hair or beard keeps too few cards to cover the
  scalp, the head takes a second material: its skin texture (or plain white without skin) with the item's colour
  (its texture's alpha-weighted mean) painted wherever the item's opaque texels lie within 12 mm outside the skin at
  full detail, feathered at the edge and made matte (roughness 1, flat normal). The hair is sampled every 3 mm over its
  triangles and only where its texture is opaque. `body.wearables[].scalp_cap` (default on for alpha-card items) and a
  "Paint far levels" box in both clients turn it off; the sidecar reports per item the colour and texels painted.
  The far material drops the skin's live layers (`GAZEFORGE_skin`). Looks to judge: under long hair the cap also
  darkens the ear and nape, and a dense dark beard becomes a solid dark jaw.
- **Hair keeps more of itself at middle distance.** On LOD1 to LOD3 a wearable now thins by the body's vertex ratio
  raised to its `coarse_power` (`body.wearables[].coarse_power`, 0.25 to 1; default 0.5 for hair, beards and other
  alpha-card items, 1 for everything else, so opaque clothes are unchanged); LOD4 and LOD5 thin with the body again.
  A ponytail keeps 2629 / 1859 / 1315 vertices at LOD1-3 (1859 / 930 / 465 before) and its scalp is covered at LOD3;
  long hair keeps 2290 / 1620 / 1145. Both clients have a "Coarse detail" number on each worn item's row.
- **Cuffs and collars follow the hand and head.** An opaque wearable fitted to the body now blends its skin weights,
  within `opening_m` (default 6 cm, 0 to 20 cm, 0 turns it off; `body.wearables[].opening_m`) of each open edge,
  toward the weights of the nearest body vertex: fully at the edge, fading to the binding's own weights at the reach.
  Measured with the posed gate on a dressed character (wrists bent 45 degrees, neck turned 40): the collar goes from 5
  body vertices through to none; the cuff from 36 (27.9 mm) to 7 (24.6 mm) at full detail and from 3 to 2 at LOD3; no
  pose gains a failure; the rest pose cannot change. With `opening_m` 0 the gate's results are the old ones exactly,
  and the export is byte-identical run to run. Rigid items, alpha cards and cloth are left as they were. Both clients
  have an "Openings (cm)" number on each worn item's row; the sidecar reports per item the reach and the vertices
  blended.
- **A per-character layer order for wearables.** A worn item's `layer` (in the spec's `body.wearables[]`, 0 to 1000)
  replaces MakeHuman's `z_depth` for that item: lower sits closer to the skin, and each opaque item is pushed out over
  every opaque item with a lower layer. Without it the item's own `z_depth` decides as before. Both clients have an
  "Own layer" box and a number on each worn item's row (the studio and the browser, in the one undo history);
  `wearable.list` reports each item's `z_depth`, and the sidecar each worn item's layer and the depth used. A layer can
  ask for what the meshes cannot do: low shoes ordered over a trouser leg swell by up to 83 mm and the hem shows
  through, so check the look after reordering.
- **Stitch gates on coarse levels of detail.** On LOD1 and coarser, the seam's normal-continuity limit is the larger of
  40 degrees and the sharpest angle inside either side at that level (the body's blend band, the head's seam row),
  capped at 60 degrees so that one sharp edge cannot open the gate; LOD0 and the stitch preview keep 40 degrees. The
  sidecar reports each level's limit and the cap. Over the 128-character grid the cap changes no result (the same 119
  pass, the same 9 fail with the same messages); the highest limit falls from 117.4 to 60 degrees.
- **Body and genital UVs in glTF's convention.** The body and genital meshes now write V as glTF reads it (1 - v of
  MakeHuman's OBJ), as garments already did, so a MakeHuman body texture maps the right way up; before, it landed on
  the wrong parts of the body. The head's UVs are unchanged (its textures are generated in glTF's convention already).
  Body glbs exported before this change carry the old V.

### Fixed

- **Baked cloth went through the body and the clothes.** It collided only with the body's capsules, which sit inside
  the skin; a cape sank up to 17.5 mm into the hips. It now also keeps 4 mm off the skinned surface of the body, the
  head and every other opaque item worn - or, where its rest already lies closer or inside, no deeper than at rest.
  Measured on the cape: 0 mm inside the body at every level (was 17.5 / 15.2 / 8.3), over a suit 0.02 mm beyond its
  rest; the report gives `inside_at_rest_mm` and `inside_beyond_rest_mm` and lists the surfaces it collided with.
- A mesh exported without a material (the head and body of a character without skin, plain eyes) took glTF's default
  material, which is fully metallic and renders black in viewers that follow the specification (three.js, Babylon;
  Blender and Godot substitute grey). Such meshes now carry a neutral material: light grey, metallic 0, roughness 0.7.
  Found by the VRM reader check, whose first renders showed the avatars' bodies as black.
- The wearable fitting's nearest-point lookup could return a point up to one 2 cm grid cell beyond the distance asked
  for. Exports of the dressed, ponytail, long-hair and cape characters are byte-identical before and after the fix
  (pins and push-out clearances never asked for a reach close to the cell size); the scalp cap, which does, found it.
- A glb wearable whose colour texture is a JPEG (generated items usually are) came in untextured; the JPEG is now
  decoded and kept as a PNG (one new dependency: `jpeg-decoder`, MIT or Apache-2.0, pure Rust, no dependencies of its
  own; the browser build does not include it).
- The eye model choice showed the eye inventory's names and raw family ids ("brown", "amber") in both clients, and its
  undo step showed the model's id; all of them are UI text now (`eyes.model.<id>`, `eyes.family.<family>`), and a test
  fails when an inventory row has no text.
- A glb wearable whose colour image is kept outside the glb was reported as having no colour texture; the binding now
  names the file. The `--body-rig` help and the export wizard's body skeleton field named 2 of the 7 rig presets; they
  name all of them, and a test fails when a preset is added without them.
- The Stitch, Body shape and Eyes panels of the browser designer laid every row into the next one; they now line up
  label, slider and value.

### Open (bodies)

- The neck seam with six levels of detail (ladder above, 800-vertex floor): 119 of the 128 characters of the macro grid
  pass the stitch gates at every level (116 with the earlier three levels); the 9 left carry 10 failures, 5 on band
  folds or flips and 5 on the seam angle (one character at two levels). The seams that pass stay at or under 53
  degrees.
- Wearables: inward-facing layers (a collar's underside, cuffs) stay where the binding puts them and can sit inside a
  heavier neck or wrist; with the wrist bent 45 degrees a cuff still lets 7 body vertices through (up to 25 mm) after
  the opening blend; at the coarsest levels a trouser hem can rise above the shoe opening; a card beard keeps only
  a few cards at the coarsest level; a bound garment follows the body's local shape more closely than MakeHuman's
  hand-chosen bindings (loose cloth can hug muscles); a rigid hat does not follow a head's shape (bind hats instead);
  a glb's colour texture kept outside the glb, or a CMYK JPEG, is not read (give a PNG; the binding's report names
  which).
- Head joints follow the head by a local fit (above): on an unchanged head they sit 0.4 mm further from the eyeball
  centres than before.
- With the thighs posed, a small ridge can show at the top of the genital patch's rim. There is no female option yet
  (MakeHuman ships no geometry for one).
- `openpose` has no humanoid map.

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
