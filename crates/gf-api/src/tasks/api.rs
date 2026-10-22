use serde_json::{Map, Value, json};

use super::data::{IMPORT_CATALOG_KEYS, IMPORT_EYERIG_KEYS, IMPORT_GAME_KEYS, IMPORT_PACK_KEYS, LABELS_KEYS, WEIGHTS_KEYS};
use super::mesh::{BODY_KEYS, BODY_RIG_KEYS, GAME_KEYS, GLB_KEYS, LID_EXPORT_KEYS, LIDS_GATE_KEYS, PATH_KEYS, SHELL_KEYS, VRM_KEYS};
use super::topology::{IMPROVE_KEYS, INSPECT_KEYS};
use super::{BUILD_KEYS, COLLIDE_KEYS, EXPORT_KEYS, F2_KEYS, F3_KEYS, F4_KEYS, F5_KEYS, F6_KEYS, FIT_KEYS, OPTIMAL_KEYS, Options, data, mesh, rig, topology};
use crate::commands::{obj, shape};
use crate::{ApiError, Code, Ctx};

const PLACEMENT_KEYS: &[&[&str]] = &[OPTIMAL_KEYS, FIT_KEYS, F2_KEYS, F3_KEYS, F4_KEYS, F5_KEYS, F6_KEYS];

const INTEGERS: &[&str] = &[
    "target",
    "tip-vertex",
    "fit-iterations",
    "fit-guard-steps",
    "f2-repair-rounds",
    "f2-uv-ring",
    "f3-rounds",
    "f3-relax-steps",
    "f3-uv-ring",
    "f3-eye-rounds",
    "f3-eye-samples",
    "f4-passes",
    "f5-uv-ring",
    "f6-sliver-rounds",
    "smooth",
    "fold-tolerance",
    "inner-tolerance",
    "rough-tolerance",
    "steps",
    "pop-limit",
    "lids-smooth",
    "rig-rounds",
    "samples",
    "seed",
    "rounds",
    "strands-count",
    "strands-seed",
    "strands-points",
];

const NUMBERS: &[&str] = &[
    "min-angle",
    "max-angle",
    "max-turn",
    "boundary-weight",
    "improve-below",
    "flip-flat",
    "max-move",
    "fit-smooth",
    "fit-min-angle",
    "fit-repair-below",
    "fit-trim",
    "f2-regular",
    "f2-max-turn",
    "f2-turn-margin",
    "f2-confident-spread",
    "f2-confident-above",
    "f2-angle-margin",
    "f2-repair-max-step",
    "f2-repair-max-step-confident",
    "bake-fps",
    "f3-regular",
    "f3-max-move",
    "f3-smooth",
    "f3-relax-step",
    "f3-backoff-step",
    "f3-eye-front",
    "f3-eye-reach",
    "f3-eye-clearance",
    "f4-target",
    "f4-step",
    "f4-gain",
    "f4-fold",
    "f5-tangential",
    "f5-smooth",
    "f5-membrane",
    "f5-max-move",
    "f5-eye-front",
    "f5-eye-clearance",
    "f6-sliver-deg",
    "f6-sliver-relax",
    "f6-sliver-seam",
    "clearance",
    "front-deg",
    "pin-top",
    "head-scale",
    "rough-mm",
    "rough-rel",
    "seam-limit-mm",
    "tolerance-mm",
    "lod-pixels",
    "lod-height",
    "morph-floor",
    "lids-step-mm",
    "lids-max-mm",
    "lids-front-deg",
    "lids-tolerance-mm",
    "body-blend-mm",
    "body-neck-offset-mm",
    "body-scale",
    "body-smooth",
    "body-tangent",
    "seam-weight",
    "below",
    "min-gain",
    "max-fold",
    "max-spread",
];

const ANY: &[&str] = &["macro-base", "sliders", "floor"];

const ENUMS: &[(&str, &[&str])] = &[
    ("f2-uvs", &["surface", "subset"]),
    ("f3-eye-lift", &["radial", "gaze"]),
    ("f4-backoff", &["dense", "subset"]),
    ("f4-plane", &["game", "dense"]),
    ("f5-normals", &["subset", "dense"]),
    ("sparse", &["auto", "always", "never"]),
    ("mode", &["gf", "mpfb"]),
    ("cells", &["on", "off"]),
    ("states", &["neutral", "all"]),
    ("precision", &["f32", "f64"]),
    ("shape-targets", &["all", "head", "none"]),
    ("levels", &["all", "finest"]),
    ("quads", &["on", "off"]),
    ("body-gates", &["refuse", "report"]),
    ("body-influences", &["8", "4"]),
    ("format", &["glb", "vrm"]),
    ("frame", &["world", "canonical"]),
    ("hide-under", &["on", "off"]),
    ("scale", &["none", "bone"]),
    ("bake-format", &["bones", "vertex"]),
];

