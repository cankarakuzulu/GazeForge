use serde::Serialize;

use crate::catalog::SparseTarget;
use crate::game_mesh::Point;
use crate::rig::{self, HEAD, JAW, NECK};

pub struct FaceJoint {
    pub name: &'static str,
    pub parent: usize,
    pub seed: usize,
    pub radius_mm: f64,
    pub rule: &'static str,
}

const fn joint(name: &'static str, parent: usize, seed: usize, radius_mm: f64, rule: &'static str) -> FaceJoint {
    FaceJoint { name, parent, seed, radius_mm, rule }
}

pub const FACE: &[FaceJoint] = &[
    joint("brow_inner_L", HEAD, 677_349, 16.0, "centroid of the +x half of what browInnerUp moves >= 70% of its maximum"),
    joint("brow_mid_L", HEAD, 676_915, 16.0, "centroid of what browDownLeft moves >= 85% of its maximum"),
    joint("brow_outer_L", HEAD, 681_658, 16.0, "centroid of what browOuterUpLeft moves >= 70% of its maximum"),
    joint("brow_inner_R", HEAD, 171_353, 16.0, "centroid of the -x half of what browInnerUp moves >= 70% of its maximum"),
    joint("brow_mid_R", HEAD, 170_904, 16.0, "centroid of what browDownRight moves >= 85% of its maximum"),
    joint("brow_outer_R", HEAD, 175_930, 16.0, "centroid of what browOuterUpRight moves >= 70% of its maximum"),
    joint("lid_upper_L", HEAD, 830_666, 13.0, "centroid of what eyeBlinkLeft moves down >= 70% of its maximum"),
    joint("lid_lower_L", HEAD, 1_023_101, 13.0, "centroid of what eyeSquintLeft moves up >= 70% of its maximum"),
    joint("lid_upper_R", HEAD, 326_174, 13.0, "centroid of what eyeBlinkRight moves down >= 70% of its maximum"),
    joint("lid_lower_R", HEAD, 520_129, 13.0, "centroid of what eyeSquintRight moves up >= 70% of its maximum"),
    joint("cheek_L", HEAD, 1_033_815, 22.0, "centroid of what cheekSquintLeft moves >= 70% of its maximum"),
    joint("cheek_R", HEAD, 531_207, 22.0, "centroid of what cheekSquintRight moves >= 70% of its maximum"),
    joint("nose", HEAD, 185_894, 12.0, "the nose tip landmark (pronasale)"),
    joint("nostril_L", HEAD, 930_597, 9.0, "the alare_l landmark"),
    joint("nostril_R", HEAD, 426_541, 9.0, "the alare_r landmark"),
    joint("lip_corner_L", HEAD, 69_828, 12.0, "centroid of what mouthStretchLeft moves >= 90% of its maximum"),
    joint("lip_corner_R", HEAD, 10_367, 12.0, "centroid of what mouthStretchRight moves >= 90% of its maximum"),
    joint("lip_upper_L", HEAD, 1_035_327, 9.0, "centroid of what mouthUpperUpLeft moves >= 85% of its maximum"),
    joint("lip_upper_C", HEAD, 56_331, 9.0, "centroid of the midline (|x| < 1.5 mm) part of what mouthRollUpper moves >= 70% of its maximum"),
    joint("lip_upper_R", HEAD, 532_523, 9.0, "centroid of what mouthUpperUpRight moves >= 85% of its maximum"),
    joint("lip_lower_L", JAW, 731_326, 9.0, "centroid of what mouthLowerDownLeft moves >= 85% of its maximum"),
    joint("lip_lower_C", JAW, 495, 9.0, "centroid of the midline (|x| < 1.5 mm) part of what mouthShrugLower moves >= 80% of its maximum"),
    joint("lip_lower_R", JAW, 225_681, 9.0, "centroid of what mouthLowerDownRight moves >= 85% of its maximum"),
    joint("chin", JAW, 44_130, 20.0, "the most anterior midline vertex 1416.7..1419.8 mm high on the base profile"),
];

pub const REGION_MM: f64 = 2.0;
pub const MAX_INFLUENCES: usize = 4;
const LID_BAND_M: f64 = 0.004;
const GN_ITERATIONS: usize = 8;
const LAMBDA_T: f64 = 0.1;
const LAMBDA_R: f64 = 1e-6;
const MAX_ROT_JAW_DEG: f64 = 35.0;
const MAX_ROT_FACE_DEG: f64 = 30.0;
const MAX_T_JAW_MM: f64 = 15.0;
const MAX_T_FACE_MM: f64 = 12.0;
const SUPPORT_M: f64 = 1e-4;
const QP_ITERATIONS: usize = 300;
const MU_REL: f64 = 0.05;
const SMOOTH_ITERATIONS: usize = 4;
const SMOOTH_ALPHA: f64 = 0.5;
const WEIGHT_STEPS: f64 = 65536.0;

