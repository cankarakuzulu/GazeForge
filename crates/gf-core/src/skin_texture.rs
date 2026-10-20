use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::skin_map::{NONE, TexelMap};
use crate::skin_regions::Regions;
use gf_geom::Uv;

type Point = [f64; 3];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkinLook {
    pub id: String,
    pub seed: u32,
    pub melanin: f32,
    pub haemoglobin: f32,
    pub undertone: f32,
    pub redness: f32,
    pub lip_tint: f32,
    pub under_eye: f32,
    pub freckles: f32,
    pub freckle_size_mm: f32,
    pub moles: u32,
    pub pores: f32,
    pub age: f32,
    pub stubble: f32,
    pub hair: [f32; 3],
    pub oil: f32,
}

struct Row(&'static str, f32, f32, f32, f32, f32, f32, f32, f32, u32, f32, f32, f32, u32, f32);

const ROWS: [Row; 12] = [
    Row("fair-young", 0.08, 0.35, -0.3, 0.45, 0.4, 0.3, 0.12, 1.2, 4, 0.45, 0.1, 0.0, 0x5a4030, 0.4),
    Row("fair-freckled", 0.10, 0.40, 0.2, 0.5, 0.35, 0.3, 0.8, 1.7, 6, 0.45, 0.15, 0.0, 0x8c4a26, 0.35),
    Row("fair-elder", 0.12, 0.45, 0.0, 0.5, 0.2, 0.5, 0.2, 1.4, 9, 0.6, 1.0, 0.2, 0xb8b4ae, 0.3),
    Row("light-olive", 0.25, 0.30, 0.6, 0.3, 0.35, 0.35, 0.05, 1.2, 5, 0.55, 0.25, 0.0, 0x3a2a20, 0.5),
    Row("light-stubble", 0.20, 0.40, 0.1, 0.4, 0.2, 0.35, 0.1, 1.2, 5, 0.65, 0.3, 0.7, 0x3c2c22, 0.5),
    Row("medium-warm", 0.42, 0.30, 0.5, 0.25, 0.3, 0.4, 0.05, 1.5, 4, 0.55, 0.2, 0.0, 0x1e1612, 0.45),
    Row("medium-stubble", 0.45, 0.30, 0.5, 0.25, 0.2, 0.45, 0.0, 1.5, 4, 0.65, 0.45, 0.8, 0x141110, 0.5),
    Row("tan-mature", 0.55, 0.30, 0.4, 0.25, 0.25, 0.5, 0.05, 1.5, 6, 0.6, 0.7, 0.3, 0x6e6660, 0.4),
    Row("brown-young", 0.68, 0.25, 0.3, 0.2, 0.3, 0.45, 0.0, 1.5, 3, 0.5, 0.1, 0.0, 0x100c0a, 0.5),
    Row("brown-beard", 0.70, 0.25, 0.3, 0.2, 0.2, 0.5, 0.0, 1.5, 4, 0.6, 0.35, 0.9, 0x0e0b0a, 0.5),
    Row("deep-young", 0.88, 0.20, 0.1, 0.15, 0.3, 0.4, 0.0, 1.5, 3, 0.5, 0.1, 0.0, 0x0c0a09, 0.55),
    Row("deep-mature", 0.90, 0.20, 0.1, 0.15, 0.25, 0.5, 0.0, 1.5, 5, 0.6, 0.7, 0.3, 0x8a8682, 0.45),
];

pub const DEFAULT_LOOK: &str = "medium-warm";

pub fn looks() -> Vec<SkinLook> {
    ROWS.iter()
        .enumerate()
        .map(|(k, r)| SkinLook {
            id: r.0.into(),
            seed: 104_729 * (k as u32 + 1),
            melanin: r.1,
            haemoglobin: r.2,
            undertone: r.3,
            redness: r.4,
            lip_tint: r.5,
            under_eye: r.6,
            freckles: r.7,
            freckle_size_mm: r.8,
            moles: r.9,
            pores: r.10,
            age: r.11,
            stubble: r.12,
            hair: [(r.13 >> 16) & 0xff, (r.13 >> 8) & 0xff, r.13 & 0xff].map(|c| c as f32 / 255.0),
            oil: r.14,
        })
        .collect()
}

pub fn look(id: &str) -> Option<SkinLook> {
    looks().into_iter().find(|l| l.id == id)
}

pub struct SkinTextures {
    pub size: usize,
    pub albedo: Vec<u8>,
    pub normal: Vec<u8>,
    pub orm: Vec<u8>,
}

const PALETTE: [[f32; 3]; 6] = [[236.0, 212.0, 198.0], [218.0, 186.0, 166.0], [190.0, 152.0, 128.0], [152.0, 112.0, 88.0], [106.0, 74.0, 56.0], [64.0, 44.0, 34.0]];

pub fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

pub fn to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

fn mul3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

pub fn tone(melanin: f32, haemoglobin: f32, undertone: f32) -> [f32; 3] {
    let m = melanin.clamp(0.0, 1.0) * (PALETTE.len() - 1) as f32;
    let i = (m.floor() as usize).min(PALETTE.len() - 2);
    let f = m - i as f32;
    let lin = |c: [f32; 3]| c.map(|x| to_linear(x / 255.0));
    let base = mix3(lin(PALETTE[i]), lin(PALETTE[i + 1]), f);
    let h = haemoglobin.clamp(0.0, 1.5);
    let blood = [1.0 - 0.02 * h, 1.0 - 0.30 * h, 1.0 - 0.26 * h];
    let u = undertone.clamp(-1.0, 1.0);
    let under = if u >= 0.0 { [1.0, 1.0 - 0.01 * u, 1.0 - 0.08 * u] } else { [1.0, 1.0 + 0.03 * u, 1.0 + 0.01 * u] };
    mul3(mul3(base, blood), under)
}

pub fn fit_tone(target: [f32; 3]) -> (f32, f32, f32, f32) {
    let mut best = (f32::INFINITY, 0.0, 0.0, 0.0);
    for mi in 0..=100 {
        for hi in 0..=20 {
            for ui in 0..=20 {
                let (m, h, u) = (mi as f32 / 100.0, hi as f32 / 20.0, ui as f32 / 10.0 - 1.0);
                let c = tone(m, h, u);
                let err: f32 = (0..3).map(|k| (c[k].max(1e-4).ln() - target[k].max(1e-4).ln()).powi(2)).sum();
                if err < best.0 {
                    best = (err, m, h, u);
                }
            }
        }
    }
    (best.1, best.2, best.3, best.0.sqrt())
}

fn hash3(x: i64, y: i64, z: i64, seed: u32) -> u64 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (z as u64).wrapping_mul(0x1656_67B1_9E37_79F9)
        ^ (seed as u64).wrapping_mul(0x27D4_EB2F_1656_67C5);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^ (h >> 33)
}