const TEXT: &[(&str, &str)] = &[
    ("store", "path of the store directory"),
    ("dense", "path of the dense base directory"),
    ("base", "path of the game base directory"),
    ("out", "path of the file or directory to write"),
    ("positions", "dense positions: base, shape:NAME or a .npy file"),
    ("before", "dense positions before: base, shape:NAME or a .npy file"),
    ("after", "dense positions after: base, shape:NAME or a .npy file"),
    ("face", "dense positions of the face: base, shape:NAME or a .npy file"),
    ("start", "dense positions the path starts from: base, shape:NAME or a .npy file"),
    ("delta", ".npy file of the dense change walked along the path"),
    ("eyes", "path of an eyes .json file"),
    ("placement", "placement name (f5, f6, subset, optimal, fit, f2, f3, f4) or NAME/VERSION; the options it reads follow it"),
    ("spec", "path of the character spec .json"),
    ("macro", "path of the MakeHuman/MPFB macro definition .json"),
    ("pack", "pack reference, sha256:<hex>, as store.list prints it"),
    ("catalog", "catalog reference, sha256:<hex>, as store.list prints it"),
    ("targets", "path of the targets directory"),
    ("makehuman", "path of the MakeHuman data directory"),
    ("macro-base", "macro base sliders: a JSON object, or a JSON text, or @ and a path"),
    ("sliders", "macro sliders: a JSON object, or a JSON text, or @ and a path"),
    ("floor", "weights below this are dropped: a number, or \"none\""),
    ("slider", "macro slider name"),
    ("values", ".npy file of slider values"),
    ("shapes", "shape keys carried into the game base: all, none or a,b"),
    ("extra", "extra dense shapes carried into the game base: A.npy,B.npy"),
    ("weights", ".npy file, per dense vertex, > 0, multiplying the collapse cost"),
    ("lods", "coarser levels of detail, nested: 8000,3000"),
    ("lod-screen", "screen fractions at which the levels switch: 0.25,0.1"),
    ("morphs", "morph target ids: ID,ID or @ and a .json path"),
    ("obj", "path of the game .obj"),
    ("glb", "path of a .glb"),
    ("gates", "gates to run: triq,uv,census,seams"),
    ("plant", "a planted forward jump STEP,GAME_VERTEX,MM, to prove the gate"),
    ("f3-relax-angle", "pair: a,b"),
    ("f3-relax-eye", "pair: a,b"),
    ("f3-backoff-turn", "pair: a,b"),
    ("f3-eye-depth", "pair: a,b"),
    ("f5-eye-depth", "pair: a,b"),
    ("f6-sliver-eye", "pair: a,b"),
    ("game", "game base reference, sha256:<hex>, as store.list prints it"),
    ("frame", "world: the garment was modelled on the character as exported or fitted (its root transform applied); canonical: on the stored body frame"),
    ("hide-under", "on: list the body vertices the garment covers (a line along each body normal meets it within 15 cm), so hide_body can leave them out"),
    ("compare-hidden", "a MakeHuman .mhclo whose hidden list to compare with, on the body vertices"),
    ("out-mhclo", "also write the binding to this file in MakeHuman's format"),
    ("diffuse", "path of the garment's colour texture (.png)"),
    ("bone", "the bone a rigid item rides (a bone of the rig, e.g. hand_r, head, spine_03)"),
    ("rig", "the rig the bone belongs to: a preset (game_engine, ...) or a MakeHuman rig file"),
    ("scale", "none: the item keeps its modelled size; bone: it scales with the bone's length"),
    ("name", "name shown for the wearable"),
    ("seam-weight", "multiplies the collapse cost of the base's open boundary (the neck seam the body stitches to), so coarse levels keep more seam points; 1 leaves it as the weights say"),
    ("body-rig", "skeleton for the spec's body: a rig preset (game_engine, game_engine_with_breast, default, mixamo, mixamo_unity, cmu_mb, openpose) or the path of a MakeHuman rig .json"),
    ("body-weights", "path of the MakeHuman weights .json that goes with a --body-rig path (a preset carries its own)"),
    ("body-gates", "refuse: a body whose stitch gates fail is not exported; report: it is exported and the failures go in the sidecar"),
    ("format", "glb (the default) or vrm: a VRM 1.0 avatar - needs --body-rig with a humanoid map and --vrm-author; the finest level only, re-posed to a T-pose (arms level along X, the rest as exported), with VRMC_vrm meta, humanoid bones and expression presets from the face morphs"),
    ("vrm-author", "the VRM avatar's author (required by VRM's meta)"),
    ("bake", "bake the spring-bone chains and the cloth offline against a clip and write the result as an animation named CLIP_baked: sway (built in: 4 s of head turns, a torso lean and a sideways sway with a bob, driving the bones the chains hang from) or the name of an animation already in the glb"),
    ("bake-format", "bones (the default): the chain joints get rotation keys beside the clip's channels; vertex: the chain joints are taken out of the skin and the swing becomes one morph target per frame played by a weights track, for engines with no skinning beyond the body"),
    ("strands", "also write the hair the character wears as strands, a USD text file (.usda) of BasisCurves in the glb's frame: each hair card's texture columns traced along the card where the texture is opaque, rooted at the end nearer the body"),
    ("strands-count", "how many strands in all (default 20000), shared out by each hair's opaque card area"),
    ("strands-seed", "seed of where the strands are taken on the cards (default 1)"),
    ("strands-points", "points per strand (default 16, 3 to 64)"),
    ("bake-fps","frames per second of the baked keys (default 30, 1 to 60); the solver steps at 60 Hz or finer"),
    ("vrm-license", "the VRM avatar's licence URL (default: the VRM Public License 1.0, https://vrm.dev/licenses/1.0/)"),
    ("vrm-name", "the VRM avatar's name (default: the spec's name)"),
    ("body-influences", "joints per vertex with --body-rig: 8 (the default; JOINTS_1 / WEIGHTS_1 carry the second four) keeps MakeHuman's weights whole, 4 fits engines that read only one set but cuts weight the shoulders need (the default preset's shoulders tear with arms raised)"),
    ("shape-targets", "catalog targets scored one at a time: all, head (designer categories, expressions and macro keys) or none"),
    ("samples", "random combinations of head targets (and expressions) scored as well"),
    ("seed", "seed of the random combinations"),
    ("below", "only edges of triangles whose worst angle over the shape range is below this many degrees are tried"),
    ("min-gain", "a flip must raise the worst angle of its two triangles by at least this many degrees"),
    ("max-fold", "degrees: in every scored shape, each new triangle's normal stays within this of the old pair's normal"),
    ("max-spread", "degrees: an edge whose four corners' rest surface normals (the dense base's, angle-weighted) differ by more than this sits on a crease and keeps its cut"),
    ("rounds", "most flip rounds"),
    ("levels", "all: the levels of detail are improved too and the new base names the new levels; finest: the old levels are kept"),
    ("quads", "on: write clean triangle pairs as quads (planar, convex, not across a seam), paired on the base's rest shape"),
];

