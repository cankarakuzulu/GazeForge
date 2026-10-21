use std::collections::BTreeMap;
use std::path::Path;

use gf_core::hash;
use gf_core::wearable::Binding;
use serde::{Deserialize, Serialize};

use crate::store::Store;

pub const WEARABLE_FORMAT: &str = "gazeforge.store.wearable/1";
const ALPHA_TEXELS: f64 = 0.01;
const MATERIAL_SLOTS: &[(&str, &str)] = &[("diffuseTexture", "diffuse"), ("normalmapTexture", "normal"), ("aomapTexture", "occlusion")];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WearableManifest {
    pub format: String,
    pub name: String,
    pub kind: String,
    pub binding: String,
    pub mesh: String,
    pub vertices: usize,
    pub hidden: usize,
    pub material: BTreeMap<String, String>,
    #[serde(default)]
    pub transparent: bool,
    #[serde(default)]
    pub double_sided: bool,
    pub source: BTreeMap<String, String>,
    pub notes: Vec<String>,
}

pub struct Wearable {
    pub manifest: WearableManifest,
    pub binding: Binding,
    pub positions: Vec<[f64; 3]>,
    pub uvs: Vec<[f64; 2]>,
    pub faces: Vec<Vec<(u32, Option<u32>)>>,
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

impl Store {
    pub fn import_mhclo(&self, mhclo: &Path) -> Result<(String, WearableManifest), String> {
        let dir = mhclo.parent().unwrap_or(Path::new("."));
        let text = read_text(mhclo)?;
        let binding = Binding::parse(&text).map_err(|e| format!("{}: {e}", mhclo.display()))?;
        let obj_name = binding.keys.get("obj_file").ok_or_else(|| format!("{}: no obj_file line", mhclo.display()))?;
        let obj_path = dir.join(obj_name);
        let mesh = crate::body::read_obj(&obj_path, None)?;
        if mesh.positions.len() != binding.bound.len() {
            return Err(format!("{} binds {} vertices, {} has {}", mhclo.display(), binding.bound.len(), obj_path.display(), mesh.positions.len()));
        }
        let mut material = BTreeMap::new();
        let (mut transparent, mut double_sided) = (false, false);
        let mut source = BTreeMap::from([("mhclo".to_string(), mhclo.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()), ("obj".to_string(), obj_name.clone())]);
        if let Some(mat) = binding.keys.get("material") {
            let mat_path = dir.join(mat);
            source.insert("material".into(), mat.clone());
            for line in read_text(&mat_path)?.lines() {
                let mut parts = line.split_whitespace();
                let (Some(key), Some(file)) = (parts.next(), parts.next()) else { continue };
                match key {
                    "transparent" => transparent = file.eq_ignore_ascii_case("true"),
                    "backfaceCull" => double_sided = file.eq_ignore_ascii_case("false"),
                    _ => {}
                }
                if let Some((_, slot)) = MATERIAL_SLOTS.iter().find(|(k, _)| *k == key) {
                    let path = mat_path.parent().unwrap_or(dir).join(file);
                    if path.is_file() {
                        material.insert(slot.to_string(), hash::reference(&self.put_file(&path)?));
                        source.insert(slot.to_string(), file.to_string());
                    }
                }
            }
        }
        if !transparent && let Some(r) = material.get("diffuse") {
            let (_, _, rgba) = crate::png::decode_rgba(&self.get_ref(r)?)?;
            let see_through = rgba.chunks_exact(4).filter(|p| p[3] < 250).count();
            transparent = see_through as f64 > ALPHA_TEXELS * (rgba.len() / 4) as f64;
        }
        let notes: Vec<String> = text.lines().take_while(|l| l.trim().is_empty() || l.starts_with('#')).map(|l| l.trim_start_matches('#').trim().to_string()).filter(|l| !l.is_empty()).collect();
        let manifest = WearableManifest {
            format: WEARABLE_FORMAT.into(),
            name: binding.keys.get("name").cloned().unwrap_or_else(|| obj_name.trim_end_matches(".obj").to_string()),
            kind: "makehuman".into(),
            binding: hash::reference(&self.put_file(mhclo)?),
            mesh: hash::reference(&self.put_file(&obj_path)?),
            vertices: binding.bound.len(),
            hidden: binding.hidden.len(),
            material,
            transparent,
            double_sided,
            source,
            notes,
        };
        Ok((self.put_manifest("wearables", &manifest)?, manifest))
    }

    pub fn load_wearable(&self, reference: &str) -> Result<Wearable, String> {
        let manifest: WearableManifest = self.get_manifest("wearables", reference)?;
        if manifest.format != WEARABLE_FORMAT {
            return Err(format!("wearable {reference} has format {:?}", manifest.format));
        }
        let binding = Binding::parse(&String::from_utf8_lossy(&self.get_ref(&manifest.binding)?)).map_err(|e| format!("wearable {reference}: {e}"))?;
        let mesh = crate::body::parse_obj(&String::from_utf8_lossy(&self.get_ref(&manifest.mesh)?), None).map_err(|e| format!("wearable {reference}: {e}"))?;
        if mesh.positions.len() != binding.bound.len() {
            return Err(format!("wearable {reference}: binding and mesh disagree on the vertex count"));
        }
        Ok(Wearable { manifest, binding, positions: mesh.positions, uvs: mesh.uvs, faces: mesh.faces })
    }
}