fn unit(h: u64, shift: u32) -> f32 {
    ((h >> shift) & 0xFF_FFFF) as f32 / (1u64 << 24) as f32
}

fn fade(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn noise3(p: [f32; 3], seed: u32) -> f32 {
    let i = p.map(|x| x.floor());
    let f = [0, 1, 2].map(|k| fade(p[k] - i[k]));
    let c = i.map(|x| x as i64);
    let v = |dx: i64, dy: i64, dz: i64| unit(hash3(c[0] + dx, c[1] + dy, c[2] + dz, seed), 8);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(v(0, 0, 0), v(1, 0, 0), f[0]);
    let x10 = lerp(v(0, 1, 0), v(1, 1, 0), f[0]);
    let x01 = lerp(v(0, 0, 1), v(1, 0, 1), f[0]);
    let x11 = lerp(v(0, 1, 1), v(1, 1, 1), f[0]);
    lerp(lerp(x00, x10, f[1]), lerp(x01, x11, f[1]), f[2])
}

fn fbm3(p: [f32; 3], seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0, 0.5, 0.0, 1.0);
    for o in 0..octaves {
        sum += amp * noise3(p.map(|x| x * f), seed.wrapping_add(o * 131));
        norm += amp;
        amp *= 0.5;
        f *= 2.03;
    }
    sum / norm
}

