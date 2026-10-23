# Changelog

All notable changes to GazeForge. The project follows [Semantic Versioning](https://semver.org); before 1.0 any minor
version may change interfaces. Every milestone ends on a measured gate, and a gate that is still open is listed here as
open, with its last measurement, rather than left out.

## [Unreleased]

### Added

- **Hair under hats: tuck or hide.** Hair cards came out through a hat's crown (196 vertices of a short hair through
  a fedora, up to 13 mm): alpha cards were never layered, and hat and hair share a layer depth. A worn item now has a
  `hair` setting (Hair under it, in both panels; `tuck` by default, or `hide`) for the alpha-card hair under it above
  the ears: tuck squashes the hair that would show through it just under it; hide also leaves out the hair inside its
  crown. Both act only on the item's triangles that do not face down, and hide leaves the brim alone (the flat band
  within 2 cm of its lowest point), so hair that shows under a brim stays. On the dressed test character: hair through
  the crown 362 triangle centres -> 0 in both modes; hide leaves out 417 hair triangles at the finest level.
  Grown hair (a groom's strands) follows the same setting under an item that covers the top of the head (a hat or a
  hood; a beard or glasses never hold hair down): every strand point above the ears and above the item's lowest point
  is kept 2 mm under the lowest of its surfaces straight over or under it (segment middles too, the higher end coming
  down first), never below 2 mm under its lowest point, and hide leaves out each strand's points under the crown from
  the root. A quiff under a fedora: 151,027 points lowered, 0 points and 0 segment middles left showing; hide leaves
  out 5,466 strands entirely. The strands report gives these counts. The body preview in both clients does the same,
  so the hat looks there as it exports (groom vertices above the fedora's top in the preview: 1,252 -> 0). Known:
  where a hat sits inside the head (the test fedora at the temples), hair held under it ends inside the skin there,
  hidden by the hat (30 cm hair: 1,507 points, at most 10.5 mm deep).
- **Bob with bangs, curtain bangs, long layers.** Three more styles (fourteen in all); a style now also sets the
  fringe (and every style sets face framing, so picking a style without them takes them away). The colours,
  highlights and forehead stay as they are, like the part.
- **Ombre, highlights, balayage.** A groom's `tip_colour` (the colour blends from the roots to it along each
  strand; absent keeps it even), `highlights` (the share of locks that are highlighted, a lock being one clump, so
  they read as streaks) and `highlight_colour` (default #b8925a; faint at the root, full a third of the way down).
  Both panels have Tip colour and Highlight colour pickers and a Highlights row. Game hair cards follow: the atlas
  paints each column from the root colour to the tip colour, gives the highlights their own columns (a seventh of
  the card columns per seventh asked, at least one), and a card whose strands are mostly highlighted takes one of
  them. A groom without these settings makes byte-identical cards. The groom report counts the highlighted strands;
  the cards report the highlight cards and columns.
- **Face-framing layers.** A groom's `frame_m` (Face framing in both panels; 0 is off): the hair rooted along the
  front hairline, ahead of the ears, is cut to that length, blending back to the full length over 5 cm behind the
  hairline, and is swept out to the sides so it falls beside the face rather than back into the rest. With a
  fringe, the fringe keeps the middle 60 % of its width and the layers take its sides. The groom report counts the
  framed strands, and now also strands with a point in front of the eyes and strands lying on the bare forehead.
- **Fringes.** A groom's `fringe` (none, straight, curtain; a Fringe choice in both panels) and `fringe_m` (its
  length): the hair rooted in a 4 cm band above the front hairline, between the outer eye corners, is combed down
  over the forehead instead of with the rest. A straight fringe is cut 8 mm above the brows; a curtain fringe parts
  at the centre and sweeps out past the eyes, where the face keep-out lays it beside the face. A ponytail leaves the
  fringe out, so a ponytail can have bangs. Fringe hair clumps only with fringe hair (a curtain with its own side).
  The groom report counts the fringe strands, how low the fringe reaches at the centre and how many fringe points lie
  below the brows.
- **Fades, undercuts, crops, quiffs.** A groom has a sides length (`sides_m`, 0 keeps the sides as long as the top),
  a fade (`fade_m`, the height of the blend from the side length up to the top length, centred a little above the
  ears), a top direction (`direction`: from the part, back, forward or up) and a lift for the top (`lift`, how far it
  stands off the scalp before it is combed). Hair combed back or forward from a styled top shortens a little toward
  the back of the crown, so a combed-back top does not run on down the neck as a tail; hair combed up rolls over at
  the front into a quiff, and the rest of the top lies back. New styles: buzz cut, crop, undercut, quiff, slick back.
  Both panels have the four fields and a Top direction choice; a style sets them too (one undo step).
