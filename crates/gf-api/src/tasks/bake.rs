use std::collections::HashMap;
use std::path::Path;

use gf_io::gltf;
use serde_json::{Value, json};

use super::Options;

type V3 = [f64; 3];
type Q = [f64; 4];
type M4 = [[f64; 4]; 4];

pub const KEYS: &[&str] = &["bake", "bake-format", "bake-fps"];
pub const RUN_KEYS: &[&str] = &["glb", "out", "bake", "bake-format", "bake-fps"];
pub const CLIPS: &[&str] = &["sway"];
pub const FORMATS: &[&str] = &["bones", "vertex"];
const SWAY_SECONDS: f64 = 4.0;
const SETTLE_SECONDS: f64 = 1.0;
const STEP_HZ: f64 = 60.0;
const REST_MARGIN_M: f64 = 0.001;
const MAX_VERTEX_BYTES: usize = 256 << 20;
const FLOAT: u32 = 5126;
const UNSIGNED_BYTE: u32 = 5121;
const UNSIGNED_SHORT: u32 = 5123;
const ARRAY_BUFFER: u32 = 34962;
const IDENTITY: Q = [0.0, 0.0, 0.0, 1.0];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    Bones,
    Vertex,
}

pub struct Settings {
    pub clip: String,
    pub format: Format,
    pub fps: f64,
}

pub fn settings(opts: &Options) -> Result<Option<Settings>, String> {
    let Some(clip) = opts.get("bake") else {
        if let Some(k) = KEYS[1..].iter().find(|k| opts.contains_key(**k)) {
            return Err(format!("--{k} needs --bake"));
        }
        return Ok(None);
    };
    let format = match opts.get("bake-format").map(String::as_str).unwrap_or("bones") {
        "bones" => Format::Bones,
        "vertex" => Format::Vertex,
        other => return Err(format!("--bake-format must be bones or vertex, got {other}")),
    };
    let fps = match opts.get("bake-fps") {
        Some(s) => s.parse::<f64>().map_err(|_| format!("--bake-fps must be a number, got {s}"))?,
        None => 30.0,
    };
    if !(1.0..=STEP_HZ).contains(&fps) {
        return Err(format!("--bake-fps must be from 1 to {STEP_HZ}, got {fps}"));
    }
    Ok(Some(Settings { clip: clip.clone(), format, fps }))
}

pub fn run(opts: &Options) -> Result<Value, String> {
    if let Some(k) = opts.keys().find(|k| !RUN_KEYS.contains(&k.as_str())) {
        return Err(format!("wearable bake does not read --{k}"));
    }
    let input = opts.get("glb").ok_or("wearable bake needs --glb")?;
    let out = opts.get("out").ok_or("wearable bake needs --out")?;
    let s = settings(opts)?.ok_or("wearable bake needs --bake (a built-in clip: sway, or the name of an animation in the glb)")?;
    let bytes = std::fs::read(input).map_err(|e| format!("{input}: {e}"))?;
    let (baked, report) = bake(&bytes, &s)?;
    if Path::new(out) == Path::new(input) {
        return Err("--out must not be the --glb it reads".into());
    }
    std::fs::write(out, &baked).map_err(|e| format!("{out}: {e}"))?;
    Ok(json!({"ok": true, "command": "wearable bake", "glb": input, "out": out, "bytes": baked.len(), "bake": report}))
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: V3) -> V3 {
    let l = norm(a);
    if l < 1e-12 { a } else { scale(a, 1.0 / l) }
}

fn qmul(a: Q, b: Q) -> Q {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn qconj(q: Q) -> Q {
    [-q[0], -q[1], -q[2], q[3]]
}

fn qnorm(q: Q) -> Q {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt().max(1e-300);
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

fn qdot(a: Q, b: Q) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn qangle(a: Q, b: Q) -> f64 {
    let d = qmul(qconj(qnorm(a)), qnorm(b));
    2.0 * norm([d[0], d[1], d[2]]).atan2(d[3].abs())
}

fn axis_angle(axis: V3, angle: f64) -> Q {
    let a = unit(axis);
    let s = (angle / 2.0).sin();
    [a[0] * s, a[1] * s, a[2] * s, (angle / 2.0).cos()]
}

fn from_to(a: V3, b: V3) -> Q {
    let (a, b) = (unit(a), unit(b));
    let d = dot(a, b);
    if d < -1.0 + 1e-9 {
        let p = unit(if a[0].abs() < 0.9 { cross(a, [1.0, 0.0, 0.0]) } else { cross(a, [0.0, 1.0, 0.0]) });
        return [p[0], p[1], p[2], 0.0];
    }
    let c = cross(a, b);
    qnorm([c[0], c[1], c[2], 1.0 + d])
}

fn slerp(a: Q, b: Q, t: f64) -> Q {
    let b = if qdot(a, b) < 0.0 { [-b[0], -b[1], -b[2], -b[3]] } else { b };
    let d = qdot(a, b).clamp(-1.0, 1.0);
    if d > 0.9995 {
        return qnorm(std::array::from_fn(|i| a[i] + t * (b[i] - a[i])));
    }
    let th = d.acos();
    let (sa, sb) = (((1.0 - t) * th).sin() / th.sin(), (t * th).sin() / th.sin());
    std::array::from_fn(|i| sa * a[i] + sb * b[i])
}

#[derive(Clone, Copy, Debug)]
struct Trs {
    t: V3,
    r: Q,
    s: V3,
}

fn matrix(p: &Trs) -> M4 {
    let [x, y, z, w] = qnorm(p.r);
    let rot = [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ];
    let mut m = [[0.0; 4]; 4];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = rot[i][j] * p.s[j];
        }
        m[i][3] = p.t[i];
    }
    m[3][3] = 1.0;
    m
}

fn mmul(a: &M4, b: &M4) -> M4 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| a[i][k] * b[k][j]).sum()))
}

fn apply(m: &M4, p: V3) -> V3 {
    std::array::from_fn(|i| m[i][0] * p[0] + m[i][1] * p[1] + m[i][2] * p[2] + m[i][3])
}

fn linear(m: &M4, v: V3) -> V3 {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

fn origin(m: &M4) -> V3 {
    [m[0][3], m[1][3], m[2][3]]
}

fn inverse(m: &M4) -> Option<M4> {
    let a = |i: usize, j: usize| m[i][j];
    let c00 = a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1);
    let c01 = a(1, 2) * a(2, 0) - a(1, 0) * a(2, 2);
    let c02 = a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0);
    let det = a(0, 0) * c00 + a(0, 1) * c01 + a(0, 2) * c02;
    if det.abs() < 1e-18 || !det.is_finite() {
        return None;
    }
    let r = [
        [c00, a(0, 2) * a(2, 1) - a(0, 1) * a(2, 2), a(0, 1) * a(1, 2) - a(0, 2) * a(1, 1)],
        [c01, a(0, 0) * a(2, 2) - a(0, 2) * a(2, 0), a(0, 2) * a(1, 0) - a(0, 0) * a(1, 2)],
        [c02, a(0, 1) * a(2, 0) - a(0, 0) * a(2, 1), a(0, 0) * a(1, 1) - a(0, 1) * a(1, 0)],
    ];
    let mut out = [[0.0; 4]; 4];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = r[i][j] / det;
        }
    }
    let t = origin(m);
    for row in out.iter_mut().take(3) {
        row[3] = -(row[0] * t[0] + row[1] * t[1] + row[2] * t[2]);
    }
    out[3][3] = 1.0;
    Some(out)
}

fn rotation_of(m: &M4) -> Q {
    let col = |j: usize| unit([m[0][j], m[1][j], m[2][j]]);
    let (x, y, z) = (col(0), col(1), col(2));
    let r = [[x[0], y[0], z[0]], [x[1], y[1], z[1]], [x[2], y[2], z[2]]];
    let trace = r[0][0] + r[1][1] + r[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(r[2][1] - r[1][2]) / s, (r[0][2] - r[2][0]) / s, (r[1][0] - r[0][1]) / s, 0.25 * s]
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let s = (1.0 + r[0][0] - r[1][1] - r[2][2]).sqrt() * 2.0;
        [0.25 * s, (r[0][1] + r[1][0]) / s, (r[0][2] + r[2][0]) / s, (r[2][1] - r[1][2]) / s]
    } else if r[1][1] > r[2][2] {
        let s = (1.0 + r[1][1] - r[0][0] - r[2][2]).sqrt() * 2.0;
        [(r[0][1] + r[1][0]) / s, 0.25 * s, (r[1][2] + r[2][1]) / s, (r[0][2] - r[2][0]) / s]
    } else {
        let s = (1.0 + r[2][2] - r[0][0] - r[1][1]).sqrt() * 2.0;
        [(r[0][2] + r[2][0]) / s, (r[1][2] + r[2][1]) / s, 0.25 * s, (r[1][0] - r[0][1]) / s]
    };
    qnorm(q)
}

fn column_major(v: &[f32]) -> M4 {
    std::array::from_fn(|i| std::array::from_fn(|j| v[j * 4 + i] as f64))
}

fn closest_on_segment(p: V3, a: V3, b: V3) -> V3 {
    let ab = sub(b, a);
    let l2 = dot(ab, ab);
    if l2 < 1e-18 {
        return a;
    }
    add(a, scale(ab, (dot(sub(p, a), ab) / l2).clamp(0.0, 1.0)))
}

struct Scene {
    parent: Vec<Option<usize>>,
    rest: Vec<Trs>,
    order: Vec<usize>,
    names: Vec<String>,
}

impl Scene {
    fn read(doc: &Value) -> Result<Scene, String> {
        let nodes = doc["nodes"].as_array().ok_or("the glb has no nodes")?;
        let n = nodes.len();
        let mut parent = vec![None; n];
        let mut children = vec![Vec::new(); n];
        let mut rest = Vec::with_capacity(n);
        for (i, node) in nodes.iter().enumerate() {
            if node.get("matrix").is_some() {
                return Err(format!("node {i} has a matrix; the bake reads translation, rotation and scale only"));
            }
            let v = |key: &str, d: &[f64]| -> Vec<f64> { node[key].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_else(|| d.to_vec()) };
            let (t, r, s) = (v("translation", &[0.0; 3]), v("rotation", &IDENTITY), v("scale", &[1.0; 3]));
            if t.len() != 3 || r.len() != 4 || s.len() != 3 {
                return Err(format!("node {i} has a malformed transform"));
            }
            rest.push(Trs { t: [t[0], t[1], t[2]], r: [r[0], r[1], r[2], r[3]], s: [s[0], s[1], s[2]] });
            for c in node["children"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
                let c = c as usize;
                if c >= n || parent[c].is_some() {
                    return Err(format!("node {c} is a child twice or does not exist"));
                }
                parent[c] = Some(i);
                children[i].push(c);
            }
        }
        let mut order: Vec<usize> = (0..n).filter(|&i| parent[i].is_none()).collect();
        let mut k = 0;
        while k < order.len() {
            let i = order[k];
            order.extend(children[i].iter().copied());
            k += 1;
        }
        if order.len() != n {
            return Err("the node hierarchy has a cycle".into());
        }
        let names = nodes.iter().enumerate().map(|(i, n)| n["name"].as_str().map_or(format!("node{i}"), String::from)).collect();
        Ok(Scene { parent, rest, order, names })
    }

