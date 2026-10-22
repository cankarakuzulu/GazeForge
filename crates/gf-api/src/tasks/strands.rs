use std::collections::HashMap;

use gf_core::game_mesh::{Point, to_gltf};
use serde_json::{Value, json};

use super::Options;

pub const KEYS: &[&str] = &["strands", "strands-count", "strands-seed", "strands-points", "groom"];
const DEFAULT_COUNT: usize = 20000;
const DEFAULT_POINTS: usize = 16;
const TRACE_STEPS: usize = 256;
const MIN_LENGTH_M: f64 = 0.01;
const PROBES: usize = 16;
const OPAQUE: u8 = 128;
const ROOT_WIDTH_M: f64 = 0.00008;
const TIP_WIDTH_M: f64 = 0.00002;
const FOLDED_SHARE: f64 = 0.1;

fn uv_signed(t: [[f64; 2]; 3]) -> f64 {
    (t[1][0] - t[0][0]) * (t[2][1] - t[0][1]) - (t[2][0] - t[0][0]) * (t[1][1] - t[0][1])
}

pub struct Settings {
    pub out: String,
    pub count: usize,
    pub seed: u64,
    pub points: usize,
}

pub fn settings(opts: &Options) -> Result<Option<Settings>, String> {
    let Some(out) = opts.get("strands") else {
        if let Some(k) = KEYS[1..].iter().find(|k| opts.contains_key(**k)) {
            return Err(format!("--{k} needs --strands"));
        }
        return Ok(None);
    };
    if !out.ends_with(".usda") {
        return Err(format!("--strands writes USD text and must end in .usda, got {out}"));
    }
    let int = |key: &str, default: usize, lo: usize, hi: usize| -> Result<usize, String> {
        let v = match opts.get(key) {
            Some(s) => s.parse::<usize>().map_err(|_| format!("--{key} must be a whole number, got {s}"))?,
            None => default,
        };
        if !(lo..=hi).contains(&v) {
            return Err(format!("--{key} must be from {lo} to {hi}, got {v}"));
        }
        Ok(v)
    };
    Ok(Some(Settings {
        out: out.clone(),
        count: int("strands-count", DEFAULT_COUNT, 1, 500_000)?,
        seed: int("strands-seed", 1, 0, u32::MAX as usize)? as u64,
        points: int("strands-points", DEFAULT_POINTS, 3, 64)?,
    }))
}

pub struct Strand {
    pub points: Vec<Point>,
    pub colours: Vec<[f32; 3]>,
}

pub struct Texture<'a> {
    pub width: usize,
    pub height: usize,
    pub rgba: &'a [u8],
}

impl Texture<'_> {
    fn texel(&self, uv: [f64; 2]) -> &[u8] {
        let wrap = |t: f64| if (0.0..=1.0).contains(&t) { t } else { t.rem_euclid(1.0) };
        let x = ((wrap(uv[0]) * self.width as f64) as usize).min(self.width - 1);
        let y = ((wrap(uv[1]) * self.height as f64) as usize).min(self.height - 1);
        &self.rgba[(y * self.width + x) * 4..(y * self.width + x) * 4 + 4]
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

fn pieces(n: usize, triangles: &[[u32; 3]]) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut a: usize) -> usize {
        while parent[a] != a {
            parent[a] = parent[parent[a]];
            a = parent[a];
        }
        a
    }
    for t in triangles {
        let a = find(&mut parent, t[0] as usize);
        for &v in &t[1..] {
            let b = find(&mut parent, v as usize);
            parent[b] = a;
        }
    }
    let mut by_root: HashMap<usize, Vec<usize>> = HashMap::new();
    for (k, t) in triangles.iter().enumerate() {
        let r = find(&mut parent, t[0] as usize);
        by_root.entry(r).or_default().push(k);
    }
    let mut out: Vec<Vec<usize>> = by_root.into_values().collect();
    out.sort_by_key(|p| p[0]);
    out
}

