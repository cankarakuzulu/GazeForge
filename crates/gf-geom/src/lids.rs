use serde::{Deserialize, Serialize};

use crate::decimate::Decimated;
use crate::placement as pl;
use crate::quadric::{cross, dot, length, sub};

#[derive(Deserialize)]
struct EyeFile {
    eyes: Vec<EyeEntry>,
}

#[derive(Deserialize)]
struct EyeEntry {
    centre: [f64; 3],
    radius: f64,
    gaze: [f64; 3],
    vertices: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
    #[serde(default)]
    parts: Vec<crate::eyeball::Part>,
    #[serde(default)]
    uvs: Vec<[f64; 2]>,
}

const SURFACE_PARTS: [&str; 2] = ["sclera", "cornea"];

impl EyeEntry {
    fn surface(&self) -> Vec<[u32; 3]> {
        if self.parts.is_empty() {
            return self.triangles.clone();
        }
        self.parts
            .iter()
            .filter(|p| SURFACE_PARTS.contains(&p.name.as_str()))
            .flat_map(|p| self.triangles.iter().skip(p.first_triangle).take(p.triangles).copied())
            .collect()
    }
}

const POLAR_STEPS: usize = 180;
const AZIMUTH_STEPS: usize = 360;

#[derive(Clone)]
pub struct Eye {
    pub centre: [f64; 3],
    pub radius: f64,
    gaze: [f64; 3],
    side: [f64; 3],
    up: [f64; 3],
    table: Vec<f64>,
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub uvs: Vec<[f64; 2]>,
    pub parts: Vec<crate::eyeball::Part>,
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let l = length(v).max(1e-300);
    v.map(|x| x / l)
}

fn outermost(origin: [f64; 3], dir: [f64; 3], vertices: &[[f64; 3]], triangles: &[[u32; 3]]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for tri in triangles {
        let (a, b, c) = (vertices[tri[0] as usize], vertices[tri[1] as usize], vertices[tri[2] as usize]);
        let (e1, e2) = (sub(b, a), sub(c, a));
        let p = cross(dir, e2);
        let det = dot(e1, p);
        if det.abs() < 1e-18 {
            continue;
        }
        let inv = 1.0 / det;
        let s = sub(origin, a);
        let u = dot(s, p) * inv;
        if !(-1e-9..=1.0 + 1e-9).contains(&u) {
            continue;
        }
        let q = cross(s, e1);
        let v = dot(dir, q) * inv;
        if v < -1e-9 || u + v > 1.0 + 1e-9 {
            continue;
        }
        let t = dot(e2, q) * inv;
        if t > 0.0 && best.is_none_or(|b| t > b) {
            best = Some(t);
        }
    }
    best
}