    fn world(&self, pose: &[Trs]) -> Vec<M4> {
        let mut w = vec![[[0.0; 4]; 4]; pose.len()];
        for &n in &self.order {
            let m = matrix(&pose[n]);
            w[n] = match self.parent[n] {
                Some(p) => mmul(&w[p], &m),
                None => m,
            };
        }
        w
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum PathKind {
    Translation,
    Rotation,
    Scale,
}

struct Track {
    node: usize,
    path: PathKind,
    times: Vec<f64>,
    values: Vec<f64>,
    step: bool,
}

impl Track {
    fn width(&self) -> usize {
        if self.path == PathKind::Rotation { 4 } else { 3 }
    }

    fn at(&self, t: f64) -> Vec<f64> {
        let w = self.width();
        let n = self.times.len();
        let key = |k: usize| &self.values[k * w..(k + 1) * w];
        if n == 1 || t <= self.times[0] {
            return key(0).to_vec();
        }
        if t >= self.times[n - 1] {
            return key(n - 1).to_vec();
        }
        let k = self.times.partition_point(|&x| x <= t) - 1;
        let f = (t - self.times[k]) / (self.times[k + 1] - self.times[k]).max(1e-12);
        let (a, b) = (key(k), key(k + 1));
        if self.step {
            return a.to_vec();
        }
        if self.path == PathKind::Rotation {
            return slerp([a[0], a[1], a[2], a[3]], [b[0], b[1], b[2], b[3]], f).to_vec();
        }
        (0..w).map(|i| a[i] + f * (b[i] - a[i])).collect()
    }
}

struct Clip {
    name: String,
    duration: f64,
    tracks: Vec<Track>,
    source: Option<usize>,
}

impl Clip {
    fn pose(&self, scene: &Scene, t: f64) -> Vec<Trs> {
        let mut p = scene.rest.clone();
        for track in &self.tracks {
            let v = track.at(t);
            let node = &mut p[track.node];
            match track.path {
                PathKind::Translation => node.t = [v[0], v[1], v[2]],
                PathKind::Rotation => node.r = qnorm([v[0], v[1], v[2], v[3]]),
                PathKind::Scale => node.s = [v[0], v[1], v[2]],
            }
        }
        p
    }
}

fn floats_of(doc: &Value, bin: &[u8], accessor: u64) -> Result<Vec<f64>, String> {
    let a = &doc["accessors"][accessor as usize];
    let raw = gltf::read_plain(doc, bin, accessor)?;
    let divisor = match (a["normalized"].as_bool().unwrap_or(false), a["componentType"].as_u64().map(|c| c as u32)) {
        (true, Some(UNSIGNED_BYTE)) => 255.0,
        (true, Some(UNSIGNED_SHORT)) => 65535.0,
        _ => 1.0,
    };
    Ok(raw.iter().map(|&x| x as f64 / divisor).collect())
}

fn gltf_clip(doc: &Value, bin: &[u8], name: &str) -> Result<Clip, String> {
    let animations = doc["animations"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let Some(index) = animations.iter().position(|a| a["name"].as_str() == Some(name)) else {
        let have: Vec<&str> = animations.iter().filter_map(|a| a["name"].as_str()).collect();
        return Err(format!("--bake {name}: not a built-in clip ({}) and the glb has no animation of that name (it has: {})", CLIPS.join(", "), if have.is_empty() { "none".to_string() } else { have.join(", ") }));
    };
    let anim = &animations[index];
    let mut tracks = Vec::new();
    for ch in anim["channels"].as_array().into_iter().flatten() {
        let path = match ch["target"]["path"].as_str() {
            Some("translation") => PathKind::Translation,
            Some("rotation") => PathKind::Rotation,
            Some("scale") => PathKind::Scale,
            _ => continue,
        };
        let Some(node) = ch["target"]["node"].as_u64() else { continue };
        let sampler = &anim["samplers"][ch["sampler"].as_u64().ok_or("a channel has no sampler")? as usize];
        let step = match sampler["interpolation"].as_str().unwrap_or("LINEAR") {
            "LINEAR" => false,
            "STEP" => true,
            other => return Err(format!("animation {name}: {other} samplers are not read by the bake (LINEAR and STEP are)")),
        };
        let output = sampler["output"].as_u64().ok_or("a sampler has no output")?;
        if path == PathKind::Rotation && doc["accessors"][output as usize]["componentType"].as_u64() != Some(FLOAT as u64) {
            return Err(format!("animation {name}: a rotation output that is not float is not read by the bake"));
        }
        let times = floats_of(doc, bin, sampler["input"].as_u64().ok_or("a sampler has no input")?)?;
        let values = floats_of(doc, bin, output)?;
        let track = Track { node: node as usize, path, times, values, step };
        if track.times.is_empty() || track.values.len() != track.times.len() * track.width() || track.node >= doc["nodes"].as_array().map_or(0, Vec::len) {
            return Err(format!("animation {name}: a channel's keys do not match its times or its node"));
        }
        tracks.push(track);
    }
    let duration = tracks.iter().filter_map(|t| t.times.last().copied()).fold(0.0, f64::max);
    if duration <= 0.0 {
        return Err(format!("animation {name} moves no joint over any time, so there is nothing to bake against"));
    }
    Ok(Clip { name: name.to_string(), duration, tracks, source: Some(index) })
}

struct Skin {
    joints: Vec<usize>,
    ibm: Vec<M4>,
}

fn read_skin(doc: &Value, bin: &[u8], k: usize) -> Result<Skin, String> {
    let skin = &doc["skins"][k];
    let joints: Vec<usize> = skin["joints"].as_array().ok_or(format!("skin {k} has no joints"))?.iter().filter_map(Value::as_u64).map(|j| j as usize).collect();
    let ibm = match skin["inverseBindMatrices"].as_u64() {
        Some(a) => {
            let v = gltf::read_plain(doc, bin, a)?;
            if v.len() != joints.len() * 16 {
                return Err(format!("skin {k}: {} inverse bind matrices for {} joints", v.len() / 16, joints.len()));
            }
            v.chunks_exact(16).map(column_major).collect()
        }
        None => vec![matrix(&Trs { t: [0.0; 3], r: IDENTITY, s: [1.0; 3] }); joints.len()],
    };
    Ok(Skin { joints, ibm })
}

struct Chain {
    nodes: Vec<usize>,
    parent: usize,
    wearable: String,
    radii: Vec<(usize, f64)>,
    hit: f64,
}

impl Chain {
    fn radius(&self, k: usize, cap: &Capsule) -> f64 {
        self.radii.iter().find(|r| r.0 == k).map_or(cap.radius, |r| r.1)
    }
}

struct Capsule {
    node: usize,
    ibm: M4,
    head: V3,
    tail: V3,
    radius: f64,
    name: String,
}

struct Spring {
    stiffness: f64,
    drag: f64,
    gravity: f64,
    hit: f64,
}

fn v3(v: &Value) -> Option<V3> {
    let a = v.as_array().filter(|a| a.len() == 3)?;
    Some([a[0].as_f64()?, a[1].as_f64()?, a[2].as_f64()?])
}

fn sway_clip(scene: &Scene, anchors: &[usize], frame: &M4, top: usize, unit_scale: f64, fps: f64) -> Clip {
    let frames = (SWAY_SECONDS * fps).round().max(1.0) as usize;
    let times: Vec<f64> = (0..=frames).map(|k| k as f64 * SWAY_SECONDS / frames as f64).collect();
    type Wave = (V3, f64, f64);
    let mut motions: Vec<(usize, Vec<Wave>)> = Vec::new();
    let mut add_motion = |node: usize, m: (V3, f64, f64)| match motions.iter_mut().find(|(n, _)| *n == node) {
        Some((_, list)) => {
            if !list.contains(&m) {
                list.push(m)
            }
        }
        None => motions.push((node, vec![m])),
    };
    for &p in anchors {
        add_motion(p, ([0.0, 1.0, 0.0], 35f64.to_radians(), 2.0));
        if let Some(pp) = scene.parent[p].filter(|&pp| pp != top && Some(pp) != scene.parent[top]) {
            add_motion(pp, ([0.0, 0.0, 1.0], 8f64.to_radians(), 1.0));
            if let Some(ppp) = scene.parent[pp].filter(|&x| x != top && Some(x) != scene.parent[top]) {
                add_motion(ppp, ([1.0, 0.0, 0.0], 10f64.to_radians(), 1.0));
            }
        }
    }
    let world = scene.world(&scene.rest);
    let unframe = inverse(frame).unwrap_or(*frame);
    let in_frame = |n: Option<usize>| n.map_or(IDENTITY, |p| rotation_of(&mmul(&unframe, &world[p])));
    let wave = |amp: f64, cycles: f64, t: f64| amp * (2.0 * std::f64::consts::PI * cycles * t / SWAY_SECONDS).sin();
    let mut tracks = Vec::new();
    for (node, list) in &motions {
        let pr = in_frame(scene.parent[*node]);
        let values = times
            .iter()
            .flat_map(|&t| {
                let q = list.iter().fold(IDENTITY, |q, (axis, amp, cycles)| qmul(q, axis_angle(*axis, wave(*amp, *cycles, t))));
                qnorm(qmul(qmul(qmul(qconj(pr), q), pr), scene.rest[*node].r))
            })
            .collect();
        tracks.push(Track { node: *node, path: PathKind::Rotation, times: times.clone(), values, step: false });
    }
    let base = scene.rest[top].t;
    let to_local = scene.parent[top].and_then(|p| inverse(&world[p])).unwrap_or_else(|| matrix(&Trs { t: [0.0; 3], r: IDENTITY, s: [1.0; 3] }));
    let values = times
        .iter()
        .flat_map(|&t| {
            let metres = [wave(0.05, 1.0, t), wave(0.015, 4.0, t), 0.0];
            add(base, linear(&to_local, scale(linear(frame, metres), 1.0 / unit_scale.max(1e-9))))
        })
        .collect();
    tracks.push(Track { node: top, path: PathKind::Translation, times: times.clone(), values, step: false });
    Clip { name: "sway".into(), duration: SWAY_SECONDS, tracks, source: None }
}

struct Baked {
    times: Vec<f64>,
    local: Vec<Vec<Vec<Q>>>,
    report: Value,
}

struct State {
    cur: Vec<Vec<V3>>,
    prev: Vec<Vec<V3>>,
    local: Vec<Vec<Q>>,
}

struct StepStats {
    penetration: (f64, usize, usize),
    length_error: f64,
}

#[allow(clippy::too_many_arguments)]
fn step(scene: &Scene, clip: &Clip, t: f64, chains: &[Chain], caps: &[Capsule], spring: &Spring, dt: f64, unit_scale: f64, state: &mut State) -> StepStats {
    let pose = clip.pose(scene, t);
    let world = scene.world(&pose);
    let placed: Vec<(V3, V3)> = caps
        .iter()
        .map(|c| {
            let m = mmul(&world[c.node], &c.ibm);
            (apply(&m, c.head), apply(&m, c.tail))
        })
        .collect();
    let gravity = [0.0, -spring.gravity * dt, 0.0];
    let mut stats = StepStats { penetration: (0.0, 0, 0), length_error: 0.0 };
    for (ci, chain) in chains.iter().enumerate() {
        let hit = chain.hit * unit_scale;
        let hits = || placed.iter().enumerate().map(|(k, (a, b))| (k, (a, b, chain.radius(k, &caps[k]) * unit_scale)));
        let mut parent_m = world[chain.parent];
        for k in 0..chain.nodes.len() - 1 {
            let r = pose[chain.nodes[k]];
            let rest_m = mmul(&parent_m, &matrix(&r));
            let head = origin(&rest_m);
            let bone = linear(&rest_m, pose[chain.nodes[k + 1]].t);
            let length = norm(bone);
            let axis = unit(bone);
            let (c, p) = (state.cur[ci][k], state.prev[ci][k]);
            let mut next = add(add(add(c, scale(sub(c, p), 1.0 - spring.drag)), scale(axis, spring.stiffness * dt)), gravity);
            next = add(head, scale(unit(sub(next, head)), length));
            for (_, (a, b, radius)) in hits() {
                let (a, b) = (*a, *b);
                let q = closest_on_segment(next, a, b);
                let d = sub(next, q);
                let reach = radius + hit;
                if norm(d) < reach {
                    let away = if norm(d) > 1e-9 { unit(d) } else { axis };
                    next = add(q, scale(away, reach));
                    next = add(head, scale(unit(sub(next, head)), length));
                }
            }
            for (cap, (a, b, radius)) in hits() {
                let gap = radius + hit - norm(sub(next, closest_on_segment(next, *a, *b)));
                if gap > stats.penetration.0 {
                    stats.penetration = (gap, ci, cap);
                }
            }
            stats.length_error = stats.length_error.max((norm(sub(next, head)) - length).abs());
            state.prev[ci][k] = c;
            state.cur[ci][k] = next;
            let turn = from_to(axis, sub(next, head));
            let pr = rotation_of(&parent_m);
            let lr = qnorm(qmul(qmul(qmul(qconj(pr), turn), pr), r.r));
            state.local[ci][k] = lr;
            parent_m = mmul(&parent_m, &matrix(&Trs { t: r.t, r: lr, s: r.s }));
        }
    }
    stats
}

fn simulate(scene: &Scene, clip: &Clip, chains: &[Chain], caps: &[Capsule], spring: &Spring, fps: f64, unit_scale: f64) -> Baked {
    let frames = (clip.duration * fps).round().max(1.0) as usize;
    let times: Vec<f64> = (0..=frames).map(|k| k as f64 * clip.duration / frames as f64).collect();
    let sub_steps = (STEP_HZ * clip.duration / frames as f64).ceil().max(1.0) as usize;
    let dt = clip.duration / frames as f64 / sub_steps as f64;
    let start = scene.world(&clip.pose(scene, 0.0));
    let cur: Vec<Vec<V3>> = chains.iter().map(|c| c.nodes[1..].iter().map(|&n| origin(&start[n])).collect()).collect();
    let local: Vec<Vec<Q>> = chains.iter().map(|c| c.nodes[..c.nodes.len() - 1].iter().map(|&n| scene.rest[n].r).collect()).collect();
    let mut state = State { prev: cur.clone(), cur, local };
    let settle = (SETTLE_SECONDS / dt).round() as usize;
    for _ in 0..settle {
        step(scene, clip, 0.0, chains, caps, spring, dt, unit_scale, &mut state);
    }
    let mut out: Vec<Vec<Vec<Q>>> = chains.iter().map(|c| vec![Vec::with_capacity(frames + 1); c.nodes.len() - 1]).collect();
    let mut tip_moved: Vec<(f64, f64)> = vec![(0.0, 0.0); chains.len()];
    let mut record = |state: &State, out: &mut Vec<Vec<Vec<Q>>>, t: f64, first: bool| {
        let still = scene.world(&clip.pose(scene, t));
        for (ci, segs) in state.local.iter().enumerate() {
            for (k, q) in segs.iter().enumerate() {
                out[ci][k].push(*q);
            }
            let moved = norm(sub(*state.cur[ci].last().expect("a chain has a tail"), origin(&still[*chains[ci].nodes.last().expect("a chain has joints")])));
            if first {
                tip_moved[ci].0 = moved;
            }
            tip_moved[ci].1 = tip_moved[ci].1.max(moved);
        }
    };
    record(&state, &mut out, 0.0, true);
    let (mut penetration, mut length_error) = ((0.0f64, 0, 0), 0.0f64);
    for f in 1..=frames {
        for s in 1..=sub_steps {
            let t = times[f - 1] + (times[f] - times[f - 1]) * s as f64 / sub_steps as f64;
            let st = step(scene, clip, t, chains, caps, spring, dt, unit_scale, &mut state);
            if st.penetration.0 > penetration.0 {
                penetration = st.penetration;
            }
            length_error = length_error.max(st.length_error);
        }
        record(&state, &mut out, times[f], false);
    }
    let clearance: Vec<(f64, String)> = chains
        .iter()
        .map(|c| {
            let mut best = (f64::INFINITY, String::new());
            for &n in &c.nodes[1..] {
                for (k, cap) in caps.iter().enumerate() {
                    let m = mmul(&start[cap.node], &cap.ibm);
                    let p = origin(&start[n]);
                    let d = norm(sub(p, closest_on_segment(p, apply(&m, cap.head), apply(&m, cap.tail)))) - c.radius(k, cap) * unit_scale;
                    if d < best.0 {
                        best = (d, cap.name.clone());
                    }
                }
            }
            best
        })
        .collect();
    let deg = |x: f64| (x.to_degrees() * 1000.0).round() / 1000.0;
    let mm = |x: f64| (x * 1e6).round() / 1000.0;
    let rows: Vec<Value> = chains
        .iter()
        .zip(&out)
        .enumerate()
        .map(|(ci, (c, segs))| {
            let rest: Vec<Q> = c.nodes[..c.nodes.len() - 1].iter().map(|&n| scene.rest[n].r).collect();
            let settled = segs.iter().zip(&rest).map(|(keys, r)| qangle(keys[0], *r)).fold(0.0, f64::max);
            let swing = segs.iter().zip(&rest).flat_map(|(keys, r)| keys.iter().map(move |q| qangle(*q, *r))).fold(0.0, f64::max);
            let loop_gap = segs.iter().map(|keys| qangle(keys[0], keys[keys.len() - 1])).fold(0.0, f64::max);
            json!({"wearable": c.wearable, "chain_root": scene.names[c.nodes[0]], "segments": segs.len(),
                   "settled_from_rest_deg": deg(settled), "max_from_rest_deg": deg(swing), "loop_gap_deg": deg(loop_gap),
                   "tip_settled_mm": mm(tip_moved[ci].0), "tip_max_mm": mm(tip_moved[ci].1),
                   "rest_clearance_mm": if clearance[ci].0.is_finite() { json!(mm(clearance[ci].0)) } else { Value::Null },
                   "rest_clearance_capsule": clearance[ci].1, "hit_radius_mm": mm(c.hit * unit_scale),
                   "shrunk_colliders": c.radii.iter().filter_map(|&(k, r)| caps.get(k).map(|cap| json!({"capsule": cap.name, "radius_mm": mm(r * unit_scale), "from_mm": mm(cap.radius * unit_scale)}))).collect::<Vec<_>>()})
        })
        .collect();
    let report = json!({
        "clip": clip.name, "seconds": clip.duration, "fps": fps, "frames": frames + 1, "substeps_per_frame": sub_steps,
        "settle_seconds": SETTLE_SECONDS, "chains": rows,
        "max_penetration_mm": mm(penetration.0.max(0.0)),
        "max_penetration_at": if penetration.0 > 0.0 { json!({"chain_root": scene.names[chains[penetration.1].nodes[0]], "capsule": caps[penetration.2].name}) } else { Value::Null },
        "max_length_error_mm": mm(length_error),
        "method": "VRM-style spring bones (verlet tails: inertia x (1 - drag), stiffness toward the rest direction, gravity down in the world, length kept, pushed out of each capsule by its radius plus the hit radius), stepped at 60 Hz or finer, after a settle at the clip's first pose; the last joint of each chain is its tip and keeps its rest rotation",
    });
    Baked { times, local: out, report }
}

fn push_bytes(doc: &mut Value, bin: &mut Vec<u8>, bytes: &[u8], component: u32, count: usize, kind: &str, target: Option<u32>) -> usize {
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let offset = bin.len();
    bin.extend_from_slice(bytes);
    let mut view = json!({"buffer": 0, "byteOffset": offset, "byteLength": bytes.len()});
    if let Some(t) = target {
        view["target"] = json!(t);
    }
    let views = doc["bufferViews"].as_array_mut().expect("bufferViews checked");
    views.push(view);
    let view = views.len() - 1;
    let accessors = doc["accessors"].as_array_mut().expect("accessors checked");
    accessors.push(json!({"bufferView": view, "componentType": component, "count": count, "type": kind}));
    accessors.len() - 1
}

fn push_floats(doc: &mut Value, bin: &mut Vec<u8>, values: &[f32], width: usize, target: Option<u32>, bounds: bool) -> usize {
    let kind = match width {
        1 => "SCALAR",
        3 => "VEC3",
        _ => "VEC4",
    };
    let bytes: Vec<u8> = values.iter().flat_map(|x| x.to_le_bytes()).collect();
    let a = push_bytes(doc, bin, &bytes, FLOAT, values.len() / width, kind, target);
    if bounds {
        let lo: Vec<f32> = (0..width).map(|i| values.iter().skip(i).step_by(width).copied().fold(f32::INFINITY, f32::min)).collect();
        let hi: Vec<f32> = (0..width).map(|i| values.iter().skip(i).step_by(width).copied().fold(f32::NEG_INFINITY, f32::max)).collect();
        doc["accessors"][a]["min"] = json!(lo);
        doc["accessors"][a]["max"] = json!(hi);
    }
    a
}

fn clip_channels(doc: &mut Value, bin: &mut Vec<u8>, clip: &Clip, skip: &[usize], input: usize, samplers: &mut Vec<Value>, channels: &mut Vec<Value>) {
    match clip.source {
        Some(index) => {
            let source = doc["animations"][index].clone();
            let mut moved: HashMap<u64, usize> = HashMap::new();
            for ch in source["channels"].as_array().into_iter().flatten() {
                if ch["target"]["node"].as_u64().is_some_and(|n| skip.contains(&(n as usize))) {
                    continue;
                }
                let Some(s) = ch["sampler"].as_u64() else { continue };
                let k = *moved.entry(s).or_insert_with(|| {
                    samplers.push(source["samplers"][s as usize].clone());
                    samplers.len() - 1
                });
                let mut c = ch.clone();
                c["sampler"] = json!(k);
                channels.push(c);
            }
        }
        None => {
            for track in &clip.tracks {
                let values: Vec<f32> = track.values.iter().map(|&x| x as f32).collect();
                let output = push_floats(doc, bin, &values, track.width(), None, false);
                let path = match track.path {
                    PathKind::Translation => "translation",
                    PathKind::Rotation => "rotation",
                    PathKind::Scale => "scale",
                };
                channels.push(json!({"sampler": samplers.len(), "target": {"node": track.node, "path": path}}));
                samplers.push(json!({"input": input, "output": output, "interpolation": "LINEAR"}));
            }
        }
    }
}

struct Weighted {
    full: Vec<Vec<(usize, f64)>>,
    body: Vec<Vec<(usize, f64)>>,
}

fn weights_of(doc: &Value, bin: &[u8], attributes: &Value, n: usize, chain_of: &HashMap<usize, usize>, skin: &Skin, chains: &[Chain]) -> Result<Option<Weighted>, String> {
    let mut full: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for set in 0..2 {
        let (Some(j), Some(w)) = (attributes[format!("JOINTS_{set}")].as_u64(), attributes[format!("WEIGHTS_{set}")].as_u64()) else { continue };
        let (j, w) = (floats_of(doc, bin, j)?, floats_of(doc, bin, w)?);
        if j.len() != 4 * n || w.len() != 4 * n {
            return Err("a skinned primitive's joints or weights do not match its vertices".into());
        }
        for v in 0..n {
            for k in 0..4 {
                if w[4 * v + k] > 0.0 {
                    full[v].push((j[4 * v + k] as usize, w[4 * v + k]));
                }
            }
        }
    }
    let on_chain = |j: usize| skin.joints.get(j).and_then(|node| chain_of.get(node)).copied();
    if !full.iter().flatten().any(|(j, _)| on_chain(*j).is_some()) {
        return Ok(None);
    }
    let mut body = Vec::with_capacity(n);
    for list in &full {
        let kept: Vec<(usize, f64)> = list.iter().filter(|(j, _)| on_chain(*j).is_none()).copied().collect();
        let sum: f64 = kept.iter().map(|x| x.1).sum();
        if sum > 1e-6 {
            body.push(kept.iter().map(|&(j, w)| (j, w / sum)).collect());
            continue;
        }
        let mut by_chain: HashMap<usize, f64> = HashMap::new();
        for (j, w) in list {
            if let Some(c) = on_chain(*j) {
                *by_chain.entry(c).or_default() += w;
            }
        }
        let c = by_chain.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))).map(|x| x.0).ok_or("a skinned vertex has no weight at all")?;
        let parent = skin.joints.iter().position(|&n| n == chains[c].parent).ok_or_else(|| format!("the chain's parent bone {} is not in the mesh's skin", chains[c].parent))?;
        body.push(vec![(parent, 1.0)]);
    }
    Ok(Some(Weighted { full, body }))
}

