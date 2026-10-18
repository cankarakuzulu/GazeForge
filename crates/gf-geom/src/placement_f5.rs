use serde::Serialize;

use crate::Uv;
use crate::decimate::Decimated;
use crate::lids::Eye;
use crate::placement::{self as pl, Csr};
use crate::prepared::Prepared;
use crate::quadric::{dot, length, sub};
use crate::uv_chart::{self, Surface};

#[derive(Clone, Serialize)]
pub struct F5Settings {
    pub tangential: f64,
    pub smooth: f64,
    pub membrane: f64,
    pub max_move: f64,
    pub uv_ring: usize,
    pub subset_normals: bool,
    pub eye_clearance: f64,
    pub eye_depth: [f64; 2],
    pub eye_front_deg: f64,
}

#[derive(Default, Serialize)]
pub struct F5Stats {
    pub vertices: usize,
    pub boundary: usize,
    pub seam: usize,
    pub footprint_points: usize,
    pub footprint_outside: usize,
    pub solver_iterations: Vec<usize>,
    pub point_gap_mm: f64,
    pub normal_gap_mm: f64,
    pub seam_offset_max_mm: f64,
    pub clamped: usize,
    pub median_move_mm: f64,
    pub max_move_mm: f64,
    pub clear: Option<LiftStats>,
    pub uv_keys_derived: usize,
    pub uv_seed_misses: usize,
    pub uv_flipped_before: usize,
    pub uv_reverted_vertices: usize,
    pub uv_flipped_after: usize,
}

pub struct Footprint {
    pub triangle: Vec<u32>,
    pub weights: Vec<[f64; 3]>,
    pub outside: usize,
}

type Block = [f64; 9];

const IDENTITY: Block = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

fn scaled(m: &Block, s: f64) -> Block {
    m.map(|x| x * s)
}

fn apply(m: &Block, v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|r| m[3 * r] * v[0] + m[3 * r + 1] * v[1] + m[3 * r + 2] * v[2])
}

fn outer_plus(n: [f64; 3], diagonal: f64) -> Block {
    let mut m = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            m[3 * r + c] = n[r] * n[c] + if r == c { diagonal } else { 0.0 };
        }
    }
    m
}

fn tangential_projector(n: [f64; 3]) -> Block {
    let mut m = outer_plus(n, 0.0);
    for (k, x) in m.iter_mut().enumerate() {
        *x = if k % 4 == 0 { 1.0 } else { 0.0 } - *x;
    }
    m
}

fn inverse(m: &Block) -> Block {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6]) + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det.abs() < 1e-300 {
        return IDENTITY;
    }
    let c = [
        m[4] * m[8] - m[5] * m[7],
        m[2] * m[7] - m[1] * m[8],
        m[1] * m[5] - m[2] * m[4],
        m[5] * m[6] - m[3] * m[8],
        m[0] * m[8] - m[2] * m[6],
        m[2] * m[3] - m[0] * m[5],
        m[3] * m[7] - m[4] * m[6],
        m[1] * m[6] - m[0] * m[7],
        m[0] * m[4] - m[1] * m[3],
    ];
    c.map(|x| x / det)
}

struct Blocks<'a> {
    cols: &'a [Vec<u32>],
    vals: Vec<Vec<Block>>,
}

pub(crate) fn block_pattern(edges: &[(u32, u32)], count: usize) -> Vec<Vec<u32>> {
    let mut cols: Vec<Vec<u32>> = (0..count as u32).map(|i| vec![i]).collect();
    for &(a, b) in edges {
        cols[a as usize].push(b);
        cols[b as usize].push(a);
    }
    cols
}

fn add_block(v: &mut Block, m: &Block) {
    for i in 0..9 {
        v[i] += m[i];
    }
}

