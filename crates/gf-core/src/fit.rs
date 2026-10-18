use std::collections::HashMap;

use serde::Serialize;

pub type Point = [f64; 3];

pub fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub fn scale(a: Point, s: f64) -> Point {
    [a[0] * s, a[1] * s, a[2] * s]
}

pub fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Similarity {
    pub scale: f64,
    pub rotation: [[f64; 3]; 3],
    pub translation: Point,
}

impl Similarity {
    pub fn identity() -> Similarity {
        Similarity { scale: 1.0, rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], translation: [0.0; 3] }
    }

    pub fn rotate(&self, p: Point) -> Point {
        let r = &self.rotation;
        [dot(r[0], p), dot(r[1], p), dot(r[2], p)]
    }

    pub fn apply(&self, p: Point) -> Point {
        add(scale(self.rotate(p), self.scale), self.translation)
    }

    pub fn then(&self, next: &Similarity) -> Similarity {
        let mut rotation = [[0.0; 3]; 3];
        for (i, row) in rotation.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| next.rotation[i][k] * self.rotation[k][j]).sum();
            }
        }
        Similarity { scale: self.scale * next.scale, rotation, translation: next.apply(self.translation) }
    }
}

pub fn symmetric_eigen(n: usize, matrix: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut a = matrix.to_vec();
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _ in 0..100 {
        let off: f64 = (0..n).flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j))).map(|(i, j)| a[i * n + j].powi(2)).sum();
        if off < 1e-30 {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                let apq = a[p * n + q];
                if apq.abs() < 1e-300 {
                    continue;
                }
                let theta = (a[q * n + q] - a[p * n + p]) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (akp, akq) = (a[k * n + p], a[k * n + q]);
                    a[k * n + p] = c * akp - s * akq;
                    a[k * n + q] = s * akp + c * akq;
                }
                for k in 0..n {
                    let (apk, aqk) = (a[p * n + k], a[q * n + k]);
                    a[p * n + k] = c * apk - s * aqk;
                    a[q * n + k] = s * apk + c * aqk;
                }
                for k in 0..n {
                    let (vkp, vkq) = (v[k * n + p], v[k * n + q]);
                    v[k * n + p] = c * vkp - s * vkq;
                    v[k * n + q] = s * vkp + c * vkq;
                }
            }
        }
    }
    ((0..n).map(|i| a[i * n + i]).collect(), v)
}

pub fn similarity(src: &[Point], dst: &[Point], weights: &[f64], with_scale: bool) -> Result<Similarity, String> {
    if src.len() != dst.len() || src.len() != weights.len() || src.len() < 3 {
        return Err(format!("a similarity needs at least 3 matched points, got {} / {} / {}", src.len(), dst.len(), weights.len()));
    }
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return Err("the point weights sum to zero".into());
    }
    let mean = |pts: &[Point]| {
        let mut m = [0.0; 3];
        for (p, w) in pts.iter().zip(weights) {
            m = add(m, scale(*p, *w));
        }
        scale(m, 1.0 / total)
    };
    let (ms, md) = (mean(src), mean(dst));
    let mut s = [[0.0; 3]; 3];
    let mut spread = 0.0;
    for ((a, b), w) in src.iter().zip(dst).zip(weights) {
        let (a, b) = (sub(*a, ms), sub(*b, md));
        spread += w * dot(a, a);
        for i in 0..3 {
            for j in 0..3 {
                s[i][j] += w * a[i] * b[j];
            }
        }
    }
    let n = [
        s[0][0] + s[1][1] + s[2][2], s[1][2] - s[2][1], s[2][0] - s[0][2], s[0][1] - s[1][0],
        s[1][2] - s[2][1], s[0][0] - s[1][1] - s[2][2], s[0][1] + s[1][0], s[2][0] + s[0][2],
        s[2][0] - s[0][2], s[0][1] + s[1][0], -s[0][0] + s[1][1] - s[2][2], s[1][2] + s[2][1],
        s[0][1] - s[1][0], s[2][0] + s[0][2], s[1][2] + s[2][1], -s[0][0] - s[1][1] + s[2][2],
    ];
    let (values, vectors) = symmetric_eigen(4, &n);
    let best = (0..4).max_by(|&a, &b| values[a].total_cmp(&values[b])).unwrap_or(0);
    let q: Vec<f64> = (0..4).map(|k| vectors[k * 4 + best]).collect();
    let (q0, q1, q2, q3) = (q[0], q[1], q[2], q[3]);
    let rotation = [
        [q0 * q0 + q1 * q1 - q2 * q2 - q3 * q3, 2.0 * (q1 * q2 - q0 * q3), 2.0 * (q1 * q3 + q0 * q2)],
        [2.0 * (q2 * q1 + q0 * q3), q0 * q0 - q1 * q1 + q2 * q2 - q3 * q3, 2.0 * (q2 * q3 - q0 * q1)],
        [2.0 * (q3 * q1 - q0 * q2), 2.0 * (q3 * q2 + q0 * q1), q0 * q0 - q1 * q1 - q2 * q2 + q3 * q3],
    ];
    let mut out = Similarity { scale: 1.0, rotation, translation: [0.0; 3] };
    if with_scale {
        let mut num = 0.0;
        for ((a, b), w) in src.iter().zip(dst).zip(weights) {
            num += w * dot(sub(*b, md), out.rotate(sub(*a, ms)));
        }
        if spread <= 0.0 || num <= 0.0 {
            return Err("the matched points are degenerate (no spread)".into());
        }
        out.scale = num / spread;
    }
    out.translation = sub(md, scale(out.rotate(ms), out.scale));
    Ok(out)
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Similarity2 {
    pub scale: f64,
    pub angle: f64,
    pub translation: [f64; 2],
}

