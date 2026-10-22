use std::path::Path;

use gf_core::game_mesh::{Point, game_normals};
use gf_core::skin_regions;
use serde_json::{Value, json};

pub use gf_core::groom::{Direction, FORMAT, Fringe, Groom, Part, Tie, parse};

use super::strands::Strand;
use super::wearable::Surface;

const SCALP: usize = 11;
const ROOT_LIFT_M: f64 = 0.001;
const CLEAR_M: f64 = 0.003;
const NEAR_M: f64 = 0.05;
const HUG_M: f64 = 0.02;
const STEER: f64 = 0.5;
const FLYAWAY_M: f64 = 0.04;
const FALL: f64 = 1.0;
const JAW_MARGIN_M: f64 = -0.015;
const THROAT_M: f64 = 0.05;
const OUTSIDE_PASSES: usize = 4;
const SETTLE_ROUNDS: usize = 3;
const POINTS_PER_TURN: usize = 8;
const MAX_POINTS: usize = 256;
const TIE_HIGH_BELOW_CROWN_M: f64 = 0.05;
const TIE_OUT_M: f64 = 0.012;
const TIE_REACH_M: f64 = 0.015;
const TAIL_RADIUS_M: f64 = 0.02;
const TAIL_SPREAD: f64 = 0.12;
const MAX_SEGMENT_M: f64 = 0.015;
const FADE_LINE_ABOVE_EARS_M: f64 = 0.035;
const TAPER: f64 = 0.65;
const TAPER_SPAN_M: f64 = 0.08;
const QUIFF_RISE_M: f64 = 0.05;
const QUIFF_SPAN_M: f64 = 0.07;
const QUIFF_BACK_SHARE: f64 = 0.3;
const QUIFF_FORWARD: f64 = 0.25;
const QUIFF_TURN_M: f64 = 0.03;
const QUIFF_BACK: Point = [0.0, 1.0, -0.35];
const QUIFF_PROFILE_HALF_M: f64 = 0.015;
const DRAPE_PULL: f64 = 1.5;
const QUIFF_PROFILE_BAND_M: f64 = 0.02;
const UP_FRONT_M: f64 = 0.03;
const TIE_UP_BIAS: f64 = 0.3;
const LIFT: f64 = 0.35;
const PART_OFFSET_M: f64 = 0.035;
const HAIRLINE_SOFT_M: f64 = 0.006;
const HAIRLINE_EARS_M: f64 = 0.02;
const TEMPLE_DROP_M: f64 = 0.01;
const MAX_ROOT_TRIES: usize = 10_000;
const HAIRLINE_CENTRE_M: f64 = 0.02;
const HAIRLINE_TEMPLE_SPAN: f64 = 0.5;
const FRINGE_BAND_M: f64 = 0.04;
const FRINGE_ABOVE_BROWS_M: f64 = 0.008;
const CURTAIN_SWEEP: f64 = 0.9;
const TIE_CLEAR_OF_M: f64 = 0.04;
const GATHER_SNUG_M: f64 = 0.004;
const FRAME_EARS_M: f64 = 0.03;
const EYES_BAND_M: f64 = 0.015;
const FRAME_DOWN: f64 = 0.35;
const FOREHEAD_CLEAR_M: f64 = 0.01;
const FRINGE_WIDTH_WITH_FRAME: f64 = 0.6;
const HIGHLIGHT_AT_ROOT: f32 = 0.15;
const HIGHLIGHT_FULL_AT: f32 = 0.3;
const FRAME_DEPTH_M: f64 = 0.05;
const CURL_FADE_M: f64 = 0.03;
const FRONT_BACK: f64 = 1.5;
const FACE_MARGIN_M: f64 = 0.01;
const FRINGE_M: f64 = 0.02;

pub fn read(path: &Path) -> Result<Groom, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?)
}

pub(crate) struct Face {
    centre_x: f64,
    half_width: f64,
    bottom: f64,
    chin: f64,
    top: f64,
    eyes_y: f64,
    ears_y: f64,
}

impl Face {
    pub(crate) fn on(landmarks: &std::collections::BTreeMap<usize, Point>) -> Result<Face, String> {
        let lm = |i: usize| landmarks.get(&i).copied().ok_or_else(|| format!("landmark {i} is missing"));
        let (eye_r, eye_l, chin, brow, ear_r, ear_l) = (lm(33)?, lm(263)?, lm(152)?, lm(10)?, lm(234)?, lm(454)?);
        Ok(Face {
            centre_x: (eye_r[0] + eye_l[0]) / 2.0,
            half_width: (eye_l[0] - eye_r[0]).abs() / 2.0 + FACE_MARGIN_M,
            bottom: chin[2] - THROAT_M,
            chin: chin[2],
            top: brow[2] - FRINGE_M,
            eyes_y: (eye_r[1] + eye_l[1]) / 2.0,
            ears_y: (ear_r[1] + ear_l[1]) / 2.0,
        })
    }

    fn under_jaw(&self, p: Point) -> bool {
        p[2] < self.chin && p[1] < self.ears_y && (p[0] - self.centre_x).abs() < self.half_width + JAW_MARGIN_M
    }

    pub(crate) fn keep_out(&self, p: Point) -> Option<Point> {
        self.keep_out_toward(p, p[0] - self.centre_x)
    }

    fn side_of(&self, p: Point) -> f64 {
        if p[0] >= self.centre_x { 1.0 } else { -1.0 }
    }

    fn keep_out_toward(&self, p: Point, side: f64) -> Option<Point> {
        let dx = (p[0] - self.centre_x).abs();
        let ahead = (self.eyes_y - p[1]).max(0.0);
        if dx >= self.half_width + ahead || p[2] <= self.bottom || p[2] >= self.top || p[1] >= self.ears_y {
            return None;
        }
        let (dx, y) = if ahead > 0.0 {
            let d = (self.half_width + ahead - dx) / 2.0;
            if p[1] + d < self.eyes_y { (dx + d, p[1] + d) } else { (self.half_width, self.eyes_y) }
        } else {
            (self.half_width, p[1])
        };
        Some([self.centre_x + if side >= 0.0 { dx } else { -dx }, y, p[2]])
    }
}

