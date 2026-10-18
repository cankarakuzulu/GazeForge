use serde::Serialize;

use crate::catalog::SparseTarget;
use crate::game_mesh::Point;

pub const JOINTS: [&str; 5] = ["neck", "head", "jaw", "eye_L", "eye_R"];
pub const PARENTS: [Option<usize>; 5] = [None, Some(0), Some(1), Some(1), Some(1)];
pub const NECK: usize = 0;
pub const HEAD: usize = 1;
pub const JAW: usize = 2;

const FIT_SHARE: f64 = 0.5;
const AXIS_FLOOR_M: f64 = 0.02;
const ANCHOR_RADIUS_M: f64 = 0.04;
const CHIN_WEIGHT: f64 = 0.9;

#[derive(Debug, Clone, Serialize)]
pub struct Hinge {
    pub centre: Point,
    pub angle_rad: f64,
    pub fitted_vertices: usize,
    pub rms_residual_mm: f64,
    pub max_target_mm: f64,
}

fn dist(a: Point, b: Point) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn solve3(m: [[f64; 3]; 3], r: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(m);
    if d.abs() < 1e-300 {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, slot) in out.iter_mut().enumerate() {
        let mut mk = m;
        for row in 0..3 {
            mk[row][k] = r[row];
        }
        *slot = det(mk) / d;
    }
    Some(out)
}

pub fn fit_hinge(base: &[Point], jaw: &SparseTarget) -> Result<Hinge, String> {
    let lengths: Vec<f64> = jaw.deltas.iter().map(|d| ((d[0] as f64).powi(2) + (d[1] as f64).powi(2) + (d[2] as f64).powi(2)).sqrt()).collect();
    let max = lengths.iter().copied().fold(0.0, f64::max);
    if max <= 0.0 {
        return Err("the jaw target moves nothing".into());
    }
    let mut ata = [[0.0f64; 3]; 3];
    let mut atb = [0.0f64; 3];
    let mut chosen = Vec::new();
    for (k, (&i, d)) in jaw.indices.iter().zip(&jaw.deltas).enumerate() {
        if lengths[k] < FIT_SHARE * max {
            continue;
        }
        let p = base[i as usize];
        for (row, rhs) in [([-p[2], 1.0, 0.0], d[1] as f64), ([p[1], 0.0, -1.0], d[2] as f64)] {
            for a in 0..3 {
                for b in 0..3 {
                    ata[a][b] += row[a] * row[b];
                }
                atb[a] += row[a] * rhs;
            }
        }
        chosen.push(k);
    }
    let [theta, a, b] = solve3(ata, atb).ok_or("the jaw hinge fit is singular")?;
    if theta.abs() < 1e-9 {
        return Err("the jaw target is not a rotation about a left-right axis".into());
    }
    let centre = [0.0, b / theta, a / theta];
    let mut sum = 0.0;
    for &k in &chosen {
        let p = base[jaw.indices[k] as usize];
        let d = jaw.deltas[k];
        let r = [0.0, -theta * (p[2] - centre[2]), theta * (p[1] - centre[1])];
        sum += (d[0] as f64 - r[0]).powi(2) + (d[1] as f64 - r[1]).powi(2) + (d[2] as f64 - r[2]).powi(2);
    }
    Ok(Hinge {
        centre,
        angle_rad: theta,
        fitted_vertices: chosen.len(),
        rms_residual_mm: (sum / chosen.len() as f64).sqrt() * 1000.0,
        max_target_mm: max * 1000.0,
    })
}

pub fn jaw_share(base: &[Point], jaw: &SparseTarget, hinge: &Hinge) -> Vec<f64> {
    let mut out = vec![0.0; base.len()];
    let floor = (hinge.angle_rad * AXIS_FLOOR_M).powi(2);
    for (&i, d) in jaw.indices.iter().zip(&jaw.deltas) {
        let p = base[i as usize];
        let r = [0.0, -hinge.angle_rad * (p[2] - hinge.centre[2]), hinge.angle_rad * (p[1] - hinge.centre[1])];
        let rr = r[1] * r[1] + r[2] * r[2];
        let along = d[0] as f64 * r[0] + d[1] as f64 * r[1] + d[2] as f64 * r[2];
        out[i as usize] = (along / rr.max(floor)).clamp(0.0, 1.0);
    }
    out
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Debug, Clone, Serialize)]
pub struct SkinReport {
    pub seam_vertices: usize,
    pub neck_falloff_mm: f64,
    pub chin_to_seam_mm: f64,
    pub jaw_vertices: usize,
    pub neck_vertices: usize,
    pub seam_jaw_share_max: f64,
}

