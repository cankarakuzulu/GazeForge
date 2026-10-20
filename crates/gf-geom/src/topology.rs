use serde::Serialize;

use crate::Uv;
use crate::quadric::{cross, dot, length, sub};

pub struct ShapeSet {
    rest: Vec<[f64; 3]>,
    names: Vec<String>,
    near: Vec<Vec<(u32, [f32; 3])>>,
    normals: Option<Vec<[f64; 3]>>,
}

impl ShapeSet {
    pub fn new(rest: Vec<[f64; 3]>) -> ShapeSet {
        let n = rest.len();
        ShapeSet { rest, names: vec!["rest".into()], near: vec![Vec::new(); n], normals: None }
    }

    pub fn set_normals(&mut self, normals: Vec<[f64; 3]>) -> Result<(), String> {
        if normals.len() != self.rest.len() {
            return Err(format!("{} normals for {} vertices", normals.len(), self.rest.len()));
        }
        crate::check_finite(&normals).map_err(|e| format!("normals: {e}"))?;
        self.normals = Some(normals.into_iter().map(|n| n.map(|x| x / length(n).max(1e-30))).collect());
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn vertices(&self) -> usize {
        self.rest.len()
    }

    pub fn rest(&self) -> &[[f64; 3]] {
        &self.rest
    }

    pub fn entries(&self) -> usize {
        self.near.iter().map(Vec::len).sum()
    }

    pub fn add_positions(&mut self, name: &str, positions: &[[f64; 3]]) -> Result<(), String> {
        if positions.len() != self.rest.len() {
            return Err(format!("shape {name} has {} vertices, the mesh {}", positions.len(), self.rest.len()));
        }
        crate::check_finite(positions).map_err(|e| format!("shape {name}: {e}"))?;
        let id = self.names.len() as u32;
        self.names.push(name.to_string());
        for (v, p) in positions.iter().enumerate() {
            let d = sub(*p, self.rest[v]).map(|x| x as f32);
            if d != [0.0; 3] {
                self.near[v].push((id, d));
            }
        }
        Ok(())
    }

    pub fn add_deltas(&mut self, name: &str, indices: &[u32], deltas: &[[f64; 3]]) -> Result<(), String> {
        if indices.len() != deltas.len() {
            return Err(format!("shape {name}: {} indices but {} deltas", indices.len(), deltas.len()));
        }
        if let Some(bad) = indices.iter().find(|&&i| i as usize >= self.rest.len()) {
            return Err(format!("shape {name}: vertex {bad} is outside the mesh's {} vertices", self.rest.len()));
        }
        crate::check_finite(deltas).map_err(|e| format!("shape {name}: {e}"))?;
        let id = self.names.len() as u32;
        self.names.push(name.to_string());
        for (&v, d) in indices.iter().zip(deltas) {
            let list = &mut self.near[v as usize];
            match list.last_mut() {
                Some((s, sum)) if *s == id => {
                    for k in 0..3 {
                        sum[k] += d[k] as f32;
                    }
                }
                _ => list.push((id, d.map(|x| x as f32))),
            }
        }
        for &v in indices {
            let list = &mut self.near[v as usize];
            if list.last().is_some_and(|(s, d)| *s == id && *d == [0.0; 3]) {
                list.pop();
            }
        }
        Ok(())
    }

    fn at(&self, v: u32, delta: Option<&[f32; 3]>) -> [f64; 3] {
        let p = self.rest[v as usize];
        match delta {
            Some(d) => [p[0] + d[0] as f64, p[1] + d[1] as f64, p[2] + d[2] as f64],
            None => p,
        }
    }

    fn position(&self, v: u32, shape: u32) -> [f64; 3] {
        if shape == 0 {
            return self.rest[v as usize];
        }
        let list = &self.near[v as usize];
        self.at(v, list.binary_search_by_key(&shape, |e| e.0).ok().map(|i| &list[i].1))
    }

    fn each<const K: usize>(&self, v: [u32; K], mut f: impl FnMut(u32, &[[f64; 3]; K]) -> bool) -> bool {
        let rest = v.map(|i| self.rest[i as usize]);
        if !f(0, &rest) {
            return false;
        }
        let lists = v.map(|i| self.near[i as usize].as_slice());
        let mut at = [0usize; K];
        loop {
            let mut shape = u32::MAX;
            for k in 0..K {
                if let Some(e) = lists[k].get(at[k]) {
                    shape = shape.min(e.0);
                }
            }
            if shape == u32::MAX {
                return true;
            }
            let p: [[f64; 3]; K] = std::array::from_fn(|k| match lists[k].get(at[k]) {
                Some(e) if e.0 == shape => {
                    at[k] += 1;
                    self.at(v[k], Some(&e.1))
                }
                _ => rest[k],
            });
            if !f(shape, &p) {
                return false;
            }
        }
    }
}

fn angle_at(p: [f64; 3], q: [f64; 3], r: [f64; 3]) -> f64 {
    let (u, v) = (sub(q, p), sub(r, p));
    length(cross(u, v)).atan2(dot(u, v)).to_degrees()
}

pub fn min_angle(p: [[f64; 3]; 3]) -> f64 {
    let e = [0, 1, 2].map(|i| {
        let d = sub(p[(i + 2) % 3], p[(i + 1) % 3]);
        dot(d, d)
    });
    let i = if e[0] <= e[1] && e[0] <= e[2] { 0 } else if e[1] <= e[2] { 1 } else { 2 };
    angle_at(p[i], p[(i + 1) % 3], p[(i + 2) % 3])
}

pub fn max_angle(p: [[f64; 3]; 3]) -> f64 {
    let e = [0, 1, 2].map(|i| {
        let d = sub(p[(i + 2) % 3], p[(i + 1) % 3]);
        dot(d, d)
    });
    let i = if e[0] >= e[1] && e[0] >= e[2] { 0 } else if e[1] >= e[2] { 1 } else { 2 };
    angle_at(p[i], p[(i + 1) % 3], p[(i + 2) % 3])
}

fn normal(p: [[f64; 3]; 3]) -> [f64; 3] {
    cross(sub(p[1], p[0]), sub(p[2], p[0]))
}

fn uv_area(t: [Uv; 3]) -> f64 {
    let [a, b, c] = t.map(|u| [u[0] as f64, u[1] as f64]);
    0.5 * ((b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]))
}