fn cut_below(line: &[Point], z: f64, points: usize) -> Vec<Point> {
    let Some(k) = (1..line.len()).find(|&k| line[k][2] < z) else { return line.to_vec() };
    let (a, b) = (line[k - 1], line[k]);
    let t = if a[2] > b[2] { ((a[2] - z) / (a[2] - b[2])).clamp(0.0, 1.0) } else { 0.0 };
    let mut kept = line[..k].to_vec();
    kept.push(add(a, scale(sub(b, a), t)));
    if kept.len() < 2 {
        return line.to_vec();
    }
    let arcs: Vec<f64> = std::iter::once(0.0)
        .chain(kept.windows(2).scan(0.0, |s, w| {
            *s += dot(sub(w[1], w[0]), sub(w[1], w[0])).sqrt();
            Some(*s)
        }))
        .collect();
    let total = *arcs.last().unwrap();
    (0..points)
        .map(|i| {
            let want = total * i as f64 / (points - 1) as f64;
            let j = arcs.partition_point(|&s| s < want).clamp(1, kept.len() - 1);
            let span = arcs[j] - arcs[j - 1];
            let f = if span > 0.0 { (want - arcs[j - 1]) / span } else { 0.0 };
            add(kept[j - 1], scale(sub(kept[j], kept[j - 1]), f.clamp(0.0, 1.0)))
        })
        .collect()
}

