use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_json::{Map, Value, json};

use crate::Ctx;
use crate::anim_glb::{self as ag, Channel, Clip};
use crate::commands::{obj, shape};
use crate::error::{ApiError, Code};
use crate::photo;

type Quat = [f64; 4];

const IDENTITY: Quat = [0.0, 0.0, 0.0, 1.0];

pub struct Take {
    pub fps: f64,
    pub names: Vec<String>,
    pub times: Vec<f64>,
    pub scores: Vec<Option<Vec<f64>>>,
    pub rotations: Vec<Option<Quat>>,
    pub faces: Vec<u32>,
    pub face_px: Vec<Option<f64>>,
    pub in_frame: Vec<Option<f64>>,
    pub source: Value,
    pub ignored: Vec<String>,
}

#[derive(Clone, Copy)]
pub struct Euro {
    pub min_cutoff: f64,
    pub beta: f64,
    pub d_cutoff: f64,
}

fn alpha(cutoff: f64, te: f64) -> f64 {
    let tau = 1.0 / (2.0 * std::f64::consts::PI * cutoff.max(1e-9));
    1.0 / (1.0 + tau / te)
}

pub fn one_euro(times: &[f64], xs: &[f64], p: Euro) -> Vec<f64> {
    let mut out = Vec::with_capacity(xs.len());
    let (mut prev, mut dprev, mut tprev) = (0.0, 0.0, 0.0);
    for (i, (&t, &x)) in times.iter().zip(xs).enumerate() {
        if i == 0 {
            prev = x;
            tprev = t;
            out.push(x);
            continue;
        }
        let te = if t > tprev { t - tprev } else { 1e-3 };
        let dx = (x - prev) / te;
        let dhat = dprev + alpha(p.d_cutoff, te) * (dx - dprev);
        let cutoff = p.min_cutoff + p.beta * dhat.abs();
        let xhat = prev + alpha(cutoff, te) * (x - prev);
        prev = xhat;
        dprev = dhat;
        tprev = t;
        out.push(xhat);
    }
    out
}

fn qmul(a: Quat, b: Quat) -> Quat {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn qconj(q: Quat) -> Quat {
    [-q[0], -q[1], -q[2], q[3]]
}

fn qnorm(q: Quat) -> Quat {
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt().max(1e-300);
    [q[0] / n, q[1] / n, q[2] / n, q[3] / n]
}

fn qdot(a: Quat, b: Quat) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

fn qslerp(a: Quat, b: Quat, t: f64) -> Quat {
    let b = if qdot(a, b) < 0.0 { [-b[0], -b[1], -b[2], -b[3]] } else { b };
    let d = qdot(a, b).clamp(-1.0, 1.0);
    if d > 0.9995 {
        return qnorm([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]), a[2] + t * (b[2] - a[2]), a[3] + t * (b[3] - a[3])]);
    }
    let th = d.acos();
    let (sa, sb) = (((1.0 - t) * th).sin() / th.sin(), (t * th).sin() / th.sin());
    [sa * a[0] + sb * b[0], sa * a[1] + sb * b[1], sa * a[2] + sb * b[2], sa * a[3] + sb * b[3]]
}

fn qlog(q: Quat) -> [f64; 3] {
    let q = if q[3] < 0.0 { [-q[0], -q[1], -q[2], -q[3]] } else { q };
    let s = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
    if s < 1e-12 {
        return [2.0 * q[0], 2.0 * q[1], 2.0 * q[2]];
    }
    let angle = 2.0 * s.atan2(q[3]);
    [q[0] / s * angle, q[1] / s * angle, q[2] / s * angle]
}

fn qexp(v: [f64; 3]) -> Quat {
    let angle = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if angle < 1e-12 {
        return qnorm([v[0] / 2.0, v[1] / 2.0, v[2] / 2.0, 1.0]);
    }
    let s = (angle / 2.0).sin() / angle;
    [v[0] * s, v[1] * s, v[2] * s, (angle / 2.0).cos()]
}

fn rotate(q: Quat, v: [f64; 3]) -> [f64; 3] {
    let p = qmul(qmul(q, [v[0], v[1], v[2], 0.0]), qconj(q));
    [p[0], p[1], p[2]]
}

pub fn quat_from_matrix(m: &[f64]) -> Option<Quat> {
    if m.len() != 16 || m.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let mut r = [[0.0; 3]; 3];
    for c in 0..3 {
        let n = (m[c] * m[c] + m[4 + c] * m[4 + c] + m[8 + c] * m[8 + c]).sqrt();
        if n < 1e-9 {
            return None;
        }
        for (row, line) in r.iter_mut().enumerate() {
            line[c] = m[4 * row + c] / n;
        }
    }
    let tr = r[0][0] + r[1][1] + r[2][2];
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
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
    Some(qnorm(q))
}

pub fn angles(q: Quat) -> [f64; 3] {
    let n = rotate(q, [0.0, 0.0, 1.0]);
    let u = rotate(q, [0.0, 1.0, 0.0]);
    let yaw = n[0].atan2(n[2]).to_degrees();
    let pitch = n[1].clamp(-1.0, 1.0).asin().to_degrees();
    let unyaw = qexp([0.0, -n[0].atan2(n[2]), 0.0]);
    let u1 = rotate(unyaw, u);
    let unpitch = qexp([n[1].clamp(-1.0, 1.0).asin(), 0.0, 0.0]);
    let u2 = rotate(unpitch, u1);
    [yaw, pitch, u2[0].atan2(u2[1]).to_degrees()]
}

pub fn from_euler(yaw_deg: f64, pitch_deg: f64, roll_deg: f64) -> Quat {
    let y = qexp([0.0, yaw_deg.to_radians(), 0.0]);
    let p = qexp([-pitch_deg.to_radians(), 0.0, 0.0]);
    let r = qexp([0.0, 0.0, -roll_deg.to_radians()]);
    qnorm(qmul(y, qmul(p, r)))
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    fn gauss(&mut self) -> f64 {
        let (a, b) = (self.next(), self.next());
        (-2.0 * a.ln()).sqrt() * (2.0 * std::f64::consts::PI * b).cos()
    }
}

fn num(v: &Value, key: &str, default: f64) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or(default)
}

fn flag(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn swap_side(name: &str) -> String {
    if let Some(s) = name.strip_suffix("Left") {
        format!("{s}Right")
    } else if let Some(s) = name.strip_suffix("Right") {
        format!("{s}Left")
    } else {
        name.to_string()
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { 0.5 * (v[n / 2 - 1] + v[n / 2]) }
}

fn p95(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() as f64 - 1.0) * 0.95).round() as usize]
}

fn r5(x: f64) -> f64 {
    (x * 1e5).round() / 1e5
}

struct Gate {
    max_yaw: f64,
    max_pitch: f64,
    max_roll: f64,
    min_face_px: f64,
    min_in_frame: f64,
    max_rejected: f64,
    margin: f64,
    reference: Option<Quat>,
    face_frame: bool,
}

fn relative(q0: Quat, q: Quat, face_frame: bool) -> Quat {
    if face_frame { qnorm(qmul(q, qconj(q0))) } else { qnorm(qmul(qconj(q0), q)) }
}

fn mean_quat(qs: impl Iterator<Item = Quat>) -> Option<Quat> {
    let mut acc = [0.0; 4];
    let mut any = false;
    for q in qs {
        let q = if qdot(acc, q) < 0.0 { [-q[0], -q[1], -q[2], -q[3]] } else { q };
        for k in 0..4 {
            acc[k] += q[k];
        }
        any = true;
    }
    any.then(|| qnorm(acc))
}

fn reasons(take: &Take, i: usize, g: &Gate) -> Vec<String> {
    let mut out = Vec::new();
    match take.faces[i] {
        0 => out.push("no_face".to_string()),
        1 => {}
        n => out.push(format!("faces:{n}")),
    }
    if take.faces[i] > 0 && take.scores[i].is_none() {
        out.push("no_scores".into());
    }
    if let Some(q) = take.rotations[i] {
        let a = angles(g.reference.map_or(q, |q0| relative(q0, q, g.face_frame)));
        for (k, (v, lim)) in [("yaw", (a[0], g.max_yaw)), ("pitch", (a[1], g.max_pitch)), ("roll", (a[2], g.max_roll))] {
            if lim > 0.0 && v.abs() > (lim - g.margin).max(0.0) {
                out.push(format!("{k}:{v:.1}"));
            }
        }
    }
    if let Some(px) = take.face_px[i]
        && px < g.min_face_px
    {
        out.push(format!("face_px:{px:.0}"));
    }
    if let Some(f) = take.in_frame[i]
        && f < g.min_in_frame
    {
        out.push(format!("in_frame:{f:.3}"));
    }
    out
}

fn fill<T: Clone>(values: &[Option<T>], accepted: &[bool], lerp: impl Fn(&T, &T, f64) -> T) -> Vec<T> {
    let idx: Vec<usize> = (0..values.len()).filter(|&i| accepted[i] && values[i].is_some()).collect();
    let at = |k: usize| values[idx[k]].clone().expect("accepted values exist");
    (0..values.len())
        .map(|i| match idx.binary_search(&i) {
            Ok(k) => at(k),
            Err(0) => at(0),
            Err(k) if k >= idx.len() => at(idx.len() - 1),
            Err(k) => lerp(&at(k - 1), &at(k), (i - idx[k - 1]) as f64 / (idx[k] - idx[k - 1]) as f64),
        })
        .collect()
}

struct Targets {
    names: Vec<String>,
    from: &'static str,
}

fn targets_of(input: &Value, doc: Option<&Value>, source: &[String]) -> Result<Targets, ApiError> {
    if let Some(doc) = doc {
        let nodes = ag::morph_nodes(doc);
        let first = nodes.first().ok_or_else(|| ApiError::invalid("the character glb has no mesh with morph targets"))?;
        if first.1.is_empty() {
            return Err(ApiError::invalid("the character glb's morph targets have no names (mesh extras.targetNames)"));
        }
        return Ok(Targets { names: first.1.clone(), from: "glb" });
    }
    if let Some(list) = input.get("morphs").and_then(Value::as_array) {
        let names: Vec<String> = list.iter().filter_map(Value::as_str).map(|s| s.rsplit('/').next().unwrap_or(s).to_string()).collect();
        return Ok(Targets { names, from: "morphs" });
    }
    Ok(Targets { names: source.iter().filter(|n| !n.starts_with('_')).cloned().collect(), from: "source" })
}