fn barycentric(p: [f64; 2], t: &[[f64; 2]; 3]) -> Option<[f64; 3]> {
    let d = (t[1][1] - t[2][1]) * (t[0][0] - t[2][0]) + (t[2][0] - t[1][0]) * (t[0][1] - t[2][1]);
    if d.abs() < 1e-14 {
        return None;
    }
    let a = ((t[1][1] - t[2][1]) * (p[0] - t[2][0]) + (t[2][0] - t[1][0]) * (p[1] - t[2][1])) / d;
    let b = ((t[2][1] - t[0][1]) * (p[0] - t[2][0]) + (t[0][0] - t[2][0]) * (p[1] - t[2][1])) / d;
    let c = 1.0 - a - b;
    let eps = -1e-9;
    (a >= eps && b >= eps && c >= eps).then_some([a, b, c])
}

struct Piece {
    lo: [f64; 2],
    hi: [f64; 2],
    cells: (usize, usize),
    grid: Vec<Vec<usize>>,
    weight: f64,
}

impl Piece {
    fn new(triangles: Vec<usize>, uvs: &[[[f64; 2]; 3]], areas: &[f64], texture: &Texture) -> Piece {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for &t in &triangles {
            for c in uvs[t] {
                for k in 0..2 {
                    lo[k] = lo[k].min(c[k]);
                    hi[k] = hi[k].max(c[k]);
                }
            }
        }
        let cells = (16, 64);
        let mut grid = vec![Vec::new(); cells.0 * cells.1];
        let cell = |x: f64, k: usize, n: usize| (((x - lo[k]) / (hi[k] - lo[k]).max(1e-12)) * n as f64).floor().clamp(0.0, n as f64 - 1.0) as usize;
        let mut weight = 0.0;
        for &t in &triangles {
            let c = uvs[t];
            let (x0, x1) = (cell(c.iter().map(|q| q[0]).fold(f64::INFINITY, f64::min), 0, cells.0), cell(c.iter().map(|q| q[0]).fold(f64::NEG_INFINITY, f64::max), 0, cells.0));
            let (y0, y1) = (cell(c.iter().map(|q| q[1]).fold(f64::INFINITY, f64::min), 1, cells.1), cell(c.iter().map(|q| q[1]).fold(f64::NEG_INFINITY, f64::max), 1, cells.1));
            for x in x0..=x1 {
                for y in y0..=y1 {
                    grid[y * cells.0 + x].push(t);
                }
            }
            let area = areas[t];
            let mut opaque = 0.0;
            for b in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [0.5, 0.5, 0.0], [0.0, 0.5, 0.5], [0.5, 0.0, 0.5], [1.0 / 3.0; 3]] {
                let uv = [0, 1].map(|k| (0..3).map(|j| b[j] * c[j][k]).sum());
                if texture.texel(uv)[3] >= OPAQUE {
                    opaque += 1.0 / 7.0;
                }
            }
            weight += area * opaque;
        }
        Piece { lo, hi, cells, grid, weight }
    }

    fn uv_box(&self) -> ([f64; 2], [f64; 2]) {
        let r = |x: [f64; 2]| x.map(|v| (v * 1000.0).round() / 1000.0);
        (r(self.lo), r(self.hi))
    }

    fn locate(&self, uv: [f64; 2], uvs: &[[[f64; 2]; 3]]) -> Option<(usize, [f64; 3])> {
        if uv[0] < self.lo[0] || uv[0] > self.hi[0] || uv[1] < self.lo[1] || uv[1] > self.hi[1] {
            return None;
        }
        let cell = |x: f64, k: usize, n: usize| (((x - self.lo[k]) / (self.hi[k] - self.lo[k]).max(1e-12)) * n as f64).floor().clamp(0.0, n as f64 - 1.0) as usize;
        let at = cell(uv[1], 1, self.cells.1) * self.cells.0 + cell(uv[0], 0, self.cells.0);
        self.grid[at].iter().find_map(|&t| barycentric(uv, &uvs[t]).map(|b| (t, b)))
    }
}

