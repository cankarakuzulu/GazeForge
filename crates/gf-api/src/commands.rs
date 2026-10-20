use std::collections::BTreeMap;
use std::path::Path;

use gf_core::catalog::{CatalogManifest, GameManifest, PackManifest};
use gf_core::character::{FORMAT, Spec, SpecBase};
use gf_core::measure;
use gf_core::placement::{self as gp, PlacementVersion};
use gf_io::character as ch;
use gf_io::dense;
use gf_io::store::Store;
use serde_json::{Map, Value, json};

use crate::error::{self, ApiError};
use crate::{CommandDef, Ctx, Kind, anim, body, designer, eyes, grid, jobs, photo, photo_fit, photo_gate, project, registry};

pub static REGISTRY: &[CommandDef] = &[
    CommandDef {
        name: "commands.list",
        kind: Kind::Query,
        summary: "Every command with its kind, input schema, output shape and limits, and the error codes.",
        limits: "Describes this build only; the same list is GET /v1/commands over HTTP and tools/list over MCP.",
        input: empty_input,
        output: commands_output,
        run: commands_list,
    },
    CommandDef {
        name: "store.list",
        kind: Kind::Query,
        summary: "The dense packs, target catalogs and game bases in a store, with the references a spec names.",
        limits: "Reads manifests only; blob integrity is checked when a command reads the blob, not here.",
        input: store_input,
        output: store_output,
        run: store_list,
    },
    CommandDef {
        name: "catalog.list",
        kind: Kind::Query,
        summary: "Search a catalog's targets by id text and category; each target is a weight in [0, 1] a spec can set.",
        limits: "Ids and the per-target maximum displacement only; what a target looks like is not described here.",
        input: catalog_input,
        output: catalog_output,
        run: catalog_list,
    },
    CommandDef {
        name: "character.create",
        kind: Kind::Mutation,
        summary: "Write a character spec (target weights over a pack and catalog) to a file and return its revision.",
        limits: "Weights only: macro sliders and a captured residual are made with the argv command `character create`.",
        input: create_input,
        output: create_output,
        run: character_create,
    },
    CommandDef {
        name: "character.validate",
        kind: Kind::Query,
        summary: "Check a spec file against its store: format, references, target ids and weight ranges.",
        limits: "A spec with macro sliders needs the macro definition, which this command does not take.",
        input: spec_input,
        output: validate_output,
        run: character_validate,
    },
    CommandDef {
        name: "character.evaluate",
        kind: Kind::Query,
        summary: "Evaluate a spec to dense positions (metres) and write them as an .npy file.",
        limits: "Dense level only; the game mesh is made by export-obj / export-glb. Macro specs are not supported here.",
        input: evaluate_input,
        output: evaluate_output,
        run: character_evaluate,
    },
    CommandDef {
        name: "character.measure",
        kind: Kind::Query,
        summary: "Measure a character in millimetres between landmark vertices (nose width, nose length, face width, eye spacing), with the base's value and the difference.",
        limits: "Landmarks are dense vertex indices chosen once on the base by rule: a target that slides the widest point along the surface is still measured at the base's vertex. Straight-line distances, not surface paths.",
        input: measure_input,
        output: measure_output,
        run: character_measure,
    },
    CommandDef {
        name: "designer.sliders",
        kind: Kind::Query,
        summary: "The designer's slider set: the head targets (10 categories) with their categories and sides, the macro sliders (head-only, option E) and the starting character.",
        limits: "Head targets only (cheek, chin, ears, eyebrows, eyes, forehead, head, mouth, neck, nose); macros are available only with a pack macro_base and a macro definition file.",
        input: designer::sliders_input,
        output: designer::sliders_output,
        run: designer::sliders,
    },
    CommandDef {
        name: "designer.pack",
        kind: Kind::Query,
        summary: "Build the preview pack for a set of targets: game triangles and each target's exact delta at the game vertices (i16 with a per-target scale), stored as a content-addressed store blob.",
        limits: "The delta is the dense target at each game vertex's dense index; the placed (settled) mesh can move differently under f3, so a preview made from it is approximate.",
        input: designer::pack_input,
        output: designer::pack_output,
        run: designer::pack,
    },
    CommandDef {
        name: "designer.settle",
        kind: Kind::Query,
        summary: "Settle a character: weights (and macros) -> dense -> placement -> game mesh; returns the game positions (and optionally the placed UVs) as base64 f32.",
        limits: "The served process keeps one pack, catalog and game base in memory between calls; a CLI call loads them every time. Placements: f3, f4, subset, optimal with their default settings.",
        input: designer::settle_input,
        output: designer::settle_output,
        run: designer::settle,
    },
    CommandDef {
        name: "designer.macro_delta",
        kind: Kind::Query,
        summary: "The game-level change per unit of one macro slider at the current macro values, one-sided up and down, for a linear preview while it is dragged.",
        limits: "Macros are piecewise linear in their sliders: the slope holds only until the next key value, so the preview drifts from the settled mesh past it.",
        input: designer::macro_delta_input,
        output: designer::macro_delta_output,
        run: designer::macro_delta,
    },
    CommandDef {
        name: "designer.scene",
        kind: Kind::Query,
        summary: "The viewer's scene extras for a game base: its coarser levels of detail as triangles over the finest level's vertices, and the eyes of an optional eyes file (centre, radius, side, gaze and mesh).",
        limits: "A level with a vertex outside the finest level drops that triangle and counts it; eyes are the file's spheres, not a placed eye mesh.",
        input: designer::scene_input,
        output: designer::scene_output,
        run: designer::scene,
    },
    CommandDef {
        name: "eyes.generate",
        kind: Kind::Mutation,
        summary: "Write an eyes file for a catalog's base head: a procedural eyeball (sclera, a cornea cap at the limbus, a recessed iris and a pupil, as separate parts with UVs) centred on each of MakeHuman's helper eyeballs as the catalog's eye rig places them, 12 mm in radius by default.",
        limits: "Overwrites the file at out. The eyes are placed for the base head; a character's eyeballs field moves them for the character. helper_radius uses MakeHuman's helper radius (about 15.75 mm).",
        input: eyes::generate_input,
        output: eyes::generate_output,
        run: eyes::generate,
    },
    CommandDef {
        name: "grid.chart",
        kind: Kind::Query,
        summary: "The face chart of a game base: every game vertex's chart coordinate (angle about the head's vertical axis, millimetres below the top of the head), its cell (column letter, row number, keypad subcell), its region, one label anchor per cell, and the chart's sanity figures (cells over the face front, the largest and smallest cell area, folded triangles).",
        limits: "Computed once from the base dense positions and carried by vertex identity, so a cell names the same vertices on every character; it is cached per base and region targets in the served process. Where the surface turns away from the axis (behind the ears, under the chin) one cell covers two layers of surface.",
        input: grid::chart_input,
        output: grid::chart_output,
        run: grid::chart,
    },
    CommandDef {
        name: "grid.locate",
        kind: Kind::Query,
        summary: "The cell, region and chart coordinate of dense vertices, game vertices, game triangles or surface points (triangle + barycentrics), and the cells added beside the vertices and triangles of a gate report.",
        limits: "Regions exist for game vertices only (a dense vertex the game base does not keep has a cell but no region). A report's fields are recognised by name: tri/triangle, vertex/game_vertex, dense_vertex, worst_visible, lost_vertices, added_vertices.",
        input: grid::locate_input,
        output: grid::locate_output,
        run: grid::locate,
    },
    CommandDef {
        name: "grid.resolve",
        kind: Kind::Query,
        summary: "The vertices an address names: a cell or subcell gives its game vertices, the number of dense vertices in it, their base centroid and the game vertex nearest it; \"dense N\", \"game vN\" or a GF@ address line gives that vertex.",
        limits: "Positions are the base's; the designer frames the same vertices on the current character. A whole address resolves to its vertex, not to its surface point.",
        input: grid::resolve_input,
        output: grid::resolve_output,
        run: grid::resolve,
    },
    CommandDef {
        name: "rig.face",
        kind: Kind::Query,
        summary: "The face rig for a character: facial joints placed from the base by index, skin weights computed on the base, and per expression shape the fitted joint poses with their reproduction error; with the settled game positions, the weights and the shapes' blend deltas for a preview.",
        limits: "The weights are cached per pack, game base and shape list in the served process; a CLI call computes them every time (seconds). The blend deltas are the targets' dense deltas at the game vertices, not the export's placed morphs.",
        input: designer::rig_face_input,
        output: designer::rig_face_output,
        run: designer::rig_face,
    },
    CommandDef {
        name: "designer.save",
        kind: Kind::Mutation,
        summary: "Write the designer's character (weights, macros, residual) as a spec file after validating it against the store.",
        limits: "Overwrites the file at out.",
        input: designer::save_input,
        output: designer::save_output,
        run: designer::save,
    },
    CommandDef {
        name: "jobs.submit",
        kind: Kind::Mutation,
        summary: "Run a command as a job in a supervised worker process; returns the job record at once.",
        limits: "Jobs live in the jobs directory of the process that serves the call (GAZEFORGE_JOBS, else the temp directory).",
        input: submit_input,
        output: job_output,
        run: jobs::submit,
    },
    CommandDef {
        name: "jobs.status",
        kind: Kind::Query,
        summary: "A job's record: state (queued, running, done, failed, cancelled), result when done, worker processes left.",
        limits: "wait_s blocks this call up to that many seconds for the job to finish.",
        input: status_input,
        output: job_output,
        run: jobs::status,
    },
    CommandDef {
        name: "jobs.cancel",
        kind: Kind::Mutation,
        summary: "Cancel a job: its worker and every process the worker started are killed together, then counted.",
        limits: "worker_processes_left is the job object's own count on Windows; elsewhere the process group is killed and the count is null.",
        input: status_input,
        output: job_output,
        run: jobs::cancel,
    },
    CommandDef {
        name: "debug.slow",
        kind: Kind::Job,
        summary: "A deliberately slow job: sleeps, with child processes that also sleep. It exists to prove cancel.",
        limits: "Job only (jobs.submit); it does no work.",
        input: slow_input,
        output: slow_output,
        run: jobs::slow,
    },
    CommandDef {
        name: "photo.ingest",
        kind: Kind::Query,
        summary: "Stage 0 of image to character: read a front and a profile photo into a run directory and check facing (the front looks at the camera, the profile shows the declared side); the front-vs-profile proportion agreement is reported, and refuses only under max_ratio_gap.",
        limits: "A failed check is an error (gate_failed) and the run stops; nothing is averaged. A mirrored front photo of a symmetric face cannot be detected. The reference-photo gate (photo.check) runs first.",
        input: photo_gate::ingest_input,
        output: photo::generic_output,
        run: photo_gate::ingest,
    },
    CommandDef {
        name: "photo.check",
        kind: Kind::Query,
        summary: "Reference-photo gate: measures head yaw, pitch and roll in a front and/or profile photo and checks the image (one face, face size, sharpness, clipped highlights and shadows, eyes open and mouth closed, plain background, declared profile side); a report per photo with each value, its limit and a reason on failure.",
        limits: "A failed check is a report (pass false), not an error; photo.ingest and character.from_photos turn it into gate_failed. Limits are calibrated on renders of known rotation (the report's calibration block gives each estimator's measured error and says where an axis misses the 3 degree target); the profile's yaw and roll come from this person's fitted head and need the front photo, and their sign is not observable; profile expression is not measured.",
        input: photo_gate::check_input,
        output: photo_gate::check_output,
        run: photo_gate::check,
    },
    CommandDef {
        name: "photo.landmarks",
        kind: Kind::Job,
        summary: "Stage 1: 478 face landmarks on the front photo, the silhouette and profile line of the profile photo, and an overlay image of each for review.",
        limits: "Read the overlays before trusting any number: lips and chin are where trackers fail. Needs the Python worker and the MediaPipe landmarker model.",
        input: photo::landmarks_input,
        output: photo::generic_output,
        run: photo::landmarks,
    },
    CommandDef {
        name: "mesh.multiview",
        kind: Kind::Job,
        summary: "Stage 2: a 3D mesh from the front and profile photos through an external image-to-mesh server, then its outer shell (visible surface only) and 3D landmarks on it.",
        limits: "The server and its graph are external (server, graph or GAZEFORGE_MESH_SERVER / GAZEFORGE_MESH_GRAPH); the mesh has no metric scale. Pass glb to reuse a mesh.",
        input: photo::mesh_input,
        output: photo::generic_output,
        run: photo::mesh,
    },
    CommandDef {
        name: "fit.register",
        kind: Kind::Job,
        summary: "Stage 3: similarity transform of the outer shell onto the dense base from matched landmarks, refined by trimmed ICP on the face; landmark residual in mm.",
        limits: "Scale comes from the base's landmark spans: photos carry no absolute size.",
        input: photo::register_input,
        output: photo::generic_output,
        run: photo::register,
    },
    CommandDef {
        name: "fit.targets",
        kind: Kind::Job,
        summary: "Stage 4: bounded least squares ([0, 1]) over the head targets on the dense topology, with terms for distance to the registered shell, front-photo landmark reprojection and the profile silhouette; per-region residual in mm.",
        limits: "Correspondence to the shell is by nearest point, re-found each round; hair-covered skull and the eyelids are not trusted.",
        input: photo::targets_input,
        output: photo::generic_output,
        run: photo::targets,
    },
    CommandDef {
        name: "fit.residual",
        kind: Kind::Job,
        summary: "Stage 5: a bounded residual toward the shell (trust radius, smoothing, eye hold) checked by the shell gates (folds, inner surfaces, roughness).",
        limits: "The residual is backed off (strength 1, 0.7, 0.5, 0.35, 0.25) until the gates pass; if none passes it is refused and nothing is written. plant adds a deliberate defect (never scaled) to prove the refusal.",
        input: photo::residual_input,
        output: photo::generic_output,
        run: photo::residual,
    },
    CommandDef {
        name: "texture.project",
        kind: Kind::Job,
        summary: "Stage 7: project the photos onto the character's game UVs through the fitted cameras, blended by facing and visibility; camera rms in px.",
        limits: "Orthographic cameras; no delighting unless albedo images are given.",
        input: photo::texture_input,
        output: photo::generic_output,
        run: photo::texture,
    },
    CommandDef {
        name: "review.sheet",
        kind: Kind::Job,
        summary: "Stage 8: a review sheet with the photos beside renders of the result (textured and clay) at the fitted front and profile cameras.",
        limits: "A software render for comparison, not the engine's lookdev.",
        input: photo::sheet_input,
        output: photo::generic_output,
        run: photo::sheet,
    },
    CommandDef {
        name: "photo.views",
        kind: Kind::Job,
        summary: "Direct fit, stage 1: for any number of photos (at least one front and one profile), face landmarks, the subject's outline, the profile keys of a side view, and the quality checks of the reference-photo gate for every extra view.",
        limits: "The outline assumes a plain background and includes hair and clothing; an extra view needs a detectable face (its pose is solved later, not gated).",
        input: photo_fit::views_input,
        output: photo::generic_output,
        run: photo_fit::views,
    },
    CommandDef {
        name: "fit.photos",
        kind: Kind::Job,
        summary: "Direct fit: bounded least squares ([0, 1]) over the head targets to every photo at once - landmark reprojection and outline distance through a solved perspective camera per photo (pose re-solved each round), with stay-at-base, symmetry, ridge and L1 terms; per-photo reprojection in px and mm, the shell gates on the result, and leave-one-out prediction of a named photo.",
        limits: "Focal length is assumed (70 mm full-frame equivalent unless focal_px is given); scale and placement are not observable from photos. The outline of hair-covered skull and of the neck is not fitted by default. Profile cameras start from the profile keys and the declared side.",
        input: photo_fit::input,
        output: photo::generic_output,
        run: photo_fit::fit_photos,
    },
    CommandDef {
        name: "review.views",
        kind: Kind::Job,
        summary: "Review sheet of a direct fit: one row per photo - the photo, a clay render at its solved camera, and both at 50 % with the photo outline (red) and the fit's outline (green).",
        limits: "A software render for comparison, not the engine's lookdev.",
        input: photo_fit::sheet_input,
        output: photo::generic_output,
        run: photo_fit::sheet,
    },
    CommandDef {
        name: "character.from_photos",
        kind: Kind::Job,
        summary: "Image to character, stages 0 to 8 end to end: ingest and facing, landmarks, mesh, register, targets, residual, spec and glb export, texture, review sheet; logs per-stage times.",
        limits: "Stops at the first failing stage with that stage's error and the times so far.",
        input: photo_gate::from_photos_input,
        output: photo::generic_output,
        run: photo_gate::from_photos,
    },
    CommandDef {
        name: "body.import",
        kind: Kind::Mutation,
        summary: "Build a body pack from a body mesh: the MakeHuman base mesh joined to the head by topology (makehuman), or any body mesh joined at its neck loop or a cut plane (mesh); with LODs and UVs.",
        limits: "makehuman mode needs the MakeHuman base topology and the dense base it was subdivided from; mesh mode aligns by the neck ring's centre and size only (no rotation) and keeps the largest connected part.",
        input: body::import_input,
        output: body::import_output,
        run: body::import,
    },
    CommandDef {
        name: "character.stitch",
        kind: Kind::Query,
        summary: "Join a character's head to a body pack at the neck: the seam vertices are shared, the body is bent onto the head over a blend band, and the seam gates run (watertight, cracks, normal continuity, folds, interpenetration).",
        limits: "LOD0 and the unplaced head (dense positions at the kept vertices); export-glb --body stitches every LOD of the placed head.",
        input: body::stitch_input,
        output: body::stitch_output,
        run: body::stitch,
    },
    CommandDef {
        name: "project.new",
        kind: Kind::Mutation,
        summary: "Start a project file <root>/<name>.gfproj.json with one character (a spec file, a spec object, or the store's base with no weights).",
        limits: "Refuses an existing name unless overwrite; one project holds one base (pack, catalog, game base), referenced by sha256.",
        input: project::new_input,
        output: project::save_output,
        run: project::new,
    },
    CommandDef {
        name: "project.save",
        kind: Kind::Mutation,
        summary: "Save a project: its characters (specs, each with a revision history), panel states, viewer state, options and notes; recovery: true writes the autosave slot instead.",
        limits: "Written to a temporary file and renamed, so a reader never sees a partial file and the last of concurrent saves wins whole; the base is referenced by sha256, never copied.",
        input: project::save_input,
        output: project::save_output,
        run: project::save,
    },
    CommandDef {
        name: "project.load",
        kind: Kind::Query,
        summary: "Read a project (or its autosave slot, or import a portable .gfproj.zip): find every resource (store, macro, eyes, face_morphs, panel resources) by option, search folder, projects folder or hint and check its identity, then check the base manifests by hash, base blob sizes against the saved fingerprint, character blobs by hash, character revisions.",
        limits: "Refuses with every unresolved resource listed, and a project whose base is missing or changed; verify: full hashes every base blob (slow on a large store). With zip: hints are not used; entry names that are absolute, hold .., a drive letter or a backslash, repeat, or are links refuse the file; decompressed bytes are counted while streaming against max_entry_bytes and max_total_bytes; the character blobs go into the store and the project file into root.",
        input: project::load_input,
        output: project::load_output,
        run: project::load,
    },
    CommandDef {
        name: "project.list",
        kind: Kind::Query,
        summary: "The projects in a projects folder, newest first, and the autosave slots with whether each is newer than its saved project.",
        limits: "Reads the project files only; no store is checked here.",
        input: project::list_input,
        output: project::list_output,
        run: project::list,
    },
    CommandDef {
        name: "project.discard",
        kind: Kind::Mutation,
        summary: "Delete a project's autosave slot (after its recovery was declined).",
        limits: "The saved project file is never touched.",
        input: project::discard_input,
        output: project::discard_output,
        run: project::discard,
    },
    CommandDef {
        name: "project.export",
        kind: Kind::Query,
        summary: "Write a portable .gfproj.zip: the project file without local paths (each resource as role, identity and file name), an index and the character-specific blobs (residuals), not the shared pack, catalog or game base.",
        limits: "Refused when any text in the project still looks like a local path, a user or home folder, the projects or temp folder or the host name; project.load with zip refuses it on a store without the same base.",
        input: project::export_input,
        output: project::export_output,
        run: project::export,
    },
    CommandDef {
        name: "animation.from_video",
        kind: Kind::Job,
        summary: "Facial animation from a video or an image sequence: per frame the face landmarker's 51 ARKit-named blendshape scores and head pose, a per-frame gate (face found, one face, head pose, face size, landmarks in frame) with rejected frames filled by interpolation, neutral and optional range-of-motion calibration (gains, leaks, head bias, a reusable profile), crosstalk couplings, an opt-in lid guard, One Euro smoothing, retarget by name to a character's morph order; writes a clip, a report and, with glb, a copy of the character glb with a glTF animation (morph weights, head joint rotation, optional blended joint tracks).",
        limits: "Job only (jobs.submit). The landmarker model is supplied by the user (landmarker); it reads one face per frame and its scores are relative estimates, not measurements. Head pose is relative to the neutral segment; translation is not exported. Joint tracks are an approximation (each shape's fitted pose blended by its weight).",
        input: anim::video_input,
        output: anim::output,
        run: anim::from_video,
    },
    CommandDef {
        name: "animation.from_csv",
        kind: Kind::Mutation,
        summary: "EXPERIMENTAL: facial animation from a CSV of ARKit blendshape columns (as a Live Link Face recording is believed to have), read by header names; the same calibration, smoothing, retarget and outputs as animation.from_video.",
        limits: "The recording app's CSV columns, timecode format and head rotation units are unconfirmed; head rotation is ignored unless head_units is given. tongueOut and eye rotation columns are not used.",
        input: anim::csv_input,
        output: anim::output,
        run: anim::from_csv,
    },
];

