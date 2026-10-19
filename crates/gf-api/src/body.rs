use std::collections::BTreeMap;
use std::path::Path;

use gf_core::body::{self as b, Controls, Limits, Plant, Point};
use gf_core::character::Spec;
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
            "weight_mode": {"enum": ["gf", "mpfb"], "description": "macro weight mode (default gf)"}
        }),
        &["mode", "obj", "dense", "out"],
    )
}

pub fn import_output() -> Value {
    shape(json!({"out": {"type": "string"}, "manifest": {"type": "object"}}))
}

fn shaping(input: &Value) -> Result<Vec<io::Shaping>, ApiError> {
    let (root, def) = (text(input, "targets"), text(input, "macro"));
    if root.is_empty() && def.is_empty() {
        return Ok(Vec::new());
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
    Ok(out)
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
            let shaping = shaping(input)?;
            let mh = io::MakeHumanInput { obj: path, group, unit: number(input, "unit_m", 0.1)?, shaping, lods: &lods };
            let mut m = io::import_makehuman(&mh, dense, out).map_err(ApiError::invalid)?;
            if let Some(s) = input.get("sliders") {
                m["source"]["sliders"] = s.clone();
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
    Ok(json!({"out": out.display().to_string(), "manifest": manifest}))
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
        "character": {"type": "object", "description": "a spec object, or the designer's character (store, pack, catalog, game, weights, ...), instead of spec"},
        "macro": {"type": "string", "description": "the macro definition, for a spec with macro sliders"},
        "body": {"type": "string", "minLength": 1, "description": "a body pack directory written by body.import"},
        "lod": {"type": "integer", "description": "0 (this command stitches the finest LOD; export-glb --body stitches every LOD)"},
        "out_obj": {"type": "string", "description": "write the head and the stitched body as one OBJ"},
        "return_mesh": {"type": "boolean", "description": "return the stitched body (base64 f32 positions and normals, u32 indices), the seam as overlay lines and the head positions it was stitched to (head_scale applied, game LOD0 order)"},
        "enforce": {"type": "boolean", "description": "fail with gate_failed when a gate fails (default true)"},
        "plant": {"enum": ["gap", "crack", "crease", "flip", "penetrate"], "description": "gate self-test: plant this defect after stitching; the gates must fail"}
    });
    for (k, v) in control_props().as_object().unwrap() {
        props[k] = v.clone();
    }
    obj(props, &["store", "body"])
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

pub fn stitch(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = text(input, "store");
    if !Path::new(root).join("packs").is_dir() {
        return Err(ApiError::not_found(format!("{root} is not a store")));
    }
    let store = Store::open(Path::new(root)).map_err(ApiError::internal)?;
    let spec: Spec = match (input.get("character"), text(input, "spec")) {
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
    };
    if input.get("lod").and_then(Value::as_u64).unwrap_or(0) != 0 {
        return Err(ApiError::invalid("this command stitches LOD0; export-glb --body stitches every LOD"));
    }
    let source = match text(input, "macro") {
        "" => None,
        p => Some(ch::read_macro_definition(Path::new(p)).map_err(ApiError::invalid)?),
    };
    let (dense, _) = ch::evaluate(&store, &spec, source.as_ref()).map_err(ApiError::invalid)?;
    let game_ref = spec.base.game_base.clone().ok_or_else(|| ApiError::invalid("the spec names no game base"))?;
    let (_, base) = store.load_game(&game_ref).map_err(ApiError::not_found)?;
    let (_, pack) = io::load(Path::new(text(input, "body"))).map_err(ApiError::not_found)?;
    let controls = controls(input)?;
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
