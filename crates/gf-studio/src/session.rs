use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use gf_api::designer;
use gf_api::{ApiError, Ctx};
use serde_json::{Value, json};

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
}

pub fn run_settle(input: &Value) -> Result<SettleOut, String> {
    let t = Instant::now();
    let s = designer::settle_mesh(input).map_err(err)?;
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
}

const EYE_RINGS: u32 = 24;
const EYE_SEGMENTS: u32 = 48;

pub fn eye_mesh(spheres: &[([f64; 3], f64)]) -> EyeMesh {
    let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    for &(c, r) in spheres {
        let first = (positions.len() / 3) as u32;
        for i in 0..=EYE_RINGS {
            let theta = std::f64::consts::PI * i as f64 / EYE_RINGS as f64;
            for j in 0..EYE_SEGMENTS {
                let phi = std::f64::consts::TAU * j as f64 / EYE_SEGMENTS as f64;
                let n = [theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos()];
                positions.extend((0..3).map(|k| (c[k] + r * n[k]) as f32));
                normals.extend(n.iter().map(|&x| x as f32));
            }
        }
        let at = |i: u32, j: u32| first + i * EYE_SEGMENTS + j % EYE_SEGMENTS;
        for i in 0..EYE_RINGS {
            for j in 0..EYE_SEGMENTS {
                let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
                indices.extend([a, b, c, a, c, d]);
            }
        }
    }
    EyeMesh { positions, normals, indices }
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
}

fn err(e: ApiError) -> String {
    serde_json::to_string(&e.to_json()).unwrap_or_else(|_| "gf-api error".into())
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

impl Session {
    pub fn open(store: &str, spec: Option<&str>, macro_file: Option<&str>, eyes_file: Option<&str>) -> Result<(Session, Value), String> {
        let ctx = Ctx::from_env();
        let eyes = match eyes_file {
            Some(p) => Some(eye_mesh(&gf_io::load_eyes(std::path::Path::new(p))?.iter().map(|e| (e.centre, e.radius)).collect::<Vec<_>>())),
            None => None,
        };
        let mut timings = BTreeMap::new();
        let t = Instant::now();
        let mut input = json!({"store": store});
        if let Some(s) = spec {
            input["spec"] = json!(s);
        }
        if let Some(m) = macro_file {
            input["macro"] = json!(m);
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
        Ok(())
    }

    pub fn reference_input(&self) -> Value {
        let neutral = Character { name: "neutral".into(), weights: BTreeMap::new(), macros: Value::Null, residual: Value::Null, ..self.start.clone() };
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

    #[test]
    fn eye_spheres_are_closed_outward_and_on_their_radius() {
        let spheres = [([0.03, -0.11, 1.5], 0.015), ([-0.03, -0.11, 1.5], 0.012)];
        let m = eye_mesh(&spheres);
        let per = ((EYE_RINGS + 1) * EYE_SEGMENTS) as usize;
        assert_eq!(m.positions.len(), 2 * per * 3);
        assert_eq!(m.indices.len(), 2 * (EYE_RINGS * EYE_SEGMENTS * 6) as usize);
        for (k, &(c, r)) in spheres.iter().enumerate() {
            let idx: Vec<u32> = m.indices[k * m.indices.len() / 2..(k + 1) * m.indices.len() / 2].to_vec();
            assert_eq!(winding_sign(&m.positions, &idx), 1.0);
            for v in k * per..(k + 1) * per {
                let d: Vec<f64> = (0..3).map(|i| m.positions[v * 3 + i] as f64 - c[i]).collect();
                let len = d.iter().map(|x| x * x).sum::<f64>().sqrt();
                assert!((len - r).abs() < 1e-6, "{len} vs {r}");
                assert!((0..3).map(|i| d[i] * m.normals[v * 3 + i] as f64).sum::<f64>() > 0.0);
            }
        }
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
