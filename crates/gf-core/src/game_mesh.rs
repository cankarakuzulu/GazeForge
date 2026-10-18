use std::collections::HashMap;

use gf_geom::Uv;
use gf_geom::decimate::Decimated;
use gf_geom::placement::angle_weighted_normals;

pub type Point = [f64; 3];

pub fn to_gltf(p: Point) -> [f32; 3] {
    [p[0] as f32, p[2] as f32, (-p[1]) as f32]
}

pub struct Surface {
    pub source: Vec<u32>,
    pub uvs: Vec<Uv>,
    pub indices: Vec<u32>,
}

pub fn split_seams(triangles: &[[u32; 3]], triangle_uvs: &[[Uv; 3]]) -> Surface {
    let mut seen: HashMap<(u32, u32, u32), u32> = HashMap::new();
    let mut s = Surface { source: Vec::new(), uvs: Vec::new(), indices: Vec::with_capacity(triangles.len() * 3) };
    for (tri, uvs) in triangles.iter().zip(triangle_uvs) {
        for c in 0..3 {
            let key = (tri[c], uvs[c][0].to_bits(), uvs[c][1].to_bits());
            let next = s.source.len() as u32;
            let index = *seen.entry(key).or_insert(next);
            if index == next {
                s.source.push(tri[c]);
                s.uvs.push(uvs[c]);
            }
            s.indices.push(index);
        }
    }
    s
}

pub fn lod_map(lod0: &Decimated, lod: &Decimated) -> Result<Vec<u32>, String> {
    let index: HashMap<u32, u32> = lod0.kept.iter().enumerate().map(|(i, &d)| (d, i as u32)).collect();
    let mut out = Vec::with_capacity(lod.kept.len());
    let mut missing = 0usize;
    for d in &lod.kept {
        match index.get(d) {
            Some(&i) => out.push(i),
            None => missing += 1,
        }
    }
    if missing > 0 {
        return Err(format!("the LOD is not nested: {missing} of its {} vertices are not vertices of the finest level", lod.kept.len()));
    }
    Ok(out)
}

pub fn lod_uvs(lod0: &Decimated, lod0_uvs: &[[Uv; 3]], lod: &Decimated, map: &[u32]) -> (Vec<[Uv; 3]>, usize, f64) {
    let mut keys: HashMap<u32, Vec<(Uv, Uv)>> = HashMap::new();
    for ((tri, base), placed) in lod0.triangles.iter().zip(&lod0.triangle_uvs).zip(lod0_uvs) {
        for c in 0..3 {
            keys.entry(tri[c]).or_default().push((base[c], placed[c]));
        }
    }
    let mut unmatched = 0usize;
    let mut worst = 0.0f64;
    let out = lod
        .triangles
        .iter()
        .zip(&lod.triangle_uvs)
        .map(|(tri, base)| {
            [0, 1, 2].map(|c| {
                let list = &keys[&map[tri[c] as usize]];
                let distance = |k: &(Uv, Uv)| ((k.0[0] - base[c][0]).abs().max((k.0[1] - base[c][1]).abs())) as f64;
                let best = list.iter().min_by(|a, b| distance(a).total_cmp(&distance(b))).unwrap();
                let d = distance(best);
                worst = worst.max(d);
                if d > 1e-4 {
                    unmatched += 1;
                }
                best.1
            })
        })
        .collect();
    (out, unmatched, worst)
}

pub fn game_normals(game: &[Point], triangles: &[[u32; 3]]) -> Vec<Point> {
    angle_weighted_normals(game, triangles)
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Point, b: Point) -> Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: Point) -> Point {
    let n = dot(a, a).sqrt();
    if n > 0.0 { [a[0] / n, a[1] / n, a[2] / n] } else { [0.0; 3] }
}

pub fn facing_dot(positions: &[Point], normals: &[Point], t: &[u32; 3]) -> f64 {
    let p = t.map(|i| positions[i as usize]);
    let face = unit(cross(sub(p[1], p[0]), sub(p[2], p[0])));
    let n = t.map(|i| normals[i as usize]);
    let mean = unit([n[0][0] + n[1][0] + n[2][0], n[0][1] + n[1][1] + n[2][1], n[0][2] + n[1][2] + n[2][2]]);
    dot(face, mean)
}