#[allow(clippy::too_many_arguments)]
fn write_vertex(doc: &mut Value, bin: &mut Vec<u8>, scene: &Scene, clip: &Clip, baked: &Baked, chains: &[Chain], name: &str, input: usize, samplers: &mut Vec<Value>, channels: &mut Vec<Value>) -> Result<Value, String> {
    let chain_of: HashMap<usize, usize> = chains.iter().enumerate().flat_map(|(c, ch)| ch.nodes.iter().map(move |&n| (n, c))).collect();
    let frames = baked.times.len();
    let worlds: Vec<Vec<M4>> = (0..frames)
        .map(|f| {
            let mut pose = clip.pose(scene, baked.times[f]);
            for (chain, segs) in chains.iter().zip(&baked.local) {
                for (k, keys) in segs.iter().enumerate() {
                    pose[chain.nodes[k]].r = keys[f];
                }
            }
            scene.world(&pose)
        })
        .collect();
    let node_count = doc["nodes"].as_array().map_or(0, Vec::len);
    let mut rows = Vec::new();
    let mut planned = 0usize;
    let mut work = Vec::new();
    for node in 0..node_count {
        let (Some(mesh), Some(skin_index)) = (doc["nodes"][node]["mesh"].as_u64(), doc["nodes"][node]["skin"].as_u64()) else { continue };
        let skin = read_skin(doc, bin, skin_index as usize)?;
        let primitives = doc["meshes"][mesh as usize]["primitives"].as_array().cloned().unwrap_or_default();
        let Some(first) = primitives.first() else { continue };
        let attributes = first["attributes"].clone();
        if primitives.iter().any(|p| p["attributes"] != attributes) {
            return Err(format!("mesh {}: primitives with different vertex attributes are not baked", doc["meshes"][mesh as usize]["name"]));
        }
        let position = attributes["POSITION"].as_u64().ok_or("a skinned primitive has no POSITION")?;
        let n = doc["accessors"][position as usize]["count"].as_u64().unwrap_or(0) as usize;
        let Some(weighted) = weights_of(doc, bin, &attributes, n, &chain_of, &skin, chains)? else { continue };
        planned += n * frames * if attributes["NORMAL"].is_u64() { 24 } else { 12 };
        work.push((node, mesh as usize, skin, attributes, n, weighted));
    }
    if work.is_empty() {
        return Err("no skinned mesh is weighted to a spring chain, so there is nothing to bake as vertex animation".into());
    }
    if planned > MAX_VERTEX_BYTES {
        return Err(format!("--bake-format vertex would add {} MB of morph targets (limit {} MB); lower --bake-fps or use --bake-format bones", planned >> 20, MAX_VERTEX_BYTES >> 20));
    }
    for (node, mesh, skin, attributes, n, weighted) in work {
        let positions = floats_of(doc, bin, attributes["POSITION"].as_u64().unwrap_or(0))?;
        let normals = match attributes["NORMAL"].as_u64() {
            Some(a) => Some(floats_of(doc, bin, a)?),
            None => None,
        };
        let existing = doc["meshes"][mesh]["primitives"][0]["targets"].as_array().map_or(0, Vec::len);
        let mut targets = Vec::with_capacity(frames);
        let mut max_delta = 0.0f64;
        for world in &worlds {
            let joint_m: Vec<M4> = skin.joints.iter().zip(&skin.ibm).map(|(&j, ibm)| mmul(&world[j], ibm)).collect();
            let blend = |list: &[(usize, f64)]| -> M4 {
                let mut m = [[0.0; 4]; 4];
                for &(j, w) in list {
                    for (row, src) in m.iter_mut().zip(&joint_m[j]) {
                        for (x, y) in row.iter_mut().zip(src) {
                            *x += w * y;
                        }
                    }
                }
                m
            };
            let mut dp = Vec::with_capacity(3 * n);
            let mut dn = Vec::with_capacity(if normals.is_some() { 3 * n } else { 0 });
            for v in 0..n {
                let full = blend(&weighted.full[v]);
                let body = blend(&weighted.body[v]);
                let back = inverse(&body).ok_or("a vertex's remaining skin matrix cannot be inverted")?;
                let p = [positions[3 * v], positions[3 * v + 1], positions[3 * v + 2]];
                let d = sub(apply(&back, apply(&full, p)), p);
                max_delta = max_delta.max(norm(d));
                dp.extend(d.iter().map(|&x| x as f32));
                if let Some(nn) = &normals {
                    let q = [nn[3 * v], nn[3 * v + 1], nn[3 * v + 2]];
                    let d = sub(linear(&back, linear(&full, q)), q);
                    dn.extend(d.iter().map(|&x| x as f32));
                }
            }
            let mut target = json!({"POSITION": push_floats(doc, bin, &dp, 3, Some(ARRAY_BUFFER), true)});
            if normals.is_some() {
                target["NORMAL"] = json!(push_floats(doc, bin, &dn, 3, Some(ARRAY_BUFFER), false));
            }
            targets.push(target);
        }
        let most = weighted.body.iter().map(Vec::len).max().unwrap_or(1);
        let sets = if most > 4 { 2 } else { 1 };
        let mut new_attributes = attributes.clone();
        for set in 0..2 {
            new_attributes.as_object_mut().expect("attributes object").remove(&format!("JOINTS_{set}"));
            new_attributes.as_object_mut().expect("attributes object").remove(&format!("WEIGHTS_{set}"));
        }
        for set in 0..sets {
            let mut joints = Vec::with_capacity(8 * n);
            let mut weights = Vec::with_capacity(4 * n);
            for list in &weighted.body {
                for k in 0..4 {
                    let (j, w) = list.get(4 * set + k).copied().unwrap_or((0, 0.0));
                    joints.extend((j as u16).to_le_bytes());
                    weights.push(w as f32);
                }
            }
            new_attributes[format!("JOINTS_{set}")] = json!(push_bytes(doc, bin, &joints, UNSIGNED_SHORT, n, "VEC4", Some(ARRAY_BUFFER)));
            new_attributes[format!("WEIGHTS_{set}")] = json!(push_floats(doc, bin, &weights, 4, Some(ARRAY_BUFFER), false));
        }
        let m = &mut doc["meshes"][mesh];
        for p in m["primitives"].as_array_mut().into_iter().flatten() {
            p["attributes"] = new_attributes.clone();
            let mut list = p["targets"].as_array().cloned().unwrap_or_default();
            list.extend(targets.iter().cloned());
            p["targets"] = json!(list);
        }
        let mut defaults = m["weights"].as_array().cloned().unwrap_or_else(|| vec![json!(0.0); existing]);
        defaults.extend(std::iter::repeat_n(json!(0.0), frames));
        m["weights"] = json!(defaults);
        let mut names = m["extras"]["targetNames"].as_array().cloned().unwrap_or_default();
        if names.len() == existing {
            names.extend((0..frames).map(|f| json!(format!("{name}_{f:04}"))));
            m["extras"]["targetNames"] = json!(names);
        }
        let total = existing + frames;
        let mut keys = vec![0.0f32; frames * total];
        for f in 0..frames {
            keys[f * total + existing + f] = 1.0;
        }
        let output = push_floats(doc, bin, &keys, 1, None, false);
        channels.push(json!({"sampler": samplers.len(), "target": {"node": node, "path": "weights"}}));
        samplers.push(json!({"input": input, "output": output, "interpolation": "LINEAR"}));
        rows.push(json!({"node": scene.names[node], "vertices": n, "targets": frames, "kept_targets": existing, "skin_sets": sets,
                         "max_delta_mm": (max_delta * 1e6).round() / 1000.0}));
    }
    Ok(json!(rows))
}

