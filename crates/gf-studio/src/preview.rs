use std::collections::{BTreeMap, HashMap};

use gf_api::designer::TargetDeltas;
use rayon::prelude::*;

const CHUNK: usize = 1024;

pub struct Normals {
    offsets: Vec<u32>,
    faces: Vec<u32>,
    face: Vec<[f64; 3]>,
}

impl Normals {
    pub fn new(idx: &[u32], n: usize) -> Normals {
        let mut count = vec![0u32; n + 1];
        for &v in idx {
            count[v as usize + 1] += 1;
        }
        for i in 0..n {
            count[i + 1] += count[i];
        }
        let mut fill = count.clone();
        let mut faces = vec![0u32; idx.len()];
        for (t, tri) in idx.chunks_exact(3).enumerate() {
            for &v in tri {
                faces[fill[v as usize] as usize] = t as u32;
                fill[v as usize] += 1;
            }
        }
        Normals { offsets: count, faces, face: vec![[0.0; 3]; idx.len() / 3] }
    }

    pub fn compute(&mut self, pos: &[f32], idx: &[u32], out: &mut [f32], parallel: bool) {
        let face_work = |(c, chunk): (usize, &mut [[f64; 3]])| {
            for (k, f) in chunk.iter_mut().enumerate() {
                let t = (c * CHUNK + k) * 3;
                let (a, b, cc) = (idx[t] as usize * 3, idx[t + 1] as usize * 3, idx[t + 2] as usize * 3);
                let p = |i: usize| pos[i] as f64;
                let (ux, uy, uz) = (p(b) - p(a), p(b + 1) - p(a + 1), p(b + 2) - p(a + 2));
                let (vx, vy, vz) = (p(cc) - p(a), p(cc + 1) - p(a + 1), p(cc + 2) - p(a + 2));
                *f = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
            }
        };
        if parallel {
            self.face.par_chunks_mut(CHUNK).enumerate().for_each(face_work);
        } else {
            self.face.chunks_mut(CHUNK).enumerate().for_each(face_work);
        }
        let (offsets, faces, face) = (&self.offsets, &self.faces, &self.face);
        let vert_work = |(c, chunk): (usize, &mut [f32])| {
            for (k, v) in chunk.chunks_exact_mut(3).enumerate() {
                let g = c * CHUNK + k;
                let mut acc = [0.0f32; 3];
                for &t in &faces[offsets[g] as usize..offsets[g + 1] as usize] {
                    let f = face[t as usize];
                    for j in 0..3 {
                        acc[j] = (acc[j] as f64 + f[j]) as f32;
                    }
                }
                let l = ((acc[0] as f64).powi(2) + (acc[1] as f64).powi(2) + (acc[2] as f64).powi(2)).sqrt();
                let l = if l == 0.0 { 1.0 } else { l };
                for j in 0..3 {
                    v[j] = (acc[j] as f64 / l) as f32;
                }
            }
        };
        if parallel {
            out.par_chunks_mut(CHUNK * 3).enumerate().for_each(vert_work);
        } else {
            out.chunks_mut(CHUNK * 3).enumerate().for_each(vert_work);
        }
    }
}

pub struct Slope {
    pub up: Vec<f32>,
    pub down: Vec<f32>,
}

pub struct Sampled {
    pub key: String,
    pub values: Vec<f64>,
    pub layers: Vec<Vec<f32>>,
}

const SAMPLED_STATES: usize = 4;
const SLOPE_STATES: usize = 8;

pub fn macro_key(macros: &BTreeMap<String, f64>) -> String {
    format!("{macros:?}")
}

fn segment(values: &[f64], x: f64) -> (usize, f64) {
    let last = values.len() - 1;
    let x = x.clamp(values[0], values[last]);
    let i = values.partition_point(|&s| s <= x).saturating_sub(1).min(last.saturating_sub(1));
    let span = values[i + 1] - values[i];
    (i, if span > 0.0 { ((x - values[i]) / span).clamp(0.0, 1.0) } else { 0.0 })
}