fn cells(p: [f32; 3], seed: u32, density: f32) -> (f32, f32) {
    let c = p.map(|x| x.floor() as i64);
    let mut best = (f32::INFINITY, 0.0f32);
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let (x, y, z) = (c[0] + dx, c[1] + dy, c[2] + dz);
                let h = hash3(x, y, z, seed);
                if unit(h, 0) > density {
                    continue;
                }
                let q = [x as f32 + unit(h, 8), y as f32 + unit(h, 16), z as f32 + unit(h, 24)];
                let d = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
                if d < best.0 {
                    best = (d, unit(h, 32));
                }
            }
        }
    }
    best
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    fade(((x - e0) / (e1 - e0)).clamp(0.0, 1.0))
}

const LIPS: usize = 0;
const MOUTH: usize = 1;
const NOSE: usize = 2;
const CHEEKS: usize = 3;
const FOREHEAD: usize = 4;
const CHIN: usize = 5;
const BEARD: usize = 7;
const UNDER_EYE: usize = 8;
const EYELIDS: usize = 9;
const EARS: usize = 10;
const T_ZONE: usize = 13;

type W = [f32; crate::skin_regions::NAMES.len()];

fn resolved(feature_mm: f32, texel_mm: f32) -> f32 {
    smoothstep(1.5, 3.0, feature_mm / texel_mm.max(1e-6))
}

fn height(mm: [f32; 3], w: &W, look: &SkinLook, texel_mm: f32) -> f32 {
    let pore_zone = 0.35 + 0.65 * (w[NOSE] + w[CHEEKS] * 0.8 + w[FOREHEAD] * 0.5 + w[CHIN] * 0.5).min(1.0);
    let pore_zone = pore_zone * (1.0 - w[LIPS]) * (1.0 - w[EYELIDS] * 0.7) * (1.0 - w[MOUTH]);
    let (d, _) = cells(mm.map(|x| x / 0.55), look.seed.wrapping_add(21), 0.85);
    let pore = -(1.0 - smoothstep(0.0, 0.42, d)) * look.pores * pore_zone * resolved(0.55, texel_mm);
    let grain = 0.25 * (fbm3(mm.map(|x| x / 0.3), look.seed.wrapping_add(22), 2) - 0.5) * (1.0 - w[MOUTH]) * resolved(0.3, texel_mm);
    let warp = 0.35 * (fbm3(mm.map(|x| x / 22.0), look.seed.wrapping_add(23), 2) - 0.5);
    let band = ((mm[2] / 8.0 + warp).fract() - 0.5) / 0.1;
    let broken = smoothstep(0.35, 0.6, fbm3(mm.map(|x| x / 12.0), look.seed.wrapping_add(25), 2));
    let centre = 1.0 - smoothstep(25.0, 45.0, mm[0].abs());
    let lines = -(-band * band).exp() * look.age * look.age * w[FOREHEAD] * 0.5 * broken * centre * resolved(1.2, texel_mm);
    let fine = -look.age * 0.35 * w[UNDER_EYE] * (1.0 - smoothstep(0.0, 0.25, cells(mm.map(|x| x / 1.4), look.seed.wrapping_add(24), 1.0).0)) * resolved(0.7, texel_mm);
    pore + grain + lines + fine
}

fn moles(look: &SkinLook, positions: &[Point], normals: &[Point], regions: &Regions) -> Vec<(Point, f32)> {
    let mut candidates: Vec<(u64, usize)> = (0..positions.len())
        .filter(|&v| {
            let w = &regions.weights[v];
            normals[v][1] < -0.1 && w[LIPS] < 0.1 && w[MOUTH] < 0.1 && w[EYELIDS] < 0.3 && w[EARS] < 0.5 && w[11] < 0.5
        })
        .map(|v| (hash3(v as i64, 7, 13, look.seed), v))
        .collect();
    candidates.sort_unstable();
    candidates.iter().take(look.moles as usize).map(|&(h, v)| (positions[v], 0.6 + 1.0 * unit(h, 40))).collect()
}

type Wedge = (u32, u32, u32);