fn field(key: &str) -> String {
    key.replace('-', "_")
}

fn property(key: &str) -> Value {
    let argv = format!("argv --{key}");
    let text = TEXT.iter().find(|(k, _)| *k == key).map_or(argv.clone(), |(_, t)| format!("{t} ({argv})"));
    if INTEGERS.contains(&key) {
        return json!({"type": "integer", "minimum": 0, "description": text});
    }
    if NUMBERS.contains(&key) {
        return json!({"type": "number", "minimum": 0, "description": text});
    }
    if ANY.contains(&key) {
        return json!({"description": text});
    }
    if let Some((_, options)) = ENUMS.iter().find(|(k, _)| *k == key) {
        return json!({"enum": options, "description": text});
    }
    json!({"type": "string", "minLength": 1, "description": text})
}

fn schema(sets: &[&[&str]], required: &[&str]) -> Value {
    let mut properties = Map::new();
    for key in sets.iter().flat_map(|set| set.iter()) {
        properties.entry(field(key)).or_insert_with(|| property(key));
    }
    let required: Vec<String> = required.iter().map(|k| field(k)).collect();
    let required: Vec<&str> = required.iter().map(String::as_str).collect();
    obj(Value::Object(properties), &required)
}

pub fn options(input: &Value) -> Options {
    let mut out = Options::new();
    for (key, value) in input.as_object().into_iter().flatten() {
        let text = match value {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            other => other.to_string(),
        };
        out.insert(key.replace('_', "-"), text);
    }
    out
}