impl Similarity2 {
    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        let (c, s) = (self.angle.cos() * self.scale, self.angle.sin() * self.scale);
        [c * p[0] - s * p[1] + self.translation[0], s * p[0] + c * p[1] + self.translation[1]]
    }

    pub fn inverse(&self) -> Similarity2 {
        let inv = Similarity2 { scale: 1.0 / self.scale, angle: -self.angle, translation: [0.0, 0.0] };
        let t = inv.apply(self.translation);
        Similarity2 { translation: [-t[0], -t[1]], ..inv }
    }
}

pub fn similarity2(src: &[[f64; 2]], dst: &[[f64; 2]]) -> Result<Similarity2, String> {
    if src.len() != dst.len() || src.len() < 2 {
        return Err(format!("a 2D similarity needs at least 2 matched points, got {}", src.len()));
    }
    let n = src.len() as f64;
    let ms = [src.iter().map(|p| p[0]).sum::<f64>() / n, src.iter().map(|p| p[1]).sum::<f64>() / n];
    let md = [dst.iter().map(|p| p[0]).sum::<f64>() / n, dst.iter().map(|p| p[1]).sum::<f64>() / n];
    let (mut re, mut im, mut spread) = (0.0, 0.0, 0.0);
    for (a, b) in src.iter().zip(dst) {
        let (ax, ay, bx, by) = (a[0] - ms[0], a[1] - ms[1], b[0] - md[0], b[1] - md[1]);
        re += ax * bx + ay * by;
        im += ax * by - ay * bx;
        spread += ax * ax + ay * ay;
    }
    if spread <= 0.0 {
        return Err("the 2D points have no spread".into());
    }
    let angle = im.atan2(re);
    let s = (re * re + im * im).sqrt() / spread;
    let partial = Similarity2 { scale: s, angle, translation: [0.0, 0.0] };
    let r = partial.apply(ms);
    Ok(Similarity2 { translation: [md[0] - r[0], md[1] - r[1]], ..partial })
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct OrthoCamera {
    pub scale: f64,
    pub rows: [Point; 2],
    pub translation: [f64; 2],
}

impl OrthoCamera {
    pub fn project(&self, p: Point) -> [f64; 2] {
        [self.scale * dot(self.rows[0], p) + self.translation[0], self.scale * dot(self.rows[1], p) + self.translation[1]]
    }
}

pub fn fit_ortho_camera(points: &[Point], pixels: &[[f64; 2]]) -> Result<OrthoCamera, String> {
    if points.len() != pixels.len() || points.len() < 4 {
        return Err(format!("a camera needs at least 4 matched points, got {}", points.len()));
    }
    let n = points.len() as f64;
    let mp = scale(points.iter().fold([0.0; 3], |a, p| add(a, *p)), 1.0 / n);
    let mu = [pixels.iter().map(|p| p[0]).sum::<f64>() / n, pixels.iter().map(|p| p[1]).sum::<f64>() / n];
    let mut ata = [0.0; 9];
    let mut atb = [[0.0; 3]; 2];
    for (p, u) in points.iter().zip(pixels) {
        let c = sub(*p, mp);
        for i in 0..3 {
            for j in 0..3 {
                ata[i * 3 + j] += c[i] * c[j];
            }
            atb[0][i] += c[i] * (u[0] - mu[0]);
            atb[1][i] += c[i] * (u[1] - mu[1]);
        }
    }
    let inv = invert3(&ata).ok_or("the camera points are coplanar or degenerate")?;
    let mut rows = [[0.0; 3]; 2];
    for (k, row) in rows.iter_mut().enumerate() {
        for i in 0..3 {
            row[i] = (0..3).map(|j| inv[i * 3 + j] * atb[k][j]).sum();
        }
    }
    let (n0, n1) = (norm(rows[0]), norm(rows[1]));
    if n0 <= 0.0 || n1 <= 0.0 {
        return Err("the camera fit is degenerate".into());
    }
    let r0 = scale(rows[0], 1.0 / n0);
    let r1 = sub(rows[1], scale(r0, dot(rows[1], r0)));
    let r1 = scale(r1, 1.0 / norm(r1));
    let s = 0.5 * (n0 + n1);
    let mut cam = OrthoCamera { scale: s, rows: [r0, r1], translation: [0.0, 0.0] };
    let c = cam.project(mp);
    cam.translation = [mu[0] - c[0], mu[1] - c[1]];
    Ok(cam)
}

fn invert3(m: &[f64; 9]) -> Option<[f64; 9]> {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det.abs() < 1e-300 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        (m[4] * m[8] - m[5] * m[7]) * d,
        (m[2] * m[7] - m[1] * m[8]) * d,
        (m[1] * m[5] - m[2] * m[4]) * d,
        (m[5] * m[6] - m[3] * m[8]) * d,
        (m[0] * m[8] - m[2] * m[6]) * d,
        (m[2] * m[3] - m[0] * m[5]) * d,
        (m[3] * m[7] - m[4] * m[6]) * d,
        (m[1] * m[6] - m[0] * m[7]) * d,
        (m[0] * m[4] - m[1] * m[3]) * d,
    ])
}