fn arc_resample(points: &[Point], colours: &[[f32; 3]], n: usize) -> Strand {
    let mut along = vec![0.0];
    for w in points.windows(2) {
        along.push(along.last().unwrap() + dist(w[0], w[1]));
    }
    let total = *along.last().unwrap();
    let mut out = Strand { points: Vec::with_capacity(n), colours: Vec::with_capacity(n) };
    let mut k = 0;
    for i in 0..n {
        let s = total * i as f64 / (n - 1) as f64;
        while k + 2 < along.len() && along[k + 1] < s {
            k += 1;
        }
        let f = if along[k + 1] > along[k] { ((s - along[k]) / (along[k + 1] - along[k])).clamp(0.0, 1.0) } else { 0.0 };
        out.points.push(std::array::from_fn(|j| points[k][j] + f * (points[k + 1][j] - points[k][j])));
        out.colours.push(std::array::from_fn(|j| colours[k][j] + f as f32 * (colours[k + 1][j] - colours[k][j])));
    }
    out
}

fn areas(positions: &[Point], triangles: &[[u32; 3]]) -> Vec<f64> {
    triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|v| positions[v as usize]);
            let (e, f) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let n = [e[1] * f[2] - e[2] * f[1], e[2] * f[0] - e[0] * f[2], e[0] * f[1] - e[1] * f[0]];
            (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.0
        })
        .collect()
}

fn linear(c: u8) -> f32 {
    let s = c as f32 / 255.0;
    if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
}

fn dist(a: Point, b: Point) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

pub fn opaque_area(positions: &[Point], triangles: &[[u32; 3]], uvs: &[[gf_geom::Uv; 3]], texture: &Texture) -> f64 {
    let uvs: Vec<[[f64; 2]; 3]> = uvs.iter().map(|t| t.map(|c| [c[0] as f64, c[1] as f64])).collect();
    let areas = areas(positions, triangles);
    pieces(positions.len(), triangles).into_iter().map(|t| Piece::new(t, &uvs, &areas, texture).weight).sum()
}

