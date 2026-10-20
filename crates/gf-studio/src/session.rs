use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use gf_api::designer;
use gf_api::{ApiError, Ctx};
use gf_core::eyerig::{self as er, EyeballSpec};
use gf_core::fit::Similarity;
use gf_geom::lids::Eye;
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Lod {
    pub label: String,
    pub indices: Vec<u32>,
    pub uvs: Vec<f32>,
    pub flags: Vec<u32>,
    pub verts: Vec<u32>,
    pub boundary: usize,
    pub seams: usize,
    pub dropped: usize,
}

impl Lod {
    pub fn new(label: String, indices: Vec<u32>, uvs: Vec<f32>, dropped: usize) -> Lod {
        let (flags, boundary, seams) = edge_flags(&indices, &uvs);
        let top = indices.iter().copied().max().unwrap_or(0) as usize;
        let mut seen = vec![false; top + 1];
        let mut verts = Vec::new();
        for &i in &indices {
            if !seen[i as usize] {
                seen[i as usize] = true;
                verts.push(i);
            }
        }
        Lod { label, indices, uvs, flags, verts, boundary, seams, dropped }
    }

    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn set_uvs(&mut self, uvs: Vec<f32>) {
        let (flags, boundary, seams) = edge_flags(&self.indices, &uvs);
        self.uvs = uvs;
        self.flags = flags;
        self.boundary = boundary;
        self.seams = seams;
    }
}

pub struct EdgePairs {
    tri: usize,
    corners: usize,
    pairs: Vec<[u32; 6]>,
    boundary: Vec<u32>,
}

impl EdgePairs {
    pub fn new(idx: &[u32]) -> EdgePairs {
        let tri = idx.len() / 3;
        let mut seen: HashMap<u64, i64> = HashMap::with_capacity(idx.len());
        let corner_of = |t: usize, v: u32| -> u32 { (0..3).find(|&k| idx[t * 3 + k] == v).map_or(u32::MAX, |k| (t * 3 + k) as u32) };
        let (mut pairs, mut order) = (Vec::new(), Vec::new());
        for t in 0..tri {
            for k in 0..3 {
                let (a, b) = (idx[t * 3 + (k + 1) % 3], idx[t * 3 + (k + 2) % 3]);
                let key = (a.min(b) as u64) * 4194304 + a.max(b) as u64;
                let Some(&first) = seen.get(&key) else {
                    seen.insert(key, (t * 3 + k) as i64);
                    order.push(key);
                    continue;
                };
                if first < 0 {
                    continue;
                }
                seen.insert(key, -1);
                let t1 = (first / 3) as usize;
                pairs.push([first as u32, (t * 3 + k) as u32, corner_of(t1, a), corner_of(t, a), corner_of(t1, b), corner_of(t, b)]);
            }
        }
        let boundary = order.into_iter().filter_map(|key| u32::try_from(seen[&key]).ok()).collect();
        EdgePairs { tri, corners: idx.len(), pairs, boundary }
    }

    pub fn flags(&self, uvs: &[f32]) -> (Vec<u32>, usize, usize) {
        let mut flags = vec![0u32; self.tri];
        if uvs.len() >= self.corners * 2 {
            let uv = |c: u32| -> [f32; 2] { if c == u32::MAX { [0.0, 0.0] } else { [uvs[c as usize * 2], uvs[c as usize * 2 + 1]] } };
            let apart = |p: [f32; 2], q: [f32; 2]| (p[0] as f64 - q[0] as f64).abs() > 1e-5 || (p[1] as f64 - q[1] as f64).abs() > 1e-5;
            for &[c1, c2, pa, qa, pb, qb] in &self.pairs {
                if apart(uv(pa), uv(qa)) || apart(uv(pb), uv(qb)) {
                    flags[c1 as usize / 3] |= 8 << (c1 % 3);
                    flags[c2 as usize / 3] |= 8 << (c2 % 3);
                }
            }
        }
        for &c in &self.boundary {
            flags[c as usize / 3] |= 1 << (c % 3);
        }
        let seams: usize = flags.iter().map(|f| (0..3).filter(|k| f & (8 << k) != 0).count()).sum();
        (flags, self.boundary.len(), seams / 2)
    }
}