fn uv_close(a: Uv, b: Uv) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

pub fn worst_angles(set: &ShapeSet, triangles: &[[u32; 3]]) -> Vec<f64> {
    crate::map_range(triangles.len(), |t| {
        let mut worst = 180.0f64;
        set.each(triangles[t], |_, p| {
            worst = worst.min(min_angle(*p));
            true
        });
        worst
    })
}

#[derive(Clone, Serialize)]
pub struct FlipSettings {
    pub below_deg: f64,
    pub min_gain_deg: f64,
    pub max_fold_deg: f64,
    pub max_rounds: usize,
    pub max_spread_deg: f64,
}

impl Default for FlipSettings {
    fn default() -> Self {
        FlipSettings { below_deg: 30.0, min_gain_deg: 0.5, max_fold_deg: 20.0, max_rounds: 32, max_spread_deg: 40.0 }
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Census {
    pub candidates: usize,
    pub boundary: usize,
    pub seam: usize,
    pub link: usize,
    pub no_gain: usize,
    pub fold: usize,
    pub turned: usize,
    pub crease: usize,
    pub uv_flip: usize,
    pub accepted: usize,
}

#[derive(Clone, Serialize)]
pub struct Round {
    pub round: usize,
    pub census: Census,
    pub flips: usize,
    pub total_flips: usize,
    pub worst_below_10: usize,
}

pub struct Flipped {
    pub triangles: Vec<[u32; 3]>,
    pub triangle_uvs: Vec<[Uv; 3]>,
    pub flips: usize,
    pub rounds: Vec<Round>,
    pub converged: bool,
}

struct Mesh<'a> {
    set: &'a ShapeSet,
    tris: Vec<[u32; 3]>,
    uvs: Vec<[Uv; 3]>,
    vtris: Vec<Vec<u32>>,
    worst: Vec<f64>,
}

struct Plan {
    t: u32,
    s: u32,
    before: f64,
    new_t: [u32; 3],
    new_s: [u32; 3],
    uv_t: [Uv; 3],
    uv_s: [Uv; 3],
    worst_t: f64,
    worst_s: f64,
}

enum Outcome {
    Boundary,
    Seam,
    Link,
    NoGain,
    Fold,
    Turn,
    Crease,
    UvFlip,
    Flip(Box<Plan>),
}

impl Mesh<'_> {
    fn across(&self, t: u32, edge: usize) -> Option<u32> {
        let tri = self.tris[t as usize];
        let (a, b) = (tri[edge], tri[(edge + 1) % 3]);
        let mut found = None;
        for &s in &self.vtris[a as usize] {
            if s != t && self.tris[s as usize].contains(&b) {
                if found.is_some() {
                    return None;
                }
                found = Some(s);
            }
        }
        found
    }

    fn linked(&self, c: u32, d: u32) -> bool {
        self.vtris[c as usize].iter().any(|&x| self.tris[x as usize].contains(&d))
    }

    fn corner_uv(&self, t: u32, v: u32) -> Uv {
        let tri = self.tris[t as usize];
        self.uvs[t as usize][tri.iter().position(|&x| x == v).unwrap()]
    }