pub fn count() -> usize {
    rig::JOINTS.len() + FACE.len()
}

pub fn names() -> Vec<String> {
    rig::JOINTS.iter().map(|s| s.to_string()).chain(FACE.iter().map(|j| j.name.to_string())).collect()
}

pub fn parents() -> Vec<Option<usize>> {
    rig::PARENTS.iter().copied().chain(FACE.iter().map(|j| Some(j.parent))).collect()
}

pub fn is_free(j: usize) -> bool {
    j == JAW || j >= rig::JOINTS.len()
}

pub const LIDS: std::ops::Range<usize> = 6..10;
const MAX_ROT_LID_DEG: f64 = 60.0;
const MAX_T_LID_MM: f64 = 6.0;
const CANDIDATE_REACH: f64 = 2.0;
pub const EYE_BEHIND_CANTHI_M: Point = [0.0, 0.0095, 0.001];

fn is_lid(j: usize) -> bool {
    j >= rig::JOINTS.len() && LIDS.contains(&(j - rig::JOINTS.len()))
}

pub fn eye_estimate(positions: &[Point]) -> (Point, Point) {
    let at = |name: &str| crate::measure::landmark(name).map(|l| positions[l.vertex]).unwrap_or([0.0; 3]);
    let centre = |a: &str, b: &str| add(scale(add(at(a), at(b)), 0.5), EYE_BEHIND_CANTHI_M);
    (centre("endocanthion_l", "exocanthion_l"), centre("endocanthion_r", "exocanthion_r"))
}

fn bounds(j: usize) -> (f64, f64) {
    if j == JAW {
        (MAX_ROT_JAW_DEG.to_radians(), MAX_T_JAW_MM / 1000.0)
    } else if is_lid(j) {
        (MAX_ROT_LID_DEG.to_radians(), MAX_T_LID_MM / 1000.0)
    } else {
        (MAX_ROT_FACE_DEG.to_radians(), MAX_T_FACE_MM / 1000.0)
    }
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: Point, s: f64) -> Point {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn norm(a: Point) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

pub type Mat = [[f64; 3]; 3];

pub const IDENTITY: Mat = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

pub fn apply(r: &Mat, v: Point) -> Point {
    [0, 1, 2].map(|i| r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2])
}

pub fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

pub fn transpose(a: &Mat) -> Mat {
    [[a[0][0], a[1][0], a[2][0]], [a[0][1], a[1][1], a[2][1]], [a[0][2], a[1][2], a[2][2]]]
}

pub fn exp_so3(w: Point) -> Mat {
    let theta = norm(w);
    if theta < 1e-12 {
        return [[1.0, -w[2], w[1]], [w[2], 1.0, -w[0]], [-w[1], w[0], 1.0]];
    }
    let k = scale(w, 1.0 / theta);
    let (s, c) = theta.sin_cos();
    let v = 1.0 - c;
    [
        [c + k[0] * k[0] * v, k[0] * k[1] * v - k[2] * s, k[0] * k[2] * v + k[1] * s],
        [k[1] * k[0] * v + k[2] * s, c + k[1] * k[1] * v, k[1] * k[2] * v - k[0] * s],
        [k[2] * k[0] * v - k[1] * s, k[2] * k[1] * v + k[0] * s, c + k[2] * k[2] * v],
    ]
}

pub fn quaternion(r: &Mat) -> [f64; 4] {
    let trace = r[0][0] + r[1][1] + r[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(r[2][1] - r[1][2]) / s, (r[0][2] - r[2][0]) / s, (r[1][0] - r[0][1]) / s, 0.25 * s]
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let s = (1.0 + r[0][0] - r[1][1] - r[2][2]).sqrt() * 2.0;
        [0.25 * s, (r[0][1] + r[1][0]) / s, (r[0][2] + r[2][0]) / s, (r[2][1] - r[1][2]) / s]
    } else if r[1][1] > r[2][2] {
        let s = (1.0 + r[1][1] - r[0][0] - r[2][2]).sqrt() * 2.0;
        [(r[0][1] + r[1][0]) / s, 0.25 * s, (r[1][2] + r[2][1]) / s, (r[0][2] - r[2][0]) / s]
    } else {
        let s = (1.0 + r[2][2] - r[0][0] - r[1][1]).sqrt() * 2.0;
        [(r[0][2] + r[2][0]) / s, (r[1][2] + r[2][1]) / s, 0.25 * s, (r[1][0] - r[0][1]) / s]
    };
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    let sign = if q[3] < 0.0 { -1.0 } else { 1.0 };
    q.map(|x| sign * x / n)
}