pub fn obj(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

pub fn shape(properties: Value) -> Value {
    json!({"type": "object", "properties": properties})
}

fn ref_text(what: &str) -> Value {
    json!({"type": "string", "minLength": 1, "description": format!("{what} reference, sha256:<hex>, as store.list prints it")})
}

fn store_prop() -> Value {
    json!({"type": "string", "minLength": 1, "description": "path of the store directory"})
}

fn empty_input() -> Value {
    obj(json!({}), &[])
}

fn commands_output() -> Value {
    shape(json!({"commands": {"type": "array"}, "errors": {"type": "array", "description": "code, CLI exit code, HTTP status, retryable"}}))
}

fn store_input() -> Value {
    obj(json!({"store": store_prop()}), &["store"])
}

fn store_output() -> Value {
    shape(json!({"packs": {"type": "array"}, "catalogs": {"type": "array"}, "game_bases": {"type": "array"}}))
}

fn catalog_input() -> Value {
    obj(
        json!({"store": store_prop(), "catalog": ref_text("catalog"),
               "contains": {"type": "string", "description": "keep ids containing this text (case-insensitive)"},
               "category": {"type": "string", "description": "keep one category, e.g. nose"},
               "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "description": "default 100"}}),
        &["store", "catalog"],
    )
}

fn catalog_output() -> Value {
    shape(json!({"total": {"type": "integer"}, "matched": {"type": "integer"}, "returned": {"type": "integer"},
                 "targets": {"type": "array", "description": "id, category, slider, opposite, mirror, side, max_mm, moved"}}))
}

fn weights_prop() -> Value {
    json!({"type": "object", "additionalProperties": {"type": "number", "minimum": 0, "maximum": 1},
           "description": "catalog target id -> weight in [0, 1]; ids from catalog.list"})
}

fn create_input() -> Value {
    obj(
        json!({"store": store_prop(), "pack": ref_text("dense pack"), "catalog": ref_text("catalog"),
               "out": {"type": "string", "minLength": 1, "description": "path of the spec file to write (.gfchar.json)"},
               "game": ref_text("game base"), "placement": {"type": "string", "description": format!("placement name/version, default {}", gp::DEFAULT)},
               "name": {"type": "string", "minLength": 1}, "weights": weights_prop()}),
        &["store", "pack", "catalog", "out"],
    )
}

fn create_output() -> Value {
    shape(json!({"out": {"type": "string"}, "revision": {"type": "string"}, "weights": {"type": "integer"}, "spec": {"type": "object"}}))
}

fn spec_input() -> Value {
    obj(json!({"store": store_prop(), "spec": {"type": "string", "minLength": 1, "description": "path of a spec file"}}), &["store", "spec"])
}

fn validate_output() -> Value {
    shape(json!({"revision": {"type": "string"}, "canonical": {"type": "boolean"}}))
}

fn evaluate_input() -> Value {
    obj(
        json!({"store": store_prop(), "spec": {"type": "string", "minLength": 1},
               "out": {"type": "string", "minLength": 1, "description": "path of the .npy file to write"},
               "precision": {"enum": ["f32", "f64"], "description": "default f32"}}),
        &["store", "spec", "out"],
    )
}

fn evaluate_output() -> Value {
    shape(json!({"out": {"type": "string"}, "revision": {"type": "string"}, "vertices": {"type": "integer"},
                 "precision": {"type": "string"}, "report": {"type": "object"}}))
}

fn landmark_text() -> String {
    let landmarks: Vec<String> = measure::LANDMARKS.iter().map(|l| format!("{} = vertex {} ({})", l.name, l.vertex, l.rule)).collect();
    let measures: Vec<String> = measure::MEASURES.iter().map(|m| format!("{} = {} to {} ({})", m.name, m.from, m.to, m.what)).collect();
    format!(
        "Default measurements on the {}-vertex base, straight-line mm: {}. Landmarks (dense vertex indices, chosen once on the base positions; +x is the character's left, up +z, forward -y): {}.",
        measure::BASE_VERTICES,
        measures.join("; "),
        landmarks.join("; ")
    )
}

fn measure_input() -> Value {
    let mut schema = obj(
        json!({"store": store_prop(), "spec": {"type": "string", "minLength": 1, "description": "path of a spec file"},
               "pairs": {"type": "object", "additionalProperties": {"type": "array", "items": {"type": "integer", "minimum": 0}, "minItems": 2, "maxItems": 2},
                         "description": "optional: name -> [vertex, vertex] measured instead of the default set"}}),
        &["store", "spec"],
    );
    schema["description"] = Value::String(landmark_text());
    schema
}

fn measure_output() -> Value {
    shape(json!({"revision": {"type": "string"}, "units": {"type": "string"},
                 "measurements": {"type": "object", "description": "name -> {mm, base_mm, delta_mm, from, to, what}"},
                 "landmarks": {"type": "array"}}))
}

fn submit_input() -> Value {
    obj(
        json!({"command": {"type": "string", "minLength": 1, "description": "a command name from commands.list"},
               "input": {"type": "object", "description": "that command's input"}}),
        &["command"],
    )
}

fn status_input() -> Value {
    obj(json!({"id": {"type": "string", "minLength": 1}, "wait_s": {"type": "number", "minimum": 0, "maximum": 600}}), &["id"])
}

fn job_output() -> Value {
    shape(json!({"id": {"type": "string"}, "command": {"type": "string"}, "state": {"type": "string"},
                 "result": {"type": "object"}, "worker_processes_left": {"type": ["integer", "null"]}}))
}

fn slow_input() -> Value {
    obj(
        json!({"seconds": {"type": "number", "minimum": 0, "maximum": 600},
               "children": {"type": "integer", "minimum": 0, "maximum": 8, "description": "child processes that sleep as long"}}),
        &["seconds"],
    )
}

fn slow_output() -> Value {
    shape(json!({"slept_s": {"type": "number"}, "children": {"type": "integer"}}))
}

pub fn text<'a>(input: &'a Value, key: &str) -> &'a str {
    input.get(key).and_then(Value::as_str).unwrap_or("")
}

fn commands_list(_: &Ctx, _: &Value) -> Result<Value, ApiError> {
    Ok(json!({"commands": registry().iter().map(CommandDef::describe).collect::<Vec<_>>(), "errors": error::code_table()}))
}

fn open_store(input: &Value) -> Result<Store, ApiError> {
    let root = text(input, "store");
    let path = Path::new(root);
    if !path.is_dir() {
        return Err(ApiError::not_found(format!("store {root} is not a directory")).details(json!({"store": root})));
    }
    if !path.join("packs").is_dir() {
        return Err(ApiError::invalid(format!("{root} is not a store: it has no packs directory")).details(json!({"store": root})));
    }
    Store::open(path).map_err(ApiError::internal)
}

fn manifests<T: serde::de::DeserializeOwned>(store: &Store, kind: &str) -> Result<Vec<(String, T)>, ApiError> {
    let dir = store.root.join(kind);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?
        .filter_map(|e| e.ok().and_then(|e| e.file_name().into_string().ok()))
        .filter_map(|n| n.strip_suffix(".json").map(str::to_string))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|hex| {
            let reference = format!("sha256:{hex}");
            store.get_manifest::<T>(kind, &reference).map(|m| (reference, m)).map_err(ApiError::internal)
        })
        .collect()
}