pub fn against_count(positions: &[Point], triangles: &[[u32; 3]]) -> usize {
    let normals = angle_weighted_normals(positions, triangles);
    triangles.iter().filter(|t| facing_dot(positions, &normals, t) < 0.0).count()
}

fn min_angle_deg(p: [Point; 3]) -> f64 {
    (0..3)
        .map(|c| {
            let e1 = unit(sub(p[(c + 1) % 3], p[c]));
            let e2 = unit(sub(p[(c + 2) % 3], p[c]));
            dot(e1, e2).clamp(-1.0, 1.0).acos().to_degrees()
        })
        .fold(f64::INFINITY, f64::min)
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct FacingRepair {
    pub against_before: Vec<usize>,
    pub against_after: Vec<usize>,
    pub flips: usize,
    pub rounds: usize,
    pub refused_seam: usize,
    pub refused_check: usize,
    pub refused_neutral: usize,
    pub refused_states: usize,
    pub refused_angle: usize,
    pub flips_relaxed: usize,
}

const FACING_MARGIN: f64 = 0.05;
const FLIP_MIN_ANGLE_DEG: f64 = 5.0;
const FLIP_RELAXED_MIN_ANGLE_DEG: f64 = 0.5;

type Flip = (f64, usize, [[u32; 3]; 2], [[Uv; 3]; 2]);

fn uv_bits(uv: Uv) -> (u32, u32) {
    (uv[0].to_bits(), uv[1].to_bits())
}

pub fn repair_facing(states: &[&[Point]], triangles: &mut [[u32; 3]], uvs: &mut [[Uv; 3]], max_rounds: usize) -> FacingRepair {
    let census = |tris: &[[u32; 3]]| states.iter().map(|s| against_count(s, tris)).collect::<Vec<_>>();
    let mut report = FacingRepair { against_before: census(triangles), ..Default::default() };
    let mut relaxed = false;
    for _ in 0..2 * max_rounds {
        let normals: Vec<Vec<Point>> = states.iter().map(|s| angle_weighted_normals(s, triangles)).collect();
        let dot_of = |s: usize, t: &[u32; 3]| facing_dot(states[s], &normals[s], t);
        let mut bad: Vec<(f64, usize)> =
            triangles.iter().enumerate().map(|(i, t)| (dot_of(0, t), i)).filter(|(d, _)| *d < FACING_MARGIN).collect();
        if bad.is_empty() {
            break;
        }
        bad.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for (i, t) in triangles.iter().enumerate() {
            for c in 0..3 {
                let (a, b) = (t[c], t[(c + 1) % 3]);
                edges.entry((a.min(b), a.max(b))).or_default().push(i);
            }
        }
        let mut created: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
        let mut touched = vec![false; triangles.len()];
        let mut flips = 0usize;
        report.rounds += 1;
        for &(_, t) in &bad {
            if touched[t] {
                continue;
            }
            let tri = triangles[t];
            let mut best: Option<Flip> = None;
            for c in 0..3 {
                let (a, b, cc) = (tri[c], tri[(c + 1) % 3], tri[(c + 2) % 3]);
                let Some(list) = edges.get(&(a.min(b), a.max(b))) else { continue };
                if list.len() != 2 {
                    continue;
                }
                let u = if list[0] == t { list[1] } else { list[0] };
                if touched[u] {
                    continue;
                }
                let ut = triangles[u];
                let Some(k) = (0..3).find(|&k| ut[k] == b && ut[(k + 1) % 3] == a) else { continue };
                let d = ut[(k + 2) % 3];
                let diagonal = (cc.min(d), cc.max(d));
                if d == cc || edges.contains_key(&diagonal) || created.contains(&diagonal) {
                    continue;
                }
                let (uv_t, uv_u) = (uvs[t], uvs[u]);
                if uv_bits(uv_t[c]) != uv_bits(uv_u[(k + 1) % 3]) || uv_bits(uv_t[(c + 1) % 3]) != uv_bits(uv_u[k]) {
                    report.refused_seam += 1;
                    continue;
                }
                let new = [[a, d, cc], [d, b, cc]];
                let new_uvs = [[uv_t[c], uv_u[(k + 2) % 3], uv_t[(c + 2) % 3]], [uv_u[(k + 2) % 3], uv_t[(c + 1) % 3], uv_t[(c + 2) % 3]]];
                let mut neutral_ok = true;
                let mut states_ok = true;
                let mut score = 0.0;
                for s in 0..states.len() {
                    let old_bad = [dot_of(s, &tri), dot_of(s, &ut)].iter().filter(|x| **x < FACING_MARGIN).count();
                    let after = [dot_of(s, &new[0]), dot_of(s, &new[1])];
                    let new_bad = after.iter().filter(|x| **x < FACING_MARGIN).count();
                    if s == 0 {
                        neutral_ok = new_bad == 0;
                        score = after[0].min(after[1]);
                    } else {
                        states_ok &= new_bad <= old_bad;
                    }
                }
                let s0 = states[0];
                let angle = |t: &[u32; 3]| min_angle_deg(t.map(|i| s0[i as usize]));
                let new_angle = angle(&new[0]).min(angle(&new[1]));
                let angle_ok = if relaxed { new_angle > FLIP_RELAXED_MIN_ANGLE_DEG } else { new_angle >= angle(&tri).min(angle(&ut)).min(FLIP_MIN_ANGLE_DEG) };
                report.refused_neutral += !neutral_ok as usize;
                report.refused_states += (neutral_ok && !states_ok) as usize;
                report.refused_angle += (neutral_ok && states_ok && !angle_ok) as usize;
                if !(neutral_ok && states_ok && angle_ok) {
                    report.refused_check += 1;
                    continue;
                }
                if best.as_ref().is_none_or(|b| score > b.0) {
                    best = Some((score, u, new, new_uvs));
                }
            }
            if let Some((_, u, new, new_uvs)) = best {
                let diagonal = (new[0][1].min(new[0][2]), new[0][1].max(new[0][2]));
                created.insert(diagonal);
                triangles[t] = new[0];
                triangles[u] = new[1];
                uvs[t] = new_uvs[0];
                uvs[u] = new_uvs[1];
                touched[t] = true;
                touched[u] = true;
                flips += 1;
            }
        }
        report.flips += flips;
        if relaxed {
            report.flips_relaxed += flips;
        }
        if flips == 0 {
            if relaxed {
                break;
            }
            relaxed = true;
        }
    }
    report.against_after = census(triangles);
    report
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Untangle {
    pub against_before: Vec<usize>,
    pub against_after: Vec<usize>,
    pub iterations: usize,
    pub moved_vertices: usize,
    pub max_move_mm: f64,
    pub capped_vertices: usize,
}

const UNTANGLE_STEP: f64 = 0.5;

pub fn untangle(game: &[Point], lods: &[(&[u32], &[[u32; 3]])], max_iterations: usize, max_move: f64) -> (Vec<Point>, Untangle) {
    let mut moved = game.to_vec();
    let neighbours: Vec<Vec<Vec<u32>>> = lods
        .iter()
        .map(|(map, tris)| {
            let mut n: Vec<Vec<u32>> = vec![Vec::new(); map.len()];
            for t in tris.iter() {
                for c in 0..3 {
                    for o in [t[(c + 1) % 3], t[(c + 2) % 3]] {
                        if !n[t[c] as usize].contains(&o) {
                            n[t[c] as usize].push(o);
                        }
                    }
                }
            }
            n
        })
        .collect();
    let census = |g: &[Point]| -> Vec<Vec<usize>> {
        lods.iter()
            .map(|(map, tris)| {
                let local: Vec<Point> = map.iter().map(|&j| g[j as usize]).collect();
                let normals = angle_weighted_normals(&local, tris);
                (0..tris.len()).filter(|&t| facing_dot(&local, &normals, &tris[t]) < 0.0).collect()
            })
            .collect()
    };
    let mut report = Untangle::default();
    let mut bad = census(&moved);
    report.against_before = bad.iter().map(Vec::len).collect();
    let mut capped = vec![false; game.len()];
    while report.iterations < max_iterations && bad.iter().any(|b| !b.is_empty()) {
        report.iterations += 1;
        let mut target: HashMap<u32, (Point, f64)> = HashMap::new();
        for (k, (map, tris)) in lods.iter().enumerate() {
            for &t in &bad[k] {
                for &v in &tris[t] {
                    let ring = &neighbours[k][v as usize];
                    if ring.is_empty() {
                        continue;
                    }
                    let mut c = [0.0; 3];
                    for &o in ring {
                        let p = moved[map[o as usize] as usize];
                        for (ci, pi) in c.iter_mut().zip(p) {
                            *ci += pi / ring.len() as f64;
                        }
                    }
                    let e = target.entry(map[v as usize]).or_insert(([0.0; 3], 0.0));
                    for (ei, ci) in e.0.iter_mut().zip(c) {
                        *ei += ci;
                    }
                    e.1 += 1.0;
                }
            }
        }
        let mut keys: Vec<u32> = target.keys().copied().collect();
        keys.sort_unstable();
        for g in keys {
            if capped[g as usize] {
                continue;
            }
            let (sum, n) = target[&g];
            let p = moved[g as usize];
            let next = [0, 1, 2].map(|i| p[i] + UNTANGLE_STEP * (sum[i] / n - p[i]));
            if dot(sub(next, game[g as usize]), sub(next, game[g as usize])).sqrt() > max_move {
                capped[g as usize] = true;
                continue;
            }
            moved[g as usize] = next;
        }
        bad = census(&moved);
    }
    report.against_after = bad.iter().map(Vec::len).collect();
    let moves: Vec<f64> = game.iter().zip(&moved).map(|(a, b)| dot(sub(*b, *a), sub(*b, *a)).sqrt()).collect();
    report.moved_vertices = moves.iter().filter(|m| **m > 0.0).count();
    report.max_move_mm = moves.iter().copied().fold(0.0, f64::max) * 1000.0;
    report.capped_vertices = capped.iter().filter(|c| **c).count();
    let delta = game.iter().zip(&moved).map(|(a, b)| sub(*b, *a)).collect();
    (delta, report)
}

pub fn point_triangle_distance(p: Point, a: Point, b: Point, c: Point) -> f64 {
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    let closest = if d1 <= 0.0 && d2 <= 0.0 {
        a
    } else {
        let bp = sub(p, b);
        let (d3, d4) = (dot(ab, bp), dot(ac, bp));
        let cp = sub(p, c);
        let (d5, d6) = (dot(ab, cp), dot(ac, cp));
        let vc = d1 * d4 - d3 * d2;
        let vb = d5 * d2 - d1 * d6;
        let va = d3 * d6 - d5 * d4;
        let along = |o: Point, e: Point, t: f64| [o[0] + e[0] * t, o[1] + e[1] * t, o[2] + e[2] * t];
        if d3 >= 0.0 && d4 <= d3 {
            b
        } else if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            along(a, ab, d1 / (d1 - d3))
        } else if d6 >= 0.0 && d5 <= d6 {
            c
        } else if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            along(a, ac, d2 / (d2 - d6))
        } else if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            along(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)))
        } else {
            let denom = 1.0 / (va + vb + vc);
            let (v, w) = (vb * denom, vc * denom);
            [a[0] + ab[0] * v + ac[0] * w, a[1] + ab[1] * v + ac[1] * w, a[2] + ab[2] * v + ac[2] * w]
        }
    };
    let d = sub(p, closest);
    dot(d, d).sqrt()
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LodError {
    pub max_mm: f64,
    pub p99_mm: f64,
    pub mean_mm: f64,
}