    fn evaluate(&self, t: u32, edge: usize, settings: &FlipSettings, cos_fold: f64, cos_spread: f64) -> Outcome {
        let Some(s) = self.across(t, edge) else { return Outcome::Boundary };
        let tri = self.tris[t as usize];
        let (a, b, c) = (tri[edge], tri[(edge + 1) % 3], tri[(edge + 2) % 3]);
        let st = self.tris[s as usize];
        let Some(d) = st.iter().copied().find(|&x| x != a && x != b) else { return Outcome::Link };
        let sa = st.iter().position(|&x| x == a).unwrap();
        if st[(sa + 2) % 3] != b {
            return Outcome::Link;
        }
        if !uv_close(self.corner_uv(t, a), self.corner_uv(s, a)) || !uv_close(self.corner_uv(t, b), self.corner_uv(s, b)) {
            return Outcome::Seam;
        }
        if c == d || self.linked(c, d) {
            return Outcome::Link;
        }
        if let Some(normals) = &self.set.normals {
            let n = [a, b, c, d].map(|v| normals[v as usize]);
            if (0..4).any(|i| (i + 1..4).any(|j| dot(n[i], n[j]) < cos_spread)) {
                return Outcome::Crease;
            }
        }
        let before = self.worst[t as usize].min(self.worst[s as usize]);
        let need = before + settings.min_gain_deg;
        let new_t = [a, d, c];
        let new_s = [d, b, c];
        let (mut worst_t, mut worst_s) = (180.0f64, 180.0f64);
        let mut refusal = None;
        let mut rest = [[0.0; 3]; 4];
        let done = self.set.each([a, b, c, d], |shape, p| {
            let [pa, pb, pc, pd] = *p;
            let (qt, qs) = ([pa, pd, pc], [pd, pb, pc]);
            worst_t = worst_t.min(min_angle(qt));
            worst_s = worst_s.min(min_angle(qs));
            if worst_t.min(worst_s) < need {
                refusal = Some(Outcome::NoGain);
                return false;
            }
            let normals = [normal([pa, pb, pc]), normal([pb, pa, pd]), normal(qt), normal(qs)];
            let old = [0, 1, 2].map(|k| normals[0][k] + normals[1][k]);
            let lo = length(old);
            for m in [normals[2], normals[3]] {
                let lm = length(m);
                if !(lo > 0.0 && lm > 0.0 && dot(m, old) > cos_fold * lo * lm) {
                    refusal = Some(Outcome::Fold);
                    return false;
                }
            }
            if shape == 0 {
                rest = normals;
            } else {
                let turned = |k: usize| (dot(normals[k], rest[k]) <= 0.0) as u8;
                if turned(2) + turned(3) > turned(0) + turned(1) {
                    refusal = Some(Outcome::Turn);
                    return false;
                }
            }
            true
        });
        if !done {
            return refusal.unwrap_or(Outcome::NoGain);
        }
        let (ua, ub, uc, ud) = (self.corner_uv(t, a), self.corner_uv(t, b), self.corner_uv(t, c), self.corner_uv(s, d));
        let uv_t = [ua, ud, uc];
        let uv_s = [ud, ub, uc];
        let sign = uv_area(self.uvs[t as usize]).signum();
        let sign_s = uv_area(self.uvs[s as usize]).signum();
        if sign == 0.0 || sign != sign_s || [uv_t, uv_s].iter().any(|u| uv_area(*u).signum() != sign) {
            return Outcome::UvFlip;
        }
        Outcome::Flip(Box::new(Plan { t, s, before, new_t, new_s, uv_t, uv_s, worst_t, worst_s }))
    }

    fn apply(&mut self, plan: &Plan) {
        let [a, d, c] = plan.new_t;
        let b = plan.new_s[1];
        self.tris[plan.t as usize] = plan.new_t;
        self.uvs[plan.t as usize] = plan.uv_t;
        self.tris[plan.s as usize] = plan.new_s;
        self.uvs[plan.s as usize] = plan.uv_s;
        self.worst[plan.t as usize] = plan.worst_t;
        self.worst[plan.s as usize] = plan.worst_s;
        self.vtris[b as usize].retain(|&x| x != plan.t);
        self.vtris[d as usize].push(plan.t);
        self.vtris[a as usize].retain(|&x| x != plan.s);
        self.vtris[c as usize].push(plan.s);
    }
}