const ROM_GAIN_MAX: f64 = 16.0;
const LEAK_MIN: f64 = 0.02;
const LOOK_LID: Option<(f64, f64)> = None;
const SQUINT_SMILE: f64 = 0.6;
const HEAD_FACE_FRAME: bool = false;
const MIN_CUTOFF: f64 = 1.0;
const BETA: f64 = 20.0;
const HEAD_MIN_CUTOFF: f64 = 4.0;
const HEAD_BETA: f64 = 8.0;
const HEAD_BIAS: &[(&str, [f64; 3])] = &[];
const GATE_MARGIN_DEG: f64 = 3.0;
const GATE_RELATIVE: bool = false;

fn neutral_frames(input: &Value, times: &[f64], ok: &[bool]) -> Vec<usize> {
    match input.get("neutral") {
        Some(seg) => {
            let (a, b) = (num(seg, "start_s", 0.0), num(seg, "end_s", f64::INFINITY));
            (0..times.len()).filter(|&i| ok[i] && times[i] >= a - 1e-9 && times[i] <= b + 1e-9).collect()
        }
        None => {
            let k = input.get("neutral_frames").and_then(Value::as_u64).unwrap_or(10) as usize;
            (0..times.len()).filter(|&i| ok[i]).take(k.max(1)).collect()
        }
    }
}

struct Window {
    frames: Vec<usize>,
    channels: Vec<usize>,
}

fn rom_windows(input: &Value, times: &[f64], accepted: &[bool], names: &[String]) -> Result<Vec<Window>, ApiError> {
    let mut out = Vec::new();
    for (k, w) in input.get("rom").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
        let (a, b) = (num(w, "start_s", 0.0), num(w, "end_s", 0.0));
        let frames: Vec<usize> = (0..times.len()).filter(|&i| accepted[i] && times[i] >= a - 1e-9 && times[i] <= b + 1e-9).collect();
        let mut channels = Vec::new();
        for c in w.get("channels").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]).iter().filter_map(Value::as_str) {
            let c = names.iter().position(|s| s == c).ok_or_else(|| ApiError::invalid(format!("rom[{k}]: the source has no channel {c}")))?;
            channels.push(c);
        }
        if frames.is_empty() || channels.is_empty() {
            return Err(ApiError::invalid(format!("rom[{k}] holds no accepted frame or names no channel")));
        }
        out.push(Window { frames, channels });
    }
    Ok(out)
}

struct Leak {
    to: usize,
    from: Vec<usize>,
    k: f64,
}

fn side(name: &str) -> Option<&'static str> {
    if name.ends_with("Left") {
        Some("Left")
    } else if name.ends_with("Right") {
        Some("Right")
    } else {
        None
    }
}

fn driver(chans: &[usize], names: &[String], to: usize) -> Vec<usize> {
    let s = side(&names[to]);
    let same: Vec<usize> = chans.iter().copied().filter(|&j| s.is_some() && side(&names[j]) == s).collect();
    if same.is_empty() { chans.to_vec() } else { same }
}

fn leaks_of(windows: &[Window], cal: &[Vec<f64>], names: &[String], profile: &Value) -> Vec<Leak> {
    let mut out = Vec::new();
    if windows.is_empty() {
        let pos = |s: &str| names.iter().position(|x| x == s);
        for l in profile["leaks"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let from: Option<Vec<usize>> = l["from"].as_array().map(|a| a.iter().filter_map(Value::as_str).filter_map(pos).collect());
            if let (Some(to), Some(from), Some(k)) = (l["to"].as_str().and_then(pos), from, l["k"].as_f64())
                && !from.is_empty()
            {
                out.push(Leak { to, from, k });
            }
        }
        return out;
    }
    for w in windows {
        for c in 0..names.len() {
            if w.channels.contains(&c) || names[c].starts_with('_') {
                continue;
            }
            let from = driver(&w.channels, names, c);
            let (mut sxy, mut sxx) = (0.0, 0.0);
            for &i in &w.frames {
                let d = from.iter().map(|&j| cal[j][i]).sum::<f64>() / from.len() as f64;
                sxy += cal[c][i] * d;
                sxx += d * d;
            }
            let k = if sxx > 1e-9 { sxy / sxx } else { 0.0 };
            if k > LEAK_MIN {
                out.push(Leak { to: c, from, k });
            }
        }
    }
    out
}

fn couplings(cfg: &Value, names: &[String], cal: &mut [Vec<f64>]) -> Value {
    let pos = |s: String| names.iter().position(|x| *x == s);
    let look = match cfg.get("look_lid") {
        None => LOOK_LID,
        Some(v) if v.is_object() => Some((num(v, "lo", 0.0), num(v, "hi", 1.0))),
        Some(_) => None,
    };
    if let Some((lo, hi)) = look {
        for s in ["Left", "Right"] {
            let Some(b) = pos(format!("eyeBlink{s}")) else { continue };
            let blink = cal[b].clone();
            for dir in ["Down", "In", "Out", "Up"] {
                let Some(c) = pos(format!("eyeLook{dir}{s}")) else { continue };
                for (v, x) in cal[c].iter_mut().zip(&blink) {
                    *v *= ((hi - x) / (hi - lo).max(1e-6)).clamp(0.0, 1.0);
                }
            }
        }
    }
    let k = match cfg.get("squint_smile") {
        None => SQUINT_SMILE,
        Some(v) => v.as_f64().unwrap_or(0.0),
    };
    if k > 0.0 {
        for s in ["Left", "Right"] {
            let (Some(sq), Some(sm)) = (pos(format!("eyeSquint{s}")), pos(format!("mouthSmile{s}"))) else { continue };
            let smile = cal[sm].clone();
            for (v, x) in cal[sq].iter_mut().zip(&smile) {
                *v = (*v - k * x).max(0.0);
            }
        }
    }
    json!({"look_lid": look.map(|(lo, hi)| json!({"lo": lo, "hi": hi, "rule": "each eyeLook* of a side x clamp((hi - eyeBlink of that side) / (hi - lo), 0, 1): the iris is hidden while the lid closes"})),
           "squint_smile": {"k": k, "rule": "eyeSquint of a side minus k x mouthSmile of that side, floored at 0: a smile's cheek raise lifts the lower lid"}})
}

fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<[f64; 3]>) -> Option<Vec<[f64; 3]>> {
    let n = b.len();
    for col in 0..n {
        let piv = (col..n).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        b.swap(col, piv);
        let (pivot_row, pivot_b) = (a[col].clone(), b[col]);
        for (row, (line, rhs)) in a.iter_mut().zip(b.iter_mut()).enumerate() {
            if row != col {
                let f = line[col] / pivot_row[col];
                for (x, p) in line.iter_mut().zip(&pivot_row).skip(col) {
                    *x -= f * p;
                }
                for (x, p) in rhs.iter_mut().zip(&pivot_b) {
                    *x -= f * p;
                }
            }
        }
    }
    Some((0..n).map(|i| b[i].map(|x| x / a[i][i])).collect())
}

