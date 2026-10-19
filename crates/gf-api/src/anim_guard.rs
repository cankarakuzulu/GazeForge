use std::collections::{BTreeMap, HashMap};

use gf_core::game_mesh as gm;
use gf_geom::lids::{self, Eye};
use serde_json::{Value, json};

type P = [f64; 3];

const FRONT_DEG: f64 = 75.0;

fn conv(p: [f32; 3]) -> P {
    [f64::from(p[0]), -f64::from(p[2]), f64::from(p[1])]
}

fn length(v: P) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

struct Sparse {
    idx: Vec<u32>,
    dp: Vec<P>,
    dn: Vec<P>,
}

struct Lod {
    name: String,
    p0: Vec<P>,
    n0: Vec<P>,
    active: Vec<[u32; 3]>,
    const_facing: usize,
    lid_index: Vec<usize>,
    morphs: Vec<Sparse>,
    target_of: Vec<Option<usize>>,
    base_lids: usize,
    base_facing: usize,
}

pub struct Guard {
    lods: Vec<Lod>,
    eyes: Vec<Eye>,
    front: f64,
    names: Vec<String>,
}

struct Eval {
    lids: usize,
    facing: usize,
    bad: Vec<u32>,
}

impl Lod {
    fn eval(&self, w: &[f64], eyes: &[Eye], front: f64) -> Eval {
        let (mut p, mut n) = (self.p0.clone(), self.n0.clone());
        for (k, m) in self.morphs.iter().enumerate() {
            let Some(c) = self.target_of[k] else { continue };
            let x = w[c];
            if x.abs() < 1e-12 {
                continue;
            }
            for ((&i, d), e) in m.idx.iter().zip(&m.dp).zip(&m.dn) {
                for a in 0..3 {
                    p[i as usize][a] += x * d[a];
                    n[i as usize][a] += x * e[a];
                }
            }
        }
        let pts: Vec<P> = self.lid_index.iter().map(|&i| p[i]).collect();
        let nrm: Vec<P> = self.lid_index.iter().map(|&i| n[i]).collect();
        let lc = lids::lid_check(&pts, &nrm, eyes, front, 0.0);
        let mut bad: Vec<u32> = lc.inside_vertices.iter().map(|&k| self.lid_index[k] as u32).collect();
        let mut facing = self.const_facing;
        for t in &self.active {
            if gm::facing_dot(&p, &n, t) < 0.0 {
                facing += 1;
                bad.extend_from_slice(t);
            }
        }
        Eval { lids: lc.inside, facing, bad }
    }

    fn fails(&self, e: &Eval) -> bool {
        e.lids > self.base_lids || e.facing > self.base_facing
    }
}

impl Guard {
    pub fn new(glb: &[u8], eyes: Vec<Eye>, front: Option<f64>, names: &[String]) -> Result<Guard, String> {
        let (_, meshes) = gf_io::gltf::read_glb(glb)?;
        let front = front.unwrap_or(FRONT_DEG);
        let mut lods = Vec::new();
        for mesh in meshes.iter().filter(|m| !m.normals.is_empty() && !m.name.starts_with("Eye_") && !m.morphs.is_empty()) {
            let p0: Vec<P> = mesh.positions.iter().map(|&p| conv(p)).collect();
            let n0: Vec<P> = mesh.normals.iter().map(|&p| conv(p)).collect();
            let morphs: Vec<Sparse> = mesh
                .morphs
                .iter()
                .map(|m| {
                    let mut s = Sparse { idx: Vec::new(), dp: Vec::new(), dn: Vec::new() };
                    for i in 0..p0.len() {
                        let (d, e) = (m.positions[i], m.normals.get(i).copied().unwrap_or([0.0; 3]));
                        if d.iter().chain(e.iter()).any(|x| *x != 0.0) {
                            s.idx.push(i as u32);
                            s.dp.push(conv(d));
                            s.dn.push(conv(e));
                        }
                    }
                    s
                })
                .collect();
            let mut touched = vec![false; p0.len()];
            let mut reach = vec![0.0f64; p0.len()];
            for m in &morphs {
                for (&i, d) in m.idx.iter().zip(&m.dp) {
                    touched[i as usize] = true;
                    reach[i as usize] += length(*d);
                }
            }
            let tris: Vec<[u32; 3]> = mesh.indices.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            let (active, still): (Vec<[u32; 3]>, Vec<[u32; 3]>) = tris.into_iter().partition(|t| t.iter().any(|&i| touched[i as usize]));
            let const_facing = still.iter().filter(|t| gm::facing_dot(&p0, &n0, t) < 0.0).count();
            let mut seen: HashMap<[u32; 3], ()> = HashMap::new();
            let lid_index: Vec<usize> = (0..p0.len())
                .filter(|&i| seen.insert(mesh.positions[i].map(f32::to_bits), ()).is_none())
                .filter(|&i| {
                    eyes.iter().any(|e| {
                        let d = length([0, 1, 2].map(|a| p0[i][a] - e.centre[a]));
                        d - reach[i] <= 1.3 * e.radius && d + reach[i] >= 0.5 * e.radius
                    })
                })
                .collect();
            let target_of: Vec<Option<usize>> = mesh.morph_names.iter().map(|t| names.iter().position(|n| n == t)).collect();
            let mut lod = Lod { name: mesh.name.clone(), p0, n0, active, const_facing, lid_index, morphs, target_of, base_lids: 0, base_facing: 0 };
            let mut w = vec![0.0; names.len()];
            let e0 = lod.eval(&w, &eyes, front);
            let (mut bl, mut bf) = (e0.lids, e0.facing);
            for k in 0..lod.morphs.len() {
                let Some(c) = lod.target_of[k] else { continue };
                w[c] = 1.0;
                let e = lod.eval(&w, &eyes, front);
                w[c] = 0.0;
                bl = bl.max(e.lids);
                bf = bf.max(e.facing);
            }
            lod.base_lids = bl;
            lod.base_facing = bf;
            lods.push(lod);
        }
        if lods.is_empty() {
            return Err("the glb has no morphed mesh with normals".into());
        }
        Ok(Guard { lods, eyes, front, names: names.to_vec() })
    }

