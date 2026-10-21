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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub genitals: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wearables: Vec<WearableRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WearableRef {
    pub asset: String,
    #[serde(default = "yes")]
    pub hide_body: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub physics: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<f64>,
    #[serde(default = "opening_default", skip_serializing_if = "is_opening_default")]
    pub opening_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coarse_power: Option<f64>,
}

pub const COARSE_POWER_CARDS: f64 = 0.5;

pub const OPENING_DEFAULT_M: f64 = 0.06;

fn opening_default() -> f64 {
    OPENING_DEFAULT_M
}

fn is_opening_default(v: &f64) -> bool {
    *v == OPENING_DEFAULT_M
}

pub const WEARABLE_PHYSICS: &[&str] = &["cloth", "springbone"];

fn yes() -> bool {
    true
}

pub const GENITALS: &[&str] = &["male"];

fn round(x: f64, per_unit: f64) -> f64 {
    let r = (x * per_unit).round() / per_unit;
    if r == 0.0 { 0.0 } else { r }
}

impl BodySpec {
    pub fn canonical(&self) -> BodySpec {
        BodySpec {
            weights: self.weights.iter().map(|(k, &w)| (k.clone(), round(w, 1e6))).filter(|(_, w)| *w != 0.0).collect(),
            compounds: self.compounds.iter().map(|(k, &w)| (k.clone(), round(w, 1e6))).filter(|(_, w)| *w != 0.0).collect(),
            blend_mm: round(self.blend_mm, 1e3),
            neck_offset_mm: round(self.neck_offset_mm, 1e3),
            body_scale: round(self.body_scale, 1e6),
            tangent: round(self.tangent, 1e4),
            genitals: if self.genitals == "none" { String::new() } else { self.genitals.clone() },
            ..self.clone()
        }
    }