pub fn lod_error(fine: &[Point], owner: &[u32], coarse: &[Point], coarse_triangles: &[[u32; 3]]) -> LodError {
    let mut incident: Vec<Vec<u32>> = vec![Vec::new(); coarse.len()];
    for (i, t) in coarse_triangles.iter().enumerate() {
        for &v in t {
            incident[v as usize].push(i as u32);
        }
    }
    let mut distances: Vec<f64> = fine
        .iter()
        .zip(owner)
        .map(|(&p, &o)| {
            let mut best = f64::INFINITY;
            for &t in &incident[o as usize] {
                for &w in &coarse_triangles[t as usize] {
                    for &s in &incident[w as usize] {
                        let tri = coarse_triangles[s as usize].map(|i| coarse[i as usize]);
                        best = best.min(point_triangle_distance(p, tri[0], tri[1], tri[2]));
                    }
                }
            }
            best
        })
        .collect();
    distances.sort_by(f64::total_cmp);
    let n = distances.len().max(1);
    LodError {
        max_mm: distances.last().copied().unwrap_or(0.0) * 1000.0,
        p99_mm: distances.get(((n - 1) as f64 * 0.99) as usize).copied().unwrap_or(0.0) * 1000.0,
        mean_mm: distances.iter().sum::<f64>() / n as f64 * 1000.0,
    }
}