#[allow(clippy::too_many_arguments)]
pub fn from_cards(positions: &[Point], triangles: &[[u32; 3]], uvs: &[[gf_geom::Uv; 3]], texture: &Texture, count: usize, points: usize, seed: u64, rank: &[f64]) -> (Vec<Strand>, Value) {
    let uvs: Vec<[[f64; 2]; 3]> = uvs.iter().map(|t| t.map(|c| [c[0] as f64, c[1] as f64])).collect();
    let areas = areas(positions, triangles);
    let mut folded = 0usize;
    let pieces: Vec<Piece> = pieces(positions.len(), triangles)
        .into_iter()
        .map(|t| {
            let signs: Vec<bool> = t.iter().map(|&i| uv_signed(uvs[i]) > 0.0).collect();
            let up = signs.iter().filter(|&&s| s).count();
            let fold = up.min(signs.len() - up) as f64 > FOLDED_SHARE * signs.len() as f64;
            let mut piece = Piece::new(t, &uvs, &areas, texture);
            if fold {
                piece.weight = 0.0;
                folded += 1;
            }
            piece
        })
        .collect();
    let total: f64 = pieces.iter().map(|p| p.weight).sum();
    let mut rng = Rng(seed);
    let mut strands = Vec::new();
    let (mut tries, mut short, mut flipped) = (0usize, 0usize, 0usize);
    if total <= 0.0 {
        return (strands, json!({"pieces": pieces.len(), "strands": 0, "reason": "no opaque texel on any card"}));
    }
    let mut quota: Vec<usize> = pieces.iter().map(|p| (p.weight / total * count as f64).floor() as usize).collect();
    let mut left = count - quota.iter().sum::<usize>();
    let mut order: Vec<usize> = (0..pieces.len()).collect();
    order.sort_by(|&a, &b| {
        let fa = pieces[a].weight / total * count as f64 - quota[a] as f64;
        let fb = pieces[b].weight / total * count as f64 - quota[b] as f64;
        fb.total_cmp(&fa)
    });
    for &k in &order {
        if left == 0 {
            break;
        }
        quota[k] += 1;
        left -= 1;
    }
    type Run = (Vec<Point>, Vec<[f32; 3]>, Vec<f64>);
    let length = |r: &[Point]| r.windows(2).map(|w| dist(w[0], w[1])).sum::<f64>();
    let trace = |piece: &Piece, fixed: usize, at: f64| -> Vec<Run> {
        let along = 1 - fixed;
        let mut runs: Vec<Run> = vec![(Vec::new(), Vec::new(), Vec::new())];
        for s in 0..=TRACE_STEPS {
            let mut uv = [0.0; 2];
            uv[fixed] = at;
            uv[along] = piece.lo[along] + (piece.hi[along] - piece.lo[along]) * s as f64 / TRACE_STEPS as f64;
            let hit = piece.locate(uv, &uvs).and_then(|(t, b)| {
                let texel = texture.texel(uv);
                (texel[3] >= OPAQUE).then(|| {
                    let p: Point = std::array::from_fn(|k| (0..3).map(|j| b[j] * positions[triangles[t][j] as usize][k]).sum());
                    let r: f64 = (0..3).map(|j| b[j] * rank[triangles[t][j] as usize]).sum();
                    (p, [texel[0], texel[1], texel[2]].map(linear), r)
                })
            });
            match hit {
                Some((p, c, r)) => {
                    let run = runs.last_mut().unwrap();
                    run.0.push(p);
                    run.1.push(c);
                    run.2.push(r);
                }
                None => {
                    if !runs.last().unwrap().0.is_empty() {
                        runs.push((Vec::new(), Vec::new(), Vec::new()));
                    }
                }
            }
        }
        runs.into_iter().filter(|r| r.0.len() >= 3 && length(&r.0) >= MIN_LENGTH_M).collect()
    };
    let mut along_u = 0;
    let mut unfilled = Vec::new();
    for (piece, want) in pieces.iter().zip(&quota) {
        if *want == 0 {
            continue;
        }
        let score = |fixed: usize| -> f64 {
            (0..PROBES).map(|k| trace(piece, fixed, piece.lo[fixed] + (piece.hi[fixed] - piece.lo[fixed]) * (k as f64 + 0.5) / PROBES as f64).iter().map(|r| length(&r.0)).fold(0.0, f64::max)).sum::<f64>()
        };
        let fixed = if score(1) > score(0) { 1 } else { 0 };
        along_u += fixed;
        let mut made = 0;
        let mut attempts = 0;
        while made < *want && attempts < want * 8 + 8 {
            attempts += 1;
            tries += 1;
            let at = piece.lo[fixed] + (piece.hi[fixed] - piece.lo[fixed]) * rng.next();
            let runs = trace(piece, fixed, at);
            if runs.is_empty() {
                short += 1;
                continue;
            }
            for (mut ps, mut cs, rs) in runs {
                if made == *want {
                    break;
                }
                if rs[rs.len() - 1] < rs[0] {
                    ps.reverse();
                    cs.reverse();
                    flipped += 1;
                }
                strands.push(arc_resample(&ps, &cs, points));
                made += 1;
            }
        }
        if made < *want {
            let (lo, hi) = piece.uv_box();
            unfilled.push(json!({"wanted": want, "made": made, "traced_along": if fixed == 1 { "u" } else { "v" }, "uv_lo": lo, "uv_hi": hi}));
        }
    }
    let lengths: Vec<f64> = strands.iter().map(|s| s.points.windows(2).map(|w| dist(w[0], w[1])).sum()).collect();
    let mean = if lengths.is_empty() { 0.0 } else { lengths.iter().sum::<f64>() / lengths.len() as f64 };
    let longest = lengths.iter().copied().fold(0.0, f64::max);
    let report = json!({"pieces": pieces.len(), "folded_pieces_skipped": folded, "strands": strands.len(), "asked": count, "traced": tries, "too_short_or_empty": short,
                        "rooted_at_the_far_uv_end": flipped, "pieces_traced_along_u": along_u, "pieces_short_of_their_share": unfilled, "points_per_strand": points,
                        "mean_length_mm": (mean * 1e4).round() / 10.0, "longest_mm": (longest * 1e4).round() / 10.0});
    (strands, report)
}

pub fn rank_on(surface: &super::wearable::Surface, positions: &[Point], triangles: &[[u32; 3]]) -> (Vec<f64>, usize) {
    let seed: Vec<bool> = positions.iter().map(|&p| surface.nearest(p, 0.1).is_some_and(|j| super::wearable::pin_weight(surface.signed(p, j)) >= super::wearable::PINNED)).collect();
    let roots = seed.iter().filter(|&&s| s).count();
    (super::wearable::mesh_distances(positions, triangles, &seed).0, roots)
}