pub fn log_so3(r: &Mat) -> Point {
    let q = quaternion(r);
    let s = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
    if s < 1e-12 {
        return [2.0 * q[0], 2.0 * q[1], 2.0 * q[2]];
    }
    let angle = 2.0 * s.atan2(q[3]);
    [q[0] / s * angle, q[1] / s * angle, q[2] / s * angle]
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct JointPose {
    pub rotation: Point,
    pub translation: Point,
}

impl JointPose {
    pub const REST: JointPose = JointPose { rotation: [0.0; 3], translation: [0.0; 3] };

    pub fn matrix(&self) -> Mat {
        exp_so3(self.rotation)
    }

    pub fn moved(&self, pivot: Point, x: Point) -> Point {
        add(add(apply(&self.matrix(), sub(x, pivot)), pivot), self.translation)
    }
}

pub fn regions(base: &[Point]) -> Vec<Vec<u32>> {
    let r = REGION_MM / 1000.0;
    FACE.iter()
        .map(|j| {
            let s = base[j.seed];
            (0..base.len() as u32).filter(|&i| norm(sub(base[i as usize], s)) <= r).collect()
        })
        .collect()
}

pub fn face_pivots(regions: &[Vec<u32>], character: &[Point]) -> Vec<Point> {
    regions
        .iter()
        .map(|reg| {
            let mut sum = [0.0; 3];
            for &i in reg {
                sum = add(sum, character[i as usize]);
            }
            scale(sum, 1.0 / reg.len().max(1) as f64)
        })
        .collect()
}

pub type Influences = Vec<Vec<(usize, f64)>>;

pub struct Problem<'a> {
    pub rest: &'a [Point],
    pub influences: &'a [Vec<(usize, f64)>],
    pub pivots: &'a [Point],
    pub emphasis: Option<&'a [f64]>,
}

pub fn skin(problem: &Problem, poses: &[JointPose]) -> Vec<Point> {
    let mats: Vec<Mat> = poses.iter().map(JointPose::matrix).collect();
    problem
        .rest
        .iter()
        .zip(problem.influences)
        .map(|(&x, inf)| {
            let mut y = [0.0; 3];
            for &(j, w) in inf {
                let m = add(add(apply(&mats[j], sub(x, problem.pivots[j])), problem.pivots[j]), poses[j].translation);
                y = add(y, scale(m, w));
            }
            y
        })
        .collect()
}

fn cholesky_solve(h: &mut [f64], g: &[f64], n: usize) -> Option<Vec<f64>> {
    for j in 0..n {
        let mut d = h[j * n + j];
        for k in 0..j {
            d -= h[j * n + k] * h[j * n + k];
        }
        if d <= 0.0 {
            return None;
        }
        let d = d.sqrt();
        h[j * n + j] = d;
        for i in j + 1..n {
            let mut s = h[i * n + j];
            for k in 0..j {
                s -= h[i * n + k] * h[j * n + k];
            }
            h[i * n + j] = s / d;
        }
    }
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut s = g[i];
        for k in 0..i {
            s -= h[i * n + k] * y[k];
        }
        y[i] = s / h[i * n + i];
    }
    for i in (0..n).rev() {
        let mut s = y[i];
        for k in i + 1..n {
            s -= h[k * n + i] * y[k];
        }
        y[i] = s / h[i * n + i];
    }
    Some(y)
}

fn clamp_pose(j: usize, pose: &mut JointPose) {
    let (max_r, max_t) = bounds(j);
    let a = norm(pose.rotation);
    if a > max_r {
        pose.rotation = scale(pose.rotation, max_r / a);
    }
    let t = norm(pose.translation);
    if t > max_t {
        pose.translation = scale(pose.translation, max_t / t);
    }
}