impl Blocks<'_> {
    fn mul(&self, x: &[[f64; 3]], y: &mut [[f64; 3]]) {
        let rows: Vec<[f64; 3]> = crate::map_range(y.len(), |r| {
            let mut s = [0.0; 3];
            for (&c, m) in self.cols[r].iter().zip(&self.vals[r]) {
                let p = apply(m, x[c as usize]);
                for k in 0..3 {
                    s[k] += p[k];
                }
            }
            s
        });
        y.copy_from_slice(&rows);
    }
}

fn dot3(a: &[[f64; 3]], b: &[[f64; 3]]) -> f64 {
    a.iter().zip(b).map(|(p, q)| p[0] * q[0] + p[1] * q[1] + p[2] * q[2]).sum()
}

fn conjugate_gradient(a: &Blocks, b: &[[f64; 3]], x: &mut [[f64; 3]]) -> usize {
    let n = b.len();
    let pre: Vec<Block> = (0..n).map(|r| inverse(&a.vals[r][0])).collect();
    let mut ax = vec![[0.0; 3]; n];
    a.mul(x, &mut ax);
    let mut r: Vec<[f64; 3]> = (0..n).map(|i| [0, 1, 2].map(|k| b[i][k] - ax[i][k])).collect();
    let mut z: Vec<[f64; 3]> = (0..n).map(|i| apply(&pre[i], r[i])).collect();
    let mut p = z.clone();
    let mut rz = dot3(&r, &z);
    let norm_b = dot3(b, b).sqrt().max(1e-300);
    let mut ap = vec![[0.0; 3]; n];
    for iteration in 0..5000 {
        if dot3(&r, &r).sqrt() < 1e-13 * norm_b {
            return iteration;
        }
        a.mul(&p, &mut ap);
        let alpha = rz / dot3(&p, &ap).max(1e-300);
        for i in 0..n {
            for k in 0..3 {
                x[i][k] += alpha * p[i][k];
                r[i][k] -= alpha * ap[i][k];
            }
            z[i] = apply(&pre[i], r[i]);
        }
        let rz_next = dot3(&r, &z);
        let beta = rz_next / rz;
        rz = rz_next;
        for i in 0..n {
            for k in 0..3 {
                p[i][k] = z[i][k] + beta * p[i][k];
            }
        }
    }
    5000
}

fn uv_barycentric(p: Uv, t: [Uv; 3]) -> Option<[f64; 3]> {
    let (ax, ay) = (t[0][0] as f64, t[0][1] as f64);
    let (bx, by) = (t[1][0] as f64 - ax, t[1][1] as f64 - ay);
    let (cx, cy) = (t[2][0] as f64 - ax, t[2][1] as f64 - ay);
    let (px, py) = (p[0] as f64 - ax, p[1] as f64 - ay);
    let det = bx * cy - by * cx;
    if det.abs() < 1e-18 {
        return None;
    }
    let v = (px * cy - py * cx) / det;
    let w = (bx * py - by * px) / det;
    Some([1.0 - v - w, v, w])
}

pub fn footprint(dense_count: usize, dense_triangles: &[[u32; 3]], dense_uvs: &[[Uv; 3]], mesh: &Decimated, rings: &[Vec<u32>]) -> Footprint {
    let mut uvs: Vec<Vec<Uv>> = vec![Vec::new(); dense_count];
    for (tri, corner_uvs) in dense_triangles.iter().zip(dense_uvs) {
        for c in 0..3 {
            let list = &mut uvs[tri[c] as usize];
            if !list.contains(&corner_uvs[c]) {
                list.push(corner_uvs[c]);
            }
        }
    }
    let indices: Vec<usize> = (0..dense_count).collect();
    let found: Vec<(u32, [f64; 3], bool)> = crate::map_each(&indices, |i| {
        let g = mesh.owner[i];
        if g == u32::MAX {
            return (u32::MAX, [0.0; 3], false);
        }
        let mut best: Option<(f64, u32, [f64; 3])> = None;
        for &t in &rings[g as usize] {
            for &uv in &uvs[i] {
                if let Some(b) = uv_barycentric(uv, mesh.triangle_uvs[t as usize]) {
                    let score = b[0].min(b[1]).min(b[2]);
                    if best.is_none_or(|x| score > x.0) {
                        best = Some((score, t, b));
                    }
                }
            }
        }
        match best {
            None => (u32::MAX, [0.0; 3], false),
            Some((score, t, b)) => {
                let clamped = b.map(|x| x.max(0.0));
                let total: f64 = clamped.iter().sum();
                (t, clamped.map(|x| x / total.max(1e-300)), score < -1e-9)
            }
        }
    });
    Footprint {
        triangle: found.iter().map(|f| f.0).collect(),
        weights: found.iter().map(|f| f.1).collect(),
        outside: found.iter().filter(|f| f.2).count(),
    }
}