pub fn screen_size(error_m: f64, radius_m: f64, pixels: f64, screen_height_px: f64) -> f64 {
    2.0 * pixels * radius_m / (error_m * screen_height_px)
}

pub struct Displacement {
    pub entries: usize,
    pub max_mm: f64,
    pub over_0p01_mm: usize,
    pub over_0p1_mm: usize,
}

pub fn displacement(base: &[Point], moved: &[Point]) -> Displacement {
    let mut d = Displacement { entries: 0, max_mm: 0.0, over_0p01_mm: 0, over_0p1_mm: 0 };
    for (a, b) in base.iter().zip(moved) {
        let delta = to_gltf([b[0] - a[0], b[1] - a[1], b[2] - a[2]]);
        if delta.iter().any(|x| *x != 0.0) {
            d.entries += 1;
        }
        let mm = (0..3).map(|k| (b[k] - a[k]).powi(2)).sum::<f64>().sqrt() * 1000.0;
        d.max_mm = d.max_mm.max(mm);
        d.over_0p01_mm += (mm > 0.01) as usize;
        d.over_0p1_mm += (mm > 0.1) as usize;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use gf_geom::decimate::Stats;

    fn mesh(kept: Vec<u32>, triangles: Vec<[u32; 3]>, uvs: Vec<[Uv; 3]>) -> Decimated {
        Decimated { kept, owner: Vec::new(), triangles, triangle_uvs: uvs, stats: Stats::default() }
    }

    #[test]
    fn seams_split_only_where_uvs_differ() {
        let tris = [[0, 1, 2], [2, 1, 3]];
        let uvs = [[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], [[0.0, 1.0], [1.0, 0.0], [1.0, 1.0]]];
        assert_eq!(split_seams(&tris, &uvs).source.len(), 4);
        let cut = [[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], [[0.5, 1.0], [1.0, 0.0], [1.0, 1.0]]];
        let s = split_seams(&tris, &cut);
        assert_eq!(s.source, vec![0, 1, 2, 2, 3]);
        assert_eq!(s.indices, vec![0, 1, 2, 3, 1, 4]);
    }

    #[test]
    fn a_lod_that_is_not_nested_is_refused() {
        let uv = [[0.0f32, 0.0]; 3];
        let fine = mesh(vec![10, 11, 12, 13], vec![[0, 1, 2]], vec![uv]);
        assert_eq!(lod_map(&fine, &mesh(vec![13, 10, 11], vec![[0, 1, 2]], vec![uv])).unwrap(), vec![3, 0, 1]);
        assert!(lod_map(&fine, &mesh(vec![10, 99, 11], vec![[0, 1, 2]], vec![uv])).is_err());
    }

    #[test]
    fn a_folded_triangle_is_counted_and_an_edge_flip_repairs_it() {
        let folded: Vec<Point> = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.5, 1.0, 0.0], [0.5, 0.5, 0.0]];
        let mut tris = vec![[0, 1, 2], [1, 0, 3]];
        assert_eq!(against_count(&folded, &tris), 1);
        let mut uvs: Vec<[Uv; 3]> = tris.iter().map(|tri| tri.map(|i| [folded[i as usize][0] as f32, folded[i as usize][1] as f32])).collect();
        let report = repair_facing(&[&folded], &mut tris, &mut uvs, 8);
        assert_eq!(report.against_before, vec![1]);
        assert_eq!(report.against_after, vec![0]);
        assert_eq!(report.flips, 1);
        assert_eq!(tris, vec![[2, 0, 3], [1, 2, 3]]);
        assert_eq!(uvs[0], [[0.5, 1.0], [0.0, 0.0], [0.5, 0.5]]);
    }

    #[test]
    fn the_distance_to_a_triangle_is_exact_on_its_regions() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(point_triangle_distance([0.2, 0.2, 0.5], a, b, c), 0.5);
        assert_eq!(point_triangle_distance([-1.0, 0.0, 0.0], a, b, c), 1.0);
        assert!((point_triangle_distance([1.0, 1.0, 0.0], a, b, c) - 0.5f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn the_gltf_frame_is_y_up_and_faces_plus_z() {
        assert_eq!(to_gltf([1.0, -2.0, 3.0]), [1.0, 3.0, 2.0]);
    }
}