pub fn solve_shape(problem: &Problem, delta: &[Point]) -> Vec<JointPose> {
    let joints = problem.pivots.len();
    let free: Vec<usize> = (0..joints).filter(|&j| is_free(j)).collect();
    let mut slot = vec![usize::MAX; joints];
    for (s, &j) in free.iter().enumerate() {
        slot[j] = s;
    }
    let n = 6 * free.len();
    let active: Vec<usize> = (0..problem.rest.len()).filter(|&v| problem.influences[v].iter().any(|&(j, w)| w > 0.0 && is_free(j))).collect();
    let mut poses = vec![JointPose::REST; joints];
    for _ in 0..GN_ITERATIONS {
        let mats: Vec<Mat> = poses.iter().map(JointPose::matrix).collect();
        let mut h = vec![0.0; n * n];
        let mut g = vec![0.0; n];
        for (s, &j) in free.iter().enumerate() {
            for k in 0..3 {
                h[(6 * s + k) * n + 6 * s + k] += LAMBDA_R;
                h[(6 * s + 3 + k) * n + 6 * s + 3 + k] += LAMBDA_T;
                g[6 * s + k] += LAMBDA_R * poses[j].rotation[k];
                g[6 * s + 3 + k] += LAMBDA_T * poses[j].translation[k];
            }
        }
        let mut blocks: Vec<(usize, [[f64; 6]; 3])> = Vec::with_capacity(8);
        for &v in &active {
            let x = problem.rest[v];
            let mut y = [0.0; 3];
            blocks.clear();
            for &(j, w) in &problem.influences[v] {
                let a = apply(&mats[j], sub(x, problem.pivots[j]));
                y = add(y, scale(add(add(a, problem.pivots[j]), poses[j].translation), w));
                if is_free(j) && w > 0.0 {
                    let b = [
                        [0.0, w * a[2], -w * a[1], w, 0.0, 0.0],
                        [-w * a[2], 0.0, w * a[0], 0.0, w, 0.0],
                        [w * a[1], -w * a[0], 0.0, 0.0, 0.0, w],
                    ];
                    blocks.push((slot[j], b));
                }
            }
            let r = sub(y, add(x, delta[v]));
            let e = problem.emphasis.map_or(1.0, |e| e[v]);
            for &(s1, b1) in &blocks {
                for c1 in 0..6 {
                    let row = (6 * s1 + c1) * n;
                    g[6 * s1 + c1] += e * (b1[0][c1] * r[0] + b1[1][c1] * r[1] + b1[2][c1] * r[2]);
                    for &(s2, b2) in &blocks {
                        for c2 in 0..6 {
                            h[row + 6 * s2 + c2] += e * (b1[0][c1] * b2[0][c2] + b1[1][c1] * b2[1][c2] + b1[2][c1] * b2[2][c2]);
                        }
                    }
                }
            }
        }
        let Some(step) = cholesky_solve(&mut h, &g, n) else {
            break;
        };
        for (s, &j) in free.iter().enumerate() {
            let dw = [-step[6 * s], -step[6 * s + 1], -step[6 * s + 2]];
            let r = mul(&exp_so3(dw), &mats[j]);
            poses[j].rotation = log_so3(&r);
            for k in 0..3 {
                poses[j].translation[k] -= step[6 * s + 3 + k];
            }
            clamp_pose(j, &mut poses[j]);
        }
    }
    poses
}

#[derive(Debug, Clone, Serialize)]
pub struct ShapeError {
    pub shape_max_mm: f64,
    pub evaluated_vertices: usize,
    pub mean_mm: f64,
    pub p95_mm: f64,
    pub max_mm: f64,
    pub explained: f64,
}

pub fn shape_error(problem: &Problem, delta: &[Point], posed: &[Point]) -> ShapeError {
    let mut errors = Vec::new();
    let (mut residual, mut energy, mut shape_max) = (0.0, 0.0, 0.0f64);
    for v in 0..problem.rest.len() {
        let d = norm(delta[v]);
        let moved = norm(sub(posed[v], problem.rest[v]));
        let e = norm(sub(posed[v], add(problem.rest[v], delta[v])));
        residual += e * e;
        energy += d * d;
        shape_max = shape_max.max(d);
        if d > SUPPORT_M || moved > SUPPORT_M {
            errors.push(e);
        }
    }
    errors.sort_by(f64::total_cmp);
    let mean = if errors.is_empty() { 0.0 } else { errors.iter().sum::<f64>() / errors.len() as f64 };
    let p95 = errors.get(((errors.len() as f64 * 0.95).ceil() as usize).saturating_sub(1)).copied().unwrap_or(0.0);
    ShapeError {
        shape_max_mm: shape_max * 1000.0,
        evaluated_vertices: errors.len(),
        mean_mm: mean * 1000.0,
        p95_mm: p95 * 1000.0,
        max_mm: errors.last().copied().unwrap_or(0.0) * 1000.0,
        explained: if energy > 0.0 { 1.0 - residual / energy } else { 1.0 },
    }
}

pub fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
    gf_geom::map_range(n, f)
}

pub fn fit_shapes(problem: &Problem, deltas: &[Vec<Point>]) -> Vec<(Vec<JointPose>, ShapeError)> {
    par_map(deltas.len(), |s| {
        let poses = solve_shape(problem, &deltas[s]);
        let posed = skin(problem, &poses);
        let error = shape_error(problem, &deltas[s], &posed);
        (poses, error)
    })
}