#[allow(clippy::too_many_arguments)]
fn head_bias(input: &Value, profile: &Value, windows: &[Window], neutral: &[usize], rots: Option<&Vec<Quat>>, w: &[Vec<f64>], names: &[String], face_frame: bool) -> Vec<(usize, [f64; 3])> {
    let by_name = |v: &Value| -> Vec<(usize, [f64; 3])> {
        v.as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, a)| {
                        let c = names.iter().position(|x| x == k)?;
                        let a = a.as_array()?;
                        Some((c, [0, 1, 2].map(|i| a.get(i).and_then(Value::as_f64).unwrap_or(0.0))))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    match input.get("head_bias") {
        Some(v) if v.is_object() => return by_name(v),
        Some(_) => return Vec::new(),
        None => {}
    }
    let Some(r) = rots else { return Vec::new() };
    if windows.is_empty() {
        if profile["head_bias"].is_object() {
            return by_name(&profile["head_bias"]);
        }
        return HEAD_BIAS.iter().filter_map(|(n, k)| names.iter().position(|x| x == n).map(|c| (c, *k))).collect();
    }
    let q0 = mean_quat(neutral.iter().map(|&i| r[i])).unwrap_or(IDENTITY);
    let mut chans: Vec<usize> = windows.iter().flat_map(|x| x.channels.iter().copied()).collect();
    chans.sort_unstable();
    chans.dedup();
    let mut frames: Vec<usize> = windows.iter().flat_map(|x| x.frames.iter().copied()).chain(neutral.iter().copied()).collect();
    frames.sort_unstable();
    frames.dedup();
    let m = chans.len();
    let mut a = vec![vec![0.0; m]; m];
    let mut b = vec![[0.0; 3]; m];
    for &f in &frames {
        let y = qlog(relative(q0, r[f], face_frame));
        for (i, &ci) in chans.iter().enumerate() {
            for (j, &cj) in chans.iter().enumerate() {
                a[i][j] += w[ci][f] * w[cj][f];
            }
            for k in 0..3 {
                b[i][k] += w[ci][f] * y[k];
            }
        }
    }
    let ridge = 1e-3 * (0..m).map(|i| a[i][i]).sum::<f64>() / m.max(1) as f64;
    for (i, row) in a.iter_mut().enumerate() {
        row[i] += ridge.max(1e-9);
    }
    solve(a, b).map(|k| chans.into_iter().zip(k).collect()).unwrap_or_default()
}

pub struct Processed {
    pub times: Vec<f64>,
    pub names: Vec<String>,
    pub weights: Vec<Vec<f64>>,
    pub head: Option<Vec<Quat>>,
    pub report: Value,
}

pub fn process(take: &Take, input: &Value, doc: Option<&Value>) -> Result<Processed, ApiError> {
    let n = take.times.len();
    if n == 0 {
        return Err(ApiError::invalid("the take has no frames"));
    }
    let gi = input.get("gate").cloned().unwrap_or(json!({}));
    let face_frame = input.get("head_reference").and_then(Value::as_str).map_or(HEAD_FACE_FRAME, |s| s == "face");
    let gate_relative = flag(&gi, "relative_to_neutral", GATE_RELATIVE);
    let posed: Vec<bool> = (0..n).map(|i| take.faces[i] == 1 && take.rotations[i].is_some()).collect();
    let reference = if gate_relative { mean_quat(neutral_frames(input, &take.times, &posed).into_iter().filter_map(|i| take.rotations[i])) } else { None };
    let gate = Gate {
        max_yaw: num(&gi, "max_yaw_deg", 40.0),
        max_pitch: num(&gi, "max_pitch_deg", 30.0),
        max_roll: num(&gi, "max_roll_deg", 30.0),
        min_face_px: num(&gi, "min_face_px", 60.0),
        min_in_frame: num(&gi, "min_in_frame", 0.98),
        max_rejected: num(&gi, "max_rejected_fraction", 0.5),
        margin: num(&gi, "margin_deg", GATE_MARGIN_DEG),
        reference,
        face_frame,
    };
    let why: Vec<Vec<String>> = (0..n).map(|i| reasons(take, i, &gate)).collect();
    let accepted: Vec<bool> = why.iter().map(Vec::is_empty).collect();
    let rejected: Vec<Value> = (0..n).filter(|&i| !accepted[i]).map(|i| json!({"frame": i, "t": r5(take.times[i]), "reasons": why[i]})).collect();
    let n_ok = accepted.iter().filter(|a| **a).count();
    let fraction = 1.0 - n_ok as f64 / n as f64;
    let gate_report = json!({"limits": {"max_yaw_deg": gate.max_yaw, "max_pitch_deg": gate.max_pitch, "max_roll_deg": gate.max_roll,
                                        "min_face_px": gate.min_face_px, "min_in_frame": gate.min_in_frame, "max_rejected_fraction": gate.max_rejected},
                             "margin_deg": gate.margin, "pose_against": if gate.reference.is_some() { "the mean neutral head rotation" } else if gate_relative { "the camera (no neutral frame with a pose)" } else { "the camera" },
                             "frames": n, "accepted": n_ok, "rejected": n - n_ok, "rejected_fraction": r5(fraction), "filled_by": "linear interpolation between the nearest accepted frames (slerp for the head), held at the ends"});
    if n_ok == 0 {
        return Err(ApiError::new(Code::GateFailed, "no frame passed the per-frame gate").details(json!({"gate": gate_report, "rejected": rejected})));
    }
    if fraction > gate.max_rejected {
        return Err(ApiError::new(Code::GateFailed, format!("{:.1} % of the frames failed the per-frame gate (limit {:.1} %)", 100.0 * fraction, 100.0 * gate.max_rejected))
            .details(json!({"gate": gate_report, "rejected": rejected}))
            .hint("raise gate.max_rejected_fraction, relax a pose limit, or trim the input to the usable part"));
    }
    let sides = input.get("sides").and_then(Value::as_str).unwrap_or("as_is");
    let mirror = flag(input, "mirror", false);
    let swap = (sides == "swap") ^ mirror;
    let src_names: Vec<String> = take.names.iter().map(|s| if swap { swap_side(s) } else { s.clone() }).collect();
    let raw = fill(&take.scores, &accepted, |a: &Vec<f64>, b: &Vec<f64>, t| a.iter().zip(b).map(|(x, y)| x + t * (y - x)).collect());
    let has_head = flag(input, "head", true) && take.rotations.iter().zip(&accepted).any(|(r, a)| *a && r.is_some());
    let rots: Option<Vec<Quat>> = if has_head {
        let mut r = fill(&take.rotations, &accepted, |a: &Quat, b: &Quat, t| qslerp(*a, *b, t));
        if mirror {
            for q in &mut r {
                *q = [q[0], -q[1], -q[2], q[3]];
            }
        }
        Some(r)
    } else {
        None
    };
    let neutral = neutral_frames(input, &take.times, &accepted);
    if neutral.is_empty() {
        return Err(ApiError::invalid("the neutral segment holds no accepted frame").details(json!({"gate": gate_report})));
    }
    let calibrate = flag(input, "calibrate", true);
    let gains = input.get("gains").cloned().unwrap_or(json!({}));
    let profile = input.get("calibration_profile").cloned().unwrap_or(Value::Null);
    let m = src_names.len();
    let rest: Vec<f64> = (0..m).map(|c| if calibrate { median(neutral.iter().map(|&i| raw[i][c]).collect()) } else { 0.0 }).collect();
    let lin: Vec<Vec<f64>> = (0..m)
        .map(|c| {
            let span = (1.0 - rest[c]).max(1e-6);
            raw.iter().map(|f| (f[c] - rest[c]) / span).collect()
        })
        .collect();
    let windows = rom_windows(input, &take.times, &accepted, &src_names)?;
    let gain_max = num(input, "rom_gain_max", ROM_GAIN_MAX).max(1.0);
    let mut rom_gain: Vec<Option<f64>> = vec![None; m];
    for w in &windows {
        for &c in &w.channels {
            let peak = w.frames.iter().map(|&i| lin[c][i]).fold(0.0, f64::max);
            rom_gain[c] = Some(if peak > 1e-9 { (1.0 / peak).clamp(1.0 / gain_max, gain_max) } else { gain_max });
        }
    }
    let gain: Vec<f64> = (0..m)
        .map(|c| {
            let user = gains.get(&src_names[c]).and_then(Value::as_f64).unwrap_or(1.0);
            user * rom_gain[c].or_else(|| profile["gains"].get(&src_names[c]).and_then(Value::as_f64)).unwrap_or(1.0)
        })
        .collect();
    let mut cal: Vec<Vec<f64>> = (0..m).map(|c| lin[c].iter().map(|x| (x * gain[c]).clamp(0.0, 1.0)).collect()).collect();
    let leaks = leaks_of(&windows, &cal, &src_names, &profile);
    let snapshot = cal.clone();
    for l in &leaks {
        for (f, v) in cal[l.to].iter_mut().enumerate() {
            let d = l.from.iter().map(|&j| snapshot[j][f]).sum::<f64>() / l.from.len() as f64;
            *v = (*v - l.k * d).max(0.0);
        }
    }
    let coupling_report = couplings(input.get("couplings").unwrap_or(&Value::Null), &src_names, &mut cal);
    let unbiased = cal.clone();
    let plant = input.get("plant").cloned().unwrap_or(Value::Null);
    let noise = num(&plant, "noise", 0.0);
    if noise > 0.0 {
        let mut rng = Rng(num(&plant, "seed", 7.0).max(1.0) as u64 | 1);
        for ch in &mut cal {
            for v in ch.iter_mut() {
                *v = (*v + noise * rng.gauss()).clamp(0.0, 1.0);
            }
        }
    }
    let si = input.get("smoothing").cloned().unwrap_or(json!({}));
    let smooth = flag(&si, "enabled", true);
    let euro = Euro { min_cutoff: num(&si, "min_cutoff", MIN_CUTOFF), beta: num(&si, "beta", BETA), d_cutoff: num(&si, "d_cutoff", 1.0) };
    let head_euro = Euro { min_cutoff: num(&si, "head_min_cutoff", HEAD_MIN_CUTOFF), beta: num(&si, "head_beta", HEAD_BETA), d_cutoff: num(&si, "d_cutoff", 1.0) };
    let bias_terms = head_bias(input, &profile, &windows, &neutral, rots.as_ref(), &unbiased, &src_names, face_frame);
    if smooth {
        for ch in &mut cal {
            *ch = one_euro(&take.times, ch, euro).into_iter().map(|x| x.clamp(0.0, 1.0)).collect();
        }
    }
    let head: Option<Vec<Quat>> = rots.map(|r| {
        let q0 = mean_quat(neutral.iter().map(|&i| r[i])).unwrap_or(IDENTITY);
        let mut rel: Vec<Quat> = r.iter().map(|q| relative(q0, *q, face_frame)).collect();
        if !bias_terms.is_empty() {
            for (f, q) in rel.iter_mut().enumerate() {
                let mut v = [0.0; 3];
                for (c, k) in &bias_terms {
                    for a in 0..3 {
                        v[a] += k[a] * unbiased[*c][f];
                    }
                }
                *q = qnorm(qmul(*q, qexp([-v[0], -v[1], -v[2]])));
            }
        }
        if !smooth {
            return rel;
        }
        let logs: Vec<[f64; 3]> = rel.iter().map(|q| qlog(*q)).collect();
        let cols: Vec<Vec<f64>> = (0..3).map(|k| one_euro(&take.times, &logs.iter().map(|v| v[k]).collect::<Vec<_>>(), head_euro)).collect();
        (0..n).map(|i| qexp([cols[0][i], cols[1][i], cols[2][i]])).collect()
    });
    let targets = targets_of(input, doc, &src_names)?;
    let mut weights = vec![vec![0.0; targets.names.len()]; n];
    let mut channels = Vec::new();
    let mut missing = Vec::new();
    for (t, name) in targets.names.iter().enumerate() {
        let Some(c) = src_names.iter().position(|s| s == name) else {
            missing.push(name.clone());
            channels.push(json!({"name": name, "source": false}));
            continue;
        };
        for (f, row) in weights.iter_mut().enumerate() {
            row[t] = cal[c][f];
        }
        let col: Vec<f64> = (0..n).map(|f| cal[c][f]).collect();
        let dw: Vec<f64> = col.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
        let still: Vec<f64> = neutral.windows(2).filter(|w| w[1] == w[0] + 1).map(|w| (col[w[1]] - col[w[0]]).abs()).collect();
        channels.push(json!({"name": name, "source": true, "source_name": take.names[c], "rest": r5(rest[c]),
                             "raw_min": r5(raw.iter().map(|f| f[c]).fold(f64::INFINITY, f64::min)), "raw_max": r5(raw.iter().map(|f| f[c]).fold(f64::NEG_INFINITY, f64::max)),
                             "min": r5(col.iter().copied().fold(f64::INFINITY, f64::min)), "max": r5(col.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
                             "mean": r5(col.iter().sum::<f64>() / n as f64), "p95_abs_dw": r5(p95(dw)), "neutral_p95_abs_dw": r5(p95(still)),
                             "neutral_max": r5(neutral.iter().map(|&i| col[i]).fold(0.0, f64::max))}));
    }
    let dropped: Vec<&String> = src_names.iter().filter(|s| !targets.names.contains(s)).collect();
    let head_report = head.as_ref().map(|h| {
        let a: Vec<[f64; 3]> = h.iter().map(|q| angles(*q)).collect();
        let max = |k: usize| r5(a.iter().map(|v| v[k].abs()).fold(0.0, f64::max));
        json!({"relative_to": "the mean head rotation over the neutral frames", "reference": if face_frame { "face" } else { "camera" },
               "expression_bias": bias_terms.iter().map(|(c, k)| (src_names[*c].clone(), json!(k.map(r5)))).collect::<Map<String, Value>>(),
               "max_abs_deg": {"yaw": max(0), "pitch": max(1), "roll": max(2)},
               "convention": "character frame (+Y up, face along +Z): yaw + turns the nose toward +X (the face's left), pitch + lifts the nose, roll + tips the top of the head toward +X"})
    });
    let report = json!({
        "frames": n, "fps": take.fps, "duration_s": r5(take.times[n - 1] - take.times[0]),
        "gate": gate_report, "rejected": rejected,
        "calibration": {"enabled": calibrate, "neutral_frames": neutral.len(), "neutral_first": neutral[0], "neutral_last": neutral[neutral.len() - 1],
                        "rule": "w = clamp((score - rest) / (1 - rest) * gain, 0, 1), rest = the median score over the neutral frames; gain = the user's gain x the ROM gain (1 / the channel's peak in its ROM windows, clamped to [1 / rom_gain_max, rom_gain_max]) or the profile's gain",
                        "rom_windows": windows.iter().map(|w| json!({"frames": w.frames.len(), "channels": w.channels.iter().map(|&c| src_names[c].clone()).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                        "rom_gain_max": gain_max,
                        "gains": (0..m).filter(|&c| (gain[c] - 1.0).abs() > 1e-12).map(|c| (src_names[c].clone(), json!(r5(gain[c])))).collect::<Map<String, Value>>(),
                        "leaks": leaks.iter().map(|l| json!({"to": src_names[l.to], "from": l.from.iter().map(|&j| src_names[j].clone()).collect::<Vec<_>>(), "k": r5(l.k)})).collect::<Vec<_>>()},
        "calibration_profile": {"format": "gazeforge.calibration/1",
                                "gains": (0..m).filter_map(|c| rom_gain[c].map(|g| (src_names[c].clone(), json!(r5(g))))).collect::<Map<String, Value>>(),
                                "leaks": leaks.iter().map(|l| json!({"to": src_names[l.to], "from": l.from.iter().map(|&j| src_names[j].clone()).collect::<Vec<_>>(), "k": r5(l.k)})).collect::<Vec<_>>(),
                                "head_bias": bias_terms.iter().map(|(c, k)| (src_names[*c].clone(), json!(k.map(r5)))).collect::<Map<String, Value>>()},
        "couplings": coupling_report,
        "smoothing": {"enabled": smooth, "filter": "One Euro (Casiez, Roussel, Vogel 2012), per weight channel and per head rotation-vector component",
                      "min_cutoff": euro.min_cutoff, "beta": euro.beta, "d_cutoff": euro.d_cutoff, "head_min_cutoff": head_euro.min_cutoff, "head_beta": head_euro.beta},
        "sides": if swap { "swapped" } else { "as_is" }, "mirror": mirror,
        "retarget": {"by": "name", "targets_from": targets.from, "targets": targets.names.len(), "missing_in_source": missing, "dropped_from_source": dropped, "ignored_columns": take.ignored},
        "channels": channels, "head": head_report, "plant": plant, "source": take.source,
    });
    Ok(Processed { times: take.times.iter().map(|t| t - take.times[0]).collect(), names: targets.names, weights, head, report })
}

fn clip_json(p: &Processed, name: &str, neck_share: f64) -> Value {
    json!({"format": "gazeforge.clip/1", "name": name, "fps": p.report["fps"], "frames": p.times.len(), "duration_s": p.report["duration_s"],
           "times": p.times.iter().map(|t| r5(*t)).collect::<Vec<_>>(), "names": p.names,
           "weights": p.weights.iter().map(|row| row.iter().map(|w| r5(*w)).collect::<Vec<_>>()).collect::<Vec<_>>(),
           "head": p.head.as_ref().map(|h| json!({"rotation_xyzw": h.iter().map(|q| q.iter().map(|x| r5(*x)).collect::<Vec<_>>()).collect::<Vec<_>>(), "neck_share": neck_share})),
           "experimental": p.report["experimental"]})
}

fn split(q: Quat, share: f64) -> (Quat, Quat) {
    if share <= 0.0 {
        return (IDENTITY, q);
    }
    let neck = qslerp(IDENTITY, q, share);
    (neck, qnorm(qmul(qconj(neck), q)))
}

fn node_rotation(doc: &Value, node: usize) -> Quat {
    match doc["nodes"][node]["rotation"].as_array() {
        Some(r) if r.len() == 4 => qnorm([r[0].as_f64().unwrap_or(0.0), r[1].as_f64().unwrap_or(0.0), r[2].as_f64().unwrap_or(0.0), r[3].as_f64().unwrap_or(1.0)]),
        _ => IDENTITY,
    }
}

fn head_channels(doc: &Value, p: &Processed, share: f64, extra_head: Option<&[Quat]>) -> (Vec<Channel>, Vec<String>) {
    let (mut out, mut used) = (Vec::new(), Vec::new());
    let Some(head) = &p.head else { return (out, used) };
    let (Some(h), neck) = (ag::node_index(doc, "head"), ag::node_index(doc, "neck")) else { return (out, used) };
    let (rh, rn) = (node_rotation(doc, h), neck.map(|k| node_rotation(doc, k)).unwrap_or(IDENTITY));
    let mut hv = Vec::with_capacity(head.len() * 4);
    let mut nv = Vec::with_capacity(head.len() * 4);
    for (f, q) in head.iter().enumerate() {
        let (qn, mut qh) = split(*q, if neck.is_some() { share } else { 0.0 });
        if let Some(e) = extra_head {
            qh = qmul(qh, e[f]);
        }
        hv.extend(qnorm(qmul(rh, qh)).iter().map(|x| *x as f32));
        nv.extend(qnorm(qmul(rn, qn)).iter().map(|x| *x as f32));
    }
    out.push(Channel { node: h, path: "rotation", values: hv });
    used.push("head".to_string());
    if let (Some(k), true) = (neck, share > 0.0) {
        out.push(Channel { node: k, path: "rotation", values: nv });
        used.push("neck".to_string());
    }
    (out, used)
}

fn weight_channels(doc: &Value, p: &Processed) -> Vec<Channel> {
    let mut out = Vec::new();
    for (node, names) in ag::morph_nodes(doc) {
        let map: Vec<Option<usize>> = names.iter().map(|n| p.names.iter().position(|x| x == n)).collect();
        let mut values = Vec::with_capacity(p.times.len() * names.len());
        for row in &p.weights {
            values.extend(map.iter().map(|k| k.map_or(0.0, |k| row[k] as f32)));
        }
        out.push(Channel { node, path: "weights", values });
    }
    out
}

fn joint_clip(doc: &Value, bin: &[u8], p: &Processed, name: &str, share: f64) -> Result<Option<(Clip, Value)>, ApiError> {
    let rests = ag::shape_poses(doc, bin, &p.names).map_err(ApiError::invalid)?;
    if rests.is_empty() {
        return Ok(None);
    }
    let head = ag::node_index(doc, "head");
    let mut channels = Vec::new();
    let mut blended_head: Option<Vec<Quat>> = None;
    for r in &rests {
        let mut values = Vec::with_capacity(p.times.len() * r.rest.len());
        let mut head_q = Vec::new();
        for row in &p.weights {
            if r.path == "rotation" {
                let base: Quat = [r.rest[0] as f64, r.rest[1] as f64, r.rest[2] as f64, r.rest[3] as f64];
                let mut acc = base;
                for (s, pose) in &r.poses {
                    let mut q: Quat = [pose[0] as f64, pose[1] as f64, pose[2] as f64, pose[3] as f64];
                    if qdot(q, base) < 0.0 {
                        q = [-q[0], -q[1], -q[2], -q[3]];
                    }
                    for k in 0..4 {
                        acc[k] += row[*s] * (q[k] - base[k]);
                    }
                }
                let acc = qnorm(acc);
                if Some(r.node) == head {
                    head_q.push(qmul(qconj(base), acc));
                } else {
                    values.extend(acc.iter().map(|x| *x as f32));
                }
            } else {
                for k in 0..3 {
                    let mut v = r.rest[k] as f64;
                    for (s, pose) in &r.poses {
                        v += row[*s] * (pose[k] as f64 - r.rest[k] as f64);
                    }
                    values.push(v as f32);
                }
            }
        }
        if Some(r.node) == head && r.path == "rotation" {
            blended_head = Some(head_q);
        } else {
            channels.push(Channel { node: r.node, path: if r.path == "rotation" { "rotation" } else { "translation" }, values });
        }
    }
    let (hc, used) = head_channels(doc, p, share, blended_head.as_deref());
    channels.extend(hc);
    let mut joints: Vec<String> = rests.iter().filter_map(|r| doc["nodes"][r.node]["name"].as_str().map(str::to_string)).collect();
    joints.dedup();
    let note = json!({"approximate": true, "rule": "per joint: rest + sum over shapes of weight * (the shape's fitted pose - rest); rotations summed as quaternion differences and normalised",
                      "joints": joints, "head_joints": used});
    let clip = Clip { name: format!("{name}_joints"), times: p.times.iter().map(|t| *t as f32).collect(), channels,
                      extras: json!({"gazeforge": {"kind": "capture_joints", "approximate": true}}) };
    Ok(Some((clip, note)))
}

fn finish(mut p: Processed, input: &Value, started: Instant) -> Result<Value, ApiError> {
    let out = PathBuf::from(input.get("out").and_then(Value::as_str).unwrap_or(""));
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?;
    }
    let name = input.get("name").and_then(Value::as_str).unwrap_or("capture").to_string();
    let share = num(input, "neck_share", 0.0).clamp(0.0, 1.0);
    let stem = out.display().to_string();
    let clip_path = format!("{stem}.clip.json");
    let report_path = format!("{stem}.report.json");
    let mut report = p.report.clone();
    let mut outputs = json!({"clip": clip_path, "report": report_path});
    if let Some(glb) = input.get("glb").and_then(Value::as_str) {
        let bytes = std::fs::read(glb).map_err(|e| ApiError::not_found(format!("{glb}: {e}")))?;
        let (doc, bin) = gf_io::gltf::chunks(&bytes).map_err(ApiError::invalid)?;
        let lg = input.get("lid_guard").cloned().unwrap_or(json!({}));
        report["lid_guard"] = match input.get("eyes").and_then(Value::as_str) {
            Some(_) if !flag(&lg, "enabled", false) => json!({"ran": false, "reason": "lid_guard.enabled is not true (opt-in: on the tuning clip it cut blink and smile peaks to under a tenth, because a single blink shape at any weight between 0 and 1 already fails the lid rule)"}),
            Some(eyes) => {
                let e = gf_io::load_eyes(Path::new(eyes)).map_err(ApiError::invalid)?;
                let g = crate::anim_guard::Guard::new(&bytes, e, lg.get("front_deg").and_then(Value::as_f64), &p.names).map_err(ApiError::invalid)?;
                let t0 = Instant::now();
                let mut r = g.apply(&mut p.weights);
                r["eyes"] = json!(eyes);
                r["seconds"] = json!(r5(t0.elapsed().as_secs_f64()));
                r
            }
            None => json!({"ran": false, "reason": "no eyes file: give eyes (the character's eyes JSON) to keep the lids out of the eyes on every frame's mix"}),
        };
        let mut channels = weight_channels(&doc, &p);
        let (hc, used) = head_channels(&doc, &p, share, None);
        channels.extend(hc);
        let mut clips = vec![Clip { name: name.clone(), times: p.times.iter().map(|t| *t as f32).collect(), channels,
                                    extras: json!({"gazeforge": {"kind": "capture", "source": report["source"]["kind"]}}) }];
        report["glb"] = json!({"weights_nodes": ag::morph_nodes(&doc).iter().map(|(k, _)| doc["nodes"][*k]["name"].clone()).collect::<Vec<_>>(), "head_joints": used,
                               "rig": ag::node_index(&doc, "head").is_some()});
        if flag(input, "joint_tracks", false) {
            match joint_clip(&doc, bin, &p, &name, share)? {
                Some((clip, note)) => {
                    report["joint_tracks"] = note;
                    clips.push(clip);
                }
                None => report["joint_tracks"] = json!({"written": false, "reason": "the glb has no per-shape joint animations (export with --rig face)"}),
            }
        }
        let written = ag::append(&bytes, &clips).map_err(ApiError::invalid)?;
        let glb_out = format!("{stem}.glb");
        std::fs::write(&glb_out, &written).map_err(|e| ApiError::internal(format!("{glb_out}: {e}")))?;
        outputs["glb"] = json!(glb_out);
        outputs["animations"] = json!(clips.iter().map(|c| c.name.clone()).collect::<Vec<_>>());
    }
    let clip = clip_json(&p, &name, share);
    photo::write_json(Path::new(&clip_path), &clip)?;
    report["outputs"] = outputs;
    report["seconds"] = json!(r5(started.elapsed().as_secs_f64()));
    photo::write_json(Path::new(&report_path), &report)?;
    if flag(input, "return_clip", false) {
        report["clip"] = clip;
    }
    Ok(report)
}

fn read_take(path: &Path) -> Result<Take, ApiError> {
    let text = std::fs::read_to_string(path).map_err(|e| ApiError::not_found(format!("{}: {e}", path.display())))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
    let names: Vec<String> = v["names"].as_array().ok_or_else(|| ApiError::invalid("the take has no names"))?.iter().filter_map(Value::as_str).map(str::to_string).collect();
    let frames = v["frames"].as_array().ok_or_else(|| ApiError::invalid("the take has no frames"))?;
    let mut take = Take { fps: v["fps"].as_f64().unwrap_or(30.0), names, times: Vec::new(), scores: Vec::new(), rotations: Vec::new(), faces: Vec::new(),
                          face_px: Vec::new(), in_frame: Vec::new(), source: v["source"].clone(), ignored: Vec::new() };
    for f in frames {
        take.times.push(f["t"].as_f64().unwrap_or(0.0));
        take.faces.push(f["faces"].as_u64().unwrap_or(0) as u32);
        take.scores.push(f["scores"].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).filter(|s: &Vec<f64>| s.len() == take.names.len()));
        take.rotations.push(f["matrix"].as_array().and_then(|a| quat_from_matrix(&a.iter().map(|x| x.as_f64().unwrap_or(f64::NAN)).collect::<Vec<_>>())));
        take.face_px.push(f["face_px"].as_f64());
        take.in_frame.push(f["in_frame"].as_f64());
    }
    Ok(take)
}

fn load_glb_doc(input: &Value) -> Result<Option<Value>, ApiError> {
    let Some(glb) = input.get("glb").and_then(Value::as_str) else { return Ok(None) };
    let bytes = std::fs::read(glb).map_err(|e| ApiError::not_found(format!("{glb}: {e}")))?;
    Ok(Some(gf_io::gltf::chunks(&bytes).map_err(ApiError::invalid)?.0))
}

pub fn from_video(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let started = Instant::now();
    let doc = load_glb_doc(input)?;
    let take_path = match input.get("take").and_then(Value::as_str) {
        Some(t) => PathBuf::from(t),
        None => {
            if input.get("video").is_some() == input.get("frames").is_some() {
                return Err(ApiError::invalid("give exactly one of video (a file) and frames (a folder of images)"));
            }
            if input.get("frames").is_some() && input.get("fps").is_none() {
                return Err(ApiError::invalid("an image sequence (frames) needs fps"));
            }
            let run = PathBuf::from(format!("{}.track", input.get("out").and_then(Value::as_str).unwrap_or("")));
            std::fs::create_dir_all(&run).map_err(|e| ApiError::internal(format!("{}: {e}", run.display())))?;
            let mut args = json!({});
            for k in ["video", "frames", "fps", "max_faces", "min_confidence", "max_frames", "landmarks"] {
                if let Some(v) = input.get(k) {
                    args[k] = v.clone();
                }
            }
            crate::jobs::progress("track", 0, 1, "face landmarker over the frames");
            let r = photo::worker(input, "anim_worker.py", "track", args, &run)?;
            PathBuf::from(r["take"].as_str().unwrap_or_default())
        }
    };
    crate::jobs::progress("process", 0, 1, "calibrate, smooth, retarget");
    let take = read_take(&take_path)?;
    let mut p = process(&take, input, doc.as_ref())?;
    p.report["take"] = json!(take_path.display().to_string());
    finish(p, input, started)
}

fn timecode(text: &str, fps: f64) -> Option<f64> {
    let parts: Vec<&str> = text.trim().split([':', ';']).collect();
    if parts.len() != 4 {
        return None;
    }
    let (h, m, s): (f64, f64, f64) = (parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?);
    let ff: f64 = parts[3].parse().ok()?;
    Some(((h * 60.0 + m) * 60.0 + s) * fps + ff)
}

pub fn parse_csv(text: &str, fps: f64, head_units: &str, signs: [f64; 3]) -> Result<Take, ApiError> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<String> = lines.next().ok_or_else(|| ApiError::invalid("the CSV is empty"))?.split(',').map(|s| s.trim().trim_matches('"').to_string()).collect();
    let mut names = Vec::new();
    let mut cols = Vec::new();
    let mut ignored = Vec::new();
    let (mut tc_col, mut head_cols) = (None, [None; 3]);
    for (i, h) in header.iter().enumerate() {
        let lower = h.to_ascii_lowercase();
        if lower == "timecode" {
            tc_col = Some(i);
            continue;
        }
        if let Some(k) = ["headyaw", "headpitch", "headroll"].iter().position(|x| *x == lower) {
            head_cols[k] = Some(i);
            continue;
        }
        let mut c = h.chars();
        let camel = match c.next() {
            Some(f) => f.to_ascii_lowercase().to_string() + c.as_str(),
            None => continue,
        };
        if ARKIT.contains(&camel.as_str()) {
            names.push(camel);
            cols.push(i);
        } else {
            ignored.push(h.clone());
        }
    }
    if names.is_empty() {
        return Err(ApiError::invalid("no column of the CSV header is an ARKit blendshape name").details(json!({"header": header})));
    }
    let mut take = Take { fps, names, times: Vec::new(), scores: Vec::new(), rotations: Vec::new(), faces: Vec::new(), face_px: Vec::new(), in_frame: Vec::new(),
                          source: json!({"kind": "csv", "experimental": true}), ignored };
    let mut codes = Vec::new();
    for (row, line) in lines.enumerate() {
        let cells: Vec<&str> = line.split(',').map(|s| s.trim().trim_matches('"')).collect();
        let vals: Vec<Option<f64>> = cols.iter().map(|&c| cells.get(c).and_then(|s| s.parse::<f64>().ok()).filter(|x| x.is_finite())).collect();
        let ok = vals.iter().all(Option::is_some);
        codes.push(tc_col.and_then(|c| cells.get(c)).and_then(|s| timecode(s, fps)));
        take.times.push(row as f64 / fps);
        take.faces.push(if ok { 1 } else { 0 });
        take.scores.push(if ok { Some(vals.into_iter().map(|v| v.unwrap_or(0.0)).collect()) } else { None });
        let angles: Option<Vec<f64>> = head_cols.iter().map(|c| c.and_then(|c| cells.get(c)).and_then(|s| s.parse::<f64>().ok())).collect();
        let rot = match (head_units, angles) {
            ("rad", Some(a)) => Some(from_euler(signs[0] * a[0].to_degrees(), signs[1] * a[1].to_degrees(), signs[2] * a[2].to_degrees())),
            ("deg", Some(a)) => Some(from_euler(signs[0] * a[0], signs[1] * a[1], signs[2] * a[2])),
            _ => None,
        };
        take.rotations.push(rot);
        take.face_px.push(None);
        take.in_frame.push(None);
    }
    if take.times.is_empty() {
        return Err(ApiError::invalid("the CSV has a header and no rows"));
    }
    let use_codes = codes.iter().all(Option::is_some) && codes.windows(2).all(|w| w[1] > w[0]);
    if use_codes {
        let first = codes[0].unwrap_or(0.0);
        take.times = codes.iter().map(|c| (c.unwrap_or(0.0) - first) / fps).collect();
    }
    take.source["time"] = json!(if use_codes { "timecode" } else if tc_col.is_some() { "row index / fps (the timecode column did not parse as HH:MM:SS:FF or was not increasing)" } else { "row index / fps" });
    take.source["head"] = json!(if head_cols.iter().all(Option::is_some) && head_units != "ignore" { head_units } else { "ignored" });
    Ok(take)
}

pub fn from_csv(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let started = Instant::now();
    let path = input.get("csv").and_then(Value::as_str).unwrap_or("");
    let text = std::fs::read_to_string(path).map_err(|e| ApiError::not_found(format!("{path}: {e}")))?;
    let fps = num(input, "fps", 60.0);
    let signs = input.get("head_signs").and_then(Value::as_array).map(|a| [0, 1, 2].map(|k| a.get(k).and_then(Value::as_f64).unwrap_or(1.0))).unwrap_or([1.0; 3]);
    let mut take = parse_csv(&text, fps, input.get("head_units").and_then(Value::as_str).unwrap_or("ignore"), signs)?;
    take.source["path"] = json!(path);
    let doc = load_glb_doc(input)?;
    let mut p = process(&take, input, doc.as_ref())?;
    p.report["experimental"] = json!("the Live Link Face CSV columns are not documented by the app's maker; this importer reads the header by ARKit names and is unconfirmed against a real recording");
    finish(p, input, started)
}

pub const ARKIT: [&str; 52] = [
    "browDownLeft", "browDownRight", "browInnerUp", "browOuterUpLeft", "browOuterUpRight", "cheekPuff", "cheekSquintLeft", "cheekSquintRight",
    "eyeBlinkLeft", "eyeBlinkRight", "eyeLookDownLeft", "eyeLookDownRight", "eyeLookInLeft", "eyeLookInRight", "eyeLookOutLeft", "eyeLookOutRight",
    "eyeLookUpLeft", "eyeLookUpRight", "eyeSquintLeft", "eyeSquintRight", "eyeWideLeft", "eyeWideRight", "jawForward", "jawLeft", "jawOpen",
    "jawRight", "mouthClose", "mouthDimpleLeft", "mouthDimpleRight", "mouthFrownLeft", "mouthFrownRight", "mouthFunnel", "mouthLeft",
    "mouthLowerDownLeft", "mouthLowerDownRight", "mouthPressLeft", "mouthPressRight", "mouthPucker", "mouthRight", "mouthRollLower",
    "mouthRollUpper", "mouthShrugLower", "mouthShrugUpper", "mouthSmileLeft", "mouthSmileRight", "mouthStretchLeft", "mouthStretchRight",
    "mouthUpperUpLeft", "mouthUpperUpRight", "noseSneerLeft", "noseSneerRight", "tongueOut",
];

fn common_props() -> Map<String, Value> {
    let v = json!({
        "out": {"type": "string", "minLength": 1, "description": "output path prefix: writes <out>.clip.json (the clip), <out>.report.json (per-channel stats, every rejected frame) and, with glb, <out>.glb"},
        "glb": {"type": "string", "minLength": 1, "description": "a character glb: its morph target names (mesh extras.targetNames) set the channel order, and a copy with the animation added is written to <out>.glb"},
        "morphs": {"type": "array", "items": {"type": "string", "minLength": 1}, "description": "channel names in order when no glb is given (an id's last path part is its name; default: the source's names)"},
        "name": {"type": "string", "minLength": 1, "description": "animation name (default capture)"},
        "neutral": {"type": "object", "properties": {"start_s": {"type": "number", "minimum": 0}, "end_s": {"type": "number", "minimum": 0}}, "additionalProperties": false,
                    "description": "the neutral segment in seconds; default the first neutral_frames accepted frames"},
        "neutral_frames": {"type": "integer", "minimum": 1, "description": "frames of the neutral segment when neutral is not given (default 10)"},
        "calibrate": {"type": "boolean", "description": "subtract the neutral's per-channel median and rescale (default true)"},
        "gains": {"type": "object", "additionalProperties": {"type": "number", "minimum": 0}, "description": "per-channel gain after calibration, by source name"},
        "smoothing": {"type": "object", "additionalProperties": false, "properties": {
            "enabled": {"type": "boolean"}, "min_cutoff": {"type": "number", "minimum": 0}, "beta": {"type": "number", "minimum": 0}, "d_cutoff": {"type": "number", "minimum": 0},
            "head_min_cutoff": {"type": "number", "minimum": 0}, "head_beta": {"type": "number", "minimum": 0}},
            "description": "One Euro filter options (defaults: on, min_cutoff 1 Hz, beta 20, d_cutoff 1 Hz; head: min_cutoff 4 Hz, beta 8)"},
        "gate": {"type": "object", "additionalProperties": false, "properties": {
            "max_yaw_deg": {"type": "number", "minimum": 0}, "max_pitch_deg": {"type": "number", "minimum": 0}, "max_roll_deg": {"type": "number", "minimum": 0},
            "min_face_px": {"type": "number", "minimum": 0}, "min_in_frame": {"type": "number", "minimum": 0, "maximum": 1},
            "max_rejected_fraction": {"type": "number", "minimum": 0, "maximum": 1},
            "margin_deg": {"type": "number", "minimum": 0, "description": "a frame is rejected when a pose angle exceeds its limit minus this margin (the landmarker's angle error; default 3)"},
            "relative_to_neutral": {"type": "boolean", "description": "measure the pose limits against the mean neutral head rotation instead of the camera"}},
            "description": "per-frame gate limits (defaults: yaw 40, pitch 30, roll 30 degrees against the camera, a pose limit of 0 turns it off; face height 60 px; 98 % of the landmarks inside the image; at most 50 % of the frames rejected)"},
        "rom": {"type": "array", "items": {"type": "object", "additionalProperties": false, "required": ["start_s", "end_s", "channels"], "properties": {
                    "start_s": {"type": "number", "minimum": 0}, "end_s": {"type": "number", "minimum": 0},
                    "channels": {"type": "array", "minItems": 1, "items": {"type": "string", "minLength": 1}}}},
                "description": "range-of-motion calibration: windows where the performer, head still, takes the named source channels to their extreme; each named channel's gain = 1 / its peak there (clamped), the other channels' leak from it is measured and removed, and the head's expression bias is fitted over the windows"},
        "rom_gain_max": {"type": "number", "minimum": 1, "description": "clamp of a ROM gain to [1 / rom_gain_max, rom_gain_max] (default 16)"},
        "calibration_profile": {"type": "object", "description": "a calibration_profile from an earlier run's report (gains, leaks, head_bias), applied where this run has no ROM"},
        "couplings": {"type": "object", "additionalProperties": false, "properties": {
            "look_lid": {"description": "false (the default), or {lo, hi}: eyeLook* scaled down as that side's eyeBlink rises from lo to hi", "anyOf": [{"type": "boolean"}, {"type": "object"}]},
            "squint_smile": {"type": "number", "minimum": 0, "description": "eyeSquint minus this x mouthSmile of the same side (default 0.6; 0 turns it off)"}},
            "description": "crosstalk couplings applied after calibration"},
        "head_reference": {"type": "string", "enum": ["camera", "face"], "description": "the neutral rotation is removed in the camera frame (q0^-1 q) or the face frame (q q0^-1)"},
        "head_bias": {"description": "false, or {channel: [x, y, z]} rotation-vector radians per unit weight removed from the head (expression-induced head motion); default: fitted over the ROM windows, else the profile's", "anyOf": [{"type": "boolean"}, {"type": "object"}]},
        "eyes": {"type": "string", "minLength": 1, "description": "the character's eyes JSON: with glb and lid_guard.enabled, every frame's evaluated mix is kept out of the eyes (lid guard)"},
        "lid_guard": {"type": "object", "additionalProperties": false, "properties": {"enabled": {"type": "boolean"}, "front_deg": {"type": "number", "minimum": 0, "maximum": 90}},
                      "description": "lid guard options (opt-in: enabled true with eyes; front_deg 75)"},
        "sides": {"type": "string", "enum": ["as_is", "swap"], "description": "swap every Left/Right channel pair (default as_is)"},
        "mirror": {"type": "boolean", "description": "the input is mirrored (a selfie view): swaps the sides and mirrors the head rotation"},
        "head": {"type": "boolean", "description": "write the head rotation (default true when the source has one)"},
        "neck_share": {"type": "number", "minimum": 0, "maximum": 1, "description": "fraction of the head rotation given to the neck joint (default 0)"},
        "joint_tracks": {"type": "boolean", "description": "with a face-rigged glb, also write <name>_joints: joint tracks blended from each shape's fitted pose by its weight (approximate)"},
        "return_clip": {"type": "boolean", "description": "include the clip in the result"},
        "plant": {"type": "object", "additionalProperties": false, "properties": {"noise": {"type": "number", "minimum": 0}, "seed": {"type": "number", "minimum": 1}},
                  "description": "a defect on purpose: Gaussian noise of this standard deviation added to the calibrated weights before smoothing"}
    });
    v.as_object().cloned().unwrap_or_default()
}

pub fn video_input() -> Value {
    let mut p = common_props();
    for (k, v) in [
        ("video", json!({"type": "string", "minLength": 1, "description": "a video file (read with OpenCV)"})),
        ("frames", json!({"type": "string", "minLength": 1, "description": "a folder of images, in name order"})),
        ("fps", json!({"type": "number", "minimum": 0.001, "description": "frame rate: needed for frames, overrides the video's own"})),
        ("take", json!({"type": "string", "minLength": 1, "description": "a take.json from an earlier run (skips the landmarker)"})),
        ("max_faces", json!({"type": "integer", "minimum": 1, "maximum": 8, "description": "faces the landmarker looks for (default 2, so a second face is seen and the frame rejected)"})),
        ("min_confidence", json!({"type": "number", "minimum": 0, "maximum": 1, "description": "the landmarker's detection, presence and tracking confidence (default 0.5); a face under it is not found"})),
        ("max_frames", json!({"type": "integer", "minimum": 1})),
        ("landmarks", json!({"type": "boolean", "description": "also write every frame's 478 landmarks in pixels to <out>.track/landmarks.npy"})),
        ("python", json!({"type": "string", "description": "Python with numpy, opencv and mediapipe (else GAZEFORGE_PYTHON, else python)"})),
        ("workers", json!({"type": "string", "description": "directory of the worker scripts (else GAZEFORGE_WORKERS, else ./workers)"})),
        ("landmarker", json!({"type": "string", "description": "MediaPipe face landmarker .task model with the blendshape head (else GAZEFORGE_FACE_LANDMARKER)"})),
    ] {
        p.insert(k.to_string(), v);
    }
    obj(Value::Object(p), &["out"])
}

pub fn csv_input() -> Value {
    let mut p = common_props();
    for (k, v) in [
        ("csv", json!({"type": "string", "minLength": 1, "description": "a CSV whose header names ARKit blendshapes (PascalCase or camelCase); Timecode (HH:MM:SS:FF) and HeadYaw/HeadPitch/HeadRoll are optional"})),
        ("fps", json!({"type": "number", "minimum": 0.001, "description": "frame rate of the rows and of the timecode frames (default 60)"})),
        ("head_units", json!({"type": "string", "enum": ["ignore", "rad", "deg"], "description": "units of HeadYaw/HeadPitch/HeadRoll (default ignore: their units are unconfirmed)"})),
        ("head_signs", json!({"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "multipliers for yaw, pitch, roll (default 1, 1, 1)"})),
    ] {
        p.insert(k.to_string(), v);
    }
    obj(Value::Object(p), &["csv", "out"])
}

pub fn output() -> Value {
    shape(json!({"frames": {"type": "integer"}, "fps": {"type": "number"}, "gate": {"type": "object"}, "rejected": {"type": "array"}, "channels": {"type": "array"},
                 "head": {"type": "object"}, "outputs": {"type": "object"}, "clip": {"type": "object"}}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic(n: usize, fps: f64) -> Take {
        let names = vec!["_neutral".to_string(), "jawOpen".to_string(), "eyeBlinkLeft".to_string()];
        let times: Vec<f64> = (0..n).map(|i| i as f64 / fps).collect();
        let scores = times.iter().map(|t| Some(vec![0.5, 0.1 + 0.8 * (t * 2.0).sin().max(0.0), 0.05])).collect();
        Take { fps, names, times, scores, rotations: vec![Some(IDENTITY); n], faces: vec![1; n], face_px: vec![Some(300.0); n], in_frame: vec![Some(1.0); n],
               source: json!({"kind": "test"}), ignored: vec![] }
    }

    #[test]
    fn one_euro_holds_a_constant_and_passes_a_step_with_beta() {
        let t: Vec<f64> = (0..60).map(|i| i as f64 / 30.0).collect();
        let c = one_euro(&t, &[0.3; 60], Euro { min_cutoff: 1.0, beta: 0.0, d_cutoff: 1.0 });
        assert!(c.iter().all(|x| (x - 0.3).abs() < 1e-12));
        let step: Vec<f64> = (0..60).map(|i| if i < 30 { 0.0 } else { 1.0 }).collect();
        let slow = one_euro(&t, &step, Euro { min_cutoff: 1.0, beta: 0.0, d_cutoff: 1.0 });
        let fast = one_euro(&t, &step, Euro { min_cutoff: 1.0, beta: 5.0, d_cutoff: 1.0 });
        assert!(fast[31] > slow[31] + 0.2, "{} {}", fast[31], slow[31]);
        let a = alpha(1.0, 1.0 / 30.0);
        assert!((slow[30] - a).abs() < 1e-12);
    }

    #[test]
    fn calibration_subtracts_the_neutral_and_rescales() {
        let take = synthetic(90, 30.0);
        let p = process(&take, &json!({"smoothing": {"enabled": false}, "neutral_frames": 1}), None).unwrap();
        assert_eq!(p.names, vec!["jawOpen", "eyeBlinkLeft"]);
        assert!(p.weights[0].iter().all(|w| w.abs() < 1e-12));
        let peak = p.weights.iter().map(|r| r[0]).fold(0.0, f64::max);
        assert!((peak - 0.8 / 0.9).abs() < 1e-3, "{peak}");
        assert!(p.weights.iter().all(|r| r[1].abs() < 1e-12));
    }

    #[test]
    fn rejected_frames_are_filled_and_listed() {
        let mut take = synthetic(30, 30.0);
        take.faces[10] = 0;
        take.scores[10] = None;
        take.faces[11] = 2;
        take.rotations[12] = Some(from_euler(60.0, 0.0, 0.0));
        let p = process(&take, &json!({"smoothing": {"enabled": false}, "calibrate": false}), None).unwrap();
        let rej = p.report["rejected"].as_array().unwrap();
        assert_eq!(rej.iter().map(|r| r["frame"].as_u64().unwrap()).collect::<Vec<_>>(), vec![10, 11, 12]);
        assert!(rej[2]["reasons"][0].as_str().unwrap().starts_with("yaw"));
        let (a, b) = (take.scores[9].as_ref().unwrap()[1], take.scores[13].as_ref().unwrap()[1]);
        assert!((p.weights[11][0] - (a + 0.5 * (b - a))).abs() < 1e-9);
        let all_bad = process(&Take { faces: vec![0; 30], scores: vec![None; 30], ..synthetic(30, 30.0) }, &json!({}), None);
        assert_eq!(all_bad.err().unwrap().code, Code::GateFailed);
    }

    #[test]
    fn euler_round_trips_and_the_head_is_relative_to_the_neutral() {
        for (y, p, r) in [(20.0, 0.0, 0.0), (0.0, -15.0, 0.0), (0.0, 0.0, 10.0), (12.0, 8.0, -5.0)] {
            let a = angles(from_euler(y, p, r));
            assert!((a[0] - y).abs() < 1e-9 && (a[1] - p).abs() < 1e-9 && (a[2] - r).abs() < 1e-9, "{a:?}");
        }
        let mut take = synthetic(20, 30.0);
        let base = from_euler(10.0, 0.0, 0.0);
        take.rotations = (0..20).map(|i| Some(qmul(base, from_euler(i as f64, 0.0, 0.0)))).collect();
        let p = process(&take, &json!({"smoothing": {"enabled": false}, "neutral_frames": 1}), None).unwrap();
        let a = angles(p.head.unwrap()[19]);
        assert!((a[0] - 19.0).abs() < 1e-6, "{a:?}");
    }

    #[test]
    fn matrix_to_quaternion_matches_a_known_turn() {
        let q = from_euler(30.0, 0.0, 0.0);
        let x = rotate(q, [1.0, 0.0, 0.0]);
        let y = rotate(q, [0.0, 1.0, 0.0]);
        let z = rotate(q, [0.0, 0.0, 1.0]);
        let m = [x[0], y[0], z[0], 0.0, x[1], y[1], z[1], 0.0, x[2], y[2], z[2], 0.0, 0.0, 0.0, 0.0, 1.0];
        let back = quat_from_matrix(&m).unwrap();
        assert!(qdot(back, q).abs() > 1.0 - 1e-12);
    }

    #[test]
    fn csv_is_read_by_header_names() {
        let csv = "Timecode,BlendShapeCount,EyeBlinkLeft,JawOpen,TongueOut,HeadYaw,HeadPitch,HeadRoll\n\
                   00:00:01:00.000,61,0.1,0.2,0.9,0,0,0\n00:00:01:01.000,61,0.5,0.4,0.9,0.1,0,0\n00:00:01:02.000,61,x,0.4,0.9,0,0,0\n";
        let take = parse_csv(csv, 60.0, "rad", [1.0; 3]).unwrap();
        assert_eq!(take.names, vec!["eyeBlinkLeft", "jawOpen", "tongueOut"]);
        assert_eq!(take.ignored, vec!["BlendShapeCount"]);
        assert_eq!(take.faces, vec![1, 1, 0]);
        assert!((take.times[1] - 1.0 / 60.0).abs() < 1e-12);
        assert!((angles(take.rotations[1].unwrap())[0] - 0.1f64.to_degrees()).abs() < 1e-9);
        let p = process(&take, &json!({"smoothing": {"enabled": false}, "neutral_frames": 1, "morphs": ["faceunits/jawOpen", "faceunits/eyeBlinkLeft"]}), None).unwrap();
        assert_eq!(p.report["retarget"]["dropped_from_source"], json!(["tongueOut"]));
        assert_eq!(p.report["rejected"][0]["frame"], 2);
    }

    #[test]
    fn swapping_sides_renames_only_paired_channels() {
        assert_eq!(swap_side("eyeBlinkLeft"), "eyeBlinkRight");
        assert_eq!(swap_side("jawRight"), "jawLeft");
        assert_eq!(swap_side("jawOpen"), "jawOpen");
    }

    #[test]
    fn a_planted_glb_animation_round_trips() {
        let lod = gf_io::gltf::Lod {
            name: "Head_LOD0".into(),
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uvs: vec![[0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            morphs: vec![
                gf_io::gltf::MorphData { positions: vec![[0.0, 0.0, 0.1]; 3], normals: vec![[0.0; 3]; 3] },
                gf_io::gltf::MorphData { positions: vec![[0.0, 0.1, 0.0]; 3], normals: vec![[0.0; 3]; 3] },
            ],
            skin: None,
            material: None,
        };
        let (bytes, _) = gf_io::gltf::glb(&[lod], &["jawOpen".into(), "eyeBlinkLeft".into()], gf_io::gltf::Sparse::Never, json!({}), "test").unwrap();
        let take = synthetic(12, 30.0);
        let (doc, _) = gf_io::gltf::chunks(&bytes).unwrap();
        let p = process(&take, &json!({"smoothing": {"enabled": false}}), Some(&doc)).unwrap();
        let ch = weight_channels(&doc, &p);
        let out = ag::append(&bytes, &[Clip { name: "capture".into(), times: p.times.iter().map(|t| *t as f32).collect(), channels: ch, extras: json!({}) }]).unwrap();
        let (d2, bin2) = gf_io::gltf::chunks(&out).unwrap();
        let anim = &d2["animations"][0];
        assert_eq!(anim["channels"][0]["target"]["path"], "weights");
        let values = gf_io::gltf::read_plain(&d2, bin2, anim["samplers"][0]["output"].as_u64().unwrap()).unwrap();
        assert_eq!(values.len(), 12 * 2);
        assert!((values[2 * 5] as f64 - p.weights[5][0]).abs() < 1e-6);
        assert!(ag::append(&out, &[Clip { name: "capture".into(), times: vec![0.0], channels: vec![], extras: json!({}) }]).is_err());
        let (reread, meshes) = gf_io::gltf::read_glb(&out).unwrap();
        assert_eq!(meshes[0].morph_names.len(), 2);
        assert_eq!(reread["accessors"][anim["samplers"][0]["input"].as_u64().unwrap() as usize]["count"], 12);
    }

    #[test]
    fn a_rigged_glb_gets_head_rotation_and_blended_joint_tracks() {
        use gf_io::gltf as g;
        let lod = g::Lod {
            name: "Head_LOD0".into(),
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uvs: vec![[0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            morphs: vec![g::MorphData { positions: vec![[0.0, 0.0, 0.1]; 3], normals: vec![[0.0; 3]; 3] }],
            skin: Some(g::SkinData { joints: vec![[1, 0, 0, 0]; 3], weights: vec![[1.0, 0.0, 0.0, 0.0]; 3], joints_1: Vec::new(), weights_1: Vec::new() }),
            material: None,
        };
        let pose = [0.0, 0.0, (0.2f32).sin(), (0.2f32).cos()];
        let rig = g::Rig {
            names: vec!["neck".into(), "head".into(), "jaw".into()],
            parents: vec![None, Some(0), Some(1)],
            global: vec![[0.0, 0.0, 0.0], [0.0, 0.1, 0.0], [0.0, 0.05, 0.05]],
            attachments: vec![],
            animations: vec![g::Animation { name: "jawOpen".into(), channels: vec![g::Channel { joint: 2, rotation: [[0.0, 0.0, 0.0, 1.0], pose], translation: [[0.0, -0.05, 0.05], [0.0, -0.05, 0.06]] }], morph: None }],
        };
        let (bytes, _) = g::glb_rigged(&[lod], &["jawOpen".into()], g::Sparse::Never, json!({}), "test", Some(&rig)).unwrap();
        let (doc, bin) = g::chunks(&bytes).unwrap();
        let mut take = synthetic(10, 30.0);
        take.rotations = (0..10).map(|i| Some(from_euler(2.0 * i as f64, 0.0, 0.0))).collect();
        let p = process(&take, &json!({"smoothing": {"enabled": false}, "neutral_frames": 1}), Some(&doc)).unwrap();
        let (hc, used) = head_channels(&doc, &p, 0.5, None);
        assert_eq!(used, vec!["head", "neck"]);
        let compose = |f: usize| {
            let h = &hc[0].values[4 * f..4 * f + 4];
            let n = &hc[1].values[4 * f..4 * f + 4];
            qmul([n[0] as f64, n[1] as f64, n[2] as f64, n[3] as f64], [h[0] as f64, h[1] as f64, h[2] as f64, h[3] as f64])
        };
        assert!((angles(compose(9))[0] - 18.0).abs() < 1e-3);
        let (clip, note) = joint_clip(&doc, bin, &p, "capture", 0.0).unwrap().unwrap();
        assert_eq!(note["approximate"], true);
        let jaw = ag::node_index(&doc, "jaw").unwrap();
        let rot = clip.channels.iter().find(|c| c.node == jaw && c.path == "rotation").unwrap();
        let w = p.weights[5][0];
        let expect = qnorm([0.0, 0.0, w * pose[2] as f64, 1.0 + w * (pose[3] as f64 - 1.0)]);
        for k in 0..4 {
            assert!((rot.values[20 + k] as f64 - expect[k]).abs() < 1e-6);
        }
        let tr = clip.channels.iter().find(|c| c.node == jaw && c.path == "translation").unwrap();
        assert!((tr.values[3 * 5 + 2] as f64 - (0.05 + 0.01 * w)).abs() < 1e-6);
    }

    fn take_of(names: &[&str], scores: Vec<Vec<f64>>, rotations: Vec<Option<Quat>>) -> Take {
        let n = scores.len();
        Take { fps: 30.0, names: names.iter().map(|s| s.to_string()).collect(), times: (0..n).map(|i| i as f64 / 30.0).collect(), scores: scores.into_iter().map(Some).collect(),
               rotations, faces: vec![1; n], face_px: vec![Some(300.0); n], in_frame: vec![Some(1.0); n], source: json!({}), ignored: vec![] }
    }

    #[test]
    fn rom_gain_rescales_and_the_measured_leak_is_removed() {
        let smile = |i: usize| 0.5 * ((i as f64 / 30.0 - 0.5) * 3.0).sin().max(0.0);
        let take = take_of(&["mouthSmileLeft", "eyeSquintLeft"], (0..60).map(|i| vec![smile(i), 0.3 * smile(i)]).collect(), vec![Some(IDENTITY); 60]);
        let p = process(&take, &json!({"smoothing": {"enabled": false}, "neutral_frames": 5, "rom": [{"start_s": 0.0, "end_s": 2.0, "channels": ["mouthSmileLeft"]}]}), None).unwrap();
        let peak = p.weights.iter().map(|r| r[0]).fold(0.0, f64::max);
        assert!((peak - 1.0).abs() < 1e-9, "{peak}");
        assert!(p.weights.iter().all(|r| r[1] < 1e-9));
        assert!((p.report["calibration_profile"]["gains"]["mouthSmileLeft"].as_f64().unwrap() - 2.0).abs() < 1e-3);
        let again = process(&take, &json!({"smoothing": {"enabled": false}, "neutral_frames": 5, "calibration_profile": p.report["calibration_profile"]}), None).unwrap();
        assert!(again.weights.iter().zip(&p.weights).all(|(a, b)| (a[0] - b[0]).abs() < 1e-4 && a[1] < 1e-4));
    }

    #[test]
    fn eye_look_fades_as_the_lid_closes_and_the_rom_fits_the_head_bias() {
        let jaw = |i: usize| if (20..40).contains(&i) { 0.5 } else { 0.0 };
        let scores = (0..60).map(|i| vec![if i >= 50 { 0.6 } else { 0.0 }, if i >= 50 { 0.7 } else if i >= 45 { 0.2 } else { 0.0 }, jaw(i)]).collect();
        let take = take_of(&["eyeBlinkLeft", "eyeLookDownLeft", "jawOpen"], scores, (0..60).map(|i| Some(qexp([0.05 * jaw(i), 0.0, 0.0]))).collect());
        let input = json!({"smoothing": {"enabled": false}, "neutral_frames": 5, "couplings": {"look_lid": {"lo": 0.2, "hi": 0.5}},
                           "rom": [{"start_s": 0.6, "end_s": 1.4, "channels": ["jawOpen"]}]});
        let p = process(&take, &input, None).unwrap();
        assert!((p.weights[47][1] - 0.2).abs() < 1e-9);
        assert!(p.weights[55][1].abs() < 1e-12);
        assert!((p.weights[30][2] - 1.0).abs() < 1e-9);
        let worst = p.head.as_ref().unwrap().iter().map(|q| angles(*q).iter().fold(0.0f64, |m, a| m.max(a.abs()))).fold(0.0, f64::max);
        assert!(worst < 0.05, "{worst}");
        let raw = process(&take, &json!({"smoothing": {"enabled": false}, "neutral_frames": 5, "head_bias": false}), None).unwrap();
        assert!(raw.head.unwrap().iter().any(|q| angles(*q)[1].abs() > 1.0));
    }

    #[test]
    fn the_lid_guard_scales_the_secondary_channel_until_the_mix_clears_the_eye() {
        use gf_io::gltf as g;
        let lod = g::Lod {
            name: "Head_LOD0".into(),
            positions: vec![[0.0, 0.0, 0.012], [0.001, 0.0, 0.012], [0.0, 0.001, 0.012]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uvs: vec![[0.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            morphs: (0..2).map(|_| g::MorphData { positions: vec![[0.0, 0.0, -0.0015], [0.0; 3], [0.0; 3]], normals: vec![[0.0; 3]; 3] }).collect(),
            skin: None,
            material: None,
        };
        let names = vec!["a".to_string(), "b".to_string()];
        let (bytes, _) = g::glb(&[lod], &names, g::Sparse::Never, json!({}), "test").unwrap();
        let eye = json!({"eyes": [{"centre": [0.0, 0.0, 0.0], "radius": 0.01, "gaze": [0.0, -1.0, 0.0],
                                   "vertices": [[0.01, 0.0, 0.0], [-0.01, 0.0, 0.0], [0.0, 0.01, 0.0], [0.0, -0.01, 0.0], [0.0, 0.0, 0.01], [0.0, 0.0, -0.01]],
                                   "triangles": [[0, 2, 4], [2, 1, 4], [1, 3, 4], [3, 0, 4], [2, 0, 5], [1, 2, 5], [3, 1, 5], [0, 3, 5]]}]});
        let eyes = gf_geom::lids::eyes_from_json(&eye.to_string()).unwrap();
        let guard = crate::anim_guard::Guard::new(&bytes, eyes, None, &names).unwrap();
        let mut w = vec![vec![1.0, 0.0], vec![1.0, 1.0], vec![0.4, 1.0]];
        let r = guard.apply(&mut w);
        assert_eq!(r["frames_failing_before"], 2);
        assert!(r["frames_still_failing"].as_array().unwrap().is_empty());
        assert_eq!(w[0], vec![1.0, 0.0]);
        assert_eq!(w[1][0], 1.0);
        assert!(w[1][1] > 0.25 && w[1][1] < 1.0 / 3.0, "{:?}", w[1]);
        assert!((w[2][1] - 1.0).abs() < 1e-12 && w[2][0] < 1.0 / 3.0, "{:?}", w[2]);
    }
}
