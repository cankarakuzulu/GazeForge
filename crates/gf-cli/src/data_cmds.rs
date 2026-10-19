use std::collections::BTreeMap;
use std::path::Path;

use gf_core::character::{FORMAT, Spec, SpecBase};
use gf_core::macros::{self, MacroDefinition, Mode};
use gf_core::placement::{self as gp, PlacementVersion};
use gf_core::residual;
use gf_io::character::{self as ch, MacroSource};
use gf_io::dense;
use gf_io::store::Store;
use serde_json::{Value, json};

use crate::{Options, need, number, only};

const IMPORT_PACK_KEYS: &[&str] = &["store", "dense", "macro-base", "pin-top"];
const IMPORT_CATALOG_KEYS: &[&str] = &["store", "targets", "pack"];
const IMPORT_GAME_KEYS: &[&str] = &["store", "base", "pack"];
const CREATE_KEYS: &[&str] = &[
    "store",
    "pack",
    "catalog",
    "game",
    "placement",
    "name",
    "weights",
    "macro",
    "sliders",
    "macro-mode",
    "head-scale",
    "residual-from",
    "quantum-mm",
    "out",
];
const VALIDATE_KEYS: &[&str] = &["store", "spec", "macro"];
const EVALUATE_KEYS: &[&str] = &["store", "spec", "macro", "out", "precision"];
const WEIGHTS_KEYS: &[&str] = &["macro", "targets", "sliders", "mode", "floor"];
const LABELS_KEYS: &[&str] = &["macro", "slider", "values", "out", "mode"];

pub fn run(group: &str, verb: &str, opts: &Options) -> Result<Value, String> {
    match (group, verb) {
        ("store", "import-pack") => import_pack(opts),
        ("store", "import-catalog") => import_catalog(opts),
        ("store", "import-game") => import_game(opts),
        ("character", "create") => create(opts),
        ("character", "validate") => validate(opts),
        ("character", "evaluate") => evaluate(opts),
        ("macro", "weights") => macro_weights(opts),
        ("macro", "labels") => macro_labels(opts),
        _ => Err(format!("unknown command {group} {verb}; run gazeforge without arguments for the list")),
    }
}

fn json_arg(text: &str, key: &str) -> Result<Value, String> {
    let (source, body) = match text.strip_prefix('@') {
        Some(path) => (path.to_string(), std::fs::read_to_string(path).map_err(|e| format!("--{key} {path}: {e}"))?),
        None => ("inline".to_string(), text.to_string()),
    };
    serde_json::from_str(&body).map_err(|e| format!("--{key} ({source}) is not JSON: {e}"))
}

fn number_map(value: &Value, key: &str) -> Result<BTreeMap<String, f64>, String> {
    let object = value.as_object().ok_or_else(|| format!("--{key} must be a JSON object of numbers"))?;
    object
        .iter()
        .map(|(k, v)| v.as_f64().map(|x| (k.clone(), x)).ok_or_else(|| format!("--{key}: {k} is not a number")))
        .collect()
}

fn open_store(opts: &Options) -> Result<Store, String> {
    Store::open(Path::new(need(opts, "store")?))
}

fn macro_source(opts: &Options) -> Result<Option<MacroSource>, String> {
    opts.get("macro").map(|p| ch::read_macro_definition(Path::new(p))).transpose()
}

fn import_pack(opts: &Options) -> Result<Value, String> {
    only(opts, "store import-pack", &[IMPORT_PACK_KEYS])?;
    let store = open_store(opts)?;
    let macro_base = opts.get("macro-base").map(|t| json_arg(t, "macro-base").and_then(|v| number_map(&v, "macro-base"))).transpose()?;
    let pin_top = opts.get("pin-top").map(|_| number(opts, "pin-top", 0.0)).transpose()?;
    let (reference, manifest) = store.import_pack(Path::new(need(opts, "dense")?), macro_base, pin_top)?;
    Ok(json!({"ok": true, "command": "store import-pack", "pack": reference, "manifest": manifest}))
}

fn import_catalog(opts: &Options) -> Result<Value, String> {
    only(opts, "store import-catalog", &[IMPORT_CATALOG_KEYS])?;
    let store = open_store(opts)?;
    let (reference, manifest, bytes) = store.import_catalog(Path::new(need(opts, "targets")?), need(opts, "pack")?)?;
    Ok(json!({"ok": true, "command": "store import-catalog", "catalog": reference, "targets": manifest.targets.len(),
              "not_on_head": manifest.not_on_head.len(), "bytes": bytes}))
}

