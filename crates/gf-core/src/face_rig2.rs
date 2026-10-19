use serde::Serialize;

use crate::face_rig::{self as fr, Influences, JointPose, Mat, Problem, ShapeError, WeightReport};
use crate::game_mesh::Point;
use crate::rig::{self, HEAD, JAW, NECK};
use gf_geom::lids::Eye;

pub const VERSION: u32 = 2;
pub const MAX_INFLUENCES: usize = 8;
const SEEDED: [usize; 20] = [0, 1, 2, 3, 4, 5, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23];
const LID_SLOTS: [&str; 3] = ["in", "mid", "out"];
pub const LID_INNER_H: f64 = 0.15;
pub const LID_OUTER_H: f64 = 0.45;
pub const LID_SPLIT_RAD: f64 = 0.12;
pub const LID_FRONT_COS: [f64; 2] = [-0.3, 0.2];
pub const MAX_ROT_LID_DEG: f64 = 85.0;
pub const MAX_T_LID_MM: f64 = 3.0;
pub const MAX_T_CORNER_MM: f64 = 20.0;
const MAX_ROT_JAW_DEG: f64 = 35.0;
const MAX_T_JAW_MM: f64 = 15.0;
const MAX_ROT_FACE_DEG: f64 = 30.0;
const MAX_T_FACE_MM: f64 = 12.0;
pub const BARRIER_MARGIN_MM: f64 = 0.25;
pub const BARRIER_KAPPA: f64 = 60.0;
pub const BARRIER_REACH: f64 = 1.6;
pub const BARRIER_EXTRA_DEG: f64 = 15.0;
pub const SINGULAR_DET: f64 = 1e-6;
const GN_ITERATIONS: usize = 10;
const LAMBDA_T: f64 = 0.1;
const LAMBDA_R: f64 = 1e-6;
const CANDIDATE_REACH: f64 = 2.0;
const QP_ITERATIONS: usize = 300;
const MU_REL: f64 = 0.05;
const SMOOTH_ITERATIONS: usize = 4;
const SMOOTH_ALPHA: f64 = 0.5;
const WEIGHT_STEPS: f64 = 65536.0;

#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub enum Kind {
    Seed(usize),
    Lid { eye: usize, upper: bool, slot: usize },
}

pub struct Joint {
    pub name: String,
    pub parent: usize,
    pub kind: Kind,
}

pub fn joints() -> Vec<Joint> {
    let mut out: Vec<Joint> = SEEDED.iter().map(|&f| Joint { name: fr::FACE[f].name.to_string(), parent: fr::FACE[f].parent, kind: Kind::Seed(f) }).collect();
    for (eye, side) in [(0usize, "L"), (1usize, "R")] {
        for upper in [true, false] {
            for (slot, part) in LID_SLOTS.iter().enumerate() {
                let lid = if upper { "upper" } else { "lower" };
                out.push(Joint { name: format!("lid_{lid}_{part}_{side}"), parent: HEAD, kind: Kind::Lid { eye, upper, slot } });
            }
        }
    }
    out
}

fn lid_joint(eye: usize, upper: bool, slot: usize) -> usize {
    rig::JOINTS.len() + SEEDED.len() + eye * 6 + if upper { 0 } else { 3 } + slot
}

pub fn count() -> usize {
    rig::JOINTS.len() + SEEDED.len() + 12
}

pub fn names() -> Vec<String> {
    rig::JOINTS.iter().map(|s| s.to_string()).chain(joints().into_iter().map(|j| j.name)).collect()
}

pub fn parents() -> Vec<Option<usize>> {
    rig::PARENTS.iter().copied().chain(joints().iter().map(|j| Some(j.parent))).collect()
}