fn run(input: &Value, task: impl FnOnce(&Options) -> Result<Value, String>) -> Result<Value, ApiError> {
    let value = task(&options(input))
        .map_err(|e| ApiError::invalid(e).hint("the fields are the argv options with _ for -; commands.list shows them"))?;
    if value.get("ok") == Some(&Value::Bool(false)) {
        let message = value.get("error").and_then(Value::as_str).unwrap_or("the command reported ok false").to_string();
        return Err(ApiError::new(Code::GateFailed, message).details(value));
    }
    Ok(value)
}

pub fn argv_output() -> Value {
    shape(json!({"ok": {"type": "boolean"}, "command": {"type": "string", "description": "the argv command this ran"}}))
}

pub fn import_pack_input() -> Value {
    schema(&[IMPORT_PACK_KEYS], &["store", "dense"])
}

pub fn import_pack(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, |o| data::run("store", "import-pack", o))
}

pub fn import_catalog_input() -> Value {
    schema(&[IMPORT_CATALOG_KEYS], &["store", "targets", "pack"])
}

pub fn import_catalog(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, |o| data::run("store", "import-catalog", o))
}

pub fn import_game_input() -> Value {
    schema(&[IMPORT_GAME_KEYS], &["store", "base", "pack"])
}

pub fn import_game(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, |o| data::run("store", "import-game", o))
}

pub fn import_eyerig_input() -> Value {
    schema(&[IMPORT_EYERIG_KEYS], &["store", "catalog", "makehuman"])
}

pub fn import_eyerig(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, |o| data::run("store", "import-eyerig", o))
}

pub fn build_input() -> Value {
    schema(&[BUILD_KEYS], &["dense", "out"])
}

pub fn build(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, super::build_base)
}

pub fn derive_input() -> Value {
    schema(&[super::DERIVE_KEYS], &["store", "game", "dense", "out", "lods"])
}

pub fn derive(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, super::derive_lods)
}

pub fn bind_input() -> Value {
    let mut s = schema(&[super::wearable::BIND_KEYS], &["store", "spec", "obj", "base"]);
    s["properties"]["obj"]["description"] = json!("path of the garment .obj, modelled over the character the spec describes (argv --obj)");
    s["properties"]["base"]["description"] = json!("path of MakeHuman's base.obj (its body group is the surface the garment is bound to) (argv --base)");
    s
}

pub fn bind(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, super::wearable::bind)
}

pub fn attach_input() -> Value {
    let mut s = schema(&[super::wearable::ATTACH_KEYS], &["store", "spec", "obj", "bone"]);
    s["properties"]["obj"]["description"] = json!("path of the solid's .obj, modelled on the character the spec describes (argv --obj)");
    s
}

pub fn attach(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, super::wearable::attach)
}

pub fn bake_input() -> Value {
    schema(&[super::bake::RUN_KEYS], &["glb", "out", "bake"])
}

pub fn bake(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, super::bake::run)
}

pub fn obj_input() -> Value {
    let mut sets = vec![EXPORT_KEYS];
    sets.extend_from_slice(PLACEMENT_KEYS);
    schema(&sets, &["dense", "base", "positions", "out"])
}

pub fn export_obj(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, super::export_obj)
}

pub fn glb_input() -> Value {
    let mut sets = vec![GLB_KEYS, LID_EXPORT_KEYS, BODY_KEYS, BODY_RIG_KEYS, VRM_KEYS, super::bake::KEYS, super::strands::KEYS];
    sets.extend_from_slice(PLACEMENT_KEYS);
    schema(&sets, &["store", "spec", "out"])
}

pub fn export_glb(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, mesh::export_glb)
}

pub fn collide_input() -> Value {
    schema(&[COLLIDE_KEYS], &["dense", "positions", "eyes", "out"])
}

pub fn collide(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, super::collide_lids)
}

pub fn weights_input() -> Value {
    schema(&[WEIGHTS_KEYS], &["macro", "sliders"])
}

pub fn weights(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, |o| data::run("macro", "weights", o))
}

pub fn labels_input() -> Value {
    schema(&[LABELS_KEYS], &["macro", "slider", "values", "out"])
}

