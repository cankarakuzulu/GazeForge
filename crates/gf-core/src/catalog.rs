use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CATALOG_FORMAT: &str = "gazeforge.catalog/1";
pub const PACK_FORMAT: &str = "gazeforge.pack/1";
pub const GAME_FORMAT: &str = "gazeforge.game/1";

pub trait BlobStore {
    fn get(&self, hex: &str) -> Result<Vec<u8>, String>;
    fn put(&self, bytes: &[u8]) -> Result<String, String>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct SparseTarget {
    pub indices: Vec<u32>,
    pub deltas: Vec<[f32; 3]>,
}

impl SparseTarget {
    pub fn check(&self, vertices: usize) -> Result<(), String> {
        if self.indices.len() != self.deltas.len() {
            return Err(format!("{} indices but {} deltas", self.indices.len(), self.deltas.len()));
        }
        if let Some(bad) = self.indices.iter().find(|&&i| i as usize >= vertices) {
            return Err(format!("index {bad} is outside the {vertices} dense vertices"));
        }
        if self.indices.windows(2).any(|w| w[0] >= w[1]) {
            return Err("indices are not strictly increasing".into());
        }
        if self.deltas.iter().any(|d| d.iter().any(|x| !x.is_finite())) {
            return Err("a delta is NaN or infinite".into());
        }
        Ok(())
    }

    pub fn add_scaled(&self, weight: f64, into: &mut [[f64; 3]]) {
        for (&i, d) in self.indices.iter().zip(&self.deltas) {
            let p = &mut into[i as usize];
            for k in 0..3 {
                p[k] += weight * d[k] as f64;
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub blob: String,
    pub category: String,
    pub name: String,
    pub moved: u64,
    pub max_mm: f64,
    pub units: String,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opposite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<String>,
    #[serde(default, rename = "macro", skip_serializing_if = "Option::is_none")]
    pub macro_family: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogManifest {
    pub format: String,
    pub pack: String,
    pub vertices: usize,
    pub targets: BTreeMap<String, CatalogEntry>,
    pub not_on_head: BTreeMap<String, Value>,
    pub source_meta: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeadOnlySettings {
    pub pin_top_m: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifest {
    pub format: String,
    pub vertices: usize,
    pub faces: usize,
    pub uv_layer: String,
    pub arrays: BTreeMap<String, String>,
    pub frame: Value,
    pub boundary_vertices: usize,
    pub macro_base: Option<BTreeMap<String, f64>>,
    pub head_only: Option<HeadOnlySettings>,
    pub source: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameLod {
    pub target: usize,
    pub game: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameManifest {
    pub format: String,
    pub pack: String,
    pub vertices: usize,
    pub triangles: usize,
    pub arrays: BTreeMap<String, String>,
    pub lods: Vec<GameLod>,
    pub build: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sparse_target_adds_by_index() {
        let t = SparseTarget { indices: vec![1, 3], deltas: vec![[1.0, 2.0, 3.0], [-1.0, 0.0, 0.5]] };
        t.check(4).unwrap();
        let mut out = vec![[0.0; 3]; 4];
        t.add_scaled(0.5, &mut out);
        assert_eq!(out, vec![[0.0; 3], [0.5, 1.0, 1.5], [0.0; 3], [-0.5, 0.0, 0.25]]);
    }

    #[test]
    fn bad_sparse_targets_are_refused() {
        let t = |i: Vec<u32>, d: Vec<[f32; 3]>| SparseTarget { indices: i, deltas: d };
        assert!(t(vec![0, 4], vec![[0.0; 3]; 2]).check(4).is_err());
        assert!(t(vec![2, 1], vec![[0.0; 3]; 2]).check(4).is_err());
        assert!(t(vec![1, 1], vec![[0.0; 3]; 2]).check(4).is_err());
        assert!(t(vec![1], vec![[0.0; 3]; 2]).check(4).is_err());
        assert!(t(vec![1], vec![[f32::NAN, 0.0, 0.0]]).check(4).is_err());
    }
}
