use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::body::Controls;
use crate::catalog::SparseTarget;
use crate::hash;

type Point = [f64; 3];

pub const TARGETS_FORMAT: &str = "gazeforge.bodytargets/1";
pub const COMPOUNDS_JSON: &str = include_str!("body_compounds.json");

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Compound {
    pub name: String,
    pub plus: Vec<(String, f64)>,
    pub minus: Vec<(String, f64)>,
}

#[derive(Deserialize)]
struct CompoundFile {
    compounds: Vec<Compound>,
}

pub fn compounds() -> Vec<Compound> {
    serde_json::from_str::<CompoundFile>(COMPOUNDS_JSON).map(|f| f.compounds).unwrap_or_default()
}

fn blend() -> f64 {
    60.0
}

fn one() -> f64 {
    1.0
}

fn half() -> f64 {
    0.5
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BodySpec {
    pub pack: String,
    pub targets: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub weights: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub compounds: BTreeMap<String, f64>,
    #[serde(default = "blend")]
    pub blend_mm: f64,
    #[serde(default)]
    pub neck_offset_mm: f64,
    #[serde(default = "one")]
    pub body_scale: f64,
    #[serde(default)]
    pub smooth: usize,
    #[serde(default = "half")]
    pub tangent: f64,
}

fn round(x: f64, q: f64) -> f64 {
    let r = (x / q).round() * q;
    if r == 0.0 { 0.0 } else { r }
}

impl BodySpec {
    pub fn canonical(&self) -> BodySpec {
        BodySpec {
            weights: self.weights.iter().map(|(k, &w)| (k.clone(), round(w, 1e-6))).filter(|(_, w)| *w != 0.0).collect(),
            compounds: self.compounds.iter().map(|(k, &w)| (k.clone(), round(w, 1e-6))).filter(|(_, w)| *w != 0.0).collect(),
            blend_mm: round(self.blend_mm, 1e-3),
            neck_offset_mm: round(self.neck_offset_mm, 1e-3),
            body_scale: round(self.body_scale, 1e-6),
            tangent: round(self.tangent, 1e-4),
            ..self.clone()
        }
    }

    pub fn controls(&self) -> Controls {
        Controls { blend_m: self.blend_mm / 1000.0, neck_offset_m: self.neck_offset_mm / 1000.0, head_scale: 1.0, body_scale: self.body_scale, smooth: self.smooth, tangent: self.tangent }
    }

    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (k, v) in [("body.pack", &self.pack), ("body.targets", &self.targets)] {
            if let Err(e) = hash::parse_reference(v) {
                out.push(format!("{k}: {e}"));
            }
        }
        if let Err(e) = self.controls().check() {
            out.push(format!("body: {e}"));
        }
        for (id, &w) in &self.weights {
            if !(w.is_finite() && (0.0..=1.0).contains(&w)) {
                out.push(format!("body.weights {id} = {w} is outside [0, 1]"));
            }
        }
        let known = compounds();
        for (name, &v) in &self.compounds {
            if !known.iter().any(|c| &c.name == name) {
                out.push(format!("body.compounds {name} is not a compound parameter"));
            }
            if !(v.is_finite() && (-1.0..=1.0).contains(&v)) {
                out.push(format!("body.compounds {name} = {v} is outside [-1, 1]"));
            }
        }
        out
    }

    pub fn target_weights(&self) -> BTreeMap<String, f64> {
        let mut out = self.weights.clone();
        let known = compounds();
        for (name, &v) in &self.compounds {
            let Some(c) = known.iter().find(|c| &c.name == name) else { continue };
            let side = if v >= 0.0 { &c.plus } else { &c.minus };
            for (id, w) in side {
                *out.entry(id.clone()).or_insert(0.0) += w * v.abs();
            }
        }
        out.retain(|_, w| *w != 0.0);
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Similarity {
    pub centre: Point,
    pub translation: Point,
    pub scale: f64,
    pub rotation: [[f64; 3]; 3],
}

fn rodrigues(w: Point) -> [[f64; 3]; 3] {
    let theta = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
    if theta < 1e-15 {
        return [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    let k = w.map(|x| x / theta);
    let (s, c) = theta.sin_cos();
    let v = 1.0 - c;
    [
        [c + k[0] * k[0] * v, k[0] * k[1] * v - k[2] * s, k[0] * k[2] * v + k[1] * s],
        [k[1] * k[0] * v + k[2] * s, c + k[1] * k[1] * v, k[1] * k[2] * v - k[0] * s],
        [k[2] * k[0] * v - k[1] * s, k[2] * k[1] * v + k[0] * s, c + k[2] * k[2] * v],
    ]
}

impl Similarity {
    pub fn identity() -> Similarity {
        Similarity { centre: [0.0; 3], translation: [0.0; 3], scale: 1.0, rotation: rodrigues([0.0; 3]) }
    }

    pub fn from_linear(coef: &[f64; 7], centre: Point) -> Similarity {
        Similarity { centre, translation: [coef[0], coef[1], coef[2]], scale: 1.0 + coef[3], rotation: rodrigues([coef[4], coef[5], coef[6]]) }
    }

    pub fn apply(&self, p: Point) -> Point {
        let d = [p[0] - self.centre[0], p[1] - self.centre[1], p[2] - self.centre[2]];
        let r = &self.rotation;
        std::array::from_fn(|i| self.centre[i] + self.translation[i] + self.scale * (r[i][0] * d[0] + r[i][1] * d[1] + r[i][2] * d[2]))
    }

    pub fn invert(&self, p: Point) -> Point {
        let d = [p[0] - self.centre[0] - self.translation[0], p[1] - self.centre[1] - self.translation[1], p[2] - self.centre[2] - self.translation[2]];
        let r = &self.rotation;
        std::array::from_fn(|i| self.centre[i] + (r[0][i] * d[0] + r[1][i] * d[1] + r[2][i] * d[2]) / self.scale)
    }

    pub fn quaternion(&self) -> [f64; 4] {
        let m = &self.rotation;
        let trace = m[0][0] + m[1][1] + m[2][2];
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            [(m[2][1] - m[1][2]) / s, (m[0][2] - m[2][0]) / s, (m[1][0] - m[0][1]) / s, 0.25 * s]
        } else if m[0][0] > m[1][1] && m[0][0] > m[2][2] {
            let s = (1.0 + m[0][0] - m[1][1] - m[2][2]).sqrt() * 2.0;
            [0.25 * s, (m[0][1] + m[1][0]) / s, (m[0][2] + m[2][0]) / s, (m[2][1] - m[1][2]) / s]
        } else if m[1][1] > m[2][2] {
            let s = (1.0 + m[1][1] - m[0][0] - m[2][2]).sqrt() * 2.0;
            [(m[0][1] + m[1][0]) / s, 0.25 * s, (m[1][2] + m[2][1]) / s, (m[0][2] - m[2][0]) / s]
        } else {
            let s = (1.0 + m[2][2] - m[0][0] - m[1][1]).sqrt() * 2.0;
            [(m[0][2] + m[2][0]) / s, (m[1][2] + m[2][1]) / s, 0.25 * s, (m[1][0] - m[0][1]) / s]
        };
        let n = (q.iter().map(|x| x * x).sum::<f64>()).sqrt();
        q.map(|x| x / n)
    }

    pub fn translation_for_origin(&self) -> Point {
        self.apply([0.0; 3])
    }
}

pub fn shape(base: &[Point], terms: &[(f64, &SparseTarget)]) -> Result<Vec<Point>, String> {
    crate::character::evaluate_dense(base, terms, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_similarity_inverts_exactly_and_matches_its_linear_form_to_first_order() {
        let s = Similarity::from_linear(&[0.01, -0.02, 0.3, -0.2, 0.03, -0.01, 0.02], [0.0, -0.05, 1.5]);
        for p in [[0.1, -0.1, 1.4], [-0.3, 0.2, 0.9], [0.0, 0.0, 0.0]] {
            let back = s.invert(s.apply(p));
            assert!((0..3).all(|k| (back[k] - p[k]).abs() < 1e-12), "{p:?} -> {back:?}");
        }
        let tiny = Similarity::from_linear(&[0.001, 0.0, -0.002, 0.001, 1e-4, -2e-4, 1e-4], [0.0, 0.0, 1.5]);
        let p = [0.05, -0.08, 1.55];
        let x = [p[0], p[1], p[2] - 1.5];
        let w = [1e-4, -2e-4, 1e-4];
        let lin = [0.001 + 0.001 * x[0] + w[1] * x[2] - w[2] * x[1], 0.001 * x[1] + w[2] * x[0] - w[0] * x[2], -0.002 + 0.001 * x[2] + w[0] * x[1] - w[1] * x[0]];
        let exact = tiny.apply(p);
        assert!((0..3).all(|k| (exact[k] - p[k] - lin[k]).abs() < 1e-6));
    }

    #[test]
    fn the_quaternion_reproduces_the_rotation() {
        let s = Similarity::from_linear(&[0.0, 0.0, 0.0, 0.0, 0.4, -0.7, 0.2], [0.0; 3]);
        let [x, y, z, w] = s.quaternion();
        let m = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
            [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
            [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
        ];
        for i in 0..3 {
            for j in 0..3 {
                assert!((m[i][j] - s.rotation[i][j]).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn a_body_spec_checks_its_references_and_ranges() {
        let r = hash::reference(&"c".repeat(64));
        let ok = BodySpec { pack: r.clone(), targets: r.clone(), weights: BTreeMap::new(), compounds: BTreeMap::new(), blend_mm: 60.0, neck_offset_mm: 0.0, body_scale: 1.0, smooth: 0, tangent: 0.5 };
        assert!(ok.problems().is_empty(), "{:?}", ok.problems());
        let bad = BodySpec { pack: "x".into(), blend_mm: 900.0, weights: [("arms/x".to_string(), 3.0)].into(), compounds: [("wings".to_string(), 0.5), ("athletic".to_string(), 2.0)].into(), ..ok };
        let p = bad.problems().join(" | ");
        for needle in ["body.pack", "blend", "arms/x", "wings", "athletic = 2"] {
            assert!(p.contains(needle), "{needle} not in {p}");
        }
    }

    #[test]
    fn compounds_parse_pair_up_and_expand_linearly() {
        let all = compounds();
        assert!(all.len() >= 10);
        for c in &all {
            assert_eq!(c.plus.len(), c.minus.len(), "{}", c.name);
            for ((p, wp), (m, wm)) in c.plus.iter().zip(&c.minus) {
                assert_eq!(wp, wm, "{}", c.name);
                let flip = |s: &str| s.replace("-incr", "-#").replace("-decr", "-incr").replace("-#", "-decr");
                assert_eq!(&flip(p), m, "{}: {p} pairs with {m}", c.name);
            }
        }
        let r = hash::reference(&"c".repeat(64));
        let mut b = BodySpec { pack: r.clone(), targets: r, weights: [("legs/upperlegs-height-incr".to_string(), 0.25)].into(), compounds: BTreeMap::new(), blend_mm: 60.0, neck_offset_mm: 0.0, body_scale: 1.0, smooth: 0, tangent: 0.5 };
        b.compounds.insert("leg_length".into(), 0.5);
        let w = b.target_weights();
        assert_eq!(w["legs/upperlegs-height-incr"], 0.75);
        assert_eq!(w["legs/lowerlegs-height-incr"], 0.5);
        b.compounds.insert("leg_length".into(), -0.4);
        let w = b.target_weights();
        assert_eq!(w["legs/upperlegs-height-decr"], 0.4);
        assert!(!w.contains_key("legs/lowerlegs-height-incr"));
    }
}