const CLOTH_HZ: f64 = 240.0;
const CLOTH_ITERATIONS: usize = 12;
const CLOTH_GRAVITY: f64 = 9.81;
const CLOTH_DAMPING: f64 = 0.02;
const CLOTH_BEND: f64 = 0.2;
const CLOTH_THICKNESS_M: f64 = 0.004;
const PINNED_REPORT: f64 = 0.99;

struct ClothMesh {
    node: usize,
    mesh: usize,
    skin: Skin,
    bind: Vec<V3>,
    normals: Option<Vec<V3>>,
    weights: Vec<Vec<(usize, f64)>>,
    pin: Vec<f64>,
    triangles: Vec<[usize; 3]>,
}

fn read_cloth_mesh(doc: &Value, bin: &[u8], node: usize, pin_name: &str) -> Result<ClothMesh, String> {
    let n_ = &doc["nodes"][node];
    let (Some(mesh), Some(skin_index)) = (n_["mesh"].as_u64(), n_["skin"].as_u64()) else {
        return Err(format!("cloth node {} has no skinned mesh", n_["name"]));
    };
    let skin = read_skin(doc, bin, skin_index as usize)?;
    let primitives = doc["meshes"][mesh as usize]["primitives"].as_array().cloned().unwrap_or_default();
    let attributes = primitives.first().ok_or("a cloth mesh has no primitive")?["attributes"].clone();
    if primitives.iter().any(|p| p["attributes"] != attributes) {
        return Err(format!("cloth node {}: primitives with different vertex attributes are not baked", n_["name"]));
    }
    let v3s = |a: Vec<f64>| -> Vec<V3> { a.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect() };
    let bind = v3s(floats_of(doc, bin, attributes["POSITION"].as_u64().ok_or("a cloth primitive has no POSITION")?)?);
    let normals = match attributes["NORMAL"].as_u64() {
        Some(a) => Some(v3s(floats_of(doc, bin, a)?)),
        None => None,
    };
    let n = bind.len();
    let pin = match attributes[pin_name].as_u64() {
        Some(a) => floats_of(doc, bin, a)?,
        None => return Err(format!("cloth node {} has no {pin_name} attribute", n_["name"])),
    };
    if pin.len() != n {
        return Err(format!("cloth node {}: {pin_name} does not match its vertices", n_["name"]));
    }
    let mut weights: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for set in 0..2 {
        let (Some(j), Some(w)) = (attributes[format!("JOINTS_{set}")].as_u64(), attributes[format!("WEIGHTS_{set}")].as_u64()) else { continue };
        let (j, w) = (floats_of(doc, bin, j)?, floats_of(doc, bin, w)?);
        for v in 0..n {
            for k in 0..4 {
                if w[4 * v + k] > 0.0 {
                    weights[v].push((j[4 * v + k] as usize, w[4 * v + k]));
                }
            }
        }
    }
    let mut triangles = Vec::new();
    for p in &primitives {
        let idx = gltf::read_indices(doc, bin, p["indices"].as_u64().ok_or("a cloth primitive has no indices")?)?;
        triangles.extend(idx.chunks_exact(3).map(|t| [t[0] as usize, t[1] as usize, t[2] as usize]));
    }
    Ok(ClothMesh { node, mesh: mesh as usize, skin, bind, normals, weights, pin, triangles })
}

