use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::commands::{obj, shape, text};
use crate::error::ApiError;
use crate::{Ctx, Kind, check_input, find, jobs, project};

pub const FORMAT: &str = "gazeforge.wizard-run/1";

pub fn definitions() -> Vec<Value> {
    vec![setup(), build_base(), lods(), create(), photos(), export(), bake_physics(), body(), wearable(), bind_garment(), groom_cards(), attach_solid(), check(), lids(), macros()]
}

fn hidden(name: &str, bind: &str) -> Value {
    json!({"name": name, "bind": bind, "hidden": true})
}

fn setup() -> Value {
    json!({
        "id": "setup", "title": "Set up a store", "group": "Setup", "developer": false,
        "summary": "A store from a dense base folder, its targets, a game base and MakeHuman's eye rig: everything the designer needs.",
        "steps": [
            {"id": "pack", "title": "Dense base", "command": "store.import_pack",
             "help": "The dense base folder (positions, faces, UVs, manifest) becomes the store's pack; it is content-addressed, so the same folder gives the same reference.",
             "fields": [
                 {"name": "store", "label": "Store folder", "widget": "folder", "bind": "$run.store", "common": true},
                 {"name": "dense", "label": "Dense base folder", "widget": "folder", "common": true},
                 {"name": "macro_base", "label": "Macro base sliders", "common": true},
                 {"name": "pin_top", "label": "Pin height (m)"},
             ],
             "shows": [{"kind": "report"}]},
            {"id": "catalog", "title": "Targets", "command": "store.import_catalog",
             "help": "The targets folder (sparse targets on the dense topology) becomes the pack's catalog.",
             "fields": [hidden("store", "$pack.input.store"), hidden("pack", "$pack.result.pack"),
                        {"name": "targets", "label": "Targets folder", "widget": "folder", "common": true}],
             "shows": [{"kind": "report"}]},
            {"id": "game", "title": "Game base", "command": "store.import_game",
             "help": "An existing game base folder for this pack (the Build a game base task makes one).",
             "fields": [hidden("store", "$pack.input.store"), hidden("pack", "$pack.result.pack"),
                        {"name": "base", "label": "Game base folder", "widget": "folder", "common": true}],
             "shows": [{"kind": "report"}]},
            {"id": "eyerig", "title": "Eye rig", "command": "store.import_eyerig", "optional": true,
             "help": "MakeHuman's helper eyeballs and their per-target moves, so eyeballs follow the character and eye models can be placed. It needs the pack imported with its macro base sliders: a pack registered at another state is refused (median skin distance over 2 mm).",
             "fields": [hidden("store", "$pack.input.store"), hidden("catalog", "$catalog.result.catalog"),
                        {"name": "makehuman", "label": "MakeHuman data folder", "widget": "folder", "common": true},
                        {"name": "macro", "label": "Macro definition", "widget": "file_open", "ext": ".json", "common": true}],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn build_base() -> Value {
    json!({
        "id": "build_base", "title": "Build a game base", "group": "Setup", "developer": false,
        "summary": "A game mesh decimated from the dense base (every vertex an original dense vertex), with nested levels of detail, imported into the store.",
        "steps": [
            {"id": "build", "title": "Build", "command": "base.build",
             "help": "Decimate to a vertex target with angle guards; the shapes the collapse must serve keep their detail. Minutes at the full dense size.",
             "fields": [
                 {"name": "dense", "label": "Dense base folder", "widget": "folder", "common": true},
                 {"name": "out", "label": "Output folder", "widget": "folder", "bind": "$dir/base", "common": true},
                 {"name": "target", "label": "Vertices", "common": true},
                 {"name": "lods", "label": "Levels of detail (e.g. 8000,3000)", "common": true},
                 {"name": "shapes", "label": "Shapes to serve (all, none, a,b)"},
                 {"name": "weights", "label": "Importance weights (.npy)", "widget": "file_open", "ext": ".npy"},
             ],
             "shows": [{"kind": "report"}]},
            {"id": "import", "title": "Import", "command": "store.import_game",
             "help": "The new base into the store, beside every base it already has.",
             "fields": [hidden("base", "$build.input.out"),
                        {"name": "store", "label": "Store folder", "widget": "folder", "bind": "$run.store", "common": true},
                        {"name": "pack", "label": "Pack", "bind": "$run.pack", "common": true}],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn lods() -> Value {
    json!({
        "id": "lods", "title": "Add levels of detail", "group": "Setup", "developer": false,
        "summary": "More or different coarser levels of detail for a game base already in the store: the finest level stays exactly as it is, so characters made on it keep their head.",
        "steps": [
            {"id": "derive", "title": "Levels", "command": "base.derive_lods",
             "help": "List the coarser levels as vertex counts, strictly decreasing and below the finest level (for example 10000,5000,2500,1200,800; 800 is the recommended floor, below it the neck seam has too few points for a body). Each is decimated from the finest, carrying the dense shapes; give the importance weights the base was built with, or the coarse levels are decimated without them. The new game base goes into the store beside the old one. Point a character's base.game_base at it, then import a body with as many levels of detail.",
             "fields": [
                 {"name": "store", "label": "Store folder", "widget": "folder", "bind": "$run.store", "common": true},
                 {"name": "game", "label": "Game base", "bind": "$run.game", "common": true},
                 {"name": "dense", "label": "Dense base folder", "widget": "folder", "common": true},
                 {"name": "out", "label": "Output folder", "widget": "folder", "bind": "$dir/base", "common": true},
                 {"name": "lods", "label": "Levels of detail (e.g. 10000,5000,2500)", "common": true},
                 {"name": "weights", "label": "Importance weights (.npy)", "widget": "file_open", "ext": ".npy", "common": true},
                 {"name": "seam_weight", "label": "Neck seam weight (keeps seam points for the body)", "common": true},
                 {"name": "shapes", "label": "Shapes to serve (all, none, a,b)"},
             ],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn create() -> Value {
    json!({
        "id": "create", "title": "Create a character from data", "group": "Characters", "developer": false,
        "summary": "A character spec from target weights, checked against its store.",
        "steps": [
            {"id": "create", "title": "Weights", "command": "character.create",
             "help": "Target weights in [0, 1] as a JSON object; macro sliders and a sculpt's residual are made with the command line form.",
             "fields": [hidden("store", "$run.store"), hidden("pack", "$run.pack"), hidden("catalog", "$run.catalog"), hidden("game", "$run.game"),
                        {"name": "weights", "label": "Weights (JSON)", "common": true},
                        {"name": "name", "label": "Name", "common": true},
                        {"name": "out", "label": "Spec file", "widget": "file_save", "ext": ".gfchar.json", "bind": "$dir/character.gfchar.json", "common": true},
                        {"name": "placement", "label": "Placement"}],
             "shows": [{"kind": "report"}]},
            {"id": "validate", "title": "Validate", "command": "character.validate",
             "help": "The spec against the store: format, references, target ids and weight ranges.",
             "fields": [hidden("store", "$run.store"), hidden("spec", "$create.result.out")],
             "shows": [{"kind": "report"}, {"kind": "spec", "from": "$create.result.out", "store": "$run.store"}]},
        ],
    })
}

fn body() -> Value {
    json!({
        "id": "body", "title": "Import a body", "group": "Body", "developer": false,
        "summary": "A body mesh made into a body pack the head stitches onto (the Stitch panel and export --body use it), and the MakeHuman targets that shape it with the character's macros and the Body shape panel.",
        "steps": [
            {"id": "import", "title": "Body", "command": "body.import",
             "help": "A MakeHuman body (exact) or any body mesh; the neck is cut where the head meets it. A MakeHuman body is refined by Catmull-Clark (subdivision levels) and each body level of detail is decimated from it: list one vertex count per head level of detail.",
             "fields": [
                 {"name": "mode", "label": "Kind", "common": true},
                 {"name": "obj", "label": "Body .obj", "widget": "file_open", "ext": ".obj", "common": true},
                 {"name": "dense", "label": "Dense base folder", "widget": "folder", "common": true},
                 {"name": "out", "label": "Body pack folder", "widget": "folder", "bind": "$dir/body", "common": true},
                 {"name": "store", "label": "Store folder", "widget": "folder", "bind": "$run.store", "common": true},
                 {"name": "targets", "label": "MakeHuman targets folder", "widget": "folder", "common": true},
                 {"name": "macro", "label": "Macro definition", "widget": "file_open", "ext": ".json", "common": true},
                 {"name": "subdivide", "label": "Subdivision levels (0-2)"},
                 {"name": "fine_lods", "label": "Body levels of detail (vertices, finest first)"},
             ],
             "shows": [{"kind": "report"}]},
            {"id": "targets", "title": "Targets", "command": "body.targets", "optional": true,
             "help": "The MakeHuman targets on this body, so the macros shape it and the Body shape panel has its sliders; name the families to import (macrodetails and breast by default; add legs, arms, torso, hip, stomach, buttocks, pelvis, neck, hands, feet and genitals for the regional sliders). Put the reference in the spec's body.targets.",
             "fields": [hidden("store", "$import.input.store"), hidden("body", "$import.result.body"),
                        {"name": "targets", "label": "MakeHuman targets folder", "widget": "folder", "bind": "$import.input.targets", "common": true},
                        {"name": "families", "label": "Target families (JSON list)", "common": true}],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn wearable() -> Value {
    json!({
        "id": "wearable", "title": "Import a garment", "group": "Body", "developer": false,
        "summary": "MakeHuman clothes, shoes, hats, hair or beards into the store, ready to wear in the Body shape panel.",
        "steps": [
            {"id": "import", "title": "Garment", "command": "wearable.import",
             "help": "The path of a MakeHuman clothes file (.mhclo) on this machine, with its .obj, .mhmat and textures beside it; several separated by commas. The binding fits the garment to any shaped body; its hidden-vertex list says what body it covers. Wear it from the Body shape panel (Wearables).",
             "fields": [
                 {"name": "store", "label": "Store folder", "widget": "folder", "bind": "$run.store", "common": true},
                 {"name": "mhclo", "label": "MakeHuman clothes file (.mhclo)", "common": true},
             ],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn bind_garment() -> Value {
    json!({
        "id": "bind_garment", "title": "Fit a garment made on a character", "group": "Body", "developer": false,
        "summary": "Any garment mesh modelled over one of your characters (Blender, a generator, another tool) into the store, ready to wear on every body.",
        "steps": [
            {"id": "bind", "title": "Garment", "command": "wearable.bind",
             "help": "Model the garment over a character exported from here (export-glb or a fit lookdev), in metres with z up, and export it as OBJ. Give the spec of that character: each garment vertex is bound to the nearest point of its body and head, so it follows every shape like MakeHuman clothes do. The body it covers is listed for Hide body. Paths are on this machine.",
             "fields": [
                 {"name": "store", "label": "Store folder", "widget": "folder", "bind": "$run.store", "common": true},
                 {"name": "spec", "label": "The character it was modelled on (spec)", "bind": "$run.spec", "common": true},
                 {"name": "obj", "label": "Garment .obj", "common": true},
                 {"name": "base", "label": "MakeHuman base.obj", "common": true},
                 {"name": "macro", "label": "Macro definition", "common": true},
                 {"name": "diffuse", "label": "Colour texture (.png)"},
                 {"name": "name", "label": "Name"},
                 {"name": "double_sided", "label": "Both faces (hair cards, thin sheets)"},
             ],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn attach_solid() -> Value {
    json!({
        "id": "attach_solid", "title": "Attach a solid to a bone", "group": "Body", "developer": false,
        "summary": "A rigid item - a sword, a helmet, a shield, armour plates - that rides one bone of the rig on every character.",
        "steps": [
            {"id": "attach", "title": "Solid", "command": "wearable.attach",
             "help": "Model the item where it belongs on a character exported from here (in the hand, on the head), in metres with z up, and export it as OBJ. Name the bone it rides (hand_r, hand_l, head, spine_03, ...). It keeps its size unless Scale says bone. Paths are on this machine.",
             "fields": [
                 {"name": "store", "label": "Store folder", "widget": "folder", "bind": "$run.store", "common": true},
                 {"name": "spec", "label": "The character it was modelled on (spec)", "bind": "$run.spec", "common": true},
                 {"name": "obj", "label": "Solid .obj", "common": true},
                 {"name": "bone", "label": "Bone", "common": true},
                 {"name": "macro", "label": "Macro definition", "common": true},
                 {"name": "rig", "label": "Rig"},
                 {"name": "scale", "label": "Scale"},
                 {"name": "diffuse", "label": "Colour texture (.png)"},
                 {"name": "name", "label": "Name"},
             ],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn groom_cards() -> Value {
    json!({
        "id": "groom_cards", "title": "Make game hair cards from a groom", "group": "Body", "developer": false,
        "summary": "A groom (grown hair) turned into hair cards with a strand-painted texture and stored as a hair wearable, for every engine that takes meshes.",
        "steps": [
            {"id": "cards", "title": "Cards", "command": "groom.cards",
             "help": "The groom is grown on the character's scalp and its strands are grouped into cards. Use a character with Grown hair turned on, or give a groom file. Wear the result from the Body shape panel: it follows every body shape like MakeHuman hair, thins over the levels of detail, and takes spring-bone physics. More cards cover the scalp better and cost more triangles. Paths are on this machine.",
             "fields": [
                 {"name": "store", "label": "Store folder", "widget": "folder", "bind": "$run.store", "common": true},
                 {"name": "spec", "label": "Character with the groom (spec)", "bind": "$run.spec", "common": true},
                 {"name": "base", "label": "MakeHuman base.obj", "common": true},
                 {"name": "macro", "label": "Macro definition", "common": true},
                 {"name": "groom", "label": "Groom file (instead of the character's own)"},
                 {"name": "cards", "label": "Cards"},
                 {"name": "atlas", "label": "Texture size (px)"},
                 {"name": "name", "label": "Name"},
             ],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn bake_physics() -> Value {
    json!({
        "id": "bake_physics", "title": "Bake hair and cloth physics into a glb", "group": "Export", "developer": false,
        "summary": "Spring-bone hair, ropes and capes simulated offline and written into a copy of an exported character as an animation, for engines that cannot run the physics themselves.",
        "steps": [
            {"id": "bake", "title": "Bake", "command": "wearable.bake",
             "help": "The glb must come from Export with a body skeleton and a wearable worn with spring bones. Clip: sway (four seconds of head turns, a lean and a sideways sway) or the name of an animation already in the file. Bones writes rotation keys for the chain joints; vertex writes one morph target per frame, which is larger. The report gives how far the hair settles from its modelled shape and how far it swings.",
             "fields": [
                 {"name": "glb", "label": "Exported glb", "widget": "file_open", "ext": ".glb", "bind": "$run.out", "common": true},
                 {"name": "out", "label": "Baked glb", "widget": "file_save", "ext": ".glb", "common": true},
                 {"name": "bake", "label": "Clip", "bind": "=sway", "common": true},
                 {"name": "bake_format", "label": "Bake as", "common": true},
                 {"name": "bake_fps", "label": "Frames per second"},
             ],
             "shows": [{"kind": "report"}, {"kind": "glb", "from": "$bake.result.out"}]},
        ],
    })
}

fn check() -> Value {
    json!({
        "id": "check", "title": "Check a sculpt against the base", "group": "Check", "developer": true,
        "summary": "Shell gates (folds, inner surfaces, roughness) between two dense surfaces.",
        "steps": [
            {"id": "shell", "title": "Shell gates", "command": "gates.shell",
             "help": "Before and after are base, shape:NAME or a dense .npy; a failing gate is reported with every value and its limit.",
             "fields": [{"name": "dense", "label": "Dense base folder", "widget": "folder", "common": true},
                        {"name": "before", "label": "Before", "bind": "=base", "common": true},
                        {"name": "after", "label": "After (.npy)", "widget": "file_open", "ext": ".npy", "common": true}],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn lids() -> Value {
    json!({
        "id": "lids", "title": "Eyelids out of the eyeballs", "group": "Check", "developer": true,
        "summary": "Dense eyelid vertices pushed out of the eyeballs, with the area inside the eyes before and after.",
        "steps": [
            {"id": "collide", "title": "Collide", "command": "lids.collide",
             "help": "Positions are base, shape:NAME or a dense .npy; the result is a dense .npy.",
             "fields": [{"name": "dense", "label": "Dense base folder", "widget": "folder", "common": true},
                        {"name": "positions", "label": "Positions", "common": true},
                        {"name": "eyes", "label": "Eyes file", "widget": "file_open", "ext": ".json", "common": true},
                        {"name": "out", "label": "Output .npy", "widget": "file_save", "ext": ".npy", "bind": "$dir/collided.npy", "common": true},
                        {"name": "clearance", "label": "Clearance (mm)", "common": true}],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn macros() -> Value {
    json!({
        "id": "macros", "title": "Macro tools", "group": "Developer", "developer": true,
        "summary": "The target weights a set of macro sliders makes, and a slider's label table over a range of values.",
        "steps": [
            {"id": "weights", "title": "Weights", "command": "macro.weights",
             "help": "Every macro slider and race value as a JSON object; the result lists the target weights they make.",
             "fields": [{"name": "macro", "label": "Macro definition", "widget": "file_open", "ext": ".json", "common": true},
                        {"name": "sliders", "label": "Sliders (JSON)", "common": true}],
             "shows": [{"kind": "report"}]},
            {"id": "labels", "title": "Labels", "command": "macro.labels", "optional": true,
             "help": "One slider's label weights over a .npy of slider values, written as a table.",
             "fields": [hidden("macro", "$weights.input.macro"),
                        {"name": "slider", "label": "Slider", "common": true},
                        {"name": "values", "label": "Values (.npy)", "widget": "file_open", "ext": ".npy", "common": true},
                        {"name": "out", "label": "Table (.npy)", "widget": "file_save", "ext": ".npy", "bind": "$dir/labels.npy", "common": true}],
             "shows": [{"kind": "report"}]},
        ],
    })
}

fn photos() -> Value {
    let fit = "$dir/fit";
    json!({
        "id": "photos",
        "title": "Character from photos",
        "group": "Characters",
        "developer": false,
        "summary": "A front and a profile photo (and optionally the other profile) to a character: checked, landmarked, fitted to the targets through solved cameras, and saved as a spec the designer opens.",
        "notes": "No texture, hair, eyes or teeth; photos carry no absolute size; likeness is limited by the targets.",
        "steps": [
            {"id": "photos", "title": "Photos", "command": null,
             "help": "A passport-style front (looking straight at the camera, plain background, even light, eyes open, mouth closed) and a true side profile; the other side's profile is optional.",
             "fields": [
                 {"name": "front", "label": "Front photo", "widget": "file_open", "ext": ".png .jpg", "common": true},
                 {"name": "profile", "label": "Profile photo", "widget": "file_open", "ext": ".png .jpg", "common": true},
                 {"name": "profile_side", "label": "The profile shows the face's", "common": true, "bind": "=right", "schema": {"enum": ["right", "left"]}},
                 {"name": "other_profile", "label": "Other profile (optional)", "widget": "file_open", "ext": ".png .jpg"},
                 {"name": "other_profile_side", "label": "The other profile shows the face's", "bind": "=left", "schema": {"enum": ["right", "left"]}},
             ],
             "shows": []},
            {"id": "check", "title": "Check", "command": "photo.check",
             "help": "Head pose and image quality of each photo against calibrated limits; a failed front or profile stops here. A limit can be allowed by name under advanced, and the run records it.",
             "fields": [
                 {"name": "front", "bind": "$photos.input.front", "hidden": true},
                 {"name": "profile", "bind": "$photos.input.profile", "hidden": true},
                 {"name": "profile_side", "bind": "$photos.input.profile_side", "hidden": true},
                 {"name": "run", "bind": "$dir/check", "hidden": true},
                 {"name": "allow", "label": "Allow these failed checks"},
             ],
             "shows": [{"kind": "report"}]},
            {"id": "views", "title": "Landmarks", "command": "photo.views",
             "help": "The 478 face landmarks, the outline and the profile key points of every photo; look at the overlays before trusting anything.",
             "fields": [
                 {"name": "run", "bind": fit, "hidden": true},
                 {"name": "photos", "hidden": true, "bind": [
                     {"name": "front", "path": "$photos.input.front", "role": "front"},
                     {"name": "profile", "path": "$photos.input.profile", "role": "profile", "side": "$photos.input.profile_side"},
                     {"name": "other_profile", "path": "$photos.input.other_profile", "role": "profile", "side": "$photos.input.other_profile_side"},
                 ]},
             ],
             "shows": [{"kind": "images", "from": "$views.result.views", "path": "overlay", "label": "name"}, {"kind": "report"}]},
            {"id": "fit", "title": "Fit", "command": "fit.photos",
             "help": "The targets fitted to every photo's landmarks and outline through solved cameras, with a stay-at-base prior; per photo the landmark and outline errors.",
             "fields": [
                 {"name": "run", "bind": fit, "hidden": true},
                 {"name": "store", "bind": "$run.store", "hidden": true},
                 {"name": "pack", "bind": "$run.pack", "hidden": true},
                 {"name": "catalog", "bind": "$run.catalog", "hidden": true},
                 {"name": "focal_px", "label": "Focal length in pixels (if known)", "common": true},
                 {"name": "base_landmarks", "label": "Base landmark table (detected on the base with eyes)", "widget": "file_open", "ext": ".json", "bind": "$run.base_landmarks", "common": true},
                 {"name": "leave_out", "label": "Cross-check: fit without this view"},
             ],
             "shows": [{"kind": "report"}]},
            {"id": "accept", "title": "Accept", "command": "character.create",
             "help": "The fitted weights saved as a character spec; open it in the designer, where the sliders take over.",
             "fields": [
                 {"name": "store", "bind": "$run.store", "hidden": true},
                 {"name": "pack", "bind": "$run.pack", "hidden": true},
                 {"name": "catalog", "bind": "$run.catalog", "hidden": true},
                 {"name": "game", "bind": "$run.game", "hidden": true},
                 {"name": "weights", "bind": "$fit.result.weights", "hidden": true},
                 {"name": "out", "label": "Spec file", "widget": "file_save", "ext": ".gfchar.json", "bind": "$dir/character.gfchar.json", "common": true},
                 {"name": "name", "label": "Name", "bind": "=from photos", "common": true},
             ],
             "shows": [{"kind": "report"}, {"kind": "spec", "from": "$accept.result.out", "store": "$run.store"}]},
        ],
    })
}

fn export() -> Value {
    json!({
        "id": "export",
        "title": "Export a character",
        "group": "Export",
        "developer": false,
        "summary": "A character spec to a glb (and optionally its finest level as an obj), checked and opened back in the viewport.",
        "steps": [
            {"id": "character", "title": "Character", "command": null,
             "help": "The character to export: the spec file the designer saved (the current character by default) and its store.",
             "fields": [
                 {"name": "store", "label": "Store", "widget": "folder", "bind": "$run.store", "common": true},
                 {"name": "spec", "label": "Character spec", "widget": "file_open", "ext": ".gfchar.json", "bind": "$run.spec", "common": true},
             ],
             "shows": []},
            {"id": "export", "title": "Export", "command": "export.glb",
             "help": "Where to write the glb and what goes in it. Everything under advanced keeps its default.",
             "fields": [
                 {"name": "store", "bind": "$character.input.store", "hidden": true},
                 {"name": "spec", "bind": "$character.input.spec", "hidden": true},
                 {"name": "out", "label": "Glb file", "widget": "file_save", "ext": ".glb", "bind": "$run.out", "common": true},
                 {"name": "lod0_out", "label": "Also an obj of the finest level (file stem)", "widget": "file_save", "ext": "", "common": true},
                 {"name": "morphs", "label": "Morph targets", "widget": "targets"},
                 {"name": "eyes", "label": "Eyes file", "widget": "file_open", "ext": ".json"},
                 {"name": "body", "label": "Body pack", "widget": "folder"},
                 {"name": "body_rig", "label": "Body skeleton (a rig preset: game_engine, game_engine_with_breast, default, mixamo, mixamo_unity, cmu_mb, openpose; or a rig .json)"},
                 {"name": "body_gates", "label": "When the body's stitch gates fail"},
                 {"name": "body_influences", "label": "Joints per body vertex (8 keeps the shoulders whole; 4 for engines that read only four)"},
                 {"name": "format", "label": "File format (glb, or vrm: a VRM 1.0 avatar in a T-pose, needs a body skeleton)"},
                 {"name": "vrm_author", "label": "VRM author (required for vrm)"},
                 {"name": "vrm_license", "label": "VRM licence URL (default: VRM Public License 1.0)"},
                 {"name": "vrm_name", "label": "VRM avatar name (default: the character's name)"},
                 {"name": "bake", "label": "Bake spring-bone physics (sway, or an animation in the file; needs a body skeleton and a wearable with spring bones)"},
                 {"name": "bake_format", "label": "Bake as (bones: joint keys; vertex: one morph target per frame, for engines without skinning beyond the body)"},
                 {"name": "bake_fps", "label": "Baked frames per second"},
                 {"name": "strands", "label": "Also the hair as strands (a USD .usda file of curves, for Blender and other tools)", "widget": "file_save", "ext": ".usda"},
                 {"name": "strands_count", "label": "Strands in all (20000; about 100000 covers a long hair's crown like its cards)"},
                 {"name": "lids", "label": "Eyelid fitting"},
                 {"name": "rig", "label": "Face rig"},
                 {"name": "sparse", "label": "Sparse morphs"},
                 {"name": "lod_screen", "label": "LOD switch screen sizes"},
             ],
             "shows": [{"kind": "report"}, {"kind": "glb", "from": "$export.result.out"}]},
            {"id": "check", "title": "Check", "command": "gates.facing", "optional": true,
             "help": "Every triangle of the exported file faces the way its vertex normals say, in the neutral and in each morph state.",
             "fields": [
                 {"name": "glb", "bind": "$export.result.out", "hidden": true},
                 {"name": "states", "label": "States", "bind": "=all", "common": true},
             ],
             "shows": [{"kind": "report"}]},
        ],
    })
}

pub fn list_input() -> Value {
    obj(json!({"developer": {"type": "boolean", "description": "include the developer wizards (default false)"}}), &[])
}

pub fn list_output() -> Value {
    shape(json!({"wizards": {"type": "array", "description": "id, title, group, developer, summary, steps"}}))
}

pub fn list(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let developer = input.get("developer").and_then(Value::as_bool).unwrap_or(false);
    let wizards: Vec<Value> = definitions()
        .into_iter()
        .filter(|w| developer || w["developer"] != Value::Bool(true))
        .map(|w| json!({"id": w["id"], "title": w["title"], "group": w["group"], "developer": w["developer"], "summary": w["summary"],
                        "steps": w["steps"].as_array().map_or(0, Vec::len)}))
        .collect();
    Ok(json!({"wizards": wizards}))
}

fn definition(id: &str) -> Result<Value, ApiError> {
    definitions().into_iter().find(|w| w["id"] == id).ok_or_else(|| ApiError::not_found(format!("no wizard named {id:?}")).hint("wizards.list names every wizard"))
}

pub fn get_input() -> Value {
    obj(json!({"id": {"type": "string", "minLength": 1, "description": "a wizard id, as wizards.list prints it"}}), &["id"])
}

pub fn get_output() -> Value {
    shape(json!({"id": {"type": "string"}, "steps": {"type": "array", "description": "each step with its command, kind, input schema, fields and what it shows"}}))
}

pub fn get(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let mut w = definition(text(input, "id"))?;
    for step in w["steps"].as_array_mut().into_iter().flatten() {
        if let Some(def) = step["command"].as_str().and_then(find) {
            step["kind"] = json!(def.kind.as_str());
            step["input_schema"] = (def.input)();
            step["summary"] = json!(def.summary);
        }
    }
    Ok(w)
}

fn runs_root(ctx: &Ctx, input: &Value) -> PathBuf {
    let root = input.get("root").and_then(Value::as_str).filter(|r| !r.is_empty()).map_or_else(|| ctx.projects.clone(), PathBuf::from);
    root.join("runs")
}

fn run_dir(ctx: &Ctx, input: &Value) -> Result<PathBuf, ApiError> {
    let run = text(input, "run");
    let ok = match run.split_once('/') {
        Some((project, wizard)) => {
            project::check_name(project, true).is_ok()
                && !wizard.is_empty()
                && wizard.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        }
        None => false,
    };
    if !ok {
        return Err(ApiError::invalid(format!("{run:?} is not a wizard run id (PROJECT/WIZARD-N)")));
    }
    let dir = runs_root(ctx, input).join(run);
    if !dir.join("run.json").is_file() {
        return Err(ApiError::not_found(format!("no wizard run {run}")));
    }
    Ok(dir)
}

pub const UPLOAD_LIMIT: usize = 64 * 1024 * 1024;

pub fn run_file(ctx: &Ctx, run: &str, path: &str) -> Result<PathBuf, ApiError> {
    let dir = run_dir(ctx, &json!({"run": run}))?;
    let inside = std::fs::canonicalize(&dir).map_err(|e| ApiError::internal(e.to_string()))?;
    let file = std::fs::canonicalize(path).map_err(|_| ApiError::not_found(format!("no file {path}")))?;
    if !file.starts_with(&inside) || !file.is_file() {
        return Err(ApiError::not_found(format!("{path} is not a file of wizard run {run}")));
    }
    Ok(file)
}

pub fn upload(ctx: &Ctx, run: &str, name: &str, bytes: &[u8]) -> Result<PathBuf, ApiError> {
    let dir = run_dir(ctx, &json!({"run": run}))?;
    let ok = (1..=128).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && !name.starts_with('.') && !name.ends_with('.') && !project::device_name(name);
    if !ok {
        return Err(ApiError::invalid(format!("upload name {name:?}: use 1 to 128 letters, digits, '-', '_' or '.', not starting or ending with '.', and not a device name such as CON or COM1")));
    }
    if bytes.len() > UPLOAD_LIMIT {
        return Err(ApiError::invalid(format!("upload of {} bytes is over the {UPLOAD_LIMIT}-byte limit", bytes.len())));
    }
    let path = dir.join("uploads").join(name);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| ApiError::internal(format!("{}: {e}", d.display())))?;
    }
    project::write_atomic(&path, bytes)?;
    Ok(path)
}

fn read_json(path: &Path) -> Option<Value> {
    std::fs::read_to_string(path).ok().and_then(|t| crate::exactjson::record(&t).ok())
}

fn write_json(path: &Path, value: &Value) -> Result<(), ApiError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?;
    }
    project::write_atomic(path, crate::text(value).as_bytes())
}

fn run_prop() -> Value {
    json!({"type": "string", "minLength": 3, "description": "the run id wizards.start returned (PROJECT/WIZARD-N)"})
}

fn root_prop() -> Value {
    json!({"type": "string", "minLength": 1, "description": "projects root (default: the server's projects folder); runs live in ROOT/runs"})
}

pub fn start_input() -> Value {
    obj(json!({"wizard": {"type": "string", "minLength": 1}, "project": {"type": "string", "minLength": 1, "description": "the project the run belongs to (default untitled)"},
               "values": {"type": "object", "description": "run values the steps bind as $run.NAME (for example store, spec, out)"}, "root": root_prop()}),
        &["wizard"])
}

pub fn state_input() -> Value {
    obj(json!({"run": run_prop(), "root": root_prop()}), &["run"])
}

pub fn step_input() -> Value {
    obj(json!({"run": run_prop(), "step": {"type": "string", "minLength": 1}, "input": {"type": "object", "description": "the step's fields; hidden and bound fields not given take their bound values"},
               "root": root_prop()}),
        &["run", "step"])
}

pub fn run_output() -> Value {
    shape(json!({"run": {"type": "string"}, "wizard": {"type": "string"}, "dir": {"type": "string", "description": "the run folder, where a client may put the files a step reads or writes"}, "values": {"type": "object"},
                 "steps": {"type": "array", "description": "per step: id, state (open, running, done, failed, stale), the proposed input, the input given, the result or error, the job id"}}))
}

pub fn start(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let w = definition(text(input, "wizard"))?;
    let name = input.get("project").and_then(Value::as_str).unwrap_or(project::UNTITLED);
    project::check_name(name, true)?;
    let base = runs_root(ctx, input).join(name);
    std::fs::create_dir_all(&base).map_err(|e| ApiError::internal(format!("{}: {e}", base.display())))?;
    let id = w["id"].as_str().unwrap_or_default();
    let n = (1..).find(|n| !base.join(format!("{id}-{n}")).exists()).unwrap_or(1);
    let run = format!("{name}/{id}-{n}");
    let dir = base.join(format!("{id}-{n}"));
    write_json(&dir.join("run.json"), &json!({"format": FORMAT, "wizard": id, "run": run, "values": input.get("values").cloned().unwrap_or_else(|| json!({})), "started": project::now_utc()}))?;
    state_of(ctx, &dir)
}

pub fn runs_input() -> Value {
    obj(json!({"project": {"type": "string", "minLength": 1, "description": "the project whose runs to list (default untitled)"},
               "limit": {"type": "integer", "minimum": 1, "maximum": 1000, "description": "at most this many runs (default 20)"}, "root": root_prop()}),
        &[])
}

pub fn runs_output() -> Value {
    shape(json!({"project": {"type": "string"}, "total": {"type": "integer"},
                 "runs": {"type": "array", "description": "per run: run, wizard, title, started, steps (id and state)"}}))
}

pub fn runs(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let name = input.get("project").and_then(Value::as_str).unwrap_or(project::UNTITLED);
    project::check_name(name, true)?;
    let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    let base = runs_root(ctx, input).join(name);
    let mut rows = Vec::new();
    for entry in std::fs::read_dir(&base).into_iter().flatten().flatten() {
        let dir = entry.path();
        let Some(run) = read_json(&dir.join("run.json")) else { continue };
        let Ok(w) = definition(run["wizard"].as_str().unwrap_or_default()) else { continue };
        let Ok(s) = state_of(ctx, &dir) else { continue };
        let steps: Vec<Value> = s["steps"].as_array().into_iter().flatten().map(|x| json!({"id": x["id"], "state": x["state"]})).collect();
        rows.push(json!({"run": run["run"], "wizard": run["wizard"], "title": w["title"], "started": run["started"], "steps": steps}));
    }
    rows.sort_by(|a, b| b["started"].as_str().cmp(&a["started"].as_str()).then_with(|| b["run"].as_str().cmp(&a["run"].as_str())));
    let total = rows.len();
    rows.truncate(limit);
    Ok(json!({"project": name, "total": total, "runs": rows}))
}

pub fn state(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    state_of(ctx, &run_dir(ctx, input)?)
}

fn lookup(path: &str, run: &Value, steps: &Map<String, Value>) -> Option<Value> {
    let mut parts = path.split('.');
    let head = parts.next()?;
    let mut at = if head == "run" { run["values"].clone() } else { steps.get(head)?.clone() };
    for p in parts {
        at = at.get(p)?.clone();
    }
    (!at.is_null()).then_some(at)
}

fn resolve(b: &Value, run: &Value, steps: &Map<String, Value>, dir: &str) -> Option<Value> {
    match b {
        Value::String(s) => {
            if let Some(literal) = s.strip_prefix('=') {
                Some(json!(literal))
            } else if let Some(rest) = s.strip_prefix("$dir") {
                Some(json!(format!("{dir}{rest}")))
            } else if let Some(path) = s.strip_prefix('$') {
                lookup(path, run, steps)
            } else {
                Some(b.clone())
            }
        }
        Value::Array(items) => Some(Value::Array(items.iter().filter_map(|x| resolve(x, run, steps, dir)).collect())),
        Value::Object(m) => m.iter().map(|(k, v)| resolve(v, run, steps, dir).map(|r| (k.clone(), r))).collect::<Option<Map<String, Value>>>().map(Value::Object),
        other => Some(other.clone()),
    }
}

fn bound(field: &Value, run: &Value, steps: &Map<String, Value>, dir: &str) -> Option<Value> {
    field.get("bind").and_then(|b| resolve(b, run, steps, dir))
}

fn step_record(dir: &Path, id: &str) -> Value {
    let at = dir.join("steps").join(id);
    let mut r = json!({"input": read_json(&at.join("input.json")), "job": read_json(&at.join("job.json")).map(|j| j["id"].clone())});
    if let Some(out) = read_json(&at.join("output.json")) {
        r["result"] = out["result"].clone();
        r["error"] = out["error"].clone();
    }
    r["stale"] = json!(at.join("stale").exists());
    r
}

fn finish_job(ctx: &Ctx, dir: &Path, id: &str) -> Result<(), ApiError> {
    let at = dir.join("steps").join(id);
    let Some(job) = read_json(&at.join("job.json")) else { return Ok(()) };
    if at.join("output.json").exists() {
        return Ok(());
    }
    let jctx = Ctx { jobs_dir: PathBuf::from(job["jobs_dir"].as_str().unwrap_or_default()), ..ctx.clone() };
    let record = match jobs::status(&jctx, &json!({"id": job["id"]})) {
        Ok(r) => r,
        Err(e) => return write_json(&at.join("output.json"), &json!({"result": null, "error": e.to_json(), "job_state": "lost"})),
    };
    match record["state"].as_str() {
        Some("done") | Some("failed") | Some("cancelled") => {
            let envelope = record.get("result").cloned().unwrap_or_else(|| json!({"ok": false, "error": record["error"]}));
            write_json(&at.join("output.json"), &json!({"result": envelope.get("result"), "error": envelope.get("error"), "job_state": record["state"]}))
        }
        _ => Ok(()),
    }
}

fn state_of(ctx: &Ctx, dir: &Path) -> Result<Value, ApiError> {
    let run = read_json(&dir.join("run.json")).ok_or_else(|| ApiError::internal(format!("{}: the run record does not read", dir.display())))?;
    let w = definition(run["wizard"].as_str().unwrap_or_default())?;
    let mut done: Map<String, Value> = Map::new();
    let mut rows = Vec::new();
    for step in w["steps"].as_array().into_iter().flatten() {
        let id = step["id"].as_str().unwrap_or_default();
        finish_job(ctx, dir, id)?;
        let mut rec = step_record(dir, id);
        let mut proposed = Map::new();
        for f in step["fields"].as_array().into_iter().flatten() {
            if let Some(v) = bound(f, &run, &done, &dir.display().to_string()) {
                proposed.insert(f["name"].as_str().unwrap_or_default().to_string(), v);
            }
        }
        let st = if rec["stale"] == Value::Bool(true) {
            "stale"
        } else if !rec["error"].is_null() {
            "failed"
        } else if rec.get("result").is_some_and(|r| !r.is_null()) || (step["command"].is_null() && !rec["input"].is_null()) {
            "done"
        } else if !rec["job"].is_null() {
            "running"
        } else {
            "open"
        };
        rec["id"] = json!(id);
        rec["state"] = json!(st);
        rec["proposed"] = Value::Object(proposed);
        done.insert(id.to_string(), json!({"input": rec["input"], "result": rec.get("result").cloned().unwrap_or(Value::Null)}));
        rows.push(rec);
    }
    Ok(json!({"run": run["run"], "wizard": run["wizard"], "dir": dir.display().to_string(), "values": run["values"], "steps": rows}))
}

fn fields_schema(step: &Value) -> Value {
    let props: Map<String, Value> = step["fields"].as_array().into_iter().flatten()
        .map(|f| (f["name"].as_str().unwrap_or_default().to_string(), f.get("schema").cloned().unwrap_or_else(|| json!({"type": "string"}))))
        .collect();
    json!({"type": "object", "additionalProperties": false, "properties": props})
}

pub fn step(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let dir = run_dir(ctx, input)?;
    let run = read_json(&dir.join("run.json")).ok_or_else(|| ApiError::internal("the run record does not read"))?;
    let w = definition(run["wizard"].as_str().unwrap_or_default())?;
    let steps = w["steps"].as_array().cloned().unwrap_or_default();
    let id = text(input, "step");
    let at = steps.iter().position(|s| s["id"] == id).ok_or_else(|| ApiError::not_found(format!("wizard {} has no step {id:?}", run["wizard"].as_str().unwrap_or(""))))?;
    let state = state_of(ctx, &dir)?;
    if let Some(open) = state["steps"].as_array().into_iter().flatten().take(at).find(|s| !matches!(s["state"].as_str(), Some("done")) && !steps.iter().any(|d| d["id"] == s["id"] && d["optional"] == Value::Bool(true))) {
        return Err(ApiError::invalid(format!("step {id} needs step {} first ({})", open["id"].as_str().unwrap_or(""), open["state"].as_str().unwrap_or(""))));
    }
    let def = &steps[at];
    let mut given = input.get("input").and_then(Value::as_object).cloned().unwrap_or_default();
    let proposed = state["steps"][at]["proposed"].as_object().cloned().unwrap_or_default();
    for f in def["fields"].as_array().into_iter().flatten() {
        let name = f["name"].as_str().unwrap_or_default();
        if let Some(v) = proposed.get(name).filter(|_| !given.contains_key(name) || f["hidden"] == Value::Bool(true)) {
            given.insert(name.to_string(), v.clone());
        }
    }
    let given = Value::Object(given);
    let cmd = match def["command"].as_str() {
        Some(name) => {
            let cmd = find(name).ok_or_else(|| ApiError::internal(format!("wizard step {id} names an unknown command {name}")))?;
            check_input(cmd, &given)?;
            Some(cmd)
        }
        None => {
            let problems = crate::schema::validate(&fields_schema(def), &given);
            if !problems.is_empty() {
                return Err(ApiError::invalid(format!("step {id} input: {}", problems.join("; "))).details(json!({"problems": problems})));
            }
            None
        }
    };
    let here = dir.join("steps").join(id);
    if state["steps"][at]["state"] == "running"
        && let Some(job) = read_json(&here.join("job.json"))
    {
        let jctx = Ctx { jobs_dir: PathBuf::from(job["jobs_dir"].as_str().unwrap_or_default()), ..ctx.clone() };
        jobs::cancel(&jctx, &json!({"id": job["id"]}))?;
    }
    std::fs::remove_dir_all(&here).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
        .map_err(|e| ApiError::internal(format!("{}: {e}", here.display())))?;
    for later in &steps[at + 1..] {
        let d = dir.join("steps").join(later["id"].as_str().unwrap_or_default());
        if d.join("input.json").exists() {
            write_json(&d.join("stale"), &json!({"because": id}))?;
        }
    }
    let Some(cmd) = cmd else {
        write_json(&here.join("input.json"), &given)?;
        return state_of(ctx, &dir);
    };
    let name = cmd.name;
    write_json(&here.join("input.json"), &given)?;
    if cmd.kind == Kind::Job {
        let record = jobs::submit(ctx, &json!({"command": name, "input": given}))?;
        write_json(&here.join("job.json"), &json!({"id": record["id"], "jobs_dir": ctx.jobs_dir.display().to_string()}))?;
    } else {
        let outcome = (cmd.run)(ctx, &given);
        let out = match outcome {
            Ok(r) => json!({"result": r, "error": null}),
            Err(e) => json!({"result": null, "error": e.to_json()}),
        };
        write_json(&here.join("output.json"), &out)?;
    }
    state_of(ctx, &dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(b: &Value) -> Vec<String> {
        match b {
            Value::String(s) => s.strip_prefix('$').map(|r| vec![r.to_string()]).unwrap_or_default(),
            Value::Array(a) => a.iter().flat_map(refs).collect(),
            Value::Object(m) => m.values().flat_map(refs).collect(),
            _ => Vec::new(),
        }
    }

    fn props(name: &str) -> Value {
        (find(name).unwrap().input)()["properties"].clone()
    }

    #[test]
    fn body_rig_label_names_every_rig_preset() {
        let defs = serde_json::to_string(&definitions()).unwrap();
        let start = defs.find("Body skeleton (").unwrap();
        let label = &defs[start..start + defs[start..].find(')').unwrap()];
        for name in gf_core::rig_presets::names() {
            assert!(label.contains(&format!(" {name},")) || label.contains(&format!(" {name};")), "{name} missing from: {label}");
        }
    }

    #[test]
    fn every_definition_names_real_commands_fields_and_earlier_bindings() {
        for w in definitions() {
            let mut seen: Vec<String> = Vec::new();
            for step in w["steps"].as_array().unwrap() {
                let id = step["id"].as_str().unwrap();
                let schema = step["command"].as_str().map(|c| {
                    assert!(find(c).is_some(), "{} names {c}", w["id"]);
                    props(c)
                });
                for f in step["fields"].as_array().unwrap() {
                    let name = f["name"].as_str().unwrap();
                    if let Some(p) = &schema {
                        assert!(p.get(name).is_some(), "{}.{id}.{name} is not an input of {}", w["id"], step["command"]);
                    }
                    for b in refs(&f["bind"]) {
                        assert!(!b.starts_with("dir/steps"), "{}.{id}.{name} binds into the step records: {b}", w["id"]);
                        let head = b.split(['.', '/']).next().unwrap();
                        assert!(head == "run" || head == "dir" || seen.iter().any(|s| s == head), "{}.{id}.{name} binds {b} before that step", w["id"]);
                    }
                }
                seen.push(id.to_string());
            }
        }
    }

    #[test]
    fn templates_resolve_drop_items_whose_values_are_missing_and_keep_literals() {
        let run = json!({"values": {"store": "S"}});
        let mut steps = Map::new();
        steps.insert("photos".into(), json!({"input": {"front": "f.png", "profile": "p.png", "side": "right"}}));
        let t = json!([{"path": "$photos.input.front", "role": "front"}, {"path": "$photos.input.other", "role": "profile"}, {"path": "$photos.input.profile", "side": "$photos.input.side"}]);
        assert_eq!(resolve(&t, &run, &steps, "D").unwrap(), json!([{"path": "f.png", "role": "front"}, {"path": "p.png", "side": "right"}]));
        assert_eq!(resolve(&json!("$dir/fit"), &run, &steps, "D").unwrap(), json!("D/fit"));
        assert_eq!(resolve(&json!("=left"), &run, &steps, "D").unwrap(), json!("left"));
        assert_eq!(resolve(&json!("$run.store"), &run, &steps, "D").unwrap(), json!("S"));
        assert!(resolve(&json!("$run.missing"), &run, &steps, "D").is_none());
    }

    #[test]
    fn a_run_steps_through_bound_values_and_marks_later_steps_stale() {
        let root = std::env::temp_dir().join(format!("gf-wizard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ctx = Ctx { jobs_dir: root.join("jobs"), exe: PathBuf::from("x"), job_id: None, projects: root.clone() };
        let s = start(&ctx, &json!({"wizard": "export", "project": "demo", "values": {"store": "S", "spec": "C.gfchar.json", "out": "o.glb"}})).unwrap();
        assert_eq!(s["run"], "demo/export-1");
        assert_eq!(s["steps"][0]["proposed"]["store"], "S");
        assert!(step(&ctx, &json!({"run": "demo/export-1", "step": "export", "input": {}})).unwrap_err().message.contains("needs step character"));
        let s = step(&ctx, &json!({"run": "demo/export-1", "step": "character", "input": {}})).unwrap();
        assert_eq!(s["steps"][0]["state"], "done");
        assert_eq!(s["steps"][1]["proposed"]["spec"], "C.gfchar.json");
        assert_eq!(s["steps"][1]["proposed"]["out"], "o.glb");
        let s = step(&ctx, &json!({"run": "demo/export-1", "step": "character", "input": {"spec": "D.gfchar.json"}})).unwrap();
        assert_eq!(s["steps"][1]["proposed"]["spec"], "D.gfchar.json");
        assert_eq!(s["steps"][2]["proposed"]["states"], "all");
        assert_eq!(start(&ctx, &json!({"wizard": "export", "project": "demo"})).unwrap()["run"], "demo/export-2");
        assert!(state(&ctx, &json!({"run": "../x"})).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn every_text_a_client_shows_starts_with_a_capital() {
        fn walk(v: &Value, at: &str, bad: &mut Vec<String>) {
            match v {
                Value::Object(m) => {
                    for (k, x) in m {
                        if k == "shows" {
                            continue;
                        }
                        if let (true, Some(s)) = (["title", "label", "summary", "help", "notes", "group"].contains(&k.as_str()), x.as_str())
                            && s.chars().next().is_some_and(char::is_lowercase)
                        {
                            bad.push(format!("{at}.{k}: {s}"));
                        }
                        walk(x, &format!("{at}.{k}"), bad);
                    }
                }
                Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| walk(x, &format!("{at}[{i}]"), bad)),
                _ => {}
            }
        }
        let mut bad = Vec::new();
        for w in definitions() {
            walk(&w, w["id"].as_str().unwrap_or("?"), &mut bad);
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }

    #[test]
    fn a_refused_step_keeps_its_record_and_a_lost_job_fails_only_its_step() {
        let root = std::env::temp_dir().join(format!("gf-wizard-refuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ctx = Ctx { jobs_dir: root.join("jobs"), exe: PathBuf::from("x"), job_id: None, projects: root.clone() };
        let s = start(&ctx, &json!({"wizard": "export", "project": "my demo", "values": {"store": "S", "spec": "C.gfchar.json"}})).unwrap();
        let run = s["run"].as_str().unwrap().to_string();
        assert_eq!(run, "my demo/export-1");
        step(&ctx, &json!({"run": run, "step": "character", "input": {}})).unwrap();
        for bad in [json!({"spec": 5}), json!({"bogus": "x"})] {
            assert!(step(&ctx, &json!({"run": run, "step": "character", "input": bad})).is_err());
            let s = state(&ctx, &json!({"run": run})).unwrap();
            assert_eq!(s["steps"][0]["state"], "done");
            assert_eq!(s["steps"][0]["input"]["spec"], "C.gfchar.json");
        }
        write_json(&root.join("runs/my demo/export-1/steps/export/job.json"), &json!({"id": "missing", "jobs_dir": root.join("jobs").display().to_string()})).unwrap();
        let s = state(&ctx, &json!({"run": run})).unwrap();
        assert_eq!(s["steps"][1]["state"], "failed");
        assert_eq!(s["steps"][0]["state"], "done");
        for name in ["CON", "com1.png", "x.", ".x", "a/b"] {
            assert!(upload(&ctx, &run, name, b"x").is_err(), "{name}");
        }
        assert!(upload(&ctx, &run, "front_a.png", b"x").is_ok());
        let listed = runs(&ctx, &json!({"project": "my demo"})).unwrap();
        assert_eq!(listed["runs"][0]["run"], run.as_str());
        assert_eq!(listed["runs"][0]["steps"][1]["state"], "failed");
        assert_eq!(runs(&ctx, &json!({"project": "none here"})).unwrap()["total"], 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}