impl Eye {
    fn new(entry: &EyeEntry) -> Result<Eye, String> {
        let finite = |v: &[f64]| v.iter().all(|x| x.is_finite());
        if !finite(&entry.centre) || !finite(&entry.gaze) || !entry.radius.is_finite() || entry.radius <= 0.0 {
            return Err("an eye has a non-finite centre or gaze, or a radius that is not positive".into());
        }
        if entry.vertices.iter().any(|v| !finite(v)) || entry.triangles.is_empty() {
            return Err("an eye has non-finite vertices or no triangles".into());
        }
        if entry.triangles.iter().flatten().any(|&i| i as usize >= entry.vertices.len()) {
            return Err("an eye triangle indexes past its vertices".into());
        }
        if length(entry.gaze) < 1e-9 {
            return Err("an eye gaze has zero length".into());
        }
        let gaze = unit(entry.gaze);
        let helper = if gaze[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        let side = unit(cross(gaze, helper));
        let up = cross(gaze, side);
        let mut eye = Eye { centre: entry.centre, radius: entry.radius, gaze, side, up, table: Vec::new(), vertices: entry.vertices.clone(), triangles: entry.triangles.clone(), uvs: entry.uvs.clone(), parts: entry.parts.clone() };
        let front: Vec<[u32; 3]> = entry.surface().into_iter().filter(|t| t.iter().any(|&i| dot(sub(entry.vertices[i as usize], entry.centre), gaze) >= 0.0)).collect();
        let mut table = vec![0.0; (POLAR_STEPS + 1) * AZIMUTH_STEPS];
        for i in 0..=POLAR_STEPS {
            for j in 0..AZIMUTH_STEPS {
                let dir = eye.direction(i as f64 * 90.0 / POLAR_STEPS as f64, j as f64 * 360.0 / AZIMUTH_STEPS as f64);
                table[i * AZIMUTH_STEPS + j] = outermost(entry.centre, dir, &entry.vertices, &front).unwrap_or(entry.radius);
            }
        }
        eye.table = table;
        Ok(eye)
    }

    pub fn gaze(&self) -> [f64; 3] {
        self.gaze
    }

    pub fn vertex_normals(&self) -> Vec<[f64; 3]> {
        let mut n = vec![[0.0f64; 3]; self.vertices.len()];
        for t in &self.triangles {
            let [a, b, c] = t.map(|i| self.vertices[i as usize]);
            let f = cross(sub(b, a), sub(c, a));
            for &i in t {
                n[i as usize] = [0, 1, 2].map(|k| n[i as usize][k] + f[k]);
            }
        }
        self.vertices
            .iter()
            .zip(n)
            .map(|(p, m)| {
                let out = sub(*p, self.centre);
                let l = length(m);
                if l <= 0.0 {
                    return unit(out);
                }
                let s = if dot(m, out) < 0.0 { -1.0 / l } else { 1.0 / l };
                m.map(|x| x * s)
            })
            .collect()
    }

    pub fn transformed(&self, scale: f64, rotation: [[f64; 3]; 3], translation: [f64; 3]) -> Eye {
        let rotate = |v: [f64; 3]| [0, 1, 2].map(|i| dot(rotation[i], v));
        let place = |p: [f64; 3]| {
            let r = rotate(p);
            [0, 1, 2].map(|k| scale * r[k] + translation[k])
        };
        Eye {
            centre: place(self.centre),
            radius: self.radius * scale,
            gaze: rotate(self.gaze),
            side: rotate(self.side),
            up: rotate(self.up),
            table: self.table.iter().map(|t| t * scale).collect(),
            vertices: self.vertices.iter().map(|&p| place(p)).collect(),
            triangles: self.triangles.clone(),
            uvs: self.uvs.clone(),
            parts: self.parts.clone(),
        }
    }

    fn direction(&self, polar_deg: f64, azimuth_deg: f64) -> [f64; 3] {
        let (p, a) = (polar_deg.to_radians(), azimuth_deg.to_radians());
        [0, 1, 2].map(|k| self.gaze[k] * p.cos() + (self.side[k] * a.cos() + self.up[k] * a.sin()) * p.sin())
    }

    pub fn polar_deg(&self, dir: [f64; 3]) -> f64 {
        dot(dir, self.gaze).clamp(-1.0, 1.0).acos().to_degrees()
    }

    pub fn surface(&self, dir: [f64; 3]) -> Option<f64> {
        let polar = self.polar_deg(dir);
        if polar > 90.0 {
            return None;
        }
        let azimuth = dot(dir, self.up).atan2(dot(dir, self.side)).to_degrees().rem_euclid(360.0);
        let fi = polar / 90.0 * POLAR_STEPS as f64;
        let fj = azimuth / 360.0 * AZIMUTH_STEPS as f64;
        let (i0, j0) = ((fi.floor() as usize).min(POLAR_STEPS - 1), fj.floor() as usize % AZIMUTH_STEPS);
        let (ti, tj) = (fi - i0 as f64, fj - fj.floor());
        let j1 = (j0 + 1) % AZIMUTH_STEPS;
        let at = |i: usize, j: usize| self.table[i * AZIMUTH_STEPS + j];
        let low = at(i0, j0) * (1.0 - tj) + at(i0, j1) * tj;
        let high = at(i0 + 1, j0) * (1.0 - tj) + at(i0 + 1, j1) * tj;
        Some(low * (1.0 - ti) + high * ti)
    }
}

pub fn eye_from_mesh(centre: [f64; 3], radius: f64, gaze: [f64; 3], mesh: &crate::eyeball::EyeballMesh) -> Result<Eye, String> {
    Eye::new(&EyeEntry { centre, radius, gaze, vertices: mesh.positions.clone(), triangles: mesh.triangles.clone(), parts: mesh.parts.clone(), uvs: mesh.uvs.clone() })
}

pub fn eyes_from_json(text: &str) -> Result<Vec<Eye>, String> {
    let file: EyeFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if file.eyes.is_empty() {
        return Err("no eyes".into());
    }
    file.eyes.iter().map(Eye::new).collect::<Result<Vec<_>, _>>()
}

#[derive(Clone, Serialize)]
pub struct CollideSettings {
    pub clearance: f64,
    pub front_deg: f64,
    pub smooth_iterations: usize,
}

#[derive(Default, Serialize)]
pub struct CollideStats {
    pub inside_before: usize,
    pub deepest_mm: f64,
    pub pushed: usize,
    pub smoothed: usize,
    pub inside_after: usize,
}

fn violation(eye: &Eye, p: [f64; 3], normal: [f64; 3], clearance: f64, front_deg: f64) -> Option<([f64; 3], f64)> {
    let offset = sub(p, eye.centre);
    let d = length(offset);
    if d < 0.5 * eye.radius || d > 1.3 * eye.radius {
        return None;
    }
    let dir = offset.map(|x| x / d);
    if eye.polar_deg(dir) > front_deg || dot(normal, dir) <= 0.0 {
        return None;
    }
    let need = eye.surface(dir)? + clearance - d;
    Some((dir, need))
}

pub fn collide_dense(positions: &[[f64; 3]], triangles: &[[u32; 3]], eyes: &[Eye], settings: &CollideSettings) -> (Vec<[f64; 3]>, CollideStats) {
    collide_dense_forced(positions, triangles, eyes, settings, &[])
}

pub fn push_out(eyes: &[Eye], p: [f64; 3], front_deg: f64, clearance: f64) -> Option<[f64; 3]> {
    let mut best: Option<(f64, [f64; 3])> = None;
    for eye in eyes {
        let offset = sub(p, eye.centre);
        let d = length(offset);
        if d < 0.5 * eye.radius || d > 1.3 * eye.radius {
            continue;
        }
        let dir = offset.map(|x| x / d);
        if eye.polar_deg(dir) > front_deg {
            continue;
        }
        let reach = eye.surface(dir).unwrap_or(eye.radius).max(eye.radius) + clearance;
        if reach > d && best.is_none_or(|b| reach - d > b.0) {
            best = Some((reach - d, [0, 1, 2].map(|k| eye.centre[k] + dir[k] * reach)));
        }
    }
    best.map(|b| b.1)
}

pub fn collide_dense_forced(
    positions: &[[f64; 3]],
    triangles: &[[u32; 3]],
    eyes: &[Eye],
    settings: &CollideSettings,
    forced: &[u32],
) -> (Vec<[f64; 3]>, CollideStats) {
    let normals = pl::angle_weighted_normals(positions, triangles);
    let mut out = positions.to_vec();
    let mut stats = CollideStats::default();
    let n = positions.len();
    let mut force = vec![false; n];
    for &i in forced {
        if let Some(f) = force.get_mut(i as usize) {
            *f = true;
        }
    }
    let near: Vec<bool> = positions.iter().map(|&p| eyes.iter().any(|e| length(sub(p, e.centre)) < 1.4 * e.radius)).collect();
    let mut displacement = vec![[0.0f64; 3]; n];
    let mut limit: Vec<Option<(usize, f64)>> = vec![None; n];
    for (i, &p) in positions.iter().enumerate() {
        if !near[i] {
            continue;
        }
        for (k, eye) in eyes.iter().enumerate() {
            let normal = if force[i] { sub(p, eye.centre) } else { normals[i] };
            if let Some((dir, need)) = violation(eye, p, normal, settings.clearance, settings.front_deg) {
                if need > settings.clearance {
                    stats.inside_before += 1;
                }
                if need > 0.0 {
                    stats.deepest_mm = stats.deepest_mm.max(need * 1000.0);
                    stats.pushed += 1;
                    displacement[i] = dir.map(|x| x * need);
                    limit[i] = Some((k, need + length(sub(p, eye.centre))));
                }
            }
        }
    }
    if settings.smooth_iterations > 0 && stats.pushed > 0 {
        let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); n];
        for tri in triangles {
            if !tri.iter().any(|&v| near[v as usize]) {
                continue;
            }
            for e in 0..3 {
                let (a, b) = (tri[e], tri[(e + 1) % 3]);
                if !neighbours[a as usize].contains(&b) {
                    neighbours[a as usize].push(b);
                    neighbours[b as usize].push(a);
                }
            }
        }
        let mut active: Vec<bool> = limit.iter().map(|l| l.is_some()).collect();
        for _ in 0..settings.smooth_iterations {
            let grown: Vec<usize> = (0..n).filter(|&i| !active[i] && near[i] && neighbours[i].iter().any(|&j| active[j as usize])).collect();
            for i in grown {
                active[i] = true;
            }
            let next: Vec<[f64; 3]> = (0..n)
                .map(|i| {
                    if !active[i] {
                        return displacement[i];
                    }
                    let mut s = displacement[i];
                    for &j in &neighbours[i] {
                        for k in 0..3 {
                            s[k] += displacement[j as usize][k];
                        }
                    }
                    s.map(|x| x / (1 + neighbours[i].len()) as f64)
                })
                .collect();
            displacement = next;
            for i in 0..n {
                if let Some((k, reach)) = limit[i] {
                    let moved = [0, 1, 2].map(|c| positions[i][c] + displacement[i][c]);
                    let offset = sub(moved, eyes[k].centre);
                    let d = length(offset);
                    if d < reach {
                        let target = [0, 1, 2].map(|c| eyes[k].centre[c] + offset[c] / d * reach);
                        displacement[i] = sub(target, positions[i]);
                    }
                }
            }
        }
        stats.smoothed = active.iter().filter(|&&a| a).count();
    }
    for i in 0..n {
        out[i] = [0, 1, 2].map(|c| positions[i][c] + displacement[i][c]);
    }
    let after = pl::angle_weighted_normals(&out, triangles);
    stats.inside_after = (0..n)
        .filter(|&i| near[i] && eyes.iter().any(|e| violation(e, out[i], after[i], 0.0, settings.front_deg).is_some_and(|(_, need)| need > 1e-7)))
        .count();
    (out, stats)
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LidCheck {
    pub vertices: usize,
    pub candidates: usize,
    pub inside: usize,
    pub inside_per_eye: Vec<usize>,
    pub deepest_mm: f64,
    pub worst_vertex: Option<usize>,
    #[serde(skip)]
    pub inside_vertices: Vec<usize>,
    pub front_deg: f64,
    pub tolerance_mm: f64,
    pub pass: bool,
}

