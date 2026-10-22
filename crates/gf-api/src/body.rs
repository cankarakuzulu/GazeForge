use std::collections::BTreeMap;
use std::path::Path;

use gf_core::body::{self as b, Controls, Limits, Plant, Point};
use gf_core::body_shape::{self as bs, Similarity};
use gf_core::character::Spec;
use gf_core::head;
use gf_core::macros::{self, Mode};
use gf_io::body as io;
use gf_io::character as ch;
use gf_io::store::Store;
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{obj, shape, text};
use crate::error::{ApiError, Code};

pub const DEFAULT_LODS: [usize; 2] = [3500, 1400];

fn number(input: &Value, key: &str, default: f64) -> Result<f64, ApiError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| ApiError::invalid(format!("{key} must be a finite number"))),
    }
}

pub const DEFAULT_FINE_LODS: [usize; 3] = [24000, 8000, 3000];
pub const DEFAULT_SUBDIVIDE: u64 = 2;
const STRANDS_PREVIEW: u64 = 4000;
const STRAND_PREVIEW_POINTS: usize = 12;
const STRAND_PREVIEW_WIDTH_M: f64 = 0.0012;

fn lods(input: &Value, key: &str, default: &[usize]) -> Result<Vec<usize>, ApiError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(default.to_vec()),
        Some(Value::Array(a)) => {
            let v: Vec<usize> = a.iter().map(|x| x.as_u64().map(|n| n as usize)).collect::<Option<_>>().ok_or_else(|| ApiError::invalid(format!("{key} must be vertex counts")))?;
            if v.windows(2).any(|w| w[1] >= w[0]) || v.iter().any(|&n| n < 100) {
                return Err(ApiError::invalid(format!("{key} must be decreasing vertex counts of at least 100")));
            }
            Ok(v)
        }
        _ => Err(ApiError::invalid(format!("{key} must be an array"))),
    }
}

fn lods_fine(input: &Value) -> Result<Vec<usize>, ApiError> {
    lods(input, "fine_lods", &DEFAULT_FINE_LODS)
}

pub fn import_input() -> Value {
    obj(
        json!({
            "mode": {"enum": ["makehuman", "mesh"], "description": "makehuman: the MakeHuman base mesh, joined by topology (exact seam); mesh: any body mesh, joined at its neck loop or a cut plane"},
            "obj": {"type": "string", "minLength": 1, "description": "the body OBJ"},
            "dense": {"type": "string", "minLength": 1, "description": "the dense base directory (face_sizes.npy, corner_vertices.npy, positions.npy)"},
            "out": {"type": "string", "minLength": 1, "description": "the body pack directory to write"},
            "group": {"type": "string", "description": "OBJ group to read (makehuman default: body)"},
            "unit_m": {"type": "number", "description": "metres per OBJ unit (makehuman default 0.1, mesh default 1)"},
            "up": {"enum": ["y", "z"], "description": "the OBJ's up axis (default y)"},
            "cut_z_m": {"type": "number", "description": "mesh mode: remove every face above this height (metres, z up, after unit and axis) and join at the cut"},
            "align": {"enum": ["translate", "similarity", "none"], "description": "mesh mode: translate (default) moves the body's neck ring centre onto the head seam's; similarity also scales the body to the seam's size; none keeps the OBJ's placement"},
            "lods": {"type": "array", "items": {"type": "integer"}, "description": "vertex counts of the coarser body LODs (default [3500, 1400]); makehuman mode with subdivide 0 and mesh mode"},
            "subdivide": {"type": "integer", "minimum": 0, "maximum": 2, "description": "makehuman mode: Catmull-Clark levels the shaped body is refined by before it is stitched and decimated (default 2; 0 keeps the cage as LOD0); each LOD is decimated from the level its size calls for (this level at 20000 vertices and up, level 1 from 2000, the cage below) with the protected neck collar at most a quarter of it, so its neck ring stays sparser than the head LOD's seam"},
            "fine_lods": {"type": "array", "items": {"type": "integer"}, "description": "makehuman mode with subdivide > 0: vertex counts of every body LOD, each a decimation of the refined body with the neck band kept fine (default [24000, 8000, 3000], matching a three-level head; one count per head level, finest first, 800 the recommended floor - e.g. [24000, 12000, 6000, 3000, 1500, 800] for a head ladder 20000/10000/5000/2500/1200/800)"},
            "targets": {"type": "string", "description": "makehuman mode: the MakeHuman targets directory, to shape the body with macro sliders"},
            "macro": {"type": "string", "description": "makehuman mode: the macro definition (macro.json) for the sliders"},
            "sliders": {"type": "object", "description": "makehuman mode: macro sliders; missing ones default to 0.5 (races 1/3)"},
            "weight_mode": {"enum": ["gf", "mpfb"], "description": "macro weight mode (default gf)"},
            "store": {"type": "string", "minLength": 1, "description": "also put the pack into this store; the result's body is its reference, which a spec's body.pack and body.targets take"}
        }),
        &["mode", "obj", "dense", "out"],
    )
}

pub fn import_output() -> Value {
    shape(json!({"out": {"type": "string"}, "manifest": {"type": "object"}}))
}

fn shaping(input: &Value) -> Result<(Vec<io::Shaping>, BTreeMap<String, f64>), ApiError> {
    let (root, def) = (text(input, "targets"), text(input, "macro"));
    if root.is_empty() && def.is_empty() {
        return Ok((Vec::new(), BTreeMap::new()));
    }
    if root.is_empty() || def.is_empty() {
        return Err(ApiError::invalid("shaping needs both targets and macro"));
    }
    let source = ch::read_macro_definition(Path::new(def)).map_err(ApiError::invalid)?;
    let mut sliders: BTreeMap<String, f64> = macros::MACRO_SLIDERS.iter().map(|s| (s.to_string(), 0.5)).collect();
    for r in macros::RACES {
        sliders.insert(r.to_string(), 1.0 / 3.0);
    }
    if let Some(given) = input.get("sliders").and_then(Value::as_object) {
        for (k, v) in given {
            sliders.insert(k.clone(), v.as_f64().ok_or_else(|| ApiError::invalid(format!("slider {k} must be a number")))?);
        }
    }
    let mode = Mode::parse(input.get("weight_mode").and_then(Value::as_str).unwrap_or("gf")).map_err(ApiError::invalid)?;
    let file = |name: &str| Path::new(root).join(format!("{name}.target.gz"));
    let exists = |name: &str| file(name).is_file();
    let weights = macros::target_weights(&source.definition, &sliders, Some(&exists), mode).map_err(ApiError::invalid)?;
    let mut out = Vec::new();
    for (name, w) in weights {
        if w > 0.0 {
            out.push((name.clone(), w, io::read_target(&file(&name)).map_err(ApiError::invalid)?));
        }
    }
    Ok((out, sliders))
}