pub fn improve(set: &ShapeSet, triangles: &[[u32; 3]], uvs: &[[Uv; 3]], settings: &FlipSettings, on_round: &mut dyn FnMut(&Round)) -> Flipped {
    let n = set.vertices();
    let mut vtris: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (t, tri) in triangles.iter().enumerate() {
        for &w in tri {
            vtris[w as usize].push(t as u32);
        }
    }
    let mut mesh = Mesh { set, tris: triangles.to_vec(), uvs: uvs.to_vec(), vtris, worst: worst_angles(set, triangles) };
    let cos_fold = settings.max_fold_deg.to_radians().cos();
    let cos_spread = settings.max_spread_deg.to_radians().cos();
    let mut rounds = Vec::new();
    let mut total = 0;
    let mut converged = false;
    for round in 1..=settings.max_rounds {
        crate::cancel::point();
        let mut keys: Vec<(u32, u8)> = Vec::new();
        for t in 0..mesh.tris.len() as u32 {
            if mesh.worst[t as usize] >= settings.below_deg {
                continue;
            }
            for e in 0..3 {
                let tri = mesh.tris[t as usize];
                let (a, b) = (tri[e], tri[(e + 1) % 3]);
                let seen_from = mesh.across(t, e).filter(|&s| s < t).and_then(|s| {
                    let st = mesh.tris[s as usize];
                    (0..3).find(|&k| st[k] == b && st[(k + 1) % 3] == a).map(|k| (s, k as u8))
                });
                keys.push(seen_from.unwrap_or((t, e as u8)));
            }
        }
        keys.sort_unstable();
        keys.dedup();
        let outcomes = crate::map_range(keys.len(), |i| mesh.evaluate(keys[i].0, keys[i].1 as usize, settings, cos_fold, cos_spread));
        let mut census = Census { candidates: keys.len(), ..Default::default() };
        let mut plans: Vec<Box<Plan>> = Vec::new();
        for o in outcomes {
            match o {
                Outcome::Boundary => census.boundary += 1,
                Outcome::Seam => census.seam += 1,
                Outcome::Link => census.link += 1,
                Outcome::NoGain => census.no_gain += 1,
                Outcome::Fold => census.fold += 1,
                Outcome::Turn => census.turned += 1,
                Outcome::Crease => census.crease += 1,
                Outcome::UvFlip => census.uv_flip += 1,
                Outcome::Flip(p) => {
                    census.accepted += 1;
                    plans.push(p);
                }
            }
        }
        plans.sort_by(|x, y| x.before.total_cmp(&y.before).then(x.t.cmp(&y.t)).then(x.s.cmp(&y.s)));
        let mut touched = vec![false; mesh.tris.len()];
        let mut flips = 0;
        for plan in &plans {
            if touched[plan.t as usize] || touched[plan.s as usize] {
                continue;
            }
            if mesh.linked(plan.new_t[2], plan.new_t[1]) {
                continue;
            }
            mesh.apply(plan);
            touched[plan.t as usize] = true;
            touched[plan.s as usize] = true;
            flips += 1;
        }
        total += flips;
        let report = Round { round, census, flips, total_flips: total, worst_below_10: mesh.worst.iter().filter(|&&w| w < 10.0).count() };
        on_round(&report);
        rounds.push(report);
        if flips == 0 {
            converged = true;
            break;
        }
    }
    Flipped { triangles: mesh.tris, triangle_uvs: mesh.uvs, flips: total, rounds, converged }
}

#[derive(Clone, Serialize, PartialEq, Debug)]
pub struct AngleStats {
    pub triangles: usize,
    pub min_angle_p1: f64,
    pub min_angle_p5: f64,
    pub min_angle_p50: f64,
    pub min_angle_worst: f64,
    pub below_5: usize,
    pub below_10: usize,
    pub below_20: usize,
    pub max_angle_worst: f64,
    pub above_150: usize,
    pub turned: usize,
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

fn summarise(mut mins: Vec<f64>, maxs: &[f64], turned: usize) -> AngleStats {
    let below = |x: f64| mins.iter().filter(|&&m| m < x).count();
    let (below_5, below_10, below_20) = (below(5.0), below(10.0), below(20.0));
    mins.sort_by(f64::total_cmp);
    AngleStats {
        triangles: mins.len(),
        min_angle_p1: percentile(&mins, 0.01),
        min_angle_p5: percentile(&mins, 0.05),
        min_angle_p50: percentile(&mins, 0.5),
        min_angle_worst: mins.first().copied().unwrap_or(0.0),
        below_5,
        below_10,
        below_20,
        max_angle_worst: maxs.iter().copied().fold(0.0, f64::max),
        above_150: maxs.iter().filter(|&&m| m > 150.0).count(),
        turned,
    }
}

pub fn shape_stats(set: &ShapeSet, triangles: &[[u32; 3]]) -> (Vec<AngleStats>, AngleStats) {
    let n = set.vertices();
    let mut vtris: Vec<Vec<u32>> = vec![Vec::new(); n];
    for (t, tri) in triangles.iter().enumerate() {
        for &w in tri {
            vtris[w as usize].push(t as u32);
        }
    }
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); set.len()];
    for (v, list) in set.near.iter().enumerate() {
        for e in list {
            members[e.0 as usize].push(v as u32);
        }
    }
    let rest_tri = |t: &[u32; 3]| t.map(|i| set.rest[i as usize]);
    let rest_min: Vec<f64> = triangles.iter().map(|t| min_angle(rest_tri(t))).collect();
    let rest_max: Vec<f64> = triangles.iter().map(|t| max_angle(rest_tri(t))).collect();
    let rest_normal: Vec<[f64; 3]> = triangles.iter().map(|t| normal(rest_tri(t))).collect();
    let per_shape = crate::map_range(set.len(), |s| {
        crate::cancel::point();
        if s == 0 {
            return (summarise(rest_min.clone(), &rest_max, 0), Vec::new());
        }
        let mut touched: Vec<u32> = members[s].iter().flat_map(|&v| vtris[v as usize].iter().copied()).collect();
        touched.sort_unstable();
        touched.dedup();
        let (mut mins, mut maxs) = (rest_min.clone(), rest_max.clone());
        let mut turned = Vec::new();
        for &t in &touched {
            let p = triangles[t as usize].map(|v| set.position(v, s as u32));
            mins[t as usize] = min_angle(p);
            maxs[t as usize] = max_angle(p);
            if dot(normal(p), rest_normal[t as usize]) <= 0.0 {
                turned.push(t);
            }
        }
        (summarise(mins, &maxs, turned.len()), turned)
    });
    let mut any_turned = vec![false; triangles.len()];
    for (_, turned) in &per_shape {
        for &t in turned {
            any_turned[t as usize] = true;
        }
    }
    let worst = worst_angles(set, triangles);
    let worst_max: Vec<f64> = crate::map_range(triangles.len(), |t| {
        let mut m = 0.0f64;
        set.each(triangles[t], |_, p| {
            m = m.max(max_angle(*p));
            true
        });
        m
    });
    let all = summarise(worst, &worst_max, any_turned.iter().filter(|&&x| x).count());
    (per_shape.into_iter().map(|(s, _)| s).collect(), all)
}

