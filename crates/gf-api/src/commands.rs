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
use crate::tasks::api;
use crate::{CommandDef, Ctx, Kind, anim, body, designer, eyes, grid, jobs, photo, photo_fit, photo_gate, project, registry, skin, wear, wizards};

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
        summary: "The viewer's scene extras for a game base: its coarser levels of detail as triangles over the finest level's vertices, and the eyes (centre, radius, side, gaze and mesh) of an eye model (eyeballs with a model, with UVs and parts) or of an optional eyes file.",
        limits: "A level with a vertex outside the finest level drops that triangle and counts it; the eyes are at their rest place, before the character moves them (designer.settle reports each eye's transform).",
        input: designer::scene_input,
        output: designer::scene_output,
        run: designer::scene,
    },
    CommandDef {
        name: "designer.eyes",
        kind: Kind::Query,
        summary: "Only the eyes of designer.scene: an eye model's eyes (eyeballs with a model) or an eyes file's, at their rest place, for a client that changes the model without reloading the scene.",
        limits: "The same eyes and format as designer.scene's eyes; no levels of detail.",
        input: designer::scene_input,
        output: designer::eyes_output,
        run: designer::eyes,
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
        name: "eyes.inventory",
        kind: Kind::Query,
        summary: "The eye inventory: premade eye models (proportions, iris colours and pattern, sclera tint and veins, cornea index of refraction), grouped by family, and the default model.",
        limits: "A character takes a model through eyeballs.model ({id, radius_mm}); the settle and the export then build that model's eyes at the eye rig's places instead of reading an eyes file.",
        input: eyes::inventory_input,
        output: eyes::inventory_output,
        run: eyes::inventory,
    },
    CommandDef {
        name: "eyes.look",
        kind: Kind::Query,
        summary: "How one eye model is shaded, part by part (sclera, cornea, iris, pupil): base colour, roughness, transmission and index of refraction, blending, and the iris and sclera textures as RGBA pixels - the same values the glb export writes as materials.",
        limits: "Textures are procedural (seeded per model) at 1024x256 (iris) and 512x256 (sclera); textures false returns only their sizes.",
        input: eyes::look_input,
        output: eyes::look_output,
        run: eyes::look_command,
    },
    CommandDef {
        name: "eyes.textures",
        kind: Kind::Mutation,
        summary: "Write the procedural textures of eye models as PNG: the iris (radial fibres, crypts, collarette, pupillary ruff, limbal ring, an optional heterochromia sector) and the sclera (tint and veins), laid out on the UVs of the eyeball's iris and sclera parts.",
        limits: "Overwrites <id>.iris.png and <id>.sclera.png in out. Deterministic per model (seeded).",
        input: eyes::textures_input,
        output: eyes::textures_output,
        run: eyes::textures,
    },
    CommandDef {
        name: "body.targets",
        kind: Kind::Mutation,
        summary: "Import MakeHuman targets onto a stored body pack (by each pack vertex's base.obj index), in metres in the dense frame, so a character spec's body can be shaped live by the macro sliders and by regional targets. Returns the reference a spec's body.targets takes.",
        limits: "Needs a pack imported in makehuman mode with this version (it records source.npy) and put in the store (body.import with store). Targets that touch none of the pack's vertices are skipped.",
        input: body::targets_input,
        output: body::targets_output,
        run: body::targets,
    },
    CommandDef {
        name: "body.refined",
        kind: Kind::Query,
        summary: "A refined body's Catmull-Clark positions at the pack's base shape, and its cage quads (diagnostic): the body is shaped on its cage and refined by a fixed stencil before it is stitched and decimated.",
        limits: "Diagnostic; only bodies imported with subdivide > 0.",
        input: body::refined_input,
        output: body::refined_output,
        run: body::refined,
    },
    CommandDef {
        name: "body.cage",
        kind: Kind::Query,
        summary: "Every dense vertex's place on the head cage, from topology alone: a cage corner, a point along a cage edge, or a point inside a cage face, as up to four cage corners and bilinear coefficients (how a per-cage-vertex value such as a skin weight reaches the dense head).",
        limits: "Diagnostic; needs the dense base's patches of 16 x 16 quads per cage face.",
        input: body::cage_input,
        output: body::cage_output,
        run: body::cage,
    },
    CommandDef {
        name: "body.joints",
        kind: Kind::Query,
        summary: "A MakeHuman rig's joints on a spec's shaped body: every bone's head and tail evaluated on the body and its joint helper cubes as the macros, regional weights and compounds shape them, in the world frame and the canonical frame.",
        limits: "Needs a body imported with its helper vertices and targets carrying their deltas (re-import older ones). Root bones whose head is the ground cube sit on the floor, not at MPFB's armature origin. The bones under the bone named head (jaw, eyes, face bones) follow the head's own sliders, head scale and sculpt as on export (head_joints says how many moved). Known gaps: the stitch's blend band near the neck is not applied to joints.",
        input: body::joints_input,
        output: body::joints_output,
        run: body::joints,
    },
    CommandDef {
        name: "wearable.list",
        kind: Kind::Query,
        summary: "The wearables in a store (clothes, shoes, hats, hair, beards imported with wearable.import): reference, name, vertex count, how many base vertices each hides, whether it is transparent, its material maps.",
        limits: "A wearable is worn by listing it in the spec's body.wearables; it needs a MakeHuman-mode body.",
        input: wear::list_input,
        output: wear::list_output,
        run: wear::list,
    },
    CommandDef {
        name: "wearable.categorize",
        kind: Kind::Mutation,
        summary: "Say what a wearable in a store is (hair, facial_hair, hat, eyewear, top, bottom, outfit, shoes, accessory or other); the clients group their wearable pickers by it.",
        limits: "Kept in the store's wearable_categories.json, beside the content-addressed wearables, so a wearable's reference never changes. Imports keep a category only when their source states it (MakeHuman's asset folder and tags; a groom's cards are hair); wearable.list infers one for the rest from the name, transparency and whether the item rides a bone. Entries with a category this build does not know are kept when the file is rewritten, and ignored.",
        input: wear::categorize_input,
        output: wear::categorize_output,
        run: wear::categorize,
    },
    CommandDef {
        name: "wearable.import",
        kind: Kind::Mutation,
        summary: "Import MakeHuman clothes files (.mhclo with their .obj, .mhmat and textures) into a store as wearables; the binding fits each one to any shaped body.",
        limits: "Content-addressed: importing the same files again gives the same reference. The garment mesh must have one vertex per binding line.",
        input: wear::import_input,
        output: wear::import_output,
        run: wear::import,
    },
    CommandDef {
        name: "wearable.attach",
        kind: Kind::Job,
        summary: "Make a rigid wearable (a sword, a helmet, armour plates) from an OBJ modelled on one of your characters: it rides one bone, placed by that bone's frame on any character and weighted wholly to it (argv `wearable attach`).",
        limits: "Fields are the argv options with _ for -. The solid must be modelled on the character the spec describes, in metres with z up (world: as exported or fitted). It is not pushed out of the body or layered.",
        input: api::attach_input,
        output: api::argv_output,
        run: api::attach,
    },
    CommandDef {
        name: "wearable.bind",
        kind: Kind::Job,
        summary: "Make a wearable from any garment OBJ modelled over one of your characters (from Blender, a generator or another tool): each vertex is bound to the nearest point of that character's body and head surface, the body it covers is listed, and the result is stored like a MakeHuman garment (argv `wearable bind`).",
        limits: "Fields are the argv options with _ for -. The garment must be modelled on the character the spec describes, in metres with z up (world: as exported or fitted). One garment per call; give its colour texture with diffuse.",
        input: api::bind_input,
        output: api::argv_output,
        run: api::bind,
    },
    CommandDef {
        name: "groom.cards",
        kind: Kind::Job,
        summary: "Turn a groom into game hair: grow it on a character's scalp, group its strands into hair cards (strips along each group's mean strand, as wide as the group) with a strand-painted alpha atlas in the groom's colour, and store the cards as a double-sided hair wearable bound to the character like any garment, to wear through body.wearables (argv `groom cards`).",
        limits: "Fields are the argv options with _ for -. The cards are made on the character the spec describes and follow other shapes through the binding, like MakeHuman hair; they are rigid strips (no spring chains are written here: wear the result with physics springbone for those). One atlas of 8 strand columns per groom; the groom's curl and frizz show only through the cards' paths.",
        input: api::cards_input,
        output: api::argv_output,
        run: api::cards,
    },
    CommandDef {
        name: "wearable.bake",
        kind: Kind::Job,
        summary: "Bake a character's spring-bone chains (hair, ropes, capes worn with physics springbone) and cloth (worn with physics cloth) offline: a VRM-style spring solver runs over the chains and the body's capsule colliders in GAZEFORGE_physics, driven by a built-in clip (sway) or an animation already in the glb, and the result is written into a copy of the glb as an animation, as bone rotation keys or as per-frame morph targets (argv `wearable bake`; export-glb takes the same bake options).",
        limits: "Fields are the argv options with _ for -. Cloth is written as morph targets in either format (it has no joints) and each of its levels of detail is simulated on its own. LINEAR and STEP source clips only. The solver's settings are the file's spring_defaults; the result does not loop exactly (the report gives the gap). Vertex output, and cloth in either format, is refused past 256 MB of morph targets.",
        input: api::bake_input,
        output: api::argv_output,
        run: api::bake,
    },
    CommandDef {
        name: "body.sliders",
        kind: Kind::Query,
        summary: "The body's regional sliders and compound parameters: every decrease/increase pair of a body targets set (arms, legs, torso, hips, stomach, buttocks, pelvis, hands, feet, ...) grouped by family, leaving out targets the head catalog also has (the head's sliders drive those on both sides), and the compound parameters with any target they need that is missing.",
        limits: "Names are MakeHuman's target ids; a pair is a slider from -1 (decrease) to +1 (increase).",
        input: body::sliders_input,
        output: body::sliders_output,
        run: body::sliders,
    },
    CommandDef {
        name: "skin.models",
        kind: Kind::Query,
        summary: "The procedural skin presets: tone (melanin, haemoglobin, undertone), regional redness, lips, under-eyes, freckles, moles, pores, age, stubble and its hair colour, T-zone oil.",
        limits: "Presets are starting points; skin.generate takes a preset and overrides any field.",
        input: skin::models_input,
        output: skin::models_output,
        run: skin::models,
    },
    CommandDef {
        name: "skin.generate",
        kind: Kind::Mutation,
        summary: "Write a procedural skin for a game base as PNG: albedo, a tangent-space normal map (pores, fine lines) and an occlusion-roughness-metallic map, laid out on the game base's UVs. The skin is evaluated on the neutral base's surface in millimetres, so it has no UV seams and follows every character built on that base.",
        limits: "Overwrites <id>.albedo.png, <id>.normal.png and <id>.orm.png in out. Deterministic per look (seeded). Needs the base's landmark table for its regions.",
        input: skin::generate_input,
        output: skin::generate_output,
        run: skin::generate,
    },
    CommandDef {
        name: "skin.layers",
        kind: Kind::Query,
        summary: "The layer kinds and styles a character spec's skin.layers can name (grime: dust, sand, dirt, mud; blood: drips, splatter; paint: eye-band, cheek-stripes, chin), each with its colour, opacity, roughness, edge spread and second state (wet, dry). A live layer is exported as a mask a renderer drives through the GAZEFORGE_skin glTF extension; a baked one is painted into the base textures.",
        limits: "Masks are procedural, placed by the base's landmark table and seeded; painted or stamped masks are not available yet.",
        input: skin::layers_input,
        output: skin::layers_output,
        run: skin::layers,
    },
    CommandDef {
        name: "skin.landmarks",
        kind: Kind::Mutation,
        summary: "Put a base's landmark table (JSON {\"vertices\": {MediaPipe landmark: dense vertex}}) into the store, after checking it against the game base's pack and the landmarks the skin regions and layers need. Returns the reference a character spec's skin.landmarks takes.",
        limits: "Checks indices and presence only, not where the landmarks sit on the face.",
        input: skin::landmarks_input,
        output: skin::landmarks_output,
        run: skin::landmarks,
    },
    CommandDef {
        name: "skin.stamp",
        kind: Kind::Mutation,
        summary: "Put a stamp image (PNG) into the store for a skin layer's stamps. Returns the reference a stamp's image takes, the image's size and how much of it the alpha and luminance channels cover.",
        limits: "A stamp's mask is one channel of the image (alpha by default, or luminance); colour stamps are not available yet.",
        input: skin::stamp_input,
        output: skin::stamp_output,
        run: skin::stamp,
    },
    CommandDef {
        name: "skin.project",
        kind: Kind::Mutation,
        summary: "Project the photos of a photos-path run onto the game base's texels, through the fitted cameras: visibility by the fitted mesh's z-buffer, weights by facing and distance from the outline, eyes and brows masked by the front photo's landmarks, delit by a spherical-harmonics shading fit, each view gain-matched to the front. What no photo sees takes the mirrored side, then a procedural skin whose tone is fitted to the photos and whose colour is pulled to the photos' by a gain field spread over the mesh. Also writes the procedural skin alone (generated) and a confidence map.",
        limits: "Cast shadows, ambient occlusion and specular light are not modelled (highlights are down-weighted, not removed). Hair on the forehead or temples is projected as skin. Needs the run's views.json, fit_photos.json and fitted_dense.npy, and Python with numpy and opencv.",
        input: skin::project_input,
        output: skin::project_output,
        run: skin::project,
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
        name: "jobs.list",
        kind: Kind::Query,
        summary: "The jobs in the jobs folder, newest first: id, command, state, submitted / started / finished times (ms since the epoch), progress while it runs, and the error or ok of a finished one.",
        limits: "Lists one jobs folder (GAZEFORGE_JOBS, else <temp>/gazeforge-jobs); jobs are never deleted by the server.",
        input: jobs_list_input,
        output: jobs_list_output,
        run: jobs::list,
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
    CommandDef {
        name: "store.import_pack",
        kind: Kind::Job,
        summary: "Import a dense base directory into a store as a pack (argv `store import-pack`); optional macro base sliders and pin height.",
        limits: "Fields are the argv options with _ for -; the result is the argv command's JSON. The pack is content-addressed: the same directory gives the same reference.",
        input: api::import_pack_input,
        output: api::argv_output,
        run: api::import_pack,
    },
    CommandDef {
        name: "store.import_catalog",
        kind: Kind::Job,
        summary: "Import a directory of sparse targets on a pack's dense topology into a store as a catalog (argv `store import-catalog`).",
        limits: "Fields are the argv options with _ for -; the result is the argv command's JSON.",
        input: api::import_catalog_input,
        output: api::argv_output,
        run: api::import_catalog,
    },
    CommandDef {
        name: "store.import_game",
        kind: Kind::Job,
        summary: "Import a game base directory (made by base.build) for a pack into a store (argv `store import-game`).",
        limits: "Fields are the argv options with _ for -; the result is the argv command's JSON.",
        input: api::import_game_input,
        output: api::argv_output,
        run: api::import_game,
    },
    CommandDef {
        name: "store.import_eyerig",
        kind: Kind::Job,
        summary: "Import MakeHuman's helper eyeballs and their per-target deltas for a catalog into a store as an eye rig (argv `store import-eyerig`), so eyeballs can follow the character.",
        limits: "Fields are the argv options with _ for -; the result is the argv command's JSON. A registration worse than 2 mm (median) is refused.",
        input: api::import_eyerig_input,
        output: api::argv_output,
        run: api::import_eyerig,
    },
    CommandDef {
        name: "base.build",
        kind: Kind::Job,
        summary: "Build a game base from a dense base: decimate to a target vertex count with angle limits, carry shapes, and optional nested levels of detail (argv `build-base`).",
        limits: "Fields are the argv options with _ for -; the result is the argv command's JSON. Takes minutes at the full dense size.",
        input: api::build_input,
        output: api::argv_output,
        run: api::build,
    },
    CommandDef {
        name: "base.derive_lods",
        kind: Kind::Job,
        summary: "Add or replace a game base's coarser levels of detail: the finest level stays exactly as it is, every coarser level is decimated from it (carrying the dense shapes and layout weights), and the result is a new game base in the store (argv `derive-lods`).",
        limits: "Fields are the argv options with _ for -; the result is the argv command's JSON. lods must be strictly decreasing and below the finest level's vertex count. Pass the importance weights the base was built with (weights): without them the coarser levels are decimated unweighted, and the base does not record which weights it used.",
        input: api::derive_input,
        output: api::argv_output,
        run: api::derive,
    },
    CommandDef {
        name: "export.obj",
        kind: Kind::Job,
        summary: "Place dense positions on a game base with a placement and write the game mesh as an .obj (argv `export-obj`).",
        limits: "Fields are the argv options with _ for -; a field the chosen placement does not read is refused. The result is the argv command's JSON.",
        input: api::obj_input,
        output: api::argv_output,
        run: api::export_obj,
    },
    CommandDef {
        name: "export.glb",
        kind: Kind::Job,
        summary: "Export a character spec as a glb: the game mesh with its levels of detail, morph targets, eyes, and optionally the face rig, eyelid fitting and a body (argv `export-glb`).",
        limits: "Fields are the argv options with _ for -; a field the spec's placement does not read is refused, and the lids_* fields need lids auto. The result is the argv command's JSON.",
        input: api::glb_input,
        output: api::argv_output,
        run: api::export_glb,
    },
    CommandDef {
        name: "lids.collide",
        kind: Kind::Job,
        summary: "Push dense eyelid vertices out of the eyeballs (argv `collide-lids`); the area inside the eyes before and after.",
        limits: "Fields are the argv options with _ for -; the result is the argv command's JSON.",
        input: api::collide_input,
        output: api::argv_output,
        run: api::collide,
    },
    CommandDef {
        name: "macro.weights",
        kind: Kind::Query,
        summary: "The target weights a set of macro sliders produces under a macro definition (argv `macro weights`).",
        limits: "Fields are the argv options with _ for -; sliders may be a JSON object. The result is the argv command's JSON.",
        input: api::weights_input,
        output: api::argv_output,
        run: api::weights,
    },
    CommandDef {
        name: "macro.labels",
        kind: Kind::Query,
        summary: "Label slider values with the macro definition's key values for one slider and write the table as .npy (argv `macro labels`).",
        limits: "Fields are the argv options with _ for -; the result is the argv command's JSON.",
        input: api::labels_input,
        output: api::argv_output,
        run: api::labels,
    },
    CommandDef {
        name: "gates.shell",
        kind: Kind::Job,
        summary: "Shell gates between two dense surfaces: folds, inner surfaces and roughness (argv `gates shell`).",
        limits: "Fields are the argv options with _ for -. A failing gate is the error gate_failed with the full report in details.",
        input: api::shell_input,
        output: api::argv_output,
        run: api::shell,
    },
    CommandDef {
        name: "gates.game",
        kind: Kind::Job,
        summary: "Game mesh gates on an exported .obj: triangle quality, UV health, census and seams (argv `gates game`).",
        limits: "Fields are the argv options with _ for -. A failing gate is the error gate_failed with the full report in details.",
        input: api::game_input,
        output: api::argv_output,
        run: api::game,
    },
    CommandDef {
        name: "gates.path",
        kind: Kind::Job,
        summary: "Continuity gate: walk a dense change in steps through a placement and refuse forward jumps of game vertices (argv `gates path`).",
        limits: "Fields are the argv options with _ for -; plant adds a deliberate jump to prove the gate. A failing gate is the error gate_failed with the full report in details.",
        input: api::path_input,
        output: api::argv_output,
        run: api::path,
    },
    CommandDef {
        name: "gates.facing",
        kind: Kind::Job,
        summary: "Facing gate on a glb: no triangle faces against its vertex normals in the neutral and, with states all, in each morph state (argv `gates facing`).",
        limits: "Fields are the argv options with _ for -. A failing gate is the error gate_failed with the full report in details.",
        input: api::facing_input,
        output: api::argv_output,
        run: api::facing,
    },
    CommandDef {
        name: "gates.lids",
        kind: Kind::Job,
        summary: "Eyelid gate on a glb: no vertex inside an eyeball beyond the tolerance in the neutral and, with states all (the default), in each morph state (argv `gates lids`).",
        limits: "Fields are the argv options with _ for -. A failing gate is the error gate_failed with the full report in details.",
        input: api::lids_input,
        output: api::argv_output,
        run: api::lids,
    },
    CommandDef {
        name: "gates.rig",
        kind: Kind::Job,
        summary: "Face rig gate on a glb: the skin binds exactly at rest with sound weights, and each joint pose folds no more triangles (and, with eyes, puts no more lid vertices inside an eye) than the blend shape of the same name (argv `gates rig`).",
        limits: "Fields are the argv options with _ for -. A failing gate is the error gate_failed with the full report in details.",
        input: api::rig_input,
        output: api::argv_output,
        run: api::rig,
    },
    CommandDef {
        name: "topology.improve",
        kind: Kind::Job,
        summary: "Re-cut a game base's triangles by edge flips so the worst angles over the shape range it serves rise, and import the result as a new game base (argv `topology improve`); before and after statistics per shape.",
        limits: "Flips only: the same vertices (kept and owner unchanged), never across a UV seam or the open boundary, never where a scored shape would turn a triangle over or its texture orientation flip. The old base is untouched; characters name their base by sha256, so they change only when pointed at the new one. Reads every scored catalog target (minutes with all of them).",
        input: api::topology_improve_input,
        output: api::argv_output,
        run: api::topology_improve,
    },
    CommandDef {
        name: "topology.inspect",
        kind: Kind::Query,
        summary: "A game base's triangle quality: every triangle's smallest angle at the rest shape, its triangles, and the record of the improve that made it (with each triangle's worst angle over that shape range), for showing bad triangles (argv `topology inspect`).",
        limits: "The rest shape only is measured here; the shape range numbers are those an improve recorded. Loads the pack (about a second).",
        input: api::topology_inspect_input,
        output: api::argv_output,
        run: api::topology_inspect,
    },
    CommandDef {
        name: "wizards.list",
        kind: Kind::Query,
        summary: "The GUI wizards: guided tasks whose every step is one registry command (id, title, group, summary, step count).",
        limits: "Developer wizards are listed only with developer true. A wizard adds no capability: each step runs the same command the CLI, HTTP and MCP run.",
        input: wizards::list_input,
        output: wizards::list_output,
        run: wizards::list,
    },
    CommandDef {
        name: "wizards.get",
        kind: Kind::Query,
        summary: "One wizard's definition: its steps, each with its command, kind, input schema, the fields a form shows (common or advanced, widget hints, bindings to run values and earlier steps) and what the step shows when it ends.",
        limits: "Bindings: $run.NAME (a value given to wizards.start), $STEP.input.FIELD, $STEP.result.PATH, =TEXT (a literal).",
        input: wizards::get_input,
        output: wizards::get_output,
        run: wizards::get,
    },
    CommandDef {
        name: "wizards.start",
        kind: Kind::Mutation,
        summary: "Start a wizard run: a folder ROOT/runs/PROJECT/WIZARD-N holding steps/STEP per step (its input, its result or job) and the files the steps write ($dir); returns the run's state.",
        limits: "Runs are resumable: wizards.state reads them back at any time; nothing is deleted.",
        input: wizards::start_input,
        output: wizards::run_output,
        run: wizards::start,
    },
    CommandDef {
        name: "wizards.state",
        kind: Kind::Query,
        summary: "A wizard run's state: per step open, running, done, failed or stale, the proposed input (bindings resolved), the input given, the result or error and the job id; a finished job's result is collected here.",
        limits: "A step's state is read from its folder; editing an earlier step marks the later ones stale until they run again.",
        input: wizards::state_input,
        output: wizards::run_output,
        run: wizards::state,
    },
    CommandDef {
        name: "wizards.runs",
        kind: Kind::Query,
        summary: "A project's wizard runs, newest first: run id, wizard, start time and each step's state, so a client can resume one with wizards.state.",
        limits: "Only runs whose run record reads are listed; a run of a wizard that no longer exists is skipped.",
        input: wizards::runs_input,
        output: wizards::runs_output,
        run: wizards::runs,
    },
    CommandDef {
        name: "wizards.step",
        kind: Kind::Mutation,
        summary: "Run one wizard step: its fields (hidden and bound fields not given take their bound values) are checked against the step command's schema and the command runs, or is submitted as a job; later steps that ran are marked stale.",
        limits: "Steps run in order: a step needs every earlier non-optional step done. A job step returns at once; wizards.state collects its result.",
        input: wizards::step_input,
        output: wizards::run_output,
        run: wizards::step,
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

fn jobs_list_input() -> Value {
    obj(json!({"limit": {"type": "integer", "minimum": 1, "description": "at most this many, newest first (default 50)"}}), &[])
}

fn jobs_list_output() -> Value {
    shape(json!({"jobs_dir": {"type": "string"}, "total": {"type": "integer"}, "jobs": {"type": "array"}}))
}

fn job_output() -> Value {
    shape(json!({"id": {"type": "string"}, "command": {"type": "string"}, "state": {"type": "string"},
                 "result": {"type": "object"}, "worker_processes_left": {"type": ["integer", "null"]},
                 "progress": {"type": "object", "description": "the latest stage, done, total and message the command reported; absent until it reports"}}))
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

fn imported(store: &Store, kind: &str, reference: &str) -> Value {
    let path = store.root.join(kind).join(format!("{}.json", reference.trim_start_matches("sha256:")));
    std::fs::metadata(path).and_then(|m| m.modified()).map_or(Value::Null, |t| json!(project::utc(t)))
}

fn store_list(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = open_store(input)?;
    let packs: Vec<Value> = manifests::<PackManifest>(&store, "packs")?
        .into_iter()
        .map(|(r, m)| json!({"pack": r, "vertices": m.vertices, "faces": m.faces, "macro_base": m.macro_base.is_some(), "imported": imported(&store, "packs", &r)}))
        .collect();
    let catalogs: Vec<Value> = manifests::<CatalogManifest>(&store, "catalogs")?
        .into_iter()
        .map(|(r, m)| json!({"catalog": r, "pack": m.pack, "targets": m.targets.len(), "imported": imported(&store, "catalogs", &r)}))
        .collect();
    let games: Vec<Value> = manifests::<GameManifest>(&store, "game")?
        .into_iter()
        .map(|(r, m)| json!({"game": r, "pack": m.pack, "vertices": m.vertices, "triangles": m.triangles, "lods": m.lods.len(), "imported": imported(&store, "game", &r)}))
        .collect();
    let eyerigs: Vec<Value> = match store.root.join("eyerigs").is_dir() {
        true => manifests::<Value>(&store, "eyerigs")?
            .into_iter()
            .map(|(r, m)| json!({"eyerig": r, "catalog": m["catalog"], "format": m["format"], "usable": m["format"] == gf_core::eyerig::FORMAT, "imported": imported(&store, "eyerigs", &r)}))
            .collect(),
        false => Vec::new(),
    };
    Ok(json!({"packs": packs, "catalogs": catalogs, "game_bases": games, "eyerigs": eyerigs}))
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
        skin: None,
        body: None,
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