pub fn import(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let (path, dense, out) = (Path::new(text(input, "obj")), Path::new(text(input, "dense")), Path::new(text(input, "out")));
    if !path.is_file() {
        return Err(ApiError::not_found(format!("obj {} does not exist", path.display())));
    }
    if !dense.join("face_sizes.npy").is_file() {
        return Err(ApiError::not_found(format!("{} is not a dense base directory", dense.display())));
    }
    let lods = lods(input, "lods", &DEFAULT_LODS)?;
    let fine_lods = lods_fine(input)?;
    let subdivide = input.get("subdivide").and_then(Value::as_u64).unwrap_or(DEFAULT_SUBDIVIDE);
    if subdivide > 2 {
        return Err(ApiError::invalid("subdivide is 0 to 2"));
    }
    let manifest = match text(input, "mode") {
        "makehuman" => {
            let group = input.get("group").and_then(Value::as_str).unwrap_or("body");
            let (shaping, used_sliders) = shaping(input)?;
            let mh = io::MakeHumanInput { obj: path, group, unit: number(input, "unit_m", 0.1)?, shaping, lods: &lods, subdivide: subdivide as usize, fine_lods: &fine_lods };
            let mut m = io::import_makehuman(&mh, dense, out).map_err(ApiError::invalid)?;
            if !used_sliders.is_empty() {
                m["source"]["sliders"] = json!(used_sliders);
                std::fs::write(out.join("manifest.json"), serde_json::to_string_pretty(&m).map_err(|e| ApiError::internal(e.to_string()))?)
                    .map_err(|e| ApiError::internal(format!("{}: {e}", out.display())))?;
            }
            m
        }
        "mesh" => {
            let up = io::Up::parse(input.get("up").and_then(Value::as_str).unwrap_or("y")).map_err(ApiError::invalid)?;
            let cut = input.get("cut_z_m").and_then(Value::as_f64);
            let mi = io::MeshInput {
                obj: path,
                group: input.get("group").and_then(Value::as_str),
                unit: number(input, "unit_m", 1.0)?,
                up,
                cut_z: cut,
                align: input.get("align").and_then(Value::as_str).unwrap_or("translate"),
                lods: &lods,
            };
            io::import_mesh(&mi, dense, out).map_err(ApiError::invalid)?
        }
        other => return Err(ApiError::invalid(format!("mode must be makehuman or mesh, got {other}"))),
    };
    let stored = match input.get("store").and_then(Value::as_str) {
        Some(root) => {
            let store = Store::open(Path::new(root)).map_err(ApiError::internal)?;
            Some(store.put_body(out).map_err(ApiError::invalid)?.0)
        }
        None => None,
    };
    Ok(json!({"out": out.display().to_string(), "manifest": manifest, "body": stored}))
}

pub fn targets_input() -> Value {
    obj(
        json!({"store": {"type": "string", "minLength": 1},
               "body": {"type": "string", "minLength": 1, "description": "a body reference from body.import with store"},
               "targets": {"type": "string", "minLength": 1, "description": "the MakeHuman targets directory"},
               "families": {"type": "array", "items": {"type": "string"}, "description": "target folders to import (default macrodetails and breast)"},
               "unit_m": {"type": "number", "description": "metres per MakeHuman unit (default 0.1)"}}),
        &["store", "body", "targets"],
    )
}

pub fn targets_output() -> Value {
    shape(json!({"targets": {"type": "string"}, "summary": {"type": "object"}}))
}

pub fn targets(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::internal)?;
    let families: Vec<String> = match input.get("families").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => vec!["macrodetails".into(), "breast".into()],
    };
    let (reference, m) = store.import_body_targets(text(input, "body"), Path::new(text(input, "targets")), &families, number(input, "unit_m", 0.1)?).map_err(ApiError::invalid)?;
    Ok(json!({"targets": reference, "summary": gf_io::body_store::summary(&m)}))
}

pub fn sliders_input() -> Value {
    obj(
        json!({"store": {"type": "string", "minLength": 1},
               "targets": {"type": "string", "minLength": 1, "description": "a body targets reference (body.targets)"},
               "catalog": {"type": "string", "minLength": 1, "description": "the head catalog: targets it also has are left to the head's sliders, which drive both"}}),
        &["store", "targets"],
    )
}

pub fn sliders_output() -> Value {
    shape(json!({"pairs": {"type": "array"}, "compounds": {"type": "array"}, "shared_with_head": {"type": "integer"},
                 "genitals": {"type": "array", "items": {"type": "string"}, "description": "the body.genitals kinds this body can wear (empty when its pack has no helper-genital patch or its targets no helpers)"},
                 "physics": {"type": "array", "items": {"type": "string"}, "description": "the body.wearables[].physics kinds besides none (cloth: pinned cloth an engine simulates; springbone: chains of joints, for hair, ropes and capes)"},
                 "groom": {"type": "object", "description": "body.groom: starter (the groom a client turns on), fields (each number setting's key, default, lo, hi and whether it is an integer), parts, ties (none, low, high: a ponytail), fringes (none, straight, curtain), directions and styles (named sets of settings and a tie a client can apply)"}}))
}

pub fn sliders(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::internal)?;
    let targets = store.load_body_targets(text(input, "targets")).map_err(ApiError::not_found)?;
    let head: std::collections::BTreeSet<String> = match input.get("catalog").and_then(Value::as_str) {
        Some(c) => store.load_catalog(c).map_err(ApiError::not_found)?.targets.into_keys().collect(),
        None => Default::default(),
    };
    let mut pairs: BTreeMap<String, Value> = BTreeMap::new();
    let mut shared = 0;
    for (id, e) in &targets.targets {
        let macro_breast = e.family == "breast" && (id.starts_with("breast/female-") || id.starts_with("breast/male-"));
        if e.family == "macrodetails" || macro_breast || (e.moved == 0 && e.family != "genitals") {
            continue;
        }
        if head.contains(id) {
            shared += 1;
            continue;
        }
        let (base, dir) = if let Some(b) = id.strip_suffix("-incr") {
            (b.to_string(), "incr")
        } else if let Some(b) = id.strip_suffix("-decr") {
            (b.to_string(), "decr")
        } else {
            (id.clone(), "incr")
        };
        let row = pairs.entry(base.clone()).or_insert_with(|| json!({"id": base, "family": e.family}));
        row[dir] = json!(id);
    }
    let compounds: Vec<Value> = bs::compounds()
        .iter()
        .map(|c| {
            let missing: Vec<&String> = c.plus.iter().chain(&c.minus).map(|(t, _)| t).filter(|t| !targets.targets.contains_key(*t)).collect();
            json!({"name": c.name, "missing": missing})
        })
        .collect();
    let body: gf_io::body_store::BodyManifest = store.get_manifest("bodies", &targets.body).map_err(ApiError::not_found)?;
    let genitals: Vec<&str> = if body.files.contains_key("genital/quads.npy") && targets.helpers > 0 { bs::GENITALS.to_vec() } else { Vec::new() };
    Ok(json!({"pairs": pairs.into_values().collect::<Vec<_>>(), "compounds": compounds, "shared_with_head": shared, "genitals": genitals, "physics": bs::WEARABLE_PHYSICS,
              "groom": {"starter": gf_core::groom::starter(), "parts": gf_core::groom::PARTS, "ties": gf_core::groom::TIES, "fringes": gf_core::groom::FRINGES, "highlight_default": gf_core::groom::HIGHLIGHT_DEFAULT, "directions": gf_core::groom::DIRECTIONS, "styles": gf_core::groom::STYLES.iter().map(|st| json!({"name": st.name, "tie": st.tie, "direction": st.direction, "fringe": st.fringe, "settings": st.settings.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<String, Value>>()})).collect::<Vec<_>>(), "fields": gf_core::groom::FIELDS.iter().map(|f| json!({"key": f.key, "default": f.default, "lo": f.lo, "hi": f.hi, "integer": f.integer})).collect::<Vec<_>>()}}))
}

pub struct Shaped {
    pub pack: b::BodyPack,
    pub manifest: Value,
    pub controls: Controls,
    pub similarity: Similarity,
    pub report: Value,
}