pub fn edge_flags(idx: &[u32], uvs: &[f32]) -> (Vec<u32>, usize, usize) {
    EdgePairs::new(idx).flags(uvs)
}

pub fn vertex_normals(pos: &[f32], idx: &[u32], n: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; n * 3];
    for t in idx.chunks_exact(3) {
        let (a, b, c) = (t[0] as usize * 3, t[1] as usize * 3, t[2] as usize * 3);
        let p = |i: usize| pos[i] as f64;
        let (ux, uy, uz) = (p(b) - p(a), p(b + 1) - p(a + 1), p(b + 2) - p(a + 2));
        let (vx, vy, vz) = (p(c) - p(a), p(c + 1) - p(a + 1), p(c + 2) - p(a + 2));
        let (nx, ny, nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
        for v in [a, b, c] {
            out[v] = (out[v] as f64 + nx) as f32;
            out[v + 1] = (out[v + 1] as f64 + ny) as f32;
            out[v + 2] = (out[v + 2] as f64 + nz) as f32;
        }
    }
    for v in out.chunks_exact_mut(3) {
        let l = ((v[0] as f64).powi(2) + (v[1] as f64).powi(2) + (v[2] as f64).powi(2)).sqrt();
        let l = if l == 0.0 { 1.0 } else { l };
        for x in v.iter_mut() {
            *x = (*x as f64 / l) as f32;
        }
    }
    out
}

pub fn winding_sign(pos: &[f32], idx: &[u32]) -> f32 {
    let mut vol = 0.0f64;
    for t in idx.chunks_exact(3) {
        let g = |i: u32| [pos[i as usize * 3] as f64, pos[i as usize * 3 + 1] as f64, pos[i as usize * 3 + 2] as f64];
        let (p, q, r) = (g(t[0]), g(t[1]), g(t[2]));
        vol += p[0] * (q[1] * r[2] - q[2] * r[1]) + p[1] * (q[2] * r[0] - q[0] * r[2]) + p[2] * (q[0] * r[1] - q[1] * r[0]);
    }
    if vol < 0.0 { -1.0 } else { 1.0 }
}

#[derive(Clone, Debug)]
pub struct Character {
    pub name: String,
    pub placement: String,
    pub weights: BTreeMap<String, f64>,
    pub macros: Value,
    pub residual: Value,
    pub eyeballs: Value,
    pub skin: Value,
}

pub struct SettleOut {
    pub positions: Vec<f32>,
    pub uvs: Option<Vec<f32>>,
    pub flags: Option<(Vec<u32>, usize, usize)>,
    pub revision: String,
    pub placement: String,
    pub approximate: bool,
    pub ms: f64,
    pub server_ms: Value,
    pub eyes: Vec<(usize, Similarity)>,
    pub eyeballs: Option<EyeballSpec>,
}

fn placed_eye(v: &Value) -> Option<(usize, Similarity)> {
    let side = match v["side"].as_str()? {
        "left" => 0,
        "right" => 1,
        _ => return None,
    };
    let row = |i: usize| -> Option<[f64; 3]> { Some([v["rotation"][i][0].as_f64()?, v["rotation"][i][1].as_f64()?, v["rotation"][i][2].as_f64()?]) };
    let t = &v["translation"];
    Some((side, Similarity { scale: v["scale"].as_f64()?, rotation: [row(0)?, row(1)?, row(2)?], translation: [t[0].as_f64()?, t[1].as_f64()?, t[2].as_f64()?] }))
}

pub fn run_settle(input: &Value) -> Result<SettleOut, String> {
    let t = Instant::now();
    let s = designer::settle_mesh(input).map_err(err)?;
    let eyes = s.eyes.as_ref().map(|e| e.iter().filter_map(placed_eye).collect()).unwrap_or_default();
    let eyeballs = serde_json::from_value::<EyeballSpec>(input["eyeballs"].clone()).ok();
    let server_ms = json!({"load": s.seconds.load * 1000.0, "evaluate": (s.seconds.evaluated - s.seconds.placer) * 1000.0,
                           "prepare": (s.seconds.prepared - s.seconds.evaluated) * 1000.0, "place": (s.seconds.placed - s.seconds.prepared) * 1000.0,
                           "prepared_cached": s.prepared_cached, "cached": s.cached});
    Ok(SettleOut {
        positions: s.positions.iter().flatten().map(|&x| x as f32).collect(),
        uvs: s.triangle_uvs.as_ref().map(|u| u.iter().flatten().flatten().copied().collect()),
        flags: None,
        revision: s.revision,
        placement: s.placement,
        approximate: s.approximate,
        ms: ms(t),
        server_ms,
        eyes,
        eyeballs,
    })
}