fn joint_matrices(world: &[M4], skin: &Skin) -> Vec<M4> {
    skin.joints.iter().zip(&skin.ibm).map(|(&j, ibm)| mmul(&world[j], ibm)).collect()
}

fn blend(list: &[(usize, f64)], joint_m: &[M4]) -> M4 {
    let mut m = [[0.0; 4]; 4];
    for &(j, w) in list {
        for (row, src) in m.iter_mut().zip(&joint_m[j]) {
            for (x, y) in row.iter_mut().zip(src) {
                *x += w * y;
            }
        }
    }
    m
}

fn face_normals(points: &[V3], triangles: &[[usize; 3]]) -> Vec<V3> {
    let mut n = vec![[0.0; 3]; points.len()];
    for t in triangles {
        let f = cross(sub(points[t[1]], points[t[0]]), sub(points[t[2]], points[t[0]]));
        for &v in t {
            n[v] = add(n[v], f);
        }
    }
    n.into_iter().map(unit).collect()
}

const BODY_REACH_M: f64 = 0.25;
const BODY_COLLIDE: bool = true;
const BODY_CELL_M: f64 = 0.05;
const BODY_NEAR_M: f64 = 0.05;

struct BodySurface {
    names: Vec<String>,
    skin: Skin,
    bind: Vec<V3>,
    normals: Vec<V3>,
    weights: Vec<Vec<(usize, f64)>>,
}

struct Grid {
    cell: f64,
    map: HashMap<(i64, i64, i64), Vec<usize>>,
}

impl Grid {
    fn new(points: &[V3], cell: f64) -> Grid {
        let mut map: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
        for (i, p) in points.iter().enumerate() {
            map.entry(Grid::key(*p, cell)).or_default().push(i);
        }
        Grid { cell, map }
    }

    fn key(p: V3, cell: f64) -> (i64, i64, i64) {
        ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64, (p[2] / cell).floor() as i64)
    }

    fn nearest(&self, points: &[V3], p: V3, reach: f64) -> Option<usize> {
        let c = Grid::key(p, self.cell);
        let r = (reach / self.cell).ceil() as i64;
        let mut best: Option<(f64, usize)> = None;
        for dx in -r..=r {
            for dy in -r..=r {
                for dz in -r..=r {
                    for &i in self.map.get(&(c.0 + dx, c.1 + dy, c.2 + dz)).into_iter().flatten() {
                        let d = norm(sub(points[i], p));
                        if d <= reach && best.is_none_or(|b| d < b.0 || (d == b.0 && i < b.1)) {
                            best = Some((d, i));
                        }
                    }
                }
            }
        }
        best.map(|b| b.1)
    }
}

fn read_body_surface(doc: &Value, bin: &[u8], cloths: &[ClothMesh]) -> Result<Option<BodySurface>, String> {
    let near = Grid::new(&cloths.iter().flat_map(|c| c.bind.iter().copied()).collect::<Vec<_>>(), BODY_REACH_M);
    let all: Vec<V3> = cloths.iter().flat_map(|c| c.bind.iter().copied()).collect();
    let mut out: Option<BodySurface> = None;
    let cloth_nodes: Vec<usize> = cloths.iter().map(|c| c.node).collect();
    let opaque = |mesh: u64| {
        doc["meshes"][mesh as usize]["primitives"].as_array().into_iter().flatten().all(|p| {
            p["material"].as_u64().is_none_or(|m| doc["materials"][m as usize]["alphaMode"].as_str().is_none_or(|a| a == "OPAQUE"))
        })
    };
    let candidates: Vec<usize> = doc["nodes"]
        .as_array()
        .map(|nodes| {
            (0..nodes.len())
                .filter(|&i| nodes[i]["name"].as_str().is_some_and(|n| n.ends_with("_LOD0")) && !cloth_nodes.contains(&i))
                .filter(|&i| nodes[i]["skin"].is_u64() && nodes[i]["mesh"].as_u64().is_some_and(opaque))
                .collect()
        })
        .unwrap_or_default();
    for node in candidates {
        let n_ = &doc["nodes"][node];
        let (Some(mesh), Some(skin_index)) = (n_["mesh"].as_u64(), n_["skin"].as_u64()) else { continue };
        let attributes = doc["meshes"][mesh as usize]["primitives"][0]["attributes"].clone();
        let (Some(pa), Some(na)) = (attributes["POSITION"].as_u64(), attributes["NORMAL"].as_u64()) else { continue };
        let v3s = |a: Vec<f64>| -> Vec<V3> { a.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect() };
        let (pos, nor) = (v3s(floats_of(doc, bin, pa)?), v3s(floats_of(doc, bin, na)?));
        let mut weights: Vec<Vec<(usize, f64)>> = vec![Vec::new(); pos.len()];
        for set in 0..2 {
            let (Some(j), Some(w)) = (attributes[format!("JOINTS_{set}")].as_u64(), attributes[format!("WEIGHTS_{set}")].as_u64()) else { continue };
            let (j, w) = (floats_of(doc, bin, j)?, floats_of(doc, bin, w)?);
            for v in 0..pos.len() {
                for k in 0..4 {
                    if w[4 * v + k] > 0.0 {
                        weights[v].push((j[4 * v + k] as usize, w[4 * v + k]));
                    }
                }
            }
        }
        let skin = read_skin(doc, bin, skin_index as usize)?;
        let body = out.get_or_insert_with(|| BodySurface { names: Vec::new(), skin: Skin { joints: skin.joints.clone(), ibm: skin.ibm.clone() }, bind: Vec::new(), normals: Vec::new(), weights: Vec::new() });
        if body.skin.joints != skin.joints {
            continue;
        }
        let before = body.bind.len();
        for v in 0..pos.len() {
            if near.nearest(&all, pos[v], BODY_REACH_M).is_some() {
                body.bind.push(pos[v]);
                body.normals.push(nor[v]);
                body.weights.push(std::mem::take(&mut weights[v]));
            }
        }
        if body.bind.len() > before {
            body.names.push(n_["name"].as_str().unwrap_or("").to_string());
        }
    }
    Ok(out.filter(|b| !b.bind.is_empty()))
}

struct ClothRun {
    positions: Vec<Vec<V3>>,
    report: Value,
}