pub fn game_deltas(targets: &[SparseTarget], kept: &[u32], dense_len: usize) -> Vec<Vec<Point>> {
    let mut game_of = vec![u32::MAX; dense_len];
    for (g, &d) in kept.iter().enumerate() {
        game_of[d as usize] = g as u32;
    }
    targets
        .iter()
        .map(|t| {
            let mut out = vec![[0.0; 3]; kept.len()];
            for (&i, d) in t.indices.iter().zip(&t.deltas) {
                let g = game_of[i as usize];
                if g != u32::MAX {
                    out[g as usize] = [d[0] as f64, d[1] as f64, d[2] as f64];
                }
            }
            out
        })
        .collect()
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn project_simplex(u: &[f64], total: f64) -> Vec<f64> {
    if total <= 0.0 {
        return vec![0.0; u.len()];
    }
    let mut s: Vec<f64> = u.to_vec();
    s.sort_by(|a, b| b.total_cmp(a));
    let (mut cum, mut theta) = (0.0, 0.0);
    for (k, &x) in s.iter().enumerate() {
        cum += x;
        let t = (cum - total) / (k + 1) as f64;
        if x - t > 0.0 {
            theta = t;
        }
    }
    u.iter().map(|&x| (x - theta).max(0.0)).collect()
}

fn simplex_qp(g: &[f64], h: &[f64], start: &[f64], total: f64) -> Vec<f64> {
    let k = h.len();
    let lipschitz = (0..k).map(|i| (0..k).map(|j| g[i * k + j].abs()).sum::<f64>()).fold(0.0, f64::max);
    if lipschitz <= 0.0 {
        return start.to_vec();
    }
    let mut w = project_simplex(start, total);
    let mut y = w.clone();
    let mut t = 1.0f64;
    for _ in 0..QP_ITERATIONS {
        let grad: Vec<f64> = (0..k).map(|i| (0..k).map(|j| g[i * k + j] * y[j]).sum::<f64>() - h[i]).collect();
        let z = project_simplex(&(0..k).map(|i| y[i] - grad[i] / lipschitz).collect::<Vec<_>>(), total);
        let t_next = (1.0 + (1.0 + 4.0 * t * t).sqrt()) / 2.0;
        y = (0..k).map(|i| z[i] + (t - 1.0) / t_next * (z[i] - w[i])).collect();
        w = z;
        t = t_next;
    }
    w
}

#[derive(Debug, Clone, Serialize)]
pub struct WeightReport {
    pub game_vertices: usize,
    pub face_vertices: usize,
    pub refine_rounds: usize,
    pub smooth_iterations: usize,
    pub influence_histogram: Vec<usize>,
    pub explained_by_round: Vec<f64>,
    pub max_sum_error: f64,
    pub method: &'static str,
}

fn sparse(row: &[f64]) -> Vec<(usize, f64)> {
    row.iter().enumerate().filter(|(_, w)| **w > 0.0).map(|(j, w)| (j, *w)).collect()
}

fn mean_explained(fits: &[(Vec<JointPose>, ShapeError)]) -> f64 {
    fits.iter().map(|f| f.1.explained).sum::<f64>() / fits.len().max(1) as f64
}

pub struct WeightInput<'a> {
    pub base: &'a [Point],
    pub kept: &'a [u32],
    pub triangles: &'a [[u32; 3]],
    pub head: &'a [[f64; 3]],
    pub pivots: &'a [Point],
    pub anchors: &'a [Point],
    pub deltas: &'a [Vec<Point>],
    pub rounds: usize,
}

