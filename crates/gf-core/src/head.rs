use std::collections::BTreeMap;

use crate::macros::{self, MacroDefinition, Mode};

pub const HEAD_SCALE_RANGE: [f64; 2] = [0.9, 1.1];
pub const SKIP_BELOW: f64 = 1e-12;

pub struct HeadOnly {
    pub centre: [f64; 3],
    pub seam_top: f64,
    pub pin_top: f64,
    pub ramp: Vec<f64>,
    x: Vec<[f64; 3]>,
    gram_inv: [[f64; 7]; 7],
}

fn columns(x: [f64; 3]) -> [[f64; 3]; 7] {
    [
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        x,
        [0.0, -x[2], x[1]],
        [x[2], 0.0, -x[0]],
        [-x[1], x[0], 0.0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn invert(m: [[f64; 7]; 7]) -> Option<[[f64; 7]; 7]> {
    let mut a = m;
    let mut inv = [[0.0; 7]; 7];
    for (i, row) in inv.iter_mut().enumerate() {
        row[i] = 1.0;
    }
    for c in 0..7 {
        let pivot = (c..7).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[pivot][c].abs() < 1e-300 {
            return None;
        }
        a.swap(c, pivot);
        inv.swap(c, pivot);
        let p = a[c][c];
        for k in 0..7 {
            a[c][k] /= p;
            inv[c][k] /= p;
        }
        for r in 0..7 {
            if r != c {
                let f = a[r][c];
                if f != 0.0 {
                    for k in 0..7 {
                        a[r][k] -= f * a[c][k];
                        inv[r][k] -= f * inv[c][k];
                    }
                }
            }
        }
    }
    Some(inv)
}

impl HeadOnly {
    pub fn new(base: &[[f64; 3]], boundary: &[u32], pin_top: f64) -> Result<HeadOnly, String> {
        if boundary.is_empty() {
            return Err("the dense base has no open boundary, so there is no seam to pin".into());
        }
        let mut centre = [0.0; 3];
        let mut seam_top = f64::NEG_INFINITY;
        for &v in boundary {
            let p = base[v as usize];
            for k in 0..3 {
                centre[k] += p[k];
            }
            seam_top = seam_top.max(p[2]);
        }
        centre = centre.map(|c| c / boundary.len() as f64);
        if !(pin_top.is_finite() && pin_top > seam_top) {
            return Err(format!("the pin top {pin_top} m must lie above the seam top {seam_top:.4} m"));
        }
        let x: Vec<[f64; 3]> = base.iter().map(|p| [p[0] - centre[0], p[1] - centre[1], p[2] - centre[2]]).collect();
        let mut gram = [[0.0; 7]; 7];
        for &xv in &x {
            let c = columns(xv);
            for i in 0..7 {
                for j in i..7 {
                    gram[i][j] += dot(c[i], c[j]);
                }
            }
        }
        let gram: [[f64; 7]; 7] = std::array::from_fn(|i| std::array::from_fn(|j| gram[i.min(j)][i.max(j)]));
        let gram_inv = invert(gram).ok_or("the similarity system is singular")?;
        let ramp = base
            .iter()
            .map(|p| {
                let s = ((p[2] - seam_top) / (pin_top - seam_top)).clamp(0.0, 1.0);
                s * s * (3.0 - 2.0 * s)
            })
            .collect();
        Ok(HeadOnly { centre, seam_top, pin_top, ramp, x, gram_inv })
    }

    pub fn similarity(&self, delta: &[[f64; 3]]) -> [f64; 7] {
        let mut rhs = [0.0; 7];
        for (&xv, &d) in self.x.iter().zip(delta) {
            let c = columns(xv);
            for i in 0..7 {
                rhs[i] += dot(c[i], d);
            }
        }
        std::array::from_fn(|i| (0..7).map(|j| self.gram_inv[i][j] * rhs[j]).sum())
    }

    pub fn strip_and_pin(&self, delta: &[[f64; 3]]) -> (Vec<[f64; 3]>, [f64; 7]) {
        let coef = self.similarity(delta);
        let out = self
            .x
            .iter()
            .zip(delta)
            .zip(&self.ramp)
            .map(|((&xv, &d), &r)| {
                let c = columns(xv);
                std::array::from_fn(|k| {
                    let fitted: f64 = (0..7).map(|i| coef[i] * c[i][k]).sum();
                    (d[k] - fitted) * r
                })
            })
            .collect();
        (out, coef)
    }

    pub fn add_head_scale(&self, scale: f64, into: &mut [[f64; 3]]) -> Result<(), String> {
        if !(scale.is_finite() && (HEAD_SCALE_RANGE[0]..=HEAD_SCALE_RANGE[1]).contains(&scale)) {
            return Err(format!("head_scale {scale} is outside [{}, {}]", HEAD_SCALE_RANGE[0], HEAD_SCALE_RANGE[1]));
        }
        if scale == 1.0 {
            return Ok(());
        }
        for ((p, &xv), &r) in into.iter_mut().zip(&self.x).zip(&self.ramp) {
            for k in 0..3 {
                p[k] += (scale - 1.0) * r * xv[k];
            }
        }
        Ok(())
    }
}

pub fn weight_changes(
    def: &MacroDefinition,
    mode: Mode,
    sliders: &BTreeMap<String, f64>,
    base_sliders: &BTreeMap<String, f64>,
    on_head: &dyn Fn(&str) -> bool,
) -> Result<Vec<(String, f64)>, String> {
    let now = macros::target_weights(def, sliders, None, mode)?;
    let before = macros::target_weights(def, base_sliders, None, mode)?;
    let mut names: Vec<&String> = now.keys().chain(before.keys()).collect();
    names.sort();
    names.dedup();
    Ok(names
        .into_iter()
        .filter_map(|n| {
            let dw = now.get(n).copied().unwrap_or(0.0) - before.get(n).copied().unwrap_or(0.0);
            (dw.abs() >= SKIP_BELOW && on_head(n)).then(|| (n.clone(), dw))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> (Vec<[f64; 3]>, Vec<u32>) {
        let mut points = Vec::new();
        for i in 0..6 {
            for j in 0..6 {
                for k in 0..6 {
                    points.push([i as f64 * 0.02 - 0.05, j as f64 * 0.02 - 0.05, 1.30 + k as f64 * 0.04]);
                }
            }
        }
        let boundary = (0..points.len() as u32).filter(|&v| points[v as usize][2] < 1.31).collect();
        (points, boundary)
    }

    #[test]
    fn a_pure_similarity_is_stripped_and_the_seam_never_moves() {
        let (points, boundary) = grid();
        let head = HeadOnly::new(&points, &boundary, 1.40).unwrap();
        let delta: Vec<[f64; 3]> = points.iter().map(|p| [0.01 + 0.02 * p[0] - 0.003 * p[2], -0.004 + 0.02 * p[1], 0.003 * p[0] + 0.02 * p[2]]).collect();
        let (out, coef) = head.strip_and_pin(&delta);
        assert!(out.iter().flatten().all(|v| v.abs() < 1e-12), "{:?}", out.iter().flatten().fold(0.0f64, |a, v| a.max(v.abs())));
        assert!((coef[3] - 0.02).abs() < 1e-12);
        let bump: Vec<[f64; 3]> = points.iter().map(|p| [0.0, 0.0, (p[0] * 40.0).sin() * 0.001]).collect();
        let (out, _) = head.strip_and_pin(&bump);
        for &v in &boundary {
            assert_eq!(out[v as usize], [0.0; 3]);
        }
        assert!(out.iter().flatten().any(|v| v.abs() > 1e-5));
    }

    #[test]
    fn the_strip_is_linear() {
        let (points, boundary) = grid();
        let head = HeadOnly::new(&points, &boundary, 1.40).unwrap();
        let a: Vec<[f64; 3]> = points.iter().map(|p| [(p[1] * 30.0).cos() * 0.002, p[0] * p[0], 0.0]).collect();
        let b: Vec<[f64; 3]> = points.iter().map(|p| [0.0, (p[2] * 9.0).sin() * 0.003, p[0] * p[1]]).collect();
        let mix: Vec<[f64; 3]> = a.iter().zip(&b).map(|(x, y)| std::array::from_fn(|k| 0.3 * x[k] + 0.7 * y[k])).collect();
        let (sa, _) = head.strip_and_pin(&a);
        let (sb, _) = head.strip_and_pin(&b);
        let (sm, _) = head.strip_and_pin(&mix);
        let gap = sm.iter().zip(sa.iter().zip(&sb)).flat_map(|(m, (x, y))| (0..3).map(move |k| (m[k] - 0.3 * x[k] - 0.7 * y[k]).abs())).fold(0.0, f64::max);
        assert!(gap < 1e-15, "{gap}");
    }

    #[test]
    fn head_scale_is_bounded_and_pinned_at_the_seam() {
        let (points, boundary) = grid();
        let head = HeadOnly::new(&points, &boundary, 1.40).unwrap();
        let mut out = vec![[0.0; 3]; points.len()];
        head.add_head_scale(1.05, &mut out).unwrap();
        for &v in &boundary {
            assert_eq!(out[v as usize], [0.0; 3]);
        }
        let top = points.iter().position(|p| p[2] > 1.49).unwrap();
        assert!((out[top][2] - 0.05 * (points[top][2] - head.centre[2])).abs() < 1e-12);
        assert!(head.add_head_scale(1.2, &mut out).is_err());
        assert!(head.add_head_scale(f64::NAN, &mut out).is_err());
        assert!(HeadOnly::new(&points, &boundary, 1.30).is_err());
    }
}
