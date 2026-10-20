use std::collections::BTreeMap;
use std::path::Path;

use gf_core::catalog::{BlobStore, CatalogManifest, PackManifest};
use gf_core::character::{MacroSpec, ResidualRef, Spec};
use gf_core::eyerig::{self, MacroTrace, PlacedEye};
use gf_core::hash;
use gf_geom::lids::Eye;
use gf_core::head::{self, HeadOnly};
use gf_core::macros::{MacroDefinition, Mode};
use gf_core::residual;
use serde_json::{Value, json};

use crate::dense::Dense;
use crate::store::Store;

pub struct MacroSource {
    pub definition: MacroDefinition,
    pub reference: String,
}

pub fn read_macro_definition(path: &Path) -> Result<MacroSource, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let definition = MacroDefinition::from_json(&value).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(MacroSource { definition, reference: hash::reference(&hash::sha256_hex(&bytes)) })
}

pub fn read_spec(path: &Path) -> Result<Spec, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write_spec(path: &Path, spec: &Spec) -> Result<String, String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, spec.canonical_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(spec.revision())
}

pub struct Resolved {
    pub pack: PackManifest,
    pub dense: Dense,
    pub catalog: CatalogManifest,
}

pub fn problems(store: &Store, spec: &Spec, macro_source: Option<&MacroSource>) -> (Vec<String>, Option<Resolved>) {
    let mut out = spec.problems();
    if !out.is_empty() {
        return (out, None);
    }
    let (pack, dense) = match store.load_pack(&spec.base.pack) {
        Ok(p) => p,
        Err(e) => return (vec![format!("base.pack: {e}")], None),
    };
    let catalog = match store.load_catalog(&spec.base.catalog) {
        Ok(c) => c,
        Err(e) => return (vec![format!("base.catalog: {e}")], None),
    };
    if catalog.pack != spec.base.pack {
        out.push(format!("base.catalog was imported for pack {}, the spec names {}", catalog.pack, spec.base.pack));
    }
    if let Some(game) = &spec.base.game_base {
        match store.load_game(game) {
            Ok((g, _)) if g.pack != spec.base.pack => out.push(format!("base.game_base belongs to pack {}", g.pack)),
            Ok(_) => {}
            Err(e) => out.push(format!("base.game_base: {e}")),
        }
    }
    for id in spec.weights.keys() {
        if !catalog.targets.contains_key(id) {
            out.push(format!("weight {id} is not a target of the catalog"));
        }
    }
    if let Some(m) = &spec.macros {
        if pack.macro_base.is_none() || pack.head_only.is_none() {
            out.push("macros need a pack with a macro_base and head_only settings".into());
        }
        match macro_source {
            None => out.push("the spec has macros: pass the macro definition file (--macro)".into()),
            Some(source) if source.reference != m.definition => {
                out.push(format!("macros.definition is {}, the file given hashes to {}", m.definition, source.reference))
            }
            Some(_) => {}
        }
    }
    if let Some(r) = spec.eyeballs.as_ref().filter(|e| e.follows()).and_then(|e| e.rig.as_ref()) {
        match store.load_eyerig(r) {
            Ok(rig) if rig.catalog != spec.base.catalog => out.push(format!("eyeballs.rig {r} belongs to catalog {}, not to base.catalog", rig.catalog)),
            Ok(_) => {}
            Err(e) => out.push(format!("eyeballs.rig: {e}")),
        }
    }
    if let Some(r) = &spec.residual {
        if r.vertices != pack.vertices {
            out.push(format!("residual has {} vertices, the pack {}", r.vertices, pack.vertices));
        }
        match hash::parse_reference(&r.blob).and_then(|hex| store.get(&hex)) {
            Ok(bytes) if bytes.len() != r.bytes => out.push(format!("residual blob is {} bytes, the spec says {}", bytes.len(), r.bytes)),
            Ok(_) => {}
            Err(e) => out.push(format!("residual.blob: {e}")),
        }
    }
    (out, Some(Resolved { pack, dense, catalog }))
}

pub type Pin = (MacroTrace, HeadOnly);
pub type Traced = (Vec<[f64; 3]>, Value, Option<Pin>);
pub type TracedWithBase = (Vec<[f64; 3]>, Vec<[f64; 3]>, Value, Option<Pin>);

pub fn parametric(store: &Store, resolved: &Resolved, spec: &Spec, macro_source: Option<&MacroSource>) -> Result<(Vec<[f64; 3]>, Value), String> {
    parametric_traced(store, resolved, spec, macro_source).map(|(p, r, _)| (p, r))
}

