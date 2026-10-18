use serde::Serialize;

use crate::decimate::Decimated;
use crate::quadric::{self, Quadric};

#[derive(Default, Serialize)]
pub struct PlacementStats {
    pub vertices: usize,
    pub boundary_kept: usize,
    pub fallbacks: usize,
    pub backed_off: usize,
    pub median_move_mm: f64,
    pub max_move_mm: f64,
    pub fit_rounds: Vec<f64>,
    pub repaired: usize,
}

#[derive(Clone, Serialize)]
pub struct FitSettings {
    pub iterations: usize,
    pub smooth: f64,
    pub max_move: f64,
    pub min_angle_deg: f64,
    pub guard_steps: usize,
    pub repair_below_deg: f64,
    pub trim: f64,
}

#[derive(Default, Serialize)]
pub struct Measure {
    pub proxy_mean_mm: f64,
    pub proxy_p95_mm: f64,
    pub proxy_max_mm: f64,
    pub proxy_over_half_mm: usize,
    pub under5: usize,
    pub under2: usize,
    pub caps170: usize,
    pub under5_outside_box: usize,
    pub orientation: Orientation,
}

pub(crate) fn boundary_vertices(mesh: &Decimated) -> Vec<bool> {
    let mut keys: Vec<u64> = mesh
        .triangles
        .iter()
        .flat_map(|t| (0..3).map(move |e| ((t[e].min(t[(e + 1) % 3]) as u64) << 32) | t[e].max(t[(e + 1) % 3]) as u64))
        .collect();
    keys.sort_unstable();
    let mut flags = vec![false; mesh.kept.len()];
    let mut i = 0;
    while i < keys.len() {
        let mut j = i + 1;
        while j < keys.len() && keys[j] == keys[i] {
            j += 1;
        }
        if j - i == 1 {
            flags[(keys[i] >> 32) as usize] = true;
            flags[(keys[i] & 0xffff_ffff) as usize] = true;
        }
        i = j;
    }
    flags
}