pub fn face_weights(input: &WeightInput) -> (Influences, WeightReport) {
    let joints = count();
    let first = rig::JOINTS.len();
    let rest: Vec<Point> = input.kept.iter().map(|&d| input.base[d as usize]).collect();
    let n = rest.len();
    let lid_mid: Vec<f64> = [6usize, 8].iter().map(|&u| 0.5 * (input.anchors[u][2] + input.anchors[u + 1][2])).collect();
    let mut phi = vec![vec![0.0f64; FACE.len()]; n];
    let mut reach = vec![vec![false; FACE.len()]; n];
    for (v, x) in rest.iter().enumerate() {
        for (f, fj) in FACE.iter().enumerate() {
            let d = norm(sub(*x, input.anchors[f]));
            let r = fj.radius_mm / 1000.0;
            let mut p = smoothstep(1.0 - d / r);
            let mut side = 1.0;
            if LIDS.contains(&f) {
                let mid = lid_mid[(f - 6) / 2];
                let up = smoothstep((x[2] - mid) / LID_BAND_M + 0.5);
                side = if f % 2 == 0 { up } else { 1.0 - up };
                p *= side;
            }
            phi[v][f] = p;
            reach[v][f] = d < CANDIDATE_REACH * r && side > 0.0;
        }
    }
    let mut w = vec![vec![0.0f64; joints]; n];
    let mut face = vec![false; n];
    for v in 0..n {
        let [neck, head, jaw] = input.head[v];
        w[v][NECK] = neck;
        let upper: f64 = FACE.iter().zip(&phi[v]).filter(|(fj, _)| fj.parent == HEAD).map(|(_, p)| p).sum();
        let lower: f64 = FACE.iter().zip(&phi[v]).filter(|(fj, _)| fj.parent == JAW).map(|(_, p)| p).sum();
        w[v][HEAD] = head * (1.0 - upper.min(1.0));
        w[v][JAW] = jaw * (1.0 - lower.min(1.0));
        for (f, fj) in FACE.iter().enumerate() {
            w[v][first + f] = if fj.parent == HEAD { head * phi[v][f] / upper.max(1.0) } else { jaw * phi[v][f] / lower.max(1.0) };
        }
        face[v] = neck < 1.0 && reach[v].iter().any(|&p| p);
    }
    let mut neighbours = vec![Vec::new(); n];
    for t in input.triangles {
        for c in 0..3 {
            let (a, b) = (t[c] as usize, t[(c + 1) % 3] as usize);
            neighbours[a].push(b);
            neighbours[b].push(a);
        }
    }
    for list in &mut neighbours {
        list.sort_unstable();
        list.dedup();
    }
    let mut explained = Vec::new();
    for _ in 0..input.rounds {
        let influences: Influences = w.iter().map(|row| sparse(row)).collect();
        let problem = Problem { rest: &rest, influences: &influences, pivots: input.pivots, emphasis: None };
        let fits = fit_shapes(&problem, input.deltas);
        explained.push(mean_explained(&fits));
        let mats: Vec<Vec<Mat>> = fits.iter().map(|f| f.0.iter().map(JointPose::matrix).collect()).collect();
        let solved = par_map(n, |v| {
            if !face[v] {
                return w[v].clone();
            }
            let x = rest[v];
            let cand: Vec<usize> = (0..joints)
                .filter(|&j| j == HEAD || (j == JAW && input.head[v][2] > 0.0) || (j >= first && reach[v][j - first]))
                .collect();
            let k = cand.len();
            let mut g = vec![0.0; k * k];
            let mut h = vec![0.0; k];
            for (s, fit) in fits.iter().enumerate() {
                let e: Vec<Point> = cand
                    .iter()
                    .map(|&j| sub(add(add(apply(&mats[s][j], sub(x, input.pivots[j])), input.pivots[j]), fit.0[j].translation), x))
                    .collect();
                let d = input.deltas[s][v];
                for a in 0..k {
                    h[a] += e[a][0] * d[0] + e[a][1] * d[1] + e[a][2] * d[2];
                    for b in 0..k {
                        g[a * k + b] += e[a][0] * e[b][0] + e[a][1] * e[b][1] + e[a][2] * e[b][2];
                    }
                }
            }
            let trace: f64 = (0..k).map(|a| g[a * k + a]).sum();
            let mu = MU_REL * trace / k as f64 + 1e-12;
            let start: Vec<f64> = cand.iter().map(|&j| w[v][j]).collect();
            for a in 0..k {
                g[a * k + a] += mu;
                h[a] += mu * start[a];
            }
            let total = 1.0 - w[v][NECK];
            let solved = simplex_qp(&g, &h, &start, total);
            let mut row = vec![0.0; joints];
            row[NECK] = w[v][NECK];
            for (a, &j) in cand.iter().enumerate() {
                row[j] = solved[a];
            }
            row
        });
        w = solved;
        for _ in 0..SMOOTH_ITERATIONS {
            let next: Vec<Vec<f64>> = (0..n)
                .map(|v| {
                    if !face[v] || neighbours[v].is_empty() {
                        return w[v].clone();
                    }
                    let mut row = vec![0.0; joints];
                    let m = neighbours[v].len() as f64;
                    for &u in &neighbours[v] {
                        for j in 0..joints {
                            row[j] += w[u][j] / m;
                        }
                    }
                    let mut out: Vec<f64> = (0..joints).map(|j| (1.0 - SMOOTH_ALPHA) * w[v][j] + SMOOTH_ALPHA * row[j]).collect();
                    out[NECK] = w[v][NECK];
                    let rest_sum: f64 = (0..joints).filter(|&j| j != NECK).map(|j| out[j]).sum();
                    let total = 1.0 - w[v][NECK];
                    for (j, o) in out.iter_mut().enumerate() {
                        if j != NECK {
                            *o = if rest_sum > 0.0 { *o * total / rest_sum } else if j == HEAD { total } else { 0.0 };
                        }
                    }
                    out
                })
                .collect();
            w = next;
        }
    }
    let influences: Influences = w.iter().map(|row| quantise(row)).collect();
    let mut histogram = vec![0usize; MAX_INFLUENCES + 1];
    let mut max_sum_error = 0.0f64;
    for inf in &influences {
        histogram[inf.len()] += 1;
        let s: f32 = inf.iter().map(|&(_, w)| w as f32).sum();
        max_sum_error = max_sum_error.max((s as f64 - 1.0).abs());
    }
    let problem = Problem { rest: &rest, influences: &influences, pivots: input.pivots, emphasis: None };
    explained.push(mean_explained(&fit_shapes(&problem, input.deltas)));
    let report = WeightReport {
        game_vertices: n,
        face_vertices: face.iter().filter(|f| **f).count(),
        refine_rounds: input.rounds,
        smooth_iterations: SMOOTH_ITERATIONS,
        influence_histogram: histogram,
        explained_by_round: explained,
        max_sum_error,
        method: "on the base at the game vertices' dense indices: start from distance falloffs around each facial joint (lids split at the lid line, upper-lip joints share the head weight and lower-lip / chin joints the jaw weight); each round fits every shape's joint poses, then re-solves each face vertex's weights (non-negative, summing to 1 - neck) against all shapes with a pull toward the previous weights, then smooths them over the mesh; finally the 4 largest are kept and quantised to 1/65536 so the f32 sum is exactly 1",
    };
    (influences, report)
}

