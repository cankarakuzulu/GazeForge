use std::collections::{BTreeMap, HashMap};
use std::io::Cursor;
use std::path::Path;

use gf_core::body::BodyPack;
use gf_core::body_shape::TARGETS_FORMAT;
use gf_core::catalog::{BlobStore, SparseTarget};
use gf_core::hash;
use ndarray::{Array1, Array2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::body::{self as io, Up};
use crate::store::{Store, sparse_from_npz};

pub const BODY_FORMAT: &str = "gazeforge.store.body/1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BodyManifest {
    pub format: String,
    pub files: BTreeMap<String, String>,
    pub pack: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BodyTargetEntry {
    pub blob: String,
    pub family: String,
    pub moved: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BodyTargets {
    pub format: String,
    pub body: String,
    pub vertices: usize,
    pub unit_m: f64,
    pub base_sliders: BTreeMap<String, f64>,
    pub targets: BTreeMap<String, BodyTargetEntry>,
}

fn sparse_npz(t: &SparseTarget) -> Result<Vec<u8>, String> {
    let mut buf = Cursor::new(Vec::new());
    {
        let mut npz = ndarray_npy::NpzWriter::new(&mut buf);
        npz.add_array("indices.npy", &Array1::from(t.indices.clone())).map_err(|e| e.to_string())?;
        let d = Array2::from_shape_vec((t.deltas.len(), 3), t.deltas.iter().flatten().copied().collect()).map_err(|e| e.to_string())?;
        npz.add_array("deltas.npy", &d).map_err(|e| e.to_string())?;
        npz.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, std::path::PathBuf)>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
        let path = e.path();
        if path.is_dir() {
            walk(&path, &rel, out)?;
        } else {
            out.push((rel, path));
        }
    }
    Ok(())
}

impl Store {
    pub fn put_body(&self, dir: &Path) -> Result<(String, BodyManifest), String> {
        let (pack_manifest, _) = io::load(dir)?;
        let mut files = Vec::new();
        walk(dir, "", &mut files)?;
        let mut map = BTreeMap::new();
        for (rel, path) in files {
            map.insert(rel, hash::reference(&self.put_file(&path)?));
        }
        let manifest = BodyManifest { format: BODY_FORMAT.into(), files: map, pack: pack_manifest };
        Ok((self.put_manifest("bodies", &manifest)?, manifest))
    }

    pub fn load_body(&self, reference: &str) -> Result<(Value, BodyPack), String> {
        let manifest: BodyManifest = self.get_manifest("bodies", reference)?;
        if manifest.format != BODY_FORMAT {
            return Err(format!("body {reference} has format {:?}", manifest.format));
        }
        let hex = hash::parse_reference(reference)?;
        let dir = self.root.join("cache").join("bodies").join(&hex);
        if !dir.join("manifest.json").is_file() {
            for (rel, blob) in &manifest.files {
                let path = dir.join(rel);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
                }
                std::fs::write(&path, self.get_ref(blob)?).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
        io::load(&dir)
    }

    pub fn import_body_targets(&self, body: &str, targets_dir: &Path, families: &[String], unit_m: f64) -> Result<(String, BodyTargets), String> {
        let (manifest, pack) = self.load_body(body)?;
        if pack.source.is_empty() {
            return Err(format!("body {body} has no base.obj index per vertex (source.npy): re-import it with body.import (makehuman mode)"));
        }
        let index: HashMap<u32, u32> = pack.source.iter().enumerate().map(|(i, &o)| (o, i as u32)).collect();
        let mut base_sliders = BTreeMap::new();
        for (k, v) in manifest["source"]["sliders"].as_object().into_iter().flatten() {
            base_sliders.insert(k.clone(), v.as_f64().ok_or_else(|| format!("body {body}: base slider {k} is not a number"))?);
        }
        if base_sliders.is_empty() {
            return Err(format!("body {body} records no macro sliders (source.sliders): its base shape is unknown, so macro changes cannot be applied; re-import it with body.import (makehuman mode, targets and macro given)"));
        }
        let mut targets = BTreeMap::new();
        for family in families {
            let dir = targets_dir.join(family);
            let mut files = Vec::new();
            walk(&dir, family, &mut files)?;
            for (rel, path) in files {
                let Some(name) = rel.strip_suffix(".target.gz").or_else(|| rel.strip_suffix(".target")) else { continue };
                let raw = io::read_target(&path)?;
                let mut entries: Vec<(u32, [f64; 3])> = raw.iter().filter_map(|(o, d)| index.get(o).map(|&i| (i, Up::Y.to_frame(*d, unit_m)))).collect();
                entries.sort_by_key(|e| e.0);
                if entries.is_empty() {
                    continue;
                }
                let t = SparseTarget { indices: entries.iter().map(|e| e.0).collect(), deltas: entries.iter().map(|e| e.1.map(|x| x as f32)).collect() };
                let blob = hash::reference(&self.put(&sparse_npz(&t)?)?);
                targets.insert(name.to_string(), BodyTargetEntry { blob, family: family.clone(), moved: t.indices.len() as u64 });
            }
        }
        let m = BodyTargets { format: TARGETS_FORMAT.into(), body: body.to_string(), vertices: pack.mesh.positions.len(), unit_m, base_sliders, targets };
        Ok((self.put_manifest("bodytargets", &m)?, m))
    }

    pub fn load_body_targets(&self, reference: &str) -> Result<BodyTargets, String> {
        let m: BodyTargets = self.get_manifest("bodytargets", reference)?;
        if m.format != TARGETS_FORMAT {
            return Err(format!("body targets {reference} have format {:?}", m.format));
        }
        Ok(m)
    }

    pub fn load_body_target(&self, m: &BodyTargets, id: &str) -> Result<SparseTarget, String> {
        let e = m.targets.get(id).ok_or_else(|| format!("body target {id} is not in the body targets"))?;
        let t = sparse_from_npz(&self.get_ref(&e.blob)?)?;
        t.check(m.vertices)?;
        Ok(t)
    }
}

pub fn summary(m: &BodyTargets) -> Value {
    let mut families: BTreeMap<&str, usize> = BTreeMap::new();
    for e in m.targets.values() {
        *families.entry(e.family.as_str()).or_default() += 1;
    }
    json!({"body": m.body, "vertices": m.vertices, "targets": m.targets.len(), "families": families, "base_sliders": m.base_sliders})
}