fn tangent_frames(positions: &[Point], triangles: &[[u32; 3]], uvs: &[[Uv; 3]]) -> Vec<[[f32; 3]; 2]> {
    let mut acc: HashMap<Wedge, ([f64; 3], [f64; 3])> = HashMap::new();
    for (t, tri) in triangles.iter().enumerate() {
        let p = tri.map(|i| positions[i as usize]);
        let w = uvs[t];
        let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
        let (du1, dv1) = ((w[1][0] - w[0][0]) as f64, (w[1][1] - w[0][1]) as f64);
        let (du2, dv2) = ((w[2][0] - w[0][0]) as f64, (w[2][1] - w[0][1]) as f64);
        let det = du1 * dv2 - du2 * dv1;
        let r = if det.abs() < 1e-20 { 0.0 } else { 1.0 / det };
        let dpdu = [0, 1, 2].map(|k| (e1[k] * dv2 - e2[k] * dv1) * r);
        let dpdv = [0, 1, 2].map(|k| (e2[k] * du1 - e1[k] * du2) * r);
        for k in 0..3 {
            let key = (tri[k], w[k][0].to_bits(), w[k][1].to_bits());
            let e = acc.entry(key).or_insert(([0.0; 3], [0.0; 3]));
            for j in 0..3 {
                e.0[j] += dpdu[j];
                e.1[j] += dpdv[j];
            }
        }
    }
    triangles
        .iter()
        .enumerate()
        .flat_map(|(t, tri)| {
            let w = uvs[t];
            let acc = &acc;
            (0..3).map(move |k| {
                let (a, b) = acc[&(tri[k], w[k][0].to_bits(), w[k][1].to_bits())];
                [a.map(|x| x as f32), b.map(|x| x as f32)]
            })
        })
        .collect()
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-20);
    v.map(|x| x / l)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[allow(clippy::too_many_arguments)]
