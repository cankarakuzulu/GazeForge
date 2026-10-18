use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use gf_core::character::{FORMAT, Spec, SpecBase};
use gf_core::fit::{self, Adjacency, OrthoCamera, Point, PointGrid, Similarity2};
use gf_core::gates::{self, ShellLimits};
use gf_io::character as ch;
use gf_io::dense::{self as dio, Dense};
use gf_io::store::Store;
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{obj, shape, text};
use crate::designer::HEAD_CATEGORIES;
use crate::error::{ApiError, Code};

pub const STABLE_LANDMARKS: [usize; 30] =
    [33, 133, 362, 263, 159, 145, 386, 374, 168, 6, 197, 195, 5, 4, 1, 2, 98, 327, 61, 291, 0, 17, 13, 14, 37, 267, 84, 314, 199, 175];
const NONE: u32 = u32::MAX;

pub(crate) fn mm(p: Point) -> Point {
    fit::scale(p, 1000.0)
}

pub(crate) fn opt<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

pub(crate) fn num(input: &Value, key: &str, default: f64) -> f64 {
    input.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn tool(input: &Value, key: &str, env: &str, default: &str) -> String {
    opt(input, key).map(str::to_string).or_else(|| std::env::var(env).ok()).unwrap_or_else(|| default.to_string())
}

pub(crate) fn run_dir(input: &Value) -> Result<PathBuf, ApiError> {
    let dir = PathBuf::from(text(input, "run"));
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?;
    Ok(dir)
}

pub(crate) fn read_json(path: &Path) -> Result<Value, ApiError> {
    let body = std::fs::read_to_string(path)
        .map_err(|e| ApiError::not_found(format!("{}: {e}", path.display())).hint("run the stage that writes it first"))?;
    serde_json::from_str(&body).map_err(|e| ApiError::internal(format!("{}: {e}", path.display())))
}

pub(crate) fn write_json(path: &Path, value: &Value) -> Result<(), ApiError> {
    let body = serde_json::to_string_pretty(value).map_err(|e| ApiError::internal(e.to_string()))?;
    std::fs::write(path, body).map_err(|e| ApiError::internal(format!("{}: {e}", path.display())))
}

fn points(path: &Path) -> Result<Vec<Point>, ApiError> {
    dio::read_points(path).map_err(ApiError::not_found)
}

pub(crate) fn tool_props() -> Value {
    json!({
        "run": {"type": "string", "minLength": 1, "description": "run directory every stage reads from and writes to"},
        "python": {"type": "string", "description": "Python with numpy, opencv and mediapipe (else GAZEFORGE_PYTHON, else python)"},
        "workers": {"type": "string", "description": "directory of the worker scripts (else GAZEFORGE_WORKERS, else ./workers)"},
        "landmarker": {"type": "string", "description": "MediaPipe face landmarker .task model (else GAZEFORGE_FACE_LANDMARKER)"},
        "server": {"type": "string", "description": "image-to-mesh server URL (else GAZEFORGE_MESH_SERVER)"},
        "graph": {"type": "string", "description": "the server's multi-view graph template, JSON (else GAZEFORGE_MESH_GRAPH)"}
    })
}

pub(crate) fn with_tools(extra: Value) -> Value {
    let mut base = tool_props();
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

pub(crate) fn store_props() -> Value {
    json!({"store": {"type": "string", "minLength": 1}, "pack": {"type": "string", "minLength": 1}, "catalog": {"type": "string", "minLength": 1}})
}

pub(crate) fn merged(a: Value, b: Value) -> Value {
    let mut a = a;
    if let (Some(x), Some(y)) = (a.as_object_mut(), b.as_object()) {
        for (k, v) in y {
            x.insert(k.clone(), v.clone());
        }
    }
    a
}

pub fn worker(input: &Value, script: &str, sub: &str, args: Value, run: &Path) -> Result<Value, ApiError> {
    let python = tool(input, "python", "GAZEFORGE_PYTHON", "python");
    let dir = tool(input, "workers", "GAZEFORGE_WORKERS", "workers");
    let path = Path::new(&dir).join(script);
    if !path.is_file() {
        return Err(ApiError::not_found(format!("worker {} not found", path.display())).hint("pass workers or set GAZEFORGE_WORKERS"));
    }
    let mut args = args;
    args["run"] = json!(run.display().to_string());
    args["landmarker"] = json!(tool(input, "landmarker", "GAZEFORGE_FACE_LANDMARKER", "face_landmarker.task"));
    let args_path = run.join(format!("{sub}.args.json"));
    write_json(&args_path, &args)?;
    let mut cmd = Command::new(&python);
    cmd.arg("-I").arg(&path).arg(sub).arg(&args_path).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::process::hide_window(&mut cmd);
    let output = cmd.output().map_err(|e| ApiError::new(Code::WorkerFailed, format!("could not start {python}: {e}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let _ = std::fs::write(run.join(format!("{sub}.log")), format!("{stdout}\n--- stderr ---\n{stderr}"));
    let last = stdout.lines().rev().find(|l| l.trim_start().starts_with('{')).unwrap_or("");
    let parsed: Option<Value> = serde_json::from_str(last).ok();
    match (output.status.success(), parsed) {
        (true, Some(v)) => Ok(v),
        (_, Some(v)) if v.get("ok") == Some(&Value::Bool(false)) => {
            let code = v.pointer("/error/code").and_then(Value::as_str).and_then(Code::parse).unwrap_or(Code::WorkerFailed);
            let message = v.pointer("/error/message").and_then(Value::as_str).unwrap_or("the worker failed").to_string();
            Err(ApiError::new(code, format!("{sub}: {message}")).details(v))
        }
        _ => {
            let tail: String = stderr.chars().rev().take(1500).collect::<Vec<_>>().into_iter().rev().collect();
            Err(ApiError::new(Code::WorkerFailed, format!("{sub} exited {:?} without a result", output.status.code())).details(json!({"stderr": tail})))
        }
    }
}

pub fn ingest_input() -> Value {
    obj(
        with_tools(json!({
            "front": {"type": "string", "minLength": 1, "description": "front photo"},
            "profile": {"type": "string", "minLength": 1, "description": "profile photo"},
            "profile_side": {"type": "string", "enum": ["right", "left"], "description": "which side of the face the profile shows (default right)"},
            "max_ratio_gap": {"type": "number", "minimum": 0, "maximum": 1, "description": "front vs profile proportion tolerance (default 0.12)"}
        })),
        &["front", "profile", "run"],
    )
}

pub fn generic_output() -> Value {
    shape(json!({"run": {"type": "string"}, "seconds": {"type": "number"}}))
}

pub fn ingest(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = run_dir(input)?;
    let args = json!({"front": text(input, "front"), "profile": text(input, "profile"),
                      "profile_side": opt(input, "profile_side").unwrap_or("right"), "max_ratio_gap": num(input, "max_ratio_gap", 0.12)});
    let result = worker(input, "photo_worker.py", "ingest", args, &run)?;
    if result.pointer("/facing/pass") != Some(&Value::Bool(true)) {
        let failures = result.pointer("/facing/failures").cloned().unwrap_or(json!([]));
        return Err(ApiError::new(Code::GateFailed, format!("the facing check failed: {failures}"))
            .details(result)
            .hint("check that the profile shows the declared side and that both photos are of one face; the run stops here"));
    }
    Ok(result)
}

pub fn landmarks_input() -> Value {
    obj(tool_props(), &["run"])
}

pub fn landmarks(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = run_dir(input)?;
    worker(input, "photo_worker.py", "landmarks", json!({}), &run)
}

pub fn mesh_input() -> Value {
    obj(with_tools(json!({"glb": {"type": "string", "description": "reuse this mesh instead of calling the server"}, "shell_mode": shell_mode_prop()})), &["run"])
}

pub fn mesh(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = run_dir(input)?;
    let ingest = read_json(&run.join("ingest.json"))?;
    let dir = run.join("mesh");
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(e.to_string()))?;
    let glb = dir.join("source.glb");
    let started = Instant::now();
    let generated = match opt(input, "glb") {
        Some(given) => {
            std::fs::copy(given, &glb).map_err(|e| ApiError::not_found(format!("{given}: {e}")))?;
            json!({"reused": given})
        }
        None => {
            let server = tool(input, "server", "GAZEFORGE_MESH_SERVER", "");
            let graph = tool(input, "graph", "GAZEFORGE_MESH_GRAPH", "");
            if server.is_empty() || graph.is_empty() {
                return Err(ApiError::invalid("the image-to-mesh server and its graph are not configured")
                    .hint("pass server and graph, or set GAZEFORGE_MESH_SERVER and GAZEFORGE_MESH_GRAPH, or pass glb"));
            }
            let args = json!({"server": server, "graph": graph, "front": ingest["front"]["path"], "profile": ingest["profile"]["upright_path"],
                              "out": glb.display().to_string()});
            worker(input, "mesh_worker.py", "mesh", args, &run)?
        }
    };
    let mesh_seconds = started.elapsed().as_secs_f64();
    let shell = worker(input, "photo_worker.py", "shell", json!({"glb": glb.display().to_string(), "shell_mode": opt(input, "shell_mode")}), &run)?;
    Ok(json!({"glb": glb.display().to_string(), "mesh": generated, "mesh_seconds": mesh_seconds, "shell": shell}))
}

pub(crate) struct Base {
    pub(crate) store: Store,
    pub(crate) dense: Dense,
    pub(crate) quads: Vec<[u32; 4]>,
    pub(crate) tip: Point,
}

pub(crate) fn load_base(input: &Value) -> Result<Base, ApiError> {
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::not_found)?;
    let (_, dense) = store.load_pack(text(input, "pack")).map_err(ApiError::not_found)?;
    let quads: Vec<[u32; 4]> = dense
        .triangles
        .chunks(2)
        .map(|pair| match pair {
            [a, b] if b[0] == a[0] && b[1] == a[2] => Ok([a[0], a[1], a[2], b[2]]),
            _ => Err(ApiError::invalid("the shell gates read quads only; the pack's faces are not all quads")),
        })
        .collect::<Result<_, _>>()?;
    let tip = gates::nose_tip(&dense.positions);
    Ok(Base { store, dense, quads, tip })
}

pub(crate) fn landmark_vertices(run: &Path) -> Result<BTreeMap<usize, usize>, ApiError> {
    let v = read_json(&run.join("base_landmarks.json"))?;
    let map = v["vertices"].as_object().ok_or_else(|| ApiError::internal("base_landmarks.json has no vertices"))?;
    Ok(map.iter().filter_map(|(k, x)| Some((k.parse().ok()?, x.as_u64()? as usize))).collect())
}

fn photo_points(run: &Path) -> Result<(Vec<[f64; 2]>, Value), ApiError> {
    let v = read_json(&run.join("landmarks.json"))?;
    let pts = v["front"]["points"].as_array().ok_or_else(|| ApiError::internal("landmarks.json has no front points"))?;
    let px = pts.iter().map(|p| [p[0].as_f64().unwrap_or(f64::NAN), p[1].as_f64().unwrap_or(f64::NAN)]).collect();
    Ok((px, v))
}

fn shell_mode_prop() -> Value {
    json!({"type": "string", "enum": ["outer_triangles", "visible_vertices"],
           "description": "outer-shell extraction: triangles front-most from 26 directions with normals oriented to their viewers (default), or vertices visible from them"})
}

fn base_table_prop() -> Value {
    json!({"base_landmarks": {"type": "string", "description": "the base's landmark table, JSON {\"vertices\": {landmark index: dense vertex}} (else detected on a render of the base)"}})
}

pub fn register_input() -> Value {
    obj(merged(merged(tool_props(), store_props()), base_table_prop()), &["run", "store", "pack"])
}

pub fn register(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = run_dir(input)?;
    let started = Instant::now();
    let base = load_base(input)?;
    let p = &base.dense.positions;
    if let Some(table) = opt(input, "base_landmarks") {
        let dest = run.join("base_landmarks.json");
        let same = std::fs::canonicalize(table).ok().zip(std::fs::canonicalize(&dest).ok()).is_some_and(|(a, b)| a == b);
        if !same {
            let body = read_json(Path::new(table))?;
            if !body["vertices"].is_object() {
                return Err(ApiError::invalid(format!("{table} has no vertices object")));
            }
            write_json(&dest, &body)?;
        }
    }
    if !run.join("base_landmarks.json").is_file() {
        let normals = gates::vertex_normals(p, &base.quads);
        dio::write_points(&run.join("base_positions.npy"), p).map_err(ApiError::internal)?;
        dio::write_points(&run.join("base_normals.npy"), &normals).map_err(ApiError::internal)?;
        worker(input, "photo_worker.py", "base", json!({}), &run)?;
    }
    let base_lm = landmark_vertices(&run)?;
    let shell_lm = read_json(&run.join("shell_landmarks.json"))?;
    let mut src = Vec::new();
    let mut dst = Vec::new();
    let mut used = Vec::new();
    for &i in &STABLE_LANDMARKS {
        if let (Some(&v), Some(q)) = (base_lm.get(&i), shell_lm["points"].get(i.to_string()).and_then(Value::as_array)) {
            let q: Vec<f64> = q.iter().filter_map(Value::as_f64).collect();
            if q.len() == 3 {
                src.push([q[0], q[1], q[2]]);
                dst.push(p[v]);
                used.push(i);
            }
        }
    }
    let mut sim = fit::similarity(&src, &dst, &vec![1.0; src.len()], true).map_err(ApiError::invalid)?;
    let shell = points(&run.join("shell_points.npy"))?;
    let shell_n = points(&run.join("shell_normals.npy"))?;
    let tip = base.tip;
    let in_face = |q: Point| {
        let r = mm(fit::sub(q, tip));
        r[0].abs() < 65.0 && r[2] > -95.0 && r[2] < 45.0 && r[1] < 60.0
    };
    let face: Vec<Point> = p.iter().copied().filter(|q| in_face(*q)).collect();
    let grid = PointGrid::new(face, 0.002);
    let stride = (shell.len() / 60000).max(1);
    let mut icp_rms = f64::NAN;
    let mut icp_pairs = 0;
    for _ in 0..15 {
        let mut pairs: Vec<(Point, Point, f64)> = Vec::new();
        for q in shell.iter().step_by(stride) {
            let t = sim.apply(*q);
            if !in_face(t) {
                continue;
            }
            if let Some((i, d)) = grid.nearest(t, 0.008) {
                pairs.push((*q, grid.points()[i], d));
            }
        }
        if pairs.len() < 100 {
            break;
        }
        let mut ds: Vec<f64> = pairs.iter().map(|x| x.2).collect();
        ds.sort_by(f64::total_cmp);
        let cut = ds[(ds.len() as f64 * 0.8) as usize];
        let kept: Vec<&(Point, Point, f64)> = pairs.iter().filter(|x| x.2 <= cut).collect();
        icp_pairs = kept.len();
        icp_rms = (kept.iter().map(|x| x.2 * x.2).sum::<f64>() / kept.len() as f64).sqrt() * 1000.0;
        let lw = 0.5 * kept.len() as f64 / src.len() as f64;
        let mut a: Vec<Point> = kept.iter().map(|x| x.0).collect();
        let mut b: Vec<Point> = kept.iter().map(|x| x.1).collect();
        let mut w = vec![1.0; a.len()];
        a.extend_from_slice(&src);
        b.extend_from_slice(&dst);
        w.extend(std::iter::repeat_n(lw, src.len()));
        sim = fit::similarity(&a, &b, &w, true).map_err(ApiError::invalid)?;
    }
    let residuals: Vec<f64> = src.iter().zip(&dst).map(|(s, d)| fit::norm(fit::sub(sim.apply(*s), *d)) * 1000.0).collect();
    let per: BTreeMap<String, f64> = used.iter().zip(&residuals).map(|(i, r)| (i.to_string(), *r)).collect();
    let rms = (residuals.iter().map(|r| r * r).sum::<f64>() / residuals.len().max(1) as f64).sqrt();
    let moved: Vec<Point> = shell.iter().map(|q| sim.apply(*q)).collect();
    let turned: Vec<Point> = shell_n.iter().map(|n| sim.rotate(*n)).collect();
    dio::write_points(&run.join("shell_registered.npy"), &moved).map_err(ApiError::internal)?;
    dio::write_points(&run.join("shell_registered_normals.npy"), &turned).map_err(ApiError::internal)?;
    let report = json!({"similarity": sim, "landmarks_used": used.len(), "landmark_rms_mm": rms,
                        "landmark_max_mm": residuals.iter().copied().fold(0.0, f64::max), "landmark_mm": per,
                        "icp_rms_mm": icp_rms, "icp_pairs": icp_pairs, "seconds": started.elapsed().as_secs_f64()});
    write_json(&run.join("register.json"), &report)?;
    Ok(report)
}

struct Problem {
    ids: Vec<String>,
    slots: Vec<u32>,
    sample: Vec<usize>,
    deltas: Vec<f32>,
}

impl Problem {
    fn t(&self) -> usize {
        self.ids.len()
    }

    fn row(&self, s: usize, k: usize) -> &[f32] {
        let t = self.t();
        &self.deltas[(s * 3 + k) * t..(s * 3 + k + 1) * t]
    }

    fn position(&self, base: &[Point], s: usize, w: &[f64]) -> Point {
        let mut p = base[self.sample[s]];
        for (k, c) in p.iter_mut().enumerate() {
            *c += self.row(s, k).iter().zip(w).map(|(d, x)| *d as f64 * x).sum::<f64>();
        }
        p
    }
}

pub(crate) fn head_ids(base: &Base, catalog_ref: &str) -> Result<(gf_core::catalog::CatalogManifest, Vec<String>), ApiError> {
    let catalog = base.store.load_catalog(catalog_ref).map_err(ApiError::not_found)?;
    let ids = catalog.targets.iter().filter(|(_, e)| HEAD_CATEGORIES.contains(&e.category.as_str())).map(|(k, _)| k.clone()).collect();
    Ok((catalog, ids))
}

struct Accum {
    t: usize,
    gram: Vec<f64>,
    rhs: Vec<f64>,
}

impl Accum {
    fn new(t: usize) -> Accum {
        Accum { t, gram: vec![0.0; t * t], rhs: vec![0.0; t] }
    }

    fn add(&mut self, a: &[f64], b: f64, weight: f64) {
        let nz: Vec<(usize, f64)> = a.iter().enumerate().filter(|(_, x)| **x != 0.0).map(|(i, x)| (i, *x)).collect();
        for &(i, x) in &nz {
            self.rhs[i] += weight * x * b;
            for &(j, y) in &nz {
                self.gram[i * self.t + j] += weight * x * y;
            }
        }
    }
}

enum FrontCam {
    Ortho(OrthoCamera),
    Persp(fit::PerspectiveCamera),
}

impl FrontCam {
    fn solve(model: &str, focal: Option<f64>, size: [f64; 2], x: &[Point], u: &[[f64; 2]]) -> Result<FrontCam, ApiError> {
        match model {
            "orthographic" => fit::fit_ortho_camera(x, u).map(FrontCam::Ortho).map_err(ApiError::invalid),
            "perspective" if size[0] > 0.0 && size[1] > 0.0 => {
                fit::fit_perspective_camera(x, u, [size[0] / 2.0, size[1] / 2.0], focal).map(FrontCam::Persp).map_err(ApiError::invalid)
            }
            "perspective" => Err(ApiError::invalid("a perspective camera needs the photo size (landmarks.json front.size)")),
            other => Err(ApiError::invalid(format!("unknown camera model {other}")).hint("perspective or orthographic")),
        }
    }

    fn project(&self, x: Point) -> [f64; 2] {
        match self {
            FrontCam::Ortho(c) => c.project(x),
            FrontCam::Persp(c) => c.project(x),
        }
    }

    fn scale_at(&self, x: Point) -> f64 {
        match self {
            FrontCam::Ortho(c) => c.scale,
            FrontCam::Persp(c) => c.pixels_per_metre(x),
        }
    }

    fn focal(&self) -> Option<f64> {
        match self {
            FrontCam::Ortho(_) => None,
            FrontCam::Persp(c) => Some(c.focal),
        }
    }

    fn to_json(&self) -> Value {
        match self {
            FrontCam::Ortho(c) => json!(c),
            FrontCam::Persp(c) => {
                let mut v = json!(c);
                v["kind"] = json!("perspective");
                v
            }
        }
    }
}

struct Profile {
    to_model: Similarity2,
    rows: Vec<(usize, f64)>,
    rms_mm: f64,
}

fn silhouette(prob: &Problem, base: &[Point], w: &[f64], midline: &[usize], tip: Point) -> BTreeMap<i64, (usize, f64)> {
    let mut bins: BTreeMap<i64, (usize, f64)> = BTreeMap::new();
    for &s in midline {
        let x = prob.position(base, s, w);
        let bin = ((x[2] - tip[2]) * 1000.0).round() as i64;
        let forward = -x[1];
        if bins.get(&bin).is_none_or(|b| forward > b.1) {
            bins.insert(bin, (s, forward));
        }
    }
    bins
}

fn scale_translation2(src: &[[f64; 2]], dst: &[[f64; 2]]) -> Result<Similarity2, String> {
    if src.len() != dst.len() || src.len() < 2 {
        return Err("a scale and translation needs at least two matched points".into());
    }
    let n = src.len() as f64;
    let ms = [src.iter().map(|p| p[0]).sum::<f64>() / n, src.iter().map(|p| p[1]).sum::<f64>() / n];
    let md = [dst.iter().map(|p| p[0]).sum::<f64>() / n, dst.iter().map(|p| p[1]).sum::<f64>() / n];
    let (mut num, mut den) = (0.0, 0.0);
    for (a, b) in src.iter().zip(dst) {
        num += (a[0] - ms[0]) * (b[0] - md[0]) + (a[1] - ms[1]) * (b[1] - md[1]);
        den += (a[0] - ms[0]).powi(2) + (a[1] - ms[1]).powi(2);
    }
    if den <= 0.0 || num <= 0.0 {
        return Err("the profile points are degenerate".into());
    }
    let scale = num / den;
    Ok(Similarity2 { scale, angle: 0.0, translation: [md[0] - scale * ms[0], md[1] - scale * ms[1]] })
}

fn fit_profile(curve: &BTreeMap<i64, (usize, f64)>, line: &[[f64; 2]], keys: &Value, tip: Point, previous: Option<&Similarity2>, upright: bool, refine: bool) -> Result<Profile, ApiError> {
    let pair = |a: &[[f64; 2]], b: &[[f64; 2]]| if upright { scale_translation2(a, b) } else { fit::similarity2(a, b) };
    let model = |bin: i64| curve.get(&bin).map(|c| [c.1, -(tip[2] + bin as f64 / 1000.0)]);
    let to_model = match previous {
        Some(p) => p.clone(),
        None => {
            let key = |k: &str| keys.get(k).and_then(Value::as_array).map(|a| [a[0].as_f64().unwrap_or(0.0), a[1].as_f64().unwrap_or(0.0)]);
            let (pt, pn) = (key("nose_tip").ok_or_else(|| ApiError::invalid("the profile has no nose_tip"))?, key("nasion").ok_or_else(|| ApiError::invalid("the profile has no nasion"))?);
            let tip_bin = curve.range(-20..=10).max_by(|a, b| a.1.1.total_cmp(&b.1.1)).map(|(k, _)| *k).ok_or_else(|| ApiError::invalid("no model profile near the nose"))?;
            let nasion_bin = curve.range(tip_bin + 8..=tip_bin + 50).min_by(|a, b| a.1.1.total_cmp(&b.1.1)).map(|(k, _)| *k).ok_or_else(|| ApiError::invalid("no model nasion"))?;
            pair(&[pt, pn], &[model(tip_bin).unwrap_or([0.0; 2]), model(nasion_bin).unwrap_or([0.0; 2])]).map_err(ApiError::invalid)?
        }
    };
    let upper: Vec<[f64; 2]> = curve.range(-12..=40).filter_map(|(b, _)| model(*b)).collect();
    let mut sim = to_model;
    for _ in 0..if refine || previous.is_none() { 25 } else { 0 } {
        let mut a = Vec::new();
        let mut b = Vec::new();
        for u in line {
            let m = sim.apply(*u);
            let z = (-m[1] - tip[2]) * 1000.0;
            if !(-12.0..=40.0).contains(&z) {
                continue;
            }
            if let Some(best) = upper.iter().min_by(|x, y| {
                let dx = (x[0] - m[0]).powi(2) + (x[1] - m[1]).powi(2);
                let dy = (y[0] - m[0]).powi(2) + (y[1] - m[1]).powi(2);
                dx.total_cmp(&dy)
            }) {
                a.push(*u);
                b.push(*best);
            }
        }
        if a.len() < 5 {
            break;
        }
        sim = pair(&a, &b).map_err(ApiError::invalid)?;
    }
    let mut rows = Vec::new();
    let mut sq = Vec::new();
    for u in line {
        let m = sim.apply(*u);
        let z = (-m[1] - tip[2]) * 1000.0;
        if !(-95.0..=40.0).contains(&z) {
            continue;
        }
        if let Some(&(s, f)) = curve.get(&(z.round() as i64)) {
            rows.push((s, m[0]));
            sq.push(((f - m[0]) * 1000.0).powi(2));
        }
    }
    let rms_mm = (sq.iter().sum::<f64>() / sq.len().max(1) as f64).sqrt();
    Ok(Profile { to_model: sim, rows, rms_mm })
}

pub(crate) fn eye_centres(p: &[Point], lm: &BTreeMap<usize, usize>) -> Vec<Point> {
    let mut out = Vec::new();
    for pair in [(33, 133), (362, 263)] {
        if let (Some(&a), Some(&b)) = (lm.get(&pair.0), lm.get(&pair.1)) {
            let m = fit::scale(fit::add(p[a], p[b]), 0.5);
            out.push(fit::add(m, [0.0, 0.011, 0.0]));
        }
    }
    out
}

fn polyline_mm(q: Point, line: &[Point]) -> f64 {
    line.windows(2)
        .map(|w| {
            let ab = fit::sub(w[1], w[0]);
            let t = (fit::dot(fit::sub(q, w[0]), ab) / fit::dot(ab, ab).max(1e-18)).clamp(0.0, 1.0);
            fit::norm(fit::sub(q, fit::add(w[0], fit::scale(ab, t)))) * 1000.0
        })
        .fold(f64::INFINITY, f64::min)
}

pub(crate) fn eye_gap_mm(q: Point, eyes: &[Point]) -> f64 {
    eyes.iter().map(|e| fit::norm(fit::sub(q, *e)) * 1000.0).fold(f64::INFINITY, f64::min)
}

pub fn targets_input() -> Value {
    obj(
        merged(
            tool_props(),
            merged(
                store_props(),
                json!({"rounds": {"type": "integer", "minimum": 1, "maximum": 20},
                       "shell_weight": {"type": "number", "minimum": 0}, "landmark_weight": {"type": "number", "minimum": 0},
                       "profile_weight": {"type": "number", "minimum": 0}, "ridge": {"type": "number", "minimum": 0},
                       "prior_weight": {"type": "number", "minimum": 0}, "symmetry_weight": {"type": "number", "minimum": 0},
                       "camera": {"type": "string", "enum": ["perspective", "orthographic"], "description": "front camera model (default perspective, focal length solved)"},
                       "focal_px": {"type": "number", "exclusiveMinimum": 0, "description": "fix the front camera's focal length in pixels instead of solving it"},
                       "profile_upright": {"type": "boolean", "description": "align the profile photo by scale and translation only (an upright photo), not rotation"},
                       "profile_align": {"type": "string", "enum": ["every_round", "first_round"], "description": "re-align the profile photo to the model each round, or once (to the base) and keep it"}}),
            ),
        ),
        &["run", "store", "pack", "catalog"],
    )
}

pub fn targets(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = run_dir(input)?;
    let started = Instant::now();
    let base = load_base(input)?;
    let p = &base.dense.positions;
    let tip = base.tip;
    let (catalog, ids) = head_ids(&base, text(input, "catalog"))?;
    let lm = landmark_vertices(&run)?;
    let (px, marks) = photo_points(&run)?;
    let line: Vec<[f64; 2]> = marks["profile"]["line"].as_array().map(|a| a.iter().map(|u| [u[0].as_f64().unwrap_or(0.0), u[1].as_f64().unwrap_or(0.0)]).collect()).unwrap_or_default();
    let normals = gates::vertex_normals(p, &base.quads);
    let eyes = eye_centres(p, &lm);
    let shell_pts = points(&run.join("shell_registered.npy"))?;
    let shell_n = points(&run.join("shell_registered_normals.npy"))?;
    let mut grid = PointGrid::new(shell_pts, 0.002);
    let mut shell_n = shell_n;
    let n = p.len();
    let mut shell_rows = Vec::new();
    let mut prior_rows = Vec::new();
    let mut mid = Vec::new();
    for (v, q) in p.iter().enumerate() {
        let r = mm(fit::sub(*q, tip));
        let front = -normals[v][1];
        let face = r[0].abs() < 65.0 && r[2] > -95.0 && r[2] < 45.0 && r[1] < 70.0 && front > 0.2;
        let neck = r[0].abs() < 70.0 && r[2] <= -95.0 && r[2] > -150.0 && r[1] < 90.0 && front > 0.2;
        if (face || neck) && eye_gap_mm(*q, &eyes) > 18.0 {
            shell_rows.push(v);
        } else if !face && !neck && eye_gap_mm(*q, &eyes) > 30.0 {
            prior_rows.push(v);
        }
        if r[0].abs() < 8.0 && r[2] > -110.0 && r[2] < 60.0 && r[1] < 40.0 && front > 0.0 {
            mid.push(v);
        }
    }
    let stride = (shell_rows.len() / 25000).max(1);
    let shell_rows: Vec<usize> = shell_rows.into_iter().step_by(stride).collect();
    let prior_stride = (prior_rows.len() / 8000).max(1);
    let prior_rows: Vec<usize> = prior_rows.into_iter().step_by(prior_stride).collect();
    let mid_stride = (mid.len() / 15000).max(1);
    let mid: Vec<usize> = mid.into_iter().step_by(mid_stride).collect();
    let lm_used: Vec<(usize, usize)> = STABLE_LANDMARKS.iter().filter_map(|i| lm.get(i).map(|v| (*i, *v))).filter(|(i, _)| px.get(*i).is_some_and(|u| u[0].is_finite())).collect();
    let mut slots = vec![NONE; n];
    let mut sample = Vec::new();
    for &v in shell_rows.iter().chain(mid.iter()).chain(prior_rows.iter()).chain(lm_used.iter().map(|(_, v)| v)) {
        if slots[v] == NONE {
            slots[v] = sample.len() as u32;
            sample.push(v);
        }
    }
    let t = ids.len();
    let mut prob = Problem { ids: ids.clone(), slots, sample, deltas: Vec::new() };
    prob.deltas = vec![0.0f32; prob.sample.len() * 3 * t];
    for (j, id) in ids.iter().enumerate() {
        let target = base.store.load_target(&catalog, id).map_err(ApiError::internal)?;
        for (&i, d) in target.indices.iter().zip(&target.deltas) {
            let s = prob.slots[i as usize];
            if s != NONE {
                for (k, dk) in d.iter().enumerate() {
                    prob.deltas[(s as usize * 3 + k) * t + j] = *dk;
                }
            }
        }
    }
    let load_seconds = started.elapsed().as_secs_f64();
    let slot = |v: usize| prob.slots[v] as usize;
    let mid_slots: Vec<usize> = mid.iter().map(|v| slot(*v)).collect();
    let (ws, wl, wp) = (num(input, "shell_weight", 1.0), num(input, "landmark_weight", 0.3), num(input, "profile_weight", 0.0));
    let ridge = num(input, "ridge", 1e-3);
    let (wprior, wsym) = (num(input, "prior_weight", 1.0), num(input, "symmetry_weight", 0.05));
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
    let sym_pairs: Vec<(usize, usize)> = ids.iter().enumerate().filter_map(|(i, k)| index.get(k.replacen("/l-", "/r-", 1).as_str()).filter(|&&j| j != i).map(|&j| (i, j))).collect();
    let mut refinements = Vec::new();
    let rounds = input.get("rounds").and_then(Value::as_u64).unwrap_or(5) as usize;
    let mut w = vec![0.0; t];
    let mut log = Vec::new();
    let mut camera: Option<Value> = None;
    let camera_model = opt(input, "camera").unwrap_or("perspective").to_string();
    let focal_fixed = input.get("focal_px").and_then(Value::as_f64);
    let profile_upright = input.get("profile_upright").and_then(Value::as_bool).unwrap_or(false);
    let profile_once = opt(input, "profile_align") == Some("first_round");
    let photo_size = [marks["front"]["size"][0].as_f64().unwrap_or(0.0), marks["front"]["size"][1].as_f64().unwrap_or(0.0)];
    let mut profile: Option<Profile> = None;
    let keys = marks["profile"]["keys"].clone();
    for round in 0..rounds {
        if round > 0 {
            let mut a = Vec::new();
            let mut b = Vec::new();
            let mut ds = Vec::new();
            for &v in &shell_rows {
                let x = prob.position(p, slot(v), &w);
                if let Some((qi, d)) = grid.nearest(x, 0.006) {
                    a.push(grid.points()[qi]);
                    b.push(x);
                    ds.push(d);
                }
            }
            if a.len() > 1000 {
                let mut sorted = ds.clone();
                sorted.sort_by(f64::total_cmp);
                let cut = sorted[(sorted.len() as f64 * 0.8) as usize];
                let keep: Vec<usize> = (0..a.len()).filter(|i| ds[*i] <= cut).collect();
                let (ka, kb): (Vec<Point>, Vec<Point>) = keep.iter().map(|i| (a[*i], b[*i])).unzip();
                let rigid = fit::similarity(&ka, &kb, &vec![1.0; ka.len()], false).map_err(ApiError::invalid)?;
                let trace = rigid.rotation[0][0] + rigid.rotation[1][1] + rigid.rotation[2][2];
                let moved: Vec<Point> = grid.points().iter().map(|q| rigid.apply(*q)).collect();
                refinements.push(json!({"round": round, "rotation_deg": ((trace - 1.0) / 2.0).clamp(-1.0, 1.0).acos().to_degrees(),
                                        "tip_shift_mm": fit::norm(fit::sub(rigid.apply(tip), tip)) * 1000.0, "pairs": ka.len()}));
                shell_n = shell_n.iter().map(|q| rigid.rotate(*q)).collect();
                grid = PointGrid::new(moved, 0.002);
            }
        }
        let mut acc = Accum::new(t);
        let prior_weight = wprior / (3 * prior_rows.len()).max(1) as f64;
        for &v in &prior_rows {
            let s = slot(v);
            for k in 0..3 {
                let a: Vec<f64> = prob.row(s, k).iter().map(|d| *d as f64).collect();
                acc.add(&a, 0.0, prior_weight);
            }
        }
        let sym_weight = wsym / sym_pairs.len().max(1) as f64;
        for (i, j) in &sym_pairs {
            let mut a = vec![0.0; t];
            a[*i] = 1.0;
            a[*j] = -1.0;
            acc.add(&a, 0.0, sym_weight);
        }
        let mut shell_res = Vec::new();
        let mut shell_items: Vec<(usize, Point, Point)> = Vec::new();
        for &v in &shell_rows {
            let x = prob.position(p, slot(v), &w);
            if let Some((qi, _)) = grid.nearest(x, 0.010) {
                let (q, nq) = (grid.points()[qi], shell_n[qi]);
                let r = fit::dot(nq, fit::sub(x, q));
                if r.abs() <= 0.006 {
                    shell_items.push((v, q, nq));
                    shell_res.push(r.abs() * 1000.0);
                }
            }
        }
        let shell_weight = ws / shell_items.len().max(1) as f64;
        for (v, q, nq) in &shell_items {
            let s = slot(*v);
            let a: Vec<f64> = (0..t).map(|j| (0..3).map(|k| nq[k] * prob.row(s, k)[j] as f64).sum()).collect();
            acc.add(&a, fit::dot(*nq, fit::sub(*q, p[*v])), shell_weight);
        }
        let xl: Vec<Point> = lm_used.iter().map(|(_, v)| prob.position(p, slot(*v), &w)).collect();
        let ul: Vec<[f64; 2]> = lm_used.iter().map(|(i, _)| px[*i]).collect();
        let cam = FrontCam::solve(&camera_model, focal_fixed, photo_size, &xl, &ul)?;
        let lm_px: Vec<f64> = xl.iter().zip(&ul).map(|(x, u)| {
            let q = cam.project(*x);
            ((q[0] - u[0]).powi(2) + (q[1] - u[1]).powi(2)).sqrt()
        }).collect();
        let lm_weight = wl / (2 * lm_used.len()).max(1) as f64;
        match &cam {
            FrontCam::Ortho(cam) => {
                for ((_, v), u) in lm_used.iter().zip(&ul) {
                    let s = slot(*v);
                    for (k, uk) in u.iter().enumerate() {
                        let a: Vec<f64> = (0..t).map(|j| (0..3).map(|c| cam.rows[k][c] * prob.row(s, c)[j] as f64).sum()).collect();
                        acc.add(&a, (uk - cam.translation[k]) / cam.scale - fit::dot(cam.rows[k], p[*v]), lm_weight);
                    }
                }
            }
            FrontCam::Persp(cam) => {
                for (((_, v), u), x) in lm_used.iter().zip(&ul).zip(&xl) {
                    let s = slot(*v);
                    let g = cam.gradient(*x);
                    let ppm = cam.pixels_per_metre(*x);
                    let q = cam.project(*x);
                    for k in 0..2 {
                        let gk = fit::scale(g[k], 1.0 / ppm);
                        let a: Vec<f64> = (0..t).map(|j| (0..3).map(|c| gk[c] * prob.row(s, c)[j] as f64).sum()).collect();
                        acc.add(&a, (u[k] - q[k]) / ppm + fit::dot(gk, fit::sub(*x, p[*v])), lm_weight);
                    }
                }
            }
        }
        let curve = silhouette(&prob, p, &w, &mid_slots, tip);
        let prof = if line.is_empty() { None } else { Some(fit_profile(&curve, &line, &keys, tip, profile.as_ref().map(|x| &x.to_model), profile_upright, !profile_once)?) };
        if let Some(pr) = &prof {
            let pw = wp / pr.rows.len().max(1) as f64;
            for (s, f) in &pr.rows {
                let a: Vec<f64> = (0..t).map(|j| -(prob.row(*s, 1)[j] as f64)).collect();
                acc.add(&a, f + p[prob.sample[*s]][1], pw);
            }
        }
        let trace: f64 = (0..t).map(|i| acc.gram[i * t + i]).sum::<f64>() / t as f64;
        for i in 0..t {
            acc.gram[i * t + i] += ridge * trace;
        }
        let shell_rms = (shell_res.iter().map(|r| r * r).sum::<f64>() / shell_res.len().max(1) as f64).sqrt();
        let lm_rms = (lm_px.iter().map(|r| r * r).sum::<f64>() / lm_px.len().max(1) as f64).sqrt();
        let centroid = fit::scale(xl.iter().fold([0.0; 3], |a, x| fit::add(a, *x)), 1.0 / xl.len().max(1) as f64);
        log.push(json!({"round": round, "before_solve": {"shell_rows": shell_items.len(), "shell_rms_mm": shell_rms, "landmark_rms_px": lm_rms,
                         "landmark_rms_mm": lm_rms / cam.scale_at(centroid) * 1000.0, "profile_rows": prof.as_ref().map(|x| x.rows.len()),
                         "profile_rms_mm": prof.as_ref().map(|x| x.rms_mm), "focal_px": cam.focal()}}));
        w = fit::bounded_least_squares(&acc.gram, &acc.rhs, 0.0, 1.0, &w, 3000);
        camera = Some(cam.to_json());
        profile = prof;
    }
    let mut regions: Vec<Vec<f64>> = vec![Vec::new(); fit::REGIONS.len()];
    let mut all = Vec::new();
    for &v in &shell_rows {
        let x = prob.position(p, slot(v), &w);
        if let Some((qi, _)) = grid.nearest(x, 0.010) {
            let r = fit::dot(shell_n[qi], fit::sub(x, grid.points()[qi])).abs() * 1000.0;
            let reg = fit::region(mm(fit::sub(p[v], tip)), eye_gap_mm(p[v], &eyes));
            regions[reg].push(r);
            all.push(r);
        }
    }
    let mut per_region: BTreeMap<String, fit::Spread> = fit::REGIONS.iter().zip(&regions).map(|(n, v)| (n.to_string(), fit::spread_mm(v))).collect();
    per_region.insert("all".into(), fit::spread_mm(&all));
    let weights: BTreeMap<String, f64> = ids.iter().zip(&w).filter(|(_, x)| **x > 1e-6).map(|(k, x)| (k.clone(), (x * 1e6).round() / 1e6)).collect();
    let report = json!({
        "targets": t, "nonzero": weights.len(), "at_1": w.iter().filter(|x| **x >= 1.0 - 1e-9).count(),
        "weights": weights, "rounds": log, "front_camera": camera, "camera_model": camera_model, "focal_px_fixed": focal_fixed, "profile_upright": profile_upright, "profile_align": if profile_once { "first_round" } else { "every_round" },
        "profile_photo_to_model": profile.as_ref().map(|x| &x.to_model),
        "profile_rms_mm": profile.as_ref().map(|x| x.rms_mm), "shell_residual_mm_by_region": per_region,
        "samples": {"shell": shell_rows.len(), "midline": mid.len(), "prior": prior_rows.len(), "landmarks": lm_used.len(), "symmetry_pairs": sym_pairs.len()},
        "shell_refinements": refinements, "term_weights": {"shell": ws, "landmarks": wl, "profile": wp, "prior": wprior, "symmetry": wsym, "ridge": ridge},
        "load_seconds": load_seconds, "seconds": started.elapsed().as_secs_f64()
    });
    dio::write_points(&run.join("shell_fitted.npy"), grid.points()).map_err(ApiError::internal)?;
    dio::write_points(&run.join("shell_fitted_normals.npy"), &shell_n).map_err(ApiError::internal)?;
    write_json(&run.join("fit_targets.json"), &report)?;
    Ok(report)
}

fn evaluate_weights(base: &Base, catalog_ref: &str, weights: &BTreeMap<String, f64>) -> Result<Vec<Point>, ApiError> {
    let catalog = base.store.load_catalog(catalog_ref).map_err(ApiError::not_found)?;
    let mut x = base.dense.positions.clone();
    for (id, w) in weights {
        base.store.load_target(&catalog, id).map_err(ApiError::internal)?.add_scaled(*w, &mut x);
    }
    Ok(x)
}

fn weights_of(report: &Value) -> BTreeMap<String, f64> {
    report["weights"].as_object().map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect()).unwrap_or_default()
}

pub fn residual_input() -> Value {
    obj(
        merged(
            tool_props(),
            merged(
                store_props(),
                json!({"trust_mm": {"type": "number", "minimum": 0.5, "maximum": 20}, "smooth_mm": {"type": "number", "minimum": 0, "maximum": 10},
                       "eye_hold_mm": {"type": "number", "minimum": 0, "maximum": 40}, "mouth_hold_mm": {"type": "number", "minimum": 0, "maximum": 20},
                       "plant": {"type": "object", "description": "a deliberate defect to prove the refusal: {kind: fold, radius_mm, depth_mm}"}}),
            ),
        ),
        &["run", "store", "pack", "catalog"],
    )
}

pub fn residual(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = run_dir(input)?;
    let started = Instant::now();
    let base = load_base(input)?;
    let p = &base.dense.positions;
    let tip = base.tip;
    let fitted = read_json(&run.join("fit_targets.json"))?;
    let x0 = evaluate_weights(&base, text(input, "catalog"), &weights_of(&fitted))?;
    let lm = landmark_vertices(&run)?;
    let eyes = eye_centres(&x0, &lm);
    let normals = gates::vertex_normals(&x0, &base.quads);
    let grid = PointGrid::new(points(&run.join("shell_fitted.npy"))?, 0.002);
    let shell_n = points(&run.join("shell_fitted_normals.npy"))?;
    let (trust, smooth_mm, hold) = (num(input, "trust_mm", 7.0), num(input, "smooth_mm", 3.0), num(input, "eye_hold_mm", 26.0));
    let n = x0.len();
    let mut active = vec![false; n];
    let mut values = vec![[0.0; 3]; n];
    let mut weights = vec![0.0; n];
    let mut hits = 0usize;
    for v in 0..n {
        let r = mm(fit::sub(p[v], tip));
        if r[0].abs() > 80.0 || r[2] < -110.0 || r[2] > 60.0 || r[1] > 90.0 {
            continue;
        }
        active[v] = true;
        if r[0].abs() > 65.0 || r[2] < -95.0 || r[2] > 45.0 || r[1] > 70.0 || -normals[v][1] < 0.2 {
            continue;
        }
        if let Some((qi, _)) = grid.nearest(x0[v], trust / 1000.0) {
            let (q, nq) = (grid.points()[qi], shell_n[qi]);
            let gap = fit::sub(q, x0[v]);
            let d = fit::dot(nq, gap);
            let tangential = fit::norm(fit::sub(gap, fit::scale(nq, d)));
            if tangential < 0.0025 && fit::dot(nq, normals[v]).abs() > 0.5 {
                let c = 1.0 - fit::smoothstep(0.6 * trust, trust, d.abs() * 1000.0);
                values[v] = fit::scale(nq, d * c);
                weights[v] = c;
                hits += 1;
            }
        }
    }
    let edges = gates::unique_edges(&base.quads);
    let mut edge_sum = 0.0;
    let mut edge_count = 0usize;
    for e in &edges {
        if active[e[0] as usize] {
            edge_sum += fit::norm(fit::sub(p[e[0] as usize], p[e[1] as usize]));
            edge_count += 1;
        }
    }
    let spacing_mm = edge_sum / edge_count.max(1) as f64 * 1000.0;
    let iterations = (((smooth_mm / spacing_mm.max(1e-6)).powi(2) / 0.4).ceil() as usize).min(400);
    let adjacency = Adjacency::from_edges(n, &edges);
    adjacency.smooth(&mut values, &mut weights, iterations, &active);
    let seam_mm = num(input, "mouth_hold_mm", 6.0);
    let seam: Option<Vec<Point>> = match (lm.get(&61), lm.get(&13), lm.get(&14), lm.get(&291)) {
        (Some(&a), Some(&b), Some(&c), Some(&d)) => Some(vec![x0[a], fit::scale(fit::add(x0[b], x0[c]), 0.5), x0[d]]),
        _ => None,
    };
    let pairs = gates::face_pairs(&base.quads);
    let folded = gates::folds(&x0, &base.quads, &pairs);
    let mut keep = vec![[1.0, 0.0, 0.0]; n];
    let mut unused = vec![1.0; n];
    for (pair, f) in pairs.iter().zip(&folded) {
        if *f {
            for face in pair {
                for &v in &base.quads[*face as usize] {
                    keep[v as usize] = [0.0; 3];
                }
            }
        }
    }
    adjacency.smooth(&mut keep, &mut unused, iterations, &active);
    let mut field = vec![[0.0; 3]; n];
    for v in 0..n {
        if active[v] && weights[v] > 1e-6 {
            let coverage = fit::smoothstep(0.05, 0.4, weights[v]);
            let facing = fit::smoothstep(0.05, 0.35, -normals[v][1]);
            let away_from_folds = fit::smoothstep(0.5, 0.95, keep[v][0]);
            let lips = seam.as_ref().map_or(1.0, |s| fit::smoothstep(seam_mm, seam_mm + 5.0, polyline_mm(x0[v], s)));
            let held = facing * away_from_folds * lips * fit::smoothstep(hold - 6.0, hold, eye_gap_mm(x0[v], &eyes));
            let mut d = fit::scale(values[v], coverage * held / weights[v]);
            let len = fit::norm(d) * 1000.0;
            if len > trust {
                d = fit::scale(d, trust / len);
            }
            field[v] = d;
        }
    }
    let mut ones = vec![1.0; n];
    adjacency.smooth(&mut field, &mut ones, iterations / 2, &active);
    let mut planted = Value::Null;
    let mut plant_field = vec![[0.0; 3]; n];
    if let Some(plant) = input.get("plant") {
        let radius = plant.get("radius_mm").and_then(Value::as_f64).unwrap_or(5.0);
        let depth = plant.get("depth_mm").and_then(Value::as_f64).unwrap_or(3.0) / 1000.0;
        let centre = fit::add(tip, [0.0, 0.012, -0.045]);
        let mut count = 0;
        for v in 0..n {
            if fit::norm(fit::sub(x0[v], centre)) * 1000.0 < radius {
                let sign = if v % 2 == 0 { 1.0 } else { -1.0 };
                plant_field[v] = fit::add(plant_field[v], fit::scale(normals[v], sign * depth));
                count += 1;
            }
        }
        planted = json!({"kind": plant.get("kind").cloned().unwrap_or(json!("fold")), "vertices": count, "radius_mm": radius, "depth_mm": depth * 1000.0});
    }
    let mut attempts = Vec::new();
    let mut chosen = (0.0, Vec::new(), None);
    for strength in [1.0, 0.7, 0.5, 0.35, 0.25] {
        let after: Vec<Point> = (0..n).map(|v| fit::add(fit::add(x0[v], fit::scale(field[v], strength)), plant_field[v])).collect();
        let report = gates::shell(&x0, &after, &base.quads, &ShellLimits::default()).map_err(ApiError::internal)?;
        attempts.push(json!({"strength": strength, "folds_rise": report.folds_after as i64 - report.folds_before as i64, "pass": report.pass}));
        let pass = report.pass;
        chosen = (strength, after, Some(report));
        if pass {
            break;
        }
    }
    let (strength, after, shell) = chosen;
    let shell = shell.ok_or_else(|| ApiError::internal("no residual strength was tried"))?;
    for (d, p) in field.iter_mut().zip(&plant_field) {
        *d = fit::add(fit::scale(*d, strength), *p);
    }
    let moves: Vec<f64> = field.iter().map(|d| fit::norm(*d) * 1000.0).filter(|m| *m > 1e-4).collect();
    let folded_after = gates::folds(&after, &base.quads, &pairs);
    let mut new_folds: BTreeMap<String, usize> = BTreeMap::new();
    let mut fold_sites = Vec::new();
    for ((pair, b), a) in pairs.iter().zip(&folded).zip(&folded_after) {
        if *a && !*b {
            let q = &base.quads[pair[0] as usize];
            let c = fit::scale(q.iter().fold([0.0; 3], |s, v| fit::add(s, x0[*v as usize])), 0.25);
            let rel = mm(fit::sub(c, tip));
            *new_folds.entry(fit::REGIONS[fit::region(rel, eye_gap_mm(c, &eyes))].to_string()).or_insert(0) += 1;
            if fold_sites.len() < 12 {
                fold_sites.push([(rel[0] * 10.0).round() / 10.0, (rel[1] * 10.0).round() / 10.0, (rel[2] * 10.0).round() / 10.0]);
            }
        }
    }
    let summary = json!({"strength": strength, "attempts": attempts, "mouth_hold_mm": seam_mm, "new_folds_by_region": new_folds, "new_fold_sites_mm_from_tip": fold_sites,"trust_mm": trust, "smooth_mm": smooth_mm, "eye_hold_mm": hold, "spacing_mm": spacing_mm, "iterations": iterations,
                         "shell_hits": hits, "moved": fit::spread_mm(&moves), "plant": planted, "shell_gate": shell});
    if !shell.pass {
        return Err(ApiError::new(Code::GateFailed, format!("the residual was refused by the shell gates: {}", shell.failures.join("; ")))
            .details(summary)
            .hint("nothing was written; lower trust_mm or raise smooth_mm, or inspect the shell gate report"));
    }
    dio::write_points(&run.join("residual.npy"), &field).map_err(ApiError::internal)?;
    dio::write_points_f64(&run.join("fitted_dense.npy"), &after).map_err(ApiError::internal)?;
    dio::write_points_f64(&run.join("targets_dense.npy"), &x0).map_err(ApiError::internal)?;
    let cameras = cameras(&run, &after, &lm, tip, &fitted)?;
    let report = merged(summary, json!({"cameras": cameras, "seconds": started.elapsed().as_secs_f64()}));
    write_json(&run.join("residual.json"), &report)?;
    Ok(report)
}

fn cameras(run: &Path, x: &[Point], lm: &BTreeMap<usize, usize>, tip: Point, fitted: &Value) -> Result<Value, ApiError> {
    let (px, marks) = photo_points(run)?;
    let used: Vec<(usize, usize)> = STABLE_LANDMARKS.iter().filter_map(|i| lm.get(i).map(|v| (*i, *v))).filter(|(i, _)| px.get(*i).is_some_and(|u| u[0].is_finite())).collect();
    let xl: Vec<Point> = used.iter().map(|(_, v)| x[*v]).collect();
    let ul: Vec<[f64; 2]> = used.iter().map(|(i, _)| px[*i]).collect();
    let size = [marks["front"]["size"][0].as_f64().unwrap_or(0.0), marks["front"]["size"][1].as_f64().unwrap_or(0.0)];
    let cam = FrontCam::solve(fitted["camera_model"].as_str().unwrap_or("orthographic"), fitted["focal_px_fixed"].as_f64(), size, &xl, &ul)?;
    let errs: Vec<f64> = xl.iter().zip(&ul).map(|(a, u)| {
        let q = cam.project(*a);
        ((q[0] - u[0]).powi(2) + (q[1] - u[1]).powi(2)).sqrt()
    }).collect();
    let rms = (errs.iter().map(|e| e * e).sum::<f64>() / errs.len().max(1) as f64).sqrt();
    let mut out = json!({"front": cam.to_json(), "front_rms_px": rms, "front_max_px": errs.iter().copied().fold(0.0, f64::max), "front_points": used.len(),
                         "photo_size": marks["front"]["size"], "profile_size": marks["profile"]["size"], "tip": tip});
    if let Ok(prof) = serde_json::from_value::<Similarity2Json>(fitted["profile_photo_to_model"].clone()) {
        let sim = Similarity2 { scale: prof.scale, angle: prof.angle, translation: prof.translation };
        let inv = sim.inverse();
        let mut curve: HashMap<i64, f64> = HashMap::new();
        for q in x {
            let r = mm(fit::sub(*q, tip));
            if r[0].abs() < 8.0 && r[1] < 40.0 {
                let bin = r[2].round() as i64;
                let f = -q[1];
                if curve.get(&bin).is_none_or(|b| f > *b) {
                    curve.insert(bin, f);
                }
            }
        }
        let line: Vec<[f64; 2]> = marks["profile"]["line"].as_array().map(|a| a.iter().map(|u| [u[0].as_f64().unwrap_or(0.0), u[1].as_f64().unwrap_or(0.0)]).collect()).unwrap_or_default();
        let mut errs = Vec::new();
        for u in &line {
            let m = sim.apply(*u);
            let z = (-m[1] - tip[2]) * 1000.0;
            if (-95.0..=40.0).contains(&z)
                && let Some(f) = curve.get(&(z.round() as i64))
            {
                let back = inv.apply([*f, m[1]]);
                errs.push((back[0] - u[0]).abs());
            }
        }
        let prms = (errs.iter().map(|e| e * e).sum::<f64>() / errs.len().max(1) as f64).sqrt();
        out["profile_model_to_photo"] = json!(inv);
        out["profile_photo_to_model"] = json!(sim);
        out["profile_rms_px"] = json!(prms);
        out["profile_rows"] = json!(errs.len());
    }
    write_json(&run.join("cameras.json"), &out)?;
    Ok(out)
}

#[derive(serde::Deserialize)]
struct Similarity2Json {
    scale: f64,
    angle: f64,
    translation: [f64; 2],
}

pub fn texture_input() -> Value {
    obj(with_tools(json!({"glb": {"type": "string", "minLength": 1}, "size": {"type": "integer", "minimum": 256, "maximum": 8192},
                          "front_albedo": {"type": "string"}, "profile_albedo": {"type": "string"}})), &["run", "glb"])
}

pub fn texture(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = run_dir(input)?;
    let args = json!({"glb": text(input, "glb"), "size": input.get("size").and_then(Value::as_u64).unwrap_or(2048),
                      "front_albedo": opt(input, "front_albedo"), "profile_albedo": opt(input, "profile_albedo")});
    worker(input, "photo_worker.py", "texture", args, &run)
}

pub fn sheet_input() -> Value {
    obj(with_tools(json!({"glb": {"type": "string", "minLength": 1}, "texture": {"type": "string", "minLength": 1}, "out": {"type": "string", "minLength": 1}})),
        &["run", "glb", "texture", "out"])
}

pub fn sheet(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = run_dir(input)?;
    worker(input, "photo_worker.py", "review", json!({"glb": text(input, "glb"), "texture": text(input, "texture"), "out": text(input, "out")}), &run)
}

pub fn from_photos_input() -> Value {
    obj(
        merged(
            with_tools(json!({
                "front": {"type": "string", "minLength": 1}, "profile": {"type": "string", "minLength": 1},
                "profile_side": {"type": "string", "enum": ["right", "left"]}, "game": {"type": "string", "minLength": 1},
                "name": {"type": "string", "minLength": 1}, "glb": {"type": "string", "description": "reuse a mesh (skips the server)"},
                "eyes": {"type": "string", "description": "eye description for the placement (export-glb --eyes)"},
                "sheet": {"type": "string", "description": "where to write the review sheet (default <run>/review_sheet.png)"},
                "texture_size": {"type": "integer", "minimum": 256, "maximum": 8192},
                "camera": {"type": "string", "enum": ["perspective", "orthographic"]},
                "focal_px": {"type": "number", "exclusiveMinimum": 0},
                "profile_upright": {"type": "boolean"},
                "profile_align": {"type": "string", "enum": ["every_round", "first_round"]},
                "shell_mode": shell_mode_prop(),
                "max_ratio_gap": {"type": "number", "minimum": 0, "maximum": 1, "description": "photo.ingest's front vs profile proportion tolerance (default 0.12)"},
                "fit": {"type": "string", "enum": ["photos", "mesh"], "description": "fit the targets directly to the photos (default: landmarks and outlines of every photo through solved cameras) or to a generated mesh (stages 2-5)"},
                "extras": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string", "minLength": 1}, "path": {"type": "string", "minLength": 1}}, "required": ["path"], "additionalProperties": false}, "description": "any number of extra views (three-quarters, the other side, slightly up or down): each is checked for quality and its pose is solved"},
                "fit_options": {"type": "object", "description": "fit.photos options (weights, rounds, focal_px, outline_regions, landmarks, prior_scope)"}
            })),
            merged(store_props(), base_table_prop()),
        ),
        &["front", "profile", "run", "store", "pack", "catalog", "game"],
    )
}

pub fn from_photos(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    if opt(input, "fit").unwrap_or("photos") == "photos" {
        return crate::photo_fit::from_photos(ctx, input);
    }
    let run = run_dir(input)?;
    let started = Instant::now();
    let mut stages: Vec<Value> = Vec::new();
    let log = |stages: &Vec<Value>| write_json(&run.join("stages.json"), &json!({"stages": stages}));
    let step = |name: &str, f: &mut dyn FnMut() -> Result<Value, ApiError>, stages: &mut Vec<Value>| -> Result<Value, ApiError> {
        let t0 = Instant::now();
        let out = f();
        let seconds = t0.elapsed().as_secs_f64();
        stages.push(json!({"stage": name, "seconds": seconds, "ok": out.is_ok()}));
        let _ = log(stages);
        out.map_err(|e| {
            let mut details = if e.details.is_object() { e.details.clone() } else { json!({"detail": e.details.clone()}) };
            details["stage"] = json!(name);
            details["stages"] = json!(stages.clone());
            ApiError { details, ..e }
        })
    };
    let mut ingest_in = json!({"front": text(input, "front"), "profile": text(input, "profile"), "profile_side": opt(input, "profile_side").unwrap_or("right")});
    for k in ["run", "python", "workers", "landmarker", "max_ratio_gap"] {
        if let Some(v) = input.get(k) {
            ingest_in[k] = v.clone();
        }
    }
    let ingested = step("0 photo.ingest", &mut || ingest(ctx, &ingest_in), &mut stages)?;
    let tools_only = {
        let mut v = json!({});
        for k in ["run", "python", "workers", "landmarker", "server", "graph"] {
            if let Some(x) = input.get(k) {
                v[k] = x.clone();
            }
        }
        v
    };
    let marks = step("1 photo.landmarks", &mut || landmarks(ctx, &tools_only), &mut stages)?;
    let mut mesh_in = tools_only.clone();
    for k in ["glb", "shell_mode"] {
        if let Some(g) = input.get(k) {
            mesh_in[k] = g.clone();
        }
    }
    let meshed = step("2 mesh.multiview", &mut || mesh(ctx, &mesh_in), &mut stages)?;
    let reg_in = {
        let mut v = tools_only.clone();
        v["store"] = input["store"].clone();
        v["pack"] = input["pack"].clone();
        if let Some(t) = input.get("base_landmarks") {
            v["base_landmarks"] = t.clone();
        }
        v
    };
    let registered = step("3 fit.register", &mut || register(ctx, &reg_in), &mut stages)?;
    let mut fit_in = reg_in.clone();
    fit_in["catalog"] = input["catalog"].clone();
    for k in ["camera", "focal_px", "profile_upright", "profile_align"] {
        if let Some(x) = input.get(k) {
            fit_in[k] = x.clone();
        }
    }
    let fitted = step("4 fit.targets", &mut || targets(ctx, &fit_in), &mut stages)?;
    let resid = step("5 fit.residual", &mut || residual(ctx, &fit_in), &mut stages)?;
    let name = opt(input, "name").unwrap_or("photo_character").to_string();
    let spec_path = run.join(format!("{name}.gfchar.json"));
    let glb = run.join(format!("{name}.glb"));
    let exported = step(
        "8a character.settle + export-glb",
        &mut || settle_export(ctx, input, &run, &spec_path, &glb, &name, &fitted),
        &mut stages,
    )?;
    let mut tex_in = tools_only.clone();
    tex_in["glb"] = json!(glb.display().to_string());
    if let Some(s) = input.get("texture_size") {
        tex_in["size"] = s.clone();
    }
    let textured = step("7 texture.project", &mut || texture(ctx, &tex_in), &mut stages)?;
    let sheet_out = opt(input, "sheet").map(str::to_string).unwrap_or_else(|| run.join("review_sheet.png").display().to_string());
    let mut sheet_in = tools_only.clone();
    sheet_in["glb"] = json!(glb.display().to_string());
    sheet_in["texture"] = textured.get("texture").cloned().unwrap_or(json!(""));
    sheet_in["out"] = json!(sheet_out);
    let reviewed = step("8b review.sheet", &mut || sheet(ctx, &sheet_in), &mut stages)?;
    let total = started.elapsed().as_secs_f64();
    let report = json!({"run": run.display().to_string(), "spec": spec_path.display().to_string(), "glb": glb.display().to_string(),
                        "sheet": sheet_out, "total_seconds": total, "stages": stages,
                        "facing": ingested.get("facing"), "landmarks": marks.get("summary"), "mesh": meshed, "register": registered,
                        "targets": {"nonzero": fitted["nonzero"], "regions": fitted["shell_residual_mm_by_region"], "rounds": fitted["rounds"]},
                        "residual": {"shell_gate": resid["shell_gate"], "moved": resid["moved"], "cameras": resid["cameras"]},
                        "export": exported, "texture": textured, "review": reviewed});
    write_json(&run.join("from_photos.json"), &report)?;
    Ok(report)
}

pub(crate) fn settle_export(ctx: &Ctx, input: &Value, run: &Path, spec_path: &Path, glb: &Path, name: &str, fitted: &Value) -> Result<Value, ApiError> {
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::not_found)?;
    let mut spec = Spec {
        format: FORMAT.into(),
        name: name.to_string(),
        base: SpecBase {
            pack: text(input, "pack").into(),
            catalog: text(input, "catalog").into(),
            game_base: Some(text(input, "game").into()),
            placement: gf_core::placement::DEFAULT.to_string(),
        },
        weights: weights_of(fitted),
        macros: None,
        residual: None,
    };
    let (parametric, _) = ch::evaluate(&store, &spec, None).map_err(ApiError::invalid)?;
    let target = read_f64_points(&run.join("fitted_dense.npy"))?;
    let (reference, codec) = ch::capture_residual(&store, &parametric, &target, 5e-6).map_err(ApiError::internal)?;
    spec.residual = Some(reference);
    let revision = ch::write_spec(spec_path, &spec).map_err(ApiError::internal)?;
    let mut cmd = Command::new(&ctx.exe);
    cmd.arg("export-glb").arg("--store").arg(text(input, "store")).arg("--spec").arg(spec_path).arg("--out").arg(glb);
    if let Some(eyes) = opt(input, "eyes") {
        cmd.arg("--eyes").arg(eyes);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::process::hide_window(&mut cmd);
    let out = cmd.output().map_err(|e| ApiError::new(Code::WorkerFailed, format!("export-glb: {e}")))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    if !out.status.success() {
        return Err(ApiError::new(Code::WorkerFailed, "export-glb failed").details(json!({"stdout": stdout, "stderr": String::from_utf8_lossy(&out.stderr)})));
    }
    let export: Value = serde_json::from_str(stdout.trim()).unwrap_or(json!({"stdout": stdout}));
    Ok(json!({"spec": spec_path.display().to_string(), "revision": revision, "residual_codec": codec, "export": export}))
}

fn read_f64_points(path: &Path) -> Result<Vec<Point>, ApiError> {
    let bytes = std::fs::read(path).map_err(|e| ApiError::not_found(format!("{}: {e}", path.display())))?;
    if bytes.len() < 12 || &bytes[1..6] != b"NUMPY" {
        return Err(ApiError::internal(format!("{} is not an .npy file", path.display())));
    }
    let start = if bytes[6] == 1 { 10 + u16::from_le_bytes([bytes[8], bytes[9]]) as usize } else { 12 + u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize };
    let header = String::from_utf8_lossy(&bytes[..start.min(bytes.len())]);
    if !header.contains("<f8") {
        return Err(ApiError::internal(format!("{} is not float64", path.display())));
    }
    let data = &bytes[start.min(bytes.len())..];
    Ok(data.chunks_exact(24).map(|c| {
        let f = |i: usize| f64::from_le_bytes(c[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8]));
        [f(0), f(1), f(2)]
    }).collect())
}
