use std::collections::VecDeque;

use gf_geom::Uv;

pub const NONE: u32 = u32::MAX;

pub struct TexelMap {
    pub size: usize,
    pub triangle: Vec<u32>,
    pub bary: Vec<[f32; 2]>,
    pub source: Vec<u32>,
}

impl TexelMap {
    pub fn covered(&self) -> usize {
        self.triangle.iter().filter(|&&t| t != NONE).count()
    }

    pub fn interpolate<const N: usize>(&self, triangles: &[[u32; 3]], values: &[[f32; N]]) -> Vec<Option<[f32; N]>> {
        (0..self.triangle.len())
            .map(|i| {
                let s = self.source[i];
                if s == NONE {
                    return None;
                }
                let s = s as usize;
                let t = triangles[self.triangle[s] as usize];
                let [b0, b1] = self.bary[s];
                let b2 = 1.0 - b0 - b1;
                let (a, b, c) = (values[t[0] as usize], values[t[1] as usize], values[t[2] as usize]);
                Some(std::array::from_fn(|k| b0 * a[k] + b1 * b[k] + b2 * c[k]))
            })
            .collect()
    }
}

pub fn mirror(map: &TexelMap, triangles: &[[u32; 3]], positions_mm: &[[f32; 3]]) -> Vec<u32> {
    let pts = map.interpolate(triangles, positions_mm);
    let cell = 1.0f32;
    let key = |p: [f32; 3]| (p[0].div_euclid(cell) as i32, p[1].div_euclid(cell) as i32, p[2].div_euclid(cell) as i32);
    let mut grid: std::collections::HashMap<(i32, i32, i32), Vec<u32>> = std::collections::HashMap::new();
    for (k, p) in pts.iter().enumerate() {
        if map.triangle[k] != NONE
            && let Some(p) = p
        {
            grid.entry(key(*p)).or_default().push(k as u32);
        }
    }
    let mut out = vec![NONE; pts.len()];
    for (k, p) in pts.iter().enumerate() {
        let Some(p) = p else { continue };
        if map.triangle[k] == NONE {
            continue;
        }
        let q = [-p[0], p[1], p[2]];
        let (cx, cy, cz) = key(q);
        let mut best = (f32::INFINITY, NONE);
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if let Some(list) = grid.get(&(cx + dx, cy + dy, cz + dz)) {
                        for &j in list {
                            let r = pts[j as usize].unwrap();
                            let d = (r[0] - q[0]).powi(2) + (r[1] - q[1]).powi(2) + (r[2] - q[2]).powi(2);
                            if d < best.0 {
                                best = (d, j);
                            }
                        }
                    }
                }
            }
        }
        out[k] = best.1;
    }
    out
}