pub fn generate(map: &TexelMap, positions: &[Point], normals: &[Point], triangles: &[[u32; 3]], uvs: &[[Uv; 3]], regions: &Regions, look: &SkinLook) -> SkinTextures {
    let n = map.size * map.size;
    let pos32: Vec<[f32; 3]> = positions.iter().map(|p| p.map(|x| (x * 1000.0) as f32)).collect();
    let nrm32: Vec<[f32; 3]> = normals.iter().map(|p| p.map(|x| x as f32)).collect();
    let frames = tangent_frames(positions, triangles, uvs);
    let mole_list: Vec<([f32; 3], f32)> = moles(look, positions, normals, regions).into_iter().map(|(p, r)| (p.map(|x| (x * 1000.0) as f32), r)).collect();
    let hair = look.hair.map(to_linear);
    let texel_mm: Vec<f32> = triangles
        .iter()
        .zip(uvs)
        .map(|(t, w)| {
            let p = t.map(|i| pos32[i as usize]);
            let e1 = [0, 1, 2].map(|k| p[1][k] - p[0][k]);
            let e2 = [0, 1, 2].map(|k| p[2][k] - p[0][k]);
            let c = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
            let area3 = 0.5 * dot(c, c).sqrt();
            let s = map.size as f32;
            let area2 = 0.5 * ((w[1][0] - w[0][0]) * (w[2][1] - w[0][1]) - (w[2][0] - w[0][0]) * (w[1][1] - w[0][1])).abs() * s * s;
            if area2 > 1e-12 { (area3 / area2).sqrt() } else { 1.0 }
        })
        .collect();
    let mut albedo = vec![0u8; n * 4];
    let mut normal = vec![0u8; n * 3];
    let mut orm = vec![0u8; n * 3];
    for k in 0..n {
        let s = map.source[k];
        if s == NONE {
            albedo[k * 4..k * 4 + 4].copy_from_slice(&[128, 128, 128, 255]);
            normal[k * 3..k * 3 + 3].copy_from_slice(&[128, 128, 255]);
            orm[k * 3..k * 3 + 3].copy_from_slice(&[255, 140, 0]);
            continue;
        }
        let s = s as usize;
        let t = map.triangle[s] as usize;
        let tri = triangles[t];
        let [b0, b1] = map.bary[s];
        let b = [b0, b1, 1.0 - b0 - b1];
        let lerp3 = |v: &[[f32; 3]]| -> [f32; 3] { [0, 1, 2].map(|j| b[0] * v[tri[0] as usize][j] + b[1] * v[tri[1] as usize][j] + b[2] * v[tri[2] as usize][j]) };
        let mm = lerp3(&pos32);
        let nn = normalize(lerp3(&nrm32));
        let mut w: W = [0.0; crate::skin_regions::NAMES.len()];
        for (r, slot) in w.iter_mut().enumerate() {
            *slot = (0..3).map(|c| b[c] * regions.weights[tri[c] as usize][r]).sum::<f32>().clamp(0.0, 1.0);
        }
        let mottle = fbm3(mm.map(|x| x / 38.0), look.seed.wrapping_add(1), 3) - 0.5;
        let flush = fbm3(mm.map(|x| x / 16.0), look.seed.wrapping_add(2), 3) - 0.5;
        let spots = smoothstep(0.7, 0.78, fbm3(mm.map(|x| x / 4.0), look.seed.wrapping_add(3), 3)) * look.age * look.age * (w[FOREHEAD] + w[CHEEKS]).min(1.0);
        let m = look.melanin + 0.06 * mottle * (0.5 + look.age) + 0.08 * spots;
        let regional = 0.55 * w[CHEEKS] + 0.45 * w[NOSE] + 0.4 * w[EARS] + 0.2 * w[CHIN] + 0.25 * w[EYELIDS];
        let h = look.haemoglobin + look.redness * regional + 0.25 * flush;
        let mut c = tone(m, h, look.undertone);
        c = mix3(c, mul3(c, [0.86, 0.82, 0.88]), w[UNDER_EYE] * look.under_eye);
        let fz = (w[NOSE] + 0.9 * w[CHEEKS] + 0.5 * w[FOREHEAD] + 0.12).min(1.0) * (1.0 - w[LIPS]);
        let (fd, fv) = cells(mm.map(|x| x / look.freckle_size_mm.max(0.3)), look.seed.wrapping_add(4), look.freckles.clamp(0.0, 1.0) * fz);
        let freckle = (1.0 - smoothstep(0.15, 0.45, fd)) * (0.5 + 0.5 * fv);
        c = mix3(c, mul3(tone(m + 0.25, h, look.undertone + 0.3), [0.9, 0.85, 0.8]), freckle * 0.75);
        for &(q, r) in &mole_list {
            let d = ((mm[0] - q[0]).powi(2) + (mm[1] - q[1]).powi(2) + (mm[2] - q[2]).powi(2)).sqrt();
            if d < r * 1.5 {
                c = mix3(c, mul3(tone(m + 0.55, h, look.undertone + 0.4), [0.7, 0.62, 0.58]), 1.0 - smoothstep(r * 0.7, r * 1.2, d));
            }
        }
        let lip = mul3(tone(m + 0.08 + 0.18 * m, h + 0.5 + 0.6 * look.lip_tint, look.undertone - 0.3), [0.92, 0.66, 0.68]);
        c = mix3(c, lip, w[LIPS]);
        let beard = w[BEARD] * look.stubble;
        if beard > 0.0 {
            let (d, _) = cells(mm.map(|x| x / 0.7), look.seed.wrapping_add(5), 0.9);
            let r = resolved(0.7, texel_mm[t]);
            let follicle = (1.0 - smoothstep(0.08, 0.3, d)) * r + 0.12 * (1.0 - r);
            c = mix3(c, mix3(mul3(c, [0.72, 0.76, 0.84]), hair, 0.25), beard * 0.8);
            c = mix3(c, hair, beard * follicle * 0.9);
        }
        c = mix3(c, [0.30, 0.06, 0.07], w[MOUTH]);
        let px = &mut albedo[k * 4..k * 4 + 4];
        for j in 0..3 {
            px[j] = (to_srgb(c[j]) * 255.0 + 0.5) as u8;
        }
        px[3] = 255;
        let corner = |j: usize| frames[t * 3 + j];
        let tu = normalize([0, 1, 2].map(|j| b[0] * corner(0)[0][j] + b[1] * corner(1)[0][j] + b[2] * corner(2)[0][j]));
        let tv = normalize([0, 1, 2].map(|j| b[0] * corner(0)[1][j] + b[1] * corner(1)[1][j] + b[2] * corner(2)[1][j]));
        let tan = normalize([0, 1, 2].map(|j| tu[j] - dot(tu, nn) * nn[j]));
        // glTF normal maps are +Y up in the image, and image rows grow with v, so green follows -dP/dv.
        let up = [0, 1, 2].map(|j| -tv[j]);
        let bit = normalize([0, 1, 2].map(|j| up[j] - dot(up, nn) * nn[j] - dot(up, tan) * tan[j]));
        let eps = 0.05f32;
        let probe = |dir: [f32; 3]| {
            let a = height([0, 1, 2].map(|j| mm[j] + eps * dir[j]), &w, look, texel_mm[t]);
            let b = height([0, 1, 2].map(|j| mm[j] - eps * dir[j]), &w, look, texel_mm[t]);
            (a - b) / (2.0 * eps)
        };
        let strength = 0.35;
        let (gx, gy) = (probe(tan), probe(bit));
        let tn = normalize([-strength * gx, -strength * gy, 1.0]);
        let np = &mut normal[k * 3..k * 3 + 3];
        for j in 0..3 {
            np[j] = ((tn[j] * 0.5 + 0.5) * 255.0 + 0.5) as u8;
        }
        let rough = (0.52 - 0.2 * look.oil * w[T_ZONE] - 0.12 * w[LIPS] - 0.25 * w[MOUTH] + 0.1 * beard + 0.06 * look.age).clamp(0.05, 1.0);
        orm[k * 3..k * 3 + 3].copy_from_slice(&[255, (rough * 255.0 + 0.5) as u8, 0]);
    }
    SkinTextures { size: map.size, albedo, normal, orm }
}

