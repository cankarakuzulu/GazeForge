use serde_json::{Map, Value, json};

use super::data::{IMPORT_CATALOG_KEYS, IMPORT_EYERIG_KEYS, IMPORT_GAME_KEYS, IMPORT_PACK_KEYS, LABELS_KEYS, WEIGHTS_KEYS};
use super::mesh::{BODY_KEYS, GAME_KEYS, GLB_KEYS, LID_EXPORT_KEYS, LIDS_GATE_KEYS, PATH_KEYS, SHELL_KEYS};
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

pub fn obj_input() -> Value {
    let mut sets = vec![EXPORT_KEYS];
    sets.extend_from_slice(PLACEMENT_KEYS);
    schema(&sets, &["dense", "base", "positions", "out"])
}

pub fn export_obj(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    run(input, super::export_obj)
}

pub fn glb_input() -> Value {
    let mut sets = vec![GLB_KEYS, LID_EXPORT_KEYS, BODY_KEYS];
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
