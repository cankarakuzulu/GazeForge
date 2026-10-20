use std::collections::BTreeMap;

type Point = [f64; 3];

pub const NAMES: [&str; 14] =
    ["lips", "mouth_inner", "nose", "cheeks", "forehead", "chin", "upper_lip", "beard", "under_eye", "eyelids", "ears", "scalp", "neck", "t_zone"];

const LIPS_OUTER: [usize; 20] = [61, 146, 91, 181, 84, 17, 314, 405, 321, 375, 291, 409, 270, 269, 267, 0, 37, 39, 40, 185];
const LIPS_INNER: [usize; 20] = [78, 95, 88, 178, 87, 14, 317, 402, 318, 324, 308, 415, 310, 311, 312, 13, 82, 81, 80, 191];
const JAW: [usize; 15] = [58, 172, 136, 150, 149, 176, 148, 152, 377, 400, 378, 379, 365, 397, 288];

pub struct Regions {
    pub weights: Vec<[f32; NAMES.len()]>,
}

impl Regions {
    pub fn index(name: &str) -> Option<usize> {
        NAMES.iter().position(|n| *n == name)
    }
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn norm(a: Point) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn blobs(p: Point, centres: &[(Point, f64)]) -> f64 {
    centres.iter().map(|&(c, r)| (-(norm(sub(p, c)) / r).powi(2)).exp()).fold(0.0, f64::max)
}

fn polygon_distance(x: f64, z: f64, poly: &[[f64; 2]]) -> f64 {
    let mut inside = false;
    let mut best = f64::INFINITY;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a[1] > z) != (b[1] > z) && x < (b[0] - a[0]) * (z - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        let (ex, ez) = (b[0] - a[0], b[1] - a[1]);
        let t = (((x - a[0]) * ex + (z - a[1]) * ez) / (ex * ex + ez * ez).max(1e-18)).clamp(0.0, 1.0);
        best = best.min(((x - a[0] - t * ex).powi(2) + (z - a[1] - t * ez).powi(2)).sqrt());
    }
    if inside { -best } else { best }
}

fn smooth(weights: &mut [[f32; NAMES.len()]], neighbours: &[Vec<u32>], which: &[usize], rounds: usize) {
    for _ in 0..rounds {
        let prev = weights.to_vec();
        for (v, nb) in neighbours.iter().enumerate() {
            if nb.is_empty() {
                continue;
            }
            for &r in which {
                let mean = nb.iter().map(|&u| prev[u as usize][r]).sum::<f32>() / nb.len() as f32;
                weights[v][r] = 0.5 * prev[v][r] + 0.5 * mean;
            }
        }
    }
}

pub fn neighbours(vertices: usize, triangles: &[[u32; 3]]) -> Vec<Vec<u32>> {
    let mut nb = vec![Vec::new(); vertices];
    for t in triangles {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            nb[a as usize].push(b);
            nb[b as usize].push(a);
        }
    }
    for n in &mut nb {
        n.sort_unstable();
        n.dedup();
    }
    nb
}

