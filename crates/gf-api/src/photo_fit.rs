use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Instant;

use gf_core::fit::{self, PerspectiveCamera, Point};
use gf_core::gates::{self, ShellLimits};
use gf_io::dense as dio;
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{obj, text};
use crate::error::{ApiError, Code};
use crate::photo::{self, Base, STABLE_LANDMARKS, num, opt, read_json, write_json};

const FAR: f32 = f32::INFINITY;
const FACE_OVAL: [usize; 36] = [10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365, 379, 378, 400, 377, 152, 148, 176, 149, 150, 136, 172, 58, 132, 93, 234, 127, 162, 21, 54, 103, 67, 109];
const FOCAL_PER_WIDTH: f64 = 70.0 / 36.0;
const RADII_PX: [f64; 6] = [40.0, 28.0, 20.0, 14.0, 10.0, 8.0];

struct Targets {
    ids: Vec<String>,
    indices: Vec<Vec<u32>>,
    deltas: Vec<Vec<[f32; 3]>>,
    cache: HashMap<usize, Vec<(usize, Point)>>,
}

impl Targets {
    fn load(base: &Base, catalog_ref: &str) -> Result<Targets, ApiError> {
        let (catalog, ids) = photo::head_ids(base, catalog_ref)?;
        let mut indices = Vec::with_capacity(ids.len());
        let mut deltas = Vec::with_capacity(ids.len());
        for id in &ids {
            let target = base.store.load_target(&catalog, id).map_err(ApiError::internal)?;
            let mut pairs: Vec<(u32, [f32; 3])> = target.indices.iter().copied().zip(target.deltas.iter().copied()).collect();
            if !pairs.windows(2).all(|w| w[0].0 < w[1].0) {
                pairs.sort_by_key(|p| p.0);
            }
            indices.push(pairs.iter().map(|p| p.0).collect());
            deltas.push(pairs.iter().map(|p| p.1).collect());
        }
        Ok(Targets { ids, indices, deltas, cache: HashMap::new() })
    }

    fn t(&self) -> usize {
        self.ids.len()
    }

    fn rows(&mut self, v: usize) -> &[(usize, Point)] {
        let (indices, deltas) = (&self.indices, &self.deltas);
        self.cache.entry(v).or_insert_with(|| {
            let mut out = Vec::new();
            for (j, idx) in indices.iter().enumerate() {
                if let Ok(k) = idx.binary_search(&(v as u32)) {
                    let d = deltas[j][k];
                    out.push((j, [d[0] as f64, d[1] as f64, d[2] as f64]));
                }
            }
            out
        })
    }

    fn evaluate(&self, base: &[Point], w: &[f64]) -> Vec<Point> {
        let mut x = base.to_vec();
        for (j, wj) in w.iter().enumerate() {
            if *wj != 0.0 {
                for (i, d) in self.indices[j].iter().zip(&self.deltas[j]) {
                    let p = &mut x[*i as usize];
                    for k in 0..3 {
                        p[k] += wj * d[k] as f64;
                    }
                }
            }
        }
        x
    }
}

struct Normal {
    t: usize,
    gram: Vec<f64>,
    rhs: Vec<f64>,
}

impl Normal {
    fn new(t: usize) -> Normal {
        Normal { t, gram: vec![0.0; t * t], rhs: vec![0.0; t] }
    }

    fn add(&mut self, a: &[(usize, f64)], b: f64, weight: f64) {
        for &(i, x) in a {
            self.rhs[i] += weight * x * b;
            for &(j, y) in a {
                self.gram[i * self.t + j] += weight * x * y;
            }
        }
    }

    fn row(rows: &[(usize, Point)], g: Point) -> Vec<(usize, f64)> {
        rows.iter().map(|(j, d)| (*j, fit::dot(g, *d))).collect()
    }
}

#[derive(Clone)]
enum Ob {
    Mark([f64; 2]),
    Edge([f64; 2], [f64; 2]),
}

struct View {
    name: String,
    role: String,
    size: [usize; 2],
    marks: Option<Vec<[f64; 2]>>,
    outline: Vec<[f64; 4]>,
    grid: Grid2,
    cam: PerspectiveCamera,
    left_out: bool,
    nominal: Option<[Point; 3]>,
}

struct Grid2 {
    cell: f64,
    cells: HashMap<(i64, i64), Vec<usize>>,
}

impl Grid2 {
    fn new(points: &[[f64; 4]], cell: f64) -> Grid2 {
        let mut cells: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, p) in points.iter().enumerate() {
            cells.entry(((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64)).or_default().push(i);
        }
        Grid2 { cell, cells }
    }

    fn nearest(&self, points: &[[f64; 4]], q: [f64; 2], normal: [f64; 2], radius: f64) -> Option<usize> {
        let reach = (radius / self.cell).ceil() as i64;
        let (cx, cy) = ((q[0] / self.cell).floor() as i64, (q[1] / self.cell).floor() as i64);
        let mut best: Option<(usize, f64)> = None;
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                if let Some(list) = self.cells.get(&(cx + dx, cy + dy)) {
                    for &i in list {
                        let p = points[i];
                        if p[2] * normal[0] + p[3] * normal[1] < 0.5 {
                            continue;
                        }
                        let d = (p[0] - q[0]).hypot(p[1] - q[1]);
                        if d <= radius && best.is_none_or(|b| d < b.1) {
                            best = Some((i, d));
                        }
                    }
                }
            }
        }
        best.map(|b| b.0)
    }
}

fn rodrigues(w: Point) -> [Point; 3] {
    let th = fit::norm(w);
    if th < 1e-15 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let k = fit::scale(w, 1.0 / th);
    let (s, c) = th.sin_cos();
    let v = 1.0 - c;
    [
        [c + k[0] * k[0] * v, k[0] * k[1] * v - k[2] * s, k[0] * k[2] * v + k[1] * s],
        [k[1] * k[0] * v + k[2] * s, c + k[1] * k[1] * v, k[1] * k[2] * v - k[0] * s],
        [k[2] * k[0] * v - k[1] * s, k[2] * k[1] * v + k[0] * s, c + k[2] * k[2] * v],
    ]
}