fn import_game(opts: &Options) -> Result<Value, String> {
    only(opts, "store import-game", &[IMPORT_GAME_KEYS])?;
    let store = open_store(opts)?;
    let (reference, manifest) = store.import_game(Path::new(need(opts, "base")?), need(opts, "pack")?)?;
    Ok(json!({"ok": true, "command": "store import-game", "game_base": reference, "vertices": manifest.vertices,
              "triangles": manifest.triangles, "lods": manifest.lods}))
}

fn create(opts: &Options) -> Result<Value, String> {
    only(opts, "character create", &[CREATE_KEYS])?;
    let store = open_store(opts)?;
    let placement = PlacementVersion::parse(opts.get("placement").map(String::as_str).unwrap_or(&gp::DEFAULT.to_string()))?;
    let weights = match opts.get("weights") {
        Some(t) => number_map(&json_arg(t, "weights")?, "weights")?,
        None => BTreeMap::new(),
    };
    let source = macro_source(opts)?;
    let macro_only = ["sliders", "macro-mode", "head-scale"];
    if source.is_none() {
        if let Some(k) = macro_only.iter().find(|k| opts.contains_key(**k)) {
            return Err(format!("--{k} needs --macro (the macro definition file of the user's install)"));
        }
    }
    let pack_ref = need(opts, "pack")?.to_string();
    let mut spec = Spec {
        format: FORMAT.into(),
        name: opts.get("name").cloned().unwrap_or_else(|| "character".into()),
        base: SpecBase {
            pack: pack_ref.clone(),
            catalog: need(opts, "catalog")?.to_string(),
            game_base: opts.get("game").cloned(),
            placement: placement.to_string(),
        },
        weights,
        macros: None,
        residual: None,
    };
    if let Some(source) = &source {
        let (pack, _) = store.load_pack(&pack_ref)?;
        let base = pack.macro_base.ok_or("--macro needs a pack imported with --macro-base")?;
        let given = match opts.get("sliders") {
            Some(t) => number_map(&json_arg(t, "sliders")?, "sliders")?,
            None => BTreeMap::new(),
        };
        let mode = Mode::parse(opts.get("macro-mode").map(String::as_str).unwrap_or("gf"))?;
        spec.macros = Some(ch::macro_spec(source, mode.name(), &given, &base, number(opts, "head-scale", 1.0)?));
    }
    spec = spec.canonical();
    let (problems, resolved) = ch::problems(&store, &spec, source.as_ref());
    if !problems.is_empty() {
        return Err(format!("the spec does not validate: {}", problems.join("; ")));
    }
    let mut residual_report = Value::Null;
    if let Some(path) = opts.get("residual-from") {
        let resolved = resolved.expect("validated specs resolve");
        let target = dense::read_points(Path::new(path))?;
        let (parametric, _) = ch::parametric(&store, &resolved, &spec, source.as_ref())?;
        let quantum = number(opts, "quantum-mm", residual::DEFAULT_QUANTUM_M * 1000.0)? / 1000.0;
        let (reference, report) = ch::capture_residual(&store, &parametric, &target, quantum)?;
        spec.residual = Some(reference);
        residual_report = report;
    } else if opts.contains_key("quantum-mm") {
        return Err("--quantum-mm needs --residual-from".into());
    }
    let out = Path::new(need(opts, "out")?);
    let revision = ch::write_spec(out, &spec)?;
    Ok(json!({"ok": true, "command": "character create", "out": out.display().to_string(), "revision": revision,
              "weights": spec.weights.len(), "macros": spec.macros.is_some(), "residual": residual_report}))
}

fn validate(opts: &Options) -> Result<Value, String> {
    only(opts, "character validate", &[VALIDATE_KEYS])?;
    let store = open_store(opts)?;
    let spec = ch::read_spec(Path::new(need(opts, "spec")?))?;
    let source = macro_source(opts)?;
    let (problems, _) = ch::problems(&store, &spec, source.as_ref());
    if !problems.is_empty() {
        return Err(format!("{} problems: {}", problems.len(), problems.join("; ")));
    }
    Ok(json!({"ok": true, "command": "character validate", "revision": spec.revision(), "canonical": spec.canonical() == spec}))
}