pub fn build(positions: &[Point], normals: &[Point], triangles: &[[u32; 3]], landmarks: &BTreeMap<usize, Point>) -> Result<Regions, String> {
    let lm = |i: usize| landmarks.get(&i).copied().ok_or_else(|| format!("landmark {i} is missing from the table"));
    let loop2 = |ids: &[usize]| -> Result<Vec<[f64; 2]>, String> { ids.iter().map(|&i| lm(i).map(|p| [p[0], p[2]])).collect() };
    let outer = loop2(&LIPS_OUTER)?;
    let inner = loop2(&LIPS_INNER)?;
    let lip_y = LIPS_OUTER.iter().map(|&i| lm(i).map(|p| p[1])).collect::<Result<Vec<_>, _>>()?.into_iter().fold(f64::INFINITY, f64::min);
    let at = |ids: &[usize], r: f64| -> Result<Vec<(Point, f64)>, String> { ids.iter().map(|&i| lm(i).map(|p| (p, r))).collect() };
    let nose = [at(&[1, 4, 5, 2, 98, 327, 48, 278], 0.008)?, at(&[195, 197, 6], 0.006)?].concat();
    let cheeks = [at(&[50, 280], 0.018)?, at(&[205, 425], 0.014)?].concat();
    let forehead = at(&[151, 9, 108, 337, 67, 297], 0.018)?;
    let chin = at(&[152, 175, 199, 200], 0.012)?;
    let upper_lip = at(&[164, 167, 393, 165, 391], 0.007)?;
    let jaw = at(&JAW, 0.016)?;
    let under_eye = at(&[230, 231, 232, 450, 451, 452, 111, 340], 0.008)?;
    let eyelids = [at(&[159, 386], 0.008)?, at(&[145, 374], 0.006)?].concat();
    let t_zone = at(&[9, 151, 168, 6, 197, 195, 5, 4, 1], 0.012)?;
    let (right, left, chin_pt, top) = (lm(234)?, lm(454)?, lm(152)?, lm(10)?);
    let (r_alar, l_alar) = (lm(98)?, lm(327)?);
    let ear_x = right[0].abs().min(left[0].abs());
    let ear_z = 0.5 * (right[2] + left[2]);
    let ear_y = 0.5 * (right[1] + left[1]);
    let nb = neighbours(positions.len(), triangles);
    let ear_raw = protrusion(positions, normals, &nb, 0.006, 0.011);
    let mut weights = vec![[0.0f32; NAMES.len()]; positions.len()];
    for (v, (&p, &n)) in positions.iter().zip(normals).enumerate() {
        let front = n[1] < -0.15;
        let w = &mut weights[v];
        let d_out = polygon_distance(p[0], p[2], &outer);
        let d_in = polygon_distance(p[0], p[2], &inner);
        let near_mouth = p[1] < lip_y + 0.012;
        let deep = p[1] > lip_y + 0.004;
        let lips = if near_mouth && front { 1.0 - smoothstep(-0.0004, 0.0004, d_out) } else { 0.0 };
        let mouth_inner = if near_mouth && deep { 1.0 - smoothstep(-0.0006, 0.0006, d_in) } else { 0.0 };
        w[0] = (lips * (1.0 - mouth_inner)) as f32;
        w[1] = mouth_inner as f32;
        let not_lips = 1.0 - lips;
        w[2] = (blobs(p, &nose) * if front { 1.0 } else { 0.3 }) as f32;
        w[3] = (blobs(p, &cheeks) * not_lips) as f32;
        w[4] = (blobs(p, &forehead) * if front { 1.0 } else { 0.0 }) as f32;
        w[5] = (blobs(p, &chin) * not_lips) as f32;
        w[6] = (blobs(p, &upper_lip) * not_lips) as f32;
        let side = |x: f64| if x < 0.0 { (right, r_alar) } else { (left, l_alar) };
        let (ear_pt, alar) = side(p[0]);
        let t = ((p[0] - ear_pt[0]) / (alar[0] - ear_pt[0])).clamp(0.0, 1.0);
        let line_z = ear_pt[2] + t * (alar[2] - ear_pt[2]) - 0.006;
        let below_line = 1.0 - smoothstep(line_z - 0.004, line_z + 0.004, p[2]);
        let above_neck = smoothstep(chin_pt[2] - 0.045, chin_pt[2] - 0.025, p[2]);
        let inside_ears = 1.0 - smoothstep(ear_x - 0.012, ear_x - 0.002, p[0].abs());
        let forward = 1.0 - smoothstep(ear_y - 0.005, ear_y + 0.01, p[1]);
        let zone = below_line * above_neck * inside_ears * forward;
        w[7] = (zone.max(blobs(p, &jaw) * inside_ears) * not_lips * (1.0 - mouth_inner)) as f32;
        w[8] = blobs(p, &under_eye) as f32;
        w[9] = blobs(p, &eyelids) as f32;
        let tragus = if p[0] < 0.0 { right } else { left };
        let ear = ear_raw[v] * (1.0 - smoothstep(0.045, 0.055, norm(sub(p, tragus))));
        w[10] = ear as f32;
        let scalp = smoothstep(top[2] - 0.012, top[2] + 0.004, p[2]).max(smoothstep(ear_y + 0.015, ear_y + 0.035, p[1]) * smoothstep(ear_z - 0.01, ear_z + 0.01, p[2]));
        w[11] = (scalp * (1.0 - ear)) as f32;
        w[12] = (1.0 - smoothstep(chin_pt[2] - 0.035, chin_pt[2] - 0.015, p[2])) as f32;
        w[13] = (blobs(p, &t_zone) * if front { 1.0 } else { 0.0 }) as f32;
    }
    smooth(&mut weights, &nb, &[2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13], 4);
    Ok(Regions { weights })
}

fn protrusion(positions: &[Point], normals: &[Point], nb: &[Vec<u32>], lo: f64, hi: f64) -> Vec<f64> {
    let mut s = positions.to_vec();
    for _ in 0..150 {
        let prev = s.clone();
        for (v, n) in nb.iter().enumerate() {
            if n.is_empty() {
                continue;
            }
            let mut m = [0.0; 3];
            for &u in n {
                for k in 0..3 {
                    m[k] += prev[u as usize][k];
                }
            }
            for k in 0..3 {
                s[v][k] = 0.5 * prev[v][k] + 0.5 * m[k] / n.len() as f64;
            }
        }
    }
    let mut raw: Vec<f64> = positions
        .iter()
        .zip(&s)
        .zip(normals)
        .map(|((p, q), n)| smoothstep(lo, hi, (p[0] - q[0]) * n[0] + (p[1] - q[1]) * n[1] + (p[2] - q[2]) * n[2]))
        .collect();
    for _ in 0..6 {
        let prev = raw.clone();
        for (v, n) in nb.iter().enumerate() {
            raw[v] = n.iter().map(|&u| prev[u as usize]).fold(prev[v], f64::max);
        }
    }
    raw
}