fn mat_mul(a: &[Point; 3], b: &[Point; 3]) -> [Point; 3] {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn solve_dense(mut a: Vec<f64>, mut b: Vec<f64>, n: usize) -> Option<Vec<f64>> {
    for col in 0..n {
        let pivot = (col..n).max_by(|x, y| a[x * n + col].abs().total_cmp(&a[y * n + col].abs()))?;
        if a[pivot * n + col].abs() < 1e-300 {
            return None;
        }
        if pivot != col {
            for k in 0..n {
                a.swap(col * n + k, pivot * n + k);
            }
            b.swap(col, pivot);
        }
        for row in col + 1..n {
            let f = a[row * n + col] / a[col * n + col];
            for k in col..n {
                a[row * n + k] -= f * a[col * n + k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = vec![0.0; n];
    for row in (0..n).rev() {
        let s: f64 = (row + 1..n).map(|k| a[row * n + k] * x[k]).sum();
        x[row] = (b[row] - s) / a[row * n + row];
    }
    Some(x)
}

fn rotation_error(r: &[Point; 3], nominal: &[Point; 3]) -> Point {
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = fit::dot(r[i], nominal[j]);
        }
    }
    let v = [m[2][1] - m[1][2], m[0][2] - m[2][0], m[1][0] - m[0][1]];
    let s = 0.5 * fit::norm(v);
    let c = 0.5 * (m[0][0] + m[1][1] + m[2][2] - 1.0);
    let angle = s.atan2(c);
    if s < 1e-12 { fit::scale(v, 0.5) } else { fit::scale(v, angle / (2.0 * s)) }
}

fn nominal_rotation(role: &str, side: &str) -> Option<[Point; 3]> {
    match role {
        "front" => {
            let (d, down) = ([0.0, 1.0, 0.0], [0.0, 0.0, -1.0]);
            Some([fit::cross(down, d), down, d])
        }
        "profile" => Some(side_rotation(side)),
        _ => None,
    }
}

fn refine_pose(cam: &mut PerspectiveCamera, obs: &[(Point, Ob, f64)], iterations: usize, prior: Option<(&[Point; 3], f64)>) {
    for _ in 0..iterations {
        let mut h = vec![0.0; 36];
        let mut g = vec![0.0; 6];
        for (x, ob, wt) in obs {
            let c = cam.camera_space(*x);
            if c[2] <= 1e-6 {
                continue;
            }
            let a = fit::sub(c, cam.translation);
            let f = cam.focal;
            let ju = [f / c[2], 0.0, -f * c[0] / (c[2] * c[2])];
            let jv = [0.0, f / c[2], -f * c[1] / (c[2] * c[2])];
            let dc: [Point; 6] = [[0.0, -a[2], a[1]], [a[2], 0.0, -a[0]], [-a[1], a[0], 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
            let q = cam.project(*x);
            let mut rows: Vec<(Point, f64)> = Vec::new();
            match ob {
                Ob::Mark(u) => {
                    rows.push((ju, q[0] - u[0]));
                    rows.push((jv, q[1] - u[1]));
                }
                Ob::Edge(p, n) => rows.push((fit::add(fit::scale(ju, n[0]), fit::scale(jv, n[1])), n[0] * (q[0] - p[0]) + n[1] * (q[1] - p[1]))),
            }
            for (jc, r) in rows {
                let j: Vec<f64> = dc.iter().map(|d| fit::dot(jc, *d)).collect();
                for i in 0..6 {
                    g[i] += wt * j[i] * r;
                    for k in 0..6 {
                        h[i * 6 + k] += wt * j[i] * j[k];
                    }
                }
            }
        }
        if let Some((nominal, weight)) = prior {
            let e = rotation_error(&cam.rotation, nominal);
            for i in 0..3 {
                h[i * 6 + i] += weight;
                g[i] += weight * e[i];
            }
        }
        for i in 0..6 {
            h[i * 6 + i] *= 1.0 + 1e-4;
            h[i * 6 + i] += 1e-12;
        }
        let Some(step) = solve_dense(h, g.iter().map(|x| -x).collect(), 6) else { return };
        let turn = rodrigues([step[0], step[1], step[2]]);
        cam.rotation = mat_mul(&turn, &cam.rotation);
        cam.translation = fit::add(cam.translation, [step[3], step[4], step[5]]);
        if fit::norm([step[0], step[1], step[2]]) < 1e-7 && fit::norm([step[3], step[4], step[5]]) < 1e-7 {
            break;
        }
    }
}

fn camera_centre(cam: &PerspectiveCamera) -> Point {
    let r = &cam.rotation;
    let t = cam.translation;
    [-(r[0][0] * t[0] + r[1][0] * t[1] + r[2][0] * t[2]), -(r[0][1] * t[0] + r[1][1] * t[1] + r[2][1] * t[2]), -(r[0][2] * t[0] + r[1][2] * t[1] + r[2][2] * t[2])]
}

fn side_rotation(side: &str) -> [Point; 3] {
    let d: Point = if side == "left" { [-1.0, 0.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let down = [0.0, 0.0, -1.0];
    [fit::cross(down, d), down, d]
}

fn translation_from_keys(rotation: [Point; 3], focal: f64, principal: [f64; 2], pairs: &[(Point, [f64; 2])]) -> Option<Point> {
    let mut ata = vec![0.0; 9];
    let mut atb = vec![0.0; 3];
    for (x, u) in pairs {
        let a = [fit::dot(rotation[0], *x), fit::dot(rotation[1], *x), fit::dot(rotation[2], *x)];
        for k in 0..2 {
            let du = u[k] - principal[k];
            let mut row = [0.0; 3];
            row[k] = focal;
            row[2] = -du;
            let rhs = du * a[2] - focal * a[k];
            for i in 0..3 {
                atb[i] += row[i] * rhs;
                for j in 0..3 {
                    ata[i * 3 + j] += row[i] * row[j];
                }
            }
        }
    }
    solve_dense(ata, atb, 3).map(|t| [t[0], t[1], t[2]])
}

struct Frame {
    w: usize,
    h: usize,
    proj: Vec<[f64; 2]>,
    z: Vec<f64>,
    depth: Vec<f32>,
}

fn frame(cam: &PerspectiveCamera, x: &[Point], tris: &[[u32; 3]], size: [usize; 2]) -> Frame {
    let (w, h) = (size[0], size[1]);
    let mut proj = Vec::with_capacity(x.len());
    let mut z = Vec::with_capacity(x.len());
    for p in x {
        let c = cam.camera_space(*p);
        z.push(c[2]);
        proj.push(if c[2] > 1e-6 { [cam.principal[0] + cam.focal * c[0] / c[2], cam.principal[1] + cam.focal * c[1] / c[2]] } else { [f64::NAN; 2] });
    }
    let mut depth = vec![FAR; w * h];
    let edge = |a: [f64; 2], b: [f64; 2], p: [f64; 2]| (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
    for t in tris {
        let (a, b, c) = (proj[t[0] as usize], proj[t[1] as usize], proj[t[2] as usize]);
        if !(a[0].is_finite() && b[0].is_finite() && c[0].is_finite()) {
            continue;
        }
        let area = edge(a, b, c);
        if area.abs() < 1e-12 {
            continue;
        }
        let x0 = (a[0].min(b[0]).min(c[0]) - 0.5).ceil().max(0.0) as i64;
        let x1 = (a[0].max(b[0]).max(c[0]) - 0.5).floor().min(w as f64 - 1.0) as i64;
        let y0 = (a[1].min(b[1]).min(c[1]) - 0.5).ceil().max(0.0) as i64;
        let y1 = (a[1].max(b[1]).max(c[1]) - 0.5).floor().min(h as f64 - 1.0) as i64;
        if x1 < x0 || y1 < y0 {
            continue;
        }
        let (za, zb, zc) = (z[t[0] as usize], z[t[1] as usize], z[t[2] as usize]);
        for py in y0..=y1 {
            for px in x0..=x1 {
                let p = [px as f64 + 0.5, py as f64 + 0.5];
                let w0 = edge(b, c, p) / area;
                let w1 = edge(c, a, p) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                    let d = (w0 * za + w1 * zb + w2 * zc) as f32;
                    let cell = &mut depth[py as usize * w + px as usize];
                    if d < *cell {
                        *cell = d;
                    }
                }
            }
        }
    }
    Frame { w, h, proj, z, depth }
}

impl Frame {
    fn pixel(&self, q: [f64; 2]) -> Option<(usize, usize)> {
        if !q[0].is_finite() || q[0] < 0.0 || q[1] < 0.0 {
            return None;
        }
        let (u, v) = (q[0] as usize, q[1] as usize);
        (u < self.w && v < self.h).then_some((u, v))
    }

    fn covered(&self, u: i64, v: i64) -> bool {
        u >= 0 && v >= 0 && (u as usize) < self.w && (v as usize) < self.h && self.depth[v as usize * self.w + u as usize] < FAR
    }

    fn visible(&self, v: usize, tol: f64) -> bool {
        match self.pixel(self.proj[v]) {
            Some((u, r)) => {
                let d = self.depth[r * self.w + u];
                d == FAR || self.z[v] <= d as f64 + tol
            }
            None => false,
        }
    }

    fn rim(&self, allowed: &[bool]) -> Vec<(usize, [f64; 2])> {
        let mut boundary = vec![-1i32; self.w * self.h];
        let mut normals: Vec<[f64; 2]> = Vec::new();
        let mut centres: Vec<[f64; 2]> = Vec::new();
        for v in 0..self.h as i64 {
            for u in 0..self.w as i64 {
                if !self.covered(u, v) {
                    continue;
                }
                if self.covered(u - 1, v) && self.covered(u + 1, v) && self.covered(u, v - 1) && self.covered(u, v + 1) {
                    continue;
                }
                let mut n = [0.0, 0.0];
                for dv in -2..=2i64 {
                    for du in -2..=2i64 {
                        if self.covered(u + du, v + dv) {
                            n[0] -= du as f64;
                            n[1] -= dv as f64;
                        }
                    }
                }
                let len = n[0].hypot(n[1]);
                if len < 1e-9 {
                    continue;
                }
                boundary[v as usize * self.w + u as usize] = normals.len() as i32;
                normals.push([n[0] / len, n[1] / len]);
                centres.push([u as f64 + 0.5, v as f64 + 0.5]);
            }
        }
        let mut best: Vec<(usize, f64)> = vec![(usize::MAX, f64::NEG_INFINITY); normals.len()];
        for (vi, q) in self.proj.iter().enumerate() {
            if !allowed[vi] || !self.visible(vi, 0.010) {
                continue;
            }
            let Some((u, r)) = self.pixel(*q) else { continue };
            for dv in -1..=1i64 {
                for du in -1..=1i64 {
                    let (bu, bv) = (u as i64 + du, r as i64 + dv);
                    if bu < 0 || bv < 0 || bu as usize >= self.w || bv as usize >= self.h {
                        continue;
                    }
                    let b = boundary[bv as usize * self.w + bu as usize];
                    if b < 0 {
                        continue;
                    }
                    let b = b as usize;
                    let s = normals[b][0] * (q[0] - centres[b][0]) + normals[b][1] * (q[1] - centres[b][1]);
                    if s > best[b].1 {
                        best[b] = (vi, s);
                    }
                }
            }
        }
        let mut seen: HashMap<usize, usize> = HashMap::new();
        let mut out: Vec<(usize, [f64; 2])> = Vec::new();
        for (b, (vi, _)) in best.iter().enumerate() {
            if *vi == usize::MAX {
                continue;
            }
            match seen.get(vi) {
                Some(&k) => {
                    let n = &mut out[k].1;
                    n[0] += normals[b][0];
                    n[1] += normals[b][1];
                }
                None => {
                    seen.insert(*vi, out.len());
                    out.push((*vi, normals[b]));
                }
            }
        }
        for (_, n) in out.iter_mut() {
            let len = n[0].hypot(n[1]).max(1e-12);
            *n = [n[0] / len, n[1] / len];
        }
        out
    }
}

struct Matched {
    vertex: usize,
    photo: [f64; 2],
    normal: [f64; 2],
    residual_px: f64,
}

fn match_outline(view: &View, fr: &Frame, rim: &[(usize, [f64; 2])], radius: f64) -> (Vec<Matched>, usize) {
    let mut out = Vec::new();
    for (v, n) in rim {
        let q = fr.proj[*v];
        if let Some(i) = view.grid.nearest(&view.outline, q, *n, radius) {
            let p = view.outline[i];
            let r = p[2] * (q[0] - p[0]) + p[3] * (q[1] - p[1]);
            out.push(Matched { vertex: *v, photo: [p[0], p[1]], normal: [p[2], p[3]], residual_px: r });
        }
    }
    let total = rim.len();
    if out.len() > 20 {
        let mut a: Vec<f64> = out.iter().map(|m| m.residual_px.abs()).collect();
        a.sort_by(f64::total_cmp);
        let cut = (3.0 * a[a.len() / 2]).max(2.0);
        out.retain(|m| m.residual_px.abs() <= cut);
    }
    (out, total)
}

fn visible_marks(view: &View, fr: &Frame, x: &[Point], normals: &[Point], lm: &BTreeMap<usize, usize>, set: &[usize]) -> Vec<(usize, usize, [f64; 2])> {
    let Some(marks) = &view.marks else { return Vec::new() };
    let centre = camera_centre(&view.cam);
    set
        .iter()
        .filter_map(|i| lm.get(i).map(|v| (*i, *v)))
        .filter_map(|(i, v)| marks.get(i).filter(|u| u[0].is_finite()).map(|u| (i, v, *u)))
        .filter(|(_, v, _)| {
            let to = fit::sub(centre, x[*v]);
            fr.visible(*v, 0.004) && fit::dot(normals[*v], to) > 0.1 * fit::norm(to)
        })
        .collect()
}

fn rms(v: &[f64]) -> f64 {
    (v.iter().map(|x| x * x).sum::<f64>() / v.len().max(1) as f64).sqrt()
}

fn p95(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s: Vec<f64> = v.iter().map(|x| x.abs()).collect();
    s.sort_by(f64::total_cmp);
    s[((s.len() - 1) as f64 * 0.95).round() as usize]
}

fn read_outline(path: &str) -> Result<Vec<[f64; 4]>, ApiError> {
    let bytes = std::fs::read(path).map_err(|e| ApiError::not_found(format!("{path}: {e}")))?;
    if bytes.len() < 12 || &bytes[1..6] != b"NUMPY" {
        return Err(ApiError::invalid(format!("{path} is not an .npy file")));
    }
    let start = if bytes[6] == 1 { 10 + u16::from_le_bytes([bytes[8], bytes[9]]) as usize } else { 12 + u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize };
    let header = String::from_utf8_lossy(&bytes[..start.min(bytes.len())]);
    if !header.contains("<f8") || !header.contains(", 4)") {
        return Err(ApiError::invalid(format!("{path} is not a float64 (n, 4) array")));
    }
    Ok(bytes[start.min(bytes.len())..]
        .chunks_exact(32)
        .map(|c| {
            let f = |i: usize| f64::from_le_bytes(c[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8]));
            [f(0), f(1), f(2), f(3)]
        })
        .collect())
}

fn camera_json(cam: &PerspectiveCamera) -> Value {
    let d = cam.rotation[2];
    json!({"focal": cam.focal, "principal": cam.principal, "rotation": cam.rotation, "translation": cam.translation, "centre": camera_centre(cam),
           "view_yaw_deg": (-d[0]).atan2(d[1]).to_degrees(), "view_elevation_deg": (-d[2]).asin().to_degrees(),
           "convention": "camera space c = R x + t; u = cx + f c0 / c2, v = cy + f c1 / c2 (pixels from the top left)"})
}

pub fn input() -> Value {
    obj(
        photo::merged(
            photo::tool_props(),
            photo::merged(
                photo::store_props(),
                json!({
                    "views": {"type": "string", "description": "views.json from photo.views (default <run>/views.json)"},
                    "base_landmarks": {"type": "string", "description": "the base's landmark table, JSON {\"vertices\": {landmark index: dense vertex}} (default <run>/base_landmarks.json)"},
                    "rounds": {"type": "integer", "minimum": 2, "maximum": 30},
                    "landmark_weight": {"type": "number", "minimum": 0}, "outline_weight": {"type": "number", "minimum": 0},
                    "prior_weight": {"type": "number", "minimum": 0}, "symmetry_weight": {"type": "number", "minimum": 0},
                    "ridge": {"type": "number", "minimum": 0},
                    "prior_scope": {"type": "string", "enum": ["unseen", "all"], "description": "where the stay-at-base term applies: vertices whose outline is not fitted, except the eyes (default), or every vertex"},
                    "landmarks": {"type": "string", "enum": ["stable", "dense"], "description": "every landmark of the base table except the face oval (default) or the 30 stable landmarks"},
                    "sparsity": {"type": "number", "minimum": 0, "description": "an L1 penalty on the weights, relative to the mean diagonal of the normal matrix (default 0.0007)"},
                    "extra_landmark_weight": {"type": "number", "minimum": 0, "description": "landmark weight of an extra view relative to the front's (default 1)"},
                    "pose_prior_deg": {"type": "number", "minimum": 0, "description": "how far the gated front and profile may turn from straight before it costs as much as every observation being 1 px off (default 3; 0 frees them)"},
                    "focal_px": {"type": "number", "exclusiveMinimum": 0, "description": "every photo's focal length in pixels (default 70/36 of the photo width: a 70 mm full-frame lens)"},
                    "outline_regions": {"type": "array", "items": {"type": "string", "enum": fit::REGIONS}, "description": "regions whose outline is fitted (default all but skull and neck: hair and clothing)"},
                    "leave_out": {"type": "string", "description": "a view name: fit without it, then solve its camera and report how well the fit predicts its outline and landmarks"}
                }),
            ),
        ),
        &["run", "store", "pack", "catalog"],
    )
}

pub fn fit_photos(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = photo::run_dir(input)?;
    let started = Instant::now();
    let views_path = opt(input, "views").map(str::to_string).unwrap_or_else(|| run.join("views.json").display().to_string());
    let views_doc = read_json(Path::new(&views_path))?;
    if let Some(table) = opt(input, "base_landmarks") {
        let dest = run.join("base_landmarks.json");
        let same = std::fs::canonicalize(table).ok().zip(std::fs::canonicalize(&dest).ok()).is_some_and(|(a, b)| a == b);
        if !same {
            write_json(&dest, &read_json(Path::new(table))?)?;
        }
    }
    let lm = photo::landmark_vertices(&run)?;
    let base = photo::load_base(input)?;
    let p = &base.dense.positions;
    let tris = &base.dense.triangles;
    let tip = base.tip;
    let mut targets = Targets::load(&base, text(input, "catalog"))?;
    let t = targets.t();
    let load_seconds = started.elapsed().as_secs_f64();
    let leave_out = opt(input, "leave_out").map(str::to_string);
    let focal_fixed = input.get("focal_px").and_then(Value::as_f64);
    let region_names: Vec<String> = match input.get("outline_regions").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => fit::REGIONS.iter().filter(|r| **r != "skull" && **r != "neck").map(|r| r.to_string()).collect(),
    };
    let eyes = photo::eye_centres(p, &lm);
    let region: Vec<usize> = p.iter().map(|q| fit::region(photo::mm(fit::sub(*q, tip)), photo::eye_gap_mm(*q, &eyes))).collect();
    let allowed: Vec<bool> = region.iter().map(|r| region_names.iter().any(|n| n == fit::REGIONS[*r])).collect();

    let mut views: Vec<View> = Vec::new();
    for rec in views_doc["views"].as_array().ok_or_else(|| ApiError::invalid(format!("{views_path} has no views")))? {
        let name = rec["name"].as_str().unwrap_or("").to_string();
        let role = rec["role"].as_str().unwrap_or("extra").to_string();
        let size = [rec["size"][0].as_u64().unwrap_or(0) as usize, rec["size"][1].as_u64().unwrap_or(0) as usize];
        if size[0] == 0 || size[1] == 0 {
            return Err(ApiError::invalid(format!("view {name} has no size")));
        }
        let outline = read_outline(rec["outline"].as_str().ok_or_else(|| ApiError::invalid(format!("view {name} has no outline")))?)?;
        let marks: Option<Vec<[f64; 2]>> = rec["landmarks"].as_array().map(|a| a.iter().map(|u| [u[0].as_f64().unwrap_or(f64::NAN), u[1].as_f64().unwrap_or(f64::NAN)]).collect());
        let focal = focal_fixed.unwrap_or(FOCAL_PER_WIDTH * size[0] as f64);
        let principal = [size[0] as f64 / 2.0, size[1] as f64 / 2.0];
        let cam = if role == "profile" {
            let keys = &rec["keys"];
            let key = |k: &str| keys.get(k).and_then(Value::as_array).map(|a| [a[0].as_f64().unwrap_or(f64::NAN), a[1].as_f64().unwrap_or(f64::NAN)]);
            let mid = |a: usize, b: usize| lm.get(&a).zip(lm.get(&b)).map(|(x, y)| fit::scale(fit::add(p[*x], p[*y]), 0.5));
            let mut pairs: Vec<(Point, [f64; 2])> = Vec::new();
            if let Some(u) = key("nose_tip") {
                pairs.push((tip, u));
            }
            for (k, model) in [("nasion", lm.get(&168).map(|v| p[*v])), ("subnasale", lm.get(&2).map(|v| p[*v])), ("stomion", mid(13, 14))] {
                if let (Some(u), Some(x)) = (key(k), model) {
                    pairs.push((x, u));
                }
            }
            pairs.retain(|(_, u)| u[0].is_finite());
            if pairs.len() < 2 {
                return Err(ApiError::invalid(format!("profile view {name} needs at least two profile keys to place its camera")));
            }
            let rotation = side_rotation(rec["side"].as_str().unwrap_or("right"));
            let translation = translation_from_keys(rotation, focal, principal, &pairs).ok_or_else(|| ApiError::invalid(format!("profile view {name}: the keys do not place a camera")))?;
            PerspectiveCamera { focal, principal, rotation, translation }
        } else {
            let marks = marks.as_ref().ok_or_else(|| ApiError::invalid(format!("view {name} has no landmarks to place its camera")))?;
            let used: Vec<(Point, [f64; 2])> = STABLE_LANDMARKS.iter().filter_map(|i| lm.get(i).zip(marks.get(*i))).filter(|(_, u)| u[0].is_finite()).map(|(v, u)| (p[*v], *u)).collect();
            if used.len() < 8 {
                return Err(ApiError::invalid(format!("view {name} has {} landmarks; 8 are needed to place its camera", used.len())));
            }
            let (xs, us): (Vec<Point>, Vec<[f64; 2]>) = used.into_iter().unzip();
            fit::fit_perspective_camera(&xs, &us, principal, Some(focal)).map_err(ApiError::invalid)?
        };
        let grid = Grid2::new(&outline, 8.0);
        let left_out = leave_out.as_deref() == Some(name.as_str());
        let nominal = nominal_rotation(&role, rec["side"].as_str().unwrap_or("right"));
        views.push(View { name, role, size, marks, outline, grid, cam, left_out, nominal });
    }
    if !views.iter().any(|v| v.role == "front") || !views.iter().any(|v| v.role == "profile") {
        return Err(ApiError::invalid("at least one front and one profile view are required"));
    }
    if let Some(name) = &leave_out
        && !views.iter().any(|v| &v.name == name)
    {
        return Err(ApiError::invalid(format!("leave_out names no view ({name})")));
    }

    let (wl, wo) = (num(input, "landmark_weight", 1.0), num(input, "outline_weight", 1.0));
    let (wprior, wsym, ridge) = (num(input, "prior_weight", 0.1), num(input, "symmetry_weight", 0.05), num(input, "ridge", 1e-3));
    let (pose_sigma, sparsity, extra_lm) = (num(input, "pose_prior_deg", 3.0), num(input, "sparsity", 0.0007), num(input, "extra_landmark_weight", 1.0));
    let rounds = input.get("rounds").and_then(Value::as_u64).unwrap_or(10) as usize;
    let mark_set: Vec<usize> = if opt(input, "landmarks") != Some("stable") { lm.keys().copied().filter(|i| !FACE_OVAL.contains(i)).collect() } else { STABLE_LANDMARKS.to_vec() };
    let index: HashMap<&str, usize> = targets.ids.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
    let sym_pairs: Vec<(usize, usize)> = targets.ids.iter().enumerate().filter_map(|(i, k)| index.get(k.replacen("/l-", "/r-", 1).as_str()).filter(|&&j| j != i).map(|&j| (i, j))).collect();
    let prior_everywhere = opt(input, "prior_scope") == Some("all");
    let prior_all: Vec<usize> = (0..p.len()).filter(|v| prior_everywhere || (!allowed[*v] && fit::REGIONS[region[*v]] != "eyes")).collect();
    let prior_rows: Vec<usize> = prior_all.iter().copied().step_by((prior_all.len() / 8000).max(1)).collect();
    let mut prior = Normal::new(t);
    let prior_weight = wprior / (3 * prior_rows.len()).max(1) as f64;
    for &v in &prior_rows {
        let rows = targets.rows(v).to_vec();
        for k in 0..3 {
            let mut g = [0.0; 3];
            g[k] = 1.0;
            prior.add(&Normal::row(&rows, g), 0.0, prior_weight);
        }
    }
    let fitted_views = views.iter().filter(|v| !v.left_out).count().max(1) as f64;
    let mut w = vec![0.0; t];
    let mut log = Vec::new();
    for round in 0..=rounds {
        let last = round == rounds;
        let radius = RADII_PX[round.min(RADII_PX.len() - 1)];
        let x = targets.evaluate(p, &w);
        let normals = gates::vertex_normals(&x, &base.quads);
        let mut acc = Normal { t, gram: prior.gram.clone(), rhs: prior.rhs.clone() };
        let mut round_log = Vec::new();
        for view in views.iter_mut() {
            let mut fr = frame(&view.cam, &x, tris, view.size);
            let rim = fr.rim(&allowed);
            let (matched, _) = match_outline(view, &fr, &rim, radius);
            let marks = visible_marks(view, &fr, &x, &normals, &lm, &mark_set);
            let n_m = matched.len().max(1) as f64;
            let n_l = (2 * marks.len()).max(1) as f64;
            let mut obs: Vec<(Point, Ob, f64)> = matched.iter().map(|m| (x[m.vertex], Ob::Edge(m.photo, m.normal), wo / n_m)).collect();
            obs.extend(marks.iter().map(|(_, v, u)| (x[*v], Ob::Mark(*u), wl / n_l)));
            let total: f64 = obs.iter().map(|o| o.2).sum();
            let prior_of = view.nominal.as_ref().filter(|_| pose_sigma > 0.0).map(|r| (r, total / pose_sigma.to_radians().powi(2)));
            refine_pose(&mut view.cam, &obs, 12, prior_of);
            fr = frame(&view.cam, &x, tris, view.size);
            let rim = fr.rim(&allowed);
            let (matched, rim_total) = match_outline(view, &fr, &rim, if last { RADII_PX[RADII_PX.len() - 1] } else { radius });
            let marks = visible_marks(view, &fr, &x, &normals, &lm, &mark_set);
            let outline_mm: Vec<f64> = matched.iter().map(|m| m.residual_px / view.cam.pixels_per_metre(x[m.vertex]) * 1000.0).collect();
            let mark_px: Vec<f64> = marks.iter().map(|(_, v, u)| {
                let q = fr.proj[*v];
                (q[0] - u[0]).hypot(q[1] - u[1])
            }).collect();
            let face_px = lm.get(&10).zip(lm.get(&152)).map(|(a, b)| {
                let (qa, qb) = (fr.proj[*a], fr.proj[*b]);
                (qa[0] - qb[0]).hypot(qa[1] - qb[1])
            });
            round_log.push(json!({"view": view.name, "outline_points": matched.len(), "rim_points": rim_total, "outline_rms_mm": rms(&outline_mm),
                                  "outline_p95_mm": p95(&outline_mm), "landmarks": marks.len(), "landmark_rms_px": (!marks.is_empty()).then(|| rms(&mark_px)),
                                  "landmark_rms_per_mille_face": face_px.filter(|_| !marks.is_empty()).map(|f| rms(&mark_px) / f * 1000.0),
                                  "landmark_max_px": mark_px.iter().copied().fold(None, |a: Option<f64>, b| Some(a.map_or(b, |a| a.max(b)))),
                                  "left_out": view.left_out, "camera": if last { camera_json(&view.cam) } else { Value::Null }}));
            if last || view.left_out || round == 0 {
                continue;
            }
            let ow = wo / (fitted_views * matched.len().max(1) as f64);
            for m in &matched {
                let xv = x[m.vertex];
                let ppm = view.cam.pixels_per_metre(xv);
                let gr = view.cam.gradient(xv);
                let g = fit::scale(fit::add(fit::scale(gr[0], m.normal[0]), fit::scale(gr[1], m.normal[1])), 1.0 / ppm);
                let rows = targets.rows(m.vertex).to_vec();
                let a = Normal::row(&rows, g);
                acc.add(&a, -m.residual_px / ppm + fit::dot(g, fit::sub(xv, p[m.vertex])), ow);
            }
            let lw = wl * if view.role == "extra" { extra_lm } else { 1.0 } / (fitted_views * (2 * marks.len()).max(1) as f64);
            for (_, v, u) in &marks {
                let xv = x[*v];
                let ppm = view.cam.pixels_per_metre(xv);
                let gr = view.cam.gradient(xv);
                let q = view.cam.project(xv);
                let rows = targets.rows(*v).to_vec();
                for k in 0..2 {
                    let g = fit::scale(gr[k], 1.0 / ppm);
                    acc.add(&Normal::row(&rows, g), (u[k] - q[k]) / ppm + fit::dot(g, fit::sub(xv, p[*v])), lw);
                }
            }
        }
        log.push(json!({"round": round, "radius_px": radius, "views": round_log}));
        if last || round == 0 {
            continue;
        }
        let sw = wsym / sym_pairs.len().max(1) as f64;
        for (i, j) in &sym_pairs {
            acc.add(&[(*i, 1.0), (*j, -1.0)], 0.0, sw);
        }
        let trace: f64 = (0..t).map(|i| acc.gram[i * t + i]).sum::<f64>() / t as f64;
        for i in 0..t {
            acc.gram[i * t + i] += ridge * trace;
        }
        let rhs: Vec<f64> = acc.rhs.iter().map(|r| r - sparsity * trace).collect();
        w = fit::bounded_least_squares(&acc.gram, &rhs, 0.0, 1.0, &w, 3000);
    }
    let x = targets.evaluate(p, &w);
    let shell = gates::shell(p, &x, &base.quads, &ShellLimits::default()).map_err(ApiError::internal)?;
    dio::write_points_f64(&run.join("fitted_dense.npy"), &x).map_err(ApiError::internal)?;
    let final_views = log.last().map(|l| l["views"].clone()).unwrap_or(json!([]));
    let mut per_view = serde_json::Map::new();
    let mut cameras = serde_json::Map::new();
    for v in final_views.as_array().into_iter().flatten() {
        let name = v["view"].as_str().unwrap_or("").to_string();
        cameras.insert(name.clone(), v["camera"].clone());
        let mut s = v.clone();
        if let Some(o) = s.as_object_mut() {
            o.remove("camera");
        }
        per_view.insert(name, s);
    }
    let weights: BTreeMap<String, f64> = targets.ids.iter().zip(&w).filter(|(_, x)| **x > 1e-6).map(|(k, x)| (k.clone(), (x * 1e6).round() / 1e6)).collect();
    let report = json!({
        "targets": t, "nonzero": weights.len(), "at_1": w.iter().filter(|x| **x >= 1.0 - 1e-9).count(),
        "weights_in_bounds": w.iter().all(|x| (0.0..=1.0).contains(x)), "weight_min": w.iter().copied().fold(f64::INFINITY, f64::min),
        "weight_max": w.iter().copied().fold(f64::NEG_INFINITY, f64::max), "weights": weights,
        "views": per_view, "cameras": cameras, "leave_out": leave_out, "outline_regions": region_names,
        "term_weights": {"landmarks": wl, "outline": wo, "prior": wprior, "symmetry": wsym, "ridge": ridge, "rounds": rounds, "pose_prior_deg": pose_sigma, "sparsity": sparsity, "extra_landmark_weight": extra_lm},
        "samples": {"prior": prior_rows.len(), "symmetry_pairs": sym_pairs.len()},
        "shell_gate": shell, "rounds": log, "load_seconds": load_seconds, "seconds": started.elapsed().as_secs_f64(),
        "fitted": run.join("fitted_dense.npy").display().to_string()
    });
    write_json(&run.join("fit_photos.json"), &report)?;
    Ok(report)
}

pub fn views_input() -> Value {
    obj(
        photo::with_tools(json!({
            "photos": {"type": "array", "minItems": 2, "items": {"type": "object", "properties": {
                "name": {"type": "string", "minLength": 1}, "path": {"type": "string", "minLength": 1},
                "role": {"type": "string", "enum": ["front", "profile", "extra"]}, "side": {"type": "string", "enum": ["right", "left"]}},
                "required": ["name", "path", "role"], "additionalProperties": false}},
            "thresholds": {"type": "object", "description": "overrides of the reference-photo gate's limits (extra views use its quality limits)"}
        })),
        &["run", "photos"],
    )
}

pub fn views(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = photo::run_dir(input)?;
    let mut args = json!({"photos": input["photos"].clone()});
    if let Some(t) = input.get("thresholds") {
        args["thresholds"] = t.clone();
    }
    photo::worker(input, "photo_views.py", "views", args, &run)
}

pub fn sheet_input() -> Value {
    obj(photo::with_tools(json!({"out": {"type": "string", "minLength": 1}, "quads": {"type": "string", "minLength": 1, "description": "the dense corner_vertices .npy"},
                                 "width": {"type": "integer", "minimum": 128, "maximum": 2048}})), &["run", "out", "quads"])
}

pub fn sheet(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = photo::run_dir(input)?;
    let args = json!({"fitted": run.join("fitted_dense.npy").display().to_string(), "quads": text(input, "quads"), "out": text(input, "out"),
                      "width": input.get("width").and_then(Value::as_u64).unwrap_or(420)});
    photo::worker(input, "photo_views.py", "sheet", args, &run)
}

fn write_quads(path: &Path, quads: &[[u32; 4]]) -> Result<(), ApiError> {
    let mut header = format!("{{'descr': '<u4', 'fortran_order': False, 'shape': ({}, 4), }}", quads.len());
    while (10 + header.len() + 1) % 64 != 0 {
        header.push(' ');
    }
    header.push('\n');
    let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
    bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
    bytes.extend_from_slice(header.as_bytes());
    for q in quads {
        for v in q {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    std::fs::write(path, bytes).map_err(|e| ApiError::internal(format!("{}: {e}", path.display())))
}

pub fn from_photos(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let run = photo::run_dir(input)?;
    let started = Instant::now();
    let mut stages: Vec<Value> = Vec::new();
    let mut step = |name: &str, out: Result<Value, ApiError>, t0: Instant| -> Result<Value, ApiError> {
        stages.push(json!({"stage": name, "seconds": t0.elapsed().as_secs_f64(), "ok": out.is_ok()}));
        let _ = write_json(&run.join("stages.json"), &json!({"stages": stages}));
        out.map_err(|e| {
            let mut details = if e.details.is_object() { e.details.clone() } else { json!({"detail": e.details.clone()}) };
            details["stage"] = json!(name);
            details["stages"] = json!(stages.clone());
            ApiError { details, ..e }
        })
    };
    let side = opt(input, "profile_side").unwrap_or("right").to_string();
    let mut tools = json!({});
    for k in ["run", "python", "workers", "landmarker"] {
        if let Some(v) = input.get(k) {
            tools[k] = v.clone();
        }
    }
    let mut ingest_in = tools.clone();
    ingest_in["front"] = json!(text(input, "front"));
    ingest_in["profile"] = json!(text(input, "profile"));
    ingest_in["profile_side"] = json!(side);
    if let Some(v) = input.get("max_ratio_gap") {
        ingest_in["max_ratio_gap"] = v.clone();
    }
    let t0 = Instant::now();
    let ingested = step("0 photo.ingest", photo::ingest(ctx, &ingest_in), t0)?;
    let mut photos = vec![json!({"name": "front", "path": text(input, "front"), "role": "front"}),
                          json!({"name": "profile", "path": text(input, "profile"), "role": "profile", "side": side})];
    for (i, e) in input.get("extras").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let name = e.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("extra_{}", i + 1));
        photos.push(json!({"name": name, "path": e["path"].clone(), "role": "extra"}));
    }
    let mut views_in = tools.clone();
    views_in["photos"] = json!(photos);
    let t0 = Instant::now();
    let viewed = step("1 photo.views", views(ctx, &views_in), t0)?;
    let mut fit_in = input.get("fit_options").cloned().filter(Value::is_object).unwrap_or(json!({}));
    for k in ["run", "store", "pack", "catalog", "base_landmarks"] {
        if let Some(v) = input.get(k) {
            fit_in[k] = v.clone();
        }
    }
    let errors = crate::schema::validate(&self::input(), &fit_in);
    if !errors.is_empty() {
        return Err(ApiError::invalid(format!("fit_options: {}", errors.join("; "))));
    }
    let t0 = Instant::now();
    let fitted = step("4 fit.photos", fit_photos(ctx, &fit_in), t0)?;
    if fitted["shell_gate"]["pass"] != json!(true) {
        let failures: Vec<&str> = fitted["shell_gate"]["failures"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        return Err(ApiError::new(Code::GateFailed, format!("the fit was refused by the shell gates: {}", failures.join("; ")))
            .details(json!({"stage": "4 fit.photos", "stages": stages, "shell_gate": fitted["shell_gate"], "fit": run.join("fit_photos.json").display().to_string()}))
            .hint("nothing was exported; raise sparsity or prior_weight in fit_options, or inspect fit_photos.json"));
    }
    let name = opt(input, "name").unwrap_or("photo_character").to_string();
    let spec_path = run.join(format!("{name}.gfchar.json"));
    let glb = run.join(format!("{name}.glb"));
    let t0 = Instant::now();
    let exported = step("8a character.settle + export-glb", photo::settle_export(ctx, input, &run, &spec_path, &glb, &name, &fitted), t0)?;
    let sheet_out = opt(input, "sheet").map(str::to_string).unwrap_or_else(|| run.join("review_sheet.png").display().to_string());
    let t0 = Instant::now();
    let quads = run.join("dense_quads.npy");
    let made = photo::load_base(input).and_then(|b| write_quads(&quads, &b.quads)).and_then(|_| {
        let mut s = tools.clone();
        s["out"] = json!(sheet_out);
        s["quads"] = json!(quads.display().to_string());
        sheet(ctx, &s)
    });
    let reviewed = step("8b review.views", made, t0)?;
    let report = json!({"run": run.display().to_string(), "fit": "photos", "spec": spec_path.display().to_string(), "glb": glb.display().to_string(),
                        "sheet": sheet_out, "total_seconds": started.elapsed().as_secs_f64(), "stages": stages,
                        "facing": ingested.get("facing"), "views": viewed.get("views").and_then(Value::as_array).map(Vec::len),
                        "fitted": {"nonzero": fitted["nonzero"], "views": fitted["views"], "shell_gate": fitted["shell_gate"], "weights_in_bounds": fitted["weights_in_bounds"]},
                        "export": exported, "review": reviewed,
                        "texture": "not projected: texture.project reads the mesh path's orthographic front and profile cameras"});
    write_json(&run.join("from_photos.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_points() -> Vec<Point> {
        let mut out = Vec::new();
        for i in 0..5 {
            for j in 0..5 {
                out.push([i as f64 * 0.03 - 0.06, 0.01 * ((i * j) % 3) as f64, j as f64 * 0.03 - 0.06]);
            }
        }
        out
    }

    #[test]
    fn rotation_error_is_the_rotation_vector() {
        let n = nominal_rotation("front", "right").unwrap();
        let r = mat_mul(&rodrigues([0.0, 0.05, 0.0]), &n);
        let e = rotation_error(&r, &n);
        assert!((e[1] - 0.05).abs() < 1e-9 && e[0].abs() < 1e-9 && e[2].abs() < 1e-9, "{e:?}");
    }

    #[test]
    fn pose_refinement_recovers_a_turned_camera() {
        let n = nominal_rotation("front", "right").unwrap();
        let truth = PerspectiveCamera { focal: 2400.0, principal: [600.0, 600.0], rotation: mat_mul(&rodrigues([0.02, -0.04, 0.01]), &n), translation: [0.004, -0.002, 0.8] };
        let obs: Vec<(Point, Ob, f64)> = grid_points().into_iter().map(|x| (x, Ob::Mark(truth.project(x)), 1.0)).collect();
        let mut cam = PerspectiveCamera { focal: 2400.0, principal: [600.0, 600.0], rotation: n, translation: [0.0, 0.0, 0.85] };
        refine_pose(&mut cam, &obs, 30, None);
        for (x, _, _) in &obs {
            let (a, b) = (cam.project(*x), truth.project(*x));
            assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6);
        }
        let e = rotation_error(&cam.rotation, &truth.rotation);
        assert!(fit::norm(e) < 1e-8, "{e:?}");
    }

    #[test]
    fn a_pose_prior_holds_a_weakly_seen_rotation() {
        let n = nominal_rotation("profile", "right").unwrap();
        let mut cam = PerspectiveCamera { focal: 2400.0, principal: [600.0, 600.0], rotation: n, translation: [0.0, 0.0, 0.85] };
        let obs: Vec<(Point, Ob, f64)> = vec![([0.0, 0.0, 0.0], Ob::Mark([600.0, 600.0]), 1.0), ([0.0, 0.0, 0.05], Ob::Mark([600.0, 458.8]), 1.0)];
        refine_pose(&mut cam, &obs, 20, Some((&n, 1e9)));
        assert!(fit::norm(rotation_error(&cam.rotation, &n)) < 1e-4);
    }

    #[test]
    fn inputs_refuse_unknown_keys() {
        assert_eq!(input().get("additionalProperties"), Some(&Value::Bool(false)));
        let ok = crate::schema::validate(&input(), &json!({"run": "r", "store": "s", "pack": "p", "catalog": "c", "landmarks": "dense", "leave_out": "front"}));
        assert!(ok.is_empty(), "{ok:?}");
        assert!(!crate::schema::validate(&input(), &json!({"run": "r", "store": "s", "pack": "p", "catalog": "c", "shell_weight": 1})).is_empty());
        assert!(!crate::schema::validate(&views_input(), &json!({"run": "r", "photos": [{"name": "a", "path": "b", "role": "side"}, {"name": "c", "path": "d", "role": "front"}]})).is_empty());
    }

    #[test]
    fn quads_are_written_as_npy() {
        let dir = std::env::temp_dir().join(format!("gf_quads_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("q.npy");
        write_quads(&path, &[[0, 1, 2, 3], [4, 5, 6, 7]]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let header = 10 + u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        assert_eq!(header % 64, 0);
        assert_eq!(bytes.len(), header + 32);
        assert_eq!(&bytes[header + 28..header + 32], &7u32.to_le_bytes());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
