use std::path::Path;

use gf_core::game_mesh::{Point, game_normals};
use gf_core::skin_regions;
use serde_json::{Value, json};

use super::strands::Strand;
use super::wearable::Surface;

pub const FORMAT: &str = "gazeforge.groom/1";
const SCALP: usize = 11;
const ROOT_LIFT_M: f64 = 0.001;
const CLEAR_M: f64 = 0.003;
const NEAR_M: f64 = 0.05;
const LIFT: f64 = 0.35;
const PART_OFFSET_M: f64 = 0.035;
const CURL_FADE_M: f64 = 0.03;
const FRONT_BACK: f64 = 1.5;
const FACE_MARGIN_M: f64 = 0.01;
const FRINGE_M: f64 = 0.02;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Left,
    Right,
    Centre,
    None,
}

#[derive(Clone, Debug)]
pub struct Groom {
    pub name: String,
    pub strands: usize,
    pub points: usize,
    pub length_m: f64,
    pub part: Part,
    pub droop: f64,
    pub curl_radius_m: f64,
    pub curl_turns_per_m: f64,
    pub clumps: usize,
    pub clump: f64,
    pub frizz_m: f64,
    pub colour: [f32; 3],
    pub seed: u64,
}

fn linear(c: f64) -> f32 {
    let s = c.clamp(0.0, 1.0);
    (if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }) as f32
}

pub fn parse(v: &Value) -> Result<Groom, String> {
    if v["format"].as_str() != Some(FORMAT) {
        return Err(format!("a groom file says \"format\": \"{FORMAT}\""));
    }
    let num = |key: &str, default: f64, lo: f64, hi: f64| -> Result<f64, String> {
        let x = match v.get(key) {
            None | Some(Value::Null) => default,
            Some(x) => x.as_f64().ok_or_else(|| format!("groom {key} must be a number"))?,
        };
        if !(lo..=hi).contains(&x) {
            return Err(format!("groom {key} must be from {lo} to {hi}, got {x}"));
        }
        Ok(x)
    };
    let part = match v["part"].as_str().unwrap_or("centre") {
        "left" => Part::Left,
        "right" => Part::Right,
        "centre" | "center" => Part::Centre,
        "none" => Part::None,
        other => return Err(format!("groom part is left, right, centre or none, got {other}")),
    };
    let colour = match v["colour"].as_str() {
        None => [0.18, 0.11, 0.07].map(linear),
        Some(hex) => {
            let h = hex.trim_start_matches('#');
            let c = (h.len() == 6).then(|| (0..3).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect::<Option<Vec<u8>>>()).flatten();
            let c = c.ok_or_else(|| format!("groom colour is #rrggbb, got {hex}"))?;
            [0, 1, 2].map(|i| linear(c[i] as f64 / 255.0))
        }
    };
    Ok(Groom {
        name: v["name"].as_str().unwrap_or("groom").to_string(),
        strands: num("strands", 20000.0, 1.0, 500_000.0)? as usize,
        points: num("points", 16.0, 3.0, 64.0)? as usize,
        length_m: num("length_m", 0.15, 0.005, 2.0)?,
        part,
        droop: num("droop", 0.5, 0.0, 1.0)?,
        curl_radius_m: num("curl_radius_m", 0.0, 0.0, 0.1)?,
        curl_turns_per_m: num("curl_turns_per_m", 0.0, 0.0, 200.0)?,
        clumps: num("clumps", 0.0, 0.0, 5000.0)? as usize,
        clump: num("clump", 0.0, 0.0, 1.0)?,
        frizz_m: num("frizz_m", 0.0, 0.0, 0.05)?,
        colour,
        seed: num("seed", 1.0, 0.0, u32::MAX as f64)? as u64,
    })
}

pub fn read(path: &Path) -> Result<Groom, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?)
}

struct Face {
    centre_x: f64,
    half_width: f64,
    bottom: f64,
    top: f64,
    eyes_y: f64,
    ears_y: f64,
}

