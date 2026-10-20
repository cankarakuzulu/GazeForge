use crate::eye_inventory::{IrisLook, ScleraLook};

fn hash(x: i64, y: i64, seed: u32) -> f32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ (seed as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn noise(x: f32, y: f32, period_x: i64, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (smooth(x - x0), smooth(y - y0));
    let (ix, iy) = (x0 as i64, y0 as i64);
    let at = |i: i64, j: i64| hash(i.rem_euclid(period_x), j, seed);
    let a = at(ix, iy) + (at(ix + 1, iy) - at(ix, iy)) * fx;
    let b = at(ix, iy + 1) + (at(ix + 1, iy + 1) - at(ix, iy + 1)) * fx;
    a + (b - a) * fy
}

fn fbm(x: f32, y: f32, period_x: i64, seed: u32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut f) = (0.0, 0.5, 0.0, 1.0);
    for o in 0..octaves {
        sum += amp * noise(x * f, y * f, period_x * f as i64, seed.wrapping_add(o * 101));
        norm += amp;
        amp *= 0.5;
        f *= 2.0;
    }
    sum / norm
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t.clamp(0.0, 1.0))
}

fn step(e0: f32, e1: f32, x: f32) -> f32 {
    smooth(((x - e0) / (e1 - e0)).clamp(0.0, 1.0))
}

fn to_srgb_u8(c: f32) -> u8 {
    (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

pub fn iris_rgba(look: &IrisLook, width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 4);
    let period = 96i64;
    for j in 0..height {
        let v = (j as f32 + 0.5) / height as f32;
        for i in 0..width {
            let u = (i as f32 + 0.5) / width as f32;
            let x = u * period as f32;
            let warp = 0.6 * (fbm(x * 0.25, v * 3.0, period / 4, look.seed, 3) - 0.5);
            let fibre = fbm(x * 1.0 + warp * 4.0, v * 1.5, period, look.seed.wrapping_add(1), 4);
            let fine = noise(x * 3.0, v * 6.0, period * 3, look.seed.wrapping_add(2));
            let wobble = 0.12 * (fbm(x * 0.12, 0.5, (period as f32 * 0.12) as i64, look.seed.wrapping_add(4), 3) - 0.5);
            let edge = look.collarette + wobble;
            let collar = step(edge - 0.09, edge + 0.09, v);
            let mut c = mix(look.inner, look.outer, collar);
            if let Some(s) = &look.sector {
                let deg = u * 360.0;
                let d = ((deg - s.start_deg).rem_euclid(360.0) - s.width_deg / 2.0).abs().min(((deg - s.start_deg - s.width_deg / 2.0).rem_euclid(360.0) - 360.0).abs());
                let inside = 1.0 - step(s.width_deg * 0.35, s.width_deg * 0.6, d);
                c = mix(c, s.colour, inside * step(look.collarette - 0.1, look.collarette + 0.05, v));
            }
            let ring = (-((v - edge) / 0.03).powi(2)).exp();
            let shade = 1.0 + look.fibres * 0.55 * (fibre - 0.5) + 0.18 * look.fibres * (fine - 0.5) + 0.22 * ring;
            let crypt = step(0.62, 0.78, fbm(x * 0.7, v * 2.2 + 0.3 * warp, period, look.seed.wrapping_add(3), 3)) * look.crypts * step(look.collarette - 0.05, look.collarette + 0.15, v) * (1.0 - step(0.8, 0.95, v));
            let ruff = 1.0 - 0.7 * (1.0 - step(0.0, 0.045, v));
            let mut p = c.map(|k| k * shade * (1.0 - 0.55 * crypt) * ruff);
            p = mix(p, look.limbal, step(1.0 - look.limbal_width, 1.0, v) * 0.92);
            out.extend(p.map(to_srgb_u8));
            out.push(255);
        }
    }
    out
}

pub fn sclera_rgba(look: &ScleraLook, limbus_v: f32, width: usize, height: usize, seed: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 4);
    let period = 64i64;
    for j in 0..height {
        let v = (j as f32 + 0.5) / height as f32;
        let front = 1.0 - step(limbus_v, limbus_v + 0.35, v);
        for i in 0..width {
            let u = (i as f32 + 0.5) / width as f32;
            let x = u * period as f32;
            let ridge = 1.0 - (2.0 * fbm(x * 1.5, v * 10.0, (period as f32 * 1.5) as i64, seed.wrapping_add(11), 4) - 1.0).abs();
            let vein = step(0.9, 0.98, ridge) * look.veins * front;
            let blotch = 0.04 * (fbm(x * 0.3, v * 2.0, period / 2, seed.wrapping_add(12), 3) - 0.5);
            let base = look.tint.map(|k| k * (1.0 + blotch) * (1.0 - 0.06 * (1.0 - front)));
            let p = mix(base, [0.62, 0.16, 0.14], vein * 0.8);
            out.extend(p.map(to_srgb_u8));
            out.push(255);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eye_inventory::{model, models};

    #[test]
    fn textures_are_deterministic_sized_and_differ_between_models() {
        let a = model("blue-sky").unwrap();
        let b = model("brown-dark").unwrap();
        let t1 = iris_rgba(&a.iris, 128, 32);
        assert_eq!(t1.len(), 128 * 32 * 4);
        assert_eq!(t1, iris_rgba(&a.iris, 128, 32));
        assert_ne!(t1, iris_rgba(&b.iris, 128, 32));
        let mean = |t: &[u8], k: usize| t.chunks(4).map(|p| p[k] as f64).sum::<f64>() / (t.len() / 4) as f64;
        assert!(mean(&t1, 2) > mean(&t1, 0), "a blue iris is bluer than it is red");
        let t2 = iris_rgba(&b.iris, 128, 32);
        assert!(mean(&t2, 0) > mean(&t2, 2), "a brown iris is redder than it is blue");
        let s = sclera_rgba(&a.sclera, 0.16, 64, 32, 1);
        assert!(mean(&s, 0) > 180.0, "the sclera is light");
        assert_eq!(models().len(), 25);
    }

    #[test]
    fn the_limbal_ring_darkens_the_rim() {
        let m = model("blue-ice").unwrap();
        let t = iris_rgba(&m.iris, 64, 64);
        let row = |j: usize| t[j * 64 * 4..(j + 1) * 64 * 4].chunks(4).map(|p| p[0] as f64 + p[1] as f64 + p[2] as f64).sum::<f64>();
        assert!(row(63) < row(48) * 0.7, "rim {} vs mid {}", row(63), row(48));
    }
}