pub fn game_weights(base: &[Point], kept: &[u32], seam: &[u32], share: &[f64], hinge: &Hinge) -> (Vec<[f64; 3]>, SkinReport) {
    let seam_points: Vec<Point> = seam.iter().map(|&g| base[kept[g as usize] as usize]).collect();
    let to_seam = |p: Point| seam_points.iter().map(|&s| dist(p, s)).fold(f64::INFINITY, f64::min);
    let chin = kept
        .iter()
        .filter(|&&d| share[d as usize] >= CHIN_WEIGHT)
        .map(|&d| base[d as usize])
        .max_by(|a, b| dist(*a, hinge.centre).total_cmp(&dist(*b, hinge.centre)));
    let chin_to_seam = chin.map(to_seam).unwrap_or(0.0);
    let falloff = 0.5 * chin_to_seam;
    let mut on_seam = vec![false; kept.len()];
    for &g in seam {
        on_seam[g as usize] = true;
    }
    let mut seam_jaw_max = 0.0f64;
    let weights: Vec<[f64; 3]> = kept
        .iter()
        .enumerate()
        .map(|(g, &d)| {
            if on_seam[g] {
                seam_jaw_max = seam_jaw_max.max(share[d as usize]);
                return [1.0, 0.0, 0.0];
            }
            let neck = if falloff > 0.0 { smoothstep(1.0 - to_seam(base[d as usize]) / falloff) } else { 0.0 };
            let jaw = share[d as usize] * (1.0 - neck);
            [neck, 1.0 - neck - jaw, jaw]
        })
        .collect();
    let report = SkinReport {
        seam_vertices: seam.len(),
        neck_falloff_mm: falloff * 1000.0,
        chin_to_seam_mm: chin_to_seam * 1000.0,
        jaw_vertices: weights.iter().filter(|w| w[JAW] > 0.0).count(),
        neck_vertices: weights.iter().filter(|w| w[NECK] > 0.0).count(),
        seam_jaw_share_max: seam_jaw_max,
    };
    (weights, report)
}

pub fn character_hinge(base: &[Point], character: &[Point], hinge: &Hinge) -> Point {
    let mut sum = [0.0; 3];
    let mut n = 0usize;
    for (b, c) in base.iter().zip(character) {
        if dist(*b, hinge.centre) < ANCHOR_RADIUS_M {
            for k in 0..3 {
                sum[k] += c[k] - b[k];
            }
            n += 1;
        }
    }
    if n == 0 {
        return hinge.centre;
    }
    [hinge.centre[0] + sum[0] / n as f64, hinge.centre[1] + sum[1] / n as f64, hinge.centre[2] + sum[2] / n as f64]
}

pub fn joints(neck: Point, hinge: Point, eye_left: Point, eye_right: Point) -> [Point; 5] {
    [neck, [neck[0], neck[1], hinge[2]], hinge, eye_left, eye_right]
}