struct System<'a> {
    points: &'a [[f64; 3]],
    normals: &'a [[f64; 3]],
    foot: &'a Footprint,
    mesh: &'a Decimated,
    fixed: &'a [bool],
    subset: &'a [[f64; 3]],
    edges: &'a [(u32, u32)],
    rows: &'a Csr,
    edge_rows: &'a Csr,
    pattern: &'a [Vec<u32>],
    tangential: f64,
    smooth: f64,
    membrane: Vec<Block>,
}

impl System<'_> {
    fn matrix(&self) -> (Blocks<'_>, f64) {
        let count = self.subset.len();
        let mut vals: Vec<Vec<Block>> = crate::map_range(count, |g| {
            let cols = &self.pattern[g];
            let mut v = vec![[0.0; 9]; cols.len()];
            if self.fixed[g] {
                return v;
            }
            let at = |c: u32| cols.iter().position(|&x| x == c).unwrap();
            for &e in self.rows.row(g) {
                let (i, r) = ((e / 3) as usize, (e % 3) as usize);
                let w = self.foot.weights[i];
                let tri = self.mesh.triangles[self.foot.triangle[i] as usize];
                let m = outer_plus(self.normals[i], self.tangential);
                for s in 0..3 {
                    if !self.fixed[tri[s] as usize] && w[s] != 0.0 {
                        add_block(&mut v[at(tri[s])], &scaled(&m, w[r] * w[s]));
                    }
                }
            }
            let diagonal = scaled(&IDENTITY, self.smooth);
            let off = scaled(&IDENTITY, -self.smooth);
            for &e in self.edge_rows.row(g) {
                let (p, q) = self.edges[(e / 2) as usize];
                let other = if e % 2 == 0 { q } else { p };
                add_block(&mut v[0], &diagonal);
                if !self.fixed[other as usize] {
                    add_block(&mut v[at(other)], &off);
                }
            }
            if !self.membrane.is_empty() {
                for &e in self.edge_rows.row(g) {
                    let projector = &self.membrane[(e / 2) as usize];
                    let (p, q) = self.edges[(e / 2) as usize];
                    let other = if e % 2 == 0 { q } else { p };
                    add_block(&mut v[0], projector);
                    if !self.fixed[other as usize] {
                        add_block(&mut v[at(other)], &scaled(projector, -1.0));
                    }
                }
            }
            v
        });
        let scale: Vec<f64> = (0..count).map(|g| (vals[g][0][0] + vals[g][0][4] + vals[g][0][8]) / 3.0).collect();
        let free = (0..count).filter(|&g| !self.fixed[g]).count().max(1);
        let mean_diagonal = (0..count).filter(|&g| !self.fixed[g]).map(|g| scale[g]).sum::<f64>() / free as f64;
        let epsilon = 1e-6 * mean_diagonal;
        for g in 0..count {
            if self.fixed[g] {
                vals[g][0] = IDENTITY;
                continue;
            }
            add_block(&mut vals[g][0], &scaled(&IDENTITY, epsilon));
        }
        (Blocks { cols: self.pattern, vals }, epsilon)
    }

    fn rhs(&self, fixed_at: &[[f64; 3]], epsilon: f64) -> Vec<[f64; 3]> {
        crate::map_range(self.subset.len(), |g| {
            if self.fixed[g] {
                return fixed_at[g];
            }
            let mut b = [0.0f64; 3];
            for &e in self.rows.row(g) {
                let (i, r) = ((e / 3) as usize, (e % 3) as usize);
                let w = self.foot.weights[i];
                let tri = self.mesh.triangles[self.foot.triangle[i] as usize];
                let m = outer_plus(self.normals[i], self.tangential);
                let mut target = self.points[i];
                for s in 0..3 {
                    if self.fixed[tri[s] as usize] {
                        for k in 0..3 {
                            target[k] -= w[s] * fixed_at[tri[s] as usize][k];
                        }
                    }
                }
                let pulled = apply(&m, target);
                for k in 0..3 {
                    b[k] += w[r] * pulled[k];
                }
            }
            for &e in self.edge_rows.row(g) {
                let (p, q) = self.edges[(e / 2) as usize];
                let d = sub(self.subset[p as usize], self.subset[q as usize]);
                if e % 2 == 0 {
                    let fq = self.fixed[q as usize];
                    for k in 0..3 {
                        b[k] += self.smooth * (d[k] + if fq { fixed_at[q as usize][k] } else { 0.0 });
                    }
                } else {
                    let fp = self.fixed[p as usize];
                    for k in 0..3 {
                        b[k] += self.smooth * (-d[k] + if fp { fixed_at[p as usize][k] } else { 0.0 });
                    }
                }
            }
            if !self.membrane.is_empty() {
                for &e in self.edge_rows.row(g) {
                    let projector = &self.membrane[(e / 2) as usize];
                    let (p, q) = self.edges[(e / 2) as usize];
                    let far = if e % 2 == 0 { q } else { p };
                    if self.fixed[far as usize] {
                        let pulled = apply(projector, fixed_at[far as usize]);
                        for k in 0..3 {
                            b[k] += pulled[k];
                        }
                    }
                }
            }
            for k in 0..3 {
                b[k] += epsilon * self.subset[g][k];
            }
            b
        })
    }

    fn solve(&self, a: &Blocks, epsilon: f64, fixed_at: &[[f64; 3]]) -> (Vec<[f64; 3]>, usize) {
        let count = self.subset.len();
        let rhs = self.rhs(fixed_at, epsilon);
        let mut x: Vec<[f64; 3]> = (0..count).map(|g| if self.fixed[g] { fixed_at[g] } else { self.subset[g] }).collect();
        let iterations = conjugate_gradient(a, &rhs, &mut x);
        (x, iterations)
    }

    fn residual(&self, game: &[[f64; 3]], i: usize) -> Option<[f64; 3]> {
        let t = self.foot.triangle[i];
        if t == u32::MAX {
            return None;
        }
        let w = self.foot.weights[i];
        let tri = self.mesh.triangles[t as usize];
        let at = [0, 1, 2].map(|k| w[0] * game[tri[0] as usize][k] + w[1] * game[tri[1] as usize][k] + w[2] * game[tri[2] as usize][k]);
        Some(sub(at, self.points[i]))
    }

    fn seam_offsets(&self, game: &[[f64; 3]], seam: &[bool], kept_normals: &[[f64; 3]], seam_rows: &Csr) -> Vec<f64> {
        crate::map_range(game.len(), |g| {
            if !seam[g] {
                return 0.0;
            }
            let (mut num, mut den) = (0.0f64, 0.0f64);
            for &e in seam_rows.row(g) {
                let (i, s) = ((e / 3) as usize, (e % 3) as usize);
                let Some(r) = self.residual(game, i) else { continue };
                let w = self.foot.weights[i];
                num += w[s] * dot(r, kept_normals[g]);
                den += w[s] * w[s];
            }
            if den > 0.0 { -num / den } else { 0.0 }
        })
    }
}