pub fn ribbons(strands: &[Strand], width: f64, centre: Point) -> (Vec<Point>, Vec<Point>, Vec<[u32; 3]>) {
    let (mut positions, mut normals, mut triangles) = (Vec::new(), Vec::new(), Vec::new());
    let sub = |a: Point, b: Point| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: Point, b: Point| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let unit = |a: Point| {
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        if l < 1e-12 { a } else { a.map(|x| x / l) }
    };
    for s in strands {
        let n = s.points.len();
        if n < 2 {
            continue;
        }
        let first = positions.len() as u32;
        for i in 0..n {
            let tangent = unit(sub(s.points[(i + 1).min(n - 1)], s.points[i.saturating_sub(1)]));
            let side = unit(cross(tangent, sub(s.points[i], centre)));
            let normal = unit(cross(side, tangent));
            let half = width / 2.0 * (1.0 - 0.7 * i as f64 / (n - 1) as f64);
            for k in [-1.0, 1.0] {
                positions.push(std::array::from_fn(|j| s.points[i][j] + k * half * side[j]));
                normals.push(normal);
            }
        }
        for i in 0..n as u32 - 1 {
            let (a, b, c, d) = (first + 2 * i, first + 2 * i + 1, first + 2 * i + 2, first + 2 * i + 3);
            triangles.push([a, b, c]);
            triangles.push([b, d, c]);
        }
    }
    (positions, normals, triangles)
}