pub fn boundary_loops(triangles: &[[u32; 3]]) -> Vec<Vec<u32>> {
    let mut count: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for t in triangles {
        for c in 0..3 {
            let (a, b) = (t[c], t[(c + 1) % 3]);
            *count.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut edges: Vec<(u32, u32)> = count.into_iter().filter(|(_, n)| *n == 1).map(|(e, _)| e).collect();
    edges.sort_unstable();
    let mut parent: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    fn root(parent: &mut std::collections::BTreeMap<u32, u32>, v: u32) -> u32 {
        let mut r = v;
        while parent[&r] != r {
            r = parent[&r];
        }
        let mut x = v;
        while parent[&x] != r {
            let next = parent[&x];
            parent.insert(x, r);
            x = next;
        }
        r
    }
    for &(a, b) in &edges {
        parent.entry(a).or_insert(a);
        parent.entry(b).or_insert(b);
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        if ra != rb {
            parent.insert(ra.max(rb), ra.min(rb));
        }
    }
    let vertices: Vec<u32> = parent.keys().copied().collect();
    let mut groups: std::collections::BTreeMap<u32, Vec<u32>> = std::collections::BTreeMap::new();
    for v in vertices {
        let r = root(&mut parent, v);
        groups.entry(r).or_default().push(v);
    }
    groups.into_values().collect()
}

pub struct Sphere {
    pub positions: Vec<Point>,
    pub normals: Vec<Point>,
    pub indices: Vec<u32>,
}

pub fn sphere(radius: f64, rings: u32, segments: u32) -> Sphere {
    let mut s = Sphere { positions: Vec::new(), normals: Vec::new(), indices: Vec::new() };
    for r in 0..=rings {
        let phi = std::f64::consts::PI * r as f64 / rings as f64;
        for k in 0..=segments {
            let theta = 2.0 * std::f64::consts::PI * k as f64 / segments as f64;
            let n = [phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()];
            s.normals.push(n);
            s.positions.push([n[0] * radius, n[1] * radius, n[2] * radius]);
        }
    }
    let row = segments + 1;
    for r in 0..rings {
        for k in 0..segments {
            let (a, b) = (r * row + k, (r + 1) * row + k);
            if r > 0 {
                s.indices.extend_from_slice(&[a, a + 1, b]);
            }
            if r + 1 < rings {
                s.indices.extend_from_slice(&[a + 1, b + 1, b]);
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pure_rotation_target_gives_its_hinge_back() {
        let centre = [0.0, 0.01, 1.5];
        let theta = 0.05;
        let base: Vec<Point> = (0..50).map(|i| [0.001 * i as f64, -0.05 - 0.001 * i as f64, 1.42 + 0.0005 * i as f64]).collect();
        let deltas: Vec<[f32; 3]> = base.iter().map(|p| [0.0, (-theta * (p[2] - centre[2])) as f32, (theta * (p[1] - centre[1])) as f32]).collect();
        let target = SparseTarget { indices: (0..50).collect(), deltas };
        let hinge = fit_hinge(&base, &target).unwrap();
        assert!((hinge.angle_rad - theta).abs() < 1e-5);
        assert!(dist(hinge.centre, centre) < 1e-4);
        let share = jaw_share(&base, &target, &hinge);
        assert!(share.iter().all(|w| (*w - 1.0).abs() < 1e-3));
    }

    #[test]
    fn the_seam_is_all_neck_and_weights_sum_to_one() {
        let base: Vec<Point> = vec![[0.0, 0.0, 1.3], [0.0, -0.1, 1.4], [0.0, -0.05, 1.35], [0.0, 0.0, 1.6]];
        let share = vec![0.0, 1.0, 0.4, 0.0];
        let hinge = Hinge { centre: [0.0, 0.0, 1.5], angle_rad: 0.1, fitted_vertices: 1, rms_residual_mm: 0.0, max_target_mm: 1.0 };
        let (w, report) = game_weights(&base, &[0, 1, 2, 3], &[0], &share, &hinge);
        assert_eq!(w[0], [1.0, 0.0, 0.0]);
        assert!(w.iter().all(|x| (x.iter().sum::<f64>() - 1.0).abs() < 1e-12 && x.iter().all(|v| *v >= 0.0)));
        assert_eq!(w[3][2], 0.0);
        assert!(report.chin_to_seam_mm > 0.0);
    }

    #[test]
    fn the_sphere_is_closed_and_outward() {
        let s = sphere(1.0, 8, 12);
        assert_eq!(s.indices.len() % 3, 0);
        for t in s.indices.chunks(3) {
            let p = [s.positions[t[0] as usize], s.positions[t[1] as usize], s.positions[t[2] as usize]];
            let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let c = [(p[0][0] + p[1][0] + p[2][0]) / 3.0, (p[0][1] + p[1][1] + p[2][1]) / 3.0, (p[0][2] + p[1][2] + p[2][2]) / 3.0];
            assert!(n[0] * c[0] + n[1] * c[1] + n[2] * c[2] > 0.0);
        }
    }
}