fn store_list(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = open_store(input)?;
    let packs: Vec<Value> = manifests::<PackManifest>(&store, "packs")?
        .into_iter()
        .map(|(r, m)| json!({"pack": r, "vertices": m.vertices, "faces": m.faces, "macro_base": m.macro_base.is_some()}))
        .collect();
    let catalogs: Vec<Value> = manifests::<CatalogManifest>(&store, "catalogs")?
        .into_iter()
        .map(|(r, m)| json!({"catalog": r, "pack": m.pack, "targets": m.targets.len()}))
        .collect();
    let games: Vec<Value> = manifests::<GameManifest>(&store, "game")?
        .into_iter()
        .map(|(r, m)| json!({"game": r, "pack": m.pack, "vertices": m.vertices, "triangles": m.triangles, "lods": m.lods.len()}))
        .collect();
    Ok(json!({"packs": packs, "catalogs": catalogs, "game_bases": games}))
}

fn load_catalog(store: &Store, reference: &str) -> Result<CatalogManifest, ApiError> {
    let hex = reference.strip_prefix("sha256:").unwrap_or(reference);
    if !store.root.join("catalogs").join(format!("{hex}.json")).is_file() {
        return Err(ApiError::not_found(format!("no catalog {reference} in the store")).hint("store.list names the catalogs"));
    }
    store.load_catalog(reference).map_err(ApiError::internal)
}