fn simulate_cloth(scene: &Scene, clip: &Clip, cloth: &ClothMesh, caps: &[Capsule], body: Option<&BodySurface>, times: &[f64], unit_scale: f64) -> ClothRun {
    let n = cloth.bind.len();
    let skinned = |t: f64| -> Vec<V3> {
        let world = scene.world(&clip.pose(scene, t));
        let jm = joint_matrices(&world, &cloth.skin);
        (0..n).map(|v| apply(&blend(&cloth.weights[v], &jm), cloth.bind[v])).collect()
    };
    let placed = |t: f64| -> Vec<(V3, V3, f64)> {
        let world = scene.world(&clip.pose(scene, t));
        caps.iter().map(|c| {
            let m = mmul(&world[c.node], &c.ibm);
            (apply(&m, c.head), apply(&m, c.tail), c.radius * unit_scale + CLOTH_THICKNESS_M)
        }).collect()
    };
    let start = skinned(times[0]);
    let mut edges: HashMap<(usize, usize), f64> = HashMap::new();
    let mut opposite: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for t in &cloth.triangles {
        for k in 0..3 {
            let (a, b) = (t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3]));
            edges.insert((a, b), norm(sub(start[a], start[b])));
            opposite.entry((a, b)).or_default().push(t[(k + 2) % 3]);
        }
    }
    let mut edges: Vec<(usize, usize, f64)> = edges.into_iter().map(|((a, b), l)| (a, b, l)).collect();
    edges.sort_by_key(|e| (e.0, e.1));
    let mut bends: Vec<(usize, usize, f64)> = opposite.values().filter(|o| o.len() == 2 && o[0] != o[1]).map(|o| (o[0].min(o[1]), o[0].max(o[1]), norm(sub(start[o[0]], start[o[1]])))).collect();
    bends.sort_by_key(|e| (e.0, e.1));
    let inv: Vec<f64> = cloth.pin.iter().map(|&p| (1.0 - p).clamp(0.0, 1.0)).collect();
    let mut x = start.clone();
    let mut vel = vec![[0.0; 3]; n];
    let dt = 1.0 / CLOTH_HZ;
    let mut stats = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let surface = |t: f64| -> Option<(Vec<V3>, Vec<V3>, Grid)> {
        let body = body?;
        let world = scene.world(&clip.pose(scene, t));
        let jm = joint_matrices(&world, &body.skin);
        let (mut pts, mut nrm) = (Vec::with_capacity(body.bind.len()), Vec::with_capacity(body.bind.len()));
        for v in 0..body.bind.len() {
            let m = blend(&body.weights[v], &jm);
            pts.push(apply(&m, body.bind[v]));
            nrm.push(unit(linear(&m, body.normals[v])));
        }
        let grid = Grid::new(&pts, BODY_CELL_M);
        Some((pts, nrm, grid))
    };
    let mut allow = vec![CLOTH_THICKNESS_M; n];
    let mut at_rest = 0.0f64;
    if let Some((pts, nrm, grid)) = surface(times[0]) {
        for v in 0..n {
            if let Some(j) = grid.nearest(&pts, start[v], BODY_NEAR_M) {
                let s = dot(sub(start[v], pts[j]), nrm[j]);
                allow[v] = s.min(CLOTH_THICKNESS_M);
                if inv[v] > 0.0 {
                    at_rest = at_rest.max(-s);
                }
            }
        }
    }
    let mut step = |t: f64, record: bool, x: &mut Vec<V3>, vel: &mut Vec<V3>| {
        let target = skinned(t);
        let caps_now = placed(t);
        let skin_now = surface(t);
        let old = x.clone();
        for v in 0..n {
            if inv[v] > 0.0 {
                vel[v][1] -= CLOTH_GRAVITY * dt;
                x[v] = add(x[v], scale(vel[v], dt * (1.0 - CLOTH_DAMPING)));
            } else {
                x[v] = target[v];
            }
        }
        for _ in 0..CLOTH_ITERATIONS {
            for &(list, stiffness) in &[(&edges, 1.0), (&bends, CLOTH_BEND)] {
                for &(a, b, rest) in list.iter() {
                    let (wa, wb) = (inv[a], inv[b]);
                    if wa + wb <= 0.0 {
                        continue;
                    }
                    let d = sub(x[b], x[a]);
                    let l = norm(d);
                    if l < 1e-12 {
                        continue;
                    }
                    let c = (l - rest) / l * stiffness / (wa + wb);
                    x[a] = add(x[a], scale(d, wa * c));
                    x[b] = sub(x[b], scale(d, wb * c));
                }
            }
            for v in 0..n {
                if inv[v] <= 0.0 {
                    continue;
                }
                for (a, b, r) in &caps_now {
                    let q = closest_on_segment(x[v], *a, *b);
                    let d = sub(x[v], q);
                    if norm(d) < *r {
                        x[v] = add(q, scale(if norm(d) > 1e-9 { unit(d) } else { [0.0, 0.0, 1.0] }, *r));
                    }
                }
                if BODY_COLLIDE
                    && let Some((pts, nrm, grid)) = &skin_now
                    && let Some(j) = grid.nearest(pts, x[v], BODY_NEAR_M)
                {
                    let s = dot(sub(x[v], pts[j]), nrm[j]);
                    if s < allow[v] {
                        x[v] = add(x[v], scale(nrm[j], allow[v] - s));
                    }
                }
            }
        }
        for v in 0..n {
            x[v] = add(x[v], scale(sub(target[v], x[v]), cloth.pin[v].clamp(0.0, 1.0)));
            vel[v] = scale(sub(x[v], old[v]), 1.0 / dt);
        }
        if record {
            for v in 0..n {
                if cloth.pin[v] >= PINNED_REPORT {
                    stats.0 = stats.0.max(norm(sub(x[v], target[v])));
                }
                for (a, b, r) in &caps_now {
                    if inv[v] > 0.0 {
                        stats.1 = stats.1.max(r - CLOTH_THICKNESS_M - norm(sub(x[v], closest_on_segment(x[v], *a, *b))));
                    }
                }
            }
            for &(a, b, rest) in &edges {
                if rest > 1e-9 {
                    stats.2 = stats.2.max(norm(sub(x[a], x[b])) / rest);
                }
            }
            if let Some((pts, nrm, grid)) = &skin_now {
                for v in 0..n {
                    if inv[v] > 0.0
                        && let Some(j) = grid.nearest(pts, x[v], BODY_NEAR_M)
                    {
                        stats.3 = stats.3.max(allow[v].min(0.0) - dot(sub(x[v], pts[j]), nrm[j]));
                    }
                }
            }
        }
    };
    for _ in 0..(SETTLE_SECONDS * CLOTH_HZ).round() as usize {
        step(times[0], false, &mut x, &mut vel);
    }
    let mut positions = vec![x.clone()];
    for f in 1..times.len() {
        let steps = ((times[f] - times[f - 1]) * CLOTH_HZ).round().max(1.0) as usize;
        for s in 1..=steps {
            step(times[f - 1] + (times[f] - times[f - 1]) * s as f64 / steps as f64, true, &mut x, &mut vel);
        }
        positions.push(x.clone());
    }
    let free = cloth.pin.iter().filter(|&&p| p < 0.5).count();
    let moved = positions.iter().zip(times).map(|(p, &t)| {
        let s = skinned(t);
        p.iter().zip(&s).map(|(a, b)| norm(sub(*a, *b))).fold(0.0, f64::max)
    }).fold(0.0, f64::max);
    let mm = |x: f64| (x * 1e6).round() / 1000.0;
    let report = json!({"node": scene.names[cloth.node], "vertices": n, "free_vertices": free, "edges": edges.len(), "bends": bends.len(),
                        "pinned_vertices": cloth.pin.iter().filter(|&&p| p >= PINNED_REPORT).count(), "pinned_error_mm": mm(stats.0), "max_penetration_mm": mm(stats.1.max(0.0)), "inside_at_rest_mm": if body.is_some() { json!(mm(at_rest.max(0.0))) } else { Value::Null }, "inside_beyond_rest_mm": if body.is_some() { json!(mm(stats.3.max(0.0))) } else { Value::Null }, "max_stretch": (stats.2 * 1000.0).round() / 1000.0,
                        "max_from_skinned_mm": mm(moved)});
    ClothRun { positions, report }
}

#[allow(clippy::too_many_arguments)]
fn write_cloth(doc: &mut Value, bin: &mut Vec<u8>, scene: &Scene, clip: &Clip, cloth: &ClothMesh, run: &ClothRun, times: &[f64], name: &str, input: usize, samplers: &mut Vec<Value>, channels: &mut Vec<Value>) -> Result<(), String> {
    let frames = times.len();
    let existing = doc["meshes"][cloth.mesh]["primitives"][0]["targets"].as_array().map_or(0, Vec::len);
    let mut targets = Vec::with_capacity(frames);
    for (f, x) in run.positions.iter().enumerate() {
        let world = scene.world(&clip.pose(scene, times[f]));
        let jm = joint_matrices(&world, &cloth.skin);
        let sim_normals = face_normals(x, &cloth.triangles);
        let (mut dp, mut dn) = (Vec::with_capacity(3 * x.len()), Vec::new());
        for v in 0..x.len() {
            let back = inverse(&blend(&cloth.weights[v], &jm)).ok_or("a cloth vertex's skin matrix cannot be inverted")?;
            dp.extend(sub(apply(&back, x[v]), cloth.bind[v]).map(|c| c as f32));
            if let Some(nn) = &cloth.normals {
                dn.extend(sub(unit(linear(&back, sim_normals[v])), nn[v]).map(|c| c as f32));
            }
        }
        let mut target = json!({"POSITION": push_floats(doc, bin, &dp, 3, Some(ARRAY_BUFFER), true)});
        if cloth.normals.is_some() {
            target["NORMAL"] = json!(push_floats(doc, bin, &dn, 3, Some(ARRAY_BUFFER), false));
        }
        targets.push(target);
    }
    let m = &mut doc["meshes"][cloth.mesh];
    for p in m["primitives"].as_array_mut().into_iter().flatten() {
        let mut list = p["targets"].as_array().cloned().unwrap_or_default();
        list.extend(targets.iter().cloned());
        p["targets"] = json!(list);
    }
    let mut defaults = m["weights"].as_array().cloned().unwrap_or_else(|| vec![json!(0.0); existing]);
    defaults.extend(std::iter::repeat_n(json!(0.0), frames));
    m["weights"] = json!(defaults);
    let mut names = m["extras"]["targetNames"].as_array().cloned().unwrap_or_default();
    if names.len() == existing {
        names.extend((0..frames).map(|f| json!(format!("{name}_{f:04}"))));
        m["extras"]["targetNames"] = json!(names);
    }
    let total = existing + frames;
    let mut keys = vec![0.0f32; frames * total];
    for f in 0..frames {
        keys[f * total + existing + f] = 1.0;
    }
    let output = push_floats(doc, bin, &keys, 1, None, false);
    channels.push(json!({"sampler": samplers.len(), "target": {"node": cloth.node, "path": "weights"}}));
    samplers.push(json!({"input": input, "output": output, "interpolation": "LINEAR"}));
    Ok(())
}

struct Physics {
    scene: Scene,
    chains: Vec<Chain>,
    caps: Vec<Capsule>,
    spring: Spring,
    top: usize,
    frame: M4,
    unit_scale: f64,
}

impl Physics {
    fn read(doc: &Value, bin: &[u8], ext: &Value) -> Result<Physics, String> {
        let scene = Scene::read(doc)?;
        let skin = read_skin(doc, bin, 0)?;
        let d = &ext["spring_defaults"];
        let spring = Spring {
            stiffness: d["stiffness"].as_f64().unwrap_or(1.0),
            drag: d["drag"].as_f64().unwrap_or(0.4),
            gravity: d["gravity"].as_f64().unwrap_or(0.1),
            hit: d["hit_radius_m"].as_f64().unwrap_or(0.02),
        };
        let node_of = |j: u64| skin.joints.get(j as usize).copied().ok_or(format!("GAZEFORGE_physics names joint {j}, the skin has {}", skin.joints.len()));
        let mut caps = Vec::new();
        for c in ext["colliders"].as_array().into_iter().flatten() {
            let j = c["joint"].as_u64().ok_or("a collider has no joint")?;
            let node = node_of(j)?;
            let (Some(head), Some(tail), Some(radius)) = (v3(&c["head"]), v3(&c["tail"]), c["radius"].as_f64()) else {
                return Err("a collider is missing head, tail or radius".into());
            };
            caps.push(Capsule { node, ibm: skin.ibm[j as usize], head, tail, radius, name: c["bone"].as_str().unwrap_or("").to_string() });
        }
        let mut chains = Vec::new();
        for sp in ext["springs"].as_array().into_iter().flatten() {
            let nodes = sp["joints"].as_array().ok_or("a spring has no joints")?.iter().map(|j| j.as_u64().ok_or("a spring joint is not an index".to_string()).and_then(node_of)).collect::<Result<Vec<_>, _>>()?;
            if nodes.len() < 2 {
                return Err("a spring chain has fewer than two joints".into());
            }
            for k in 1..nodes.len() {
                if scene.parent[nodes[k]] != Some(nodes[k - 1]) {
                    return Err(format!("{} is not the child of {}, so it is not a chain", scene.names[nodes[k]], scene.names[nodes[k - 1]]));
                }
            }
            let parent = scene.parent[nodes[0]].ok_or("a chain's root has no parent bone")?;
            let mut radii = Vec::new();
            for r in sp["collider_radii"].as_array().into_iter().flatten() {
                let (Some(c), Some(radius)) = (r["collider"].as_u64(), r["radius"].as_f64()) else {
                    return Err("a spring's collider_radii entry needs collider and radius".into());
                };
                if c as usize >= caps.len() || radius.is_nan() || radius < 0.0 {
                    return Err(format!("a spring sets collider {c} to radius {radius}; the file has {} colliders", caps.len()));
                }
                radii.push((c as usize, radius));
            }
            let hit = sp["hit_radius_m"].as_f64().unwrap_or(spring.hit);
            chains.push(Chain { nodes, parent, wearable: sp["wearable"].as_str().unwrap_or("").to_string(), radii, hit });
        }
        let in_skin = |n: usize| skin.joints.contains(&n);
        let top = *skin.joints.iter().find(|&&n| !scene.parent[n].is_some_and(in_skin)).ok_or("the skin has no top joint")?;
        let rest_world = scene.world(&scene.rest);
        let frame = scene.parent[top].map_or_else(|| matrix(&Trs { t: [0.0; 3], r: IDENTITY, s: [1.0; 3] }), |p| rest_world[p]);
        let unit_scale = norm([frame[0][0], frame[1][0], frame[2][0]]);
        Ok(Physics { scene, chains, caps, spring, top, frame, unit_scale })
    }