pub fn labels(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, |o| data::run("macro", "labels", o))
}

pub fn shell_input() -> Value {
    schema(&[SHELL_KEYS], &["dense", "before", "after"])
}

pub fn shell(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, mesh::gates_shell)
}

pub fn game_input() -> Value {
    schema(&[GAME_KEYS], &["dense", "base", "face", "obj"])
}

pub fn game(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, mesh::gates_game)
}

pub fn path_input() -> Value {
    let mut sets = vec![PATH_KEYS];
    sets.extend_from_slice(PLACEMENT_KEYS);
    schema(&sets, &["dense", "base", "start", "delta"])
}

pub fn path(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, mesh::gates_path)
}

pub fn facing_input() -> Value {
    schema(&[&["glb", "states"]], &["glb"])
}

pub fn facing(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, mesh::gates_facing)
}

pub fn lids_input() -> Value {
    schema(&[LIDS_GATE_KEYS], &["glb", "eyes"])
}

pub fn lids(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, mesh::gates_lids)
}

pub fn topology_improve_input() -> Value {
    schema(&[IMPROVE_KEYS], &["store", "game"])
}

pub fn topology_improve(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, topology::improve)
}

pub fn topology_inspect_input() -> Value {
    schema(&[INSPECT_KEYS], &["store", "game"])
}

pub fn topology_inspect(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, topology::inspect)
}

pub fn rig_input() -> Value {
    schema(&[&["glb", "eyes", "front-deg", "tolerance-mm"]], &["glb"])
}

pub fn rig(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, rig::gates_rig)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_input() -> Vec<Value> {
        vec![
            import_pack_input(),
            import_catalog_input(),
            import_game_input(),
            import_eyerig_input(),
            build_input(),
            obj_input(),
            glb_input(),
            collide_input(),
            weights_input(),
            labels_input(),
            shell_input(),
            game_input(),
            path_input(),
            facing_input(),
            lids_input(),
            rig_input(),
            topology_improve_input(),
            topology_inspect_input(),
        ]
    }

    #[test]
    fn body_rig_help_names_every_rig_preset() {
        let help = TEXT.iter().find(|(k, _)| *k == "body-rig").unwrap().1;
        for name in gf_core::rig_presets::names() {
            assert!(help.contains(&format!(" {name},")) || help.contains(&format!(" {name})")) || help.contains(&format!("({name},")), "{name} missing from: {help}");
        }
    }

    #[test]
    fn fields_map_back_to_the_argv_options() {
        let o = options(&json!({"min_angle": 4.5, "target": 20000, "f2_uvs": "subset", "macro_base": {"a": 1}, "lods": "8000,3000"}));
        assert_eq!(o["min-angle"], "4.5");
        assert_eq!(o["target"], "20000");
        assert_eq!(o["f2-uvs"], "subset");
        assert_eq!(o["macro-base"], "{\"a\":1}");
        assert_eq!(o["lods"], "8000,3000");
    }

    #[test]
    fn numbers_survive_the_round_trip_through_text() {
        for x in [0.01f64, 1e-3, 0.1 + 0.2, 4.0, 1e-7, 123456.789, 0.05] {
            let o = options(&json!({"f": x}));
            assert_eq!(o["f"].parse::<f64>().unwrap().to_bits(), x.to_bits(), "{x}");
        }
    }

    #[test]
    fn every_option_key_has_a_typed_field_and_no_argv_key_holds_an_underscore() {
        for s in every_input() {
            for (name, p) in s["properties"].as_object().unwrap() {
                assert!(!name.contains('-'), "{name}");
                assert!(p.get("description").is_some(), "{name}");
            }
            for r in s["required"].as_array().unwrap() {
                assert!(s["properties"].get(r.as_str().unwrap()).is_some(), "{r}");
            }
        }
        for key in INTEGERS.iter().chain(NUMBERS).chain(ANY) {
            assert!(!key.contains('_'));
        }
    }

    #[test]
    fn a_report_with_ok_false_becomes_gate_failed_with_the_report() {
        let e = run(&json!({}), |_| Ok(json!({"ok": false, "error": "gates failed: triq"}))).unwrap_err();
        assert_eq!(e.code, Code::GateFailed);
        assert_eq!(e.message, "gates failed: triq");
        assert_eq!(e.details["ok"], false);
        assert_eq!(run(&json!({}), |_| Ok(json!({"ok": true}))).unwrap(), json!({"ok": true}));
    }
}