fn catalog_list(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = open_store(input)?;
    let catalog = load_catalog(&store, text(input, "catalog"))?;
    let contains = input.get("contains").and_then(Value::as_str).map(str::to_lowercase);
    let category = input.get("category").and_then(Value::as_str);
    let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
    let matched: Vec<(&String, &gf_core::catalog::CatalogEntry)> = catalog
        .targets
        .iter()
        .filter(|(id, e)| contains.as_ref().is_none_or(|c| id.to_lowercase().contains(c)) && category.is_none_or(|c| e.category == c))
        .collect();
    let targets: Vec<Value> = matched
        .iter()
        .take(limit)
        .map(|(id, e)| {
            json!({"id": id, "category": e.category, "slider": e.slider, "opposite": e.opposite, "mirror": e.mirror,
                   "side": e.side, "max_mm": measure::round_mm(e.max_mm), "moved": e.moved})
        })
        .collect();
    Ok(json!({"catalog": text(input, "catalog"), "total": catalog.targets.len(), "matched": matched.len(),
              "returned": targets.len(), "targets": targets}))
}

fn number_map(value: Option<&Value>) -> BTreeMap<String, f64> {
    value.and_then(Value::as_object).map(|m| m.iter().filter_map(|(k, v)| v.as_f64().map(|x| (k.clone(), x))).collect()).unwrap_or_default()
}