fn smoothstep(x: f64, range: [f64; 2]) -> f64 {
    let t = ((x - range[0]) / (range[1] - range[0]).max(1e-12)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Default, Serialize)]
pub struct LiftStats {
    pub lifted: usize,
    pub largest_lift_mm: f64,
}

fn eye_need(eye: &Eye, p: [f64; 3], normal: [f64; 3], floor: f64, settings: &F5Settings) -> Option<([f64; 3], f64)> {
    let offset = sub(p, eye.centre);
    let d = length(offset);
    if d < 1e-12 {
        return None;
    }
    let dir = offset.map(|x| x / d);
    let surface = eye.surface(dir)?;
    let depth = surface - d;
    let taper = 1.0 - smoothstep(depth, settings.eye_depth);
    let front = 1.0 - smoothstep(eye.polar_deg(dir), [settings.eye_front_deg - 10.0, settings.eye_front_deg]);
    let facing = smoothstep(dot(normal, dir), [0.0, 0.2]);
    let need = (surface + floor.min(settings.eye_clearance) - d).max(0.0) * taper * front * facing;
    (need > 0.0).then_some((dir, need))
}

fn lift_vertices(game: &mut [[f64; 3]], subset: &[[f64; 3]], mesh: &Decimated, fixed: &[bool], eyes: &[Eye], settings: &F5Settings) -> LiftStats {
    let mut normals = vec![[0.0f64; 3]; game.len()];
    for tri in &mesh.triangles {
        let n = pl::triangle_normal(tri.map(|i| game[i as usize]));
        for &g in tri {
            for k in 0..3 {
                normals[g as usize][k] += n[k];
            }
        }
    }
    let mut stats = LiftStats::default();
    let lifts: Vec<[f64; 3]> = (0..game.len())
        .map(|g| {
            let mut lift = [0.0; 3];
            if fixed[g] {
                return lift;
            }
            let l = length(normals[g]).max(1e-30);
            let normal = normals[g].map(|x| x / l);
            for eye in eyes {
                let s = subset[g];
                let floor = match eye.surface(sub(s, eye.centre).map(|x| x / length(sub(s, eye.centre)).max(1e-30))) {
                    Some(surface) => length(sub(s, eye.centre)) - surface,
                    None => settings.eye_clearance,
                };
                if let Some((dir, need)) = eye_need(eye, game[g], normal, floor, settings) {
                    for k in 0..3 {
                        lift[k] += 1.05 * need * dir[k];
                    }
                }
            }
            lift
        })
        .collect();
    for (g, lift) in lifts.iter().enumerate() {
        let l = length(*lift);
        if l > 0.0 {
            stats.lifted += 1;
            stats.largest_lift_mm = stats.largest_lift_mm.max(l * 1000.0);
            for k in 0..3 {
                game[g][k] += lift[k];
            }
        }
    }
    stats
}