pub(crate) fn solve(q: &Quadric, anchor: [f64; 3], regular: f64) -> Option<[f64; 3]> {
    let a = [[q[0] + regular, q[1], q[2]], [q[1], q[4] + regular, q[5]], [q[2], q[5], q[7] + regular]];
    let b = [q[3] - regular * anchor[0], q[6] - regular * anchor[1], q[8] - regular * anchor[2]];
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if det.abs() < 1e-30 {
        return None;
    }
    let column = |c: usize| -> f64 {
        let mut m = a;
        for r in 0..3 {
            m[r][c] = -b[r];
        }
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    Some([column(0) / det, column(1) / det, column(2) / det])
}

pub(crate) fn cluster_quadrics(positions: &[[f64; 3]], dense_triangles: &[[u32; 3]], mesh: &Decimated) -> ([f64; 3], Vec<[f64; 3]>, Vec<Quadric>) {
    let n = positions.len();
    let centre = positions.iter().fold([0.0; 3], |c, p| [c[0] + p[0], c[1] + p[1], c[2] + p[2]]).map(|x| x / n as f64);
    let local: Vec<[f64; 3]> = positions.iter().map(|&p| quadric::sub(p, centre)).collect();
    let mut sums: Vec<Quadric> = vec![[0.0; 10]; mesh.kept.len()];
    for tri in dense_triangles {
        let p = tri.map(|i| local[i as usize]);
        let normal = quadric::cross(quadric::sub(p[1], p[0]), quadric::sub(p[2], p[0]));
        let area2 = quadric::length(normal);
        if area2 < 1e-20 {
            continue;
        }
        let unit = normal.map(|x| x / area2);
        let q = quadric::from_plane(unit, -quadric::dot(unit, p[0]), area2 * 0.5);
        for &corner in tri {
            let g = mesh.owner[corner as usize];
            if g != u32::MAX {
                quadric::add(&mut sums[g as usize], &q);
            }
        }
    }
    (centre, local, sums)
}

pub(crate) fn regular_of(q: &Quadric) -> f64 {
    1e-4 * (q[0] + q[4] + q[7]) / 3.0
}

pub fn optimal(positions: &[[f64; 3]], dense_triangles: &[[u32; 3]], mesh: &Decimated, max_move: f64) -> (Vec<[f64; 3]>, PlacementStats) {
    let (centre, local, sums) = cluster_quadrics(positions, dense_triangles, mesh);
    let boundary = boundary_vertices(mesh);
    let mut stats = PlacementStats { vertices: mesh.kept.len(), ..Default::default() };
    let out: Vec<[f64; 3]> = mesh
        .kept
        .iter()
        .enumerate()
        .map(|(g, &d)| {
            let anchor = local[d as usize];
            if boundary[g] {
                stats.boundary_kept += 1;
                return positions[d as usize];
            }
            let q = &sums[g];
            match solve(q, anchor, regular_of(q)) {
                Some(x) if quadric::length(quadric::sub(x, anchor)) <= max_move => [x[0] + centre[0], x[1] + centre[1], x[2] + centre[2]],
                _ => {
                    stats.fallbacks += 1;
                    positions[d as usize]
                }
            }
        })
        .collect();
    let subset: Vec<[f64; 3]> = mesh.kept.iter().map(|&d| positions[d as usize]).collect();
    let mut out: Vec<[f64; 3]> = out;
    let rule = Guard { floor_deg: 5.0, cap: false, steps: vec![0.5, 0.25, 0.0], rounds: 4 };
    stats.backed_off = guard(&mut out, &subset, mesh, &boundary, &rule);
    finish_moves(&mut stats, &out, &subset);
    (out, stats)
}

pub(crate) fn finish_moves(stats: &mut PlacementStats, out: &[[f64; 3]], subset: &[[f64; 3]]) {
    let mut moves: Vec<f64> = out.iter().zip(subset).map(|(a, b)| quadric::length(quadric::sub(*a, *b))).collect();
    moves.sort_by(f64::total_cmp);
    if !moves.is_empty() {
        stats.median_move_mm = moves[moves.len() / 2] * 1000.0;
        stats.max_move_mm = moves[moves.len() - 1] * 1000.0;
    }
}

pub(crate) fn min_angle(p: [[f64; 3]; 3]) -> f64 {
    quadric::corner_angles(p).iter().cloned().fold(180.0, f64::min)
}

fn sound(tri: [u32; 3], at: &[[f64; 3]], reference: &[[f64; 3]], rule: &Guard) -> bool {
    let p = tri.map(|i| at[i as usize]);
    let r = tri.map(|i| reference[i as usize]);
    let n = quadric::cross(quadric::sub(p[1], p[0]), quadric::sub(p[2], p[0]));
    let m = quadric::cross(quadric::sub(r[1], r[0]), quadric::sub(r[2], r[0]));
    let (ln, lm) = (quadric::length(n), quadric::length(m));
    if ln < 1e-20 || quadric::dot(n, m) < 0.5 * ln * lm {
        return false;
    }
    let corners = quadric::corner_angles(p);
    let before = quadric::corner_angles(r);
    let now_min = corners.iter().cloned().fold(180.0, f64::min);
    let before_min = before.iter().cloned().fold(180.0, f64::min);
    let now_max = corners.iter().cloned().fold(0.0, f64::max);
    let before_max = before.iter().cloned().fold(0.0, f64::max);
    now_min >= before_min.min(rule.floor_deg) - 1e-9 && (!rule.cap || now_max <= before_max.max(170.0) + 1e-9)
}

pub(crate) fn vertex_triangles(mesh: &Decimated, count: usize) -> Vec<Vec<u32>> {
    let mut vtris: Vec<Vec<u32>> = vec![Vec::new(); count];
    for (t, tri) in mesh.triangles.iter().enumerate() {
        for &g in tri {
            vtris[g as usize].push(t as u32);
        }
    }
    vtris
}

pub(crate) struct Guard {
    pub(crate) floor_deg: f64,
    pub(crate) cap: bool,
    pub(crate) steps: Vec<f64>,
    pub(crate) rounds: usize,
}

fn guard(out: &mut [[f64; 3]], subset: &[[f64; 3]], mesh: &Decimated, boundary: &[bool], rule: &Guard) -> usize {
    guard_by(out, subset, mesh, boundary, rule, |t, at| sound(mesh.triangles[t as usize], at, subset, rule))
}

pub(crate) fn guard_by<F: Fn(u32, &[[f64; 3]]) -> bool>(
    out: &mut [[f64; 3]],
    subset: &[[f64; 3]],
    mesh: &Decimated,
    boundary: &[bool],
    rule: &Guard,
    sound_triangle: F,
) -> usize {
    let vtris = vertex_triangles(mesh, out.len());
    let target: Vec<[f64; 3]> = out.to_vec();
    let mut backed_off = vec![false; out.len()];
    for _ in 0..rule.rounds {
        let mut changed = 0;
        for g in 0..out.len() {
            if boundary[g] {
                continue;
            }
            let ok = |at: &[[f64; 3]]| vtris[g].iter().all(|&t| sound_triangle(t, at));
            if ok(out) {
                continue;
            }
            backed_off[g] = true;
            let mut placed = false;
            for &step in &rule.steps {
                let candidate = [0, 1, 2].map(|i| subset[g][i] + step * (target[g][i] - subset[g][i]));
                let keep = out[g];
                out[g] = candidate;
                if ok(out) {
                    placed = true;
                    break;
                }
                out[g] = keep;
            }
            if !placed {
                out[g] = subset[g];
            }
            changed += 1;
        }
        if changed == 0 {
            break;
        }
    }
    backed_off.iter().filter(|&&b| b).count()
}

pub(crate) fn closest(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> ([f64; 3], f64) {
    let ab = quadric::sub(b, a);
    let ac = quadric::sub(c, a);
    let ap = quadric::sub(p, a);
    let (d1, d2) = (quadric::dot(ab, ap), quadric::dot(ac, ap));
    let w = 'w: {
        if d1 <= 0.0 && d2 <= 0.0 {
            break 'w [1.0, 0.0, 0.0];
        }
        let bp = quadric::sub(p, b);
        let (d3, d4) = (quadric::dot(ab, bp), quadric::dot(ac, bp));
        if d3 >= 0.0 && d4 <= d3 {
            break 'w [0.0, 1.0, 0.0];
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            let v = d1 / (d1 - d3);
            break 'w [1.0 - v, v, 0.0];
        }
        let cp = quadric::sub(p, c);
        let (d5, d6) = (quadric::dot(ab, cp), quadric::dot(ac, cp));
        if d6 >= 0.0 && d5 <= d6 {
            break 'w [0.0, 0.0, 1.0];
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            let t = d2 / (d2 - d6);
            break 'w [1.0 - t, 0.0, t];
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            let t = (d4 - d3) / ((d4 - d3) + (d5 - d6));
            break 'w [0.0, 1.0 - t, t];
        }
        let total = va + vb + vc;
        if total.abs() < 1e-300 {
            break 'w [1.0, 0.0, 0.0];
        }
        let (v, t) = (vb / total, vc / total);
        [1.0 - v - t, v, t]
    };
    let q = [0, 1, 2].map(|i| w[0] * a[i] + w[1] * b[i] + w[2] * c[i]);
    let d = quadric::sub(p, q);
    (w, quadric::dot(d, d))
}

pub(crate) fn ring_triangles(mesh: &Decimated, vtris: &[Vec<u32>]) -> Vec<Vec<u32>> {
    (0..vtris.len())
        .map(|g| {
            let mut out: Vec<u32> = Vec::with_capacity(32);
            for &t in &vtris[g] {
                for &w in &mesh.triangles[t as usize] {
                    for &s in &vtris[w as usize] {
                        if !out.contains(&s) {
                            out.push(s);
                        }
                    }
                }
            }
            out
        })
        .collect()
}

pub(crate) fn project(points: &[[f64; 3]], owner: &[u32], game: &[[f64; 3]], mesh: &Decimated, rings: &[Vec<u32>], stride: usize) -> Vec<(u32, [f64; 3], f64)> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    let indices: Vec<usize> = (0..points.len()).step_by(stride).collect();
    let chunk = indices.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = indices
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .map(|&i| {
                            let g = owner[i];
                            if g == u32::MAX {
                                return (u32::MAX, [0.0; 3], 0.0);
                            }
                            let mut best = (u32::MAX, [0.0; 3], f64::INFINITY);
                            for &t in &rings[g as usize] {
                                let tri = mesh.triangles[t as usize];
                                let (w, d2) = closest(points[i], game[tri[0] as usize], game[tri[1] as usize], game[tri[2] as usize]);
                                if d2 < best.2 {
                                    best = (t, w, d2);
                                }
                            }
                            best
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}

pub(crate) struct Sparse {
    pub(crate) cols: Vec<Vec<u32>>,
    pub(crate) vals: Vec<Vec<f64>>,
}

impl Sparse {
    pub(crate) fn new(mesh: &Decimated, count: usize) -> Sparse {
        let mut cols: Vec<Vec<u32>> = (0..count as u32).map(|i| vec![i]).collect();
        for tri in &mesh.triangles {
            for e in 0..3 {
                let (a, b) = (tri[e], tri[(e + 1) % 3]);
                if !cols[a as usize].contains(&b) {
                    cols[a as usize].push(b);
                    cols[b as usize].push(a);
                }
            }
        }
        let vals = cols.iter().map(|c| vec![0.0; c.len()]).collect();
        Sparse { cols, vals }
    }

    pub(crate) fn add(&mut self, r: u32, c: u32, v: f64) {
        let k = self.cols[r as usize].iter().position(|&x| x == c).unwrap();
        self.vals[r as usize][k] += v;
    }

    fn mul(&self, x: &[f64], y: &mut [f64]) {
        for (r, out) in y.iter_mut().enumerate() {
            *out = self.cols[r].iter().zip(&self.vals[r]).map(|(&c, &v)| v * x[c as usize]).sum();
        }
    }
}

pub(crate) fn conjugate_gradient(a: &Sparse, b: &[f64], x: &mut [f64]) {
    let n = b.len();
    let inverse_diagonal: Vec<f64> = (0..n).map(|r| 1.0 / a.vals[r][0].max(1e-300)).collect();
    let mut ax = vec![0.0; n];
    a.mul(x, &mut ax);
    let mut r: Vec<f64> = (0..n).map(|i| b[i] - ax[i]).collect();
    let mut z: Vec<f64> = (0..n).map(|i| r[i] * inverse_diagonal[i]).collect();
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let norm_b = b.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-300);
    let mut ap = vec![0.0; n];
    for _ in 0..2000 {
        if r.iter().map(|v| v * v).sum::<f64>().sqrt() < 1e-12 * norm_b {
            break;
        }
        a.mul(&p, &mut ap);
        let alpha = rz / p.iter().zip(&ap).map(|(a, b)| a * b).sum::<f64>().max(1e-300);
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
            z[i] = r[i] * inverse_diagonal[i];
        }
        let rz_next: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let beta = rz_next / rz;
        rz = rz_next;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
}

pub(crate) fn unique_edges(mesh: &Decimated) -> Vec<(u32, u32)> {
    let mut edges: Vec<(u32, u32)> =
        mesh.triangles.iter().flat_map(|t| (0..3).map(move |e| (t[e].min(t[(e + 1) % 3]), t[e].max(t[(e + 1) % 3])))).collect();
    edges.sort_unstable();
    edges.dedup();
    edges
}

pub fn fit(positions: &[[f64; 3]], dense_triangles: &[[u32; 3]], mesh: &Decimated, settings: &FitSettings) -> (Vec<[f64; 3]>, PlacementStats) {
    let (mut game, mut stats) = optimal(positions, dense_triangles, mesh, settings.max_move);
    let subset: Vec<[f64; 3]> = mesh.kept.iter().map(|&d| positions[d as usize]).collect();
    let boundary = boundary_vertices(mesh);
    let count = mesh.kept.len();
    let vtris = vertex_triangles(mesh, count);
    let rings = ring_triangles(mesh, &vtris);
    fit_rounds(positions, mesh, &mut game, &subset, &boundary, &rings, settings, &mut stats);
    clamp_moves(&mut game, &subset, settings.max_move);
    let steps: Vec<f64> = (1..=settings.guard_steps).map(|k| 1.0 - k as f64 / settings.guard_steps as f64).collect();
    let rule = Guard { floor_deg: settings.min_angle_deg, cap: true, steps, rounds: 8 };
    stats.backed_off = guard(&mut game, &subset, mesh, &boundary, &rule);
    if settings.repair_below_deg > 0.0 {
        stats.repaired = repair(&mut game, mesh, &boundary, &vtris, settings.repair_below_deg);
    }
    finish_moves(&mut stats, &game, &subset);
    (game, stats)
}

pub(crate) fn clamp_moves(game: &mut [[f64; 3]], subset: &[[f64; 3]], max_move: f64) -> usize {
    let mut clamped = 0;
    for g in 0..game.len() {
        let d = quadric::sub(game[g], subset[g]);
        let l = quadric::length(d);
        if l > max_move {
            game[g] = [0, 1, 2].map(|k| subset[g][k] + d[k] * max_move / l);
            clamped += 1;
        }
    }
    clamped
}


pub(crate) fn fit_rounds(
    positions: &[[f64; 3]],
    mesh: &Decimated,
    game: &mut [[f64; 3]],
    subset: &[[f64; 3]],
    boundary: &[bool],
    rings: &[Vec<u32>],
    settings: &FitSettings,
    stats: &mut PlacementStats,
) {
    let count = mesh.kept.len();
    let edges = unique_edges(mesh);
    for _ in 0..settings.iterations {
        let hits = project(positions, &mesh.owner, &game, mesh, &rings, 1);
        let mean = hits.iter().filter(|h| h.0 != u32::MAX).map(|h| h.2.sqrt()).sum::<f64>() / hits.len() as f64;
        stats.fit_rounds.push(mean * 1000.0);
        let mut a = Sparse::new(mesh, count);
        let mut rhs = vec![[0.0f64; 3]; count];
        for (i, &(t, w, d2)) in hits.iter().enumerate() {
            if t == u32::MAX || (settings.trim > 0.0 && d2 > settings.trim * settings.trim) {
                continue;
            }
            let tri = mesh.triangles[t as usize];
            let mut target = positions[i];
            for s in 0..3 {
                if boundary[tri[s] as usize] {
                    for k in 0..3 {
                        target[k] -= w[s] * game[tri[s] as usize][k];
                    }
                }
            }
            for r in 0..3 {
                if boundary[tri[r] as usize] {
                    continue;
                }
                for k in 0..3 {
                    rhs[tri[r] as usize][k] += w[r] * target[k];
                }
                for s in 0..3 {
                    if !boundary[tri[s] as usize] {
                        a.add(tri[r], tri[s], w[r] * w[s]);
                    }
                }
            }
        }
        let lambda = settings.smooth;
        for &(p, q) in &edges {
            let e = quadric::sub(subset[p as usize], subset[q as usize]);
            let (fp, fq) = (boundary[p as usize], boundary[q as usize]);
            if !fp {
                a.add(p, p, lambda);
                for k in 0..3 {
                    rhs[p as usize][k] += lambda * (e[k] + if fq { game[q as usize][k] } else { 0.0 });
                }
                if !fq {
                    a.add(p, q, -lambda);
                }
            }
            if !fq {
                a.add(q, q, lambda);
                for k in 0..3 {
                    rhs[q as usize][k] += lambda * (-e[k] + if fp { game[p as usize][k] } else { 0.0 });
                }
                if !fp {
                    a.add(q, p, -lambda);
                }
            }
        }
        let mean_diagonal = (0..count).map(|r| a.vals[r][0]).sum::<f64>() / count as f64;
        let epsilon = 1e-6 * mean_diagonal;
        for g in 0..count {
            if boundary[g] {
                a.vals[g][0] = 1.0;
                rhs[g] = game[g];
            } else {
                a.add(g as u32, g as u32, epsilon);
                for k in 0..3 {
                    rhs[g][k] += epsilon * game[g][k];
                }
            }
        }
        for k in 0..3 {
            let b: Vec<f64> = rhs.iter().map(|r| r[k]).collect();
            let mut x: Vec<f64> = game.iter().map(|p| p[k]).collect();
            conjugate_gradient(&a, &b, &mut x);
            for g in 0..count {
                game[g][k] = x[g];
            }
        }
    }
}

pub(crate) fn triangle_normal(p: [[f64; 3]; 3]) -> [f64; 3] {
    quadric::cross(quadric::sub(p[1], p[0]), quadric::sub(p[2], p[0]))
}

pub(crate) fn star(game: &[[f64; 3]], mesh: &Decimated, tris: &[u32]) -> (f64, f64) {
    tris.iter().fold((180.0f64, 0.0f64), |(lo, hi), &t| {
        let corners = quadric::corner_angles(mesh.triangles[t as usize].map(|i| game[i as usize]));
        (corners.iter().cloned().fold(lo, f64::min), corners.iter().cloned().fold(hi, f64::max))
    })
}

fn repair(game: &mut [[f64; 3]], mesh: &Decimated, boundary: &[bool], vtris: &[Vec<u32>], below_deg: f64) -> usize {
    repair_by(game, mesh, boundary, vtris, below_deg, |_, _| true)
}

pub(crate) fn repair_by<A: Fn(&[[f64; 3]], &[u32]) -> bool>(
    game: &mut [[f64; 3]],
    mesh: &Decimated,
    boundary: &[bool],
    vtris: &[Vec<u32>],
    below_deg: f64,
    accept: A,
) -> usize {
    let cos_turn = 30f64.to_radians().cos();
    let mut moved = vec![false; game.len()];
    for _ in 0..16 {
        let mut changed = 0;
        for t in 0..mesh.triangles.len() {
            let tri = mesh.triangles[t];
            if min_angle(tri.map(|i| game[i as usize])) >= below_deg {
                continue;
            }
            let mut best: Option<(f64, u32, [f64; 3])> = None;
            for &g in &tri {
                if boundary[g as usize] {
                    continue;
                }
                let ring = &vtris[g as usize];
                let (old_lo, old_hi) = star(game, mesh, ring);
                let normals: Vec<[f64; 3]> = ring.iter().map(|&s| triangle_normal(mesh.triangles[s as usize].map(|i| game[i as usize]))).collect();
                let mut n = normals.iter().fold([0.0; 3], |a, b| [a[0] + b[0], a[1] + b[1], a[2] + b[2]]);
                let ln = quadric::length(n).max(1e-30);
                n = n.map(|x| x / ln);
                let mut centroid = [0.0; 3];
                let mut count = 0.0;
                for &s in ring {
                    for &w in &mesh.triangles[s as usize] {
                        if w != g {
                            for k in 0..3 {
                                centroid[k] += game[w as usize][k];
                            }
                            count += 1.0;
                        }
                    }
                }
                let centroid = centroid.map(|x| x / count);
                let x = game[g as usize];
                let d = quadric::sub(centroid, x);
                let d = quadric::sub(d, n.map(|c| c * quadric::dot(d, n)));
                for f in [0.1, 0.2, 0.35, 0.5, 0.75, 1.0] {
                    game[g as usize] = [0, 1, 2].map(|k| x[k] + f * d[k]);
                    let (lo, hi) = star(game, mesh, ring);
                    let upright = ring.iter().zip(&normals).all(|(&s, m)| {
                        let now = triangle_normal(mesh.triangles[s as usize].map(|i| game[i as usize]));
                        quadric::dot(now, *m) >= cos_turn * quadric::length(now) * quadric::length(*m)
                    });
                    if upright && lo > old_lo + 1e-6 && hi <= old_hi.max(170.0) && best.is_none_or(|b| lo - old_lo > b.0) && accept(game, ring) {
                        best = Some((lo - old_lo, g, game[g as usize]));
                    }
                }
                game[g as usize] = x;
            }
            if let Some((_, g, at)) = best {
                game[g as usize] = at;
                moved[g as usize] = true;
                changed += 1;
            }
        }
        if changed == 0 {
            break;
        }
    }
    moved.iter().filter(|&&m| m).count()
}

pub(crate) fn angle_weighted_normals(positions: &[[f64; 3]], triangles: &[[u32; 3]]) -> Vec<[f64; 3]> {
    let mut out = vec![[0.0f64; 3]; positions.len()];
    for tri in triangles {
        let p = tri.map(|i| positions[i as usize]);
        let n = triangle_normal(p);
        let unit = n.map(|x| x / quadric::length(n).max(1e-30));
        let angles = quadric::corner_angles(p);
        for c in 0..3 {
            let a = angles[c].to_radians();
            for k in 0..3 {
                out[tri[c] as usize][k] += unit[k] * a;
            }
        }
    }
    for n in &mut out {
        let l = quadric::length(*n).max(1e-30);
        *n = n.map(|x| x / l);
    }
    out
}

pub(crate) fn triangle_references(mesh: &Decimated, kept_normals: &[[f64; 3]]) -> Vec<[f64; 3]> {
    mesh.triangles
        .iter()
        .map(|tri| {
            let s = tri.iter().fold([0.0; 3], |a, &g| {
                let n = kept_normals[g as usize];
                [a[0] + n[0], a[1] + n[1], a[2] + n[2]]
            });
            let l = quadric::length(s).max(1e-30);
            s.map(|x| x / l)
        })
        .collect()
}

pub(crate) fn deviation_deg(p: [[f64; 3]; 3], reference: [f64; 3]) -> f64 {
    let n = triangle_normal(p);
    let unit = n.map(|x| x / quadric::length(n).max(1e-30));
    quadric::dot(unit, reference).clamp(-1.0, 1.0).acos().to_degrees()
}

#[derive(Default, Serialize)]
pub struct Orientation {
    pub tip: [f64; 3],
    pub over30: usize,
    pub over60: usize,
    pub over90: usize,
    pub box_over30: usize,
    pub box_over60: usize,
    pub box_over90: usize,
    pub front_over30: usize,
    pub front_over60: usize,
    pub front_over90: usize,
    pub front_worst: Vec<(u32, f64)>,
}

pub fn orientation(positions: &[[f64; 3]], dense_triangles: &[[u32; 3]], mesh: &Decimated, game: &[[f64; 3]], tip: [f64; 3]) -> Orientation {
    let normals = angle_weighted_normals(positions, dense_triangles);
    let kept_normals: Vec<[f64; 3]> = mesh.kept.iter().map(|&d| normals[d as usize]).collect();
    let references = triangle_references(mesh, &kept_normals);
    let mut o = Orientation { tip, ..Default::default() };
    let mut front: Vec<(u32, f64)> = Vec::new();
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let p = tri.map(|i| game[i as usize]);
        let dev = deviation_deg(p, references[t]);
        let rel = [0, 1, 2].map(|k| ((p[0][k] + p[1][k] + p[2][k]) / 3.0 - tip[k]) * 1000.0);
        let in_box = rel[0].abs() < 45.0 && rel[2] > -10.0 && rel[2] < 50.0 && rel[1] < 45.0;
        let in_front = !in_box && rel[1] < 60.0 && rel[0].abs() < 75.0 && rel[2] > -80.0 && rel[2] < 80.0;
        let counts = [dev > 30.0, dev > 60.0, dev > 90.0];
        o.over30 += counts[0] as usize;
        o.over60 += counts[1] as usize;
        o.over90 += counts[2] as usize;
        if in_box {
            o.box_over30 += counts[0] as usize;
            o.box_over60 += counts[1] as usize;
            o.box_over90 += counts[2] as usize;
        }
        if in_front {
            o.front_over30 += counts[0] as usize;
            o.front_over60 += counts[1] as usize;
            o.front_over90 += counts[2] as usize;
            front.push((t as u32, dev));
        }
    }
    front.sort_by(|a, b| b.1.total_cmp(&a.1));
    o.front_worst = front.into_iter().take(6).collect();
    o
}