pub struct SettleInfo {
    pub revision: String,
    pub placement: String,
    pub approximate: bool,
    pub ms: f64,
    pub server_ms: Value,
}

pub struct Head {
    pub n: usize,
    pub lods: Vec<Lod>,
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub wind: f32,
    pub reference: Option<Vec<f32>>,
    pub revision: u64,
    pub eyes: Option<EyeMesh>,
}

pub struct EyeMesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
    pub uvs: Vec<f32>,
    pub parts: Vec<EyePart>,
    pub model: Option<String>,
    pub key: u64,
}

pub struct EyePart {
    pub first: usize,
    pub count: usize,
    pub name: String,
}

pub struct EyeBase {
    pub side: usize,
    pub centre: [f64; 3],
    pub gaze: [f64; 3],
    pub first: usize,
    pub positions: Vec<[f64; 3]>,
    pub normals: Vec<[f64; 3]>,
}

pub fn eye_meshes(eyes: &[Eye]) -> (EyeMesh, Vec<EyeBase>) {
    let (mut positions, mut normals, mut indices, mut uvs, mut parts) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut bases = Vec::new();
    for e in eyes {
        let first = positions.len() / 3;
        let first_triangle = indices.len() / 3;
        if e.uvs.len() == e.vertices.len() {
            uvs.extend(e.uvs.iter().flatten().map(|&x| x as f32));
        } else {
            uvs.extend(std::iter::repeat_n(0.0f32, e.vertices.len() * 2));
        }
        parts.extend(e.parts.iter().map(|p| EyePart { first: first_triangle + p.first_triangle, count: p.triangles, name: p.name.clone() }));
        let mut n = vec![[0.0f64; 3]; e.vertices.len()];
        for t in &e.triangles {
            let [a, b, c] = t.map(|i| e.vertices[i as usize]);
            let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let f = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            for &i in t {
                for k in 0..3 {
                    n[i as usize][k] += f[k];
                }
            }
        }
        for (p, m) in e.vertices.iter().zip(n.iter_mut()) {
            let out = [p[0] - e.centre[0], p[1] - e.centre[1], p[2] - e.centre[2]];
            let l = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
            let s = if e.parts.is_empty() && m[0] * out[0] + m[1] * out[1] + m[2] * out[2] < 0.0 { -1.0 } else { 1.0 };
            *m = if l > 0.0 { m.map(|x| s * x / l) } else { out };
        }
        positions.extend(e.vertices.iter().flatten().map(|&x| x as f32));
        normals.extend(n.iter().flatten().map(|&x| x as f32));
        indices.extend(e.triangles.iter().flatten().map(|&i| first as u32 + i));
        bases.push(EyeBase { side: usize::from(e.centre[0] < 0.0), centre: e.centre, gaze: e.gaze(), first, positions: e.vertices.clone(), normals: n });
    }
    (EyeMesh { positions, normals, indices, uvs, parts, model: None, key: 0 }, bases)
}

fn invert(t: &Similarity) -> Similarity {
    let r = t.rotation;
    let rt = [[r[0][0], r[1][0], r[2][0]], [r[0][1], r[1][1], r[2][1]], [r[0][2], r[1][2], r[2][2]]];
    let inv = Similarity { scale: 1.0 / t.scale, rotation: rt, translation: [0.0; 3] };
    let back = inv.apply(t.translation);
    Similarity { translation: back.map(|x| -x), ..inv }
}

pub fn eye_display(base: &EyeBase, settled: Option<&Similarity>, settled_spec: Option<&EyeballSpec>, live: Option<&EyeballSpec>) -> Similarity {
    let frame = er::eye_frame(base.centre, base.gaze);
    let adjust = |s: Option<&EyeballSpec>| er::adjustment(&frame, &s.map(|s| if base.side == 0 { s.left } else { s.right }).unwrap_or_default());
    let follows = |s: Option<&EyeballSpec>| s.is_some_and(EyeballSpec::follows);
    match settled {
        Some(t) if follows(live) && follows(settled_spec) => adjust(live).then(&invert(&adjust(settled_spec))).then(t),
        _ => adjust(live),
    }
}