fn spec_problems(problems: Vec<String>) -> ApiError {
    ApiError::invalid(format!("the spec does not validate: {}", problems.join("; "))).details(json!({"problems": problems}))
}

fn character_create(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = open_store(input)?;
    let default = gp::DEFAULT.to_string();
    let placement = PlacementVersion::parse(input.get("placement").and_then(Value::as_str).unwrap_or(&default)).map_err(ApiError::invalid)?;
    let spec = Spec {
        format: FORMAT.into(),
        name: input.get("name").and_then(Value::as_str).unwrap_or("character").to_string(),
        base: SpecBase {
            pack: text(input, "pack").to_string(),
            catalog: text(input, "catalog").to_string(),
            game_base: input.get("game").and_then(Value::as_str).map(str::to_string),
            placement: placement.to_string(),
        },
        weights: number_map(input.get("weights")),
        macros: None,
        residual: None,
        eyeballs: None,
    }
    .canonical();
    let (problems, _) = ch::problems(&store, &spec, None);
    if !problems.is_empty() {
        return Err(spec_problems(problems));
    }
    let out = text(input, "out");
    let revision = ch::write_spec(Path::new(out), &spec).map_err(ApiError::internal)?;
    let spec_json = serde_json::to_value(&spec).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(json!({"out": out, "revision": revision, "weights": spec.weights.len(), "spec": spec_json}))
}