pub fn lid_check(points: &[[f64; 3]], normals: &[[f64; 3]], eyes: &[Eye], front_deg: f64, tolerance: f64) -> LidCheck {
    let mut c = LidCheck {
        vertices: points.len(),
        inside_per_eye: vec![0; eyes.len()],
        front_deg,
        tolerance_mm: tolerance * 1000.0,
        ..Default::default()
    };
    for (i, (&p, &n)) in points.iter().zip(normals).enumerate() {
        let mut worst: Option<(usize, f64)> = None;
        let mut candidate = false;
        for (k, eye) in eyes.iter().enumerate() {
            let offset = sub(p, eye.centre);
            let d = length(offset);
            if d < 0.5 * eye.radius || d > 1.3 * eye.radius {
                continue;
            }
            let dir = offset.map(|x| x / d);
            if eye.polar_deg(dir) > front_deg || dot(n, dir) <= 0.0 {
                continue;
            }
            candidate = true;
            let need = eye.surface(dir).unwrap_or(eye.radius).max(eye.radius) - d;
            if need > tolerance && worst.is_none_or(|w| need > w.1) {
                worst = Some((k, need));
            }
        }
        c.candidates += candidate as usize;
        if let Some((k, need)) = worst {
            c.inside += 1;
            c.inside_per_eye[k] += 1;
            c.inside_vertices.push(i);
            if need * 1000.0 > c.deepest_mm {
                c.deepest_mm = need * 1000.0;
                c.worst_vertex = Some(i);
            }
        }
    }
    c.pass = c.inside == 0;
    c
}