pub fn region_map(map: &TexelMap, triangles: &[[u32; 3]], regions: &Regions) -> Vec<u8> {
    const COLOURS: [[u8; 3]; 14] = [
        [220, 40, 60], [90, 0, 20], [250, 160, 40], [240, 110, 150], [80, 160, 240], [140, 90, 200], [255, 220, 120],
        [40, 40, 40], [120, 60, 160], [200, 200, 255], [60, 200, 120], [150, 120, 90], [170, 210, 210], [255, 255, 255],
    ];
    let mut out = vec![0u8; map.size * map.size * 4];
    for k in 0..map.size * map.size {
        let px = &mut out[k * 4..k * 4 + 4];
        let s = map.source[k];
        if s == NONE {
            px.copy_from_slice(&[20, 20, 20, 255]);
            continue;
        }
        let tri = triangles[map.triangle[s as usize] as usize];
        let [b0, b1] = map.bary[s as usize];
        let b = [b0, b1, 1.0 - b0 - b1];
        let mut c = [96.0f32; 3];
        for (r, colour) in COLOURS.iter().enumerate() {
            if r == T_ZONE {
                continue;
            }
            let w = (0..3).map(|j| b[j] * regions.weights[tri[j] as usize][r]).sum::<f32>().clamp(0.0, 1.0);
            c = [0, 1, 2].map(|j| c[j] + (colour[j] as f32 - c[j]) * w);
        }
        px.copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, 255]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_darkens_with_melanin_and_reddens_with_haemoglobin() {
        let light = tone(0.1, 0.3, 0.0);
        let dark = tone(0.9, 0.3, 0.0);
        assert!(light.iter().zip(&dark).all(|(a, b)| a > b));
        let flushed = tone(0.3, 0.9, 0.0);
        let pale = tone(0.3, 0.1, 0.0);
        assert!(flushed[0] / flushed[1] > pale[0] / pale[1]);
    }

    #[test]
    fn noise_is_deterministic_and_bounded() {
        for i in 0..200 {
            let p = [i as f32 * 0.37, i as f32 * -1.3, 2.0 + i as f32 * 0.11];
            let a = fbm3(p, 3, 4);
            assert_eq!(a, fbm3(p, 3, 4));
            assert!((0.0..=1.0).contains(&a));
        }
    }
}
