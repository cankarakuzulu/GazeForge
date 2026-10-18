use std::collections::{BTreeSet, HashMap};

use gf_geom::Uv;
use gf_geom::decimate::Decimated;
use gf_geom::placement::{angle_weighted_normals, deviation_deg, triangle_references};
use serde::Serialize;

pub type Point = [f64; 3];
pub type Uv64 = [f64; 2];

const MM: f64 = 0.001;

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Point, b: Point) -> Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: Point, floor: f64) -> Point {
    let l = norm(a).max(floor);
    a.map(|x| x / l)
}

fn round_to(x: f64, digits: i32) -> f64 {
    let s = 10f64.powi(digits);
    (x * s).round() / s
}

pub fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = q / 100.0 * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(sorted.len() - 1);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

pub fn from_tip_mm(p: Point, tip: Point) -> Point {
    sub(p, tip).map(|x| x * 1000.0)
}

pub fn in_box(rel: Point) -> bool {
    rel[0].abs() < 45.0 && rel[2] > -10.0 && rel[2] < 50.0 && rel[1] < 45.0
}

pub fn in_front(rel: Point) -> bool {
    !in_box(rel) && rel[1] < 60.0 && rel[0].abs() < 75.0 && rel[2] > -80.0 && rel[2] < 80.0
}

pub fn check_frame(positions: &[Point], tip: Point) -> Result<(), String> {
    if positions.is_empty() {
        return Err("no positions".into());
    }
    let n = positions.len() as f64;
    let c = positions.iter().fold([0.0; 3], |a, &p| add(a, p)).map(|x| x / n);
    let ahead = c[1] - tip[1];
    if !(ahead > 0.05 && (tip[0] - c[0]).abs() < 0.01) {
        return Err(format!(
            "the gates assume the world frame (-y forward, z up, x across): the nose tip should sit > 50 mm in front (-y) of the centroid \
             and on its midline, but it is {:.1} mm ahead and {:.1} mm to the side",
            ahead * 1000.0,
            (tip[0] - c[0]) * 1000.0
        ));
    }
    Ok(())
}

fn centroid(p: [Point; 3]) -> Point {
    add(add(p[0], p[1]), p[2]).map(|x| x / 3.0)
}

pub fn corner_angles_deg(p: [Point; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| {
        let e1 = sub(p[(i + 1) % 3], p[i]);
        let e2 = sub(p[(i + 2) % 3], p[i]);
        let cos = dot(e1, e2) / (norm(e1) * norm(e2)).max(1e-30);
        cos.clamp(-1.0, 1.0).acos().to_degrees()
    })
}

#[derive(Serialize, Clone, Debug)]
pub struct Sliver {
    pub tri: usize,
    pub min_deg: f64,
    pub max_deg: f64,
    pub from_tip_mm: Point,
}

#[derive(Serialize, Clone, Debug)]
pub struct TriangleQuality {
    pub under5: usize,
    pub under2: usize,
    pub under5_outside_box: usize,
    pub caps170: usize,
    pub caps170_outside_box: usize,
    pub outside_box_examples: Vec<Sliver>,
    pub pass: bool,
}

pub fn triangle_quality(game: &[Point], triangles: &[[u32; 3]], tip: Point) -> TriangleQuality {
    let mut q = TriangleQuality {
        under5: 0,
        under2: 0,
        under5_outside_box: 0,
        caps170: 0,
        caps170_outside_box: 0,
        outside_box_examples: Vec::new(),
        pass: false,
    };
    for (t, tri) in triangles.iter().enumerate() {
        let p = tri.map(|i| game[i as usize]);
        let a = corner_angles_deg(p);
        let (lo, hi) = (a[0].min(a[1]).min(a[2]), a[0].max(a[1]).max(a[2]));
        let rel = from_tip_mm(centroid(p), tip);
        let outside = !in_box(rel);
        let (u5, cap) = (lo < 5.0, hi > 170.0);
        q.under5 += u5 as usize;
        q.under2 += (lo < 2.0) as usize;
        q.caps170 += cap as usize;
        q.under5_outside_box += (u5 && outside) as usize;
        q.caps170_outside_box += (cap && outside) as usize;
        if (u5 || cap) && outside && q.outside_box_examples.len() < 6 {
            q.outside_box_examples.push(Sliver {
                tri: t,
                min_deg: round_to(lo, 2),
                max_deg: round_to(hi, 2),
                from_tip_mm: rel.map(|x| round_to(x, 1)),
            });
        }
    }
    q.pass = q.under5_outside_box == 0 && q.caps170_outside_box == 0;
    q
}