#[derive(Clone, Serialize)]
pub struct ClearSettings {
    pub along_gaze: bool,
    pub depth: [f64; 2],
    pub reach: f64,
    pub clearance: f64,
    pub front_deg: f64,
    pub rounds: usize,
    pub samples: usize,
}

#[derive(Default, Serialize)]
pub struct ClearStats {
    pub triangles_near: usize,
    pub short_before: usize,
    pub rounds: usize,
    pub lifted: usize,
    pub largest_lift_mm: f64,
    pub short_after: usize,
}

fn barycentric_grid(n: usize) -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    for i in 0..=n {
        for j in 0..=n - i {
            let (a, b) = (i as f64 / n as f64, j as f64 / n as f64);
            out.push([a, b, 1.0 - a - b]);
        }
    }
    out
}

pub struct PointGrid {
    cell: f64,
    cells: std::collections::HashMap<[i64; 3], Vec<u32>>,
}

impl PointGrid {
    pub fn new(points: &[[f64; 3]], keep: &[bool], cell: f64) -> PointGrid {
        let mut cells: std::collections::HashMap<[i64; 3], Vec<u32>> = std::collections::HashMap::new();
        for (i, p) in points.iter().enumerate() {
            if keep[i] {
                cells.entry(p.map(|x| (x / cell).floor() as i64)).or_default().push(i as u32);
            }
        }
        PointGrid { cell, cells }
    }