pub fn place_eye_mesh(mesh: &mut EyeMesh, base: &EyeBase, t: &Similarity) {
    for (k, (p, n)) in base.positions.iter().zip(&base.normals).enumerate() {
        let (q, m) = (t.apply(*p), t.rotate(*n));
        for c in 0..3 {
            mesh.positions[(base.first + k) * 3 + c] = q[c] as f32;
            mesh.normals[(base.first + k) * 3 + c] = m[c] as f32;
        }
    }
}

pub struct Session {
    pub store: String,
    pub pack: String,
    pub catalog: String,
    pub game: String,
    pub macro_file: Option<String>,
    pub eyes_file: Option<String>,
    pub character: Character,
    pub start: Character,
    pub head: Head,
    pub settled: Option<SettleInfo>,
    pub timings: BTreeMap<String, f64>,
    pub targets: usize,
    pub eye_bases: Vec<EyeBase>,
    pub settled_eyes: Vec<(usize, Similarity)>,
    pub settled_eyeballs: Option<EyeballSpec>,
    pub eye_key: String,
    pub eye_error: Option<String>,
    eye_generation: u64,
}

fn eye_key(eyeballs: Option<&EyeballSpec>, eyes_file: Option<&str>) -> String {
    match eyeballs.and_then(|e| e.model.as_ref().map(|m| (e, m))) {
        Some((e, m)) => format!("model|{}|{}|{}", e.rig.as_deref().unwrap_or_default(), m.id, m.radius_mm),
        None => format!("file|{}", eyes_file.unwrap_or_default()),
    }
}