- **Game hair cards from a groom.** `groom cards` (task "Make game hair cards from a groom"): the groom grown on the
  character, its strands grouped by shape (root, middle and tip, so strands combed apart never share a card) into
  600 cards by default, each a strip along its group's mean strand as wide as the group, a 1024 px strand atlas
  painted in the groom's colour, a card that would reach in front of the face ending before it; stored as a
  transparent, double-sided wearable through `wearable bind`, so it follows every shape and takes the hair ladder,
  the scalp cap and spring-bone physics. Gate (`tools/diag/groom_cards_gate.py`, not shipped, proven to fail on
  single-sided cards): deterministic, binding back within 0.01 mm, faces outward, worn and exported on six levels
  with a masked double-sided material, strands traced back from the cards. Worn with spring-bone physics, every
  level's cards follow the chains alike (mean chain weight 0.53 on LOD0-3, 0.56-0.59 on LOD4-5), so the far sets do
  not pop when the level changes; the scalp cap and hair lying on the head stay with the head.
- **Ponytails.** A groom's `tie` (none, low, high; a Tie choice in both panels, and a Ponytail style): every strand is
  combed along the head to a tie point at the back of the head (below the crown, or at the nape), joins a 2 cm bundle
  and hangs from it as a tail that fans out a little toward its end, its length kept as asked. A strand now also gets
  a point at least every 1.5 cm (a 45 cm strand had 3 cm segments, and its first one stood off the forehead).
- **Hair styles.** The Grown hair section in both panels has a Style choice - short, bob, long, wavy, curly - that sets
  length, droop, clumps, curl and frizz at once (one undo step) and keeps the part, colour, strands and seed; edit any
  value after and it shows as custom. `body.sliders` serves the styles; `gf_core::groom::{STYLES, with_style,
  style_of}`. Every style passes `groom_gate` (0 flyaways, 0 strands under the jaw, nothing inside the skin); the
  flyaway distance now allows for the groom's curl radius (a wavy groom's hair hanging off the back of the head is
  not a flyaway).
- **Make game hair cards from the Grown hair section.** A button in both Body shape panels opens the task for the
  current character, with the store, the macro and the character already filled in.
- **Groom cards have a scalp under them.** A cap over the head's scalp skin region (lifted 1.5 mm, decimated to
  700 vertices on the head's own UVs) rides with the near cards, painted in the atlas's eighth column: opaque hair
  where the scalp weight is full, fading in streaks where it falls, so no bare skin shows between cards and the
  hairline is soft. Strand tracing (`--strands` from worn cards) now skips any piece whose UVs fold over (more than a
  tenth of its triangles mirrored) - such a piece is not a card; MakeHuman hair traces byte for byte as before.
- **Groom cards keep their cover on the far levels.** A wearable can now carry more than one set of faces in its
  mesh, each from a level on (`levels` in the store manifest; LOD0 and every reader that wants one mesh see the
  first set). `groom cards` writes three: the 600 cards, then 150 cards from every other strand point (up to 80 mm
  wide) from LOD4, and 75 from every fourth point (up to 120 mm) from LOD5, each thinned to the level's usual vertex
  budget. Before, LOD4 and LOD5 kept a few dozen of the narrow LOD0 cards and read as loose threads. Existing
  wearables export byte for byte as before. Gate: `groom_cards_gate.py` reads every level's index buffer (near cards
  to LOD3, only 9- and 5-row cards on LOD4 and LOD5), proven to fail on a ladder that ignores the sets.
