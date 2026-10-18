use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::time::Instant;

use serde::Serialize;

use crate::dense::Uv;
use crate::quadric::{self, Quadric};

#[derive(Clone, Serialize)]
pub struct Settings {
    pub target_vertices: usize,
    pub min_angle_deg: f64,
    pub max_angle_deg: f64,
    pub max_normal_turn_deg: f64,
    pub boundary_weight: f64,
    pub improve_below_deg: f64,
    pub flip_flat_deg: f64,
}

#[derive(Clone, Default, Serialize)]
pub struct Stats {
    pub start_vertices: usize,
    pub start_triangles: usize,
    pub shapes: usize,
    pub vertices: usize,
    pub triangles: usize,
    pub collapses: usize,
    pub refused_link: usize,
    pub refused_boundary: usize,
    pub refused_uv_island: usize,
    pub refused_normal: usize,
    pub refused_quality: usize,
    pub refused_uv_flip: usize,
    pub flips: usize,
    pub reached_target: bool,
    pub seconds: f64,
}

pub struct Decimated {
    pub kept: Vec<u32>,
    pub owner: Vec<u32>,
    pub triangles: Vec<[u32; 3]>,
    pub triangle_uvs: Vec<[Uv; 3]>,
    pub stats: Stats,
}

struct Entry {
    cost: f64,
    u: u32,
    v: u32,
    version_u: u32,
    version_v: u32,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cost.total_cmp(&other.cost) == Ordering::Equal
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.total_cmp(&self.cost)
    }
}

enum Refusal {
    Link,
    Boundary,
    UvIsland,
    Normal,
    Quality,
    UvFlip,
}

struct Plan {
    moved: Vec<(u32, usize, Uv)>,
    shared: Vec<u32>,
}

struct Mesh<'a> {
    settings: &'a Settings,
    weights: Option<&'a [f64]>,
    shapes: Vec<Vec<[f64; 3]>>,
    tris: Vec<[u32; 3]>,
    uvs: Vec<[Uv; 3]>,
    tri_alive: Vec<bool>,
    vtris: Vec<Vec<u32>>,
    alive: Vec<bool>,
    version: Vec<u32>,
    quadrics: Vec<Vec<Quadric>>,
    parent: Vec<u32>,
    cos_turn: f64,
}

fn uv_close(a: Uv, b: Uv) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

fn uv_area(t: [Uv; 3]) -> f64 {
    let (a, b, c) = (t[0], t[1], t[2]);
    0.5 * (((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1])) as f64)
}