pub fn parametric_traced(store: &Store, resolved: &Resolved, spec: &Spec, macro_source: Option<&MacroSource>) -> Result<Traced, String> {
    let spec = spec.canonical();
    let mut pin = None;
    let mut loaded = Vec::new();
    for (id, &w) in &spec.weights {
        loaded.push((w, store.load_target(&resolved.catalog, id)?));
    }
    let terms: Vec<(f64, &gf_core::catalog::SparseTarget)> = loaded.iter().map(|(w, t)| (*w, t)).collect();
    let mut layers: Vec<Vec<[f64; 3]>> = Vec::new();
    let mut report = json!({"targets": terms.len()});
    if let Some(m) = &spec.macros {
        let source = macro_source.ok_or("the spec has macros: pass the macro definition file")?;
        let base_sliders = resolved.pack.macro_base.as_ref().ok_or("the pack has no macro_base")?;
        let settings = resolved.pack.head_only.as_ref().ok_or("the pack has no head_only settings")?;
        let mode = Mode::parse(&m.mode)?;
        let on_head = |n: &str| resolved.catalog.targets.contains_key(n);
        let changes = head::weight_changes(&source.definition, mode, &m.sliders, base_sliders, &on_head)?;
        let mut raw = vec![[0.0f64; 3]; resolved.dense.positions.len()];
        for (id, dw) in &changes {
            store.load_target(&resolved.catalog, id)?.add_scaled(*dw, &mut raw);
        }
        let boundary = gf_geom::open_boundary(&resolved.dense.triangles);
        let frame = HeadOnly::new(&resolved.dense.positions, &boundary, settings.pin_top_m)?;
        let (mut pinned, coef) = frame.strip_and_pin(&raw);
        frame.add_head_scale(m.head_scale, &mut pinned)?;
        report["macros"] = json!({"mode": mode.name(), "targets_changed": changes.len(), "similarity_stripped": coef,
                                  "seam_top_m": frame.seam_top, "pin_top_m": frame.pin_top, "head_scale": m.head_scale});
        layers.push(pinned);
        pin = Some((MacroTrace { changes, coef, head_scale: m.head_scale }, frame));
    }
    let refs: Vec<&[[f64; 3]]> = layers.iter().map(|l| l.as_slice()).collect();
    Ok((gf_core::character::evaluate_dense(&resolved.dense.positions, &terms, &refs)?, report, pin))
}

pub fn evaluate(store: &Store, spec: &Spec, macro_source: Option<&MacroSource>) -> Result<(Vec<[f64; 3]>, Value), String> {
    evaluate_with_base(store, spec, macro_source).map(|(positions, _, report)| (positions, report))
}

pub type Evaluated = (Vec<[f64; 3]>, Vec<[f64; 3]>, Value);

pub fn evaluate_with_base(store: &Store, spec: &Spec, macro_source: Option<&MacroSource>) -> Result<Evaluated, String> {
    evaluate_traced(store, spec, macro_source).map(|(dense, base, report, _)| (dense, base, report))
}

pub const MODEL_GAZE: [f64; 3] = [0.0, -1.0, 0.0];

pub fn model_eyes(store: &Store, spec: &Spec) -> Result<Option<Vec<Eye>>, String> {
    match spec.eyeballs.as_ref() {
        Some(eb) => model_eyes_for(store, &spec.base.catalog, eb),
        None => Ok(None),
    }
}

pub fn model_eyes_for(store: &Store, catalog: &str, eb: &gf_core::eyerig::EyeballSpec) -> Result<Option<Vec<Eye>>, String> {
    let Some(choice) = eb.model.as_ref() else { return Ok(None) };
    let rig_ref = eb.rig.as_deref().ok_or("eyeballs.model needs eyeballs.rig")?;
    let rig = store.load_eyerig(rig_ref)?;
    if rig.catalog != catalog {
        return Err(format!("eye rig {rig_ref} belongs to catalog {}, not {catalog}", rig.catalog));
    }
    let model = gf_core::eye_inventory::model(&choice.id).ok_or_else(|| format!("eye model {:?} is not in the inventory", choice.id))?;
    let shape = gf_geom::eyeball::EyeballShape { radius_mm: choice.radius_mm, ..model.shape };
    let mesh = shape.mesh()?;
    let mut eyes = Vec::new();
    for helper in [&rig.helpers.left, &rig.helpers.right] {
        let n = helper.len().max(1) as f64;
        let centre = [0, 1, 2].map(|k| helper.iter().map(|p| p[k]).sum::<f64>() / n);
        let placed = mesh.placed(centre, MODEL_GAZE)?;
        eyes.push(gf_geom::lids::eye_from_mesh(centre, choice.radius_mm / 1000.0, MODEL_GAZE, &placed)?);
    }
    Ok(Some(eyes))
}