pub fn measure(positions: &[[f64; 3]], dense_triangles: &[[u32; 3]], mesh: &Decimated, game: &[[f64; 3]], tip: [f64; 3]) -> Measure {
    let vtris = vertex_triangles(mesh, game.len());
    let rings = ring_triangles(mesh, &vtris);
    let hits = project(positions, &mesh.owner, game, mesh, &rings, (positions.len() / 300_000).max(1));
    let mut gaps: Vec<f64> = hits.iter().filter(|h| h.0 != u32::MAX).map(|h| h.2.sqrt() * 1000.0).collect();
    gaps.sort_by(f64::total_cmp);
    let mut m = Measure { orientation: orientation(positions, dense_triangles, mesh, game, tip), ..Default::default() };
    if !gaps.is_empty() {
        m.proxy_mean_mm = gaps.iter().sum::<f64>() / gaps.len() as f64;
        m.proxy_p95_mm = gaps[((gaps.len() - 1) as f64 * 0.95) as usize];
        m.proxy_max_mm = gaps[gaps.len() - 1];
        m.proxy_over_half_mm = gaps.iter().filter(|&&g| g > 0.5).count();
    }
    for tri in &mesh.triangles {
        let p = tri.map(|i| game[i as usize]);
        let corners = quadric::corner_angles(p);
        let small = min_angle(p);
        if corners.iter().any(|&c| c > 170.0) {
            m.caps170 += 1;
        }
        if small < 2.0 {
            m.under2 += 1;
        }
        if small < 5.0 {
            m.under5 += 1;
            let rel = [0, 1, 2].map(|k| ((p[0][k] + p[1][k] + p[2][k]) / 3.0 - tip[k]) * 1000.0);
            let thin = rel[0].abs() < 45.0 && rel[2] > -10.0 && rel[2] < 50.0 && rel[1] < 45.0;
            if !thin {
                m.under5_outside_box += 1;
            }
        }
    }
    m
}