fn read_spec(input: &Value) -> Result<Spec, ApiError> {
    let path = text(input, "spec");
    if !Path::new(path).is_file() {
        return Err(ApiError::not_found(format!("spec file {path} does not exist")).details(json!({"spec": path})));
    }
    ch::read_spec(Path::new(path)).map_err(ApiError::invalid)
}

fn character_validate(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = open_store(input)?;
    let spec = read_spec(input)?;
    let (problems, _) = ch::problems(&store, &spec, None);
    if !problems.is_empty() {
        return Err(spec_problems(problems));
    }
    Ok(json!({"revision": spec.revision(), "canonical": spec.canonical() == spec}))
}

fn evaluation_error(message: String) -> ApiError {
    match message.strip_prefix("the spec does not validate: ") {
        Some(rest) => spec_problems(rest.split("; ").map(str::to_string).collect()),
        None => ApiError::internal(message),
    }
}

fn character_evaluate(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = open_store(input)?;
    let spec = read_spec(input)?;
    let precision = input.get("precision").and_then(Value::as_str).unwrap_or("f32");
    let (positions, report) = ch::evaluate(&store, &spec, None).map_err(evaluation_error)?;
    let out = text(input, "out");
    if precision == "f32" {
        dense::write_points(Path::new(out), &positions)
    } else {
        dense::write_points_f64(Path::new(out), &positions)
    }
    .map_err(ApiError::internal)?;
    Ok(json!({"out": out, "revision": spec.revision(), "vertices": positions.len(), "precision": precision, "report": report}))
}