    pub fn controls(&self) -> Controls {
        Controls { blend_m: self.blend_mm / 1000.0, neck_offset_m: self.neck_offset_mm / 1000.0, head_scale: 1.0, body_scale: self.body_scale, smooth: self.smooth, tangent: self.tangent }
    }

    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !(self.genitals.is_empty() || self.genitals == "none" || GENITALS.contains(&self.genitals.as_str())) {
            out.push(format!("body.genitals {:?} is not none or one of {GENITALS:?}", self.genitals));
        }
        for (k, v) in [("body.pack", &self.pack), ("body.targets", &self.targets)] {
            if let Err(e) = hash::parse_reference(v) {
                out.push(format!("{k}: {e}"));
            }
        }
        for (i, w) in self.wearables.iter().enumerate() {
            if let Err(e) = hash::parse_reference(&w.asset) {
                out.push(format!("body.wearables[{i}].asset: {e}"));
            }
            if !(w.physics.is_empty() || w.physics == "none" || WEARABLE_PHYSICS.contains(&w.physics.as_str())) {
                out.push(format!("body.wearables[{i}].physics {:?} is not none or one of {WEARABLE_PHYSICS:?}", w.physics));
            }
            if let Some(l) = w.layer
                && !(l.is_finite() && (0.0..=1000.0).contains(&l))
            {
                out.push(format!("body.wearables[{i}].layer {l} is not a number from 0 to 1000"));
            }
            if !(w.opening_m.is_finite() && (0.0..=0.2).contains(&w.opening_m)) {
                out.push(format!("body.wearables[{i}].opening_m {} is not a distance from 0 to 0.2 m", w.opening_m));
            }
            if let Some(p) = w.coarse_power
                && !(p.is_finite() && (0.25..=1.0).contains(&p))
            {
                out.push(format!("body.wearables[{i}].coarse_power {p} is not from 0.25 to 1"));
            }
            if self.wearables[..i].iter().any(|o| o.asset == w.asset) {
                out.push(format!("body.wearables[{i}]: {} is worn twice", w.asset));
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
        for (row, want) in m.iter().zip(&s.rotation) {
            for (a, b) in row.iter().zip(want) {
                assert!((a - b).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn a_body_spec_checks_its_references_and_ranges() {
        let r = hash::reference(&"c".repeat(64));
        let ok = BodySpec { pack: r.clone(), targets: r.clone(), weights: BTreeMap::new(), compounds: BTreeMap::new(), blend_mm: 60.0, neck_offset_mm: 0.0, body_scale: 1.0, smooth: 0, tangent: 0.5, genitals: String::new(), wearables: Vec::new() };
        assert!(ok.problems().is_empty(), "{:?}", ok.problems());
        let bad = BodySpec { pack: "x".into(), blend_mm: 900.0, weights: [("arms/x".to_string(), 3.0)].into(), compounds: [("wings".to_string(), 0.5), ("athletic".to_string(), 2.0)].into(), genitals: "female".into(), ..ok.clone() };
        let p = bad.problems().join(" | ");
        for needle in ["body.pack", "blend", "arms/x", "wings", "athletic = 2", "body.genitals \"female\""] {
            assert!(p.contains(needle), "{needle} not in {p}");
        }
        assert!(BodySpec { genitals: "none".into(), ..ok.clone() }.canonical().genitals.is_empty());
        assert!(BodySpec { genitals: "male".into(), ..ok.clone() }.problems().is_empty());
        let worn = |asset: &str| WearableRef { asset: asset.into(), hide_body: true, physics: String::new(), layer: None, opening_m: OPENING_DEFAULT_M, coarse_power: None };
        let dressed = BodySpec { wearables: vec![worn(&r), worn("nope")], ..ok.clone() };
        let p = dressed.problems();
        assert!(p.iter().any(|x| x.starts_with("body.wearables[1].asset")), "{p:?}");
        assert!(BodySpec { wearables: vec![worn(&r), worn(&r)], ..ok.clone() }.problems().iter().any(|x| x.contains("worn twice")));
        let text = serde_json::to_string(&ok).unwrap();
        assert!(!text.contains("wearables"), "an undressed body keeps its bytes: {text}");
        let read: BodySpec = serde_json::from_str(&format!("{}, \"wearables\": [{{\"asset\": \"{r}\"}}]}}", text.trim_end_matches('}'))).unwrap();
        assert!(read.wearables[0].hide_body, "hide_body defaults to true");
        assert!(read.wearables[0].physics.is_empty());
        let shaken = BodySpec { wearables: vec![WearableRef { physics: "jelly".into(), ..worn(&r) }], ..ok.clone() };
        assert!(shaken.problems().iter().any(|x| x.contains("physics")));
        assert!(BodySpec { wearables: vec![WearableRef { physics: "cloth".into(), ..worn(&r) }], ..ok.clone() }.problems().is_empty());
        assert!(read.wearables[0].layer.is_none(), "no layer means the item's own z_depth");
        assert!(BodySpec { wearables: vec![WearableRef { layer: Some(7.5), ..worn(&r) }], ..ok.clone() }.problems().is_empty());
        for bad in [-1.0, 1000.5, f64::NAN] {
            assert!(BodySpec { wearables: vec![WearableRef { layer: Some(bad), ..worn(&r) }], ..ok.clone() }.problems().iter().any(|x| x.contains("layer")), "{bad}");
        }
        let layered = serde_json::to_string(&WearableRef { layer: Some(7.5), ..worn(&r) }).unwrap();
        assert!(layered.contains("\"layer\":7.5") && !serde_json::to_string(&worn(&r)).unwrap().contains("layer"));
        assert_eq!(read.wearables[0].opening_m, OPENING_DEFAULT_M, "no opening_m means the default reach");
        assert!(!serde_json::to_string(&worn(&r)).unwrap().contains("opening"), "the default is not written");
        assert!(serde_json::to_string(&WearableRef { opening_m: 0.0, ..worn(&r) }).unwrap().contains("\"opening_m\":0.0"));
        for bad in [-0.01, 0.21, f64::INFINITY] {
            assert!(BodySpec { wearables: vec![WearableRef { opening_m: bad, ..worn(&r) }], ..ok.clone() }.problems().iter().any(|x| x.contains("opening_m")), "{bad}");
        }
        assert!(read.wearables[0].coarse_power.is_none(), "no coarse_power means the item's kind decides");
        assert!(BodySpec { wearables: vec![WearableRef { coarse_power: Some(0.75), ..worn(&r) }], ..ok.clone() }.problems().is_empty());
        for bad in [0.2, 1.01, f64::NAN] {
            assert!(BodySpec { wearables: vec![WearableRef { coarse_power: Some(bad), ..worn(&r) }], ..ok.clone() }.problems().iter().any(|x| x.contains("coarse_power")), "{bad}");
        }
    }

    #[test]
    fn canonical_values_are_the_nearest_doubles_of_their_decimals() {
        let r = hash::reference(&"d".repeat(64));
        let b = BodySpec { pack: r.clone(), targets: r, weights: [("a".to_string(), 0.8), ("b".to_string(), 0.30000000000000004)].into(), compounds: [("c".to_string(), -0.7)].into(), blend_mm: 60.1, neck_offset_mm: 0.0, body_scale: 1.1, smooth: 0, tangent: 0.55, genitals: String::new(), wearables: Vec::new() };
        let s = serde_json::to_string(&b.canonical()).unwrap();
        for needle in ["\"a\":0.8,", "\"b\":0.3}", "\"c\":-0.7}", "60.1,", "1.1,", "0.55"] {
            assert!(s.contains(needle), "{needle} not in {s}");
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
                let flip = |s: &str| {
                    [("-incr", "-decr"), ("-decr", "-incr"), ("-up", "-down"), ("-down", "-up")]
                        .iter()
                        .find_map(|(a, b)| s.strip_suffix(a).map(|x| format!("{x}{b}")))
                        .unwrap_or_else(|| s.to_string())
                };
                assert_eq!(&flip(p), m, "{}: {p} pairs with {m}", c.name);
            }
        }
        let r = hash::reference(&"c".repeat(64));
        let mut b = BodySpec { pack: r.clone(), targets: r, weights: [("legs/upperlegs-height-incr".to_string(), 0.25)].into(), compounds: BTreeMap::new(), blend_mm: 60.0, neck_offset_mm: 0.0, body_scale: 1.0, smooth: 0, tangent: 0.5, genitals: String::new(), wearables: Vec::new() };
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