fn edge(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

// Texel (i, j) is centred at u = (i + 0.5) / size, v = (j + 0.5) / size and is image row j: glTF reads v = 0 at the top
// row, and the exporter writes the UVs unflipped.
pub fn build(triangle_uvs: &[[Uv; 3]], size: usize, pad: usize) -> TexelMap {
    let n = size * size;
    let mut triangle = vec![NONE; n];
    let mut bary = vec![[0.0f32; 2]; n];
    let s = size as f64;
    for (ti, t) in triangle_uvs.iter().enumerate() {
        let p: [[f64; 2]; 3] = t.map(|c| [c[0] as f64 * s, c[1] as f64 * s]);
        let area = edge(p[0], p[1], p[2]);
        if area.abs() < 1e-12 {
            continue;
        }
        let lo = |k: usize| (p.iter().map(|q| q[k]).fold(f64::INFINITY, f64::min) - 0.5).floor().max(0.0) as usize;
        let hi = |k: usize| ((p.iter().map(|q| q[k]).fold(f64::NEG_INFINITY, f64::max) - 0.5).ceil().max(0.0) as usize).min(size - 1);
        for j in lo(1)..=hi(1) {
            for i in lo(0)..=hi(0) {
                let c = [i as f64 + 0.5, j as f64 + 0.5];
                let w0 = edge(p[1], p[2], c) / area;
                let w1 = edge(p[2], p[0], c) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < -1e-9 || w1 < -1e-9 || w2 < -1e-9 {
                    continue;
                }
                let k = j * size + i;
                if triangle[k] == NONE {
                    triangle[k] = ti as u32;
                    bary[k] = [w0 as f32, w1 as f32];
                }
            }
        }
    }
    let mut source = vec![NONE; n];
    let mut ring = vec![0usize; n];
    let mut queue = VecDeque::new();
    for k in 0..n {
        if triangle[k] != NONE {
            source[k] = k as u32;
            queue.push_back(k);
        }
    }
    while let Some(k) = queue.pop_front() {
        if ring[k] >= pad {
            continue;
        }
        let (i, j) = ((k % size) as i64, (k / size) as i64);
        for (di, dj) in [(-1, 0), (1, 0), (0, -1), (0, 1), (-1, -1), (1, -1), (-1, 1), (1, 1)] {
            let (x, y) = (i + di, j + dj);
            if x < 0 || y < 0 || x >= size as i64 || y >= size as i64 {
                continue;
            }
            let q = y as usize * size + x as usize;
            if source[q] == NONE {
                source[q] = source[k];
                ring[q] = ring[k] + 1;
                queue.push_back(q);
            }
        }
    }
    TexelMap { size, triangle, bary, source }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> (Vec<[u32; 3]>, Vec<[Uv; 3]>) {
        (vec![[0, 1, 2], [0, 2, 3]], vec![[[0.25, 0.25], [0.75, 0.25], [0.75, 0.75]], [[0.25, 0.25], [0.75, 0.75], [0.25, 0.75]]])
    }

    #[test]
    fn every_texel_inside_the_square_maps_into_a_triangle_with_valid_barycentrics() {
        let (_, uvs) = square();
        let m = build(&uvs, 16, 0);
        assert_eq!(m.covered(), 64, "the square spans 8 x 8 texel centres");
        for k in 0..256 {
            if m.triangle[k] == NONE {
                continue;
            }
            let [b0, b1] = m.bary[k];
            assert!(b0 >= -1e-6 && b1 >= -1e-6 && b0 + b1 <= 1.0 + 1e-6);
            let t = uvs[m.triangle[k] as usize];
            let (i, j) = (k % 16, k / 16);
            let u = b0 * t[0][0] + b1 * t[1][0] + (1.0 - b0 - b1) * t[2][0];
            let v = b0 * t[0][1] + b1 * t[1][1] + (1.0 - b0 - b1) * t[2][1];
            assert!((u - (i as f32 + 0.5) / 16.0).abs() < 1e-5 && (v - (j as f32 + 0.5) / 16.0).abs() < 1e-5);
        }
    }

    #[test]
    fn padding_grows_by_rings_and_copies_a_covered_texel() {
        let (_, uvs) = square();
        let m = build(&uvs, 16, 2);
        let filled = m.source.iter().filter(|&&s| s != NONE).count();
        assert_eq!(filled, 12 * 12, "two rings around the 8 x 8 block");
        for (k, &s) in m.source.iter().enumerate() {
            if s != NONE {
                assert_ne!(m.triangle[s as usize], NONE, "texel {k} copies an uncovered texel");
            }
        }
    }

    #[test]
    fn interpolation_reproduces_a_linear_field() {
        let (tris, uvs) = square();
        let m = build(&uvs, 32, 1);
        let values = [[0.0f32, 1.0], [10.0, 1.0], [10.0, 11.0], [0.0, 11.0]];
        let out = m.interpolate(&tris, &values);
        for (k, v) in out.iter().enumerate() {
            if m.triangle[k] == NONE {
                continue;
            }
            let (i, j) = (k % 32, k / 32);
            let (u, w) = ((i as f32 + 0.5) / 32.0, (j as f32 + 0.5) / 32.0);
            let v = v.unwrap();
            assert!((v[0] - (u - 0.25) * 20.0).abs() < 1e-3 && (v[1] - (1.0 + (w - 0.25) * 20.0)).abs() < 1e-3);
        }
    }
}