    fn axis_distances(&self, chain: &Chain) -> Vec<f64> {
        let world = self.scene.world(&self.scene.rest);
        self.caps
            .iter()
            .map(|cap| {
                let m = mmul(&world[cap.node], &cap.ibm);
                let (a, b) = (apply(&m, cap.head), apply(&m, cap.tail));
                chain.nodes[1..]
                    .iter()
                    .map(|&n| {
                        let p = origin(&world[n]);
                        norm(sub(p, closest_on_segment(p, a, b))) / self.unit_scale
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .collect()
    }
}

pub fn fit_colliders(glb: &[u8]) -> Result<(Vec<u8>, Value), String> {
    let (doc, bin) = gltf::chunks(glb)?;
    let mut doc = doc;
    let ext = doc["extensions"]["GAZEFORGE_physics"].clone();
    if ext["springs"].as_array().is_none_or(|s| s.is_empty()) {
        return Ok((glb.to_vec(), Value::Null));
    }
    let physics = Physics::read(&doc, bin, &ext)?;
    let mut rows = Vec::new();
    for (k, chain) in physics.chains.iter().enumerate() {
        let axis = physics.axis_distances(chain);
        let hit = axis.iter().map(|a| a - REST_MARGIN_M).fold(physics.spring.hit, f64::min).max(0.0);
        let radii: Vec<(usize, f64)> = physics.caps.iter().enumerate().filter(|(c, cap)| axis[*c] < cap.radius + hit + REST_MARGIN_M).map(|(c, _)| (c, (axis[c] - hit - REST_MARGIN_M).max(0.0))).collect();
        let sp = &mut doc["extensions"]["GAZEFORGE_physics"]["springs"][k];
        sp["hit_radius_m"] = json!(hit as f32);
        sp["collider_radii"] = json!(radii.iter().map(|&(c, r)| json!({"collider": c, "radius": r as f32})).collect::<Vec<_>>());
        let mm = |x: f64| (x * physics.unit_scale * 1e6).round() / 1000.0;
        rows.push(json!({"wearable": chain.wearable, "chain_root": physics.scene.names[chain.nodes[0]], "hit_radius_mm": mm(hit),
                         "shrunk": radii.iter().map(|&(c, r)| json!({"capsule": physics.caps[c].name, "radius_mm": mm(r), "from_mm": mm(physics.caps[c].radius)})).collect::<Vec<_>>()}));
    }
    Ok((gltf::assemble(&doc, bin)?, json!(rows)))
}

pub fn bake(glb: &[u8], s: &Settings) -> Result<(Vec<u8>, Value), String> {
    let (doc, bin) = gltf::chunks(glb)?;
    let mut doc = doc;
    let mut bin = bin.to_vec();
    let ext = doc["extensions"]["GAZEFORGE_physics"].clone();
    if ext.is_null() {
        return Err("--bake needs a character with GAZEFORGE_physics: wear an item with \"physics\": \"springbone\" (and export with --body-rig)".into());
    }
    if ext["springs"].as_array().is_none_or(|s| s.is_empty()) && ext["cloth"].as_array().is_none_or(|c| c.is_empty()) {
        return Err("--bake bakes spring-bone chains and cloth, and this character has neither".into());
    }
    let name = format!("{}_baked", s.clip);
    if doc["animations"].as_array().is_some_and(|a| a.iter().any(|x| x["name"].as_str() == Some(name.as_str()))) {
        return Err(format!("the glb already has an animation named {name}"));
    }
    for key in ["bufferViews", "accessors", "animations"] {
        if doc[key].is_null() {
            doc[key] = json!([]);
        }
    }
    if doc["buffers"].as_array().is_none_or(|b| b.is_empty()) {
        return Err("the glb has no buffer".into());
    }
    let Physics { scene, chains, caps, spring, top, frame, unit_scale } = Physics::read(&doc, &bin, &ext)?;
    let node_named = |name: &str| doc["nodes"].as_array().and_then(|nodes| nodes.iter().position(|n| n["name"] == name));
    let mut cloths = Vec::new();
    for c in ext["cloth"].as_array().into_iter().flatten() {
        let pin = c["pin"].as_str().unwrap_or("_PIN");
        for node in c["nodes"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            let index = node_named(node).ok_or_else(|| format!("GAZEFORGE_physics names cloth node {node}, the file has no such node"))?;
            cloths.push(read_cloth_mesh(&doc, &bin, index, pin)?);
        }
    }
    let mut anchors: Vec<usize> = chains.iter().map(|c| c.parent).collect();
    for cloth in &cloths {
        let mut share: HashMap<usize, f64> = HashMap::new();
        for list in &cloth.weights {
            for &(j, w) in list {
                *share.entry(j).or_default() += w;
            }
        }
        if let Some((j, _)) = share.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))) {
            anchors.push(cloth.skin.joints[j]);
        }
    }
    anchors.sort();
    anchors.dedup();
    let clip = if CLIPS.contains(&s.clip.as_str()) { sway_clip(&scene, &anchors, &frame, top, unit_scale, s.fps) } else { gltf_clip(&doc, &bin, &s.clip)? };
    let baked = simulate(&scene, &clip, &chains, &caps, &spring, s.fps, unit_scale);
    let before = bin.len();
    let times: Vec<f32> = baked.times.iter().map(|&t| t as f32).collect();
    let input = push_floats(&mut doc, &mut bin, &times, 1, None, true);
    let (mut samplers, mut channels) = (Vec::new(), Vec::new());
    let chain_nodes: Vec<usize> = chains.iter().flat_map(|c| c.nodes.iter().copied()).collect();
    clip_channels(&mut doc, &mut bin, &clip, &chain_nodes, input, &mut samplers, &mut channels);
    let mut report = baked.report.clone();
    match s.format {
        Format::Bones => {
            for (chain, segs) in chains.iter().zip(&baked.local) {
                for (k, keys) in segs.iter().enumerate() {
                    let values: Vec<f32> = keys.iter().flat_map(|q| q.map(|x| x as f32)).collect();
                    let output = push_floats(&mut doc, &mut bin, &values, 4, None, false);
                    channels.push(json!({"sampler": samplers.len(), "target": {"node": chain.nodes[k], "path": "rotation"}}));
                    samplers.push(json!({"input": input, "output": output, "interpolation": "LINEAR"}));
                }
            }
        }
        Format::Vertex if !chains.is_empty() => {
            report["meshes"] = write_vertex(&mut doc, &mut bin, &scene, &clip, &baked, &chains, &name, input, &mut samplers, &mut channels)?;
        }
        Format::Vertex => {}
    }
    if !cloths.is_empty() {
        let body = read_body_surface(&doc, &bin, &cloths)?;
        let mut rows = Vec::new();
        for cloth in &cloths {
            let run = simulate_cloth(&scene, &clip, cloth, &caps, body.as_ref(), &baked.times, unit_scale);
            write_cloth(&mut doc, &mut bin, &scene, &clip, cloth, &run, &baked.times, &name, input, &mut samplers, &mut channels)?;
            rows.push(run.report);
        }
        report["cloth"] = json!(rows);
        report["cloth_collides_with"] = json!(body.as_ref().map(|b| b.names.clone()).unwrap_or_default());
        report["cloth_method"] = json!("position-based cloth on each cloth node's own mesh (every level of detail simulated on its own): the mesh's edges at their length in the clip's first pose (hard) and the pairs across each edge (bending, 0.2), gravity 9.81 m/s2, 240 Hz steps of 12 iterations, pushed out of each body capsule by its radius plus 4 mm and kept 4 mm outside the skinned surface of the body, the head and every other opaque item worn (their finest level, within 25 cm) - or, where the cloth's own rest already lies closer or inside, no deeper than it lies at rest - each vertex pulled to its skinned place by its _PIN (1 follows the skin exactly); written as one morph target per frame in either --bake-format, since cloth has no joints");
    }
    let format = if s.format == Format::Bones { "bones" } else { "vertex" };
    doc["animations"].as_array_mut().expect("animations set").push(json!({"name": name, "samplers": samplers, "channels": channels,
        "extras": {"gazeforge": {"baked": {"clip": clip.name, "format": format}}}}));
    let mut baked_list = ext["baked"].as_array().cloned().unwrap_or_default();
    baked_list.push(json!({"animation": name, "clip": clip.name, "format": format, "fps": s.fps, "seconds": clip.duration}));
    doc["extensions"]["GAZEFORGE_physics"]["baked"] = json!(baked_list);
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    report["animation"] = json!(name);
    report["format"] = json!(format);
    report["bytes_added"] = json!(bin.len() - before);
    Ok((gltf::assemble(&doc, &bin)?, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_scene(n: usize, step: V3) -> Scene {
        let mut rest = vec![Trs { t: [0.0; 3], r: IDENTITY, s: [1.0; 3] }];
        let mut parent = vec![None];
        for k in 0..n {
            rest.push(Trs { t: step, r: IDENTITY, s: [1.0; 3] });
            parent.push(Some(k));
        }
        let names = (0..=n).map(|k| format!("j{k}")).collect();
        Scene { order: (0..=n).collect(), parent, rest, names }
    }

    fn still(_: &Scene) -> Clip {
        Clip { name: "still".into(), duration: 1.0, tracks: vec![Track { node: 0, path: PathKind::Translation, times: vec![0.0, 1.0], values: vec![0.0; 6], step: false }], source: None }
    }

    fn sheet(k: usize, size: f64) -> ClothMesh {
        let mut bind = Vec::new();
        for i in 0..=k {
            for j in 0..=k {
                bind.push([j as f64 * size / k as f64, 0.0, i as f64 * size / k as f64]);
            }
        }
        let at = |i: usize, j: usize| i * (k + 1) + j;
        let mut triangles = Vec::new();
        for i in 0..k {
            for j in 0..k {
                triangles.push([at(i, j), at(i + 1, j), at(i, j + 1)]);
                triangles.push([at(i, j + 1), at(i + 1, j), at(i + 1, j + 1)]);
            }
        }
        let pin = (0..bind.len()).map(|v| if v <= k { 1.0 } else { 0.0 }).collect();
        let n = bind.len();
        ClothMesh { node: 0, mesh: 0, skin: Skin { joints: vec![0], ibm: vec![matrix(&Trs { t: [0.0; 3], r: IDENTITY, s: [1.0; 3] })] }, bind, normals: None, weights: vec![vec![(0, 1.0)]; n], pin, triangles }
    }

    #[test]
    fn a_sheet_pinned_on_one_edge_hangs_without_stretching() {
        let scene = line_scene(0, [0.0; 3]);
        let cloth = sheet(6, 0.3);
        let times: Vec<f64> = (0..=10).map(|k| k as f64 / 10.0).collect();
        let run = simulate_cloth(&scene, &still(&scene), &cloth, &[], None, &times, 1.0);
        let last = run.positions.last().unwrap();
        assert!(run.report["pinned_error_mm"].as_f64().unwrap() < 1e-6, "{}", run.report);
        for v in 0..=6 {
            assert!(norm(sub(last[v], cloth.bind[v])) < 1e-9, "pinned vertices stay put");
        }
        let far = last[cloth.bind.len() - 1];
        assert!(far[1] < -0.2, "the free edge hangs down: {far:?}");
        assert!(run.report["max_stretch"].as_f64().unwrap() < 1.1, "{}", run.report);
    }

    #[test]
    fn a_capsule_keeps_the_hanging_sheet_out() {
        let scene = line_scene(0, [0.0; 3]);
        let cloth = sheet(6, 0.3);
        let caps = vec![Capsule { node: 0, ibm: matrix(&Trs { t: [0.0; 3], r: IDENTITY, s: [1.0; 3] }), head: [-0.1, -0.12, 0.01], tail: [0.4, -0.12, 0.01], radius: 0.04, name: "c".into() }];
        let times: Vec<f64> = (0..=10).map(|k| k as f64 / 10.0).collect();
        let free = simulate_cloth(&scene, &still(&scene), &cloth, &[], None, &times, 1.0);
        let held = simulate_cloth(&scene, &still(&scene), &cloth, &caps, None, &times, 1.0);
        let inside = |p: &Vec<V3>| p.iter().map(|x| 0.04 - norm(sub(*x, closest_on_segment(*x, caps[0].head, caps[0].tail)))).fold(f64::NEG_INFINITY, f64::max);
        assert!(inside(free.positions.last().unwrap()) > 0.01, "without the capsule the sheet hangs through it");
        assert!(inside(held.positions.last().unwrap()) < 0.002, "with it the sheet stays outside: {}", inside(held.positions.last().unwrap()));
    }

    #[test]
    fn no_force_keeps_the_rest() {
        let scene = line_scene(4, [0.0, 0.0, 0.1]);
        let chains = vec![Chain { nodes: vec![1, 2, 3, 4], parent: 0, wearable: "t".into(), radii: vec![], hit: 0.02 }];
        let spring = Spring { stiffness: 1.0, drag: 0.4, gravity: 0.0, hit: 0.02 };
        let baked = simulate(&scene, &still(&scene), &chains, &[], &spring, 30.0, 1.0);
        for keys in &baked.local[0] {
            for q in keys {
                assert!(qangle(*q, IDENTITY) < 1e-9, "a chain with no gravity and no motion stays at rest: {q:?}");
            }
        }
    }

    #[test]
    fn gravity_bends_a_level_chain_down_and_keeps_lengths() {
        let scene = line_scene(4, [0.0, 0.0, 0.1]);
        let chains = vec![Chain { nodes: vec![1, 2, 3, 4], parent: 0, wearable: "t".into(), radii: vec![], hit: 0.02 }];
        let spring = Spring { stiffness: 0.2, drag: 0.4, gravity: 1.0, hit: 0.02 };
        let baked = simulate(&scene, &still(&scene), &chains, &[], &spring, 30.0, 1.0);
        let mut pose = scene.rest.clone();
        for (k, keys) in baked.local[0].iter().enumerate() {
            pose[chains[0].nodes[k]].r = *keys.last().unwrap();
        }
        let w = scene.world(&pose);
        let tip = origin(&w[4]);
        assert!(tip[1] < -0.05, "the tip hangs below its root: {tip:?}");
        for k in 1..4 {
            let seg = norm(sub(origin(&w[k + 1]), origin(&w[k])));
            assert!((seg - 0.1).abs() < 1e-9, "segment {k} keeps its length: {seg}");
        }
        assert!(baked.report["max_length_error_mm"].as_f64().unwrap() < 1e-3);
    }

    #[test]
    fn a_capsule_pushes_the_chain_out() {
        let scene = line_scene(3, [0.0, -0.1, 0.0]);
        let chains = vec![Chain { nodes: vec![1, 2, 3], parent: 0, wearable: "t".into(), radii: vec![], hit: 0.01 }];
        let caps = vec![Capsule { node: 0, ibm: matrix(&Trs { t: [0.0; 3], r: IDENTITY, s: [1.0; 3] }), head: [0.0, -0.15, -0.05], tail: [0.0, -0.15, 0.05], radius: 0.03, name: "c".into() }];
        let spring = Spring { stiffness: 1.0, drag: 0.4, gravity: 1.0, hit: 0.01 };
        let baked = simulate(&scene, &still(&scene), &chains, &caps, &spring, 30.0, 1.0);
        let mut pose = scene.rest.clone();
        for (k, keys) in baked.local[0].iter().enumerate() {
            pose[chains[0].nodes[k]].r = *keys.last().unwrap();
        }
        let w = scene.world(&pose);
        let tail = origin(&w[3]);
        let gap = norm(sub(tail, closest_on_segment(tail, caps[0].head, caps[0].tail)));
        assert!(gap >= 0.04 - 1e-9, "the tip sits at least radius + hit from the capsule: {gap}");
        assert!(baked.report["max_penetration_mm"].as_f64().unwrap() < 1e-3);
    }

    #[test]
    fn a_capsule_shrunk_to_the_rest_depth_keeps_the_rest_and_still_holds() {
        let mut scene = line_scene(3, [0.0, -0.05, 0.0]);
        scene.rest.push(Trs { t: [0.0; 3], r: IDENTITY, s: [1.0; 3] });
        scene.parent.push(None);
        scene.order.push(4);
        scene.names.push("still".into());
        let cap = Capsule { node: 4, ibm: matrix(&Trs { t: [0.0; 3], r: IDENTITY, s: [1.0; 3] }), head: [0.03, -0.2, -0.05], tail: [0.03, -0.2, 0.05], radius: 0.07, name: "c".into() };
        let spring = Spring { stiffness: 1.0, drag: 0.4, gravity: 0.0, hit: 0.01 };
        let whole = vec![Chain { nodes: vec![1, 2, 3], parent: 0, wearable: "t".into(), radii: vec![], hit: 0.01 }];
        let thrown = simulate(&scene, &still(&scene), &whole, std::slice::from_ref(&cap), &spring, 30.0, 1.0);
        assert!(thrown.report["chains"][0]["tip_settled_mm"].as_f64().unwrap() > 5.0, "a chain starting inside the capsule is thrown out of it");
        let axis = norm(sub([0.0, -0.15, 0.0], [0.03, -0.2, 0.0]));
        let fitted = vec![Chain { nodes: vec![1, 2, 3], parent: 0, wearable: "t".into(), radii: vec![(0, axis - 0.01)], hit: 0.01 }];
        let kept = simulate(&scene, &still(&scene), &fitted, std::slice::from_ref(&cap), &spring, 30.0, 1.0);
        assert!(kept.report["chains"][0]["tip_settled_mm"].as_f64().unwrap() < 1e-6, "shrunk to its rest depth, the capsule leaves the rest alone");
        let times: Vec<f64> = (0..=30).map(|k| k as f64 / 30.0).collect();
        let values = times.iter().flat_map(|&t| { let s = (t * std::f64::consts::PI).sin(); [0.02 * s, -0.03 * s, 0.0] }).collect();
        let push = Clip { name: "push".into(), duration: 1.0, tracks: vec![Track { node: 0, path: PathKind::Translation, times, values, step: false }], source: None };
        let depth = |baked: &Baked| -> f64 {
            let mut worst = 0.0f64;
            for f in 0..baked.times.len() {
                let mut pose = push.pose(&scene, baked.times[f]);
                for (k, keys) in baked.local[0].iter().enumerate() {
                    pose[fitted[0].nodes[k]].r = keys[f];
                }
                let w = scene.world(&pose);
                for &n in &fitted[0].nodes[1..] {
                    let p = origin(&w[n]);
                    let (a, b) = (apply(&w[4], cap.head), apply(&w[4], cap.tail));
                    worst = worst.max(axis - norm(sub(p, closest_on_segment(p, a, b))));
                }
            }
            worst
        };
        let held = simulate(&scene, &push, &fitted, std::slice::from_ref(&cap), &spring, 30.0, 1.0);
        let free = simulate(&scene, &push, &whole, &[], &spring, 30.0, 1.0);
        let (held, free) = (depth(&held), depth(&free));
        assert!(free > 0.01, "the push drives a free chain into the capsule: {free}");
        assert!(held < 0.25 * free, "the shrunk capsule holds the chain: {held} deep against {free} free");
    }

    #[test]
    fn a_turning_parent_drags_the_chain_and_it_lags() {
        let scene = line_scene(4, [0.0, -0.1, 0.0]);
        let chains = vec![Chain { nodes: vec![1, 2, 3, 4], parent: 0, wearable: "t".into(), radii: vec![], hit: 0.02 }];
        let times: Vec<f64> = (0..=30).map(|k| k as f64 / 30.0).collect();
        let values = times.iter().flat_map(|&t| axis_angle([0.0, 0.0, 1.0], 0.8 * (t * std::f64::consts::PI).sin())).collect();
        let clip = Clip { name: "turn".into(), duration: 1.0, tracks: vec![Track { node: 0, path: PathKind::Rotation, times, values, step: false }], source: None };
        let spring = Spring { stiffness: 1.0, drag: 0.4, gravity: 1.0, hit: 0.02 };
        let baked = simulate(&scene, &clip, &chains, &[], &spring, 30.0, 1.0);
        let lag = baked.local[0][0].iter().map(|q| qangle(*q, IDENTITY)).fold(0.0, f64::max);
        assert!(lag > 0.05, "the root segment swings against its parent's turn: {lag}");
    }

    #[test]
    fn tracks_interpolate_like_gltf() {
        let t = Track { node: 0, path: PathKind::Translation, times: vec![0.0, 1.0, 3.0], values: vec![0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 2.0, 1.0], step: false };
        assert_eq!(t.at(0.5), vec![0.5, 1.0, 1.5]);
        assert_eq!(t.at(2.0), vec![2.0, 2.0, 2.0]);
        assert_eq!(t.at(9.0), vec![3.0, 2.0, 1.0]);
        let s = Track { step: true, ..t };
        assert_eq!(s.at(0.99), vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn settings_read_and_refuse() {
        let o = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Options>();
        assert!(settings(&o(&[])).unwrap().is_none());
        assert!(settings(&o(&[("bake-format", "bones")])).is_err());
        let s = settings(&o(&[("bake", "sway"), ("bake-format", "vertex"), ("bake-fps", "15")])).unwrap().unwrap();
        assert_eq!((s.clip.as_str(), s.format, s.fps), ("sway", Format::Vertex, 15.0));
        assert!(settings(&o(&[("bake", "sway"), ("bake-format", "mesh")])).is_err());
        assert!(settings(&o(&[("bake", "sway"), ("bake-fps", "0")])).is_err());
        assert!(settings(&o(&[("bake", "sway"), ("bake-fps", "120")])).is_err());
    }

    #[test]
    fn inverse_and_rotation_round_trip() {
        let p = Trs { t: [0.3, -1.0, 2.0], r: qnorm([0.2, -0.4, 0.1, 0.9]), s: [1.5; 3] };
        let m = matrix(&p);
        let i = inverse(&m).unwrap();
        let x = apply(&i, apply(&m, [0.7, 0.1, -0.2]));
        assert!(norm(sub(x, [0.7, 0.1, -0.2])) < 1e-12);
        assert!(qangle(rotation_of(&m), p.r) < 1e-9);
        assert!(qangle(from_to([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), axis_angle([0.0, 0.0, 1.0], std::f64::consts::FRAC_PI_2)) < 1e-9);
    }
}