fn evaluate(opts: &Options) -> Result<Value, String> {
    only(opts, "character evaluate", &[EVALUATE_KEYS])?;
    let store = open_store(opts)?;
    let spec = ch::read_spec(Path::new(need(opts, "spec")?))?;
    let source = macro_source(opts)?;
    let out = Path::new(need(opts, "out")?);
    let precision = opts.get("precision").map(String::as_str).unwrap_or("f32");
    if precision != "f32" && precision != "f64" {
        return Err(format!("--precision must be f32 or f64, got {precision}"));
    }
    let (positions, report) = ch::evaluate(&store, &spec, source.as_ref())?;
    if precision == "f32" {
        dense::write_points(out, &positions)?;
    } else {
        dense::write_points_f64(out, &positions)?;
    }
    Ok(json!({"ok": true, "command": "character evaluate", "out": out.display().to_string(), "revision": spec.revision(),
              "vertices": positions.len(), "precision": precision, "report": report}))
}

fn definition(opts: &Options) -> Result<(MacroDefinition, String), String> {
    let source = ch::read_macro_definition(Path::new(need(opts, "macro")?))?;
    Ok((source.definition, source.reference))
}

fn macro_weights(opts: &Options) -> Result<Value, String> {
    only(opts, "macro weights", &[WEIGHTS_KEYS])?;
    let (def, reference) = definition(opts)?;
    let mode = Mode::parse(opts.get("mode").map(String::as_str).unwrap_or("gf"))?;
    let floor = match (mode, opts.get("floor").map(String::as_str)) {
        (Mode::Gf, Some(_)) => return Err("--floor applies to --mode mpfb only".into()),
        (Mode::Gf, None) | (Mode::Mpfb, Some("none")) => None,
        (Mode::Mpfb, None) => Some(macros::KEY_FLOOR),
        (Mode::Mpfb, Some(_)) => Some(number(opts, "floor", macros::KEY_FLOOR)?),
    };
    let files = opts.get("targets").map(|d| gf_io::macro_target_files(Path::new(d))).transpose()?;
    if files.as_ref().is_some_and(|f| f.is_empty()) {
        return Err(format!("no macro target files under {}", opts["targets"]));
    }
    let given = json_arg(need(opts, "sliders")?, "sliders")?;
    let sets: Vec<&Value> = match &given {
        Value::Array(list) => list.iter().collect(),
        other => vec![other],
    };
    let exists = |n: &str| files.as_ref().is_none_or(|f| f.contains(n));
    let mut results = Vec::new();
    for set in sets {
        let sliders = number_map(set, "sliders")?;
        let mut weights = macros::target_weights(&def, &sliders, Some(&exists), mode)?;
        if let Some(f) = floor {
            weights = macros::mpfb_keys(&weights, f);
        }
        results.push(json!({"sliders": sliders, "weights": weights}));
    }
    Ok(json!({"ok": true, "command": "macro weights", "mode": mode.name(), "floor": floor, "definition": reference,
              "target_files": files.as_ref().map(|f| f.len()), "results": results}))
}

fn macro_labels(opts: &Options) -> Result<Value, String> {
    only(opts, "macro labels", &[LABELS_KEYS])?;
    let (def, reference) = definition(opts)?;
    let mode = Mode::parse(opts.get("mode").map(String::as_str).unwrap_or("gf"))?;
    let slider = need(opts, "slider")?;
    if !macros::MACRO_SLIDERS.contains(&slider) {
        return Err(format!("--slider must be one of {}, got {slider}", macros::MACRO_SLIDERS.join(", ")));
    }
    let labels = def.labels_of(slider);
    let values = gf_io::read_values(Path::new(need(opts, "values")?))?;
    let mut table = Vec::with_capacity(values.len() * labels.len());
    for &v in &values {
        if !v.is_finite() {
            return Err(format!("--values holds a non-finite value {v}"));
        }
        let row = macros::slider_labels(&def, slider, v, mode)?;
        table.extend(labels.iter().map(|l| row.get(l).copied().unwrap_or(0.0)));
    }
    let out = Path::new(need(opts, "out")?);
    gf_io::write_table(out, values.len(), labels.len(), table)?;
    Ok(json!({"ok": true, "command": "macro labels", "mode": mode.name(), "definition": reference, "slider": slider,
              "labels": labels, "values": values.len(), "out": out.display().to_string()}))
}