pub fn quantise(row: &[f64]) -> Vec<(usize, f64)> {
    let mut order: Vec<usize> = (0..row.len()).filter(|&j| row[j] > 0.0).collect();
    order.sort_by(|&a, &b| row[b].total_cmp(&row[a]).then(a.cmp(&b)));
    order.truncate(MAX_INFLUENCES);
    let total: f64 = order.iter().map(|&j| row[j]).sum();
    if total <= 0.0 {
        return vec![(HEAD, 1.0)];
    }
    let mut q: Vec<(usize, f64)> = order.iter().map(|&j| (j, (row[j] / total * WEIGHT_STEPS).round() / WEIGHT_STEPS)).collect();
    let rest: f64 = q[1..].iter().map(|e| e.1).sum();
    q[0].1 = 1.0 - rest;
    q.retain(|e| e.1 > 0.0);
    q.sort_by_key(|e| e.0);
    q
}

pub fn local_pose(parents: &[Option<usize>], pivots: &[Point], poses: &[JointPose]) -> Vec<([f64; 4], Point)> {
    let mats: Vec<Mat> = poses.iter().map(JointPose::matrix).collect();
    (0..pivots.len())
        .map(|j| {
            let own = add(pivots[j], poses[j].translation);
            match parents[j] {
                Some(p) => {
                    let rp = transpose(&mats[p]);
                    let r = mul(&rp, &mats[j]);
                    (quaternion(&r), apply(&rp, sub(own, add(pivots[p], poses[p].translation))))
                }
                None => (quaternion(&mats[j]), own),
            }
        })
        .collect()
}

pub const PIVOT_GRID: f64 = 1_048_576.0;

pub fn neck_seam(triangles: &[[u32; 3]], game: &[Point]) -> Result<Vec<u32>, String> {
    let loops = rig::boundary_loops(triangles);
    let mean_up = |l: &Vec<u32>| l.iter().map(|&v| game[v as usize][2]).sum::<f64>() / l.len() as f64;
    loops.into_iter().min_by(|a, b| mean_up(a).total_cmp(&mean_up(b))).ok_or_else(|| "the game base has no open boundary for the neck".to_string())
}

pub fn mean(points: impl Iterator<Item = Point>) -> Point {
    let (mut sum, mut n) = ([0.0; 3], 0.0);
    for p in points {
        sum = add(sum, p);
        n += 1.0;
    }
    scale(sum, 1.0 / f64::max(n, 1.0))
}

pub fn character_pivots(head: [Point; 4], regions: &[Vec<u32>], character: &[Point]) -> Vec<Point> {
    let [neck, hinge, eye_left, eye_right] = head;
    let mut p: Vec<Point> = rig::joints(neck, hinge, eye_left, eye_right).to_vec();
    let mut face = face_pivots(regions, character);
    for f in LIDS {
        face[f] = if f < 8 { eye_left } else { eye_right };
    }
    p.extend(face);
    p.iter().map(|x| x.map(|c| (c * PIVOT_GRID).round() / PIVOT_GRID)).collect()
}