#[derive(Serialize, Clone, Debug)]
pub struct UvHealth {
    pub triangles: usize,
    pub flipped: usize,
    pub zero_area: usize,
    pub log_stretch_p50_p95_p99: [f64; 3],
    pub pass: bool,
}

pub fn uv_health(game: &[Point], triangles: &[[u32; 3]], uvs: &[[Uv64; 3]]) -> UvHealth {
    let mut ratios = Vec::with_capacity(triangles.len());
    let mut signs = Vec::with_capacity(triangles.len());
    for (tri, t) in triangles.iter().zip(uvs) {
        let p = tri.map(|i| game[i as usize]);
        let area3 = norm(cross(sub(p[1], p[0]), sub(p[2], p[0]))) / 2.0;
        let area2 = ((t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])) / 2.0;
        if area3 > 0.0 {
            ratios.push(area2.abs() / area3);
            signs.push(if area2 > 0.0 {
                1.0
            } else if area2 < 0.0 {
                -1.0
            } else {
                0.0
            });
        }
    }
    let mut sorted_signs = signs.clone();
    sorted_signs.sort_by(f64::total_cmp);
    let median_sign = percentile(&sorted_signs, 50.0);
    let major = if median_sign > 0.0 {
        1.0
    } else if median_sign < 0.0 {
        -1.0
    } else {
        0.0
    };
    let flipped = signs.iter().filter(|&&s| s != major).count();
    let zero_area = ratios.iter().filter(|&&r| r == 0.0).count();
    let mut sorted = ratios.clone();
    sorted.sort_by(f64::total_cmp);
    let median = percentile(&sorted, 50.0);
    let mut logs: Vec<f64> = ratios.iter().filter(|&&r| r > 0.0).map(|r| (r / median).ln().abs()).collect();
    logs.sort_by(f64::total_cmp);
    let spread = [50.0, 95.0, 99.0].map(|q| round_to(percentile(&logs, q), 2));
    UvHealth { triangles: ratios.len(), flipped, zero_area, log_stretch_p50_p95_p99: spread, pass: flipped == 0 && zero_area == 0 }
}

#[derive(Serialize, Clone, Debug)]
pub struct Census {
    pub triangles: usize,
    pub over60: usize,
    pub flipped_over90: usize,
    pub box_over60: usize,
    pub front_over30: usize,
    pub front_over60: usize,
    pub front_over90: usize,
    pub over60_invisible_to_min5_cap170: usize,
    pub worst_visible: Vec<(usize, f64)>,
    pub pass: bool,
}

pub fn census(dense_face: &[Point], dense_triangles: &[[u32; 3]], mesh: &Decimated, game: &[Point], tip: Point) -> Census {
    let normals = angle_weighted_normals(dense_face, dense_triangles);
    let kept: Vec<Point> = mesh.kept.iter().map(|&d| normals[d as usize]).collect();
    let refs = triangle_references(mesh, &kept);
    let mut c = Census {
        triangles: mesh.triangles.len(),
        over60: 0,
        flipped_over90: 0,
        box_over60: 0,
        front_over30: 0,
        front_over60: 0,
        front_over90: 0,
        over60_invisible_to_min5_cap170: 0,
        worst_visible: Vec::new(),
        pass: false,
    };
    let mut front = Vec::new();
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let p = tri.map(|i| game[i as usize]);
        let dev = deviation_deg(p, refs[t]);
        let rel = from_tip_mm(centroid(p), tip);
        let a = corner_angles_deg(p);
        let hidden = a[0].min(a[1]).min(a[2]) >= 5.0 && a[0].max(a[1]).max(a[2]) <= 170.0;
        c.over60 += (dev > 60.0) as usize;
        c.flipped_over90 += (dev > 90.0) as usize;
        c.box_over60 += (in_box(rel) && dev > 60.0) as usize;
        c.over60_invisible_to_min5_cap170 += (hidden && dev > 60.0) as usize;
        if in_front(rel) {
            c.front_over30 += (dev > 30.0) as usize;
            c.front_over60 += (dev > 60.0) as usize;
            c.front_over90 += (dev > 90.0) as usize;
            front.push((t, round_to(dev, 1)));
        }
    }
    front.sort_by(|a, b| b.1.total_cmp(&a.1));
    c.worst_visible = front.into_iter().take(3).collect();
    c.pass = c.front_over60 == 0;
    c
}