- `wearable bind --double-sided on`: a bound wearable that renders both faces (hair cards, thin sheets).
- **Grooms: hair grown from scratch (first step).** `export-glb --strands X.usda --groom GROOM.json`: strands rooted on
  the character's own scalp (the head's `scalp` skin region, from 478 built-in landmarks on the dense base, so one groom
  fits every head), combed away from a part (left, right, centre or none; backwards behind the crown, back and
  sideways at the front hairline), bent by droop, kept 3 mm outside the head and body, then clumped, curled and
  frizzed, and kept off the face: no strand point between the outer eye corners (plus 1 cm, widening ahead of
  the eyes) from the chin to the forehead; such a point moves to the zone's nearest edge, so front hair falls beside
  the cheek. Gate (`tools/diag/groom_gate.py`, not shipped, proven to fail on a groom grown inside the skin): roots on
  the scalp (mean scalp weight 0.97), length as asked (0.249 of 0.25 m), at most 2 mm inside the skin, deterministic.
  A character keeps its groom in `body.groom` (validated with the spec: a misspelt or out-of-range setting is refused),
  and both Body shape panels have a **Grown hair** section (on/off, part, strands, length, droop, clumps, clump
  strength, curl, frizz, seed, colour; every change undoable); the stitch preview draws up to 4000 of its strands and
  `export-glb --strands` grows it when no `--groom` file is given (identical output to the same groom as a file).
  Known: a strand pressed onto the face zone's edge can read as a straight line beside the cheek; a few strands can
  hang under the jaw, and with a side part a few rise above it.
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

- **Design panels sat in developer order.** Both clients now list Body shape, Body, (Eyes,) Face rig, Skin (studio)
  and Timeline first, and the developer sections (Measurements, Pins, Topology, Store, Jobs; Topology and Diagnostics
  in the studio) after them. The studio keeps Store and Project at the top.
- **Status lines spoke engine.** "Settled f5/1 1210 ms" reads "Ready in 1210 ms" in both clients, with the
  placement in its tooltip; a draft reads "Draft in … ms (approximate, …)". The browser header shows "20000 vertices ·
  39636 triangles · 290 targets" and keeps the GL renderer string in its tooltip.
- **The browser opened without a store was a dead end.** It now shows a start card: a store folder and an optional
  macro file (remembered from the last visit in this browser; Enter or Open loads them), Set up a store… (opens Tasks)
  and the saved projects, each opening on a click. The viewport no longer says "Approximate preview" on an empty
  page.
- **Every wearable carried a short hash in its name.** Both clients name an item by its name alone and add the
  first 8 hex digits of its reference only when another item in the store has the same name ("male_casualsuit01",
  but "cape (47c7634b)" beside a second cape).
- **Every path in the studio had to be typed.** The Store section (store, spec, macro, eyes), the save field, Body
  (store, body pack), Skin (landmarks, stamp), Topology (dense base), Timeline (clip, capture source, glb) and every task
  field that names a file or folder now have a folder button that opens the system's file dialog (rfd), starting
  beside the path already in the field; picking fills the field, cancelling leaves it alone. The dialog runs off the
  interface thread (on macOS on it, as the system requires). A unit test drives the plumbing with a stand-in dialog;
  the system dialog itself was not opened in testing.
- **Two browser tabs on one project shared its recovery slot.** The second tab offered the first one's live autosave,
  both autosaved over each other, and the second tab's first Save deleted the first tab's "untitled" recovery. A tab
  now holds its slot with a Web Lock: a second tab on the same project says so in the status line, neither autosaves
  into it nor offers its recovery, and leaves it alone when it saves; Save As gives it its own name and slot. When the
  first tab closes, the next one to load takes the slot and offers its work. (Two-tab CDP run: tab B refused while A
  was open, 0 autosaves after an edit; after Save As "tabb" it holds "tabb" and autosaves; A's untitled recovery is
  still listed; after A closed, B took the slot and offered the work. The deletion before the fix is read from the
  code, not reproduced.)
- **The studio's wireframe and back-face highlight stopped at the neck.** Both now draw on the body, clothes and hair
  as well (the browser already did): with the hat character, hair cards seen from behind light up (479 px in a front
  render, 0 before), X-ray shows the clothes' and body's hidden edges, points mark their vertices, and Seams outlines
  their open boundaries in orange (hems, openings, the body's hidden cut: 1,032 boundary pixels in a front render
  against 127 from the head alone; a grown-hair run's time unchanged). The render modes apply to them too (they were
  always clay): normals, matcap, triangle quality and curvature checked on the shirt. Displacement, the UV checker and
  UV seams need a reference surface or UVs, so they stay head-only.