fn err(e: ApiError) -> String {
    serde_json::to_string(&e.to_json()).unwrap_or_else(|_| "gf-api error".into())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

impl Session {
    pub fn open(store: &str, spec: Option<&str>, character: Option<&Value>, macro_file: Option<&str>, eyes_file: Option<&str>, game: Option<&str>) -> Result<(Session, Value), String> {
        let ctx = Ctx::from_env();
        let (eyes, eye_bases) = match eyes_file {
            Some(p) => {
                let (mesh, bases) = eye_meshes(&gf_io::load_eyes(std::path::Path::new(p))?);
                (Some(mesh), bases)
            }
            None => (None, Vec::new()),
        };
        let mut timings = BTreeMap::new();
        let t = Instant::now();
        let mut input = json!({"store": store});
        if let Some(s) = spec {
            input["spec"] = json!(s);
        }
        if let Some(c) = character {
            input["character"] = c.clone();
        }
        if let Some(m) = macro_file {
            input["macro"] = json!(m);
        }
        if let Some(g) = game {
            input["game"] = json!(g);
        }
        let sl = designer::sliders(&ctx, &input).map_err(err)?;
        timings.insert("sliders_ms".into(), ms(t));
        let text = |k: &str| sl[k].as_str().unwrap_or_default().to_string();
        let (pack, catalog, game) = (text("pack"), text("catalog"), text("game"));
        let st = &sl["start"];
        let start = Character {
            name: st["name"].as_str().unwrap_or("character").to_string(),
            placement: st["placement"].as_str().unwrap_or_default().to_string(),
            weights: st["weights"].as_object().map(|m| m.iter().filter_map(|(k, v)| v.as_f64().map(|x| (k.clone(), x))).collect()).unwrap_or_default(),
            macros: st["macros"].clone(),
            residual: st["residual"].clone(),
            eyeballs: st["eyeballs"].clone(),
            skin: st["skin"].clone(),
        };
        let t = Instant::now();
        let scene = designer::scene_mesh(&json!({"store": store, "pack": pack, "catalog": catalog, "game": game})).map_err(err)?;
        timings.insert("scene_ms".into(), ms(t));
        let t = Instant::now();
        let base_idx: Vec<u32> = scene.triangles.iter().flatten().copied().collect();
        let base_uvs: Vec<f32> = scene.triangle_uvs.iter().flatten().flatten().copied().collect();
        let mut lods = vec![Lod::new("LOD0".into(), base_idx, base_uvs, 0)];
        for l in &scene.lods {
            lods.push(Lod::new(format!("LOD {}", l.target), l.indices.clone(), l.triangle_uvs.iter().map(|&x| x as f32).collect(), l.dropped_triangles));
        }
        timings.insert("lods_ms".into(), ms(t));
        let n = scene.vertices;
        let head = Head { n, lods, positions: vec![0.0; n * 3], normals: vec![0.0; n * 3], wind: 1.0, reference: None, revision: 0, eyes };
        let targets = sl["targets"].as_array().map_or(0, |a| a.len());
        let mut s = Session {
            store: store.to_string(),
            pack,
            catalog,
            game,
            macro_file: macro_file.map(str::to_string),
            eyes_file: eyes_file.map(str::to_string),
            character: start.clone(),
            start,
            head,
            settled: None,
            timings,
            targets,
            eye_bases,
            settled_eyes: Vec::new(),
            settled_eyeballs: None,
            eye_key: eye_key(None, eyes_file),
            eye_error: None,
            eye_generation: 0,
        };
        s.settle()?;
        Ok((s, sl))
    }

    pub fn settle_input(&self, c: &Character, uvs: bool) -> Value {
        let mut input = json!({"store": self.store, "pack": self.pack, "catalog": self.catalog, "game": self.game, "placement": c.placement,
                               "name": c.name, "weights": c.weights, "residual": c.residual});
        if let Some(e) = &self.eyes_file {
            input["eyes"] = json!(e);
        }
        if !c.skin.is_null() {
            input["skin"] = c.skin.clone();
        }
        if !c.eyeballs.is_null() {
            input["eyeballs"] = c.eyeballs.clone();
        }
        if !c.macros.is_null() {
            input["macros"] = c.macros.clone();
            if let Some(m) = &self.macro_file {
                input["macro"] = json!(m);
            }
        }
        if uvs {
            input["uvs"] = json!(true);
        }
        input
    }

    pub fn settle(&mut self) -> Result<&SettleInfo, String> {
        let out = run_settle(&self.settle_input(&self.character, true))?;
        self.apply_settle(&out)?;
        let t = Instant::now();
        let h = &mut self.head;
        h.normals = vertex_normals(&h.positions, &h.lods[0].indices, h.n);
        h.revision += 1;
        self.timings.insert("normals_ms".into(), ms(t));
        Ok(self.settled.as_ref().expect("set by apply_settle"))
    }

    pub fn apply_settle(&mut self, s: &SettleOut) -> Result<(), String> {
        if s.positions.len() != self.head.n * 3 {
            return Err(format!("settle returned {} vertices, the game base has {}", s.positions.len() / 3, self.head.n));
        }
        let h = &mut self.head;
        h.positions.copy_from_slice(&s.positions);
        if let Some(uvs) = s.uvs.as_ref().filter(|u| **u != h.lods[0].uvs) {
            match &s.flags {
                Some((flags, boundary, seams)) => {
                    let l = &mut h.lods[0];
                    l.uvs = uvs.clone();
                    l.flags = flags.clone();
                    l.boundary = *boundary;
                    l.seams = *seams;
                }
                None => h.lods[0].set_uvs(uvs.clone()),
            }
        }
        if h.revision == 0 {
            h.wind = winding_sign(&h.positions, &h.lods[0].indices);
        }
        self.settled = Some(SettleInfo { revision: s.revision.clone(), placement: s.placement.clone(), approximate: s.approximate, ms: s.ms, server_ms: s.server_ms.clone() });
        self.settled_eyes = s.eyes.clone();
        self.settled_eyeballs = s.eyeballs.clone();
        let live = self.settled_eyeballs.clone();
        self.preview_eyes(live.as_ref());
        Ok(())
    }

    pub fn sync_eye_model(&mut self, live: Option<&EyeballSpec>) -> Result<bool, String> {
        let key = eye_key(live, self.eyes_file.as_deref());
        if key == self.eye_key {
            return Ok(false);
        }
        let model = live.and_then(|e| e.model.as_ref()).map(|m| m.id.clone());
        let eyes = match live.filter(|e| e.model.is_some()) {
            Some(eb) => {
                let store = gf_io::store::Store::open(std::path::Path::new(&self.store))?;
                gf_io::character::model_eyes_for(&store, &self.catalog, eb)?.unwrap_or_default()
            }
            None => match &self.eyes_file {
                Some(p) => gf_io::load_eyes(std::path::Path::new(p))?,
                None => Vec::new(),
            },
        };
        let (mut mesh, bases) = eye_meshes(&eyes);
        self.eye_generation += 1;
        mesh.key = self.eye_generation;
        mesh.model = model;
        self.head.eyes = (!eyes.is_empty()).then_some(mesh);
        self.eye_bases = bases;
        self.eye_key = key;
        Ok(true)
    }

    pub fn preview_eyes(&mut self, live: Option<&EyeballSpec>) {
        match self.sync_eye_model(live) {
            Ok(_) => self.eye_error = None,
            Err(e) => self.eye_error = Some(e),
        }
        let Some(mesh) = self.head.eyes.as_mut() else { return };
        for base in &self.eye_bases {
            let settled = self.settled_eyes.iter().find(|(s, _)| *s == base.side).map(|(_, t)| t);
            let t = eye_display(base, settled, self.settled_eyeballs.as_ref(), live);
            place_eye_mesh(mesh, base, &t);
        }
    }

    pub fn reference_input(&self) -> Value {
        let neutral = Character { name: "neutral".into(), weights: BTreeMap::new(), macros: Value::Null, residual: Value::Null, eyeballs: Value::Null, skin: Value::Null, ..self.start.clone() };
        let mut input = self.settle_input(&neutral, false);
        input.as_object_mut().map(|o| o.remove("residual"));
        input
    }

    pub fn summary(&self) -> Value {
        let lods: Vec<Value> = self
            .head
            .lods
            .iter()
            .map(|l| json!({"label": l.label, "triangles": l.triangles(), "vertices": l.verts.len(), "boundary_edges": l.boundary, "seam_edges": l.seams, "dropped_triangles": l.dropped}))
            .collect();
        json!({"store": self.store, "pack": self.pack, "catalog": self.catalog, "game": self.game, "vertices": self.head.n, "lods": lods,
               "head_targets": self.targets, "character": {"name": self.character.name, "placement": self.character.placement, "weights": self.character.weights.len()},
               "settled": self.settled.as_ref().map(|s| json!({"revision": s.revision, "placement": s.placement, "approximate": s.approximate, "ms": s.ms, "server_ms": s.server_ms})),
               "timings_ms": self.timings})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge_flags_reference(idx: &[u32], uvs: &[f32]) -> (Vec<u32>, usize, usize) {
        let tri = idx.len() / 3;
        let mut flags = vec![0u32; tri];
        let mut seen: HashMap<u64, i64> = HashMap::with_capacity(idx.len());
        let has_uv = uvs.len() >= idx.len() * 2;
        let uv_at = |t: usize, v: u32| -> [f32; 2] {
            for k in 0..3 {
                if idx[t * 3 + k] == v {
                    return [uvs[(t * 3 + k) * 2], uvs[(t * 3 + k) * 2 + 1]];
                }
            }
            [0.0, 0.0]
        };
        let mut order: Vec<u64> = Vec::new();
        for t in 0..tri {
            for k in 0..3 {
                let (a, b) = (idx[t * 3 + (k + 1) % 3], idx[t * 3 + (k + 2) % 3]);
                let key = (a.min(b) as u64) * 4194304 + a.max(b) as u64;
                let Some(&first) = seen.get(&key) else {
                    seen.insert(key, (t * 3 + k) as i64);
                    order.push(key);
                    continue;
                };
                if first < 0 {
                    continue;
                }
                seen.insert(key, -1);
                if has_uv {
                    let (t1, k1) = ((first / 3) as usize, (first % 3) as u32);
                    let mut seam = false;
                    for v in [a, b] {
                        let (p, q) = (uv_at(t1, v), uv_at(t, v));
                        if (p[0] as f64 - q[0] as f64).abs() > 1e-5 || (p[1] as f64 - q[1] as f64).abs() > 1e-5 {
                            seam = true;
                        }
                    }
                    if seam {
                        flags[t1] |= 8 << k1;
                        flags[t] |= 8 << k;
                    }
                }
            }
        }
        let mut boundary = 0;
        for key in order {
            let c = seen[&key];
            if c >= 0 {
                flags[(c / 3) as usize] |= 1 << (c % 3);
                boundary += 1;
            }
        }
        let mut seams = 0;
        for f in &flags {
            for k in 0..3 {
                if f & (8 << k) != 0 {
                    seams += 1;
                }
            }
        }
        (flags, boundary, seams / 2)
    }

    fn eye_file(centres: &[[f64; 3]]) -> Vec<Eye> {
        let (rings, segments, r) = (12usize, 24usize, 0.012);
        let eyes: Vec<Value> = centres
            .iter()
            .map(|c| {
                let mut v = vec![[c[0], c[1], c[2] + r]];
                for i in 1..rings {
                    let p = std::f64::consts::PI * i as f64 / rings as f64;
                    for j in 0..segments {
                        let a = std::f64::consts::TAU * j as f64 / segments as f64;
                        v.push([c[0] + r * p.sin() * a.cos(), c[1] + r * p.sin() * a.sin(), c[2] + r * p.cos()]);
                    }
                }
                v.push([c[0], c[1], c[2] - r]);
                let last = v.len() - 1;
                let ring = |i: usize, j: usize| 1 + (i - 1) * segments + j % segments;
                let mut t = Vec::new();
                for j in 0..segments {
                    t.push([0, ring(1, j), ring(1, j + 1)]);
                    t.push([last, ring(rings - 1, j + 1), ring(rings - 1, j)]);
                }
                for i in 1..rings - 1 {
                    for j in 0..segments {
                        t.push([ring(i, j), ring(i + 1, j), ring(i + 1, j + 1)]);
                        t.push([ring(i, j), ring(i + 1, j + 1), ring(i, j + 1)]);
                    }
                }
                json!({"centre": c, "radius": r, "gaze": [0.0, -1.0, 0.0], "vertices": v, "triangles": t})
            })
            .collect();
        gf_geom::lids::eyes_from_json(&json!({"eyes": eyes}).to_string()).unwrap()
    }

    #[test]
    fn the_eye_mesh_is_the_eye_file_mesh_with_outward_normals() {
        let eyes = eye_file(&[[0.03, -0.11, 1.5], [-0.03, -0.11, 1.5]]);
        let (m, bases) = eye_meshes(&eyes);
        assert_eq!(m.positions.len(), (eyes[0].vertices.len() + eyes[1].vertices.len()) * 3);
        assert_eq!(bases.iter().map(|b| b.side).collect::<Vec<_>>(), vec![0, 1]);
        for (e, b) in eyes.iter().zip(&bases) {
            for (k, p) in e.vertices.iter().enumerate() {
                let v = b.first + k;
                let d: Vec<f64> = (0..3).map(|i| m.positions[v * 3 + i] as f64 - e.centre[i]).collect();
                assert!((0..3).all(|i| (m.positions[v * 3 + i] as f64 - p[i]).abs() < 1e-6));
                assert!((0..3).map(|i| d[i] * m.normals[v * 3 + i] as f64).sum::<f64>() > 0.0);
            }
        }
    }

    #[test]
    fn a_model_eye_mesh_keeps_its_uvs_and_offsets_each_eyes_parts() {
        let mesh = gf_geom::eyeball::EyeballShape::default().mesh().unwrap();
        let eyes: Vec<Eye> = [[0.03, -0.11, 1.5], [-0.03, -0.11, 1.5]]
            .iter()
            .map(|&c| gf_geom::lids::eye_from_mesh(c, 0.012, [0.0, -1.0, 0.0], &mesh.placed(c, [0.0, -1.0, 0.0]).unwrap()).unwrap())
            .collect();
        let (m, _) = eye_meshes(&eyes);
        let per_eye = eyes[0].triangles.len();
        assert_eq!(m.uvs.len(), m.positions.len() / 3 * 2);
        assert_eq!(m.parts.len(), 8);
        assert_eq!(m.parts[4].first, per_eye);
        assert_eq!(m.parts.iter().map(|p| p.count).sum::<usize>(), per_eye * 2);
        assert_eq!(m.parts.iter().map(|p| p.name.as_str()).take(4).collect::<Vec<_>>(), gf_geom::eyeball::PARTS);
    }

    #[test]
    fn the_displayed_eye_is_the_settled_one_until_an_adjustment_moves_it() {
        let eyes = eye_file(&[[0.03, -0.11, 1.5], [-0.03, -0.11, 1.5]]);
        let (_, bases) = eye_meshes(&eyes);
        let a = 0.2_f64;
        let settled = Similarity { scale: 1.1, rotation: [[a.cos(), -a.sin(), 0.0], [a.sin(), a.cos(), 0.0], [0.0, 0.0, 1.0]], translation: [0.001, 0.002, -0.003] };
        let mut spec = EyeballSpec { follow: er::FOLLOW_HELPER.into(), rig: Some(format!("sha256:{}", "0".repeat(64))), model: None, left: Default::default(), right: Default::default() };
        spec.left.offset_mm = [1.0, 0.0, 0.0];
        let same = eye_display(&bases[0], Some(&settled), Some(&spec), Some(&spec));
        let c = bases[0].centre;
        let gap = |p: [f64; 3], q: [f64; 3]| (0..3).map(|k| (p[k] - q[k]).powi(2)).sum::<f64>().sqrt();
        assert!(gap(same.apply(c), settled.apply(c)) < 1e-12);
        let mut live = spec.clone();
        live.left.offset_mm = [1.0, 2.0, 0.0];
        let moved = eye_display(&bases[0], Some(&settled), Some(&spec), Some(&live));
        assert!((gap(moved.apply(c), settled.apply(c)) - 0.0022).abs() < 1e-9, "the 2 mm up offset rides on the settled scale 1.1");
        assert!(gap(eye_display(&bases[1], None, None, None).apply(bases[1].centre), bases[1].centre) < 1e-15);
        let off = eye_display(&bases[0], Some(&settled), Some(&spec), None);
        assert!(gap(off.apply(c), c) < 1e-15, "a live character without eyeballs shows the eyes file's eye, not the last settled follow");
        let still = EyeballSpec { follow: er::FOLLOW_NONE.into(), ..spec.clone() };
        let shown = eye_display(&bases[0], Some(&settled), Some(&spec), Some(&still));
        assert!((gap(shown.apply(c), c) - 0.001).abs() < 1e-12, "follow none keeps only the 1 mm adjustment");
    }

    #[test]
    fn precomputed_edge_pairs_flag_exactly_like_the_per_settle_scan() {
        let n = 9u32;
        let mut idx = Vec::new();
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                if (x, y) == (3, 3) {
                    continue;
                }
                let v = y * n + x;
                idx.extend([v, v + 1, v + n, v + n, v + 1, v + n + 1]);
            }
        }
        idx.extend([0, 1, 2 * n]);
        idx.extend([1, 0, 3 * n]);
        let corners = idx.len();
        let uvs: Vec<f32> = (0..corners * 2).map(|i| { let v = idx[i / 2] as f32; if (i / 6) % 7 == 0 { v * 0.37 + 0.5 } else { v * 0.37 } }).collect();
        for uv in [uvs.clone(), Vec::new()] {
            assert_eq!(edge_flags(&idx, &uv), edge_flags_reference(&idx, &uv));
        }
        let (_, boundary, seams) = edge_flags(&idx, &uvs);
        assert!(boundary > 0 && seams > 0);
    }

    #[test]
    fn edge_flags_mark_open_boundaries_and_uv_seams() {
        let idx = [0, 1, 2, 2, 1, 3];
        let same = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
        let (flags, boundary, seams) = edge_flags(&idx, &same);
        assert_eq!((boundary, seams), (4, 0));
        assert_eq!(flags[0] & 1, 0);
        let mut cut = same;
        cut[6] = 0.5;
        let (flags, _, seams) = edge_flags(&idx, &cut);
        assert_eq!(seams, 1);
        assert_ne!(flags[0] & 8, 0);
    }

    #[test]
    fn normals_and_winding_follow_the_triangle_order() {
        let pos = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        let n = vertex_normals(&pos, &[0, 1, 2], 4);
        assert_eq!(&n[0..3], &[0.0, 0.0, 1.0]);
        assert_eq!(&n[9..12], &[0.0, 0.0, 0.0]);
        let tet = [0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3];
        let flipped = [0, 1, 2, 0, 3, 1, 0, 2, 3, 1, 3, 2];
        assert_eq!(winding_sign(&pos, &tet), -winding_sign(&pos, &flipped));
    }
}