pub struct PointGrid {
    cell: f64,
    cells: HashMap<(i32, i32, i32), (u32, u32)>,
    order: Vec<u32>,
    points: Vec<Point>,
}

impl PointGrid {
    pub fn new(points: Vec<Point>, cell: f64) -> PointGrid {
        let key = |p: &Point| ((p[0] / cell).floor() as i32, (p[1] / cell).floor() as i32, (p[2] / cell).floor() as i32);
        let mut order: Vec<u32> = (0..points.len() as u32).collect();
        order.sort_by_key(|&i| key(&points[i as usize]));
        let mut cells = HashMap::new();
        let mut start = 0usize;
        while start < order.len() {
            let k = key(&points[order[start] as usize]);
            let mut end = start + 1;
            while end < order.len() && key(&points[order[end] as usize]) == k {
                end += 1;
            }
            cells.insert(k, (start as u32, end as u32));
            start = end;
        }
        PointGrid { cell, cells, order, points }
    }

    pub fn points(&self) -> &[Point] {
        &self.points
    }

    pub fn nearest(&self, p: Point, radius: f64) -> Option<(usize, f64)> {
        let reach = (radius / self.cell).ceil() as i32;
        let c = ((p[0] / self.cell).floor() as i32, (p[1] / self.cell).floor() as i32, (p[2] / self.cell).floor() as i32);
        let mut best: Option<(usize, f64)> = None;
        for dx in -reach..=reach {
            for dy in -reach..=reach {
                for dz in -reach..=reach {
                    if let Some(&(s, e)) = self.cells.get(&(c.0 + dx, c.1 + dy, c.2 + dz)) {
                        for &i in &self.order[s as usize..e as usize] {
                            let d = norm(sub(self.points[i as usize], p));
                            if d <= radius && best.is_none_or(|(_, b)| d < b) {
                                best = Some((i as usize, d));
                            }
                        }
                    }
                }
            }
        }
        best
    }
}