pub fn spec_body(store: &Store, spec: &Spec, source: Option<&ch::MacroSource>, pin: Option<&ch::Pin>) -> Result<Shaped, String> {
    let body = spec.body.as_ref().ok_or("the spec has no body")?.canonical();
    let (manifest, mut pack) = store.load_body(&body.pack)?;
    let targets = store.load_body_targets(&body.targets)?;
    if targets.body != body.pack {
        return Err(format!("body.targets {} belong to body {}, not {}", body.targets, targets.body, body.pack));
    }
    let mut loaded = Vec::new();
    let mut weights = body.target_weights();
    let mut shared = 0;
    for (id, &w) in &spec.canonical().weights {
        if targets.targets.contains_key(id) {
            *weights.entry(id.clone()).or_insert(0.0) += w;
            shared += 1;
        }
    }
    for (id, &w) in &weights {
        loaded.push((w, store.load_body_target(&targets, id)?));
    }
    let mut macro_changes = 0;
    if let Some(m) = &spec.macros {
        let source = source.ok_or("the spec has macros: pass the macro definition file")?;
        let mode = Mode::parse(&m.mode)?;
        let on_body = |n: &str| targets.targets.contains_key(n);
        let changes = head::weight_changes(&source.definition, mode, &m.sliders, &targets.base_sliders, &on_body)?;
        macro_changes = changes.len();
        for (id, dw) in changes {
            loaded.push((dw, store.load_body_target(&targets, &id)?));
        }
    }
    let terms: Vec<(f64, &gf_core::catalog::SparseTarget)> = loaded.iter().map(|(w, t)| (*w, t)).collect();
    let n = pack.mesh.positions.len();
    let helpers_shaped = targets.helpers > 0;
    if helpers_shaped && targets.helpers != pack.helpers.positions.len() {
        return Err(format!("body.targets carry {} helper vertices, the body has {}", targets.helpers, pack.helpers.positions.len()));
    }
    let mut base = pack.mesh.positions.clone();
    if helpers_shaped {
        base.extend_from_slice(&pack.helpers.positions);
    }
    let full = bs::shape(&base, &terms)?;
    let similarity = match pin {
        Some((trace, frame)) => Similarity::from_linear(&trace.coef, frame.centre),
        None => Similarity::identity(),
    };
    let canonical: Vec<[f64; 3]> = full.iter().map(|&p| similarity.invert(p)).collect();
    pack.mesh.positions = canonical[..n].to_vec();
    if helpers_shaped {
        pack.helpers.positions = canonical[n..].to_vec();
    } else {
        pack.helpers = Default::default();
    }
    let report = json!({"pack": body.pack, "targets": body.targets, "regional": body.weights.len(), "compounds": body.compounds, "shared_with_head": shared, "helpers_shaped": pack.helpers.positions.len(), "macro_targets_changed": macro_changes,
                        "base_sliders": targets.base_sliders,
                        "similarity": {"scale": similarity.scale, "translation_m": similarity.translation, "centre_m": similarity.centre, "rotation_quaternion": similarity.quaternion(),
                                       "meaning": "the head follows the body: the head is built in its canonical frame (the macros' similarity stripped) and the whole character is mapped to the world by this similarity; the body was mapped into the canonical frame by its inverse"}});
    Ok(Shaped { pack, manifest, controls: body.controls(), similarity, report })
}

pub fn controls(input: &Value) -> Result<Controls, ApiError> {
    let c = Controls {
        blend_m: number(input, "blend_mm", 60.0)? / 1000.0,
        neck_offset_m: number(input, "neck_offset_mm", 0.0)? / 1000.0,
        head_scale: number(input, "head_scale", 1.0)?,
        body_scale: number(input, "body_scale", 1.0)?,
        smooth: number(input, "smooth", 0.0)?.round().max(0.0) as usize,
        tangent: number(input, "tangent", 0.5)?,
    };
    c.check().map_err(ApiError::invalid)?;
    Ok(c)
}

pub fn control_props() -> Value {
    json!({
        "blend_mm": {"type": "number", "description": "width of the band below the seam over which the body is bent onto the head (default 60)"},
        "neck_offset_mm": {"type": "number", "description": "move the body up (+) or down (-) before the blend (default 0)"},
        "head_scale": {"type": "number", "description": "scale the head about the seam centre (default 1, 0.5..2)"},
        "body_scale": {"type": "number", "description": "scale the body about its neck ring centre (default 1, 0.5..2)"},
        "smooth": {"type": "integer", "description": "Laplacian smoothing passes over the band, weighted by the band (default 0)"},
        "tangent": {"type": "number", "description": "0..1: how far the body's first row is turned to continue the head surface across the seam (default 0.5)"}
    })
}

pub fn stitch_input() -> Value {
    let mut props = json!({
        "store": {"type": "string", "minLength": 1, "description": "path of the store directory"},
        "spec": {"type": "string", "description": "path of a spec file"},
        "out_shaped": {"type": "string", "description": "with a spec body: write the shaped body (before the stitch, world frame, metres) to this .npy"},
        "character": {"type": "object", "description": "a spec object, or the designer's character (store, pack, catalog, game, weights, ...), instead of spec"},
        "macro": {"type": "string", "description": "the macro definition, for a spec with macro sliders"},
        "body": {"type": "string", "minLength": 1, "description": "a body pack directory written by body.import; without it the spec's body is used, shaped by its macros"},
        "lod": {"type": "integer", "description": "0 (this command stitches the finest LOD; export-glb --body stitches every LOD)"},
        "out_obj": {"type": "string", "description": "write the head and the stitched body as one OBJ"},
        "return_mesh": {"type": "boolean", "description": "return the stitched body (base64 f32 positions and normals, u32 indices), the seam as overlay lines and the head positions it was stitched to (head_scale applied, game LOD0 order)"},
        "enforce": {"type": "boolean", "description": "fail with gate_failed when a gate fails (default true)"},
        "strands_preview": {"type": "integer", "minimum": 1, "maximum": 50000, "description": "with return_mesh: how many strands a worn item with strands on, or the body.groom, is drawn with at most (default 4000; ribbons 1.2 mm wide at the root, in its average hair colour or the groom's colour, in place of its cards)"},
        "plant": {"enum": ["gap", "crack", "crease", "flip", "penetrate"], "description": "gate self-test: plant this defect after stitching; the gates must fail"}
    });
    for (k, v) in control_props().as_object().unwrap() {
        props[k] = v.clone();
    }
    obj(props, &["store"])
}

pub fn stitch_output() -> Value {
    shape(json!({"gates": {"type": "object"}, "body": {"type": "object"}, "controls": {"type": "object"}, "mesh": {"type": "object"}}))
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

fn f32s(points: &[Point]) -> String {
    b64(&points.iter().flat_map(|p| p.iter().flat_map(|x| (*x as f32).to_le_bytes())).collect::<Vec<u8>>())
}

fn spec_input(input: &Value, root: &str) -> Result<Spec, ApiError> {
    Ok(match (input.get("character"), text(input, "spec")) {
        (Some(c), _) if c.is_object() && c.get("format").is_none() && c.get("pack").is_some() => {
            let mut form = c.clone();
            if form.get("store").is_none() {
                form["store"] = json!(root);
            }
            if form.get("macro").is_none()
                && let Some(m) = input.get("macro")
            {
                form["macro"] = m.clone();
            }
            crate::designer::spec_from_input(&form)?
        }
        (Some(c), _) if c.is_object() => serde_json::from_value(c.clone()).map_err(|e| ApiError::invalid(format!("character: {e}")))?,
        (_, p) if !p.is_empty() => ch::read_spec(Path::new(p)).map_err(ApiError::invalid)?,
        _ => return Err(ApiError::invalid("pass spec (a path) or character (a spec object)")),
    })
}

pub struct BodyRig {
    pub rig: gf_core::body_rig::RigDef,
    pub heads: Vec<Point>,
    pub by_obj: std::collections::HashMap<u32, gf_core::body_rig::Influences>,
    pub head_joint: usize,
    pub meta: Value,
}

pub fn rig_source(rig: &str, weights: Option<&str>) -> Result<(Value, Value, Value), String> {
    let read = |p: &str| -> Result<Value, String> { serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?).map_err(|e| format!("{p}: {e}")) };
    let file = |p: &str| Path::new(p).file_name().map(|n| n.to_string_lossy().to_string());
    if gf_core::rig_presets::names().contains(&rig) {
        let (r, w) = gf_core::rig_presets::preset(rig)?;
        return match weights {
            None => Ok((r, w, json!({"preset": rig, "source": "MakeHuman's standard rig and weights, bundled (CC0; THIRD_PARTY.md)"}))),
            Some(p) => Ok((r, read(p)?, json!({"preset": rig, "weights": file(p)}))),
        };
    }
    let w = weights.ok_or_else(|| format!("{rig} is not a rig preset ({}); a rig file needs its weights file too", gf_core::rig_presets::names().join(", ")))?;
    Ok((read(rig)?, read(w)?, json!({"rig": file(rig), "weights": file(w)})))
}