impl<'a> Mesh<'a> {
    fn neighbours(&self, u: u32) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::with_capacity(12);
        for &t in &self.vtris[u as usize] {
            for &w in &self.tris[t as usize] {
                if w != u && !out.contains(&w) {
                    out.push(w);
                }
            }
        }
        out
    }

    fn shared(&self, u: u32, v: u32) -> Vec<u32> {
        self.vtris[u as usize].iter().copied().filter(|&t| self.tris[t as usize].contains(&v)).collect()
    }

    fn boundary_neighbours(&self, u: u32) -> Vec<u32> {
        let mut counts: Vec<(u32, u32)> = Vec::with_capacity(12);
        for &t in &self.vtris[u as usize] {
            for &w in &self.tris[t as usize] {
                if w == u {
                    continue;
                }
                match counts.iter_mut().find(|(x, _)| *x == w) {
                    Some(entry) => entry.1 += 1,
                    None => counts.push((w, 1)),
                }
            }
        }
        counts.into_iter().filter(|&(_, c)| c == 1).map(|(w, _)| w).collect()
    }

    fn candidates(&self, u: u32) -> Vec<(f64, u32)> {
        let edges = self.boundary_neighbours(u);
        let mut out: Vec<(f64, u32)> = self
            .neighbours(u)
            .into_iter()
            .filter(|w| edges.is_empty() || edges.contains(w))
            .map(|v| {
                let cost = (0..self.shapes.len())
                    .map(|k| {
                        quadric::eval_pair(&self.quadrics[k][u as usize], &self.quadrics[k][v as usize], self.shapes[k][v as usize])
                    })
                    .sum::<f64>();
                let cost = match self.weights {
                    Some(w) => cost * 0.5 * (w[u as usize] + w[v as usize]),
                    None => cost,
                };
                (cost.max(0.0), v)
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    fn check(&self, u: u32, v: u32) -> Result<Plan, Refusal> {
        let shared = self.shared(u, v);
        if shared.is_empty() || shared.len() > 2 {
            return Err(Refusal::Link);
        }
        let edges = self.boundary_neighbours(u);
        if !edges.is_empty() && (edges.len() != 2 || shared.len() != 1 || !edges.contains(&v)) {
            return Err(Refusal::Boundary);
        }
        let near_v = self.neighbours(v);
        let common = self.neighbours(u).into_iter().filter(|w| near_v.contains(w)).count();
        if common != shared.len() {
            return Err(Refusal::Link);
        }
        let mut moved = Vec::with_capacity(self.vtris[u as usize].len());
        for &t in &self.vtris[u as usize] {
            if shared.contains(&t) {
                continue;
            }
            let tri = self.tris[t as usize];
            let corner = tri.iter().position(|&x| x == u).unwrap();
            let uv_u = self.uvs[t as usize][corner];
            let source = shared.iter().find(|&&s| {
                let st = self.tris[s as usize];
                uv_close(self.uvs[s as usize][st.iter().position(|&x| x == u).unwrap()], uv_u)
            });
            let Some(&s) = source else {
                return Err(Refusal::UvIsland);
            };
            let st = self.tris[s as usize];
            let uv_v = self.uvs[s as usize][st.iter().position(|&x| x == v).unwrap()];
            let mut new_tri = tri;
            new_tri[corner] = v;
            for shape in &self.shapes {
                let old = tri.map(|i| shape[i as usize]);
                let new = new_tri.map(|i| shape[i as usize]);
                let n_old = quadric::cross(quadric::sub(old[1], old[0]), quadric::sub(old[2], old[0]));
                let n_new = quadric::cross(quadric::sub(new[1], new[0]), quadric::sub(new[2], new[0]));
                let (l_old, l_new) = (quadric::length(n_old), quadric::length(n_new));
                if l_new < 1e-14 || quadric::dot(n_old, n_new) < self.cos_turn * l_old * l_new {
                    return Err(Refusal::Normal);
                }
            }
            let base = &self.shapes[0];
            let old_angles = quadric::corner_angles(tri.map(|i| base[i as usize]));
            let new_angles = quadric::corner_angles(new_tri.map(|i| base[i as usize]));
            let (old_min, new_min) = (old_angles.iter().cloned().fold(180.0, f64::min), new_angles.iter().cloned().fold(180.0, f64::min));
            let (old_max, new_max) = (old_angles.iter().cloned().fold(0.0, f64::max), new_angles.iter().cloned().fold(0.0, f64::max));
            if (new_min < self.settings.min_angle_deg && new_min < old_min - 1e-9)
                || (new_max > self.settings.max_angle_deg && new_max > old_max + 1e-9)
            {
                return Err(Refusal::Quality);
            }
            let old_uv = self.uvs[t as usize];
            let mut new_uv = old_uv;
            new_uv[corner] = uv_v;
            let (a_old, a_new) = (uv_area(old_uv), uv_area(new_uv));
            if a_new.abs() < 1e-14 || a_old.signum() != a_new.signum() {
                return Err(Refusal::UvFlip);
            }
            moved.push((t, corner, uv_v));
        }
        Ok(Plan { moved, shared })
    }

    fn apply(&mut self, u: u32, v: u32, plan: Plan) {
        for &t in &plan.shared {
            self.tri_alive[t as usize] = false;
            for w in self.tris[t as usize] {
                self.vtris[w as usize].retain(|&x| x != t);
            }
        }
        for (t, corner, uv) in plan.moved {
            self.tris[t as usize][corner] = v;
            self.uvs[t as usize][corner] = uv;
            self.vtris[v as usize].push(t);
        }
        self.vtris[u as usize].clear();
        for k in 0..self.shapes.len() {
            let q = self.quadrics[k][u as usize];
            quadric::add(&mut self.quadrics[k][v as usize], &q);
        }
        self.alive[u as usize] = false;
        self.parent[u as usize] = v;
        self.version[u as usize] += 1;
        self.version[v as usize] += 1;
        for w in self.neighbours(v) {
            self.version[w as usize] += 1;
        }
    }

    fn min_angle(&self, tri: [u32; 3]) -> f64 {
        quadric::corner_angles(tri.map(|i| self.shapes[0][i as usize])).iter().cloned().fold(180.0, f64::min)
    }

    fn try_flip(&mut self, t: u32, edge: usize, flat_cos: f64) -> bool {
        let tri = self.tris[t as usize];
        let (a, b, c) = (tri[edge], tri[(edge + 1) % 3], tri[(edge + 2) % 3]);
        let others: Vec<u32> = self.vtris[a as usize]
            .iter()
            .copied()
            .filter(|&s| s != t && self.tris[s as usize].contains(&b))
            .collect();
        if others.len() != 1 {
            return false;
        }
        let s = others[0];
        let st = self.tris[s as usize];
        let ds = (0..3).find(|&i| st[i] != a && st[i] != b).unwrap();
        let d = st[ds];
        if d == c || self.neighbours(c).contains(&d) {
            return false;
        }
        let corner_uv = |tri_index: u32, vertex: u32| {
            let tr = self.tris[tri_index as usize];
            self.uvs[tri_index as usize][tr.iter().position(|&x| x == vertex).unwrap()]
        };
        if !uv_close(corner_uv(t, a), corner_uv(s, a)) || !uv_close(corner_uv(t, b), corner_uv(s, b)) {
            return false;
        }
        let new_t = [a, d, c];
        let new_s = [d, b, c];
        for shape in &self.shapes {
            let normal = |x: [u32; 3]| {
                let p = x.map(|i| shape[i as usize]);
                quadric::cross(quadric::sub(p[1], p[0]), quadric::sub(p[2], p[0]))
            };
            let (n_t, n_s) = (normal(tri), normal(st));
            let (l_t, l_s) = (quadric::length(n_t), quadric::length(n_s));
            if l_t < 1e-20 || l_s < 1e-20 || quadric::dot(n_t, n_s) < flat_cos * l_t * l_s {
                return false;
            }
            for x in [new_t, new_s] {
                let n = normal(x);
                let l = quadric::length(n);
                if l < 1e-20 || quadric::dot(n, n_t) <= 0.0 || quadric::dot(n, n_s) <= 0.0 {
                    return false;
                }
            }
        }
        let before = self.min_angle(tri).min(self.min_angle(st));
        let after = self.min_angle(new_t).min(self.min_angle(new_s));
        if after <= before + 1e-6 {
            return false;
        }
        let uv_t = [corner_uv(t, a), corner_uv(s, d), corner_uv(t, c)];
        let uv_s = [corner_uv(s, d), corner_uv(t, b), corner_uv(t, c)];
        let sign = uv_area(self.uvs[t as usize]).signum();
        if [uv_t, uv_s].iter().any(|u| uv_area(*u).abs() < 1e-14 || uv_area(*u).signum() != sign) {
            return false;
        }
        self.tris[t as usize] = new_t;
        self.uvs[t as usize] = uv_t;
        self.tris[s as usize] = new_s;
        self.uvs[s as usize] = uv_s;
        self.vtris[b as usize].retain(|&x| x != t);
        self.vtris[d as usize].push(t);
        self.vtris[a as usize].retain(|&x| x != s);
        self.vtris[c as usize].push(s);
        true
    }

    fn improve(&mut self, below_deg: f64, flat_deg: f64) -> usize {
        let flat_cos = flat_deg.to_radians().cos();
        let mut total = 0;
        for _ in 0..8 {
            let mut flipped = 0;
            for t in 0..self.tris.len() as u32 {
                if !self.tri_alive[t as usize] || self.min_angle(self.tris[t as usize]) >= below_deg {
                    continue;
                }
                for edge in 0..3 {
                    if self.try_flip(t, edge, flat_cos) {
                        flipped += 1;
                        break;
                    }
                }
            }
            total += flipped;
            if flipped == 0 {
                break;
            }
        }
        total
    }

    fn push_best(&self, heap: &mut BinaryHeap<Entry>, u: u32) {
        if let Some(&(cost, v)) = self.candidates(u).first() {
            heap.push(Entry { cost, u, v, version_u: self.version[u as usize], version_v: self.version[v as usize] });
        }
    }
}

pub fn decimate(shapes: Vec<Vec<[f64; 3]>>, triangles: &[[u32; 3]], uvs: &[[Uv; 3]], settings: &Settings) -> Decimated {
    decimate_levels(shapes, triangles, uvs, settings, None, &[]).remove(0)
}

pub fn decimate_levels(
    shapes: Vec<Vec<[f64; 3]>>,
    triangles: &[[u32; 3]],
    uvs: &[[Uv; 3]],
    settings: &Settings,
    weights: Option<&[f64]>,
    coarser: &[usize],
) -> Vec<Decimated> {
    let started = Instant::now();
    let n = shapes[0].len();
    let centre = {
        let mut c = [0.0f64; 3];
        for p in &shapes[0] {
            for i in 0..3 {
                c[i] += p[i];
            }
        }
        c.map(|x| x / n as f64)
    };
    let shapes: Vec<Vec<[f64; 3]>> = shapes.into_iter().map(|s| s.into_iter().map(|p| quadric::sub(p, centre)).collect()).collect();
    let mut vtris: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (t, tri) in triangles.iter().enumerate() {
        for &w in tri {
            vtris[w as usize].push(t as u32);
        }
    }
    let mut quadrics: Vec<Vec<Quadric>> = vec![vec![[0.0; 10]; n]; shapes.len()];
    for (k, shape) in shapes.iter().enumerate() {
        for tri in triangles {
            let p = tri.map(|i| shape[i as usize]);
            let normal = quadric::cross(quadric::sub(p[1], p[0]), quadric::sub(p[2], p[0]));
            let area2 = quadric::length(normal);
            if area2 < 1e-20 {
                continue;
            }
            let unit = normal.map(|x| x / area2);
            let q = quadric::from_plane(unit, -quadric::dot(unit, p[0]), area2 * 0.5);
            for &w in tri {
                quadric::add(&mut quadrics[k][w as usize], &q);
            }
        }
    }
    let mut keys: Vec<(u64, u32)> = Vec::with_capacity(triangles.len() * 3);
    for (t, tri) in triangles.iter().enumerate() {
        for e in 0..3 {
            let (a, b) = (tri[e], tri[(e + 1) % 3]);
            keys.push((((a.min(b) as u64) << 32) | a.max(b) as u64, t as u32));
        }
    }
    keys.sort_unstable_by_key(|k| k.0);
    let mut i = 0;
    while i < keys.len() {
        let mut j = i + 1;
        while j < keys.len() && keys[j].0 == keys[i].0 {
            j += 1;
        }
        if j - i == 1 {
            let (a, b) = ((keys[i].0 >> 32) as usize, (keys[i].0 & 0xffff_ffff) as usize);
            let tri = triangles[keys[i].1 as usize];
            for (k, shape) in shapes.iter().enumerate() {
                let p = tri.map(|x| shape[x as usize]);
                let face = quadric::cross(quadric::sub(p[1], p[0]), quadric::sub(p[2], p[0]));
                let edge = quadric::sub(shape[b], shape[a]);
                let plane = quadric::cross(edge, face);
                let length = quadric::length(plane);
                if length < 1e-20 {
                    continue;
                }
                let unit = plane.map(|x| x / length);
                let q = quadric::from_plane(unit, -quadric::dot(unit, shape[a]), settings.boundary_weight * quadric::dot(edge, edge));
                quadric::add(&mut quadrics[k][a], &q);
                quadric::add(&mut quadrics[k][b], &q);
            }
        }
        i = j;
    }
    drop(keys);
    let mut mesh = Mesh {
        settings,
        weights,
        shapes,
        tris: triangles.to_vec(),
        uvs: uvs.to_vec(),
        tri_alive: vec![true; triangles.len()],
        vtris,
        alive: vec![true; n],
        version: vec![0; n],
        quadrics,
        parent: (0..n as u32).collect(),
        cos_turn: settings.max_normal_turn_deg.to_radians().cos(),
    };
    let mut stats = Stats { start_vertices: n, start_triangles: triangles.len(), shapes: mesh.shapes.len(), ..Default::default() };
    let mut heap = BinaryHeap::with_capacity(n);
    for u in 0..n as u32 {
        mesh.push_best(&mut heap, u);
    }
    let mut alive_count = n;
    let mut levels = Vec::with_capacity(1 + coarser.len());
    for (level, &target) in std::iter::once(&settings.target_vertices).chain(coarser).enumerate() {
        if level > 0 {
            heap.clear();
            for u in 0..n as u32 {
                if mesh.alive[u as usize] {
                    mesh.push_best(&mut heap, u);
                }
            }
        }
        collapse_to(&mut mesh, &mut heap, &mut alive_count, target, &mut stats);
        stats.flips = mesh.improve(settings.improve_below_deg, settings.flip_flat_deg);
        stats.reached_target = alive_count <= target;
        stats.seconds = started.elapsed().as_secs_f64();
        levels.push(extract(&mut mesh, &stats));
    }
    levels
}

fn collapse_to(mesh: &mut Mesh, heap: &mut BinaryHeap<Entry>, alive_count: &mut usize, target: usize, stats: &mut Stats) {
    while *alive_count > target {
        let Some(entry) = heap.pop() else { break };
        let (u, v) = (entry.u, entry.v);
        if !mesh.alive[u as usize]
            || !mesh.alive[v as usize]
            || entry.version_u != mesh.version[u as usize]
            || entry.version_v != mesh.version[v as usize]
        {
            continue;
        }
        match mesh.check(u, v) {
            Ok(plan) => {
                mesh.apply(u, v, plan);
                *alive_count -= 1;
                stats.collapses += 1;
                mesh.push_best(heap, v);
                for w in mesh.neighbours(v) {
                    mesh.push_best(heap, w);
                }
            }
            Err(refusal) => {
                match refusal {
                    Refusal::Link => stats.refused_link += 1,
                    Refusal::Boundary => stats.refused_boundary += 1,
                    Refusal::UvIsland => stats.refused_uv_island += 1,
                    Refusal::Normal => stats.refused_normal += 1,
                    Refusal::Quality => stats.refused_quality += 1,
                    Refusal::UvFlip => stats.refused_uv_flip += 1,
                }
                for (cost, w) in mesh.candidates(u).into_iter().filter(|&(_, w)| w != v) {
                    if mesh.check(u, w).is_ok() {
                        heap.push(Entry { cost, u, v: w, version_u: mesh.version[u as usize], version_v: mesh.version[w as usize] });
                        break;
                    }
                }
            }
        }
    }
}

fn extract(mesh: &mut Mesh, stats: &Stats) -> Decimated {
    let n = mesh.alive.len();
    let mut remap = vec![u32::MAX; n];
    let mut kept = Vec::new();
    let mut out_tris = Vec::new();
    let mut out_uvs = Vec::new();
    for t in 0..mesh.tris.len() {
        if !mesh.tri_alive[t] {
            continue;
        }
        let tri = mesh.tris[t].map(|w| {
            if remap[w as usize] == u32::MAX {
                remap[w as usize] = kept.len() as u32;
                kept.push(w);
            }
            remap[w as usize]
        });
        out_tris.push(tri);
        out_uvs.push(mesh.uvs[t]);
    }
    let mut owner = vec![u32::MAX; n];
    for d in 0..n {
        let mut root = d as u32;
        while mesh.parent[root as usize] != root {
            root = mesh.parent[root as usize];
        }
        owner[d] = remap[root as usize];
        let mut walk = d as u32;
        while mesh.parent[walk as usize] != root {
            let next = mesh.parent[walk as usize];
            mesh.parent[walk as usize] = root;
            walk = next;
        }
    }
    let stats = Stats { vertices: kept.len(), triangles: out_tris.len(), ..stats.clone() };
    Decimated { kept, owner, triangles: out_tris, triangle_uvs: out_uvs, stats }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize) -> (Vec<[f64; 3]>, Vec<[u32; 3]>, Vec<[Uv; 3]>) {
        let mut points = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let (x, y) = (i as f64 / (n - 1) as f64, j as f64 / (n - 1) as f64);
                points.push([x * 0.1, y * 0.1, 0.01 * ((x - 0.5).powi(2) + (y - 0.3).powi(2)) + 0.002 * (9.0 * x).sin()]);
            }
        }
        let mut tris = Vec::new();
        let mut uvs = Vec::new();
        let uv = |v: u32| [(v as usize % n) as f32 / (n - 1) as f32, (v as usize / n) as f32 / (n - 1) as f32];
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let a = (j * n + i) as u32;
                let (b, c, d) = (a + 1, a + n as u32 + 1, a + n as u32);
                for t in [[a, b, c], [a, c, d]] {
                    tris.push(t);
                    uvs.push(t.map(uv));
                }
            }
        }
        (points, tris, uvs)
    }

    fn settings(target: usize) -> Settings {
        Settings {
            target_vertices: target,
            min_angle_deg: 4.0,
            max_angle_deg: 170.0,
            max_normal_turn_deg: 45.0,
            boundary_weight: 100.0,
            improve_below_deg: 20.0,
            flip_flat_deg: 15.0,
        }
    }

    #[test]
    fn coarser_levels_keep_a_subset_of_the_finer_vertices() {
        let (points, tris, uvs) = grid(24);
        let levels = decimate_levels(vec![points], &tris, &uvs, &settings(300), None, &[150, 60]);
        assert_eq!(levels.iter().map(|l| l.kept.len()).collect::<Vec<_>>(), vec![300, 150, 60]);
        for pair in levels.windows(2) {
            assert!(pair[1].kept.iter().all(|k| pair[0].kept.contains(k)), "a coarser level kept a vertex the finer one dropped");
        }
        for level in &levels {
            assert!(level.owner.iter().all(|&o| (o as usize) < level.kept.len()));
            assert!(level.kept.iter().enumerate().all(|(g, &d)| level.owner[d as usize] == g as u32));
        }
    }

    #[test]
    fn the_first_level_equals_a_single_level_build() {
        let (points, tris, uvs) = grid(20);
        let single = decimate(vec![points.clone()], &tris, &uvs, &settings(200));
        let nested = decimate_levels(vec![points], &tris, &uvs, &settings(200), None, &[80]);
        assert_eq!(single.kept, nested[0].kept);
        assert_eq!(single.triangles, nested[0].triangles);
        assert_eq!(single.owner, nested[0].owner);
    }

    #[test]
    fn a_heavy_region_weight_keeps_more_of_that_region() {
        let (points, tris, uvs) = grid(24);
        let weights: Vec<f64> = points.iter().map(|p| if p[0] < 0.05 { 8.0 } else { 1.0 }).collect();
        let plain = decimate_levels(vec![points.clone()], &tris, &uvs, &settings(150), None, &[]);
        let weighted = decimate_levels(vec![points.clone()], &tris, &uvs, &settings(150), Some(&weights), &[]);
        let left = |d: &Decimated| d.kept.iter().filter(|&&k| points[k as usize][0] < 0.05).count();
        assert!(left(&weighted[0]) > left(&plain[0]), "weighted {} vs plain {}", left(&weighted[0]), left(&plain[0]));
    }
}
