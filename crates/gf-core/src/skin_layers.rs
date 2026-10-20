use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::hash;
use crate::skin_map::{NONE, TexelMap};
use crate::skin_regions::{self, Regions};
use crate::skin_texture::{SkinLook, SkinTextures, cells, fbm3, smoothstep, to_linear, to_srgb};
use gf_geom::Uv;

type Point = [f64; 3];

pub const EXTENSION: &str = "GAZEFORGE_skin";
pub const DEFAULT_SIZE: usize = 2048;
pub const SIZE_RANGE: [usize; 2] = [256, 8192];
pub const MAX_LIVE: usize = 16;
pub const LANDMARKS: [usize; 20] = [1, 10, 17, 33, 50, 61, 67, 98, 105, 145, 152, 234, 263, 280, 291, 297, 327, 334, 374, 454];
pub const STYLES: [(&str, &[&str]); 3] = [("grime", &["dust", "sand", "dirt", "mud"]), ("blood", &["drips", "splatter"]), ("paint", &["eye-band", "cheek-stripes", "chin"])];

fn default_size() -> usize {
    DEFAULT_SIZE
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinSpec {
    pub look: SkinLook,
    pub landmarks: String,
    #[serde(default = "default_size")]
    pub size: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<SkinLayer>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinLayer {
    pub name: String,
    pub kind: String,
    pub style: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<bool>,
    pub amount: f64,
    #[serde(default)]
    pub variant: f64,
    #[serde(default)]
    pub seed: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<[f64; 3]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Response {
    pub colour: [f32; 3],
    pub opacity: f32,
    pub roughness: f32,
    pub spread: f32,
    pub variant: Option<(&'static str, [f32; 3], f32)>,
}

fn round4(x: f64) -> f64 {
    let r = (x * 1e4).round() / 1e4;
    if r == 0.0 { 0.0 } else { r }
}

const SHADER_WORDS: [&str; 74] = [
    "abs", "atan", "bool", "break", "bvec2", "bvec3", "bvec4", "case", "ceil", "clamp", "const", "continue", "cos", "cross", "default",
    "discard", "distance", "do", "dot", "else", "exp", "false", "flat", "float", "floor", "for", "fract", "highp", "if", "in", "inout",
    "int", "ivec2", "ivec3", "ivec4", "length", "log", "lowp", "mat2", "mat3", "mat4", "max", "mediump", "min", "mix", "mod",
    "normalize", "out", "pow", "precision", "return", "sampler2d", "sign", "sin", "smooth", "smoothstep", "sqrt", "step", "struct",
    "switch", "tan", "texture", "true", "uint", "uniform", "uvec2", "uvec3", "uvec4", "varying", "vec2", "vec3", "vec4", "void", "while",
];

fn identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !name.starts_with("gf_")
        && !SHADER_WORDS.contains(&name)
}

impl SkinLayer {
    pub fn is_live(&self) -> bool {
        self.live.unwrap_or(true)
    }

    pub fn response(&self) -> Option<Response> {
        let blood = Response { colour: [0.20, 0.008, 0.008], opacity: 0.95, roughness: 0.22, spread: 0.08, variant: Some(("dry", [0.045, 0.008, 0.006], 0.7)) };
        let mut r = match (self.kind.as_str(), self.style.as_str()) {
            ("grime", "dust") => Response { colour: [0.36, 0.31, 0.24], opacity: 0.35, roughness: 0.9, spread: 0.6, variant: None },
            ("grime", "sand") => Response { colour: [0.50, 0.40, 0.26], opacity: 0.8, roughness: 0.95, spread: 0.25, variant: None },
            ("grime", "dirt") => Response { colour: [0.13, 0.10, 0.075], opacity: 0.75, roughness: 0.85, spread: 0.35, variant: Some(("wet", [0.06, 0.045, 0.035], 0.35)) },
            ("grime", "mud") => Response { colour: [0.06, 0.042, 0.028], opacity: 0.95, roughness: 0.9, spread: 0.65, variant: Some(("wet", [0.045, 0.03, 0.02], 0.25)) },
            ("blood", "drips" | "splatter") => blood,
            ("paint", "eye-band" | "cheek-stripes" | "chin") => Response { colour: [0.012, 0.012, 0.012], opacity: 0.95, roughness: 0.55, spread: 0.06, variant: None },
            _ => return None,
        };
        if let Some(c) = self.colour {
            r.colour = c.map(|x| x as f32);
        }
        Some(r)
    }

    pub fn variant_parameter(&self) -> Option<String> {
        self.response().and_then(|r| r.variant).map(|(suffix, _, _)| format!("{}_{suffix}", self.name))
    }
}

impl SkinSpec {
    pub fn canonical(&self) -> SkinSpec {
        let layers = self
            .layers
            .iter()
            .map(|l| SkinLayer {
                live: Some(l.is_live()),
                amount: round4(l.amount),
                variant: round4(l.variant),
                colour: l.colour.map(|c| c.map(round4)),
                ..l.clone()
            })
            .collect();
        SkinSpec { layers, ..self.clone() }
    }

    pub fn live(&self) -> Vec<&SkinLayer> {
        self.layers.iter().filter(|l| l.is_live()).collect()
    }

    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Err(e) = hash::parse_reference(&self.landmarks) {
            out.push(format!("skin.landmarks: {e}"));
        }
        if !(SIZE_RANGE[0]..=SIZE_RANGE[1]).contains(&self.size) {
            out.push(format!("skin.size {} is outside [{}, {}]", self.size, SIZE_RANGE[0], SIZE_RANGE[1]));
        }
        if self.look.id.trim().is_empty() {
            out.push("skin.look.id is empty".into());
        }
        let look = serde_json::to_value(&self.look).unwrap_or(Value::Null);
        for (k, v) in look.as_object().into_iter().flatten() {
            if v.as_f64().is_some_and(|x| !x.is_finite()) || v.as_array().is_some_and(|a| a.iter().any(|x| x.as_f64().is_none_or(|x| !x.is_finite()))) {
                out.push(format!("skin.look.{k} is not finite"));
            }
        }
        let mut names: BTreeMap<String, usize> = BTreeMap::new();
        for (i, l) in self.layers.iter().enumerate() {
            let at = format!("skin.layers[{i}] ({})", l.name);
            if !identifier(&l.name) {
                out.push(format!("{at}: the name must be a lowercase identifier, [a-z][a-z0-9_]*, not a shading-language word and not gf_..."));
            }
            match STYLES.iter().find(|(k, _)| *k == l.kind) {
                None => out.push(format!("{at}: kind {:?} is not one of grime, blood, paint", l.kind)),
                Some((_, styles)) if !styles.contains(&l.style.as_str()) => out.push(format!("{at}: style {:?} is not one of {}", l.style, styles.join(", "))),
                _ => {}
            }
            for (field, x) in [("amount", l.amount), ("variant", l.variant)] {
                if !(x.is_finite() && (0.0..=1.0).contains(&x)) {
                    out.push(format!("{at}: {field} {x} is outside [0, 1]"));
                }
            }
            if l.colour.is_some_and(|c| c.iter().any(|x| !(x.is_finite() && (0.0..=1.0).contains(x)))) {
                out.push(format!("{at}: colour must be three linear values in [0, 1]"));
            }
            if l.variant != 0.0 && l.response().is_some_and(|r| r.variant.is_none()) {
                out.push(format!("{at}: {} {} has no variant state", l.kind, l.style));
            }
            for p in std::iter::once(l.name.clone()).chain(l.variant_parameter()) {
                if let Some(j) = names.insert(p.clone(), i) {
                    out.push(format!("{at}: parameter {p:?} is also used by skin.layers[{j}]"));
                }
            }
        }
        if self.live().len() > MAX_LIVE {
            out.push(format!("skin has {} live layers, at most {MAX_LIVE} fit in four masks", self.live().len()));
        }
        out
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn len3(a: [f32; 3]) -> f32 {
    dot3(a, a).sqrt()
}

fn segment_distance(p: [f32; 3], a: [f32; 3], b: [f32; 3]) -> (f32, f32) {
    let ab = sub3(b, a);
    let l2 = dot3(ab, ab);
    let t = if l2 > 0.0 { (dot3(sub3(p, a), ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
    (len3(sub3(p, [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]])), t)
}

pub struct Surface<'a> {
    pub map: &'a TexelMap,
    pub positions: &'a [Point],
    pub normals: &'a [Point],
    pub triangles: &'a [[u32; 3]],
    pub uvs: &'a [[Uv; 3]],
    pub regions: &'a Regions,
    pub landmarks: &'a BTreeMap<usize, Point>,
}

pub struct Texel {
    mm: [f32; 3],
    n: [f32; 3],
    mouth: f32,
    eyelids: f32,
    lips: f32,
    concave: f32,
    area: f32,
}

const MOUTH: usize = 1;
const EYELIDS: usize = 9;
const LIPS: usize = 0;

fn concavity(positions: &[Point], normals: &[Point], triangles: &[[u32; 3]]) -> Vec<f32> {
    let nb = skin_regions::neighbours(positions.len(), triangles);
    let raw: Vec<f32> = (0..positions.len())
        .map(|v| {
            if nb[v].is_empty() {
                return 0.0;
            }
            let p = positions[v];
            let k = nb[v].len() as f64;
            let mean = [0, 1, 2].map(|j| nb[v].iter().map(|&u| positions[u as usize][j]).sum::<f64>() / k);
            let edge2 = nb[v].iter().map(|&u| (0..3).map(|j| (positions[u as usize][j] - p[j]).powi(2)).sum::<f64>()).sum::<f64>() / k;
            let lap = [0, 1, 2].map(|j| mean[j] - p[j]);
            ((lap[0] * normals[v][0] + lap[1] * normals[v][1] + lap[2] * normals[v][2]) / edge2.max(1e-12) * 1e-3) as f32
        })
        .collect();
    let mut smooth = raw.clone();
    for _ in 0..2 {
        smooth = (0..smooth.len())
            .map(|v| (smooth[v] + nb[v].iter().map(|&u| smooth[u as usize]).sum::<f32>()) / (1 + nb[v].len()) as f32)
            .collect();
    }
    let mut positive: Vec<f32> = smooth.iter().copied().filter(|&c| c > 0.0).collect();
    positive.sort_by(f32::total_cmp);
    let p90 = positive.get(positive.len() * 9 / 10).copied().unwrap_or(1.0).max(1e-6);
    smooth.iter().map(|&c| (c / p90).clamp(0.0, 1.0)).collect()
}

pub fn texels(s: &Surface) -> Vec<Option<Texel>> {
    let pos: Vec<[f32; 3]> = s.positions.iter().map(|p| p.map(|x| (x * 1000.0) as f32)).collect();
    let nrm: Vec<[f32; 3]> = s.normals.iter().map(|p| p.map(|x| x as f32)).collect();
    let concave = concavity(s.positions, s.normals, s.triangles);
    let size = s.map.size as f32;
    (0..s.map.size * s.map.size)
        .map(|k| {
            let t = s.map.triangle[k];
            if t == NONE {
                return None;
            }
            let tri = s.triangles[t as usize];
            let [b0, b1] = s.map.bary[k];
            let b = [b0, b1, 1.0 - b0 - b1];
            let lerp = |v: &[[f32; 3]]| [0, 1, 2].map(|j| (0..3).map(|c| b[c] * v[tri[c] as usize][j]).sum::<f32>());
            let w = |r: usize| (0..3).map(|c| b[c] * s.regions.weights[tri[c] as usize][r]).sum::<f32>().clamp(0.0, 1.0);
            let n = lerp(&nrm);
            let nl = len3(n).max(1e-9);
            let p = tri.map(|i| pos[i as usize]);
            let area3 = {
                let (e1, e2) = (sub3(p[1], p[0]), sub3(p[2], p[0]));
                len3([e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]])
            };
            let u = s.uvs[t as usize];
            let area2 = ((u[1][0] - u[0][0]) * (u[2][1] - u[0][1]) - (u[2][0] - u[0][0]) * (u[1][1] - u[0][1])).abs() * size * size;
            Some(Texel {
                mm: lerp(&pos),
                n: n.map(|x| x / nl),
                mouth: w(MOUTH),
                eyelids: w(EYELIDS),
                lips: w(LIPS),
                concave: (0..3).map(|c| b[c] * concave[tri[c] as usize]).sum(),
                area: if area2 > 1e-9 { area3 / area2 } else { 0.0 },
            })
        })
        .collect()
}

struct Marks {
    nose_tip: [f32; 3],
    chin: [f32; 3],
    ear_y: f32,
    face_x: f32,
}

fn lm(s: &Surface, i: usize) -> Result<[f32; 3], String> {
    s.landmarks.get(&i).map(|p| p.map(|x| (x * 1000.0) as f32)).ok_or_else(|| format!("landmark {i} is missing from the table"))
}

fn marks(s: &Surface) -> Result<Marks, String> {
    let (r, l) = (lm(s, 234)?, lm(s, 454)?);
    Ok(Marks { nose_tip: lm(s, 1)?, chin: lm(s, 152)?, ear_y: 0.5 * (r[1] + l[1]), face_x: 0.5 * (r[0].abs() + l[0].abs()) })
}

fn ranked(texels: &[Option<Texel>], score: impl Fn(&Texel) -> Option<f32>) -> Vec<f32> {
    let scored: Vec<Option<f32>> = texels.iter().map(|t| t.as_ref().and_then(&score)).collect();
    let mut order: Vec<usize> = (0..scored.len()).filter(|&k| scored[k].is_some()).collect();
    order.sort_by(|&a, &b| scored[a].unwrap_or(0.0).total_cmp(&scored[b].unwrap_or(0.0)));
    let total: f64 = order.iter().map(|&k| texels[k].as_ref().map_or(0.0, |t| t.area as f64)).sum::<f64>().max(1e-12);
    let mut out = vec![0.0f32; scored.len()];
    let mut cum = 0.0f64;
    for &k in &order {
        cum += texels[k].as_ref().map_or(0.0, |t| t.area as f64);
        out[k] = (cum / total) as f32;
    }
    out
}

fn grime(texels: &[Option<Texel>], m: &Marks, style: &str, seed: u32) -> Vec<f32> {
    let (scale, grain, w_noise, w_crease, w_up, w_low, reach) = match style {
        "dust" => (4.0, 0.0, 0.5, 0.25, 0.35, 0.0, 1.0),
        "sand" => (6.0, 0.35, 0.45, 0.2, 0.1, 0.35, 0.6),
        "dirt" => (14.0, 0.0, 0.5, 0.35, 0.0, 0.4, 0.7),
        _ => (22.0, 0.0, 0.5, 0.2, 0.0, 0.8, 0.55),
    };
    let rank = ranked(texels, |t| {
        if t.mouth > 0.5 {
            return None;
        }
        let p = t.mm;
        let noise = fbm3(p.map(|x| x / scale), seed.wrapping_add(11), 4);
        let speckle = if grain > 0.0 {
            let (d, v) = cells(p.map(|x| x / 1.2), seed.wrapping_add(12), 0.5);
            (1.0 - smoothstep(0.1, 0.35, d)) * (0.5 + 0.5 * v)
        } else {
            0.0
        };
        let low = 1.0 - smoothstep(m.chin[2], m.nose_tip[2], p[2]);
        let shield = 1.0 - 0.5 * t.eyelids - 0.3 * t.lips;
        Some((w_noise * noise + grain * speckle + w_crease * t.concave + w_up * t.n[2].max(0.0) + w_low * low) * shield)
    });
    rank.into_iter().map(|r| ((r - (1.0 - reach)) / reach).max(0.0)).collect()
}

fn snap(s: &Surface, p: [f32; 3]) -> usize {
    (0..s.positions.len())
        .min_by(|&a, &b| {
            let d = |i: usize| len3(sub3(s.positions[i].map(|x| (x * 1000.0) as f32), p));
            d(a).total_cmp(&d(b))
        })
        .unwrap_or(0)
}

fn descend(s: &Surface, nb: &[Vec<u32>], start: usize, length_mm: f32, rng: &mut Rng) -> Vec<[f32; 3]> {
    let mm = |i: usize| s.positions[i].map(|x| (x * 1000.0) as f32);
    let mut path = vec![mm(start)];
    let (mut v, mut run) = (start, 0.0f32);
    while run < length_mm {
        let here = mm(v);
        let next = nb[v]
            .iter()
            .map(|&u| u as usize)
            .filter(|&u| mm(u)[2] < here[2])
            .map(|u| {
                let d = sub3(mm(u), here);
                let slope = d[2] / len3(d).max(1e-6);
                (u, slope + 0.35 * (rng.next() - 0.5))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let Some((u, _)) = next else { break };
        run += len3(sub3(mm(u), here));
        path.push(mm(u));
        v = u;
    }
    path
}

fn blood(s: &Surface, texels: &[Option<Texel>], style: &str, seed: u32) -> Result<Vec<f32>, String> {
    let mut rng = Rng(seed as u64 ^ 0xB100D);
    let sites: Vec<[f32; 3]> = [67, 297, 105, 334, 98, 327, 61, 291, 50, 280, 10].iter().map(|&i| lm(s, i)).collect::<Result<_, _>>()?;
    let nb = skin_regions::neighbours(s.positions.len(), s.triangles);
    let mut out = vec![0.0f32; texels.len()];
    let mut stamp = |shape: &dyn Fn([f32; 3]) -> f32, lo: [f32; 3], hi: [f32; 3]| {
        for (k, t) in texels.iter().enumerate() {
            let Some(t) = t else { continue };
            if (0..3).any(|j| t.mm[j] < lo[j] || t.mm[j] > hi[j]) {
                continue;
            }
            out[k] = out[k].max(shape(t.mm));
        }
    };
    if style == "drips" {
        let count = 2 + (rng.next() * 2.0) as usize;
        for _ in 0..count {
            let site = sites[(rng.next() * sites.len() as f32) as usize % sites.len()];
            let start = snap(s, [site[0] + rng.range(-4.0, 4.0), site[1], site[2] + rng.range(-3.0, 3.0)]);
            let path = descend(s, &nb, start, rng.range(40.0, 110.0), &mut rng);
            let total: f32 = path.windows(2).map(|w| len3(sub3(w[1], w[0]))).sum::<f32>().max(1e-3);
            let width = rng.range(1.6, 2.6);
            let wobble = rng.next() * 1000.0;
            let pad = width * 2.5 + 5.0;
            let lo = [0, 1, 2].map(|j| path.iter().map(|p| p[j]).fold(f32::INFINITY, f32::min) - pad);
            let hi = [0, 1, 2].map(|j| path.iter().map(|p| p[j]).fold(f32::NEG_INFINITY, f32::max) + pad);
            let path = &path;
            stamp(
                &|p| {
                    let pool = 1.0 - smoothstep(3.5, 5.0, len3(sub3(p, path[0])));
                    let mut best = 0.0f32;
                    let mut along = 0.0f32;
                    for w in path.windows(2) {
                        let seg = len3(sub3(w[1], w[0]));
                        let (d, f) = segment_distance(p, w[0], w[1]);
                        let t = ((along + f * seg) / total).clamp(0.0, 1.0);
                        let r = width * (1.0 - 0.45 * t) * (0.8 + 0.4 * fbm3([t * total / 8.0, wobble, 0.0], seed, 2)) * (1.0 + 0.7 * smoothstep(0.9, 1.0, t));
                        let edge = 1.0 - smoothstep(0.75 * r, r, d);
                        best = best.max(edge * (1.0 - 0.9 * t));
                        along += seg;
                    }
                    best.max(pool)
                },
                lo,
                hi,
            );
        }
    } else {
        let centre = sites[(rng.next() * sites.len() as f32) as usize % sites.len()];
        let hit = s.positions[snap(s, centre)].map(|x| (x * 1000.0) as f32);
        let near: Vec<usize> = (0..s.positions.len()).filter(|&i| len3(sub3(s.positions[i].map(|x| (x * 1000.0) as f32), hit)) < 50.0).collect();
        let mut drops: Vec<([f32; 3], f32, f32)> = vec![(hit, 4.0, 1.0)];
        for _ in 0..90 {
            if near.is_empty() {
                break;
            }
            let q = s.positions[near[(rng.next() * near.len() as f32) as usize % near.len()]].map(|x| (x * 1000.0) as f32);
            let d = len3(sub3(q, hit));
            if rng.next() < d / 60.0 {
                continue;
            }
            drops.push((q, rng.range(0.35, 1.0) * (2.2 - 1.6 * d / 50.0), (1.0 - 0.8 * d / 50.0) * rng.range(0.75, 1.0)));
        }
        let lo = hit.map(|x| x - 55.0);
        let hi = hit.map(|x| x + 55.0);
        stamp(
            &|p| {
                drops
                    .iter()
                    .map(|&(q, r, order)| {
                        let d = len3(sub3(p, q));
                        if d > r * 1.3 { 0.0 } else { (1.0 - smoothstep(0.8 * r, r, d)) * order }
                    })
                    .fold(0.0, f32::max)
            },
            lo,
            hi,
        );
    }
    Ok(out)
}

fn paint(s: &Surface, texels: &[Option<Texel>], m: &Marks, style: &str, seed: u32) -> Result<Vec<f32>, String> {
    let ragged = |p: [f32; 3], amp: f32| amp * (fbm3(p.map(|x| x / 3.5), seed.wrapping_add(21), 3) - 0.5) * 2.0;
    let strokes = |p: [f32; 3]| fbm3([p[0] / 14.0, p[1] / 14.0, p[2] / 0.9], seed.wrapping_add(22), 2);
    let front = |t: &Texel| t.mm[1] < m.ear_y && t.mouth < 0.5;
    let inside = |sd: f32| 1.0 - smoothstep(-0.3, 0.3, sd);
    let mut out = vec![0.0f32; texels.len()];
    match style {
        "eye-band" => {
            let brow = 0.5 * (lm(s, 105)?[2] + lm(s, 334)?[2]);
            let lid = 0.5 * (lm(s, 145)?[2] + lm(s, 374)?[2]);
            let outer = 0.5 * (lm(s, 33)?[0].abs() + lm(s, 263)?[0].abs());
            let (top, bottom, end) = (brow - 1.0, lid - 8.0, m.face_x - 3.0);
            let (mid0, half0) = (0.5 * (top + bottom), 0.5 * (top - bottom));
            let sway = |p: [f32; 3]| 2.0 * (fbm3([p[0] / 16.0, 0.0, p[2] / 16.0], seed.wrapping_add(23), 2) - 0.5) * 2.0;
            for (k, t) in texels.iter().enumerate() {
                let Some(t) = t.as_ref().filter(|t| front(t)) else { continue };
                let p = t.mm;
                let ax = p[0].abs();
                let bridge = 1.0 - 0.3 * (1.0 - smoothstep(4.0, 13.0, ax));
                let wing = 1.0 - smoothstep(outer - 2.0, end, ax);
                let half = half0 * bridge * wing.powf(0.6);
                let mid = mid0 + 0.35 * (ax - outer).max(0.0);
                let sd = ((p[2] - mid).abs() - half + ragged(p, 1.2) + sway(p)).max(ax - end);
                let dry = smoothstep(-3.0, 0.0, sd) * smoothstep(0.55, 0.7, strokes(p));
                let order = 1.0 - 0.45 * ax / end - 0.15 * (p[2] - mid).abs() / half0;
                out[k] = inside(sd) * (1.0 - dry) * order.clamp(0.1, 1.0);
            }
        }
        "cheek-stripes" => {
            let mouth = 0.5 * (lm(s, 61)?[2] + lm(s, 291)?[2]);
            let mut stripes = Vec::new();
            for (eye, side) in [(lm(s, 145)?, -1.0f32), (lm(s, 374)?, 1.0)] {
                let z0 = eye[2] - 6.0;
                let length = (z0 - mouth + 4.0).max(10.0);
                let dir = {
                    let d = [side * 0.25, -1.0];
                    let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
                    [d[0] / l, d[1] / l]
                };
                for i in 0..3 {
                    stripes.push(([eye[0] + side * (-6.0 + 7.0 * i as f32), z0], dir, length, i as f32));
                }
            }
            for (k, t) in texels.iter().enumerate() {
                let Some(t) = t.as_ref().filter(|t| front(t) && t.n[1] < 0.3) else { continue };
                let p = t.mm;
                let mut best = 0.0f32;
                for &(a, d, length, i) in &stripes {
                    let rel = [p[0] - a[0], p[2] - a[1]];
                    let along = (rel[0] * d[0] + rel[1] * d[1]).clamp(0.0, length);
                    let f = along / length;
                    let across = ((rel[0] - along * d[0]).powi(2) + (rel[1] - along * d[1]).powi(2)).sqrt();
                    let sd = across - 2.25 * (1.0 - 0.4 * f) + ragged(p, 0.6);
                    let dry = smoothstep(-1.5, 0.0, sd) * smoothstep(0.55, 0.7, strokes(p));
                    best = best.max(inside(sd) * (1.0 - dry) * (1.0 - 0.5 * f - 0.1 * i));
                }
                out[k] = best;
            }
        }
        _ => {
            let lip = lm(s, 17)?;
            let (top, bottom) = (lip[2] - 3.0, m.chin[2] - 2.0);
            for (k, t) in texels.iter().enumerate() {
                let Some(t) = t.as_ref().filter(|t| front(t) && t.n[1] < 0.2 && t.mm[1] < m.chin[1] + 20.0) else { continue };
                let p = t.mm;
                let mut best = 0.0f32;
                for (i, dx) in [-8.0f32, 0.0, 8.0].into_iter().enumerate() {
                    let sd = ((p[0] - lip[0] - dx).abs() - 2.0 + ragged(p, 0.5)).max(p[2] - top).max(bottom - p[2]);
                    let f = ((top - p[2]) / (top - bottom).max(1.0)).clamp(0.0, 1.0);
                    best = best.max(inside(sd) * (1.0 - 0.5 * f - 0.05 * i as f32));
                }
                out[k] = best;
            }
        }
    }
    Ok(out)
}

pub fn mask(s: &Surface, texels: &[Option<Texel>], layer: &SkinLayer) -> Result<Vec<u8>, String> {
    let m = marks(s)?;
    let values = match layer.kind.as_str() {
        "grime" => grime(texels, &m, &layer.style, layer.seed),
        "blood" => blood(s, texels, &layer.style, layer.seed)?,
        "paint" => paint(s, texels, &m, &layer.style, layer.seed)?,
        other => return Err(format!("no mask for kind {other:?}")),
    };
    Ok((0..texels.len())
        .map(|k| {
            let src = s.map.source[k];
            let v = if src == NONE { 0.0 } else { values[src as usize] };
            (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        })
        .collect())
}

pub fn coverage(m: u8, amount: f32, spread: f32) -> f32 {
    ((m as f32 / 255.0 + amount - 1.0) / spread).clamp(0.0, 1.0)
}

pub fn bake(tex: &mut SkinTextures, mask: &[u8], r: &Response, amount: f32, variant: f32) {
    let (colour, rough) = match r.variant {
        Some((_, c, ro)) => ([0, 1, 2].map(|j| r.colour[j] + (c[j] - r.colour[j]) * variant), r.roughness + (ro - r.roughness) * variant),
        None => (r.colour, r.roughness),
    };
    for (k, &m) in mask.iter().enumerate() {
        let a = coverage(m, amount, r.spread) * r.opacity;
        if a <= 0.0 {
            continue;
        }
        let px = &mut tex.albedo[k * 4..k * 4 + 3];
        for j in 0..3 {
            let lin = to_linear(px[j] as f32 / 255.0);
            px[j] = (to_srgb(lin + (colour[j] - lin) * a) * 255.0 + 0.5) as u8;
        }
        let g = &mut tex.orm[k * 3 + 1];
        let lin = *g as f32 / 255.0;
        *g = ((lin + (rough - lin) * a) * 255.0 + 0.5) as u8;
    }
}

pub fn pack(masks: &[Vec<u8>]) -> Vec<Vec<u8>> {
    masks
        .chunks(4)
        .map(|group| {
            let n = group[0].len();
            let mut rgba = vec![0u8; n * 4];
            for (c, m) in group.iter().enumerate() {
                for (k, &v) in m.iter().enumerate() {
                    rgba[k * 4 + c] = v;
                }
            }
            rgba
        })
        .collect()
}

pub fn extension(spec: &SkinSpec) -> Value {
    let rgb = |c: [f32; 3]| c.map(|x| round4(x as f64));
    let live: Vec<Value> = spec
        .live()
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let r = l.response()?;
            let mut v = json!({"name": l.name, "kind": l.kind, "parameter": l.name, "default": round4(l.amount),
                               "mask": {"index": i / 4, "channel": i % 4}, "spread": round4(r.spread as f64),
                               "colour": rgb(r.colour), "opacity": round4(r.opacity as f64), "roughness": round4(r.roughness as f64)});
            if let Some((_, c, ro)) = r.variant {
                v["variant"] = json!({"parameter": l.variant_parameter(), "default": round4(l.variant), "colour": rgb(c), "roughness": round4(ro as f64)});
            }
            Some(v)
        })
        .collect();
    let baked: Vec<Value> = spec.layers.iter().filter(|l| !l.is_live()).map(|l| json!({"name": l.name, "kind": l.kind})).collect();
    json!({"version": 1, "look": spec.look.id, "baked": baked, "live": live, "expressions": []})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skin_texture;

    fn spec() -> SkinSpec {
        SkinSpec {
            look: skin_texture::look("medium-warm").unwrap(),
            landmarks: hash::reference(&"b".repeat(64)),
            size: 1024,
            layers: vec![
                SkinLayer { name: "blood".into(), kind: "blood".into(), style: "drips".into(), live: None, amount: 0.25, variant: 0.0, seed: 3, colour: None },
                SkinLayer { name: "dust".into(), kind: "grime".into(), style: "dust".into(), live: Some(false), amount: 0.5, variant: 0.0, seed: 1, colour: None },
                SkinLayer { name: "mud".into(), kind: "grime".into(), style: "mud".into(), live: Some(true), amount: 0.0, variant: 0.3, seed: 2, colour: None },
            ],
        }
    }

    #[test]
    fn a_good_spec_has_no_problems_and_bad_layers_are_named() {
        assert!(spec().problems().is_empty(), "{:?}", spec().problems());
        let mut bad = spec();
        bad.layers[0].name = "Blood-1".into();
        bad.layers[1].style = "glitter".into();
        bad.layers[2].amount = 1.5;
        bad.layers.push(SkinLayer { name: "mud_wet".into(), kind: "paint".into(), style: "chin".into(), live: None, amount: 0.1, variant: 0.5, seed: 0, colour: Some([2.0, 0.0, 0.0]) });
        bad.size = 100;
        let p = bad.problems().join(" | ");
        for needle in ["lowercase identifier", "glitter", "amount 1.5", "mud_wet", "has no variant state", "colour must be", "skin.size"] {
            assert!(p.contains(needle), "{needle} not in {p}");
        }
    }

    #[test]
    fn shading_words_and_the_reader_prefix_are_refused_as_names() {
        for (name, ok) in [("mix", false), ("float", false), ("gf_blood", false), ("mixed", true), ("blood_2", true)] {
            let mut s = spec();
            s.layers[0].name = name.into();
            assert_eq!(s.problems().is_empty(), ok, "{name}: {:?}", s.problems());
        }
    }

    #[test]
    fn coverage_follows_the_reach_order() {
        assert_eq!(coverage(255, 0.0, 0.1), 0.0);
        assert_eq!(coverage(0, 1.0, 0.1), 0.0);
        assert_eq!(coverage(255, 1.0, 0.1), 1.0);
        assert!(coverage(230, 0.2, 0.1) > coverage(200, 0.2, 0.1));
        assert_eq!(coverage(200, 0.2, 0.1), 0.0);
    }

    #[test]
    fn the_extension_lists_live_layers_with_packed_channels_and_baked_names() {
        let e = extension(&spec().canonical());
        assert_eq!(e["live"].as_array().unwrap().len(), 2);
        assert_eq!(e["live"][1]["mask"], json!({"index": 0, "channel": 1}));
        assert_eq!(e["live"][1]["variant"]["parameter"], "mud_wet");
        assert_eq!(e["live"][1]["variant"]["default"], 0.3);
        assert_eq!(e["live"][0]["variant"]["parameter"], "blood_dry");
        assert_eq!(e["baked"], json!([{"name": "dust", "kind": "grime"}]));
    }

    #[test]
    fn baking_at_full_cover_lands_on_the_layer_colour_and_roughness() {
        let mut tex = SkinTextures { size: 1, albedo: vec![200, 150, 120, 255], normal: vec![128, 128, 255], orm: vec![255, 140, 0] };
        let r = Response { colour: [0.5, 0.25, 0.0], opacity: 1.0, roughness: 0.9, spread: 0.1, variant: None };
        bake(&mut tex, &[255], &r, 1.0, 0.0);
        let want = [0.5f32, 0.25, 0.0].map(|c| (to_srgb(c) * 255.0 + 0.5) as u8);
        assert_eq!(&tex.albedo[..3], &want);
        assert_eq!(tex.orm[1], (0.9f32 * 255.0 + 0.5) as u8);
        let mut untouched = SkinTextures { size: 1, albedo: vec![200, 150, 120, 255], normal: vec![128, 128, 255], orm: vec![255, 140, 0] };
        bake(&mut untouched, &[0], &r, 1.0, 0.0);
        assert_eq!(untouched.albedo, vec![200, 150, 120, 255]);
    }

    #[test]
    fn packing_puts_four_masks_per_rgba_image() {
        let masks: Vec<Vec<u8>> = (0..5).map(|i| vec![i as u8 * 10; 2]).collect();
        let packed = pack(&masks);
        assert_eq!(packed.len(), 2);
        assert_eq!(&packed[0][..4], &[0, 10, 20, 30]);
        assert_eq!(&packed[1][..4], &[40, 0, 0, 0]);
    }
}