pub fn bounded_least_squares(gram: &[f64], rhs: &[f64], lower: f64, upper: f64, start: &[f64], iterations: usize) -> Vec<f64> {
    let n = rhs.len();
    let mut v: Vec<f64> = (0..n).map(|i| 1.0 / (1.0 + i as f64)).collect();
    let mut lipschitz = 0.0;
    for _ in 0..200 {
        let w: Vec<f64> = (0..n).map(|i| (0..n).map(|j| gram[i * n + j] * v[j]).sum()).collect();
        let len = w.iter().map(|x| x * x).sum::<f64>().sqrt();
        if len <= 0.0 {
            break;
        }
        lipschitz = len;
        v = w.iter().map(|x| x / len).collect();
    }
    let step = if lipschitz > 0.0 { 1.0 / (1.01 * lipschitz) } else { 1.0 };
    let mut x: Vec<f64> = start.iter().map(|w| w.clamp(lower, upper)).collect();
    let mut y = x.clone();
    let mut t = 1.0f64;
    for _ in 0..iterations {
        let grad: Vec<f64> = (0..n).map(|i| (0..n).map(|j| gram[i * n + j] * y[j]).sum::<f64>() - rhs[i]).collect();
        let next: Vec<f64> = (0..n).map(|i| (y[i] - step * grad[i]).clamp(lower, upper)).collect();
        let t_next = (1.0 + (1.0 + 4.0 * t * t).sqrt()) / 2.0;
        y = (0..n).map(|i| next[i] + ((t - 1.0) / t_next) * (next[i] - x[i])).collect();
        x = next;
        t = t_next;
    }
    x
}

pub struct Adjacency {
    pub starts: Vec<u32>,
    pub neighbours: Vec<u32>,
}

impl Adjacency {
    pub fn from_edges(vertices: usize, edges: &[[u32; 2]]) -> Adjacency {
        let mut degree = vec![0u32; vertices + 1];
        for e in edges {
            degree[e[0] as usize] += 1;
            degree[e[1] as usize] += 1;
        }
        let mut starts = vec![0u32; vertices + 1];
        for i in 0..vertices {
            starts[i + 1] = starts[i] + degree[i];
        }
        let mut fill = starts.clone();
        let mut neighbours = vec![0u32; starts[vertices] as usize];
        for e in edges {
            for (a, b) in [(e[0], e[1]), (e[1], e[0])] {
                neighbours[fill[a as usize] as usize] = b;
                fill[a as usize] += 1;
            }
        }
        Adjacency { starts, neighbours }
    }

    pub fn of(&self, v: usize) -> &[u32] {
        &self.neighbours[self.starts[v] as usize..self.starts[v + 1] as usize]
    }

    pub fn smooth(&self, values: &mut [Point], weights: &mut [f64], iterations: usize, active: &[bool]) {
        let n = values.len();
        let mut next_v = vec![[0.0; 3]; n];
        let mut next_w = vec![0.0; n];
        for _ in 0..iterations {
            for v in 0..n {
                if !active[v] {
                    next_v[v] = values[v];
                    next_w[v] = weights[v];
                    continue;
                }
                let ring = self.of(v);
                let mut sv = values[v];
                let mut sw = weights[v];
                for &u in ring {
                    sv = add(sv, values[u as usize]);
                    sw += weights[u as usize];
                }
                let k = 1.0 / (1 + ring.len()) as f64;
                next_v[v] = scale(sv, k);
                next_w[v] = sw * k;
            }
            values.copy_from_slice(&next_v);
            weights.copy_from_slice(&next_w);
        }
    }
}

pub fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub const REGIONS: [&str; 7] = ["eyes", "nose", "mouth", "ears", "cheeks/jaw", "skull", "neck"];