pub fn bounds() -> Vec<(f64, f64)> {
    let mut b: Vec<(f64, f64)> = (0..rig::JOINTS.len()).map(|j| if j == JAW { (MAX_ROT_JAW_DEG.to_radians(), MAX_T_JAW_MM / 1000.0) } else { (0.0, 0.0) }).collect();
    for j in joints() {
        b.push(match j.kind {
            Kind::Lid { .. } => (MAX_ROT_LID_DEG.to_radians(), MAX_T_LID_MM / 1000.0),
            Kind::Seed(f) if fr::FACE[f].name.starts_with("lip_corner") => (MAX_ROT_FACE_DEG.to_radians(), MAX_T_CORNER_MM / 1000.0),
            Kind::Seed(_) => (MAX_ROT_FACE_DEG.to_radians(), MAX_T_FACE_MM / 1000.0),
        });
    }
    b
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

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn regions(base: &[Point]) -> Vec<Vec<u32>> {
    let r = fr::REGION_MM / 1000.0;
    SEEDED
        .iter()
        .map(|&f| {
            let s = base[fr::FACE[f].seed];
            (0..base.len() as u32).filter(|&i| norm(sub(base[i as usize], s)) <= r).collect()
        })
        .collect()
}

pub fn character_pivots(head: [Point; 4], regions: &[Vec<u32>], character: &[Point]) -> Vec<Point> {
    let [neck, hinge, eye_left, eye_right] = head;
    let mut p: Vec<Point> = rig::joints(neck, hinge, eye_left, eye_right).to_vec();
    let seeded = fr::face_pivots(regions, character);
    let mut k = 0;
    for j in joints() {
        p.push(match j.kind {
            Kind::Seed(_) => {
                k += 1;
                seeded[k - 1]
            }
            Kind::Lid { eye, .. } => {
                if eye == 0 {
                    eye_left
                } else {
                    eye_right
                }
            }
        });
    }
    p.iter().map(|x| x.map(|c| (c * fr::PIVOT_GRID).round() / fr::PIVOT_GRID)).collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct LidFrame {
    pub centre: Point,
    pub margin_radius_m: f64,
    pub frame_points: usize,
    pub medial_x: f64,
    pub lateral_x: f64,
    #[serde(skip)]
    pub upper: Vec<(f64, f64)>,
    #[serde(skip)]
    pub lower: Vec<(f64, f64)>,
}

fn elevation(o: Point) -> f64 {
    o[2].atan2(-o[1])
}

fn interp(profile: &[(f64, f64)], x: f64) -> f64 {
    let (first, last) = (profile[0], profile[profile.len() - 1]);
    if x <= first.0 {
        return first.1;
    }
    if x >= last.0 {
        return last.1;
    }
    for w in profile.windows(2) {
        if x <= w[1].0 {
            let span = w[1].0 - w[0].0;
            let t = if span > 0.0 { (x - w[0].0) / span } else { 0.0 };
            return w[0].1 + t * (w[1].1 - w[0].1);
        }
    }
    last.1
}

pub fn lid_frames(positions: &[Point], centres: [Point; 2]) -> Result<Vec<LidFrame>, String> {
    let at = |name: &str| crate::measure::landmark(name).map(|l| positions[l.vertex]).ok_or(format!("no landmark {name}"));
    centres
        .iter()
        .enumerate()
        .map(|(e, &c)| {
            let side = if e == 0 { "l" } else { "r" };
            let medial = at(&format!("endocanthion_{side}"))?;
            let lateral = at(&format!("exocanthion_{side}"))?;
            let upper = positions[fr::FACE[6 + 2 * e].seed];
            let lower = positions[fr::FACE[7 + 2 * e].seed];
            let profile = |mid: Point| {
                let mut p: Vec<(f64, f64)> = [medial, mid, lateral].iter().map(|&q| (q[0], elevation(sub(q, c)))).collect();
                p.sort_by(|a, b| a.0.total_cmp(&b.0));
                p
            };
            let radius = [medial, lateral, upper, lower].iter().map(|&q| norm(sub(q, c))).sum::<f64>() / 4.0;
            Ok(LidFrame { centre: c, margin_radius_m: radius, frame_points: 4, medial_x: medial[0], lateral_x: lateral[0], upper: profile(upper), lower: profile(lower) })
        })
        .collect()
}

fn lid_share(frame: &LidFrame, x: Point) -> Option<(f64, f64, [f64; 3])> {
    let o = sub(x, frame.centre);
    let d = norm(o);
    if d <= 0.0 {
        return None;
    }
    let h = (d - frame.margin_radius_m) / frame.margin_radius_m;
    let radial = 1.0 - smoothstep((h - LID_INNER_H) / (LID_OUTER_H - LID_INNER_H));
    let front = smoothstep((-o[1] / d - LID_FRONT_COS[0]) / (LID_FRONT_COS[1] - LID_FRONT_COS[0]));
    let t = radial * front;
    if t <= 0.0 {
        return None;
    }
    let mid = 0.5 * (interp(&frame.upper, x[0]) + interp(&frame.lower, x[0]));
    let up = smoothstep((elevation(o) - mid) / LID_SPLIT_RAD + 0.5);
    let u = ((x[0] - frame.medial_x) / (frame.lateral_x - frame.medial_x)).clamp(0.0, 1.0);
    let inner = ((0.5 - u) / 0.3).clamp(0.0, 1.0);
    let outer = ((u - 0.5) / 0.3).clamp(0.0, 1.0);
    Some((t, up, [inner, 1.0 - inner - outer, outer]))
}

pub struct Barrier<'a> {
    pub eyes: &'a [Eye],
    pub eye_of: Vec<Option<usize>>,
    pub front_deg: f64,
}

pub fn barrier<'a>(rest: &[Point], eyes: &'a [Eye], front_deg: f64) -> Barrier<'a> {
    let eye_of = rest
        .iter()
        .map(|&x| {
            eyes.iter().position(|e| {
                let o = sub(x, e.centre);
                let d = norm(o);
                if d <= 0.5 * e.radius || d >= BARRIER_REACH * e.radius {
                    return false;
                }
                let dir = scale(o, 1.0 / d);
                e.polar_deg(dir) <= front_deg + BARRIER_EXTRA_DEG && e.surface(dir).unwrap_or(e.radius).max(e.radius) < d
            })
        })
        .collect();
    Barrier { eyes, eye_of, front_deg }
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