pub struct HeadFit {
    pub reference: Vec<Point>,
    pub fitted: Vec<Point>,
    pub ring_offset_mm: f64,
}

const HEAD_FIT_NEAREST: usize = 8;

impl HeadFit {
    pub fn shift(&self, p: Point) -> Point {
        let mut near: Vec<(f64, usize)> = self.reference.iter().enumerate().map(|(i, q)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2), i)).collect();
        let k = HEAD_FIT_NEAREST.min(near.len());
        if k == 0 {
            return [0.0; 3];
        }
        near.select_nth_unstable_by(k - 1, |a, b| a.0.total_cmp(&b.0));
        let (mut sum, mut total) = ([0.0; 3], 0.0);
        for &(d2, i) in &near[..k] {
            let w = 1.0 / (d2 + 1e-10);
            for (s, (f, r)) in sum.iter_mut().zip(self.fitted[i].iter().zip(&self.reference[i])) {
                *s += w * (f - r);
            }
            total += w;
        }
        sum.map(|s| s / total)
    }
}

pub fn head_fit(store: &Store, spec: &Spec, macros: Option<&ch::MacroSource>, pack: &b::BodyPack, controls: &Controls, positions: &[Point]) -> Result<HeadFit, String> {
    let body = spec.body.as_ref().ok_or("the spec has no body")?.canonical();
    let targets = store.load_body_targets(&body.targets)?;
    let mut reference = spec.clone();
    reference.weights.retain(|id, _| targets.targets.contains_key(id));
    reference.residual = None;
    if let Some(m) = reference.macros.as_mut() {
        m.head_scale = 1.0;
    }
    let (plain, _, _, _) = ch::evaluate_traced(store, &reference, macros)?;
    let n = pack.cage_source.len().min(plain.len()).min(positions.len());
    let mut fitted = positions[..n].to_vec();
    gf_core::body::scale_about(&mut fitted, pack.pivot(), controls.head_scale);
    let at: std::collections::HashMap<u32, usize> = pack.source.iter().enumerate().map(|(i, &o)| (o, i)).collect();
    let ring_offset_mm = pack.cage_source[..n].iter().enumerate().filter_map(|(d, o)| at.get(o).map(|&i| (plain[d], pack.mesh.positions[i])))
        .map(|(a, b)| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() * 1000.0).fold(0.0f64, f64::max);
    Ok(HeadFit { reference: plain[..n].to_vec(), fitted, ring_offset_mm })
}

pub fn under_bone(rig: &gf_core::body_rig::RigDef, mut j: usize, ancestor: usize) -> bool {
    while let Some(p) = rig.bones[j].parent.as_deref().and_then(|n| rig.bones.iter().position(|b| b.name == n)) {
        if p == ancestor {
            return true;
        }
        j = p;
    }
    false
}

pub fn drop_face_bones(br: &mut BodyRig, keep: &[usize]) -> Value {
    let n = br.rig.bones.len();
    let parent_of = |j: usize| br.rig.bones[j].parent.as_deref().and_then(|p| br.rig.bones.iter().position(|b| b.name == p));
    let dropped: Vec<bool> = (0..n).map(|j| j != br.head_joint && under_bone(&br.rig, j, br.head_joint) && !keep.contains(&j)).collect();
    let fold: Vec<usize> = (0..n)
        .map(|j| {
            let mut k = j;
            while dropped[k] {
                k = parent_of(k).unwrap_or(br.head_joint);
            }
            k
        })
        .collect();
    let mut new_index = vec![usize::MAX; n];
    let mut next = 0;
    for j in 0..n {
        if !dropped[j] {
            new_index[j] = next;
            next += 1;
        }
    }
    let names: Vec<String> = (0..n).filter(|&j| dropped[j]).map(|j| br.rig.bones[j].name.clone()).collect();
    if names.is_empty() {
        return json!({"dropped": 0});
    }
    let mut folded_entries = 0usize;
    for infl in br.by_obj.values_mut() {
        let mut merged: Vec<(usize, f64)> = Vec::with_capacity(infl.len());
        for &(j, w) in infl.iter() {
            if dropped[j] {
                folded_entries += 1;
            }
            let t = new_index[fold[j]];
            match merged.iter_mut().find(|e| e.0 == t) {
                Some(e) => e.1 += w,
                None => merged.push((t, w)),
            }
        }
        merged.sort_by(|a, b| b.1.total_cmp(&a.1));
        *infl = merged;
    }
    let keep_mask: Vec<bool> = dropped.iter().map(|d| !d).collect();
    let reparent: Vec<Option<String>> = (0..n).map(|j| parent_of(j).filter(|&p| dropped[p]).map(|p| br.rig.bones[fold[p]].name.clone())).collect();
    let mut bones = Vec::with_capacity(next);
    for (j, mut b) in std::mem::take(&mut br.rig.bones).into_iter().enumerate() {
        if !keep_mask[j] {
            continue;
        }
        if let Some(p) = reparent[j].clone() {
            b.parent = Some(p);
        }
        bones.push(b);
    }
    br.rig.bones = bones;
    br.heads = br.heads.iter().zip(&keep_mask).filter(|h| *h.1).map(|h| *h.0).collect();
    br.head_joint = new_index[br.head_joint];
    json!({"dropped": names.len(), "bones": names, "weight_entries_folded": folded_entries,
           "method": "a face rig was asked: the preset's bones under the head bone are left out, except the jaw and the eyes, which the face rig takes over; each left-out bone's weights go to its nearest kept ancestor (the jaw for the tongue, the head for the rest)"})
}