pub const SEAM_CELL: f64 = 0.004;
pub const SEAM_UV_TOL: f64 = 1e-5;
pub const SEAM_KEY_TOL: f64 = 1e-4;

#[derive(Serialize, Clone, Debug)]
pub struct SeamVertex {
    pub vertex: usize,
    pub gap_mm: f64,
    pub keys: usize,
    pub from_tip_mm: Point,
}

#[derive(Serialize, Clone, Debug)]
pub struct Seams {
    pub seam_vertices: usize,
    pub max_mm: f64,
    pub over_0p01: usize,
    pub over_0p05: usize,
    pub over_0p1: usize,
    pub keys_off_chart: usize,
    pub worst_uv_miss: f64,
    pub seam_keys_lost: usize,
    pub seam_keys_added: usize,
    pub lost_vertices: Vec<usize>,
    pub added_vertices: Vec<usize>,
    pub worst: Vec<SeamVertex>,
    pub limit_mm: f64,
    pub pass: bool,
}

pub struct DenseSurface<'a> {
    pub positions: &'a [Point],
    pub triangles: &'a [[u32; 3]],
    pub triangle_uvs: &'a [[Uv; 3]],
}

fn cell_of(p: Point) -> [i64; 3] {
    p.map(|x| (x / SEAM_CELL).floor() as i64)
}

fn key_clusters(points: &mut [Uv64]) -> usize {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    let mut out: Vec<Uv64> = Vec::new();
    for p in points.iter() {
        if !out.iter().any(|q| (p[0] - q[0]).abs().max((p[1] - q[1]).abs()) <= SEAM_KEY_TOL) {
            out.push(*p);
        }
    }
    out.len()
}

fn surface_point(q: Uv64, cand: &[u32], dense: &DenseSurface, at: Point) -> Option<(Point, f64)> {
    let mut rows: Vec<(f64, [f64; 3], u32)> = Vec::with_capacity(cand.len());
    for &t in cand {
        let uv = dense.triangle_uvs[t as usize].map(|c| [c[0] as f64, c[1] as f64]);
        let (a, b, c) = (uv[0], uv[1], uv[2]);
        let v0 = [b[0] - a[0], b[1] - a[1]];
        let v1 = [c[0] - a[0], c[1] - a[1]];
        let v2 = [q[0] - a[0], q[1] - a[1]];
        let den = v0[0] * v1[1] - v1[0] * v0[1];
        let ok = den.abs() > 1e-20;
        let (l1, l2) = if ok {
            ((v2[0] * v1[1] - v1[0] * v2[1]) / den, (v0[0] * v2[1] - v2[0] * v0[1]) / den)
        } else {
            (-1.0, -1.0)
        };
        let mut w = [1.0 - l1 - l2, l1, l2].map(|x| x.max(0.0));
        let s = (w[0] + w[1] + w[2]).max(1e-30);
        w = w.map(|x| x / s);
        let back = [w[0] * a[0] + w[1] * b[0] + w[2] * c[0], w[0] * a[1] + w[1] * b[1] + w[2] * c[1]];
        let miss = if ok { ((back[0] - q[0]).powi(2) + (back[1] - q[1]).powi(2)).sqrt() } else { f64::INFINITY };
        rows.push((miss, w, t));
    }
    let best = rows.iter().map(|r| r.0).fold(f64::INFINITY, f64::min);
    if !best.is_finite() {
        return None;
    }
    let mut chosen: Option<(f64, Point)> = None;
    for (miss, w, t) in &rows {
        if *miss <= best + 1e-9 {
            let tri = dense.triangles[*t as usize].map(|i| dense.positions[i as usize]);
            let p = [0, 1, 2].map(|k| w[0] * tri[0][k] + w[1] * tri[1][k] + w[2] * tri[2][k]);
            let d = norm(sub(p, at));
            if chosen.as_ref().is_none_or(|c| d < c.0) {
                chosen = Some((d, p));
            }
        }
    }
    chosen.map(|(_, p)| (p, best))
}