fn character_measure(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = open_store(input)?;
    let spec = read_spec(input)?;
    let (positions, base, _) = ch::evaluate_with_base(&store, &spec, None).map_err(evaluation_error)?;
    let custom = input.get("pairs").and_then(Value::as_object);
    let pairs: BTreeMap<String, [usize; 2]> = match custom {
        Some(map) => map
            .iter()
            .map(|(k, v)| {
                let a = v.get(0).and_then(Value::as_u64).unwrap_or(0) as usize;
                let b = v.get(1).and_then(Value::as_u64).unwrap_or(0) as usize;
                (k.clone(), [a, b])
            })
            .collect(),
        None => {
            if positions.len() != measure::BASE_VERTICES {
                return Err(ApiError::invalid(format!(
                    "the default landmarks are defined on the {}-vertex base; this pack has {} vertices",
                    measure::BASE_VERTICES,
                    positions.len()
                ))
                .hint("pass pairs: {name: [vertex, vertex]} to measure another base"));
            }
            measure::default_pairs()
        }
    };
    let mut measurements = Map::new();
    for (name, pair) in &pairs {
        let now = measure::distance_mm(&positions, *pair).map_err(ApiError::invalid)?;
        let was = measure::distance_mm(&base, *pair).map_err(ApiError::invalid)?;
        let mut entry = json!({"mm": measure::round_mm(now), "base_mm": measure::round_mm(was), "delta_mm": measure::round_mm(now - was),
                               "vertices": pair});
        if custom.is_none() {
            let m = measure::MEASURES.iter().find(|m| m.name == name).expect("default set");
            entry["from"] = json!(m.from);
            entry["to"] = json!(m.to);
            entry["what"] = json!(m.what);
        }
        measurements.insert(name.clone(), entry);
    }
    let landmarks: Vec<Value> = match custom {
        Some(_) => Vec::new(),
        None => measure::LANDMARKS.iter().map(|l| json!({"name": l.name, "vertex": l.vertex})).collect(),
    };
    Ok(json!({"revision": spec.revision(), "vertices": positions.len(), "units": "mm", "measurements": measurements,
              "landmarks": landmarks}))
}
