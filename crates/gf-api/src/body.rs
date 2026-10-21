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

fn lods(input: &Value) -> Result<Vec<usize>, ApiError> {
    match input.get("lods") {
        None | Some(Value::Null) => Ok(DEFAULT_LODS.to_vec()),
        Some(Value::Array(a)) => {
            let v: Vec<usize> = a.iter().map(|x| x.as_u64().map(|n| n as usize)).collect::<Option<_>>().ok_or_else(|| ApiError::invalid("lods must be vertex counts"))?;
            if v.windows(2).any(|w| w[1] >= w[0]) || v.iter().any(|&n| n < 100) {
                return Err(ApiError::invalid("lods must be decreasing vertex counts of at least 100"));
            }
            Ok(v)
        }
        _ => Err(ApiError::invalid("lods must be an array")),
    }
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
            "lods": {"type": "array", "items": {"type": "integer"}, "description": "vertex counts of the coarser body LODs (default [3500, 1400])"},
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
    let lods = lods(input)?;
    let manifest = match text(input, "mode") {
        "makehuman" => {
            let group = input.get("group").and_then(Value::as_str).unwrap_or("body");
            let (shaping, used_sliders) = shaping(input)?;
            let mh = io::MakeHumanInput { obj: path, group, unit: number(input, "unit_m", 0.1)?, shaping, lods: &lods };
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
    shape(json!({"pairs": {"type": "array"}, "compounds": {"type": "array"}, "shared_with_head": {"type": "integer"}}))
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
        if e.family == "macrodetails" || e.family == "breast" {
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
    Ok(json!({"pairs": pairs.into_values().collect::<Vec<_>>(), "compounds": compounds, "shared_with_head": shared}))
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
    let (_, _, _, pin) = ch::evaluate_traced(&store, &spec, source.as_ref()).map_err(ApiError::invalid)?;
    let s = spec_body(&store, &spec, source.as_ref(), pin.as_ref()).map_err(ApiError::invalid)?;
    if s.pack.helpers.positions.is_empty() {
        return Err(ApiError::invalid("the body carries no shaped helper vertices: re-import the body (body.import) and its targets (body.targets) so the joints follow it"));
    }
    let at = gf_core::body_rig::Resolver::new(&s.pack.mesh.positions, &s.pack.source, &s.pack.helpers);
    let ends = rig.evaluate(&at).map_err(ApiError::invalid)?;
    let r6 = |p: Point| p.map(|x| (x * 1e6).round() / 1e6);
    let bones: Vec<Value> = rig
        .bones
        .iter()
        .zip(&ends)
        .map(|(b, (h, t))| json!({"name": b.name, "parent": b.parent, "head": r6(s.similarity.apply(*h)), "tail": r6(s.similarity.apply(*t)), "roll": b.roll,
                                  "head_canonical": r6(*h), "tail_canonical": r6(*t)}))
        .collect();
    Ok(json!({"bones": bones, "frame": "metres, z up: head/tail in the world (the character's root similarity applied), *_canonical in the head's canonical frame as the body is stored",
              "body": s.report}))
}

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
            (s.pack, s.controls, Some(s.report), s.similarity)
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
    if let Some(r) = shaped_report {
        result["body"]["shaped"] = r;
    }
    result["body"]["world"] = json!({"scale": similarity.scale, "rotation_quaternion": similarity.quaternion(), "origin_m": similarity.translation_for_origin()});
    if input.get("return_mesh").and_then(Value::as_bool).unwrap_or(false) {
        let (_, normals) = b::two_sided_normals(&head, &base.triangles, &st);
        let indices: Vec<u8> = st.triangles.iter().flatten().flat_map(|i| i.to_le_bytes()).collect();
        let mut lines = Vec::new();
        for m in 0..seam.len() {
            lines.push(seam[m].position);
            lines.push(seam[(m + 1) % seam.len()].position);
        }
        result["mesh"] = json!({"frame": "dense frame: metres, z up", "positions": f32s(&st.positions), "normals": f32s(&normals),
                                "indices": b64(&indices), "seam_lines": f32s(&lines), "head_positions": f32s(&head),
                                "head_scale": controls.head_scale});
    }
    if !failed.is_empty() && input.get("enforce").and_then(Value::as_bool).unwrap_or(true) {
        return Err(ApiError::new(Code::GateFailed, format!("stitch gates failed: {}", failed.join("; "))).details(result));
    }
    Ok(result)
}