pub fn body_rig(pack: &b::BodyPack, controls: &Controls, rig_json: &Value, weights_json: &Value, source: Value, head_fit: Option<&HeadFit>) -> Result<BodyRig, String> {
    let rig = gf_core::body_rig::RigDef::parse(rig_json)?;
    if rig.bones.len() > 256 {
        return Err(format!("the rig has {} bones; a glTF skin here holds at most 256", rig.bones.len()));
    }
    if pack.helpers.positions.is_empty() || pack.cage_source.is_empty() {
        return Err("the body carries no helper vertices or cage map: re-import it (body.import, then body.targets) to rig it".into());
    }
    let at = gf_core::body_rig::Resolver::new(&pack.mesh.positions, &pack.source, &pack.helpers);
    let ends = rig.evaluate(&at)?;
    let control = pack.control_joint(controls);
    let (by_obj, unknown) = gf_core::body_rig::rig_weights(weights_json, &rig)?;
    let mut score = vec![0.0; rig.bones.len()];
    for o in &pack.cage_source {
        for &(j, w) in by_obj.get(o).map(Vec::as_slice).unwrap_or(&[]) {
            score[j] += w;
        }
    }
    let head_joint = (0..score.len()).max_by(|&a, &b| score[a].total_cmp(&score[b])).ok_or("the rig has no bones")?;
    let unweighted = pack.source.iter().chain(&pack.cage_source).filter(|o| by_obj.get(o).is_none_or(Vec::is_empty)).count();
    let mut heads: Vec<Point> = ends.iter().map(|e| control(e.0)).collect();
    let mut head_moves = Vec::new();
    if let Some(fit) = head_fit {
        for j in (0..heads.len()).filter(|&j| under_bone(&rig, j, head_joint)) {
            let d = fit.shift(heads[j]);
            heads[j] = [heads[j][0] + d[0], heads[j][1] + d[1], heads[j][2] + d[2]];
            head_moves.push((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * 1000.0);
        }
    }
    let head_joints = match head_fit {
        Some(fit) => json!({"moved": head_moves.len(), "max_mm": head_moves.iter().copied().fold(0.0f64, f64::max), "ring_offset_mm": fit.ring_offset_mm,
                            "method": "joints under the head bone follow the head's own shape: each moves by the inverse-square mean displacement of its 8 nearest head cage vertices, from the head as the body targets shape it (the spec without head-only weights, residual and head scale) to the head as exported"}),
        None => json!({"moved": 0, "method": "joints under the head bone stay where the body targets put them"}),
    };
    let meta = json!({"source": source, "head_joints": head_joints,
                      "bones": rig.bones.len(), "head_bone": rig.bones[head_joint].name, "weights_for_bones_not_in_the_rig": unknown, "unweighted_vertices": unweighted,
                      "joints": "one glTF joint per bone at the bone's head, evaluated on this body (MakeHuman's rule: a helper cube's mean, a vertex or a mean of vertices); translation-only rest, so each inverse bind is the joint's negated position; tails and rolls are not used",
                      "weights_method": "MakeHuman's weights per base.obj vertex: a body vertex takes its own; a head vertex mixes the weights of the cage corners around it bilinearly (its place on the cage, body.cage); a body seam vertex takes the weights of the head vertex it copies; then the strongest 8 (or 4 with --body-influences 4), renormalised; past 4 they go in JOINTS_1 / WEIGHTS_1",
                      "eyes": "rigid on the head bone"});
    Ok(BodyRig { heads, rig, by_obj, head_joint, meta })
}

pub fn refined_input() -> Value {
    obj(
        json!({"store": {"type": "string", "minLength": 1},
               "body": {"type": "string", "minLength": 1, "description": "a body reference (body.import with store)"},
               "out": {"type": "string", "minLength": 1, "description": "writes the refined body's positions at the pack's base shape (.npy, n x 3) and, beside it, <out>.quads.npy (the cage quads)"}}),
        &["store", "body", "out"],
    )
}

pub fn refined_output() -> Value {
    shape(json!({"level": {"type": "integer"}, "cage_vertices": {"type": "integer"}, "vertices": {"type": "integer"}, "lods": {"type": "array"}}))
}

pub fn refined(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::internal)?;
    let (_, pack) = store.load_body(text(input, "body")).map_err(ApiError::not_found)?;
    let fine = pack.fine.as_ref().ok_or_else(|| ApiError::invalid("the body is not refined (imported with subdivide 0, or before refinement existed)"))?;
    let out = text(input, "out");
    gf_io::dense::write_points(Path::new(out), &fine.stencil.apply(&pack.mesh.positions)).map_err(ApiError::internal)?;
    gf_io::write_table_i32(Path::new(&format!("{out}.quads.npy")), fine.quads.len(), 4, fine.quads.iter().flatten().map(|&v| v as i32).collect()).map_err(ApiError::internal)?;
    Ok(json!({"level": fine.level, "cage_vertices": pack.mesh.positions.len(), "vertices": fine.stencil.rows.len(),
              "lods": fine.lods.iter().map(|l| json!({"vertices": l.kept.len(), "triangles": l.triangles.len()})).collect::<Vec<_>>()}))
}

pub fn cage_input() -> Value {
    obj(
        json!({"dense": {"type": "string", "minLength": 1, "description": "the dense base directory (face_sizes, corner_vertices, positions)"},
               "out": {"type": "string", "minLength": 1, "description": "path prefix: writes <out>_corners.npy (n x 4 int32, -1 unused) and <out>_coefs.npy (n x 4 float64)"}}),
        &["dense", "out"],
    )
}

pub fn cage_output() -> Value {
    shape(json!({"vertices": {"type": "integer"}, "corners": {"type": "integer"}, "edge_points": {"type": "integer"}, "face_points": {"type": "integer"}}))
}

pub fn cage(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let dense = gf_io::body::read_dense_quads(Path::new(text(input, "dense"))).map_err(ApiError::not_found)?;
    let points = gf_core::cage::cage_points(&dense.quads, dense.positions.len(), 256).map_err(ApiError::invalid)?;
    let mut ids = Vec::with_capacity(points.len() * 4);
    let mut coefs = Vec::with_capacity(points.len() * 4);
    let mut counts = [0usize; 3];
    for p in &points {
        counts[match p {
            gf_core::cage::CagePoint::Corner(_) => 0,
            gf_core::cage::CagePoint::Edge(..) => 1,
            gf_core::cage::CagePoint::Face(..) => 2,
        }] += 1;
        let terms = p.terms();
        for k in 0..4 {
            let (c, w) = terms.get(k).copied().map_or((-1, 0.0), |(c, w)| (c as i32, w));
            ids.push(c);
            coefs.push(w);
        }
    }
    let out = text(input, "out");
    gf_io::write_table_i32(Path::new(&format!("{out}_corners.npy")), points.len(), 4, ids).map_err(ApiError::internal)?;
    gf_io::write_table(Path::new(&format!("{out}_coefs.npy")), points.len(), 4, coefs).map_err(ApiError::internal)?;
    Ok(json!({"vertices": points.len(), "corners": counts[0], "edge_points": counts[1], "face_points": counts[2]}))
}

pub fn joints_input() -> Value {
    obj(
        json!({"store": {"type": "string", "minLength": 1},
               "spec": {"type": "string", "description": "path of a spec file with a body"},
               "character": {"type": "object", "description": "a spec object, or the designer's character, instead of spec"},
               "macro": {"type": "string", "description": "the macro definition, for a spec with macro sliders"},
               "rig": {"type": "string", "minLength": 1, "description": "a MakeHuman rig file (rig.<name>.json): each joint end is a helper cube's mean, a vertex or a mean of vertices of base.obj"}}),
        &["store", "rig"],
    )
}

pub fn joints_output() -> Value {
    shape(json!({"bones": {"type": "array"}, "frame": {"type": "string"}, "body": {"type": "object"}}))
}

