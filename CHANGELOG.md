# Changelog

All notable changes to GazeForge. The project follows [Semantic Versioning](https://semver.org); before 1.0 any minor
version may change interfaces. Every milestone ends on a measured gate, and a gate that is still open is listed here as
open, with its last measurement, rather than left out.

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