    pub fn envelope(&self, points: &[[f64; 3]], values: &[f64], p: [f64; 3], reach: f64) -> f64 {
        let c = p.map(|x| (x / self.cell).floor() as i64);
        let r = (reach / self.cell).ceil() as i64;
        let mut best = f64::INFINITY;
        for dx in -r..=r {
            for dy in -r..=r {
                for dz in -r..=r {
                    let Some(list) = self.cells.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) else { continue };
                    for &i in list {
                        let d = length(sub(points[i as usize], p));
                        if d <= reach {
                            best = best.min(values[i as usize] + d);
                        }
                    }
                }
            }
        }
        best
    }
}

pub fn dense_clearance(points: &[[f64; 3]], normals: &[[f64; 3]], eyes: &[Eye], front_deg: f64) -> Vec<f64> {
    points
        .iter()
        .zip(normals)
        .map(|(&p, &n)| {
            eyes.iter()
                .filter_map(|eye| {
                    let offset = sub(p, eye.centre);
                    let d = length(offset);
                    if d < 0.5 * eye.radius || d > 1.3 * eye.radius {
                        return None;
                    }
                    let dir = offset.map(|x| x / d);
                    if eye.polar_deg(dir) > front_deg || dot(n, dir) <= 0.0 {
                        return None;
                    }
                    Some(d - eye.surface(dir)?)
                })
                .fold(f64::INFINITY, f64::min)
        })
        .collect()
}

