use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::catalog::SparseTarget;
use crate::eyerig::EyeballSpec;
use crate::hash;
use crate::head::HEAD_SCALE_RANGE;
use crate::macros::{self, Mode};
use crate::placement::PlacementVersion;
use crate::residual;

pub const FORMAT: &str = "gazeforge.character/1";
pub const WEIGHT_STEP: f64 = 1e-6;
pub const WEIGHT_RANGE: [f64; 2] = [0.0, 1.0];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpecBase {
    pub pack: String,
    pub catalog: String,
    #[serde(default)]
    pub game_base: Option<String>,
    pub placement: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MacroSpec {
    pub mode: String,
    pub sliders: BTreeMap<String, f64>,
    pub head_scale: f64,
    pub definition: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResidualRef {
    pub blob: String,
    pub encoding: String,
    pub quantum_mm: f64,
    pub vertices: usize,
    pub bytes: usize,
    pub rms_mm: f64,
    pub max_mm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spec {
    pub format: String,
    pub name: String,
    pub base: SpecBase,
    #[serde(default)]
    pub weights: BTreeMap<String, f64>,
    #[serde(default)]
    pub macros: Option<MacroSpec>,
    #[serde(default)]
    pub residual: Option<ResidualRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eyeballs: Option<EyeballSpec>,
}

pub fn round_weight(w: f64) -> f64 {
    let r = (w / WEIGHT_STEP).round() * WEIGHT_STEP;
    if r == 0.0 { 0.0 } else { r }
}

impl Spec {
    pub fn canonical(&self) -> Spec {
        let mut out = self.clone();
        out.weights = self.weights.iter().map(|(k, &w)| (k.clone(), round_weight(w))).filter(|(_, w)| *w != 0.0).collect();
        out.eyeballs = self.eyeballs.as_ref().map(EyeballSpec::canonical);
        out
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        hash::canonical_bytes(&serde_json::to_value(self.canonical()).expect("a spec always serialises"))
    }

    pub fn revision(&self) -> String {
        hash::reference(&hash::sha256_hex(&self.canonical_bytes()))
    }

    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.format != FORMAT {
            out.push(format!("format is {:?}, expected {FORMAT:?}", self.format));
        }
        if self.name.trim().is_empty() {
            out.push("name is empty".into());
        }
        for (key, value) in [("base.pack", Some(&self.base.pack)), ("base.catalog", Some(&self.base.catalog)), ("base.game_base", self.base.game_base.as_ref())] {
            if let Some(Err(e)) = value.map(|v| hash::parse_reference(v)) {
                out.push(format!("{key}: {e}"));
            }
        }
        if let Err(e) = PlacementVersion::parse(&self.base.placement) {
            out.push(format!("base.placement: {e}"));
        } else if !self.base.placement.contains('/') {
            out.push(format!("base.placement {:?} must name its version, e.g. {}", self.base.placement, crate::placement::DEFAULT));
        }
        for (id, &w) in &self.weights {
            if !(w.is_finite() && (WEIGHT_RANGE[0]..=WEIGHT_RANGE[1]).contains(&w)) {
                out.push(format!("weight {id} = {w} is outside [{}, {}]", WEIGHT_RANGE[0], WEIGHT_RANGE[1]));
            }
        }
        if let Some(m) = &self.macros {
            if let Err(e) = hash::parse_reference(&m.definition) {
                out.push(format!("macros.definition: {e}"));
            }
            if let Err(e) = Mode::parse(&m.mode) {
                out.push(format!("macros.mode: {e}"));
            }
            if let Err(e) = macros::check_sliders(&m.sliders) {
                out.push(format!("macros.sliders: {e}"));
            }
            if !(m.head_scale.is_finite() && (HEAD_SCALE_RANGE[0]..=HEAD_SCALE_RANGE[1]).contains(&m.head_scale)) {
                out.push(format!("macros.head_scale {} is outside [{}, {}]", m.head_scale, HEAD_SCALE_RANGE[0], HEAD_SCALE_RANGE[1]));
            }
        }
        if let Some(r) = &self.residual {
            if let Err(e) = hash::parse_reference(&r.blob) {
                out.push(format!("residual.blob: {e}"));
            }
            if r.encoding != residual::ENCODING {
                out.push(format!("residual.encoding {:?} is not {:?}", r.encoding, residual::ENCODING));
            }
            if r.vertices == 0 || !(r.quantum_mm.is_finite() && r.quantum_mm > 0.0) {
                out.push("residual needs vertices > 0 and a positive quantum".into());
            }
        }
        if let Some(e) = &self.eyeballs {
            out.extend(e.problems());
        }
        out
    }
}

pub fn evaluate_dense(base: &[[f64; 3]], targets: &[(f64, &SparseTarget)], layers: &[&[[f64; 3]]]) -> Result<Vec<[f64; 3]>, String> {
    let mut out = base.to_vec();
    for (w, t) in targets {
        t.check(base.len())?;
        t.add_scaled(*w, &mut out);
    }
    for (n, layer) in layers.iter().enumerate() {
        if layer.len() != base.len() {
            return Err(format!("layer {n} has {} vertices, the base {}", layer.len(), base.len()));
        }
        for (p, d) in out.iter_mut().zip(layer.iter()) {
            for k in 0..3 {
                p[k] += d[k];
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> Spec {
        let r = hash::reference(&"a".repeat(64));
        Spec {
            format: FORMAT.into(),
            name: "probe".into(),
            base: SpecBase { pack: r.clone(), catalog: r.clone(), game_base: None, placement: "f3/1".into() },
            weights: [("nose/x".to_string(), 0.1234567891), ("nose/y".to_string(), 1e-8)].into(),
            macros: None,
            residual: None,
            eyeballs: None,
        }
    }

    #[test]
    fn canonical_rounds_weights_drops_zeros_and_is_idempotent() {
        let c = spec().canonical();
        assert_eq!(c.weights.len(), 1);
        assert_eq!(c.weights["nose/x"], 0.123457);
        assert_eq!(c.canonical(), c);
        assert_eq!(spec().revision(), c.revision());
        let mut moved = spec();
        moved.weights.insert("nose/x".into(), 0.123458);
        assert_ne!(moved.revision(), c.revision());
    }

    #[test]
    fn a_good_spec_has_no_problems_and_bad_fields_are_named() {
        assert!(spec().problems().is_empty(), "{:?}", spec().problems());
        let mut bad = spec();
        bad.base.placement = "f3".into();
        bad.weights.insert("nose/z".into(), 1.5);
        bad.base.pack = "sha256:xyz".into();
        bad.macros = Some(MacroSpec { mode: "MPFB".into(), sliders: BTreeMap::new(), head_scale: 2.0, definition: "x".into() });
        let p = bad.problems().join(" | ");
        for needle in ["base.placement", "nose/z", "base.pack", "macros.mode", "macros.sliders", "head_scale"] {
            assert!(p.contains(needle), "{needle} not in {p}");
        }
    }

    #[test]
    fn unknown_fields_are_refused_on_read() {
        let mut v = serde_json::to_value(spec()).unwrap();
        v["colour"] = serde_json::json!("red");
        assert!(serde_json::from_value::<Spec>(v).is_err());
    }

    #[test]
    fn evaluate_is_base_plus_weighted_targets_plus_layers() {
        let base = vec![[1.0, 2.0, 3.0], [0.0; 3]];
        let t = SparseTarget { indices: vec![1], deltas: vec![[2.0, 0.0, -2.0]] };
        let layer = vec![[0.5, 0.0, 0.0], [0.0, 0.25, 0.0]];
        let out = evaluate_dense(&base, &[(0.5, &t)], &[&layer]).unwrap();
        assert_eq!(out, vec![[1.5, 2.0, 3.0], [1.0, 0.25, -1.0]]);
        assert!(evaluate_dense(&base, &[], &[&layer[..1]]).is_err());
    }
}