fn smoothstep01(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

pub(crate) struct Hairline {
    z: f64,
    brows: f64,
    ears_z: f64,
    centre_x: f64,
    half_width: f64,
    eyes_y: f64,
    ears_y: f64,
}

impl Hairline {
    pub(crate) fn on(landmarks: &std::collections::BTreeMap<usize, Point>, face: &Face, forehead: f64) -> Result<Hairline, String> {
        let z = |ids: &[usize]| -> Result<f64, String> {
            ids.iter().map(|i| landmarks.get(i).map(|p| p[2]).ok_or_else(|| format!("landmark {i} is missing"))).sum::<Result<f64, String>>().map(|s| s / ids.len() as f64)
        };
        let brows = z(&[105, 334])?;
        let middle = brows - z(&[2])?;
        Ok(Hairline { z: brows + forehead * middle, brows, ears_z: z(&[234, 454])?, centre_x: face.centre_x, half_width: face.half_width, eyes_y: face.eyes_y, ears_y: face.ears_y })
    }

    pub(crate) fn low(&self, p: Point) -> f64 {
        let a = (p[0] - self.centre_x).abs() / self.half_width;
        let temple = self.z - TEMPLE_DROP_M * (a * a).min(1.0);
        let toward_ears = smoothstep01((p[1] - self.eyes_y) / (self.ears_y - self.eyes_y)) * smoothstep01((a - 1.0) / HAIRLINE_TEMPLE_SPAN);
        temple + (self.ears_z - temple) * toward_ears
    }

    fn behind_face_band(&self, p: Point) -> f64 {
        let ahead = 1.0 - smoothstep01((p[1] - (self.ears_y - FRAME_EARS_M)) / HAIRLINE_EARS_M);
        smoothstep01((p[2] - self.low(p)) / FRAME_DEPTH_M).max(1.0 - ahead)
    }

    pub(crate) fn below(&self, p: Point) -> bool {
        self.keep(p) <= 0.0
    }

    fn off_forehead(&self, p: Point) -> Option<Point> {
        let inner = self.half_width - FACE_MARGIN_M;
        let w = 1.0 - smoothstep01(((p[0] - self.centre_x).abs() - inner) / FACE_MARGIN_M);
        let floor = self.low(p) - HAIRLINE_SOFT_M;
        if w <= 0.0 || p[1] >= self.eyes_y || p[2] <= self.brows || p[2] >= floor - 1e-6 {
            return None;
        }
        Some([p[0], p[1], p[2] + (floor - p[2]) * w])
    }

    pub(crate) fn depth_below(&self, p: Point) -> f64 {
        (self.low(p) - HAIRLINE_SOFT_M - p[2]).max(0.0)
    }

    pub(crate) fn keep(&self, p: Point) -> f64 {
        let low = self.low(p);
        let behind_ears = smoothstep01((p[1] - self.ears_y) / HAIRLINE_EARS_M);
        let ramp = smoothstep01((p[2] - (low - HAIRLINE_SOFT_M)) / (2.0 * HAIRLINE_SOFT_M));
        1.0 - (1.0 - behind_ears) * (1.0 - ramp)
    }
}

fn outside(surface: &Surface, p: Point) -> Option<Point> {
    let mut q = p;
    for _ in 0..OUTSIDE_PASSES {
        let Some(j) = surface.nearest(q, NEAR_M) else { break };
        let s = surface.signed(q, j);
        if s >= CLEAR_M - 1e-9 {
            break;
        }
        q = add(q, scale(surface.normals[j], CLEAR_M - s));
    }
    (q != p).then_some(q)
}

fn strand_points(g: &Groom) -> usize {
    let curls = g.length_m * g.curl_turns_per_m * if g.curl_radius_m > 0.0 { 1.0 } else { 0.0 };
    let curled = (curls * POINTS_PER_TURN as f64).ceil() as usize + 1;
    let fine = (g.length_m / MAX_SEGMENT_M).ceil() as usize + 1;
    g.points.max(curled.max(fine).min(MAX_POINTS))
}

pub(crate) fn highlight_lift(t: f32) -> f32 {
    HIGHLIGHT_AT_ROOT + (1.0 - HIGHLIGHT_AT_ROOT) * (t / HIGHLIGHT_FULL_AT).min(1.0)
}

fn lock_unit(seed: u64, lock: usize) -> f64 {
    Rng(seed ^ (lock as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93)).next()
}

pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub(crate) fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(crate) fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn scale(a: Point, s: f64) -> Point {
    a.map(|x| x * s)
}

pub(crate) fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn cross(a: Point, b: Point) -> Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub(crate) fn unit(a: Point) -> Point {
    let l = dot(a, a).sqrt();
    if l < 1e-12 { a } else { scale(a, 1.0 / l) }
}

fn tangent(v: Point, n: Point) -> Point {
    sub(v, scale(n, dot(v, n)))
}

pub fn grow(g: &Groom, head: &[Point], triangles: &[[u32; 3]], dense: &[[f64; 3]], surface: &Surface) -> Result<(Vec<Strand>, Value), String> {
    let landmarks = gf_core::base_landmarks::on(dense)?;
    let face = Face::on(&landmarks)?;
    let hairline = Hairline::on(&landmarks, &face, g.forehead)?;
    let brim = [234usize, 454].iter().map(|i| landmarks.get(i).map_or(0.0, |p| p[2])).sum::<f64>() / 2.0;
    let mut hugged = 0usize;
    let mut kept_off_face = 0usize;
    let mut off_forehead = 0usize;
    let normals = game_normals(head, triangles);
    let regions = skin_regions::build(head, &normals, triangles, &landmarks)?;
    let scalp: Vec<f64> = regions.weights.iter().map(|w| w[SCALP] as f64).collect();
    let mut weights = Vec::with_capacity(triangles.len());
    for t in triangles {
        let [a, b, c] = t.map(|v| head[v as usize]);
        let area = dot(cross(sub(b, a), sub(c, a)), cross(sub(b, a), sub(c, a))).sqrt() / 2.0;
        let s = t.iter().map(|&v| scalp[v as usize]).sum::<f64>() / 3.0;
        weights.push(if s >= 0.5 { area } else { 0.0 });
    }
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return Err("this head has no scalp to grow a groom on".into());
    }
    let mut cumulative = Vec::with_capacity(weights.len());
    let mut acc = 0.0;
    for w in &weights {
        acc += w / total;
        cumulative.push(acc);
    }
    let crown = (0..head.len()).filter(|&v| scalp[v] >= 0.5).map(|v| head[v]).max_by(|a, b| a[2].total_cmp(&b[2])).ok_or("no scalp vertex")?;
    let part_x = match g.part {
        Part::Left => Some(crown[0] + PART_OFFSET_M),
        Part::Right => Some(crown[0] - PART_OFFSET_M),
        Part::Centre => Some(crown[0]),
        Part::None => None,
    };
    let down = [0.0, 0.0, -1.0];
    let tie_point = match g.tie {
        Tie::None => None,
        Tie::Low | Tie::High => {
            let z = if g.tie == Tie::High { crown[2] - TIE_HIGH_BELOW_CROWN_M } else { brim };
            let back = head.iter().filter(|p| (p[2] - z).abs() < 0.01 && (p[0] - crown[0]).abs() < 0.02).map(|p| p[1]).fold(f64::MIN, f64::max);
            if back == f64::MIN {
                return Err("no back of the head at the tie's height".into());
            }
            Some(([crown[0], back + TIE_OUT_M, z], if g.tie == Tie::High { 0.4 } else { 0.15 }))
        }
    };
    let mut tails: Vec<usize> = Vec::new();
    let mut fringe_of: Vec<bool> = Vec::with_capacity(g.strands);
    let mut gather_off = 0.0f64;
    let mut framed = 0usize;
    let mut draped = 0usize;
    let fringe_half_width = face.half_width * if g.frame_m > 0.0 { FRINGE_WIDTH_WITH_FRAME } else { 1.0 };
    let fade_line = brim + FADE_LINE_ABOVE_EARS_M;
    let topness = |z: f64| -> f64 {
        if g.sides_m <= 0.0 {
            return 1.0;
        }
        let x = ((z - (fade_line - g.fade_m / 2.0)) / g.fade_m).clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let top_flow = |direction: Direction, n: Point| -> Option<Point> {
        let want = match direction {
            Direction::Part => return None,
            Direction::Back => [0.0, 1.0, -0.3],
            Direction::Forward => [0.0, -1.0, -0.3],
            Direction::Up => [0.0, 0.3, 0.0],
        };
        let f = tangent(want, n);
        (dot(f, f) > 1e-10).then(|| unit(f))
    };
    let mut asked = 0.0;
    let mut top_strands = 0usize;
    let flow = |p: Point, n: Point| -> Point {
        let lateral = match part_x {
            Some(x) => [if p[0] >= x { 1.0 } else { -1.0 }, 0.0, 0.0],
            None => unit([p[0] - crown[0], p[1] - crown[1], 0.0]),
        };
        let behind = ((p[1] - crown[1]) / 0.05).clamp(0.0, 1.0);
        let ahead = ((crown[1] - p[1]) / 0.05).clamp(0.0, 1.0);
        let back = [0.0, behind + FRONT_BACK * ahead, 0.0];
        let f = tangent(add(add(scale(lateral, 1.0 - behind), back), scale(down, 0.6)), n);
        if dot(f, f) < 1e-10 { unit(tangent(down, n)) } else { unit(f) }
    };
    let mut rng = Rng(g.seed);
    let points = strand_points(g);
    let seg = g.length_m / (points - 1) as f64;
    let mut roots = Vec::with_capacity(g.strands);
    let mut lines: Vec<Vec<Point>> = Vec::with_capacity(g.strands);
    let mut pushes = 0usize;
    for _ in 0..g.strands {
        let mut tries = 0;
        let (p, n) = loop {
            tries += 1;
            if tries > MAX_ROOT_TRIES {
                return Err("no scalp above the hairline: lower the forehead".into());
            }
            let r = rng.next();
            let t = cumulative.partition_point(|&c| c < r).min(triangles.len() - 1);
            let (mut u, mut w) = (rng.next(), rng.next());
            if u + w > 1.0 {
                u = 1.0 - u;
                w = 1.0 - w;
            }
            let b = [1.0 - u - w, u, w];
            let tri = triangles[t];
            let p: Point = std::array::from_fn(|k| (0..3).map(|i| b[i] * head[tri[i] as usize][k]).sum());
            let n = unit(std::array::from_fn(|k| (0..3).map(|i| b[i] * normals[tri[i] as usize][k]).sum()));
            if rng.next() < hairline.keep(p) {
                break (p, n);
            }
        };
        roots.push((p, n));
        let mut line = vec![add(p, scale(n, ROOT_LIFT_M))];
        let in_fringe = g.fringe != Fringe::None && (p[0] - face.centre_x).abs() < fringe_half_width && p[1] < crown[1] && p[2] < hairline.z + FRINGE_BAND_M;
        fringe_of.push(in_fringe);
        if in_fringe {
            let side = face.side_of(p);
            let want = if g.fringe == Fringe::Curtain { [side * CURTAIN_SWEEP, -0.1, -1.0] } else { [0.0, -0.15, -1.0] };
            let comb = |m: Point| {
                let f = tangent(want, m);
                if dot(f, f) > 1e-10 { unit(f) } else { unit(tangent(down, m)) }
            };
            let seg = g.fringe_m / (points - 1) as f64;
            let mut dir = unit(add(scale(n, LIFT), scale(comb(n), 1.0 - LIFT)));
            for _ in 1..points {
                let last = *line.last().unwrap();
                let mut next = add(last, scale(dir, seg));
                if let Some(q) = outside(surface, next) {
                    next = q;
                    pushes += 1;
                }
                if let Some(q) = face.keep_out_toward(next, side) {
                    next = q;
                    kept_off_face += 1;
                }
                let step = sub(next, last);
                next = add(last, scale(unit(step), seg));
                dir = unit(add(unit(step), scale(down, g.droop * 0.35)));
                if next[2] > brim
                    && let Some(j) = surface.nearest(next, HUG_M)
                {
                    let m = surface.normals[j];
                    dir = unit(add(scale(dir, 1.0 - STEER), scale(comb(m), STEER)));
                    let out = dot(dir, m);
                    if out > 0.0 {
                        dir = unit(sub(dir, scale(m, out)));
                        hugged += 1;
                    }
                } else if next[2] > brim {
                    dir = unit(add(dir, scale(down, FALL)));
                }
                line.push(next);
            }
            if g.fringe == Fringe::Straight {
                line = cut_below(&line, hairline.brows + FRINGE_ABOVE_BROWS_M, points);
            }
            asked += line.windows(2).map(|w| dot(sub(w[1], w[0]), sub(w[1], w[0])).sqrt()).sum::<f64>();
            tails.push(points);
            lines.push(line);
            continue;
        }
        if let Some((tp, lean)) = tie_point {
            let mut tail: Option<Point> = None;
            let mut tied_at = points;
            let mut spread = [0.0; 3];
            let mut arc = 0.0;
            let mut tail_seg = seg;
            for k in 1..points {
                let last = *line.last().unwrap();
                let mut next = match tail {
                    None => {
                        let to = sub(tp, last);
                        let d = dot(to, to).sqrt();
                        if d < TIE_REACH_M {
                            let a = rng.next() * std::f64::consts::TAU;
                            let r = TAIL_RADIUS_M * rng.next().sqrt();
                            tail = Some(unit([0.0, lean, -1.0]));
                            tied_at = k;
                            spread = scale([a.cos(), a.sin(), 0.0], r * TAIL_SPREAD);
                            let origin = [tp[0] + r * a.cos(), tp[1] + r * a.sin(), tp[2]];
                            arc += dot(sub(origin, last), sub(origin, last)).sqrt();
                            tail_seg = ((g.length_m - arc) / (points - 1 - k).max(1) as f64).max(0.0);
                            origin
                        } else {
                            let mut dir = unit(to);
                            if let Some(j) = surface.nearest(last, HUG_M) {
                                let along = tangent(add(dir, [0.0, 0.0, TIE_UP_BIAS]), surface.normals[j]);
                                if dot(along, along) > 1e-12 {
                                    dir = unit(along);
                                    hugged += 1;
                                }
                            }
                            add(last, scale(dir, seg.min(d)))
                        }
                    }
                    Some(dir) => {
                        let bent = unit(add(dir, scale(down, g.droop * 0.35)));
                        tail = Some(bent);
                        add(last, scale(unit(add(bent, scale(spread, 1.0 / TAIL_RADIUS_M))), tail_seg))
                    }
                };
                if tail.is_none()
                    && dot(sub(tp, next), sub(tp, next)) > TIE_CLEAR_OF_M * TIE_CLEAR_OF_M
                    && let Some(j) = surface.nearest(next, NEAR_M)
                {
                    let off = surface.signed(next, j);
                    if off > GATHER_SNUG_M {
                        next = sub(next, scale(surface.normals[j], off - GATHER_SNUG_M));
                    }
                }
                if tail.is_none() || tied_at != k {
                    arc += dot(sub(next, last), sub(next, last)).sqrt();
                }
                if let Some(q) = outside(surface, next) {
                    next = q;
                    pushes += 1;
                }
                if tail.is_none()
                    && dot(sub(tp, next), sub(tp, next)) > TIE_CLEAR_OF_M * TIE_CLEAR_OF_M
                    && let Some(j) = surface.nearest(next, HUG_M)
                {
                    gather_off = gather_off.max(surface.signed(next, j));
                }
                line.push(next);
            }
            asked += g.length_m;
            tails.push(tied_at);
            lines.push(line);
            continue;
        }
        let w = topness(p[2]);
        let on_top = w > 0.5;
        top_strands += usize::from(on_top);
        let styled = on_top && g.direction != Direction::Part;
        let behind = ((p[1] - crown[1]) / TAPER_SPAN_M).clamp(0.0, 1.0);
        let top_length = if styled { g.length_m * (1.0 - TAPER * behind * behind * (3.0 - 2.0 * behind)) } else { g.length_m };
        let length = if g.sides_m > 0.0 { g.sides_m + (top_length - g.sides_m).max(0.0) * w } else { top_length };
        let back_of_front = if g.frame_m > 0.0 { hairline.behind_face_band(p) } else { 1.0 };
        let length = if g.frame_m > 0.0 && back_of_front < 1.0 { length.min(g.frame_m + (length - g.frame_m).max(0.0) * back_of_front) } else { length };
        framed += usize::from(g.frame_m > 0.0 && back_of_front < 0.5 && length < g.length_m);
        asked += length;
        let seg = length / (points - 1) as f64;
        let lift = LIFT + (g.lift - LIFT) * w;
        let free = on_top && g.direction == Direction::Up && p[1] < crown[1] - UP_FRONT_M;
        let combed = if g.direction == Direction::Up && !free { Direction::Back } else { g.direction };
        let side_of_face = face.side_of(p);
        let combed_flow = |q: Point, m: Point| if on_top { top_flow(combed, m).unwrap_or_else(|| flow(q, m)) } else { flow(q, m) };
        let comb = |q: Point, m: Point| {
            let f = combed_flow(q, m);
            if back_of_front >= 1.0 {
                return f;
            }
            let frame = tangent([side_of_face, -0.05, -FRAME_DOWN], m);
            if dot(frame, frame) < 1e-10 {
                return f;
            }
            let mixed = add(scale(unit(frame), 1.0 - back_of_front), scale(f, back_of_front));
            if dot(mixed, mixed) < 1e-10 { f } else { unit(mixed) }
        };
        tails.push(if free { 0 } else { points });
        let frontness = ((crown[1] - UP_FRONT_M - p[1]) / QUIFF_SPAN_M).clamp(0.0, 1.0);
        let rise = g.lift * QUIFF_RISE_M * (QUIFF_BACK_SHARE + (1.0 - QUIFF_BACK_SHARE) * frontness);
        let rise_dir = unit([0.0, -QUIFF_FORWARD * frontness, 1.0]);
        let quiff_dir = |s: f64| {
            let w = smoothstep01((s - rise) / QUIFF_TURN_M);
            unit(add(scale(rise_dir, 1.0 - w), scale(unit(QUIFF_BACK), w)))
        };
        let mut arc = 0.0;
        let mut dir = if free { quiff_dir(0.0) } else { unit(add(scale(n, lift), scale(comb(p, n), 1.0 - lift))) };
        let drape = unit([0.0, if p[1] > face.ears_y { 1.0 } else { -1.0 }, -1.0]);
        for _ in 1..points {
            let mut next = add(*line.last().unwrap(), scale(dir, seg));
            let mut landed = false;
            if let Some(q) = outside(surface, next) {
                next = q;
                pushes += 1;
                landed = next[2] <= brim;
            }
            if let Some(q) = face.keep_out_toward(next, face.side_of(p)) {
                next = q;
                kept_off_face += 1;
            }
            let step = sub(next, *line.last().unwrap());
            next = add(*line.last().unwrap(), scale(unit(step), seg));
            dir = unit(add(unit(step), scale(down, g.droop * 0.35)));
            if landed {
                dir = unit(add(dir, scale(drape, DRAPE_PULL)));
                draped += 1;
            }
            arc += seg;
            if free {
                dir = quiff_dir(arc);
            } else if next[2] > brim
                && let Some(j) = surface.nearest(next, HUG_M)
            {
                let n = surface.normals[j];
                dir = unit(add(scale(dir, 1.0 - STEER), scale(comb(next, n), STEER)));
                let out = dot(dir, n);
                if out > 0.0 {
                    dir = unit(sub(dir, scale(n, out)));
                    hugged += 1;
                }
            } else if next[2] > brim {
                dir = unit(add(dir, scale(down, FALL)));
            }
            line.push(next);
        }
        lines.push(line);
    }
    let depth_of = |lines: &[Vec<Point>]| -> f64 {
        lines.iter().flat_map(|l| l.iter().skip(1)).filter_map(|&p| surface.nearest(p, NEAR_M).map(|j| -surface.signed(p, j))).fold(0.0, f64::max)
    };
    let grown_depth = depth_of(&lines);
    let mut lock_of: Vec<usize> = (0..lines.len()).collect();
    let mut highlighted = 0usize;
    if g.clumps > 0 && g.clump > 0.0 {
        let k = g.clumps.min(lines.len());
        let centres: Vec<usize> = (0..k).map(|i| i * lines.len() / k).collect();
        let side = |i: usize| {
            if fringe_of[i] {
                (true, (g.fringe == Fringe::Curtain).then(|| roots[i].0[0] >= face.centre_x))
            } else {
                (false, part_x.map(|x| roots[i].0[0] >= x))
            }
        };
        for s in 0..lines.len() {
            let Some(&c) = centres.iter().filter(|&&a| side(a) == side(s)).min_by(|&&a, &&b| dot(sub(roots[a].0, roots[s].0), sub(roots[a].0, roots[s].0)).total_cmp(&dot(sub(roots[b].0, roots[s].0), sub(roots[b].0, roots[s].0)))) else {
                continue;
            };
            lock_of[s] = c;
            if c == s {
                continue;
            }
            let centre = lines[c].clone();
            let offset = sub(roots[s].0, roots[c].0);
            for (k_, p) in lines[s].iter_mut().enumerate() {
                let t = k_ as f64 / (points - 1) as f64;
                let target = add(centre[k_], scale(offset, 1.0 - t));
                *p = add(*p, scale(sub(target, *p), g.clump * t));
            }
        }
    }
    let mut strands = Vec::with_capacity(lines.len());
    let mut deepest = 0.0f64;
    let mut deepest_at = [0.0f64; 3];
    let mut under_jaw = 0usize;
    let mut flyaways = 0usize;
    let mut fly_roots: Vec<Value> = Vec::new();
    let mut jaw_roots: Vec<Value> = Vec::new();
    let mut lengths = Vec::with_capacity(lines.len());
    for (s, line) in lines.iter_mut().enumerate() {
        let tail_from = tails[s];
        let side = face.side_of(line[0]);
        let clear_forehead = !fringe_of[s] && g.direction != Direction::Forward;
        let phase = rng.next() * std::f64::consts::TAU;
        let mut arc = 0.0;
        let base = line.clone();
        for k in 0..base.len() {
            if k > 0 {
                arc += dot(sub(base[k], base[k - 1]), sub(base[k], base[k - 1])).sqrt();
            }
            let t = unit(sub(base[(k + 1).min(base.len() - 1)], base[k.saturating_sub(1)]));
            let u = unit(cross(t, if t[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] }));
            let v = cross(t, u);
            let fade = 1.0 - (-arc / CURL_FADE_M).exp();
            let theta = std::f64::consts::TAU * g.curl_turns_per_m * arc + phase;
            let curl = scale(add(scale(u, theta.cos()), scale(v, theta.sin())), g.curl_radius_m * fade);
            let tip = k as f64 / (base.len() - 1) as f64;
            let frizz = scale([rng.next() - 0.5, rng.next() - 0.5, rng.next() - 0.5], 2.0 * g.frizz_m * tip);
            line[k] = add(add(base[k], curl), frizz);
            for _ in 0..SETTLE_ROUNDS * usize::from(k > 0) {
                let face_moved = face.keep_out_toward(line[k], side);
                if let Some(q) = face_moved {
                    line[k] = q;
                    kept_off_face += 1;
                }
                let forehead = if clear_forehead { hairline.off_forehead(line[k]) } else { None };
                if let Some(q) = forehead {
                    line[k] = q;
                    off_forehead += 1;
                }
                let skin_moved = outside(surface, line[k]);
                if let Some(q) = skin_moved {
                    line[k] = q;
                }
                if skin_moved.is_none() || (face.keep_out_toward(line[k], side).is_none() && (!clear_forehead || hairline.off_forehead(line[k]).is_none())) {
                    break;
                }
            }
        }
        lengths.push(base.windows(2).map(|w| dot(sub(w[1], w[0]), sub(w[1], w[0])).sqrt()).sum::<f64>());
        for &p in line.iter().skip(1) {
            if let Some(j) = surface.nearest(p, NEAR_M) {
                let d = -surface.signed(p, j);
                if d > deepest {
                    deepest = d;
                    deepest_at = p;
                }
            }
        }
        let lit = g.highlights > 0.0 && lock_unit(g.seed, lock_of[s]) < g.highlights;
        highlighted += usize::from(lit);
        let colours = (0..line.len()).map(|k| {
            let t = k as f32 / (line.len() - 1) as f32;
            let base: [f32; 3] = std::array::from_fn(|i| g.colour[i] + (g.tip_colour[i] - g.colour[i]) * t);
            let lift = if lit { highlight_lift(t) } else { 0.0 };
            std::array::from_fn(|i| (base[i] + (g.highlight_colour[i] - base[i]) * lift) * (0.55 + 0.45 * t))
        });
        if let Some(q) = line.iter().take(tail_from).find(|q| q[2] > brim && surface.nearest(**q, FLYAWAY_M + g.curl_radius_m).is_none()) {
            flyaways += 1;
            if fly_roots.len() < 6 {
                fly_roots.push(json!({"root": line[0].map(|x| (x * 1e4).round() / 1e4), "far_point": q.map(|x| (x * 1e4).round() / 1e4)}));
            }
        }
        if line.iter().any(|q| face.under_jaw(*q)) {
            under_jaw += 1;
            if jaw_roots.len() < 8 {
                jaw_roots.push(json!({"root": line[0].map(|x| (x * 1e4).round() / 1e4), "lowest_ahead": line.iter().filter(|q| face.under_jaw(**q)).map(|q| q.map(|x| (x * 1e4).round() / 1e4)).next()}));
            }
        }
        strands.push(Strand { points: line.clone(), colours: colours.collect() });
    }
    let root_scalp: f64 = roots
        .iter()
        .map(|(p, _)| {
            let near = (0..head.len()).min_by(|&a, &b| dot(sub(head[a], *p), sub(head[a], *p)).total_cmp(&dot(sub(head[b], *p), sub(head[b], *p)))).unwrap_or(0);
            scalp[near]
        })
        .take(500)
        .sum::<f64>()
        / roots.len().min(500) as f64;
    let mean = lengths.iter().sum::<f64>() / lengths.len().max(1) as f64;
    let height = |ids: &[usize]| ids.iter().filter_map(|i| landmarks.get(i)).map(|p| p[2]).sum::<f64>() / ids.len() as f64;
    let brows = hairline.brows;
    let (subnasale, chin) = (height(&[2]), height(&[152]));
    let lowest_front = roots.iter().filter(|(p, _)| (p[0] - face.centre_x).abs() < HAIRLINE_CENTRE_M && p[1] < crown[1]).map(|(p, _)| p[2]).fold(f64::INFINITY, f64::min);
    let fringe_low = strands
        .iter()
        .zip(&fringe_of)
        .filter(|(_, f)| **f)
        .flat_map(|(s, _)| s.points.iter())
        .filter(|p| (p[0] - face.centre_x).abs() < HAIRLINE_CENTRE_M)
        .map(|p| p[2])
        .fold(f64::INFINITY, f64::min);
    let eyes_z = height(&[33, 263]);
    let over_eyes: Vec<usize> = strands
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.points.iter().any(|p| (p[0] - face.centre_x).abs() < face.half_width - FACE_MARGIN_M && p[2] > eyes_z - EYES_BAND_M && p[2] < brows && p[1] < face.eyes_y)
        })
        .map(|(i, _)| i)
        .collect();
    let on_forehead: Vec<(usize, usize)> = strands
        .iter()
        .zip(&fringe_of)
        .enumerate()
        .filter(|(_, (_, f))| !**f)
        .filter_map(|(i, (s, _))| {
            s.points
                .iter()
                .position(|p| (p[0] - face.centre_x).abs() < face.half_width - FACE_MARGIN_M && p[2] > brows && p[2] < hairline.low(*p) - HAIRLINE_SOFT_M - FOREHEAD_CLEAR_M && p[1] < face.eyes_y)
                .map(|k| (i, k))
        })
        .collect();
    let mid = |q: &Point| (q[0] - face.centre_x).abs() < QUIFF_PROFILE_HALF_M;
    let front_y = roots.iter().map(|(q, _)| *q).filter(mid).map(|q| q[1]).fold(f64::INFINITY, f64::min);
    let in_front = |q: &Point| mid(q) && q[1] < front_y + QUIFF_PROFILE_BAND_M;
    let front_scalp = roots.iter().map(|(q, _)| *q).filter(in_front).map(|q| q[2]).fold(f64::NEG_INFINITY, f64::max);
    let front_hair = strands.iter().flat_map(|s| s.points.iter()).filter(|q| in_front(q)).map(|q| q[2]).fold(f64::NEG_INFINITY, f64::max);
    let front_thickness = (front_hair - front_scalp).max(0.0);
    let below_throat: Vec<&Strand> = strands.iter().filter(|s| s.points.iter().any(|q| q[2] < face.bottom)).collect();
    let tips_outward = below_throat
        .iter()
        .filter(|s| {
            let n = s.points.len();
            let k = (n / 4).max(2).min(n - 1);
            let d = sub(s.points[n - 1], s.points[n - 1 - k]);
            dot(d, d) > 1e-12 && unit(d)[0].abs() > 0.5
        })
        .count();
    let fringe_below =strands.iter().zip(&fringe_of).filter(|(_, f)| **f).flat_map(|(s, _)| s.points.iter()).filter(|p| p[2] < hairline.brows).count();
    let cm = |x: f64| (x * 1e4).round() / 1e4;
    let face_report = json!({"hairline_above_brows_m": lowest_front.is_finite().then(|| cm(lowest_front - brows)), "hairline_asked_above_brows_m": cm(hairline.z - brows),
                             "hairline_soft_m": HAIRLINE_SOFT_M, "fringe_strands": fringe_of.iter().filter(|f| **f).count(),
                             "fringe_lowest_above_brows_m": fringe_low.is_finite().then(|| cm(fringe_low - brows)), "fringe_end_above_brows_m": FRINGE_ABOVE_BROWS_M, "fringe_points_below_brows": fringe_below,
                             "landmark_10_above_brows_m": cm(height(&[10]) - brows),
                             "brows_to_subnasale_m": cm(brows - subnasale), "subnasale_to_chin_m": cm(subnasale - chin)});
    let report = json!({"face": face_report,"name": g.name, "strands": strands.len(), "points_per_strand": points, "length_m": g.length_m,
                        "mean_centreline_m": (mean * 1e5).round() / 1e5, "scalp_triangles": weights.iter().filter(|w| **w > 0.0).count(),
                        "mean_scalp_weight_at_roots_first_500": (root_scalp * 1000.0).round() / 1000.0, "surface_pushes": pushes, "tie_gather_off_head_mm": (gather_off * 1e4).round() / 10.0, "framed_strands": framed, "highlighted_strands": highlighted, "draped_steps": draped,
                        "strands_below_throat": below_throat.len(), "tips_outward": tips_outward,
                        "front_thickness_m": if front_thickness.is_finite() { (front_thickness * 1e4).round() / 1e4 } else { 0.0 },
                        "quiff_rise_asked_m": if g.direction == Direction::Up { (g.lift * QUIFF_RISE_M * 1e4).round() / 1e4 } else { 0.0 },
                        "strands_over_eyes": over_eyes.len(), "strands_on_forehead": on_forehead.len(), "on_forehead_examples": on_forehead.iter().take(6).map(|&(i, k)| json!({"root": roots[i].0.map(cm), "point": k, "at": strands[i].points[k].map(cm)})).collect::<Vec<_>>(),"over_eyes_roots": over_eyes.iter().take(6).map(|&i| roots[i].0.map(|x| (x * 1e4).round() / 1e4)).collect::<Vec<_>>(),"mean_asked_m": (asked / lines.len().max(1) as f64 * 1e5).round() / 1e5, "top_strands": top_strands, "kept_off_face": kept_off_face, "lifted_off_forehead": off_forehead,"hugged_steps": hugged,
                        "deepest_inside_surface_mm": (deepest.max(0.0) * 1e6).round() / 1000.0, "strands_under_jaw": under_jaw, "under_jaw_examples": jaw_roots, "flyaways": flyaways, "flyaway_distance_mm": ((FLYAWAY_M + g.curl_radius_m) * 1e4).round() / 10.0, "flyaway_examples": fly_roots, "deepest_at_m": deepest_at.map(|x| (x * 1e4).round() / 1e4), "ears_m": ([234usize, 454].map(|i| landmarks.get(&i).map(|p| p.map(|x| (x * 1e4).round() / 1e4)))), "deepest_after_growth_mm": (grown_depth.max(0.0) * 1e6).round() / 1000.0, "part": format!("{:?}", g.part).to_lowercase(),
                        "method": "roots sampled by area over the head's scalp region (skin region scalp >= 0.5, from the built-in landmarks on this character's dense surface); with sides_m set, a root below the fade line (3.5 cm above the ears, blended over fade_m) gets the sides length instead of length_m; each strand leaves the scalp lifted by lift (0.35; the top's own lift above the fade line) along the normal, follows the head's curve while above the ears and within 2 cm of it (each step there turns halfway back to the comb direction and never away from the surface; past 2 cm off the head above the ears it falls), and is combed (on the top, back, forward or up when direction says so, a styled top shortening toward the back to 35 % at 8 cm behind the crown; up stands the hair from 3 cm ahead of the crown forward free of the head, mostly toward world up leaning back and rolling back over, and combs the rest back) along a flow away from the part ahead of the crown, backwards behind it and back-and-sideways at the front (downward weighted 0.6), bends down by droop, kept 3 mm outside the head and body (pushed out along the nearest surface normal, again from the new nearest point up to 4 times, for creases such as behind the ears); then clumped toward a few strands (on the same side of the part), curled (fading in over 3 cm), frizzed toward the tip (a strand gets a point at least every 1.5 cm and, curled, at least 8 points per turn, up to 256), and every point kept 3 mm outside again; no point in front of the face: between the outer eye corners plus 1 cm, widening by 1 cm per cm ahead of them, from 5 cm under the chin (in front of the throat) to 2 cm under the top of the forehead, ahead of the ears; such a point moves to the nearest edge on its strand's side (the root's), so a curl that crosses the middle does not jump to the other side (sideways and back ahead of the eyes, so hair falls beside the cheek, not in front of it); and, unless direction is forward, no point of a strand outside the fringe on the forehead (between the outer eye corners, ahead of the eyes, above the brows and below the hairline less its soft edge): it rises to that height, the rise fading out over the last 1 cm toward the eye corners"});
    Ok((strands, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_curled_strand_gets_points_enough_for_its_turns_and_keeps_its_side() {
        let mut g = parse(&json!({"format": FORMAT, "length_m": 0.3})).unwrap();
        assert_eq!(strand_points(&g), 21, "a straight 30 cm groom gets a point every 1.5 cm");
        g.curl_turns_per_m = 20.0;
        assert_eq!(strand_points(&g), 21, "turns without a radius are no curl");
        g.curl_radius_m = 0.01;
        assert_eq!(strand_points(&g), 49, "6 turns, 8 points each, plus the root");
        g.curl_turns_per_m = 200.0;
        g.length_m = 2.0;
        assert_eq!(strand_points(&g), MAX_POINTS);
        let face = Face { centre_x: 0.0, half_width: 0.05, bottom: 1.5, chin: 1.55, top: 1.7, eyes_y: -0.08, ears_y: 0.0 };
        assert_eq!(face.keep_out_toward([-0.01, -0.05, 1.6], 1.0), Some([0.05, -0.05, 1.6]), "a right-side strand's point that curled left goes back right");
        assert_eq!(face.keep_out([-0.01, -0.05, 1.6]), Some([-0.05, -0.05, 1.6]), "a lone point goes to its own side");
    }

    #[test]
    fn hair_in_front_of_the_face_moves_to_the_nearest_edge_beside_it() {
        let face = Face { centre_x: 0.0, half_width: 0.05, bottom: 1.5, chin: 1.55, top: 1.7, eyes_y: -0.08, ears_y: 0.0 };
        let inside = |p: Point| (p[0].abs() < face.half_width + (face.eyes_y - p[1]).max(0.0) - 1e-9) && p[1] < face.ears_y;
        assert_eq!(face.keep_out([0.02, -0.05, 1.6]), Some([0.05, -0.05, 1.6]));
        assert_eq!(face.keep_out([-0.02, -0.05, 1.6]), Some([-0.05, -0.05, 1.6]));
        let q = face.keep_out([0.04, -0.12, 1.6]).unwrap();
        assert!((q[0] - 0.065).abs() < 1e-12 && (q[1] + 0.095).abs() < 1e-12, "{q:?}");
        assert!(!inside(q) && q[1] > -0.12);
        assert_eq!(face.keep_out([0.0, -0.081, 1.6]), Some([0.05, -0.08, 1.6]));
        for p in [[0.06, -0.05, 1.6], [0.1, -0.12, 1.6], [0.0, -0.05, 1.45], [0.0, -0.05, 1.75], [0.0, 0.01, 1.6]] {
            assert_eq!(face.keep_out(p), None, "{p:?}");
        }
    }

    #[test]
    fn hair_on_the_forehead_rises_to_the_hairline_and_fades_out_at_the_eye_corners() {
        let hairline = Hairline { z: 1.7, brows: 1.62, ears_z: 1.6, centre_x: 0.0, half_width: 0.05, eyes_y: -0.08, ears_y: 0.0 };
        let floor = 1.7 - HAIRLINE_SOFT_M;
        let q = hairline.off_forehead([0.01, -0.1, 1.65]).unwrap();
        assert_eq!([q[0], q[1]], [0.01, -0.1]);
        assert!((q[2] - (hairline.low(q) - HAIRLINE_SOFT_M)).abs() < 1e-12, "{q:?}");
        assert_eq!(hairline.off_forehead(q), None, "a lifted point stays put");
        let edge = hairline.off_forehead([0.045, -0.1, 1.65]).unwrap();
        assert!(edge[2] > 1.65 && edge[2] < hairline.low(edge) - HAIRLINE_SOFT_M, "{edge:?}");
        for p in [[0.0, -0.1, 1.61], [0.0, -0.07, 1.65], [0.0, -0.1, floor + 0.001], [0.06, -0.1, 1.65]] {
            assert_eq!(hairline.off_forehead(p), None, "{p:?}");
        }
    }
}