impl Sampled {
    pub fn valid(&self) -> bool {
        self.values.len() >= 2 && self.layers.len() == self.values.len() && self.values.windows(2).all(|w| w[0] < w[1])
    }

    pub fn coefficients(&self, from: f64, to: f64) -> Vec<(usize, f64)> {
        let (a, ta) = segment(&self.values, to);
        let (b, tb) = segment(&self.values, from);
        let mut c: Vec<(usize, f64)> = Vec::with_capacity(4);
        for (i, w) in [(a, 1.0 - ta), (a + 1, ta), (b, tb - 1.0), (b + 1, -tb)] {
            match c.iter_mut().find(|e| e.0 == i) {
                Some(e) => e.1 += w,
                None => c.push((i, w)),
            }
        }
        c.retain(|e| e.1 != 0.0);
        c
    }
}

pub struct Preview {
    pub ids: Vec<String>,
    offsets: Vec<u32>,
    index: Vec<u32>,
    delta: Vec<[f32; 3]>,
    pub anchor: Vec<f32>,
    pub anchor_w: Vec<f64>,
    pub anchor_macros: BTreeMap<String, f64>,
    slopes: HashMap<(String, String), (u64, Slope)>,
    slope_seq: u64,
    pub slope_key: String,
    pub samples: HashMap<String, Vec<Sampled>>,
    pub state_keys: HashMap<String, String>,
    pool: Option<rayon::ThreadPool>,
    pub threads: usize,
}

pub struct Computed {
    pub targets: usize,
    pub macros: usize,
    pub missing_slopes: Vec<String>,
    pub sampled: Vec<String>,
    pub slope_fallback: Vec<String>,
    pub stale_slope: Vec<String>,
}

impl Preview {
    pub fn new(d: TargetDeltas, threads: usize) -> Result<Preview, String> {
        let pool = match threads {
            0 | 1 => None,
            n => Some(rayon::ThreadPoolBuilder::new().num_threads(n).thread_name(|i| format!("preview-{i}")).build().map_err(|e| e.to_string())?),
        };
        Ok(Preview {
            anchor_w: vec![0.0; d.ids.len()],
            anchor: vec![0.0; d.vertices * 3],
            ids: d.ids,
            offsets: d.offsets,
            index: d.index,
            delta: d.delta,
            anchor_macros: BTreeMap::new(),
            slopes: HashMap::new(),
            slope_seq: 0,
            slope_key: macro_key(&BTreeMap::new()),
            samples: HashMap::new(),
            state_keys: HashMap::new(),
            pool,
            threads: threads.max(1),
        })
    }

    pub fn entries(&self) -> usize {
        self.index.len()
    }

    pub fn set_anchor(&mut self, positions: &[f32], weights: &BTreeMap<String, f64>, macros: &BTreeMap<String, f64>) {
        self.anchor.copy_from_slice(positions);
        self.anchor_w = self.ids.iter().map(|id| weights.get(id).copied().unwrap_or(0.0)).collect();
        self.anchor_macros = macros.clone();
        self.slope_key = macro_key(macros);
        self.state_keys.clear();
    }

    pub fn has_slope(&self, name: &str, key: &str) -> bool {
        self.slopes.contains_key(&(name.to_string(), key.to_string()))
    }

    pub fn slope_for(&self, name: &str) -> Option<(&Slope, bool)> {
        if let Some((_, s)) = self.slopes.get(&(name.to_string(), self.slope_key.clone())) {
            return Some((s, true));
        }
        self.slopes.iter().filter(|((n, _), _)| n == name).max_by_key(|(_, (seq, _))| *seq).map(|(_, (_, s))| (s, false))
    }

    pub fn insert_slope(&mut self, name: &str, key: &str, s: Slope) {
        if s.up.len() != self.anchor.len() || s.down.len() != self.anchor.len() {
            return;
        }
        self.slope_seq += 1;
        self.slopes.insert((name.to_string(), key.to_string()), (self.slope_seq, s));
        while self.slopes.len() > SLOPE_STATES {
            let current = &self.slope_key;
            let Some(old) = self.slopes.iter().filter(|((_, k), _)| k != current).min_by_key(|(_, (seq, _))| *seq).map(|(id, _)| id.clone()) else { break };
            self.slopes.remove(&old);
        }
    }