pub fn region(rel_mm: Point, eye_gap_mm: f64) -> usize {
    let m = [rel_mm[0].abs(), rel_mm[1], rel_mm[2]];
    let in_box = |low: Point, high: Point| {
        let o = [
            (low[0] - m[0]).max(m[0] - high[0]).max(0.0),
            (low[1] - m[1]).max(m[1] - high[1]).max(0.0),
            (low[2] - m[2]).max(m[2] - high[2]).max(0.0),
        ];
        norm(o) < 6.0
    };
    if eye_gap_mm < 28.0 {
        return 0;
    }
    if in_box([0.0, -60.0, -8.0], [20.0, 30.0, 45.0]) {
        return 1;
    }
    if in_box([0.0, -60.0, -55.0], [32.0, 40.0, -8.0]) {
        return 2;
    }
    if in_box([60.0, 40.0, -10.0], [95.0, 110.0, 50.0]) {
        return 3;
    }
    if rel_mm[2] < -82.5 {
        return 6;
    }
    if rel_mm[2] > 52.5 || (rel_mm[1] > 117.5 && rel_mm[2] > -40.0) {
        return 5;
    }
    4
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct Spread {
    pub count: usize,
    pub mean_mm: f64,
    pub p95_mm: f64,
    pub max_mm: f64,
}

pub fn spread_mm(values_mm: &[f64]) -> Spread {
    if values_mm.is_empty() {
        return Spread::default();
    }
    let mut v = values_mm.to_vec();
    v.sort_by(f64::total_cmp);
    let p95 = v[((v.len() - 1) as f64 * 0.95).round() as usize];
    Spread { count: v.len(), mean_mm: v.iter().sum::<f64>() / v.len() as f64, p95_mm: p95, max_mm: v[v.len() - 1] }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn similarity_recovers_a_known_transform() {
        let truth = Similarity {
            scale: 2.5,
            rotation: [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            translation: [0.1, -0.2, 0.3],
        };
        let src: Vec<Point> = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.3, 0.7, 0.2]];
        let dst: Vec<Point> = src.iter().map(|p| truth.apply(*p)).collect();
        let got = similarity(&src, &dst, &[1.0; 5], true).unwrap();
        for p in &src {
            assert!(norm(sub(got.apply(*p), truth.apply(*p))) < 1e-9);
        }
    }

    #[test]
    fn a_2d_similarity_and_its_inverse_round_trip() {
        let src = [[0.0, 0.0], [1.0, 0.0], [0.0, 2.0]];
        let truth = Similarity2 { scale: 3.0, angle: 0.4, translation: [5.0, -1.0] };
        let dst: Vec<[f64; 2]> = src.iter().map(|p| truth.apply(*p)).collect();
        let got = similarity2(&src, &dst).unwrap();
        let back = got.inverse();
        for p in &src {
            let q = back.apply(got.apply(*p));
            assert!((q[0] - p[0]).abs() < 1e-9 && (q[1] - p[1]).abs() < 1e-9);
        }
        assert!((got.scale - 3.0).abs() < 1e-9 && (got.angle - 0.4).abs() < 1e-9);
    }

    #[test]
    fn bounded_least_squares_respects_the_box() {
        let gram = [2.0, 0.0, 0.0, 2.0];
        let rhs = [4.0, -1.0];
        let w = bounded_least_squares(&gram, &rhs, 0.0, 1.0, &[0.5, 0.5], 500);
        assert!((w[0] - 1.0).abs() < 1e-9 && w[1].abs() < 1e-9);
    }

    #[test]
    fn the_grid_finds_the_nearest_point() {
        let pts: Vec<Point> = (0..1000).map(|i| [(i % 10) as f64 * 0.01, ((i / 10) % 10) as f64 * 0.01, (i / 100) as f64 * 0.01]).collect();
        let grid = PointGrid::new(pts, 0.02);
        let (i, d) = grid.nearest([0.051, 0.049, 0.031], 0.05).unwrap();
        assert_eq!(grid.points()[i], [0.05, 0.05, 0.03]);
        assert!(d < 0.002);
    }

    #[test]
    fn an_orthographic_camera_is_recovered() {
        let pts: Vec<Point> = vec![[0.0, 0.0, 0.0], [0.1, 0.0, 0.02], [0.0, 0.1, 0.01], [0.05, 0.03, 0.1], [-0.04, 0.02, 0.05]];
        let truth = OrthoCamera { scale: 4000.0, rows: [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0]], translation: [600.0, 500.0] };
        let px: Vec<[f64; 2]> = pts.iter().map(|p| truth.project(*p)).collect();
        let got = fit_ortho_camera(&pts, &px).unwrap();
        for (p, u) in pts.iter().zip(&px) {
            let q = got.project(*p);
            assert!((q[0] - u[0]).abs() < 1e-6 && (q[1] - u[1]).abs() < 1e-6);
        }
    }
}