impl Face {
    fn keep_out(&self, p: Point) -> Option<Point> {
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
        Some([self.centre_x + if p[0] >= self.centre_x { dx } else { -dx }, y, p[2]])
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: Point, s: f64) -> Point {
    a.map(|x| x * s)
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Point, b: Point) -> Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(a: Point) -> Point {
    let l = dot(a, a).sqrt();
    if l < 1e-12 { a } else { scale(a, 1.0 / l) }
}

fn tangent(v: Point, n: Point) -> Point {
    sub(v, scale(n, dot(v, n)))
}

pub fn grow(g: &Groom, head: &[Point], triangles: &[[u32; 3]], dense: &[[f64; 3]], surface: &Surface) -> Result<(Vec<Strand>, Value), String> {
    let landmarks = gf_core::base_landmarks::on(dense)?;
    let lm = |i: usize| landmarks.get(&i).copied().ok_or_else(|| format!("landmark {i} is missing"));
    let (eye_r, eye_l, chin, brow, ear_r, ear_l) = (lm(33)?, lm(263)?, lm(152)?, lm(10)?, lm(234)?, lm(454)?);
    let face = Face {
        centre_x: (eye_r[0] + eye_l[0]) / 2.0,
        half_width: (eye_l[0] - eye_r[0]).abs() / 2.0 + FACE_MARGIN_M,
        bottom: chin[2],
        top: brow[2] - FRINGE_M,
        eyes_y: (eye_r[1] + eye_l[1]) / 2.0,
        ears_y: (ear_r[1] + ear_l[1]) / 2.0,
    };
    let mut kept_off_face = 0usize;
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
    let seg = g.length_m / (g.points - 1) as f64;
    let mut roots = Vec::with_capacity(g.strands);
    let mut lines: Vec<Vec<Point>> = Vec::with_capacity(g.strands);
    let mut pushes = 0usize;
    for _ in 0..g.strands {
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
        roots.push((p, n));
        let mut line = vec![add(p, scale(n, ROOT_LIFT_M))];
        let mut dir = unit(add(scale(n, LIFT), scale(flow(p, n), 1.0 - LIFT)));
        for _ in 1..g.points {
            let mut next = add(*line.last().unwrap(), scale(dir, seg));
            if let Some(j) = surface.nearest(next, NEAR_M) {
                let s = surface.signed(next, j);
                if s < CLEAR_M {
                    next = add(next, scale(surface.normals[j], CLEAR_M - s));
                    pushes += 1;
                }
            }
            if let Some(q) = face.keep_out(next) {
                next = q;
                kept_off_face += 1;
            }
            let step = sub(next, *line.last().unwrap());
            next = add(*line.last().unwrap(), scale(unit(step), seg));
            dir = unit(add(unit(step), scale(down, g.droop * 0.35)));
            line.push(next);
        }
        lines.push(line);
    }
    let depth_of = |lines: &[Vec<Point>]| -> f64 {
        lines.iter().flat_map(|l| l.iter().skip(1)).filter_map(|&p| surface.nearest(p, NEAR_M).map(|j| -surface.signed(p, j))).fold(0.0, f64::max)
    };
    let grown_depth = depth_of(&lines);
    if g.clumps > 0 && g.clump > 0.0 {
        let k = g.clumps.min(lines.len());
        let centres: Vec<usize> = (0..k).map(|i| i * lines.len() / k).collect();
        for s in 0..lines.len() {
            let c = *centres.iter().min_by(|&&a, &&b| dot(sub(roots[a].0, roots[s].0), sub(roots[a].0, roots[s].0)).total_cmp(&dot(sub(roots[b].0, roots[s].0), sub(roots[b].0, roots[s].0)))).unwrap();
            if c == s {
                continue;
            }
            let centre = lines[c].clone();
            let offset = sub(roots[s].0, roots[c].0);
            for (k_, p) in lines[s].iter_mut().enumerate() {
                let t = k_ as f64 / (g.points - 1) as f64;
                let target = add(centre[k_], scale(offset, 1.0 - t));
                *p = add(*p, scale(sub(target, *p), g.clump * t));
            }
        }
    }
    let mut strands = Vec::with_capacity(lines.len());
    let mut deepest = 0.0f64;
    let mut lengths = Vec::with_capacity(lines.len());
    for line in &mut lines {
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
            if k > 0
                && let Some(q) = face.keep_out(line[k])
            {
                line[k] = q;
                kept_off_face += 1;
            }
            if k > 0
                && let Some(j) = surface.nearest(line[k], NEAR_M)
            {
                let s = surface.signed(line[k], j);
                if s < CLEAR_M {
                    line[k] = add(line[k], scale(surface.normals[j], CLEAR_M - s));
                }
            }
        }
        lengths.push(base.windows(2).map(|w| dot(sub(w[1], w[0]), sub(w[1], w[0])).sqrt()).sum::<f64>());
        for &p in line.iter().skip(1) {
            if let Some(j) = surface.nearest(p, NEAR_M) {
                deepest = deepest.max(-surface.signed(p, j));
            }
        }
        let colours = (0..line.len()).map(|k| {
            let t = k as f32 / (line.len() - 1) as f32;
            g.colour.map(|c| c * (0.55 + 0.45 * t))
        });
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
    let report = json!({"name": g.name, "strands": strands.len(), "points_per_strand": g.points, "length_m": g.length_m,
                        "mean_centreline_m": (mean * 1e5).round() / 1e5, "scalp_triangles": weights.iter().filter(|w| **w > 0.0).count(),
                        "mean_scalp_weight_at_roots_first_500": (root_scalp * 1000.0).round() / 1000.0, "surface_pushes": pushes, "kept_off_face": kept_off_face,
                        "deepest_inside_surface_mm": (deepest.max(0.0) * 1e6).round() / 1000.0, "deepest_after_growth_mm": (grown_depth.max(0.0) * 1e6).round() / 1000.0, "part": format!("{:?}", g.part).to_lowercase(),
                        "method": "roots sampled by area over the head's scalp region (skin region scalp >= 0.5, from the built-in landmarks on this character's dense surface); each strand leaves the scalp lifted 0.35 along the normal and combed along a flow away from the part ahead of the crown, backwards behind it and back-and-sideways at the front (downward weighted 0.6), bends down by droop, kept 3 mm outside the head and body; then clumped toward a few strands, curled (fading in over 3 cm), frizzed toward the tip, and every point kept 3 mm outside again; no point in front of the face: between the outer eye corners plus 1 cm, widening by 1 cm per cm ahead of them, from the chin to 2 cm under the top of the forehead, ahead of the ears; such a point moves to the nearest edge on the side it came from (sideways and back ahead of the eyes, so hair falls beside the cheek, not in front of it)"});
    Ok((strands, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_groom_file_reads_with_defaults_and_refuses_nonsense() {
        let g = parse(&json!({"format": FORMAT, "length_m": 0.3, "part": "left", "colour": "#806040"})).unwrap();
        assert_eq!((g.length_m, g.part, g.points, g.strands), (0.3, Part::Left, 16, 20000));
        assert!((g.colour[0] - linear(128.0 / 255.0)).abs() < 1e-6);
        assert!(parse(&json!({"length_m": 0.3})).is_err(), "the format is required");
        assert!(parse(&json!({"format": FORMAT, "part": "middle"})).is_err());
        assert!(parse(&json!({"format": FORMAT, "droop": 2.0})).is_err());
        assert!(parse(&json!({"format": FORMAT, "colour": "brown"})).is_err());
    }

    #[test]
    fn hair_in_front_of_the_face_moves_to_the_nearest_edge_beside_it() {
        let face = Face { centre_x: 0.0, half_width: 0.05, bottom: 1.5, top: 1.7, eyes_y: -0.08, ears_y: 0.0 };
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
}