pub fn clear_game<F: Fn([f64; 3]) -> f64>(game: &mut [[f64; 3]], mesh: &Decimated, fixed: &[bool], eyes: &[Eye], floor: F, settings: &ClearSettings) -> ClearStats {
    let grid = barycentric_grid(settings.samples.max(1));
    let near: Vec<u32> = (0..mesh.triangles.len() as u32)
        .filter(|&t| mesh.triangles[t as usize].iter().any(|&g| eyes.iter().any(|e| length(sub(game[g as usize], e.centre)) < 1.4 * e.radius)))
        .collect();
    let mut stats = ClearStats { triangles_near: near.len(), ..Default::default() };
    let taper = |depth: f64| {
        let t = ((depth - settings.depth[0]) / (settings.depth[1] - settings.depth[0]).max(1e-12)).clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    };
    let shortfalls = |game: &[[f64; 3]]| -> Vec<(u32, usize, Vec<(f64, [f64; 3])>)> {
        let mut out = Vec::new();
        for &t in &near {
            let tri = mesh.triangles[t as usize];
            let p = tri.map(|g| game[g as usize]);
            let normal = unit(pl::triangle_normal(p));
            for (k, eye) in eyes.iter().enumerate() {
                let mut short = Vec::new();
                for b in &grid {
                    let s = [0, 1, 2].map(|c| b[0] * p[0][c] + b[1] * p[1][c] + b[2] * p[2][c]);
                    let offset = sub(s, eye.centre);
                    let d = length(offset);
                    if d < 0.5 * eye.radius || d > 1.3 * eye.radius {
                        continue;
                    }
                    let dir = offset.map(|x| x / d);
                    if eye.polar_deg(dir) > settings.front_deg || dot(normal, dir) <= 0.0 {
                        continue;
                    }
                    if let Some(surface) = eye.surface(dir) {
                        let need = (surface + floor(s).min(settings.clearance) - d) * taper(surface - d);
                        if need > 1e-7 {
                            short.push((need, *b));
                        }
                    }
                }
                if !short.is_empty() {
                    out.push((t, k, short));
                }
            }
        }
        out
    };
    let mut lifted = vec![false; game.len()];
    for round in 0..settings.rounds {
        let short = shortfalls(game);
        if round == 0 {
            stats.short_before = short.len();
        }
        if short.is_empty() {
            break;
        }
        stats.rounds = round + 1;
        let mut lift: Vec<(f64, usize)> = vec![(0.0, 0); game.len()];
        for (t, k, samples) in &short {
            let tri = mesh.triangles[*t as usize];
            for &(need, b) in samples {
                let top = (0..3).filter(|&c| !fixed[tri[c] as usize]).map(|c| b[c]).fold(0.0f64, f64::max);
                if top <= 0.0 {
                    continue;
                }
                for c in 0..3 {
                    let g = tri[c] as usize;
                    let share = need * b[c] / top;
                    if !fixed[g] && share > lift[g].0 {
                        lift[g] = (share, *k);
                    }
                }
            }
        }
        for g in 0..game.len() {
            let (amount, k) = lift[g];
            if amount <= 0.0 {
                continue;
            }
            let eye = &eyes[k];
            let radial = unit(sub(game[g], eye.centre));
            let (dir, step) = if settings.along_gaze {
                (eye.gaze, 1.05 * amount / dot(radial, eye.gaze).max(0.25))
            } else {
                (radial, 1.05 * amount)
            };
            game[g] = [0, 1, 2].map(|c| game[g][c] + dir[c] * step);
            lifted[g] = true;
            stats.largest_lift_mm = stats.largest_lift_mm.max(step * 1000.0);
        }
    }
    stats.short_after = shortfalls(game).len();
    stats.lifted = lifted.iter().filter(|&&l| l).count();
    stats
}