pub fn place_eyes(store: &Store, spec: &Spec, pin: Option<&Pin>, eyes: Vec<Eye>) -> Result<(Vec<Eye>, Option<Vec<PlacedEye>>), String> {
    let spec = spec.canonical();
    let Some(eb) = spec.eyeballs.as_ref() else {
        return Ok((eyes, None));
    };
    let rig = match (eb.follows(), eb.rig.as_deref()) {
        (true, Some(r)) => {
            let rig = store.load_eyerig(r)?;
            if rig.catalog != spec.base.catalog {
                return Err(format!("eye rig {r} belongs to catalog {}, not {}", rig.catalog, spec.base.catalog));
            }
            Some(rig)
        }
        _ => None,
    };
    let weights = spec.canonical().weights;
    let (placed, report) = eyerig::place_eyes(eb, rig.as_ref(), &weights, pin.map(|(t, h)| (t, h)), &eyes)?;
    Ok((placed, Some(report)))
}

pub fn evaluate_traced(store: &Store, spec: &Spec, macro_source: Option<&MacroSource>) -> Result<TracedWithBase, String> {
    let (problems, resolved) = problems(store, spec, macro_source);
    if !problems.is_empty() {
        return Err(format!("the spec does not validate: {}", problems.join("; ")));
    }
    let resolved = resolved.expect("validated specs resolve");
    let (mut dense, mut report, pin) = parametric_traced(store, &resolved, spec, macro_source)?;
    if let Some(r) = &spec.residual {
        let (values, _) = residual::decode(&store.get_ref(&r.blob)?)?;
        if values.len() != dense.len() {
            return Err(format!("residual has {} vertices, the pack {}", values.len(), dense.len()));
        }
        for (p, d) in dense.iter_mut().zip(&values) {
            for k in 0..3 {
                p[k] += d[k];
            }
        }
        report["residual"] = json!({"vertices": values.len(), "bytes": r.bytes});
    }
    Ok((dense, resolved.dense.positions, report, pin))
}

pub fn capture_residual(store: &Store, parametric: &[[f64; 3]], target: &[[f64; 3]], quantum_m: f64) -> Result<(ResidualRef, Value), String> {
    if parametric.len() != target.len() {
        return Err(format!("the residual source has {} vertices, the pack {}", target.len(), parametric.len()));
    }
    let r: Vec<[f64; 3]> = target.iter().zip(parametric).map(|(t, p)| [t[0] - p[0], t[1] - p[1], t[2] - p[2]]).collect();
    let encoded = residual::encode(&r, quantum_m)?;
    let (back, _) = residual::decode(&encoded.bytes)?;
    let norm = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let lengths: Vec<f64> = r.iter().map(|&v| norm(v)).collect();
    let rms = (lengths.iter().map(|l| l * l).sum::<f64>() / lengths.len().max(1) as f64).sqrt();
    let max = lengths.iter().copied().fold(0.0, f64::max);
    let codec_error = r.iter().zip(&back).map(|(a, b)| norm([a[0] - b[0], a[1] - b[1], a[2] - b[2]])).fold(0.0, f64::max);
    let blob = hash::reference(&store.put(&encoded.bytes)?);
    let reference = ResidualRef {
        blob,
        encoding: residual::ENCODING.into(),
        quantum_mm: quantum_m * 1000.0,
        vertices: r.len(),
        bytes: encoded.bytes.len(),
        rms_mm: rms * 1000.0,
        max_mm: max * 1000.0,
    };
    let report = json!({"raw_bytes": encoded.raw_bytes, "bytes": encoded.bytes.len(), "max_axis_error_mm": encoded.max_error_m * 1000.0,
                        "max_error_mm": codec_error * 1000.0, "rms_mm": rms * 1000.0, "max_mm": max * 1000.0});
    Ok((reference, report))
}

pub fn macro_spec(source: &MacroSource, mode: &str, given: &BTreeMap<String, f64>, base: &BTreeMap<String, f64>, head_scale: f64) -> MacroSpec {
    let mut sliders = base.clone();
    for (k, v) in given {
        sliders.insert(k.clone(), *v);
    }
    MacroSpec { mode: mode.to_string(), sliders, head_scale, definition: source.reference.clone() }
}