fn clamp_pose(bound: (f64, f64), pose: &mut JointPose) {
    let (max_r, max_t) = bound;
    let a = norm(pose.rotation);
    if a > max_r {
        pose.rotation = scale(pose.rotation, max_r / a);
    }
    let t = norm(pose.translation);
    if t > max_t {
        pose.translation = if t > 0.0 { scale(pose.translation, max_t / t) } else { [0.0; 3] };
    }
}

pub fn solve_shape(problem: &Problem, delta: &[Point], bounds: &[(f64, f64)], barrier: Option<&Barrier>) -> Vec<JointPose> {
    let joints = problem.pivots.len();
    let free: Vec<usize> = (0..joints).filter(|&j| fr::is_free(j)).collect();
    let mut slot = vec![usize::MAX; joints];
    for (s, &j) in free.iter().enumerate() {
        slot[j] = s;
    }
    let n = 6 * free.len();
    let active: Vec<usize> = (0..problem.rest.len()).filter(|&v| problem.influences[v].iter().any(|&(j, w)| w > 0.0 && fr::is_free(j))).collect();
    let margin = BARRIER_MARGIN_MM / 1000.0;
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
        let mut blocks: Vec<(usize, [[f64; 6]; 3])> = Vec::with_capacity(MAX_INFLUENCES);
        for &v in &active {
            let x = problem.rest[v];
            let mut y = [0.0; 3];
            blocks.clear();
            for &(j, w) in &problem.influences[v] {
                let a = fr::apply(&mats[j], sub(x, problem.pivots[j]));
                y = add(y, scale(add(add(a, problem.pivots[j]), poses[j].translation), w));
                if fr::is_free(j) && w > 0.0 {
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
            let Some(bar) = barrier else { continue };
            let Some(k) = bar.eye_of[v] else { continue };
            let eye = &bar.eyes[k];
            let o = sub(y, eye.centre);
            let d = norm(o);
            if d <= 0.0 {
                continue;
            }
            let u = scale(o, 1.0 / d);
            if eye.polar_deg(u) > bar.front_deg + BARRIER_EXTRA_DEG {
                continue;
            }
            let need = eye.surface(u).unwrap_or(eye.radius).max(eye.radius) + margin - d;
            if need <= 0.0 {
                continue;
            }
            let rb = BARRIER_KAPPA * need;
            let jb: Vec<(usize, [f64; 6])> = blocks.iter().map(|&(s, b)| (s, [0, 1, 2, 3, 4, 5].map(|c| -BARRIER_KAPPA * (u[0] * b[0][c] + u[1] * b[1][c] + u[2] * b[2][c])))).collect();
            for &(s1, j1) in &jb {
                for c1 in 0..6 {
                    g[6 * s1 + c1] += j1[c1] * rb;
                    let row = (6 * s1 + c1) * n;
                    for &(s2, j2) in &jb {
                        for c2 in 0..6 {
                            h[row + 6 * s2 + c2] += j1[c1] * j2[c2];
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
            let r = fr::mul(&fr::exp_so3(dw), &mats[j]);
            poses[j].rotation = fr::log_so3(&r);
            for k in 0..3 {
                poses[j].translation[k] -= step[6 * s + 3 + k];
            }
            clamp_pose(bounds[j], &mut poses[j]);
        }
    }
    poses
}

pub fn fit_shapes(problem: &Problem, deltas: &[Vec<Point>], bounds: &[(f64, f64)], barrier: Option<&Barrier>) -> Vec<(Vec<JointPose>, ShapeError)> {
    fr::par_map(deltas.len(), |s| {
        let poses = solve_shape(problem, &deltas[s], bounds, barrier);
        let posed = fr::skin(problem, &poses);
        let error = fr::shape_error(problem, &deltas[s], &posed);
        (poses, error)
    })
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
        return project_simplex(start, total);
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

pub struct WeightInput<'a> {
    pub base: &'a [Point],
    pub kept: &'a [u32],
    pub triangles: &'a [[u32; 3]],
    pub head: &'a [[f64; 3]],
    pub pivots: &'a [Point],
    pub anchors: &'a [Point],
    pub deltas: &'a [Vec<Point>],
    pub rounds: usize,
    pub lids: &'a [LidFrame],
}

fn mean_explained(fits: &[(Vec<JointPose>, ShapeError)]) -> f64 {
    fits.iter().map(|f| f.1.explained).sum::<f64>() / fits.len().max(1) as f64
}

pub fn face_weights(input: &WeightInput) -> (Influences, WeightReport, usize) {
    let joints = count();
    let first = rig::JOINTS.len();
    let ns = SEEDED.len();
    let lid_from = first + ns;
    let bounds = bounds();
    let rest: Vec<Point> = input.kept.iter().map(|&d| input.base[d as usize]).collect();
    let n = rest.len();
    let mut phi = vec![vec![0.0f64; ns]; n];
    let mut reach = vec![vec![false; ns]; n];
    for (v, x) in rest.iter().enumerate() {
        for (k, &f) in SEEDED.iter().enumerate() {
            let d = norm(sub(*x, input.anchors[k]));
            let r = fr::FACE[f].radius_mm / 1000.0;
            phi[v][k] = smoothstep(1.0 - d / r);
            reach[v][k] = d < CANDIDATE_REACH * r;
        }
    }
    let mut lid_start: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    for (v, x) in rest.iter().enumerate() {
        let total = 1.0 - input.head[v][0];
        let best = input.lids.iter().enumerate().filter_map(|(e, f)| lid_share(f, *x).map(|s| (e, s))).max_by(|a, b| a.1 .0.total_cmp(&b.1 .0));
        if let Some((eye, (t, up, hats))) = best {
            for upper in [true, false] {
                let side = if upper { up } else { 1.0 - up };
                for (slot, hat) in hats.iter().enumerate() {
                    let w = total * t * side * hat;
                    if w > 0.0 {
                        lid_start[v].push((lid_joint(eye, upper, slot), w));
                    }
                }
            }
        }
    }
    let lid_vertices = lid_start.iter().filter(|f| !f.is_empty()).count();
    let mut w = vec![vec![0.0f64; joints]; n];
    let mut face = vec![false; n];
    for v in 0..n {
        let [neck, head, jaw] = input.head[v];
        let total = 1.0 - neck;
        let start_sum: f64 = lid_start[v].iter().map(|e| e.1).sum();
        let keep = if total > 0.0 { 1.0 - start_sum / total } else { 1.0 };
        let upper: f64 = (0..ns).filter(|&k| fr::FACE[SEEDED[k]].parent == HEAD).map(|k| phi[v][k]).sum();
        let lower: f64 = (0..ns).filter(|&k| fr::FACE[SEEDED[k]].parent == JAW).map(|k| phi[v][k]).sum();
        w[v][NECK] = neck;
        w[v][HEAD] = keep * head * (1.0 - upper.min(1.0));
        w[v][JAW] = keep * jaw * (1.0 - lower.min(1.0));
        for k in 0..ns {
            w[v][first + k] = keep * if fr::FACE[SEEDED[k]].parent == HEAD { head * phi[v][k] / upper.max(1.0) } else { jaw * phi[v][k] / lower.max(1.0) };
        }
        for &(j, x) in &lid_start[v] {
            w[v][j] = x;
        }
        face[v] = neck < 1.0 && (reach[v].iter().any(|&p| p) || !lid_start[v].is_empty());
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
    let sparse = |row: &[f64]| -> Vec<(usize, f64)> { row.iter().enumerate().filter(|(_, w)| **w > 0.0).map(|(j, w)| (j, *w)).collect() };
    let mut explained = Vec::new();
    for _ in 0..input.rounds {
        let influences: Influences = w.iter().map(|row| sparse(row)).collect();
        let problem = Problem { rest: &rest, influences: &influences, pivots: input.pivots, emphasis: None };
        let fits = fit_shapes(&problem, input.deltas, &bounds, None);
        explained.push(mean_explained(&fits));
        let mats: Vec<Vec<Mat>> = fits.iter().map(|f| f.0.iter().map(JointPose::matrix).collect()).collect();
        let moved = |s: usize, j: usize, x: Point| sub(add(add(fr::apply(&mats[s][j], sub(x, input.pivots[j])), input.pivots[j]), fits[s].0[j].translation), x);
        let solved = fr::par_map(n, |v| {
            let total = 1.0 - w[v][NECK];
            if !face[v] || total <= 1e-9 {
                return w[v].clone();
            }
            let x = rest[v];
            let cand: Vec<usize> = (0..lid_from)
                .filter(|&j| j == HEAD || (j == JAW && input.head[v][2] > 0.0) || (j >= first && reach[v][j - first]))
                .chain(lid_start[v].iter().map(|e| e.0))
                .collect();
            let k = cand.len();
            let mut g = vec![0.0; k * k];
            let mut h = vec![0.0; k];
            for s in 0..fits.len() {
                let e: Vec<Point> = cand.iter().map(|&j| moved(s, j, x)).collect();
                let d = input.deltas[s][v];
                for a in 0..k {
                    h[a] += dot(e[a], d);
                    for b in 0..k {
                        g[a * k + b] += dot(e[a], e[b]);
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
                    let m = neighbours[v].len() as f64;
                    let mut out = w[v].clone();
                    for (j, o) in out.iter_mut().enumerate().skip(1) {
                        let avg: f64 = neighbours[v].iter().map(|&u| w[u][j]).sum::<f64>() / m;
                        *o = (1.0 - SMOOTH_ALPHA) * w[v][j] + SMOOTH_ALPHA * avg;
                    }
                    let free_sum: f64 = out[1..].iter().sum();
                    let total = (1.0 - w[v][NECK]).max(0.0);
                    for (j, o) in out.iter_mut().enumerate().skip(1) {
                        *o = if free_sum > 0.0 { *o * total / free_sum } else if j == HEAD { total } else { 0.0 };
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
    explained.push(mean_explained(&fit_shapes(&problem, input.deltas, &bounds, None)));
    let report = WeightReport {
        game_vertices: n,
        face_vertices: face.iter().filter(|f| **f).count(),
        refine_rounds: input.rounds,
        smooth_iterations: SMOOTH_ITERATIONS,
        influence_histogram: histogram,
        explained_by_round: explained,
        max_sum_error,
        method: "version 2, on the base at the game vertices' dense indices: around each eye (the lid frame: the two canthus landmarks and the upper / lower lid seed vertices, their mean distance from the eye centre the margin radius) the lid joints start with a share that is 1 up to 0.15 margin radii outside the margin radius and falls (smoothstep) to 0 at 0.45, in front of the eye, split upper / lower at the mid line between the two lids' elevation profiles and along the margin into in / mid / out hats; the other joints share the rest as in version 1 (distance falloffs); then rounds of fit-the-poses / re-solve each vertex's weights against all shapes / smooth, and the 8 largest are kept and quantised to 1/65536 so the f32 sum is exactly 1",
    };
    (influences, report, lid_vertices)
}

pub struct BaseRig {
    pub hinge: rig::Hinge,
    pub regions: Vec<Vec<u32>>,
    pub lids: Vec<LidFrame>,
    pub influences: Influences,
    pub report: WeightReport,
    pub lid_vertices: usize,
    pub skin: rig::SkinReport,
}

pub fn base_rig(dense_base: &[Point], kept: &[u32], triangles: &[[u32; 3]], jaw: &crate::catalog::SparseTarget, targets: &[crate::catalog::SparseTarget], rounds: usize) -> Result<BaseRig, String> {
    let hinge = rig::fit_hinge(dense_base, jaw)?;
    let share = rig::jaw_share(dense_base, jaw, &hinge);
    let game: Vec<Point> = kept.iter().map(|&d| dense_base[d as usize]).collect();
    let seam = fr::neck_seam(triangles, &game)?;
    let (head, skin) = rig::game_weights(dense_base, kept, &seam, &share, &hinge);
    let regions = regions(dense_base);
    let neck = fr::mean(seam.iter().map(|&g| game[g as usize]));
    let (eye_left, eye_right) = fr::eye_estimate(dense_base);
    let lids = lid_frames(dense_base, [eye_left, eye_right])?;
    let pivots = character_pivots([neck, hinge.centre, eye_left, eye_right], &regions, dense_base);
    let anchors = fr::face_pivots(&regions, dense_base);
    let deltas = fr::game_deltas(targets, kept, dense_base.len());
    let (influences, report, lid_vertices) =
        face_weights(&WeightInput { base: dense_base, kept, triangles, head: &head, pivots: &pivots, anchors: &anchors, deltas: &deltas, rounds, lids: &lids });
    Ok(BaseRig { hinge, regions, lids, influences, report, lid_vertices, skin })
}

fn solve3(m: &Mat, b: Point) -> Option<Point> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < SINGULAR_DET {
        return None;
    }
    let col = |k: usize| -> Mat {
        let mut a = *m;
        for r in 0..3 {
            a[r][k] = b[r];
        }
        a
    };
    let d = |a: &Mat| a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0]) + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    Some([d(&col(0)) / det, d(&col(1)) / det, d(&col(2)) / det])
}

pub struct Corrective {
    pub positions: Vec<Point>,
    pub normals: Vec<Point>,
    pub singular: usize,
}

pub fn corrective(rest: &[Point], rest_normals: &[Point], influences: &[&[(usize, f64)]], pivots: &[Point], poses: &[JointPose], target: &[Point], target_normals: &[Point]) -> Corrective {
    let mats: Vec<Mat> = poses.iter().map(JointPose::matrix).collect();
    let mut out = Corrective { positions: Vec::with_capacity(rest.len()), normals: Vec::with_capacity(rest.len()), singular: 0 };
    for (v, &x) in rest.iter().enumerate() {
        let mut m = [[0.0; 3]; 3];
        let mut y = [0.0; 3];
        for &(j, w) in influences[v] {
            for (r, row) in m.iter_mut().enumerate() {
                for (c, e) in row.iter_mut().enumerate() {
                    *e += w * mats[j][r][c];
                }
            }
            y = add(y, scale(add(add(fr::apply(&mats[j], sub(x, pivots[j])), pivots[j]), poses[j].translation), w));
        }
        let want = sub(target[v], y);
        let (dp, dn) = match (solve3(&m, want), solve3(&m, target_normals[v])) {
            (Some(dp), Some(n)) => (dp, sub(n, rest_normals[v])),
            _ => {
                out.singular += 1;
                (want, sub(target_normals[v], rest_normals[v]))
            }
        };
        out.positions.push(dp);
        out.normals.push(dn);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_table_is_consistent() {
        let names = names();
        assert_eq!(names.len(), count());
        assert_eq!(parents().len(), count());
        assert_eq!(bounds().len(), count());
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
        assert_eq!(names[lid_joint(1, false, 2)], "lid_lower_out_R");
        assert_eq!(names[lid_joint(0, true, 0)], "lid_upper_in_L");
    }

    #[test]
    fn quantised_eight_weights_sum_to_one_exactly_in_f32() {
        let row: Vec<f64> = (0..12).map(|j| 0.01 + 0.07 * ((j * 7) % 5) as f64).collect();
        let q = quantise(&row);
        assert_eq!(q.len(), MAX_INFLUENCES);
        assert_eq!(q.iter().map(|e| e.1 as f32).sum::<f32>(), 1.0);
    }

    #[test]
    fn a_lid_rotation_about_the_eye_centre_keeps_the_distance() {
        let c = [0.03, -0.1, 1.5];
        let x = [0.03, -0.112, 1.506];
        let pose = JointPose { rotation: [1.2, 0.0, 0.0], translation: [0.0; 3] };
        let y = pose.moved(c, x);
        assert!((norm(sub(y, c)) - norm(sub(x, c))).abs() < 1e-12);
    }
    #[test]
    fn a_corrective_makes_the_skinned_pose_land_on_the_target() {
        let rest = vec![[0.01, 0.0, 0.0], [0.0, 0.02, 0.005], [0.003, -0.01, 0.02]];
        let normals = vec![[0.0, 0.0, 1.0]; 3];
        let rows: Vec<Vec<(usize, f64)>> = vec![vec![(0, 0.7), (1, 0.3)], vec![(0, 0.25), (1, 0.75)], vec![(1, 1.0)]];
        let inf: Vec<&[(usize, f64)]> = rows.iter().map(|r| r.as_slice()).collect();
        let pivots = vec![[0.0; 3], [0.002, 0.001, 0.0]];
        let poses = vec![JointPose { rotation: [0.2, -0.1, 0.3], translation: [0.001, 0.0, -0.002] }, JointPose { rotation: [-0.4, 0.2, 0.1], translation: [0.0, 0.003, 0.0] }];
        let target = vec![[0.012, 0.001, 0.002], [0.001, 0.018, 0.009], [0.0, -0.012, 0.021]];
        let c = corrective(&rest, &normals, &inf, &pivots, &poses, &target, &normals);
        assert_eq!(c.singular, 0);
        let moved: Vec<Point> = rest.iter().zip(&c.positions).map(|(x, d)| add(*x, *d)).collect();
        let problem = Problem { rest: &moved, influences: &rows, pivots: &pivots, emphasis: None };
        for (y, t) in fr::skin(&problem, &poses).iter().zip(&target) {
            assert!(norm(sub(*y, *t)) < 1e-12, "{y:?} {t:?}");
        }
    }
}