pub fn penetration_area(points: &[[f64; 3]], triangles: &[[u32; 3]], eyes: &[Eye], front_deg: f64, samples: usize) -> (f64, f64) {
    let grid = barycentric_grid(samples.max(1));
    let mut area = 0.0;
    let mut deepest = 0.0f64;
    for tri in triangles {
        let p = tri.map(|g| points[g as usize]);
        if !eyes.iter().any(|e| p.iter().any(|&q| length(sub(q, e.centre)) < 1.4 * e.radius)) {
            continue;
        }
        let n = pl::triangle_normal(p);
        let a = 0.5 * length(n);
        let normal = unit(n);
        let mut inside = 0usize;
        for b in &grid {
            let s = [0, 1, 2].map(|c| b[0] * p[0][c] + b[1] * p[1][c] + b[2] * p[2][c]);
            if eyes.iter().any(|eye| {
                let offset = sub(s, eye.centre);
                let d = length(offset);
                if d < 0.5 * eye.radius || d > 1.3 * eye.radius {
                    return false;
                }
                let dir = offset.map(|x| x / d);
                if eye.polar_deg(dir) > front_deg || dot(normal, dir) <= 0.0 {
                    return false;
                }
                match eye.surface(dir) {
                    Some(surface) if surface > d => {
                        deepest = deepest.max(surface - d);
                        true
                    }
                    _ => false,
                }
            }) {
                inside += 1;
            }
        }
        area += a * inside as f64 / grid.len() as f64;
    }
    (area, deepest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere(radius: f64, rings: usize, segments: usize) -> EyeEntry {
        let mut vertices = vec![[0.0, 0.0, radius]];
        for r in 1..rings {
            let polar = std::f64::consts::PI * r as f64 / rings as f64;
            for s in 0..segments {
                let a = 2.0 * std::f64::consts::PI * s as f64 / segments as f64;
                vertices.push([radius * polar.sin() * a.cos(), radius * polar.sin() * a.sin(), radius * polar.cos()]);
            }
        }
        vertices.push([0.0, 0.0, -radius]);
        let last = vertices.len() as u32 - 1;
        let ring = |r: usize, s: usize| (1 + (r - 1) * segments + s % segments) as u32;
        let mut triangles = Vec::new();
        for s in 0..segments {
            triangles.push([0, ring(1, s), ring(1, s + 1)]);
            triangles.push([last, ring(rings - 1, s + 1), ring(rings - 1, s)]);
        }
        for r in 1..rings - 1 {
            for s in 0..segments {
                triangles.push([ring(r, s), ring(r + 1, s), ring(r + 1, s + 1)]);
                triangles.push([ring(r, s), ring(r + 1, s + 1), ring(r, s + 1)]);
            }
        }
        EyeEntry { centre: [0.0; 3], radius, gaze: [0.0, 0.0, 1.0], vertices, triangles, parts: Vec::new(), uvs: Vec::new() }
    }

    #[test]
    fn the_lid_check_counts_outward_front_vertices_inside_the_eye_only() {
        let eye = Eye::new(&sphere(0.012, 24, 48)).unwrap();
        let out = [0.0, 0.0, 1.0];
        let points = [[0.0, 0.0, 0.011], [0.0, 0.0, 0.011], [0.0, 0.0, 0.0125], [0.0108, 0.0, -0.002], [0.0, 0.004, 0.0105]];
        let normals = [out, [0.0, 0.0, -1.0], out, [1.0, 0.0, 0.0], unit([0.0, 0.3, 0.9])];
        let c = lid_check(&points, &normals, &[eye], 75.0, 0.0);
        assert_eq!(c.candidates, 3);
        assert_eq!(c.inside, 2);
        assert_eq!(c.worst_vertex, Some(0));
        assert!((c.deepest_mm - 1.0).abs() < 0.01, "{}", c.deepest_mm);
        assert!(!c.pass);
        let eye = Eye::new(&sphere(0.012, 24, 48)).unwrap();
        assert!(lid_check(&points[2..3], &normals[2..3], &[eye], 75.0, 0.0).pass);
    }

    #[test]
    fn the_surface_of_a_sphere_eye_is_its_radius() {
        let eye = Eye::new(&sphere(0.012, 24, 48)).unwrap();
        for dir in [[0.0, 0.0, 1.0], unit([0.3, 0.2, 0.9]), unit([1.0, 0.0, 0.2])] {
            let r = eye.surface(dir).unwrap();
            assert!(r > 0.0119 && r <= 0.012 + 1e-9, "surface {r} along {dir:?}");
        }
        assert!(eye.surface([0.0, 0.0, -1.0]).is_none());
    }

    #[test]
    fn a_transformed_eye_matches_one_built_from_the_transformed_mesh() {
        let mut entry = sphere(0.012, 24, 48);
        entry.centre = [0.003, -0.002, 0.001];
        entry.gaze = unit([0.2, -0.1, 1.0]);
        let eye = Eye::new(&entry).unwrap();
        let (a, s, t) = (0.35_f64, 1.17, [0.03, -0.12, 1.49]);
        let rotation = [[a.cos(), 0.0, a.sin()], [0.0, 1.0, 0.0], [-a.sin(), 0.0, a.cos()]];
        let moved = eye.transformed(s, rotation, t);
        let rot = |v: [f64; 3]| [0, 1, 2].map(|i| dot(rotation[i], v));
        let place = |p: [f64; 3]| {
            let r = rot(p);
            [0, 1, 2].map(|k| s * r[k] + t[k])
        };
        let fresh = Eye::new(&EyeEntry {
            centre: place(entry.centre),
            radius: entry.radius * s,
            gaze: rot(entry.gaze),
            vertices: entry.vertices.iter().map(|&p| place(p)).collect(),
            triangles: entry.triangles.clone(),
            parts: Vec::new(),
            uvs: Vec::new(),
        })
        .unwrap();
        assert_eq!(moved.centre, place(entry.centre));
        assert_eq!(moved.vertices[7], place(entry.vertices[7]));
        let g = moved.gaze();
        let mut worst: f64 = 0.0;
        for i in 0..40 {
            let (p, q) = (i as f64 * 0.037, i as f64 * 1.31);
            let side = unit(cross(g, [0.0, 0.0, 1.0]));
            let up = cross(g, side);
            let dir = unit([0, 1, 2].map(|k| g[k] * p.cos() + (side[k] * q.cos() + up[k] * q.sin()) * p.sin()));
            let (m, f) = (moved.surface(dir).unwrap(), fresh.surface(dir).unwrap());
            worst = worst.max((m - f).abs());
            let back = [0, 1, 2].map(|i| (0..3).map(|k| rotation[k][i] * dir[k]).sum::<f64>());
            assert!((m - s * eye.surface(back).unwrap()).abs() < 1e-12);
        }
        assert!(worst < 2e-5 * s, "transformed vs rebuilt surface differs by {worst} m");
    }

    #[test]
    fn bad_eyes_are_refused() {
        let mut entry = sphere(0.012, 8, 16);
        entry.radius = 0.0;
        assert!(Eye::new(&entry).is_err());
        let mut entry = sphere(0.012, 8, 16);
        entry.triangles.push([0, 1, 9999]);
        assert!(Eye::new(&entry).is_err());
    }

    #[test]
    fn skin_inside_the_eye_is_pushed_out_and_skin_outside_is_left() {
        let eye = Eye::new(&sphere(0.012, 24, 48)).unwrap();
        let z = |r: f64| r;
        let positions = vec![[-0.002, -0.002, z(0.0115)], [0.002, -0.002, z(0.0115)], [0.0, 0.003, z(0.0115)], [0.0, 0.0, 0.0125]];
        let triangles = vec![[0, 1, 2]];
        let settings = CollideSettings { clearance: 0.0002, front_deg: 75.0, smooth_iterations: 0 };
        let (out, stats) = collide_dense(&positions, &triangles, &[eye], &settings);
        assert_eq!(stats.pushed, 3);
        for p in &out[..3] {
            assert!(length(*p) >= 0.0119 + 0.0002 - 1e-9, "still inside: {}", length(*p));
        }
        assert_eq!(out[3], positions[3]);
    }

    #[test]
    fn a_game_triangle_cutting_into_the_eye_is_lifted_to_the_floor() {
        let eye = Eye::new(&sphere(0.012, 24, 48)).unwrap();
        let mesh = Decimated {
            kept: vec![0, 1, 2],
            owner: vec![0, 1, 2],
            triangles: vec![[0, 1, 2]],
            triangle_uvs: vec![[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]],
            stats: Default::default(),
        };
        let mut game = vec![[-0.004, -0.004, 0.0116], [0.004, -0.004, 0.0116], [0.0, 0.005, 0.0116]];
        let settings = ClearSettings { along_gaze: false, reach: 0.002, depth: [0.003, 0.006], clearance: 0.00005, front_deg: 75.0, rounds: 12, samples: 6 };
        let stats = clear_game(&mut game, &mesh, &[false; 3], &[eye], |_| f64::INFINITY, &settings);
        assert!(stats.short_before > 0 && stats.short_after == 0, "{} -> {}", stats.short_before, stats.short_after);
        let (area, _) = penetration_area(&game, &mesh.triangles, &[Eye::new(&sphere(0.012, 24, 48)).unwrap()], 75.0, 6);
        assert_eq!(area, 0.0);
    }

    #[test]
    fn a_floor_below_the_surface_lets_the_game_stay_as_deep_as_the_dense() {
        let eye = Eye::new(&sphere(0.012, 24, 48)).unwrap();
        let mesh = Decimated {
            kept: vec![0, 1, 2],
            owner: vec![0, 1, 2],
            triangles: vec![[0, 1, 2]],
            triangle_uvs: vec![[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]],
            stats: Default::default(),
        };
        let start = vec![[-0.004, -0.004, 0.0116], [0.004, -0.004, 0.0116], [0.0, 0.005, 0.0116]];
        let mut game = start.clone();
        let settings = ClearSettings { along_gaze: false, reach: 0.002, depth: [0.003, 0.006], clearance: 0.00005, front_deg: 75.0, rounds: 12, samples: 6 };
        let stats = clear_game(&mut game, &mesh, &[false; 3], &[eye], |_| -0.002, &settings);
        assert_eq!(stats.short_before, 0);
        assert_eq!(game, start);
    }
}