fn uv_sign(t: [Uv; 3]) -> f64 {
    ((t[1][0] as f64 - t[0][0] as f64) * (t[2][1] as f64 - t[0][1] as f64) - (t[2][0] as f64 - t[0][0] as f64) * (t[1][1] as f64 - t[0][1] as f64)).signum()
}

pub fn place(
    positions: &[[f64; 3]],
    dense_triangles: &[[u32; 3]],
    dense_uvs: &[[Uv; 3]],
    mesh: &Decimated,
    eyes: &[Eye],
    settings: &F5Settings,
) -> (Vec<[f64; 3]>, Vec<[Uv; 3]>, F5Stats) {
    let prepared = Prepared::new(positions.len(), dense_triangles, dense_uvs, mesh, settings.uv_ring);
    place_prepared(positions, dense_triangles, dense_uvs, mesh, eyes, settings, &prepared)
}

pub fn place_prepared(
    positions: &[[f64; 3]],
    dense_triangles: &[[u32; 3]],
    dense_uvs: &[[Uv; 3]],
    mesh: &Decimated,
    eyes: &[Eye],
    settings: &F5Settings,
    prepared: &Prepared,
) -> (Vec<[f64; 3]>, Vec<[Uv; 3]>, F5Stats) {
    let count = mesh.kept.len();
    let (boundary, seam, fixed) = (&prepared.boundary, &prepared.seam, &prepared.fixed);
    let mut stats = F5Stats {
        vertices: count,
        boundary: boundary.iter().filter(|&&b| b).count(),
        seam: seam.iter().filter(|&&s| s).count(),
        ..Default::default()
    };
    let points = positions;
    let dense_normals = pl::angle_weighted_normals_with(points, dense_triangles, &prepared.corners);
    let parts = prepared.f5_parts(dense_triangles, dense_uvs, mesh);
    let foot = &parts.foot;
    stats.footprint_points = foot.triangle.iter().filter(|&&t| t != u32::MAX).count();
    stats.footprint_outside = foot.outside;
    let subset: Vec<[f64; 3]> = mesh.kept.iter().map(|&d| points[d as usize]).collect();
    let edges = &prepared.edges;
    let mut vertex_normals = vec![[0.0f64; 3]; count];
    for tri in &mesh.triangles {
        let n = pl::triangle_normal(tri.map(|i| subset[i as usize]));
        for &g in tri {
            for k in 0..3 {
                vertex_normals[g as usize][k] += n[k];
            }
        }
    }
    let kept_normals: Vec<[f64; 3]> = if settings.subset_normals {
        vertex_normals.iter().map(|n| n.map(|x| x / length(*n).max(1e-30))).collect()
    } else {
        mesh.kept.iter().map(|&d| dense_normals[d as usize]).collect()
    };
    let data_normals: Vec<[f64; 3]> = if settings.subset_normals {
        crate::map_range(points.len(), |i| {
            let t = foot.triangle[i];
            if t == u32::MAX {
                return [0.0; 3];
            }
            let w = foot.weights[i];
            let tri = mesh.triangles[t as usize];
            let n = [0, 1, 2].map(|k| w[0] * vertex_normals[tri[0] as usize][k] + w[1] * vertex_normals[tri[1] as usize][k] + w[2] * vertex_normals[tri[2] as usize][k]);
            let l = length(n).max(1e-30);
            n.map(|x| x / l)
        })
    } else {
        dense_normals.clone()
    };
    let system = System {
        points,
        normals: &data_normals,
        foot,
        mesh,
        fixed,
        subset: &subset,
        edges,
        rows: &parts.rows,
        edge_rows: &prepared.edge_rows,
        pattern: &parts.pattern,
        tangential: settings.tangential,
        smooth: settings.smooth,
        membrane: if settings.membrane > 0.0 {
            edges
                .iter()
                .map(|&(p, q)| {
                    let n = [0, 1, 2].map(|k| kept_normals[p as usize][k] + kept_normals[q as usize][k]);
                    let n = n.map(|x| x / length(n).max(1e-30));
                    scaled(&tangential_projector(n), settings.membrane)
                })
                .collect()
        } else {
            Vec::new()
        },
    };
    let (a, epsilon) = system.matrix();
    let mut fixed_at = subset.clone();
    let (first, iterations) = system.solve(&a, epsilon, &fixed_at);
    stats.solver_iterations.push(iterations);
    let h = system.seam_offsets(&first, seam, &kept_normals, &parts.seam_rows);
    for g in 0..count {
        if seam[g] {
            fixed_at[g] = [0, 1, 2].map(|k| subset[g][k] + h[g] * kept_normals[g][k]);
            stats.seam_offset_max_mm = stats.seam_offset_max_mm.max(h[g].abs() * 1000.0);
        }
    }
    let (mut game, iterations) = system.solve(&a, epsilon, &fixed_at);
    stats.solver_iterations.push(iterations);
    let gaps: Vec<Option<(f64, f64)>> = crate::map_range(points.len(), |i| system.residual(&game, i).map(|r| (length(r), dot(r, dense_normals[i]).abs())));
    let (mut gap, mut normal_gap, mut n) = (0.0, 0.0, 0usize);
    for (l, d) in gaps.iter().flatten() {
        gap += l;
        normal_gap += d;
        n += 1;
    }
    stats.point_gap_mm = gap / n.max(1) as f64 * 1000.0;
    stats.normal_gap_mm = normal_gap / n.max(1) as f64 * 1000.0;
    drop(dense_normals);
    stats.clamped = pl::clamp_moves(&mut game, &subset, settings.max_move);
    if !eyes.is_empty() {
        stats.clear = Some(lift_vertices(&mut game, &subset, mesh, fixed, eyes, settings));
    }
    let mut moves: Vec<f64> = game.iter().zip(&subset).map(|(a, b)| length(sub(*a, *b))).collect();
    moves.sort_by(f64::total_cmp);
    stats.median_move_mm = moves[moves.len() / 2] * 1000.0;
    stats.max_move_mm = moves[moves.len() - 1] * 1000.0;

    let surface = Surface::with_topology(points, dense_triangles, dense_uvs, mesh, &prepared.surface);
    let spheres = uv_chart::triangle_spheres(points, dense_triangles);
    let derived: Vec<Option<Uv>> = crate::map_range(count, |g| {
        if fixed[g] {
            return None;
        }
        surface.closest_in(prepared.patches.row(g), game[g], &spheres).map(|hit| surface.uv(hit))
    });
    stats.uv_keys_derived = derived.iter().filter(|d| d.is_some()).count();
    stats.uv_seed_misses = (0..count).filter(|&g| !fixed[g] && derived[g].is_none()).count();
    let mut use_derived: Vec<bool> = derived.iter().map(|d| d.is_some()).collect();
    let build = |use_derived: &[bool]| -> Vec<[Uv; 3]> {
        mesh.triangles
            .iter()
            .zip(&mesh.triangle_uvs)
            .map(|(tri, uvs)| [0, 1, 2].map(|c| if use_derived[tri[c] as usize] { derived[tri[c] as usize].unwrap() } else { uvs[c] }))
            .collect()
    };
    let mut uvs = build(&use_derived);
    let flipped = |uvs: &[[Uv; 3]]| -> Vec<usize> {
        (0..uvs.len()).filter(|&t| uv_sign(mesh.triangle_uvs[t]) != 0.0 && uv_sign(uvs[t]) != uv_sign(mesh.triangle_uvs[t])).collect()
    };
    let mut bad = flipped(&uvs);
    stats.uv_flipped_before = bad.len();
    for _ in 0..64 {
        if bad.is_empty() {
            break;
        }
        for &t in &bad {
            for &g in &mesh.triangles[t] {
                if use_derived[g as usize] {
                    use_derived[g as usize] = false;
                    stats.uv_reverted_vertices += 1;
                }
            }
        }
        uvs = build(&use_derived);
        bad = flipped(&uvs);
    }
    stats.uv_flipped_after = bad.len();
    (game, uvs, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Vec<[f64; 3]>, Vec<[u32; 3]>, Vec<[Uv; 3]>, Decimated) {
        let (fine, n, step) = (8usize, 7usize, 0.004);
        let size = (n - 1) * fine + 1;
        let spacing = step / fine as f64;
        let dense: Vec<[f64; 3]> = (0..size * size)
            .map(|i| {
                let (x, y) = ((i % size) as f64 * spacing, (i / size) as f64 * spacing);
                [x, y, 0.002 * (x * 260.0).sin() * (y * 190.0).cos()]
            })
            .collect();
        let grid = |m: usize| {
            let mut tris = Vec::new();
            for y in 0..m - 1 {
                for x in 0..m - 1 {
                    let a = (y * m + x) as u32;
                    tris.push([a, a + 1, a + m as u32 + 1]);
                    tris.push([a, a + m as u32 + 1, a + m as u32]);
                }
            }
            tris
        };
        let dense_tris = grid(size);
        let uv_of = |p: [f64; 3]| [p[0] as f32 * 50.0, p[1] as f32 * 50.0];
        let dense_uvs: Vec<[Uv; 3]> = dense_tris.iter().map(|t| t.map(|i| uv_of(dense[i as usize]))).collect();
        let mut kept: Vec<u32> = (0..n * n).map(|i| ((i / n) * fine * size + (i % n) * fine) as u32).collect();
        kept[2 * n + 2] = (2 * fine * size + 2 * fine + 3) as u32;
        let triangles = grid(n);
        let triangle_uvs: Vec<[Uv; 3]> = triangles.iter().map(|t| t.map(|g| uv_of(dense[kept[g as usize] as usize]))).collect();
        let owner: Vec<u32> = dense
            .iter()
            .map(|p| {
                (0..kept.len())
                    .min_by(|&a, &b| length(sub(*p, dense[kept[a] as usize])).total_cmp(&length(sub(*p, dense[kept[b] as usize]))))
                    .unwrap() as u32
            })
            .collect();
        (dense, dense_tris, dense_uvs, Decimated { kept, owner, triangles, triangle_uvs, stats: Default::default() })
    }

    fn settings() -> F5Settings {
        F5Settings {
            tangential: 0.01,
            smooth: 0.1,
            membrane: 0.1,
            max_move: 0.003,
            uv_ring: 2,
            subset_normals: true,
            eye_clearance: 0.00005,
            eye_depth: [0.0003, 0.0006],
            eye_front_deg: 75.0,
        }
    }

    fn noise(count: usize, amplitude: f64) -> Vec<[f64; 3]> {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        (0..count)
            .map(|_| {
                let d = [0, 1, 2].map(|_| {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    (state >> 11) as f64 / (1u64 << 53) as f64 - 0.5
                });
                let l = length(d).max(1e-12);
                d.map(|x| x * amplitude / l)
            })
            .collect()
    }

    #[test]
    fn micrometre_noise_moves_the_game_mesh_continuously_and_linearly() {
        let (dense, tris, uvs, mesh) = fixture();
        let place_at = |amplitude: f64| -> Vec<[f64; 3]> {
            let moved: Vec<[f64; 3]> = dense.iter().zip(noise(dense.len(), amplitude)).map(|(p, d)| [0, 1, 2].map(|k| p[k] + d[k])).collect();
            place(&moved, &tris, &uvs, &mesh, &[], &settings()).0
        };
        let base = place_at(0.0);
        let moves = |game: &[[f64; 3]]| -> Vec<f64> { game.iter().zip(&base).map(|(a, b)| length(sub(*a, *b))).collect() };
        let one = moves(&place_at(1e-6));
        let two = moves(&place_at(2e-6));
        let worst = (0..one.len()).max_by(|&a, &b| one[a].total_cmp(&one[b])).unwrap();
        assert!(one[worst] <= 10.0 * 1e-6, "1 um of dense noise moved game vertex {worst} by {} um", one[worst] * 1e6);
        assert!(one[worst] > 0.0);
        let scaling = two[worst] / one[worst];
        assert!((1.8..=2.2).contains(&scaling), "doubling the noise scaled the response by {scaling}");
    }

    #[test]
    fn the_footprint_reads_no_positions() {
        let (_, tris, uvs, mesh) = fixture();
        let vtris = pl::vertex_triangles(&mesh, mesh.kept.len());
        let rings = pl::ring_triangles(&mesh, &vtris);
        let foot = footprint(mesh.owner.len(), &tris, &uvs, &mesh, &rings);
        assert_eq!(foot.triangle.iter().filter(|&&t| t == u32::MAX).count(), 0);
        for w in &foot.weights {
            assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-9 && w.iter().all(|&x| x >= 0.0));
        }
    }
}