fn num(x: f64) -> String {
    let s = format!("{x:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

pub fn usda(groups: &[(String, Vec<Strand>)], root: Option<&gf_io::gltf::Root>, generator: &str) -> String {
    let mut s = String::new();
    s.push_str("#usda 1.0\n(\n    defaultPrim = \"Character\"\n    metersPerUnit = 1\n    upAxis = \"Y\"\n");
    s.push_str(&format!("    doc = \"{generator}: strand hair traced from hair cards\"\n)\n\n"));
    s.push_str("def Xform \"Character\"\n{\n");
    if let Some(r) = root {
        let t = r.translation;
        let q = r.rotation;
        s.push_str(&format!("    float3 xformOp:translate = ({}, {}, {})\n", num(t[0]), num(t[1]), num(t[2])));
        s.push_str(&format!("    quatf xformOp:orient = ({}, {}, {}, {})\n", num(q[3]), num(q[0]), num(q[1]), num(q[2])));
        s.push_str(&format!("    float3 xformOp:scale = ({0}, {0}, {0})\n", num(r.scale)));
        s.push_str("    uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:orient\", \"xformOp:scale\"]\n");
    }
    for (name, strands) in groups {
        let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
        let safe = if safe.starts_with(|c: char| c.is_ascii_digit()) { format!("_{safe}") } else { safe };
        s.push_str(&format!("\n    def BasisCurves \"{safe}\"\n    {{\n        uniform token type = \"linear\"\n        uniform token wrap = \"nonperiodic\"\n"));
        let counts: Vec<String> = strands.iter().map(|st| st.points.len().to_string()).collect();
        s.push_str(&format!("        int[] curveVertexCounts = [{}]\n", counts.join(", ")));
        let pts: Vec<String> = strands.iter().flat_map(|st| st.points.iter().map(|p| {
            let g = to_gltf(*p);
            format!("({}, {}, {})", num(g[0] as f64), num(g[1] as f64), num(g[2] as f64))
        })).collect();
        s.push_str(&format!("        point3f[] points = [{}]\n", pts.join(", ")));
        let widths: Vec<String> = strands.iter().flat_map(|st| {
            let n = st.points.len();
            (0..n).map(move |i| num(ROOT_WIDTH_M + (TIP_WIDTH_M - ROOT_WIDTH_M) * i as f64 / (n - 1).max(1) as f64))
        }).collect();
        s.push_str(&format!("        float[] widths = [{}] (\n            interpolation = \"vertex\"\n        )\n", widths.join(", ")));
        let colours: Vec<String> = strands.iter().flat_map(|st| st.colours.iter().map(|c| format!("({}, {}, {})", num(c[0] as f64), num(c[1] as f64), num(c[2] as f64)))).collect();
        s.push_str(&format!("        color3f[] primvars:displayColor = [{}] (\n            interpolation = \"vertex\"\n        )\n    }}\n", colours.join(", ")));
    }
    s.push_str("}\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(n: usize) -> (Vec<Point>, Vec<[u32; 3]>, Vec<[gf_geom::Uv; 3]>) {
        let mut p = Vec::new();
        for i in 0..=n {
            let y = -(i as f64) * 0.02;
            p.push([0.0, 0.0, y]);
            p.push([0.01, 0.0, y]);
        }
        let mut tris = Vec::new();
        let mut uvs = Vec::new();
        let uv = |k: usize| -> gf_geom::Uv { [(k % 2) as f32, (k / 2) as f32 / n as f32] };
        for i in 0..n {
            let (a, b, c, d) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
            for t in [[a, b, c], [b, d, c]] {
                tris.push(t.map(|v| v as u32));
                uvs.push(t.map(uv));
            }
        }
        (p, tris, uvs)
    }

    fn rank(p: &[Point], from: f64) -> Vec<f64> {
        p.iter().map(|q| (q[2] + from).abs()).collect()
    }

    #[test]
    fn a_card_becomes_strands_along_v_rooted_at_the_body() {
        let (p, tris, uvs) = strip(10);
        let mut rgba = vec![255u8; 4 * 8 * 64];
        for y in 48..64 {
            for x in 0..8 {
                rgba[(y * 8 + x) * 4 + 3] = 0;
            }
        }
        let tex = Texture { width: 8, height: 64, rgba: &rgba };
        let (strands, report) = from_cards(&p, &tris, &uvs, &tex, 20, 8, 7, &rank(&p, 0.0));
        assert_eq!(strands.len(), 20, "{report}");
        for s in &strands {
            assert_eq!(s.points.len(), 8);
            assert!(s.points[0][2] > -0.001, "rooted at the top: {:?}", s.points[0]);
            let tip = s.points[7][2];
            assert!((tip + 0.15).abs() < 0.004, "ends where the texture turns transparent (v = 0.75 of 0.2 m): {tip}");
            assert!(s.points.iter().all(|q| (0.0..=0.01).contains(&q[0]) && q[1] == 0.0), "on the card");
        }
        let (same, _) = from_cards(&p, &tris, &uvs, &tex, 20, 8, 7, &rank(&p, 0.0));
        assert_eq!(same.iter().map(|s| s.points[3]).collect::<Vec<_>>(), strands.iter().map(|s| s.points[3]).collect::<Vec<_>>(), "seeded");
    }

    #[test]
    fn a_piece_whose_uvs_fold_over_is_not_a_card_and_grows_no_strands() {
        let (p, tris, uvs) = strip(10);
        let n = p.len() as u32;
        let mut all = p.clone();
        all.extend(p.iter().map(|q| [q[0] + 1.0, q[1], q[2]]));
        let mut all_tris = tris.clone();
        all_tris.extend(tris.iter().map(|t| t.map(|v| v + n)));
        let mut all_uvs = uvs.clone();
        all_uvs.extend(uvs.iter().enumerate().map(|(i, t)| if i % 2 == 1 { t.map(|c| [1.0 - c[0], c[1]]) } else { *t }));
        let rgba = vec![255u8; 4 * 8 * 64];
        let tex = Texture { width: 8, height: 64, rgba: &rgba };
        let (strands, report) = from_cards(&all, &all_tris, &all_uvs, &tex, 20, 6, 3, &rank(&all, 0.0));
        assert_eq!(report["folded_pieces_skipped"], 1, "{report}");
        assert_eq!(strands.len(), 20, "{report}");
        assert!(strands.iter().all(|s| s.points.iter().all(|q| q[0] < 0.5)), "every strand on the unfolded card");
    }

    #[test]
    fn the_root_follows_the_body_not_the_uv_direction() {
        let (p, tris, uvs) = strip(10);
        let rgba = vec![255u8; 4 * 8 * 64];
        let tex = Texture { width: 8, height: 64, rgba: &rgba };
        let (strands, report) = from_cards(&p, &tris, &uvs, &tex, 5, 6, 3, &rank(&p, 0.2));
        assert_eq!(report["rooted_at_the_far_uv_end"], 5);
        assert!(strands.iter().all(|s| s.points[0][2] < -0.199));
    }

    #[test]
    fn every_opaque_run_of_a_column_becomes_a_strand() {
        let (p, tris, uvs) = strip(10);
        let mut rgba = vec![255u8; 4 * 8 * 64];
        for y in (20..28).chain(48..64) {
            for x in 0..8 {
                rgba[(y * 8 + x) * 4 + 3] = 0;
            }
        }
        let tex = Texture { width: 8, height: 64, rgba: &rgba };
        let (strands, report) = from_cards(&p, &tris, &uvs, &tex, 40, 6, 9, &rank(&p, 0.0));
        assert_eq!(strands.len(), 40, "{report}");
        let upper = strands.iter().filter(|s| s.points[0][2] > -0.001).count();
        let lower = strands.iter().filter(|s| (s.points[0][2] + 0.0875).abs() < 0.004).count();
        assert!(upper > 10 && lower > 10, "both opaque stretches give strands: {upper} upper, {lower} lower");
        assert_eq!(upper + lower, 40);
    }

    #[test]
    fn a_card_painted_along_u_is_traced_along_u() {
        let (p, tris, uvs) = strip(10);
        let swapped: Vec<[gf_geom::Uv; 3]> = uvs.iter().map(|t| t.map(|c| [c[1], c[0]])).collect();
        let rgba = vec![255u8; 4 * 64 * 8];
        let tex = Texture { width: 64, height: 8, rgba: &rgba };
        let (strands, report) = from_cards(&p, &tris, &swapped, &tex, 12, 6, 5, &rank(&p, 0.0));
        assert_eq!(report["pieces_traced_along_u"], 1, "{report}");
        assert_eq!(strands.len(), 12);
        for s in &strands {
            let span = s.points[0][2] - s.points[5][2];
            assert!((span - 0.2).abs() < 0.002, "runs the card's length, root to tip: {span}");
        }
    }

    #[test]
    fn a_transparent_card_gives_no_strands() {
        let (p, tris, uvs) = strip(4);
        let rgba = vec![0u8; 4 * 8 * 8];
        let tex = Texture { width: 8, height: 8, rgba: &rgba };
        let (strands, report) = from_cards(&p, &tris, &uvs, &tex, 10, 6, 1, &rank(&p, 0.0));
        assert!(strands.is_empty());
        assert_eq!(report["strands"], 0);
    }

    #[test]
    fn usda_counts_match_and_the_root_transform_is_written() {
        let s = Strand { points: vec![[0.0, 0.0, 0.0], [0.0, 0.0, -0.1], [0.0, 0.0, -0.2]], colours: vec![[1.0, 0.5, 0.0]; 3] };
        let root = gf_io::gltf::Root { name: "Character".into(), translation: [0.0, 0.1, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: 1.1 };
        let text = usda(&[("pony tail".into(), vec![s])], Some(&root), "test");
        assert!(text.contains("def BasisCurves \"pony_tail\""));
        assert!(text.contains("int[] curveVertexCounts = [3]"));
        assert!(text.contains("point3f[] points = [(0, 0, 0), (0, -0.1, 0), (0, -0.2, 0)]"), "{text}");
        assert!(text.contains("quatf xformOp:orient = (1, 0, 0, 0)"));
        assert!(text.contains("xformOp:scale = (1.1, 1.1, 1.1)"));
    }

    #[test]
    fn settings_read_and_refuse() {
        let o = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Options>();
        assert!(settings(&o(&[])).unwrap().is_none());
        assert!(settings(&o(&[("strands-count", "10")])).is_err());
        assert!(settings(&o(&[("strands", "x.abc")])).is_err());
        let s = settings(&o(&[("strands", "x.usda"), ("strands-count", "500"), ("strands-points", "8")])).unwrap().unwrap();
        assert_eq!((s.count, s.points, s.seed), (500, 8, 1));
        assert!(settings(&o(&[("strands", "x.usda"), ("strands-points", "2")])).is_err());
    }
}