pub fn joints(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = text(input, "store");
    let store = Store::open(Path::new(root)).map_err(ApiError::internal)?;
    let spec = spec_input(input, root)?;
    if spec.body.is_none() {
        return Err(ApiError::invalid("the spec has no body"));
    }
    let rig_path = text(input, "rig");
    let rig_text = std::fs::read_to_string(rig_path).map_err(|e| ApiError::not_found(format!("{rig_path}: {e}")))?;
    let rig = gf_core::body_rig::RigDef::parse(&serde_json::from_str(&rig_text).map_err(|e| ApiError::invalid(format!("{rig_path}: {e}")))?).map_err(ApiError::invalid)?;
    let source = match text(input, "macro") {
        "" => None,
        p => Some(ch::read_macro_definition(Path::new(p)).map_err(ApiError::invalid)?),
    };
    let (dense, _, _, pin) = ch::evaluate_traced(&store, &spec, source.as_ref()).map_err(ApiError::invalid)?;
    let s = spec_body(&store, &spec, source.as_ref(), pin.as_ref()).map_err(ApiError::invalid)?;
    if s.pack.helpers.positions.is_empty() {
        return Err(ApiError::invalid("the body carries no shaped helper vertices: re-import the body (body.import) and its targets (body.targets) so the joints follow it"));
    }
    let at = gf_core::body_rig::Resolver::new(&s.pack.mesh.positions, &s.pack.source, &s.pack.helpers);
    let control = s.pack.control_joint(&s.controls);
    let mut ends: Vec<(Point, Point)> = rig.evaluate(&at).map_err(ApiError::invalid)?.into_iter().map(|(h, t)| (control(h), control(t))).collect();
    let head_bone = rig.bones.iter().position(|b| {
        let n = b.name.to_lowercase();
        n == "head" || n.ends_with(":head") || n.ends_with("_head")
    });
    let mut head_joints = json!({"moved": 0, "method": "no bone named head: the joints follow the body targets only"});
    if let Some(hb) = head_bone {
        let fit = head_fit(&store, &spec, source.as_ref(), &s.pack, &s.controls, &dense).map_err(ApiError::invalid)?;
        let add = |p: Point, d: Point| [p[0] + d[0], p[1] + d[1], p[2] + d[2]];
        let mut moved = 0;
        for j in (0..ends.len()).filter(|&j| under_bone(&rig, j, hb)) {
            let (h, t) = ends[j];
            ends[j] = (add(h, fit.shift(h)), add(t, fit.shift(t)));
            moved += 1;
        }
        head_joints = json!({"head_bone": rig.bones[hb].name, "moved": moved, "ring_offset_mm": fit.ring_offset_mm,
                             "method": "bones under the head bone follow the head's own shape, as export-glb --body-rig places them"});
    }
    let r6 = |p: Point| p.map(|x| (x * 1e6).round() / 1e6);
    let bones: Vec<Value> = rig
        .bones
        .iter()
        .zip(&ends)
        .map(|(b, (h, t))| json!({"name": b.name, "parent": b.parent, "head": r6(s.similarity.apply(*h)), "tail": r6(s.similarity.apply(*t)), "roll": b.roll,
                                  "head_canonical": r6(*h), "tail_canonical": r6(*t)}))
        .collect();
    Ok(json!({"bones": bones, "frame": "metres, z up: head/tail in the world (the character's root similarity applied), *_canonical in the head's canonical frame as the body is stored, with the body controls (body_scale, neck_offset_mm) applied as the stitch applies them to joints below the neck ring (the head, which the controls do not move, keeps the joints above it)",
              "head_joints": head_joints, "body": s.report}))
}

pub struct Scalp {
    pub head: Vec<Point>,
    pub triangles: Vec<[u32; 3]>,
    pub triangle_uvs: Vec<[gf_geom::Uv; 3]>,
    pub dense: Vec<[f64; 3]>,
    pub surface: crate::tasks::wearable::Surface,
}

pub fn scalp(store: &Store, spec: &Spec, source: Option<&ch::MacroSource>) -> Result<Scalp, String> {
    let (dense, _, _, pin) = ch::evaluate_traced(store, spec, source)?;
    let game_ref = spec.base.game_base.clone().ok_or("the spec names no game base")?;
    let (_, base) = store.load_game(&game_ref)?;
    let s = spec_body(store, spec, source, pin.as_ref())?;
    let mut head: Vec<Point> = base.kept.iter().map(|&d| dense[d as usize]).collect();
    b::scale_about(&mut head, s.pack.pivot(), s.controls.head_scale);
    let seam = b::head_seam(&s.pack, &head, &base.triangles, &base.kept)?;
    let st = b::stitch_lod(&s.pack, 0, &seam, &s.controls)?;
    let mut points = head.clone();
    points.extend_from_slice(&st.positions);
    let h = head.len() as u32;
    let tris: Vec<[u32; 3]> = base.triangles.iter().copied().chain(st.triangles.iter().map(|t| t.map(|v| v + h))).collect();
    let surface = crate::tasks::wearable::Surface::new(points, &tris);
    Ok(Scalp { head, triangles: base.triangles, triangle_uvs: base.triangle_uvs, dense, surface })
}

type HatCover = (Vec<[f64; 3]>, Vec<[u32; 3]>, bool);