- **Most of the browser's controls had no accessible name.** Every target, macro and face-rig slider, the joint and
  wearable pickers, the category select, the clip scrubber and the project dialog's name field now carry one (an
  `aria-label` from the visible text, or the dialog's heading): unnamed controls on the designer page with a dressed
  character, 363 of 638 -> 1: the closed dialog's field, which reads "Save project as" from its heading once open. The studio
  still has no accessibility tree (egui's AccessKit support is not built in).
- **Sliders were named by raw ids.** Both clients read a target or macro id as words: `l-cheek-trans-down` is "Left
  cheek move down", `nose-scale-horiz-incr` "Nose scale horizontal more", `cupsize` "Cup size", and a paired slider's
  ends read "less" / "more" instead of "decr" / "incr". The words are in the locale table (`target.frag.*`), the id
  stays in the hover, and the browser's search finds a slider by either ("upper lip" and "upperlip" find the same 10).
- **A task took a missing file in one step and failed in the next, in schema words.** A step that asks for a file
  (`file_open`) now refuses one that is not there when that step is submitted, naming the field ("Character spec:
  there is no file at …"), so the error shows where the path can be fixed instead of in the next step, where the field
  is hidden. Other refusals name the field too ("Glb file: needed", "…(it comes from an earlier step)") instead of
  "export.glb input does not match its schema: input.out: is required". The studio drew task errors under the step
  pane, out of sight; they now sit at the top of the Tasks window. The browser shows the message without the command
  name in front.
- **A running task step showed " / " or "Submitted" as its progress.** A step that reports a count shows "stage: 3 of
  10"; one that does not shows "Running for 12 s (this step does not report its progress)" on a moving bar, in both
  clients.
- **The studio froze for about 1.7 s the first time Body shape opened.** It read the store's wearable list on the
  interface thread. The list is now read in the background ("Reading the store's wearables…"; worn items show their
  short id until their names arrive): the click that opens the panel took 1608 and 1730 ms, now 11 to 14 ms, and the
  list lands about 0.8 s later. `wearable.list` itself no longer parses every item's mesh to read its layer (1.4 s ->
  0.9-1.05 s on the 68-item test store, same answer), and a long-running process (the server, the studio) remembers
  each item's row by its reference, so later lists are quick (one server: 792 ms, then 22 ms, then 1 ms). A wearable whose mesh is damaged is now listed; wearing
  it fails with its own error, where before the whole list failed.
- **Remove, Reset and Remove the body happened silently.** They stay one click (undo covers them), and now say so:
  "Take off cape (47c7634b). Ctrl+Z puts it back", "Reset all sliders. …", "Reset body shape. …", "Remove the body. …"
  (the studio names the key its keymap binds to undo). A Reset with nothing to reset says nothing, and in the browser
  no longer adds an empty undo step.
- **A new character opened with empty eye sockets and nothing said why.** When the catalog has an eye rig and the
  character has no eye model or eyes file, both clients show "No eyeballs in the sockets" under the head with Add eyes
  (medium brown, following the sockets; one undo step, "Eye model Medium brown") and a button that hides the note. The
  character's data is unchanged until Add eyes is pressed: a new character still has no eye model by default.
- **The studio saved specs and screenshots wherever it was started from.** A spec saved under a name or a relative
  path now goes into a `characters` folder beside the projects folder (`%APPDATA%/GazeForge/characters` on Windows),
  and a screenshot into `screenshots` there; the save field's tooltip names the folder, and the field and the status
  show the full path. Saving no longer leaves "Saving …" beside "Saved …" in the status bar. `designer.save` answers
  with `path`, its `out` made absolute, and the browser shows that.
- **The studio's Load replaced unsaved work without asking.** Store → Load, a wizard's Open in studio and Topology's
  Use now ask first when there are unsaved changes (Discard changes / Cancel; Escape cancels). A load that then failed
  used to leave the edited character on screen marked as saved, its project name gone; the project, its name and its
  unsaved mark now come back as they were (a scripted run: dirty after the failed load, true; it was false). An empty
  Store field is refused before anything happens ("Enter a store folder to load"), and a failed load says why in
  words ("… is not a store"), not as a JSON object.
- **The browser's recovery offer covered the toolbar.** "Unsaved work … was kept by autosave" floated at a fixed
  52 px from the top, over the second toolbar row when the header wraps and over the view's chips when it does not.
  It is now a strip between the header and the view that pushes the view down. Showing it also exposed an older fault:
  the view's outline boxes kept their size after the view changed size (a window resize did the same), so the page
  scrolled by the strip's height (an 855 px window held an 894 px page; now a 700 px window holds a 700 px page).
- **Saved times were raw ISO strings.** The browser shows the autosave, save, recovery and project-list times in local
  time ("10/23/2026, 4:21:54 AM", not "2026-10-23T01:20:19.992Z"); the studio, which has no time zone without a new
  dependency, shows "2026-10-23 01:25:10 UTC".
- **The browser said nothing when its server stopped.** A request that cannot reach `gazeforge serve` now puts a
  banner at the top ("Cannot reach the GazeForge server at 127.0.0.1:PORT…", with Retry); the next answer from the
  server removes it. The help now says F frames everything shown.
- **Finding a wearable meant scrolling the whole store.** "Add a wearable…" in both clients is sorted by name and has
  a filter box above it: "fedora" narrows the test store's 68 items to 2 and the list says so ("2 of 68").
- **The studio's Frame and Home framed the head only.** With a body, clothes and hair shown they now frame all of
  it, as the browser's F does.
- **Studio undo said "Panel settings" for every body, clothes and hair edit.** It now names the setting it reverts,
  such as "Undo: Body shape: Length (cm)", "Body shape: Hair under it" or "Body: Blend width (mm)".
- **Hair cards and garment fitting asked for MakeHuman's base.obj by hand.** Both tasks now find it beside the macro
  definition the client was opened with (`data/3dobjs/base.obj` of the same MakeHuman install), so "Make game hair
  cards…" opens ready to run; a base.obj you give wins, and none is proposed when the file is not there.
- **Browser usability, from a UX review.** Typing into a number box (wearable layer, openings, every grown-hair
  setting) also fired the viewport's keys: "25" became "2" and the view switched to orthographic. Number boxes, menus
  and text areas now keep their keys. A character with a body now stitches when it opens, re-stitches after each
  change by default and keeps its body, clothes and hair shown meanwhile; F frames everything shown (the head alone
  before), and the seam outline is off by default (on again when a stitch fails). Tasks: Escape leaves the field
  first, then hides the task without losing it (reopening Tasks returns to it); Run, Next and Back no longer act
  with no task chosen (a raw script error before); a finished export offers Download, the report is folded, and
  Bake physics fills in the last exported glb (the studio offers Show in folder and Copy path). Grown hair: turning
  it off and on again restores its settings; a value out of range says so instead of silently reverting; the seed
  box takes the whole seed range; number boxes are wide enough for 20000. The face rig's presets ask for the rig to
  be loaded instead of adding history entries that do nothing, and a rig that fails to load shows in red. The
  autosave note gives local time and no longer claims the last session ended without saving. In both clients a
  task's Advanced section lists only the task's own options (Export: 19, was 100 with every raw command option;
  Developer tasks shows them all), and a long option label shows its first words with the rest in its tooltip.
- **Studio usability, from a UX review.** A character with a body opened as a bare head, and every edit after a
  stitch hid the body, clothes and hair until Stitch was pressed again in the collapsed Body panel. The studio now
  stitches on load, re-stitches after each change by default, and keeps showing the last body, clothes and hair
  while it does ("Stitching the body, clothes and hair" in the viewport and status bar). The left panel scrolls
  (its sections no longer fall off the window's bottom; undo/redo, Save and the head sliders stay below). The
  "Export a character" task failed for any character with macros (it never passed the macro file); it now does,
  in both clients. Bake physics fills in the last exported glb. One failed Body shape edit no longer locks the whole
  panel under "Could not list the body sliders"; it shows "That change did not apply" with Dismiss. Turning grown
  hair off and on again brings its settings back instead of a starter groom. The Tasks list scrolls, shows each
  group once and no doubled title, and a task cannot be started while another step runs. The seam outline and the
  timing overlay are off by default (the seam still shows when a stitch fails).
- **A scaled head grew its hair by the unscaled face.** With `--body PACK --head-scale`, a groom (its landmarks,
  hairline, face keep-out and scalp), the hat test and the scalp cap read the head before scaling while the hair grew
  on the scaled one: at 1.08 the hairline stayed at 4.81 cm above the brows and the scalp counted 7,222 triangles
  instead of 5,713-ish. They now read the scaled head: 5.26 cm (asked 5.66 = 5.24 x 1.08), 5,779 triangles. Scale 1
  and spec bodies are unchanged.
- **Cloth was pinned to clothes worn over it.** Cloth pins to the opaque items worn under it, but every opaque item
  counted, whatever its layer, and an inside-out item counted by its wrong side: a suit worn over a cape pinned 341
  of the cape's 403 vertices. Only items under the cloth (lower layer, or earlier in the list at the same layer),
  faced outward, pin it now: 4 (from the body). A cape over a suit exports unchanged. Cloth bakes now also refuse to
  add more than 256 MB of morph targets, like vertex bakes (read, not reached by the test assets: their largest
  cloth bake is about 11 MB).
- **Opening the studio's Body shape panel changed a large groom seed.** Its seed box stopped at 1,000,000 while a
  groom may use any seed up to 4,294,967,295 (the browser writes those), and the box pulled a larger seed down to
  its limit and saved it as an edit: a character with seed 2,000,000 came out with 1,000,000 and new hair just by
  being looked at. The box now takes the groom's whole range.
- **Reports that described the wrong thing.** A grown groom's strands report and `.usda` header said the hair was
  traced from cards; they now say it was grown, and `--strands-count`, `--strands-seed` and `--strands-points`
  (which only shape traced hair) are refused next to a groom instead of ignored. `groom cards` counted far-level
  cards in its `highlight_cards` and `cut_at_face` beside a near-only `cards`; those are near-only now, and each far
  set gives its own. Layering's `tucked_under` counted only the first pass's vertices, facets once per pass and
  the largest single push (a cape over a suit: 31 vertices, 7.6 mm); it now counts every vertex moved, each facet
  once and the largest total move (62, 9.1 mm). The meshes are unchanged.
- **A second bake into the same file wrote invalid glTF.** Each bake adds morph targets to the cloth (and vertex-baked)
  meshes, so an earlier bake's weight tracks on them came up short, and a bake made from a baked clip copied that
  clip's weight tracks next to its own on the same nodes. Earlier tracks now gain zero weights for the new targets
  (the first animation plays exactly as before) and copied weight tracks of the nodes a bake writes are left out:
  sway baked from sway_baked, 18 glTF problems -> 0.
- **A piece of cloth with no pinned vertex fell without end.** Nothing held a separate piece of a cloth mesh with
  no pin (a loose pocket or trim): a test triangle fell 9.6 m. Such a piece now follows the skin as if pinned; the
  cloth report counts them (`unpinned_islands`, `unpinned_island_vertices`). The test cape has none and bakes
  unchanged.
- **With a ponytail, changing the length or adding a fringe reshuffled all the hair.** Where each strand joins the
  tail was drawn from the same random sequence that places the roots, and whether a strand reaches the tie depends
  on its length: 0.45 -> 0.08 m moved 2,999 of 3,000 roots (median 94 mm). The tail now has its own random draw per
  strand; the same change keeps all 3,000 roots in place. A tied groom grown from the same seed as before now has
  different roots and tail places than it did (the other styles are unchanged).
- **Choosing a comb direction cut long hair short at the back.** Without shorter sides every root counts as the
  top, so any direction but the part applied the top's taper (35 % of the length 8 cm behind the crown) to the
  whole head: long hair combed back asked 0.286 m on average instead of 0.40. The taper now shapes only a top over
  shorter sides (`sides_m` set), as on the quiff, crop, undercut and slick back; those export unchanged.
- **A groom file with an odd colour crashed the command.** A colour of six bytes holding a non-ASCII letter (such as
  `#aééb`) stopped any command that read the groom, with no JSON answer. Colours are now exactly `#rrggbb` (the `#`
  optional, once), whole-number settings (strands, points, clumps, seed) refuse fractions instead of cutting them,
  and words given as numbers or other non-text are refused instead of falling back to defaults. The undocumented
  part `center` is gone; the clients only ever wrote `centre`.
- **No more wisps across the forehead.** Framed strands rooted at the front of the part were combed down and
  sideways across the forehead, and a curl swung them further: 6 strands of the wavy framed groom (2 without the
  curl). The face's keep-out stopped at the top of the brows, so nothing guarded the forehead above them. The last
  pass now lifts any point of a strand outside the fringe that falls on the forehead (between the outer eye corners,
  ahead of the eyes, above the brows, below the hairline less its soft edge) to that height, fading out over the
  last 1 cm toward the eye corners. A forward-combed groom (the crop) is left as it was, since its hair lies on the
  forehead by design. The strands report gives `lifted_off_forehead` and examples of any strand still on the
  forehead.
- **Baked cloth stretched at a partly pinned band.** A cloth vertex partly pinned to the skin was pulled toward its
  skinned place after the cloth's edges were solved, so the edges around it ended each step stretched: a cape over a
  suit, baked with sway, reached 1.31 (LOD0) and 1.34 (LOD1) times its rest length at the collar band, and its
  partly pinned vertices ended up to 13 mm inside a body capsule. The pull now comes before the solve (fully pinned
  vertices still end exactly on the skin): 1.055 and 1.036, and 0 mm inside on those levels. LOD2, where the coarse
  cape's pinned rows sit inside a capsule at rest, now shows the conflict as stretch at its pinned edges (1.112,
  was 1.015) instead of as penetration (11.5 mm, was 13.6). The bake report gives the largest stretch for edges with
  free, partly pinned and fully pinned ends.
- **Body capsules shoved baked cloth off its rest.** A capsule pushed every cloth vertex out to its full radius, even
  a vertex that rests inside it (capsules are rounder than the body, and a cape at rest lies inside the neck and
  shoulder capsules): a cape alone stood off the back, and a cape over a suit stretched 1.11 at its pinned rows.
  Cloth now treats capsules as it already treats the body's surface (and as spring chains already treat capsules):
  a vertex is kept no deeper in a capsule than it rests. Cape over a suit: stretch at most 1.037 on every level
  (LOD2 1.112 -> 1.016); the cape alone hangs from the shoulders as fitted. The bake report gives how far any vertex
  went into a capsule beyond its rest (0 to 0.11 mm here); nothing goes into the body beyond its rest.
- **An inside-out item pulled the clothes under it out through it.** Layering took "outside" from each item's own
  winding, so under an item modelled inside out (an imported or older wearable; `wearable bind` rewinds new ones)
  everything counted as showing and was moved outward through it: a cape over a suit pushed 288 suit vertices out by
  up to 63 mm. Export and the body preview now read each worn item's facing against the body (`faces_out` in the
  wearable report) and lay it out as if it faced out; the cape now moves 89 suit vertices inward by at most 7.9 mm,
  like the same cape rewound (92, 7.6 mm). Items that already face out export unchanged.
