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

pub struct Preview {
    pub ids: Vec<String>,
    offsets: Vec<u32>,
    index: Vec<u32>,
    delta: Vec<[f32; 3]>,
    pub anchor: Vec<f32>,
    pub anchor_w: Vec<f64>,
    pub anchor_macros: BTreeMap<String, f64>,
    pub slopes: HashMap<String, Slope>,
    pool: Option<rayon::ThreadPool>,
    pub threads: usize,
}

pub struct Computed {
    pub targets: usize,
    pub macros: usize,
    pub missing_slopes: Vec<String>,
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
        self.slopes.clear();
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
        let mut missing = Vec::new();
        let mut layers: Vec<(f32, &[f32])> = Vec::new();
        for (name, &v) in macros {
            let dm = (v - self.anchor_macros.get(name).copied().unwrap_or(v)) as f32;
            if dm == 0.0 {
                continue;
            }
            match self.slopes.get(name) {
                Some(s) => layers.push((dm, if dm > 0.0 { &s.up } else { &s.down })),
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
        Computed { targets: active.len(), macros: layers.len(), missing_slopes: missing }
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
        p.slopes.insert("age".into(), Slope { up: slope.clone(), down: slope });
        let c = p.compute(&p.anchor_w.iter().zip(&p.ids).map(|(w, id)| (id.clone(), *w)).collect(), &BTreeMap::from([("age".to_string(), 0.75)]), &mut out);
        assert_eq!((c.targets, c.macros), (0, 1));
        assert_eq!(out[0], 1.0);
        assert_eq!(out[7500], 0.5);
    }
}