    pub fn sampled_for(&self, name: &str) -> Option<&Sampled> {
        let key = self.state_keys.get(name)?;
        self.samples.get(name)?.iter().find(|s| &s.key == key)
    }

    pub fn add_samples(&mut self, name: &str, s: Sampled) {
        if !s.valid() || s.layers.iter().any(|l| l.len() != self.anchor.len()) {
            return;
        }
        let list = self.samples.entry(name.to_string()).or_default();
        list.retain(|e| e.key != s.key);
        list.insert(0, s);
        list.truncate(SAMPLED_STATES);
    }

    pub fn compute(&self, weights: &BTreeMap<String, f64>, macros: &BTreeMap<String, f64>, out: &mut [f32]) -> Computed {
        let active: Vec<(usize, f32)> = self
            .ids
            .iter()
            .enumerate()
            .filter_map(|(t, id)| {
                let dw = (weights.get(id).copied().unwrap_or(0.0) - self.anchor_w[t]) as f32;
                (dw != 0.0).then_some((t, dw))
            })
            .collect();
        let (mut missing, mut sampled, mut fallback, mut stale) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut layers: Vec<(f32, &[f32])> = Vec::new();
        for (name, &v) in macros {
            let v0 = self.anchor_macros.get(name).copied().unwrap_or(v);
            let dm = (v - v0) as f32;
            if dm == 0.0 {
                continue;
            }
            if let Some(s) = self.sampled_for(name) {
                layers.extend(s.coefficients(v0, v).into_iter().map(|(i, c)| (c as f32, s.layers[i].as_slice())));
                sampled.push(name.clone());
                continue;
            }
            match self.slope_for(name) {
                Some((s, exact)) => {
                    layers.push((dm, if dm > 0.0 { &s.up } else { &s.down }));
                    fallback.push(name.clone());
                    if !exact {
                        stale.push(name.clone());
                    }
                }
                None => missing.push(name.clone()),
            }
        }
        let (offsets, index, delta, anchor) = (&self.offsets, &self.index, &self.delta, &self.anchor);
        let work = |(c, chunk): (usize, &mut [f32])| {
            let lo = (c * CHUNK) as u32;
            let hi = lo + (chunk.len() / 3) as u32;
            chunk.copy_from_slice(&anchor[lo as usize * 3..hi as usize * 3]);
            for (dm, s) in &layers {
                for (o, x) in chunk.iter_mut().zip(&s[lo as usize * 3..hi as usize * 3]) {
                    *o += dm * x;
                }
            }
            for &(t, dw) in &active {
                let rows = &index[offsets[t] as usize..offsets[t + 1] as usize];
                let a = rows.partition_point(|&g| g < lo);
                let b = rows.partition_point(|&g| g < hi);
                let base = offsets[t] as usize;
                for e in a..b {
                    let v = (rows[e] - lo) as usize * 3;
                    let d = delta[base + e];
                    chunk[v] += dw * d[0];
                    chunk[v + 1] += dw * d[1];
                    chunk[v + 2] += dw * d[2];
                }
            }
        };
        match &self.pool {
            Some(p) => p.install(|| out.par_chunks_mut(CHUNK * 3).enumerate().for_each(work)),
            None => out.chunks_mut(CHUNK * 3).enumerate().for_each(work),
        }
        Computed { targets: active.len(), macros: sampled.len() + fallback.len(), missing_slopes: missing, sampled, slope_fallback: fallback, stale_slope: stale }
    }