pub fn stitch(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = text(input, "store");
    if !Path::new(root).join("packs").is_dir() {
        return Err(ApiError::not_found(format!("{root} is not a store")));
    }
    let store = Store::open(Path::new(root)).map_err(ApiError::internal)?;
    let spec = spec_input(input, root)?;
    if input.get("lod").and_then(Value::as_u64).unwrap_or(0) != 0 {
        return Err(ApiError::invalid("this command stitches LOD0; export-glb --body stitches every LOD"));
    }
    let source = match text(input, "macro") {
        "" => None,
        p => Some(ch::read_macro_definition(Path::new(p)).map_err(ApiError::invalid)?),
    };
    let (dense, _, _, pin) = ch::evaluate_traced(&store, &spec, source.as_ref()).map_err(ApiError::invalid)?;
    let game_ref = spec.base.game_base.clone().ok_or_else(|| ApiError::invalid("the spec names no game base"))?;
    let (_, base) = store.load_game(&game_ref).map_err(ApiError::not_found)?;
    let (pack, controls, shaped_report, similarity) = match (text(input, "body"), &spec.body) {
        ("", Some(_)) => {
            let s = spec_body(&store, &spec, source.as_ref(), pin.as_ref()).map_err(ApiError::invalid)?;
            if let Some(path) = input.get("out_shaped").and_then(Value::as_str) {
                let world: Vec<Point> = s.pack.mesh.positions.iter().map(|&p| s.similarity.apply(p)).collect();
                gf_io::dense::write_points(Path::new(path), &world).map_err(ApiError::internal)?;
            }
            (s.pack, s.controls, Some((s.report, s.manifest)), s.similarity)
        }
        ("", None) => return Err(ApiError::invalid("pass body (a pack directory) or a spec with a body")),
        (dir, _) => (io::load(Path::new(dir)).map_err(ApiError::not_found)?.1, controls(input)?, None, Similarity::identity()),
    };
    let mut head: Vec<Point> = base.kept.iter().map(|&d| dense[d as usize]).collect();
    b::scale_about(&mut head, pack.pivot(), controls.head_scale);
    let seam = b::head_seam(&pack, &head, &base.triangles, &base.kept).map_err(ApiError::invalid)?;
    let mut st = b::stitch_lod(&pack, 0, &seam, &controls).map_err(ApiError::invalid)?;
    let planted = match input.get("plant").and_then(Value::as_str) {
        Some(kind) => Some(b::plant(&mut st, &Plant::parse(kind).map_err(ApiError::invalid)?, b::centroid(&seam.iter().map(|s| s.position).collect::<Vec<_>>())).map_err(ApiError::invalid)?),
        None => None,
    };
    let limits = if pack.mode == "makehuman" { Limits::exact() } else { Limits::any_body() };
    let (mut gates, failed) = b::gates(&head, &base.triangles, &seam, &st, &limits);
    gates["head_positions"] = json!("the character's dense positions at the game base's kept vertices (not placed); export-glb --body gates the placed head");
    if let Some(p) = &planted {
        gates["planted"] = json!(p);
    }
    if let Some(path) = input.get("out_obj").and_then(Value::as_str) {
        let mut text = String::from("o head\n");
        for p in head.iter().chain(&st.positions) {
            text.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
        }
        for t in &base.triangles {
            text.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
        }
        text.push_str("o body\n");
        let h = head.len() as u32;
        for t in &st.triangles {
            text.push_str(&format!("f {} {} {}\n", t[0] + h + 1, t[1] + h + 1, t[2] + h + 1));
        }
        std::fs::write(path, text).map_err(|e| ApiError::internal(format!("{path}: {e}")))?;
    }
    let mut result = json!({"gates": gates, "controls": controls, "body": {"mode": pack.mode, "vertices": st.positions.len(), "triangles": st.triangles.len(),
                            "seam_points": seam.len(), "seam_copies": st.seam_copies.len()}});
    if let Some((r, _)) = &shaped_report {
        result["body"]["shaped"] = r.clone();
    }
    result["body"]["world"] = json!({"scale": similarity.scale, "rotation_quaternion": similarity.quaternion(), "origin_m": similarity.translation_for_origin()});
    if input.get("return_mesh").and_then(Value::as_bool).unwrap_or(false) {
        let (_, mut normals) = b::two_sided_normals(&head, &base.triangles, &st);
        let mut positions = st.positions.clone();
        let mut triangles = st.triangles.clone();
        let mut hat_covers: Vec<HatCover> = Vec::new();
        let mut cover_head: Option<f64> = None;
        let mut worn = match (&shaped_report, spec.body.as_ref()) {
            (Some((_, manifest)), Some(b)) if !b.wearables.is_empty() => {
                let frame = crate::tasks::wearable::BaseFrame::new(&pack, &controls, manifest, &dense);
                let mut points = head.clone();
                points.extend_from_slice(&st.positions);
                let h = head.len() as u32;
                let tris: Vec<[u32; 3]> = base.triangles.iter().copied().chain(st.triangles.iter().map(|t| t.map(|v| v + h))).collect();
                let surface = crate::tasks::wearable::Surface::new(points, &tris);
                let mut covered = std::collections::HashSet::new();
                let mut dressed = Vec::new();
                for w in &b.wearables {
                    let item = store.load_wearable(&w.asset).map_err(ApiError::not_found)?;
                    let mut p = crate::tasks::wearable::place(&item, &frame, &pack, &controls).map_err(ApiError::invalid)?;
                    if w.hide_body {
                        covered.extend(item.binding.hidden.iter().copied());
                    }
                    let tris = crate::tasks::wearable::triangles_of(&item.faces);
                    if !item.manifest.transparent && item.manifest.attach.is_none() {
                        crate::tasks::wearable::push_out(&mut p, &tris, &surface, crate::tasks::wearable::CLEARANCE_M);
                    }
                    dressed.push((w, item, p, tris));
                }
                let head_z = gf_core::base_landmarks::on(&dense).ok().and_then(|l| Some((l.get(&234)?[2] + l.get(&454)?[2]) / 2.0));
                let mut cuts: Vec<Vec<bool>> = dressed.iter().map(|d| vec![false; d.2.len()]).collect();
                {
                    let mut layers: Vec<crate::tasks::wearable::Layered> = dressed
                        .iter_mut()
                        .zip(cuts.iter_mut())
                        .map(|((w, item, p, tris), cut)| crate::tasks::wearable::Layered {
                            triangles: tris.clone(),
                            depth: crate::tasks::wearable::layer_depth(&item.binding, w.layer),
                            opaque: !item.manifest.transparent && item.manifest.attach.is_none(),
                            transparent: item.manifest.transparent,
                            rigid: item.manifest.attach.is_some(),
                            hide_hair: w.hair == "hide",
                            cut: Some(cut),
                            positions: p,
                        })
                        .collect();
                    crate::tasks::wearable::layer(&mut layers, head_z);
                }
                for ((_, _, _, tris), cut) in dressed.iter_mut().zip(&cuts) {
                    tris.retain(|t| !t.iter().all(|&v| cut[v as usize]));
                }
                cover_head = head_z;
                hat_covers = dressed.iter().filter(|(_, item, ..)| !item.manifest.transparent).map(|(w, _, p, tris)| (p.clone(), tris.clone(), w.hair == "hide")).collect();
                let mut meshes = Vec::new();
                let strand_count = input.get("strands_preview").and_then(Value::as_u64).unwrap_or(STRANDS_PREVIEW) as usize;
                for ((w, item, p, tris), cut) in dressed.iter().zip(&cuts) {
                    if w.strands && item.manifest.transparent && item.manifest.attach.is_none() {
                        let texture = item.manifest.material.get("diffuse").and_then(|r| store.get_ref(r).ok()).and_then(|png| gf_io::png::decode_rgba(&png).ok());
                        if let Some((tw, th, rgba)) = texture {
                            let (ctris, uvs): (Vec<[u32; 3]>, Vec<[gf_geom::Uv; 3]>) = {
                                let (t, u) = crate::tasks::mesh::garment_triangles(item, p);
                                t.into_iter().zip(u).filter(|(t, _)| !t.iter().all(|&v| cut[v as usize])).unzip()
                            };
                            let (rank, _) = crate::tasks::strands::rank_on(&surface, p, &ctris);
                            let tex = crate::tasks::strands::Texture { width: tw, height: th, rgba: &rgba };
                            let (strands, report) = crate::tasks::strands::from_cards(p, &ctris, &uvs, &tex, strand_count, STRAND_PREVIEW_POINTS, 1, &rank);
                            let centre = head.iter().fold([0.0; 3], |a, q| [a[0] + q[0], a[1] + q[1], a[2] + q[2]]).map(|x| x / head.len().max(1) as f64);
                            let (rp, rn, rt) = crate::tasks::strands::ribbons(&strands, STRAND_PREVIEW_WIDTH_M, centre);
                            let colour = crate::tasks::wearable::hair_colour(&rgba).map(|c| c.map(|x| x / 255.0));
                            let idx: Vec<u8> = rt.iter().flatten().flat_map(|i| i.to_le_bytes()).collect();
                            meshes.push(json!({"asset": w.asset, "name": item.manifest.name, "hide_body": w.hide_body, "transparent": false, "strands": report["strands"], "color": colour,
                                               "positions": f32s(&rp), "normals": f32s(&rn), "indices": b64(&idx), "vertices": rp.len(), "triangles": rt.len()}));
                            continue;
                        }
                    }
                    let n = crate::tasks::wearable::vertex_normals(p, tris);
                    let idx: Vec<u8> = tris.iter().flatten().flat_map(|i| i.to_le_bytes()).collect();
                    meshes.push(json!({"asset": w.asset, "name": item.manifest.name, "hide_body": w.hide_body, "transparent": item.manifest.transparent,
                                       "positions": f32s(p), "normals": f32s(&n), "indices": b64(&idx), "vertices": p.len(), "triangles": tris.len()}));
                }
                if !covered.is_empty() {
                    let hidden = crate::tasks::wearable::hidden_body(&pack, 0, &st, &covered).map_err(ApiError::invalid)?;
                    triangles.retain(|t| !t.iter().all(|&v| hidden[v as usize]));
                }
                meshes
            }
            _ => Vec::new(),
        };
        if let Some(v) = spec.body.as_ref().and_then(|b| b.groom.as_ref()).filter(|_| text(input, "body").is_empty()) {
            let mut g = crate::tasks::groom::parse(v).map_err(|e| ApiError::invalid(format!("body.groom: {e}")))?;
            g.strands = g.strands.min(input.get("strands_preview").and_then(Value::as_u64).unwrap_or(STRANDS_PREVIEW) as usize);
            let mut points = head.clone();
            points.extend_from_slice(&st.positions);
            let h = head.len() as u32;
            let tris: Vec<[u32; 3]> = base.triangles.iter().copied().chain(st.triangles.iter().map(|t| t.map(|v| v + h))).collect();
            let surface = crate::tasks::wearable::Surface::new(points, &tris);
            let (mut strands, report) = crate::tasks::groom::grow(&g, &head, &base.triangles, &dense, &surface).map_err(ApiError::invalid)?;
            if let Some(head_z) = cover_head
                && !hat_covers.is_empty()
            {
                let covers: Vec<crate::tasks::wearable::HairCover> = hat_covers.iter().map(|(p, t, hide)| crate::tasks::wearable::HairCover { positions: p, triangles: t, hide: *hide }).collect();
                let top = dense.iter().copied().max_by(|a, b| a[2].total_cmp(&b[2])).unwrap_or([0.0, 0.0, head_z]);
                let under = crate::tasks::wearable::HairUnder::new(&covers, head_z, top);
                for s in strands.iter_mut() {
                    let (cut, _) = under.apply(&mut s.points);
                    let from = cut.iter().position(|c| !c).unwrap_or(cut.len());
                    s.points.drain(..from);
                    s.colours.drain(..from.min(s.colours.len()));
                }
                strands.retain(|s| s.points.len() >= 2);
            }
            let centre = head.iter().fold([0.0; 3], |a, q| [a[0] + q[0], a[1] + q[1], a[2] + q[2]]).map(|x| x / head.len().max(1) as f64);
            let (rp, rn, rt) = crate::tasks::strands::ribbons(&strands, STRAND_PREVIEW_WIDTH_M, centre);
            let srgb = g.colour.map(|c| if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 });
            let idx: Vec<u8> = rt.iter().flatten().flat_map(|i| i.to_le_bytes()).collect();
            worn.push(json!({"groom": g.name, "name": g.name, "hide_body": false, "transparent": false, "strands": report["strands"], "color": srgb,
                             "positions": f32s(&rp), "normals": f32s(&rn), "indices": b64(&idx), "vertices": rp.len(), "triangles": rt.len()}));
        }
        if spec.body.as_ref().is_some_and(|s| !s.genitals.is_empty()) && text(input, "body").is_empty() {
            let level = pack.fine.as_ref().map_or(2, |f| f.level);
            let g = crate::tasks::mesh::genital_mesh(&pack, &controls, level, &st.positions, &normals).map_err(ApiError::invalid)?;
            let offset = positions.len() as u32;
            triangles.extend(g.triangles.iter().map(|t| t.map(|i| i + offset)));
            positions.extend(&g.positions);
            normals.extend(&g.normals);
            result["body"]["genitals"] = json!({"vertices": g.positions.len(), "triangles": g.triangles.len(), "subdivision_level": level,
                                                "meaning": "drawn with the body mesh (appended after its vertices); export-glb writes it as its own Genitals_LODn node"});
        }
        let indices: Vec<u8> = triangles.iter().flatten().flat_map(|i| i.to_le_bytes()).collect();
        let mut lines = Vec::new();
        for m in 0..seam.len() {
            lines.push(seam[m].position);
            lines.push(seam[(m + 1) % seam.len()].position);
        }
        result["mesh"] = json!({"frame": "dense frame: metres, z up", "positions": f32s(&positions), "normals": f32s(&normals),
                                "indices": b64(&indices), "seam_lines": f32s(&lines), "head_positions": f32s(&head),
                                "head_scale": controls.head_scale, "wearables": worn});
    }
    if !failed.is_empty() && input.get("enforce").and_then(Value::as_bool).unwrap_or(true) {
        return Err(ApiError::new(Code::GateFailed, format!("stitch gates failed: {}", failed.join("; "))).details(result));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Vec<Point> {
        (0..10).flat_map(|i| (0..10).map(move |j| [i as f64 * 0.01, j as f64 * 0.01, 0.0])).collect()
    }

    #[test]
    fn face_bones_drop_into_their_nearest_kept_ancestor() {
        let bone = |name: &str, parent: Option<&str>| gf_core::body_rig::BoneDef {
            name: name.into(),
            parent: parent.map(str::to_string),
            head: gf_core::body_rig::JointRef::Vertex(0),
            tail: gf_core::body_rig::JointRef::Vertex(0),
            roll: 0.0,
            default_head: [0.0; 3],
            default_tail: [0.0; 3],
        };
        let bones = vec![bone("spine", None), bone("head", Some("spine")), bone("jaw", Some("head")), bone("tongue", Some("jaw")), bone("eye.L", Some("head")), bone("lip", Some("head")), bone("lid", Some("lip"))];
        let mut by_obj = std::collections::HashMap::new();
        by_obj.insert(1u32, vec![(5usize, 0.6), (1, 0.4)]);
        by_obj.insert(2u32, vec![(3usize, 0.7), (2, 0.3)]);
        by_obj.insert(3u32, vec![(0usize, 1.0)]);
        let mut br = BodyRig { rig: gf_core::body_rig::RigDef { bones }, heads: (0..7).map(|k| [k as f64, 0.0, 0.0]).collect(), by_obj, head_joint: 1, meta: Value::Null };
        let report = drop_face_bones(&mut br, &[2, 4, 6]);
        let names: Vec<&str> = br.rig.bones.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["spine", "head", "jaw", "eye.L", "lid"], "{report}");
        assert_eq!(br.rig.bones[4].parent.as_deref(), Some("head"), "a kept bone whose parent is dropped hangs from the parent's kept ancestor");
        assert_eq!(br.heads, vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [4.0, 0.0, 0.0], [6.0, 0.0, 0.0]]);
        assert_eq!(br.head_joint, 1);
        assert_eq!(br.by_obj[&1], vec![(1, 1.0)], "the lip's weight folds into the head");
        assert_eq!(br.by_obj[&2], vec![(2, 1.0)], "the tongue's folds into the jaw");
        assert_eq!(br.by_obj[&3], vec![(0, 1.0)], "a body vertex is untouched");
        assert_eq!(report["dropped"], 2);
    }

    #[test]
    fn a_joint_moves_with_the_head_near_it() {
        let reference = grid();
        let fitted: Vec<Point> = reference.iter().map(|p| [p[0] + 0.004, p[1], p[2] + 0.002]).collect();
        let fit = HeadFit { reference: reference.clone(), fitted, ring_offset_mm: 0.0 };
        let d = fit.shift([0.045, 0.045, 0.01]);
        assert!((d[0] - 0.004).abs() < 1e-12 && d[1].abs() < 1e-12 && (d[2] - 0.002).abs() < 1e-12, "a rigid shift moves the joint by exactly that: {d:?}");
        let local: Vec<Point> = reference.iter().map(|p| if p[0] < 0.03 { [p[0], p[1], p[2] + 0.01] } else { *p }).collect();
        let fit = HeadFit { reference, fitted: local, ring_offset_mm: 0.0 };
        assert!((fit.shift([0.0, 0.05, 0.005])[2] - 0.01).abs() < 1e-9, "a joint over the raised part rises with it");
        assert!(fit.shift([0.09, 0.05, 0.005])[2].abs() < 1e-9, "a joint over the still part stays");
    }
}