    fn first_failure(&self, w: &[f64]) -> Option<(usize, Vec<u32>)> {
        for (k, lod) in self.lods.iter().enumerate() {
            let e = lod.eval(w, &self.eyes, self.front);
            if lod.fails(&e) {
                return Some((k, e.bad));
            }
        }
        None
    }

    fn passes(&self, w: &[f64]) -> bool {
        self.first_failure(w).is_none()
    }

    fn involved(&self, lod: usize, bad: &[u32], w: &[f64]) -> Vec<usize> {
        let l = &self.lods[lod];
        let mut out = Vec::new();
        for (k, m) in l.morphs.iter().enumerate() {
            let Some(c) = l.target_of[k] else { continue };
            if w[c] > 1e-9 && !out.contains(&c) && bad.iter().any(|v| m.idx.binary_search(v).is_ok()) {
                out.push(c);
            }
        }
        out
    }

    fn largest_passing(&self, w: &[f64], scaled: &[usize]) -> Option<f64> {
        let at = |s: f64| {
            let mut x = w.to_vec();
            for &c in scaled {
                x[c] *= s;
            }
            x
        };
        if !self.passes(&at(0.0)) {
            return None;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..12 {
            let mid = 0.5 * (lo + hi);
            if self.passes(&at(mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some(lo * 0.97)
    }

    fn fix(&self, w: &mut [f64]) -> (bool, bool) {
        let mut touched = false;
        for _ in 0..6 {
            let Some((lod, bad)) = self.first_failure(w) else { return (touched, true) };
            let set = self.involved(lod, &bad, w);
            if set.is_empty() {
                return (touched, false);
            }
            let primary = set.iter().copied().fold(set[0], |a, c| if w[c] > w[a] { c } else { a });
            let secondary: Vec<usize> = set.iter().copied().filter(|&c| c != primary).collect();
            touched = true;
            match (!secondary.is_empty()).then(|| self.largest_passing(w, &secondary)).flatten() {
                Some(s) => {
                    for &c in &secondary {
                        w[c] *= s;
                    }
                }
                None => {
                    for &c in &secondary {
                        w[c] = 0.0;
                    }
                    match self.largest_passing(w, &[primary]) {
                        Some(s) => w[primary] *= s,
                        None => return (touched, false),
                    }
                }
            }
        }
        (touched, self.passes(w))
    }

    pub fn apply(&self, weights: &mut [Vec<f64>]) -> Value {
        let mut failing_before = Vec::new();
        let mut still = Vec::new();
        let mut reduction: BTreeMap<String, f64> = BTreeMap::new();
        for (f, row) in weights.iter_mut().enumerate() {
            if self.passes(row) {
                continue;
            }
            failing_before.push(f);
            let before = row.clone();
            let (_, ok) = self.fix(row);
            if !ok {
                still.push(f);
            }
            for (c, (a, b)) in before.iter().zip(row.iter()).enumerate() {
                if a - b > 1e-9 {
                    let e = reduction.entry(self.names[c].clone()).or_insert(0.0);
                    *e = e.max(a - b);
                }
            }
        }
        let r = |x: f64| (x * 1e5).round() / 1e5;
        json!({"ran": true, "front_deg": self.front,
               "rule": "per frame, per LOD: the evaluated mix may not have more lid-inside vertices or facing-against triangles than the worst of the neutral and the single shapes at weight 1 of this glb; a failing frame scales the other channels that move the failing vertices (all but the strongest) by the largest passing factor (bisection, then x0.97), and the strongest too only if that is not enough",
               "lods": self.lods.iter().map(|l| json!({"mesh": l.name, "baseline_lids_inside": l.base_lids, "baseline_facing_against": l.base_facing, "lid_vertices_checked": l.lid_index.len(), "active_triangles": l.active.len()})).collect::<Vec<_>>(),
               "frames": weights.len(), "frames_failing_before": failing_before.len(), "frames_still_failing": still,
               "first_failing_frames": failing_before.iter().take(40).collect::<Vec<_>>(),
               "max_reduction": reduction.into_iter().map(|(k, v)| (k, json!(r(v)))).collect::<serde_json::Map<String, Value>>()})
    }
}