- **Specks of an inner garment showed through the one over it.** Worn layering tucked the inner item's vertices
  behind the outer one, but two flat-faceted meshes can still cross between their vertices: a cape over a suit
  showed suit-coloured dots near its top edge (8 sample points of the suit outside the cape, up to 0.72 mm, while
  every vertex on both sides stayed clear). After the vertex tuck, the inner item's triangles whose centre or edge
  midpoints still show through are now moved behind too (up to 3 passes); the layering report counts them. The cape
  shows no dots in any baked frame; the suit sits no deeper in the body than before.
- **Game hair cards kept the old low hairline.** The scalp cap under the cards still followed the head's scalp
  skin region, which starts low on the forehead, so every hair-card wearable showed an opaque band down to near the
  brows - a fringe on a quiff, an undercut, a curtain. The cap now follows the groom's hairline like the strands'
  roots do: the old cap reached 31.6 mm below it (97 of 700 cap vertices), the new one 0.3 mm (the cap's lift).
  The cards report gives both numbers.
- **Long hair splayed out over the shoulders.** Hair landing on the shoulders slid outward along them toward the
  arms (12.7 % of a long groom's strands ended pointing outward, reaching 27 cm from the centre line). Hair that lands
  on the body below the ears now turns down and back over the shoulders (front, for hair rooted ahead of the ears):
  2.6 % point outward, reaching 23 cm; wavy 10.9 % -> 3.4-4.7 %, curly 5.1 % -> 1.2 %. The groom report counts the
  strands below the throat line and their tips pointing outward. Landing also keeps only 30 % of the strand's
  sideways motion, so hair falls off the shoulder's crest instead of sliding along it to the shoulder's tip: of the
  strands below the throat, long 2.6 % -> 1.0 % end pointing outward, wavy 3.45 % -> 2.45 %, curly 0.9 % -> 0.37 %,
  and the hair in front hangs down the chest rather than out over the shoulders.
- **The quiff was a low dome.** Measured down the middle of the head, its hair stood 0.9 cm above the scalp at the
  front and 1.5 cm at most further back. The top hair combed up now rises from its root (straight up, a little
  forward at the hairline), turns over within 3 cm and runs back: a pompadour, highest at the front (lift x 5 cm at
  the hairline, 30 % of that at the back of the quiff; 4.4 cm on the test head) and tapering back. The groom
  report gives the hair's height over the front 2 cm of the scalp and the rise asked.
- **Ponytail hair lifted off the side of the head.** Hair combed to a ponytail's tie drifted outward on its way
  over the head (up to 15 mm, and on one seed 20 mm, more than 4 cm from the tie), and a few strands from low at the
  temples showed as thin arcs beside the head. It is now pulled back to 4 mm from the head until it is within 4 cm
  of the tie (3 mm being the skin clearance), and again after clumping and settling, which had lifted it to 6.4 mm.
  The groom report gives the largest such gap on the strands as written (4.0 mm; it was read before clumping until
  2026-10-23, so an earlier 4.1 mm described the hair before its last shaping).
- **The groom's hairline sat low on the forehead.** It sat 1.8 cm above the brows, so from the front the edge of the
  roots read as a short fringe on every style. It now sits a forehead above the brows: by default as high as the
  brows-to-nose height (the classic thirds of the face; 4.8 cm on the test character, asked 5.2 cm), highest at the
  centre, 1 cm lower at the temples, and down to the ears beside them. The roots thin out over 1.2 cm instead of
  stopping at a hard line. A new groom field `forehead` (a Forehead row in both panels) sets the height as a share of
  that third; styles keep it. The groom report gives the measured and asked hairline and the face's thirds.
- **Curly grooms were zigzags and drew lines across the throat.** A strand kept its 16 points however many turns it
  curled, about two points per turn; a curled groom now gets at least 8 points per turn (up to 256). Points the face's
  keep-out moved went to the side each point happened to be on, so a curl crossing the middle jumped to the other
  edge and back; a strand now keeps its root's side. Clumping pulled strands toward locks across the part, which
  blended into hair in front of the throat; a strand now clumps only with locks on its own side.
- **Tasks that work on "the character it was modelled on" did not get it.** In both clients, Fit a garment, Attach a
  solid and Make game hair cards opened with that field empty: the current character is saved into the run after
  the run starts, and the run's values never named it. A run now takes the character saved into it as its spec,
  and the macro definition the client was opened with fills the Macro field. Integer fields from the studio's task
  form reached commands as `300.0` and were refused; they are sent as integers, and the API turns a whole-valued
  number into a whole number for every command.
- **Grown hair no longer strays under the jaw or flies off the temples.** Strands from the front of the part slid
  down the forehead, along the face's edge and in under the chin (49 of 20000). While on the head, each step now turns
  halfway back to the comb direction, so front hair keeps combing back; a strand that leaves the head above the ears
  falls instead of arcing out; the face's keep-out reaches 5 cm under the chin. The last pass alternates keeping a
  point off the face and outside the skin (up to three rounds), so neither undoes the other. The groom report counts
  flyaways and strands under the jaw (now 0 and 0, centre and side part) and `groom_gate` checks both.
- **Grown hair rose off the head with a side part.** A strand kept the direction it left the scalp with, so hair
  combed across the top from a side part flew off in a straight line into a tall overhang. Strands now follow the
  head's curve while above the ears and near it, then fall; every groom lies flatter and reads as combed hair. The
  keep-outside push now repeats from the new nearest point (up to 4 times), so no point is left inside a crease such
  as behind the ears (a left-part groom had one 6 mm inside); the groom report says where its deepest point is.
- **The far-level scalp cap painted hair colour onto the face.** Hair passing within its reach of the cheeks, eyes or
  forehead (cards hanging beside the face) painted them on LOD4 and LOD5. The cap now never paints the face's skin
  regions (lips, mouth, nose, cheeks, forehead, chin, upper lip, under the eyes, eyelids, T-zone; weighted, so the
  hairline is soft) and still paints the scalp, ears, neck and jaw where hair lies on them. Exports of characters
  wearing alpha-card hair change on LOD4-5. The sidecar lists the regions (`scalp_cap.never_on`).
- A VRM test held an approximate 1/sqrt(2) that the workspace clippy refuses, so the test target did not compile under
  clippy; one cloth test looped by index.
- **Clothes showed through a coarse outer item, and cloth over clothes was barely pinned.** Layering pushed the
  outer item's vertices out, but a fine suit still rose between a coarse cape's vertices (visible patches at its top
  edge). Now the vertices of the items underneath that lie in front of one of the outer item's triangles (or behind
  it by less than the clearance, within 2 cm) are moved behind it; the layering report gives `tucked_under`. Cape
  over suit: 32 suit vertices tucked (at most 7 mm), the patches gone but for a dot or two at the edge; a five-item
  outfit changes by 12 pixels. A cloth item's `_PIN` is now measured against the body and the opaque items worn
  under it, not the body alone: a cape over a suit had no vertex pinned at 0.99 (its collar rests on the suit, off
  the skin), now 28.
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