#[derive(Clone, Serialize)]
pub struct QuadSettings {
    pub max_fold_deg: f64,
    pub min_corner_deg: f64,
}

impl Default for QuadSettings {
    fn default() -> Self {
        QuadSettings { max_fold_deg: 10.0, min_corner_deg: 45.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Face {
    Triangle(u32),
    Quad([(u32, u8); 4]),
}

fn convex_corners(p: [[f64; 3]; 4], up: [f64; 3]) -> bool {
    (0..4).all(|i| {
        let (prev, here, next) = (p[(i + 3) % 4], p[i], p[(i + 1) % 4]);
        dot(cross(sub(here, prev), sub(next, here)), up) > 0.0
    })
}

pub fn quads(positions: &[[f64; 3]], triangles: &[[u32; 3]], uvs: &[[Uv; 3]], settings: &QuadSettings) -> Vec<Face> {
    let mut vtris: Vec<Vec<u32>> = vec![Vec::new(); positions.len()];
    for (t, tri) in triangles.iter().enumerate() {
        for &w in tri {
            vtris[w as usize].push(t as u32);
        }
    }
    let cos_fold = settings.max_fold_deg.to_radians().cos();
    let mut candidates: Vec<(f64, u32, u32, [(u32, u8); 4])> = Vec::new();
    for (t, tri) in triangles.iter().enumerate() {
        let t = t as u32;
        for e in 0..3 {
            let (a, b, c) = (tri[e], tri[(e + 1) % 3], tri[(e + 2) % 3]);
            let across: Vec<u32> = vtris[a as usize].iter().copied().filter(|&s| s != t && triangles[s as usize].contains(&b)).collect();
            let [s] = across[..] else { continue };
            if s < t {
                continue;
            }
            let st = triangles[s as usize];
            let Some(ds) = (0..3).find(|&k| st[k] != a && st[k] != b) else { continue };
            if st[(ds + 1) % 3] != b {
                continue;
            }
            let d = st[ds];
            let corner = |tt: u32, v: u32| triangles[tt as usize].iter().position(|&x| x == v).unwrap() as u8;
            let uv = |tt: u32, v: u32| uvs[tt as usize][corner(tt, v) as usize];
            if c == d || !uv_close(uv(t, a), uv(s, a)) || !uv_close(uv(t, b), uv(s, b)) {
                continue;
            }
            let p = [a, d, b, c].map(|v| positions[v as usize]);
            let (n1, n2) = (normal([p[0], p[2], p[3]]), normal([p[2], p[0], p[1]]));
            let (l1, l2) = (length(n1), length(n2));
            if !(l1 > 0.0 && l2 > 0.0 && dot(n1, n2) > cos_fold * l1 * l2) {
                continue;
            }
            let up = [n1[0] + n2[0], n1[1] + n2[1], n1[2] + n2[2]];
            if !convex_corners(p, up) {
                continue;
            }
            let q = [uv(t, a), uv(s, d), uv(t, b), uv(t, c)].map(|u| [u[0] as f64, u[1] as f64, 0.0]);
            let uv_up = [0.0, 0.0, uv_area(uvs[t as usize]).signum()];
            if uv_up[2] == 0.0 || !convex_corners(q, uv_up) {
                continue;
            }
            let angles = [0, 1, 2, 3].map(|i| angle_at(p[i], p[(i + 1) % 4], p[(i + 3) % 4]));
            if angles.iter().any(|&x| x < settings.min_corner_deg || x > 180.0 - settings.min_corner_deg) {
                continue;
            }
            let score = angles.iter().map(|&x| (x - 90.0).abs()).fold(0.0, f64::max);
            candidates.push((score, t, s, [(t, corner(t, a)), (s, corner(s, d)), (t, corner(t, b)), (t, corner(t, c))]));
        }
    }
    candidates.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)).then(x.2.cmp(&y.2)));
    let mut partner: Vec<Option<(u32, [(u32, u8); 4])>> = vec![None; triangles.len()];
    for (_, t, s, corners) in candidates {
        if partner[t as usize].is_some() || partner[s as usize].is_some() {
            continue;
        }
        partner[t as usize] = Some((s, corners));
        partner[s as usize] = Some((t, corners));
    }
    let mut faces = Vec::with_capacity(triangles.len());
    for t in 0..triangles.len() as u32 {
        match partner[t as usize] {
            None => faces.push(Face::Triangle(t)),
            Some((s, corners)) if s > t => faces.push(Face::Quad(corners)),
            Some(_) => {}
        }
    }
    faces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(n: usize, sheared: bool) -> (Vec<[f64; 3]>, Vec<[u32; 3]>, Vec<[Uv; 3]>) {
        let mut points = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let (x, y) = (i as f64, j as f64);
                let (shear, rise) = if sheared { (0.9, 0.5) } else { (0.0, 1.0) };
                points.push([(x + shear * y) * 0.001, y * rise * 0.001, 0.0]);
            }
        }
        let mut tris = Vec::new();
        let mut uvs = Vec::new();
        let uv = |p: [f64; 3]| [(p[0] * 100.0) as f32, (p[1] * 100.0) as f32];
        for j in 0..n - 1 {
            for i in 0..n - 1 {
                let a = (j * n + i) as u32;
                let (b, c, d) = (a + 1, a + n as u32 + 1, a + n as u32);
                for t in [[a, b, c], [a, c, d]] {
                    tris.push(t);
                    uvs.push(t.map(|v| uv(points[v as usize])));
                }
            }
        }
        (points, tris, uvs)
    }

    fn worst(set: &ShapeSet, tris: &[[u32; 3]]) -> f64 {
        worst_angles(set, tris).into_iter().fold(180.0, f64::min)
    }

    fn mesh_is_sound(tris: &[[u32; 3]], n: usize) {
        let mut edges = std::collections::HashMap::new();
        for t in tris {
            assert!(t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
            for i in 0..3 {
                *edges.entry((t[i], t[(i + 1) % 3])).or_insert(0) += 1;
            }
        }
        assert!(edges.values().all(|&c| c == 1), "a directed edge repeats: orientation or manifoldness broke");
        assert!(tris.iter().flatten().all(|&v| (v as usize) < n));
    }

    #[test]
    fn flips_raise_the_worst_angle_and_keep_the_mesh_sound() {
        let (p, tris, uvs) = sheet(8, true);
        let set = ShapeSet::new(p.clone());
        let before = worst(&set, &tris);
        let out = improve(&set, &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert!(out.flips > 0 && out.converged);
        let after = worst(&set, &out.triangles);
        assert!(after > before + 5.0, "worst {before} -> {after}");
        mesh_is_sound(&out.triangles, p.len());
        for (t, tri) in out.triangles.iter().enumerate() {
            let expect = tri.map(|v| [(p[v as usize][0] * 100.0) as f32, (p[v as usize][1] * 100.0) as f32]);
            assert_eq!(out.triangle_uvs[t], expect, "a corner lost its texture coordinate");
            assert!(uv_area(out.triangle_uvs[t]) > 0.0);
        }
    }

    #[test]
    fn the_result_is_deterministic() {
        let (p, tris, uvs) = sheet(9, true);
        let mut set = ShapeSet::new(p.clone());
        let bent: Vec<[f64; 3]> = p.iter().map(|q| [q[0], q[1], 0.3 * q[0] * q[0] / 0.001]).collect();
        set.add_positions("bent", &bent).unwrap();
        let a = improve(&set, &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        let b = improve(&set, &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert_eq!(a.triangles, b.triangles);
        assert_eq!(a.triangle_uvs, b.triangle_uvs);
        assert_eq!(shape_stats(&set, &a.triangles), shape_stats(&set, &b.triangles));
    }

    #[test]
    fn a_seam_edge_is_never_flipped() {
        let (p, tris, mut uvs) = sheet(6, true);
        for (t, tri) in tris.iter().enumerate() {
            for k in 0..3 {
                if tri[k] % 6 >= 3 {
                    uvs[t][k][1] += 0.5 * (t % 2) as f32;
                }
            }
        }
        let set = ShapeSet::new(p.clone());
        let out = improve(&set, &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert!(out.flips > 0);
        let uv_of = |tt: &[[u32; 3]], uu: &[[Uv; 3]], t: usize, v: u32| uu[t][tt[t].iter().position(|&x| x == v).unwrap()];
        let edges = |tt: &[[u32; 3]]| -> std::collections::HashMap<(u32, u32), Vec<usize>> {
            let mut m: std::collections::HashMap<(u32, u32), Vec<usize>> = std::collections::HashMap::new();
            for (t, tri) in tt.iter().enumerate() {
                for k in 0..3 {
                    let (a, b) = (tri[k], tri[(k + 1) % 3]);
                    m.entry((a.min(b), a.max(b))).or_default().push(t);
                }
            }
            m
        };
        let after = edges(&out.triangles);
        let mut seams = 0;
        for ((a, b), ts) in edges(&tris) {
            if ts.len() == 2 && [a, b].iter().any(|&v| !uv_close(uv_of(&tris, &uvs, ts[0], v), uv_of(&tris, &uvs, ts[1], v))) {
                seams += 1;
                assert!(after.contains_key(&(a, b)), "seam edge {a}-{b} was flipped");
            }
        }
        assert!(seams > 10);
    }

    #[test]
    fn the_open_boundary_is_never_flipped_and_a_lone_triangle_stays() {
        let p = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 0.01, 0.0]];
        let tris = vec![[0, 1, 2]];
        let uvs = vec![[[0.0, 0.0], [1.0, 0.0], [0.5, 0.01]]];
        let out = improve(&ShapeSet::new(p), &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert_eq!(out.triangles, tris);
        assert_eq!(out.rounds[0].census.boundary, 3);
    }

    #[test]
    fn a_shape_that_would_fold_the_new_diagonal_refuses_the_flip() {
        let p = vec![[0.0, 0.0, 0.0], [0.004, 0.0, 0.0], [0.002, 0.0002, 0.0], [0.002, -0.003, 0.0]];
        let tris = vec![[0, 1, 2], [1, 0, 3]];
        let uvs: Vec<[Uv; 3]> = tris.iter().map(|t| t.map(|v| [p[v as usize][0] as f32 * 100.0, p[v as usize][1] as f32 * 100.0])).collect();
        let flat = improve(&ShapeSet::new(p.clone()), &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert_eq!(flat.flips, 1);
        let mut set = ShapeSet::new(p.clone());
        set.add_deltas("ridge", &[2], &[[0.0, 0.0, 0.003]]).unwrap();
        let out = improve(&set, &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert_eq!(out.flips, 0);
        assert_eq!(out.rounds[0].census.fold, 1);
    }

    #[test]
    fn a_flip_that_helps_the_rest_but_hurts_a_shape_is_refused() {
        let p = vec![[0.0, 0.0, 0.0], [0.004, 0.0, 0.0], [0.002, 0.0004, 0.0], [0.002, -0.003, 0.0]];
        let tris = vec![[0, 1, 2], [1, 0, 3]];
        let uvs: Vec<[Uv; 3]> = tris.iter().map(|t| t.map(|v| [p[v as usize][0] as f32 * 100.0, p[v as usize][1] as f32 * 100.0])).collect();
        let mut set = ShapeSet::new(p.clone());
        assert_eq!(improve(&ShapeSet::new(p.clone()), &tris, &uvs, &FlipSettings::default(), &mut |_| {}).flips, 1);
        set.add_deltas("pinch", &[2], &[[0.0, -0.003, 0.0]]).unwrap();
        let out = improve(&set, &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert_eq!(out.flips, 0, "{:?}", out.triangles);
    }

    #[test]
    fn a_crease_in_the_rest_normals_keeps_its_cut() {
        let p = vec![[0.0, 0.0, 0.0], [0.004, 0.0, 0.0], [0.002, 0.0002, 0.0], [0.002, -0.003, 0.0]];
        let tris = vec![[0, 1, 2], [1, 0, 3]];
        let uvs: Vec<[Uv; 3]> = tris.iter().map(|t| t.map(|v| [p[v as usize][0] as f32 * 100.0, p[v as usize][1] as f32 * 100.0])).collect();
        let mut set = ShapeSet::new(p.clone());
        set.set_normals(vec![[0.0, 0.0, 2.0], [0.0, 0.0, 1.0], [0.0, 0.3, 1.0], [0.0, -0.3, 1.0]]).unwrap();
        assert_eq!(improve(&set, &tris, &uvs, &FlipSettings::default(), &mut |_| {}).flips, 1);
        let mut set = ShapeSet::new(p.clone());
        set.set_normals(vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.8], [0.0, 0.0, 1.0]]).unwrap();
        let out = improve(&set, &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert_eq!(out.flips, 0);
        assert_eq!(out.rounds[0].census.crease, 1);
        assert!(set.set_normals(vec![[0.0; 3]; 3]).is_err());
    }

    #[test]
    fn a_flip_that_would_turn_a_texture_triangle_over_is_refused() {
        let p = vec![[0.0, 0.0, 0.0], [0.004, 0.0, 0.0], [0.002, 0.0002, 0.0], [0.002, -0.003, 0.0]];
        let tris = vec![[0, 1, 2], [1, 0, 3]];
        let uv: [Uv; 4] = [[0.0, 0.0], [0.4, 0.0], [0.2, 0.02], [0.6, -0.01]];
        let uvs: Vec<[Uv; 3]> = tris.iter().map(|t| t.map(|v| uv[v as usize])).collect();
        assert!(uv_area(uvs[0]) > 0.0 && uv_area(uvs[1]) > 0.0);
        let out = improve(&ShapeSet::new(p), &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert_eq!(out.flips, 0);
        assert_eq!(out.rounds[0].census.uv_flip, 1);
    }

    #[test]
    fn a_shape_where_the_new_pair_would_turn_over_more_than_the_old_one_refuses_the_flip() {
        let p = vec![[0.0, 0.0, 0.0], [0.004, 0.0, 0.0], [0.002, 0.0003, 0.0], [0.002, -0.003, 0.0]];
        let tris = vec![[0, 1, 2], [1, 0, 3]];
        let uvs: Vec<[Uv; 3]> = tris.iter().map(|t| t.map(|v| [p[v as usize][0] as f32 * 100.0, p[v as usize][1] as f32 * 100.0])).collect();
        assert_eq!(improve(&ShapeSet::new(p.clone()), &tris, &uvs, &FlipSettings::default(), &mut |_| {}).flips, 1);
        let mut set = ShapeSet::new(p.clone());
        set.add_deltas("lift", &[3], &[[0.0, 0.0045, 0.0]]).unwrap();
        let out = improve(&set, &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert_eq!(out.flips, 0);
        assert_eq!(out.rounds[0].census.turned, 1);
    }

    #[test]
    fn over_rough_shapes_no_shape_gains_a_turned_triangle_and_the_worst_never_drops() {
        let (p, tris, uvs) = sheet(10, true);
        let mut set = ShapeSet::new(p.clone());
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut unit = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64 - 0.5
        };
        for s in 0..12 {
            let size = 0.00008 * (1 + s % 4) as f64;
            let moved: Vec<[f64; 3]> = p.iter().map(|q| [q[0] + size * unit(), q[1] + size * unit(), q[2] + 2.0 * size * unit()]).collect();
            set.add_positions(&format!("rough{s}"), &moved).unwrap();
        }
        let out = improve(&set, &tris, &uvs, &FlipSettings::default(), &mut |_| {});
        assert!(out.flips > 0);
        let (before, all_before) = shape_stats(&set, &tris);
        let (after, all_after) = shape_stats(&set, &out.triangles);
        for (b, a) in before.iter().zip(&after) {
            assert!(a.turned <= b.turned, "turned {} -> {}", b.turned, a.turned);
        }
        assert!(all_after.min_angle_worst >= all_before.min_angle_worst);
        mesh_is_sound(&out.triangles, p.len());
    }

    #[test]
    fn the_shape_walk_visits_rest_and_each_touching_shape_once() {
        let p = vec![[0.0; 3]; 4];
        let mut set = ShapeSet::new(p);
        set.add_deltas("a", &[0, 2], &[[1.0, 0.0, 0.0], [2.0, 0.0, 0.0]]).unwrap();
        set.add_deltas("b", &[3], &[[0.0, 1.0, 0.0]]).unwrap();
        set.add_deltas("c", &[1, 1], &[[0.0, 0.0, 1.0], [0.0, 0.0, -1.0]]).unwrap();
        let mut seen = Vec::new();
        set.each([0, 1, 2], |s, q| {
            seen.push((s, q[0][0], q[2][0]));
            true
        });
        assert_eq!(seen, vec![(0, 0.0, 0.0), (1, 1.0, 2.0)]);
        assert_eq!(set.entries(), 3);
        assert_eq!(set.position(2, 1), [2.0, 0.0, 0.0]);
        assert_eq!(set.position(2, 2), [0.0; 3]);
        assert!(set.add_deltas("bad", &[9], &[[0.0; 3]]).is_err());
    }

    #[test]
    fn stats_count_slivers_and_turned_triangles_per_shape() {
        let p = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.05, 0.0]];
        let tris = vec![[0, 1, 2], [1, 3, 2]];
        let mut set = ShapeSet::new(p);
        set.add_deltas("over", &[3], &[[-2.0, 0.0, 0.0]]).unwrap();
        let (per, all) = shape_stats(&set, &tris);
        assert_eq!(per.len(), 2);
        assert_eq!(per[0].below_5, 1);
        assert_eq!(per[0].turned, 0);
        assert_eq!(per[1].turned, 1);
        assert_eq!(all.turned, 1);
        assert!((per[0].min_angle_worst - min_angle([[1.0, 0.0, 0.0], [1.0, 0.05, 0.0], [0.0, 1.0, 0.0]])).abs() < 1e-12);
    }

    #[test]
    fn quads_pair_a_clean_grid_and_refuse_seams_and_folds() {
        let (p, tris, uvs) = sheet(5, false);
        let faces = quads(&p, &tris, &uvs, &QuadSettings::default());
        let q = faces.iter().filter(|f| matches!(f, Face::Quad(_))).count();
        let t = faces.iter().filter(|f| matches!(f, Face::Triangle(_))).count();
        assert_eq!(2 * q + t, tris.len());
        assert_eq!(q, 16);
        for f in &faces {
            if let Face::Quad(c) = f {
                let v = c.map(|(tt, k)| tris[tt as usize][k as usize]);
                let mut s = v.to_vec();
                s.sort_unstable();
                s.dedup();
                assert_eq!(s.len(), 4);
            }
        }
        let mut seam = uvs.clone();
        for u in seam.iter_mut().skip(1).step_by(2) {
            for c in u.iter_mut() {
                c[1] += 0.25;
            }
        }
        assert!(quads(&p, &tris, &seam, &QuadSettings::default()).iter().all(|f| matches!(f, Face::Triangle(_))));
        let folded: Vec<[f64; 3]> = p.iter().enumerate().map(|(i, q)| [q[0], q[1], if (i % 5 + i / 5) % 2 == 1 { 0.0008 } else { 0.0 }]).collect();
        for f in quads(&folded, &tris, &uvs, &QuadSettings::default()) {
            if let Face::Quad(c) = f {
                let (t, s) = (c[0].0, c[1].0);
                assert!(t / 2 != s / 2, "the folded diagonal pair {t}, {s} became a quad");
                let (n1, n2) = (normal(tris[t as usize].map(|v| folded[v as usize])), normal(tris[s as usize].map(|v| folded[v as usize])));
                assert!(dot(n1, n2) > (10.0f64).to_radians().cos() * length(n1) * length(n2));
            }
        }
    }
}