pub struct BaseRig {
    pub hinge: rig::Hinge,
    pub regions: Vec<Vec<u32>>,
    pub influences: Influences,
    pub report: WeightReport,
    pub skin: rig::SkinReport,
}

pub fn base_rig(dense_base: &[Point], kept: &[u32], triangles: &[[u32; 3]], jaw: &SparseTarget, targets: &[SparseTarget], rounds: usize) -> Result<BaseRig, String> {
    let hinge = rig::fit_hinge(dense_base, jaw)?;
    let share = rig::jaw_share(dense_base, jaw, &hinge);
    let game: Vec<Point> = kept.iter().map(|&d| dense_base[d as usize]).collect();
    let seam = neck_seam(triangles, &game)?;
    let (head, skin) = rig::game_weights(dense_base, kept, &seam, &share, &hinge);
    let regions = regions(dense_base);
    let neck = mean(seam.iter().map(|&g| game[g as usize]));
    let (eye_left, eye_right) = eye_estimate(dense_base);
    let pivots = character_pivots([neck, hinge.centre, eye_left, eye_right], &regions, dense_base);
    let anchors = face_pivots(&regions, dense_base);
    let deltas = game_deltas(targets, kept, dense_base.len());
    let (influences, report) = face_weights(&WeightInput { base: dense_base, kept, triangles, head: &head, pivots: &pivots, anchors: &anchors, deltas: &deltas, rounds });
    Ok(BaseRig { hinge, regions, influences, report, skin })
}

pub const LID_EMPHASIS: f64 = 30.0;
pub const LID_EMPHASIS_REACH: f64 = 1.4;

pub fn lid_emphasis(rest: &[Point], eyes: &[(Point, f64)]) -> Vec<f64> {
    rest.iter()
        .map(|&x| if eyes.iter().any(|&(c, r)| norm(sub(x, c)) < LID_EMPHASIS_REACH * r) { LID_EMPHASIS } else { 1.0 })
        .collect()
}

pub const POOR_EXPLAINED: f64 = 0.8;
pub const POOR_P95_MM: f64 = 1.0;

pub fn poorly_expressed(e: &ShapeError) -> bool {
    e.explained < POOR_EXPLAINED || e.p95_mm > POOR_P95_MM
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotations_round_trip_through_log_and_quaternion() {
        let w = [0.3, -0.2, 0.5];
        let back = log_so3(&exp_so3(w));
        assert!(norm(sub(back, w)) < 1e-12, "{back:?}");
        let q = quaternion(&exp_so3([0.0, 0.0, std::f64::consts::FRAC_PI_2]));
        assert!((q[2] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12 && (q[3] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12);
    }

    #[test]
    fn a_rigid_motion_of_one_joint_is_recovered() {
        let mut rest = Vec::new();
        for i in 0..10 {
            for k in 0..10 {
                rest.push([i as f64 * 0.002, k as f64 * 0.002, 0.001 * ((i + k) % 3) as f64]);
            }
        }
        let joints = count();
        let pivots: Vec<Point> = (0..joints).map(|j| [0.005, 0.005, 0.0001 * j as f64]).collect();
        let target = rig::JOINTS.len() + 3;
        let influences: Influences = rest.iter().map(|_| vec![(target, 1.0)]).collect();
        let truth = JointPose { rotation: [0.05, -0.1, 0.2], translation: [0.001, 0.002, -0.0015] };
        let delta: Vec<Point> = rest.iter().map(|&x| sub(truth.moved(pivots[target], x), x)).collect();
        let problem = Problem { rest: &rest, influences: &influences, pivots: &pivots, emphasis: None };
        let poses = solve_shape(&problem, &delta);
        let posed = skin(&problem, &poses);
        let err = shape_error(&problem, &delta, &posed);
        assert!(err.max_mm < 0.01, "{err:?}");
        assert_eq!(poses[HEAD], JointPose::REST);
    }

    #[test]
    fn quantised_weights_sum_to_one_exactly_in_f32() {
        let q = quantise(&[0.0, 0.3, 0.1, 0.0, 0.0, 0.2, 0.15, 0.25, 0.05]);
        assert_eq!(q.len(), MAX_INFLUENCES);
        assert_eq!(q.iter().map(|e| e.1 as f32).sum::<f32>(), 1.0);
        assert!(q.windows(2).all(|p| p[0].0 < p[1].0));
    }
}