pub fn seams(
    game: &[Point],
    triangles: &[[u32; 3]],
    game_uvs: &[[Uv64; 3]],
    base_uvs: &[[Uv; 3]],
    dense: &DenseSurface,
    tip: Point,
    limit_mm: f64,
) -> Seams {
    let mut base_keys: HashMap<usize, Vec<Uv64>> = HashMap::new();
    let mut game_keys: HashMap<usize, Vec<Uv64>> = HashMap::new();
    let mut rounded: HashMap<usize, BTreeSet<(i64, i64)>> = HashMap::new();
    for ((tri, g), b) in triangles.iter().zip(game_uvs).zip(base_uvs) {
        for c in 0..3 {
            let v = tri[c] as usize;
            base_keys.entry(v).or_default().push([b[c][0] as f64, b[c][1] as f64]);
            game_keys.entry(v).or_default().push(g[c]);
            rounded.entry(v).or_default().insert(((g[c][0] * 1e7).round() as i64, (g[c][1] * 1e7).round() as i64));
        }
    }
    let mut lost = Vec::new();
    let mut added = Vec::new();
    let mut vertices: Vec<usize> = base_keys.keys().copied().collect();
    vertices.sort_unstable();
    for v in &vertices {
        let nb = key_clusters(base_keys.get_mut(v).unwrap());
        let ng = key_clusters(game_keys.get_mut(v).unwrap());
        if ng < nb {
            lost.push(*v);
        } else if ng > nb {
            added.push(*v);
        }
    }
    let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
    for (t, tri) in dense.triangles.iter().enumerate() {
        let c = centroid(tri.map(|i| dense.positions[i as usize]));
        grid.entry(cell_of(c)).or_default().push(t as u32);
    }
    let mut rows: Vec<SeamVertex> = Vec::new();
    let mut off_chart = 0usize;
    let mut worst_miss = 0.0f64;
    for v in &vertices {
        let keys = &rounded[v];
        if keys.len() < 2 {
            continue;
        }
        let at = game[*v];
        let c = cell_of(at);
        let mut cand: Vec<u32> = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = grid.get(&[c[0] + dx, c[1] + dy, c[2] + dz]) {
                        cand.extend_from_slice(list);
                    }
                }
            }
        }
        let mut points = Vec::new();
        for &(u, w) in keys {
            let q = [u as f64 / 1e7, w as f64 / 1e7];
            match surface_point(q, &cand, dense, at) {
                Some((p, miss)) => {
                    worst_miss = worst_miss.max(miss);
                    off_chart += (miss > SEAM_UV_TOL) as usize;
                    points.push(p);
                }
                None => off_chart += 1,
            }
        }
        let mut gap = 0.0f64;
        for i in 0..points.len() {
            for j in i + 1..points.len() {
                gap = gap.max(norm(sub(points[i], points[j])));
            }
        }
        rows.push(SeamVertex { vertex: *v, gap_mm: gap * 1000.0, keys: keys.len(), from_tip_mm: from_tip_mm(at, tip).map(|x| round_to(x, 1)) });
    }
    rows.sort_by(|a, b| b.gap_mm.total_cmp(&a.gap_mm).then(b.vertex.cmp(&a.vertex)));
    let count = |limit: f64| rows.iter().filter(|r| r.gap_mm > limit).count();
    let max_mm = rows.first().map_or(0.0, |r| r.gap_mm);
    let pass = max_mm <= limit_mm && lost.is_empty() && added.is_empty();
    Seams {
        seam_vertices: rows.len(),
        max_mm,
        over_0p01: count(0.01),
        over_0p05: count(0.05),
        over_0p1: count(0.1),
        keys_off_chart: off_chart,
        worst_uv_miss: worst_miss,
        seam_keys_lost: lost.len(),
        seam_keys_added: added.len(),
        lost_vertices: lost.into_iter().take(8).collect(),
        added_vertices: added.into_iter().take(8).collect(),
        worst: rows.into_iter().take(6).collect(),
        limit_mm,
        pass,
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct PathPops {
    pub steps: usize,
    pub pops: usize,
    pub worst_excess_mm: f64,
    pub per_step: Vec<usize>,
    pub vertices: Vec<usize>,
    pub limit: usize,
    pub pass_zero: bool,
    pub pass: bool,
}

pub fn path_pops(placed: &[Vec<Point>], subset: &[Vec<Point>], limit: usize) -> Result<PathPops, String> {
    if placed.len() != subset.len() || placed.len() < 2 {
        return Err(format!("a path needs at least 2 matching steps, got {} placed and {} subset", placed.len(), subset.len()));
    }
    let mut pops = 0;
    let mut worst = 0.0f64;
    let mut per_step = Vec::new();
    let mut popped = BTreeSet::new();
    for k in 0..placed.len() - 1 {
        let mut here = 0;
        for v in 0..placed[k].len() {
            let dp = norm(sub(placed[k + 1][v], placed[k][v])) * 1000.0;
            let ds = norm(sub(subset[k + 1][v], subset[k][v])) * 1000.0;
            if dp > 0.5 && dp > 3.0 * ds {
                here += 1;
                popped.insert(v);
            }
            worst = worst.max(dp - ds);
        }
        pops += here;
        per_step.push(here);
    }
    Ok(PathPops {
        steps: placed.len() - 1,
        pops,
        worst_excess_mm: worst,
        per_step,
        vertices: popped.into_iter().take(40).collect(),
        limit,
        pass_zero: pops == 0,
        pass: pops <= limit,
    })
}

pub fn quad_normals(co: &[Point], quads: &[[u32; 4]]) -> Vec<Point> {
    quads
        .iter()
        .map(|q| {
            let [a, b, c, d] = q.map(|i| co[i as usize]);
            unit(add(cross(sub(b, a), sub(c, a)), cross(sub(c, a), sub(d, a))), 1e-15)
        })
        .collect()
}

pub fn face_pairs(quads: &[[u32; 4]]) -> Vec<[u32; 2]> {
    let mut keys: Vec<(u64, u32)> = Vec::with_capacity(quads.len() * 4);
    for (f, q) in quads.iter().enumerate() {
        for i in 0..4 {
            let (a, b) = (q[i] as u64, q[(i + 1) % 4] as u64);
            keys.push(((a.min(b) << 32) | a.max(b), f as u32));
        }
    }
    keys.sort_by_key(|k| k.0);
    keys.windows(2).filter(|w| w[0].0 == w[1].0).map(|w| [w[0].1, w[1].1]).collect()
}

pub fn unique_edges(quads: &[[u32; 4]]) -> Vec<[u32; 2]> {
    let mut edges: Vec<[u32; 2]> = Vec::with_capacity(quads.len() * 4);
    for q in quads {
        for i in 0..4 {
            let (a, b) = (q[i], q[(i + 1) % 4]);
            edges.push([a.min(b), a.max(b)]);
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

pub fn folds(co: &[Point], quads: &[[u32; 4]], pairs: &[[u32; 2]]) -> Vec<bool> {
    let n = quad_normals(co, quads);
    pairs.iter().map(|p| dot(n[p[0] as usize], n[p[1] as usize]) < -0.5).collect()
}

pub fn move_roughness(mv: &[Point], edges: &[[u32; 2]]) -> Vec<f64> {
    let mut total = vec![[0.0f64; 3]; mv.len()];
    let mut degree = vec![0usize; mv.len()];
    for e in edges {
        let (a, b) = (e[0] as usize, e[1] as usize);
        total[a] = add(total[a], mv[b]);
        total[b] = add(total[b], mv[a]);
        degree[a] += 1;
        degree[b] += 1;
    }
    mv.iter()
        .zip(total.iter().zip(&degree))
        .map(|(m, (t, &d))| {
            let dd = d.max(1) as f64;
            norm(sub(*m, t.map(|x| x / dd)))
        })
        .collect()
}

pub fn vertex_normals(co: &[Point], quads: &[[u32; 4]]) -> Vec<Point> {
    let mut out = vec![[0.0f64; 3]; co.len()];
    for q in quads {
        let p = q.map(|i| co[i as usize]);
        let n = unit(cross(sub(p[0], p[2]), sub(p[1], p[3])), 1e-30);
        for c in 0..4 {
            let prev = unit(sub(p[(c + 3) % 4], p[c]), 1e-30);
            let next = unit(sub(p[(c + 1) % 4], p[c]), 1e-30);
            let angle = dot(prev, next).clamp(-1.0, 1.0).acos();
            let v = q[c] as usize;
            out[v] = add(out[v], n.map(|x| x * angle));
        }
    }
    out.into_iter().map(|n| unit(n, 1e-12)).collect()
}

pub fn nose_tip(co: &[Point]) -> Point {
    let mut z: Vec<f64> = co.iter().map(|p| p[2]).collect();
    z.sort_by(f64::total_cmp);
    let (hi, lo) = (percentile(&z, 95.0), percentile(&z, 30.0));
    let mut best: Option<Point> = None;
    for p in co {
        if p[0].abs() < 0.004 && p[2] < hi && p[2] > lo && best.is_none_or(|b| p[1] < b[1]) {
            best = Some(*p);
        }
    }
    best.unwrap_or([0.0; 3])
}

pub fn inner_showing(co: &[Point], normals: &[Point], tip: Point) -> usize {
    const CELL: f64 = 0.5 * MM;
    let idx: Vec<usize> = (0..co.len())
        .filter(|&i| {
            let below = tip[2] - co[i][2];
            below > -0.075 && below < 0.075 && co[i][0].abs() < 0.07 && co[i][1] < tip[1] + 0.06
        })
        .collect();
    if idx.is_empty() {
        return 0;
    }
    let fx: Vec<i64> = idx.iter().map(|&i| (co[i][0] / CELL).round_ties_even() as i64).collect();
    let fz: Vec<i64> = idx.iter().map(|&i| (co[i][2] / CELL).round_ties_even() as i64).collect();
    let (fx0, fz0) = (*fx.iter().min().unwrap(), *fz.iter().min().unwrap());
    let span = fz.iter().max().unwrap() - fz0 + 1;
    let mut rows: Vec<(i64, f64, usize)> =
        idx.iter().enumerate().map(|(k, &i)| ((fx[k] - fx0) * span + (fz[k] - fz0), co[i][1], i)).collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut count = 0;
    let mut last = None;
    for (cell, _, i) in rows {
        if last != Some(cell) {
            last = Some(cell);
            count += (normals[i][1] > 0.3) as usize;
        }
    }
    count
}

#[derive(Serialize, Clone, Debug)]
pub struct ShellLimits {
    pub fold_tolerance: usize,
    pub inner_tolerance: usize,
    pub rough_mm: f64,
    pub rough_relative: f64,
    pub rough_tolerance: usize,
}

impl Default for ShellLimits {
    fn default() -> Self {
        ShellLimits { fold_tolerance: 10, inner_tolerance: 15, rough_mm: 0.02, rough_relative: 0.05, rough_tolerance: 200 }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Shell {
    pub folds_before: usize,
    pub folds_after: usize,
    pub new_folds: usize,
    pub inner_showing_before: usize,
    pub inner_showing_after: usize,
    pub faces_flipped: usize,
    pub rough_over: usize,
    pub rough_over_absolute_only: usize,
    pub rough_max_mm: f64,
    pub rough_p999_mm: f64,
    pub surface_roughness_before_mm: f64,
    pub surface_roughness_after_mm: f64,
    pub smoothing: bool,
    pub max_move_mm: f64,
    pub vertices_moved: usize,
    pub tip: Point,
    pub limits: ShellLimits,
    pub failures: Vec<String>,
    pub pass: bool,
}

pub fn shell(before: &[Point], after: &[Point], quads: &[[u32; 4]], limits: &ShellLimits) -> Result<Shell, String> {
    if before.len() != after.len() {
        return Err(format!("before has {} vertices, after {}", before.len(), after.len()));
    }
    let pairs = face_pairs(quads);
    let edges = unique_edges(quads);
    let tip = nose_tip(before);
    let showing_before = inner_showing(before, &vertex_normals(before, quads), tip);
    let showing_after = inner_showing(after, &vertex_normals(after, quads), tip);
    let folded_before = folds(before, quads, &pairs);
    let folded_after = folds(after, quads, &pairs);
    let new_folds = folded_before.iter().zip(&folded_after).filter(|(b, a)| **a && !**b).count();
    let (nb, na) = (quad_normals(before, quads), quad_normals(after, quads));
    let faces_flipped = nb.iter().zip(&na).filter(|(b, a)| dot(**a, **b) < 0.0).count();
    let mv: Vec<Point> = after.iter().zip(before).map(|(a, b)| sub(*a, *b)).collect();
    let moved: Vec<f64> = mv.iter().map(|m| norm(*m)).collect();
    let rough = move_roughness(&mv, &edges);
    let limit = limits.rough_mm * MM;
    let rough_over = rough.iter().zip(&moved).filter(|(r, m)| **r > limit && **r > limits.rough_relative * **m).count();
    let rough_absolute = rough.iter().filter(|r| **r > limit).count();
    let touched: Vec<bool> = moved.iter().map(|m| *m > 1e-6).collect();
    let surface = |co: &[Point]| {
        let r = move_roughness(co, &edges);
        let picked: Vec<f64> = r.iter().zip(&touched).filter(|(_, t)| **t).map(|(r, _)| *r).collect();
        if picked.is_empty() { 0.0 } else { picked.iter().sum::<f64>() / picked.len() as f64 }
    };
    let (sb, sa) = (surface(before), surface(after));
    let smoothing = sa < 0.98 * sb;
    let mut sorted = rough.clone();
    sorted.sort_by(f64::total_cmp);
    let count = |v: &[bool]| v.iter().filter(|x| **x).count();
    let (fb, fa) = (count(&folded_before), count(&folded_after));
    let mut failures = Vec::new();
    if fa > fb + limits.fold_tolerance {
        failures.push(format!("folds rose by {}", fa - fb));
    }
    let inner_limit = (limits.inner_tolerance as f64).max(0.25 * showing_before as f64);
    if showing_after as f64 - showing_before as f64 > inner_limit {
        failures.push(format!("inner surfaces showing rose {showing_before} -> {showing_after}"));
    }
    if rough_over > limits.rough_tolerance && !smoothing {
        failures.push(format!("{rough_over} vertices moved unlike their neighbours (> {:.3} mm)", limits.rough_mm));
    }
    Ok(Shell {
        folds_before: fb,
        folds_after: fa,
        new_folds,
        inner_showing_before: showing_before,
        inner_showing_after: showing_after,
        faces_flipped,
        rough_over,
        rough_over_absolute_only: rough_absolute,
        rough_max_mm: sorted.last().copied().unwrap_or(0.0) / MM,
        rough_p999_mm: percentile(&sorted, 99.9) / MM,
        surface_roughness_before_mm: sb / MM,
        surface_roughness_after_mm: sa / MM,
        smoothing,
        max_move_mm: moved.iter().copied().fold(0.0, f64::max) / MM,
        vertices_moved: moved.iter().filter(|m| **m > 1e-5).count(),
        tip,
        limits: limits.clone(),
        pass: failures.is_empty(),
        failures,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize, step: f64) -> (Vec<Point>, Vec<[u32; 4]>) {
        let mut co = Vec::new();
        for i in 0..n {
            for j in 0..n {
                co.push([i as f64 * step, 0.0, j as f64 * step]);
            }
        }
        let mut quads = Vec::new();
        for i in 0..n - 1 {
            for j in 0..n - 1 {
                let v = |a: usize, b: usize| (a * n + b) as u32;
                quads.push([v(i, j), v(i + 1, j), v(i + 1, j + 1), v(i, j + 1)]);
            }
        }
        (co, quads)
    }

    #[test]
    fn a_scatter_is_rough_and_a_smooth_bump_is_not() {
        let (co, quads) = grid(60, 0.0005);
        let limits = ShellLimits { rough_tolerance: 20, ..Default::default() };
        let mut bump = co.clone();
        for p in &mut bump {
            let r2 = ((p[0] - 0.015).powi(2) + (p[2] - 0.015).powi(2)) / 0.01f64.powi(2);
            if r2 < 1.0 {
                p[1] -= 0.001 * (1.0 - r2).powi(3);
            }
        }
        let smooth = shell(&co, &bump, &quads, &limits).unwrap();
        assert!(smooth.pass, "{:?}", smooth.failures);
        let mut scatter = co.clone();
        for (i, p) in scatter.iter_mut().enumerate() {
            if i % 3 == 0 {
                p[1] += if i % 2 == 0 { 0.00015 } else { -0.00015 };
            }
        }
        let rough = shell(&co, &scatter, &quads, &limits).unwrap();
        assert!(!rough.pass && rough.rough_over > 20, "{}", rough.rough_over);
    }

    #[test]
    fn a_folded_quad_pair_is_counted() {
        let (co, quads) = grid(4, 0.001);
        let pairs = face_pairs(&quads);
        assert_eq!(pairs.len(), 12);
        assert_eq!(folds(&co, &quads, &pairs).iter().filter(|f| **f).count(), 0);
        let mut bent = co.clone();
        for j in 0..4 {
            bent[8 + j][0] = 0.0005;
        }
        assert_eq!(folds(&bent, &quads, &pairs).iter().filter(|f| **f).count(), 6);
    }

    #[test]
    fn a_sliver_outside_the_box_fails_and_inside_passes() {
        let tip = [0.0, 0.0, 0.0];
        let far = [[0.1, 0.0, 0.0], [0.11, 0.0, 0.0], [0.105, 0.0, 0.0001]];
        let near = [[0.0, 0.0, 0.01], [0.01, 0.0, 0.01], [0.005, 0.0, 0.0101]];
        let fat = [[0.1, 0.0, 0.0], [0.11, 0.0, 0.0], [0.105, 0.0, 0.008]];
        let quality = |p: [[f64; 3]; 3]| triangle_quality(&p, &[[0, 1, 2]], tip);
        assert!(!quality(far).pass);
        assert!(quality(near).pass && quality(near).under5 == 1);
        assert!(quality(fat).pass);
    }

    #[test]
    fn one_mirrored_uv_triangle_is_a_flip() {
        let game = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 1.0]];
        let tris = [[0, 1, 2], [1, 3, 2], [0, 1, 2]];
        let good = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        let other = [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let flipped = [[0.0, 0.0], [0.0, 1.0], [1.0, 0.0]];
        assert!(uv_health(&game, &tris, &[good, other, good]).pass);
        let bad = uv_health(&game, &tris, &[good, other, flipped]);
        assert_eq!((bad.flipped, bad.pass), (1, false));
    }

    #[test]
    fn a_planted_jump_on_a_path_is_a_pop() {
        let steps: Vec<Vec<Point>> = (0..5).map(|k| vec![[k as f64 * 1e-4, 0.0, 0.0]; 3]).collect();
        assert_eq!(path_pops(&steps, &steps, 0).unwrap().pops, 0);
        let mut placed = steps.clone();
        placed[3][1][1] += 0.002;
        let report = path_pops(&placed, &steps, 0).unwrap();
        assert_eq!((report.pops, report.pass), (2, false));
    }

    #[test]
    fn percentile_matches_linear_interpolation() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 50.0), 2.5);
        assert!((percentile(&v, 95.0) - 3.85).abs() < 1e-12);
    }
}