    pub fn normals(&self, normals: &mut Normals, pos: &[f32], idx: &[u32], out: &mut [f32]) {
        match &self.pool {
            Some(p) => p.install(|| normals.compute(pos, idx, out, true)),
            None => normals.compute(pos, idx, out, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::vertex_normals;

    #[test]
    fn parallel_normals_equal_the_sequential_ones_bit_for_bit() {
        let n = 3000;
        let pos: Vec<f32> = (0..n * 3).map(|i| ((i as f32) * 0.618).sin() * 0.1).collect();
        let idx: Vec<u32> = (0..n as u32 * 4).map(|i| (i * 7919 + i / 3) % n as u32).collect();
        let idx = &idx[..idx.len() / 3 * 3];
        let want = vertex_normals(&pos, idx, n);
        for parallel in [true, false] {
            let mut out = vec![0.0f32; n * 3];
            Normals::new(idx, n).compute(&pos, idx, &mut out, parallel);
            assert!(out.iter().zip(&want).all(|(a, b)| a.to_bits() == b.to_bits()));
        }
    }

    #[test]
    fn the_preview_adds_the_weight_change_of_each_target_and_macro() {
        let d = TargetDeltas { vertices: 3000, ids: vec!["a".into(), "b".into()], offsets: vec![0, 2, 3], index: vec![1, 2500, 2500], delta: vec![[1.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 4.0]] };
        let mut p = Preview::new(d, 2).unwrap();
        let anchor = vec![0.5f32; 9000];
        let none = BTreeMap::new();
        p.set_anchor(&anchor, &BTreeMap::from([("a".to_string(), 0.25)]), &BTreeMap::from([("age".to_string(), 0.5)]));
        let mut out = vec![0.0f32; 9000];
        let w = BTreeMap::from([("a".to_string(), 0.75), ("b".to_string(), 0.5)]);
        let c = p.compute(&w, &none, &mut out);
        assert_eq!((c.targets, c.macros), (2, 0));
        assert_eq!(out[3], 1.0);
        assert_eq!(&out[7500..7503], &[0.5, 1.5, 2.5]);
        assert_eq!(out[0], 0.5);
        let mut slope = vec![0.0f32; 9000];
        slope[0] = 2.0;
        let key = p.slope_key.clone();
        p.insert_slope("age", &key, Slope { up: slope.clone(), down: slope });
        let c = p.compute(&p.anchor_w.iter().zip(&p.ids).map(|(w, id)| (id.clone(), *w)).collect(), &BTreeMap::from([("age".to_string(), 0.75)]), &mut out);
        assert_eq!((c.targets, c.macros), (0, 1));
        assert_eq!(out[0], 1.0);
        assert_eq!(out[7500], 0.5);
        assert_eq!(c.slope_fallback, vec!["age".to_string()]);
    }

    fn kinked(x: f64) -> f64 {
        if x < 0.5 { 0.01 * x } else { 0.005 + 0.04 * (x - 0.5) }
    }

    #[test]
    fn a_sampled_layer_follows_a_kinked_macro_across_the_knot_and_falls_back_to_the_slope_without_samples() {
        let d = TargetDeltas { vertices: 1, ids: vec![], offsets: vec![0], index: vec![], delta: vec![] };
        let mut p = Preview::new(d, 1).unwrap();
        let macros = |age: f64| BTreeMap::from([("age".to_string(), age), ("gender".to_string(), 0.3)]);
        p.set_anchor(&[0.0, 0.0, kinked(0.45) as f32], &BTreeMap::new(), &macros(0.45));
        let key = p.slope_key.clone();
        p.insert_slope("age", &key, Slope { up: vec![0.0, 0.0, 0.01], down: vec![0.0, 0.0, 0.01] });
        let mut out = vec![0.0f32; 3];
        let c = p.compute(&BTreeMap::new(), &macros(0.85), &mut out);
        assert_eq!((c.slope_fallback.len(), c.sampled.len()), (1, 0));
        assert!((out[2] as f64 - kinked(0.85)).abs() > 0.01);
        let values = vec![0.0, 0.25, 0.5, 0.75, 1.0];
        let layers = values.iter().map(|&x| vec![0.0, 0.0, kinked(x) as f32]).collect();
        p.state_keys.insert("age".into(), "gender 0.3".into());
        p.add_samples("age", Sampled { key: "gender 0.4".into(), values: values.clone(), layers });
        assert!(p.sampled_for("age").is_none());
        let layers = values.iter().map(|&x| vec![0.0, 0.0, kinked(x) as f32]).collect();
        p.add_samples("age", Sampled { key: "gender 0.3".into(), values, layers });
        for to in [0.0, 0.2, 0.45, 0.5, 0.51, 0.85, 1.0] {
            let c = p.compute(&BTreeMap::new(), &macros(to), &mut out);
            assert_eq!(c.sampled.len(), (to != 0.45) as usize);
            assert!((out[2] as f64 - kinked(to)).abs() < 1e-7, "{to}: {} vs {}", out[2], kinked(to));
        }
        p.set_anchor(&[0.0, 0.0, kinked(0.7) as f32], &BTreeMap::new(), &macros(0.7));
        assert!(p.sampled_for("age").is_none());
        p.state_keys.insert("age".into(), "gender 0.3".into());
        let key = p.slope_key.clone();
        assert!(p.sampled_for("age").is_some() && !p.has_slope("age", &key));
        p.compute(&BTreeMap::new(), &macros(0.3), &mut out);
        assert!((out[2] as f64 - kinked(0.3)).abs() < 1e-7);
        p.state_keys.insert("age".into(), "gender 0.9".into());
        assert!(p.sampled_for("age").is_none());
    }

    #[test]
    fn a_slope_applies_only_at_its_macro_state_and_survives_a_target_only_anchor_move() {
        let d = TargetDeltas { vertices: 1, ids: vec![], offsets: vec![0], index: vec![], delta: vec![] };
        let mut p = Preview::new(d, 1).unwrap();
        let macros = |age: f64| BTreeMap::from([("age".to_string(), age), ("gender".to_string(), 0.3)]);
        p.set_anchor(&[0.0; 3], &BTreeMap::new(), &macros(0.5));
        let mut out = vec![0.0f32; 3];
        assert_eq!(p.compute(&BTreeMap::new(), &macros(0.6), &mut out).missing_slopes, vec!["age".to_string()]);
        p.insert_slope("age", &macro_key(&macros(0.7)), Slope { up: vec![0.0, 0.0, 1.0], down: vec![0.0, 0.0, 1.0] });
        assert!(p.slope_for("age").is_some_and(|(_, exact)| !exact));
        let c = p.compute(&BTreeMap::new(), &macros(0.6), &mut out);
        assert_eq!((c.missing_slopes.len(), c.stale_slope.clone()), (0, vec!["age".to_string()]));
        assert!((out[2] - 0.1).abs() < 1e-6);
        p.set_anchor(&[0.0; 3], &BTreeMap::from([("t".to_string(), 0.4)]), &macros(0.7));
        assert!(p.slope_for("age").is_some_and(|(_, exact)| exact));
        p.set_anchor(&[0.0; 3], &BTreeMap::from([("t".to_string(), 0.9)]), &macros(0.7));
        let c = p.compute(&BTreeMap::new(), &macros(0.8), &mut out);
        assert_eq!((c.slope_fallback.clone(), c.stale_slope.len()), (vec!["age".to_string()], 0));
        assert!((out[2] - 0.1).abs() < 1e-6);
        p.set_anchor(&[0.0; 3], &BTreeMap::new(), &BTreeMap::from([("age".to_string(), 0.7), ("gender".to_string(), 0.4)]));
        assert!(p.slope_for("age").is_some_and(|(_, exact)| !exact));
        for k in 0..12 {
            p.insert_slope("gender", &format!("k{k}"), Slope { up: vec![0.0; 3], down: vec![0.0; 3] });
        }
        assert!(p.has_slope("gender", "k11") && !p.has_slope("gender", "k0"));
        p.insert_slope("age", "short", Slope { up: vec![0.0], down: vec![0.0] });
        assert!(!p.has_slope("age", "short"));
    }
}
