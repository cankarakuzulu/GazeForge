use std::collections::BTreeMap;
use std::path::Path;

use gf_core::catalog::BlobStore;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach: Option<Attach>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<Level>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Level {
    pub from: usize,
    pub first_face: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attach {
    pub bone: String,
    pub rig: String,
    pub length_m: f64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scale_with_bone: bool,
}

pub struct Wearable {
    pub manifest: WearableManifest,
    pub binding: Binding,
    pub positions: Vec<[f64; 3]>,
    pub uvs: Vec<[f64; 2]>,
    pub faces: Vec<Face>,
    pub levels: Vec<FaceSet>,
}

pub type Face = Vec<(u32, Option<u32>)>;

pub type FaceSet = (usize, Vec<Face>);

pub struct Extras {
    pub double_sided: bool,
    pub levels: Vec<Level>,
}

pub fn split_levels(mut faces: Vec<Face>, levels: &[Level]) -> Result<(Vec<Face>, Vec<FaceSet>), String> {
    for (k, l) in levels.iter().enumerate() {
        let after = k.checked_sub(1).map(|j| levels[j]);
        if l.from == 0 || l.first_face == 0 || l.first_face >= faces.len() || after.is_some_and(|a| a.from >= l.from || a.first_face >= l.first_face) {
            return Err(format!("levels[{k}] ({} from face {}) must start after level 0 and after the set before it, within the {} faces", l.from, l.first_face, faces.len()));
        }
    }
    let mut sets = Vec::new();
    for l in levels.iter().rev() {
        sets.push((l.from, faces.split_off(l.first_face)));
    }
    sets.reverse();
    Ok((faces, sets))
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
            attach: None,
            levels: Vec::new(),
        };
        Ok((self.put_manifest("wearables", &manifest)?, manifest))
    }

    pub fn import_attached(&self, name: &str, attach: Attach, local: &[[f64; 3]], obj_path: &Path, diffuse: Option<&Path>, notes: Vec<String>) -> Result<(String, WearableManifest), String> {
        let text = std::fs::read_to_string(obj_path).map_err(|e| format!("{}: {e}", obj_path.display()))?;
        let mut out = String::with_capacity(text.len());
        let mut k = 0;
        for line in text.lines() {
            if line.starts_with("v ") {
                let p = local.get(k).ok_or_else(|| format!("{} has more vertices than were placed", obj_path.display()))?;
                out.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
                k += 1;
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        if k != local.len() {
            return Err(format!("{} has {k} vertices, {} were placed", obj_path.display(), local.len()));
        }
        let mut material = BTreeMap::new();
        let mut transparent = false;
        let mut source = BTreeMap::from([("obj".to_string(), obj_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())]);
        if let Some(d) = diffuse {
            let r = hash::reference(&self.put_file(d)?);
            let (_, _, rgba) = crate::png::decode_rgba(&self.get_ref(&r)?)?;
            transparent = rgba.chunks_exact(4).filter(|p| p[3] < 250).count() as f64 > ALPHA_TEXELS * (rgba.len() / 4) as f64;
            material.insert("diffuse".to_string(), r);
            source.insert("diffuse".into(), d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        }
        let manifest = WearableManifest {
            format: WEARABLE_FORMAT.into(),
            name: name.into(),
            kind: "attach".into(),
            binding: hash::reference(&self.put(b"")?),
            mesh: hash::reference(&self.put(out.as_bytes())?),
            vertices: local.len(),
            hidden: 0,
            material,
            transparent,
            double_sided: false,
            source,
            notes,
            attach: Some(attach),
            levels: Vec::new(),
        };
        Ok((self.put_manifest("wearables", &manifest)?, manifest))
    }

    pub fn import_generated(&self, binding: &Binding, obj_path: &Path, diffuse: Option<&Path>, kind: &str, extras: Extras, notes: Vec<String>) -> Result<(String, WearableManifest), String> {
        let mesh = crate::body::read_obj(obj_path, None)?;
        if mesh.positions.len() != binding.bound.len() {
            return Err(format!("the binding has {} vertices, {} has {}", binding.bound.len(), obj_path.display(), mesh.positions.len()));
        }
        let mut material = BTreeMap::new();
        let mut transparent = false;
        let mut source = BTreeMap::from([("obj".to_string(), obj_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())]);
        if let Some(d) = diffuse {
            let r = hash::reference(&self.put_file(d)?);
            let (_, _, rgba) = crate::png::decode_rgba(&self.get_ref(&r)?)?;
            transparent = rgba.chunks_exact(4).filter(|p| p[3] < 250).count() as f64 > ALPHA_TEXELS * (rgba.len() / 4) as f64;
            material.insert("diffuse".to_string(), r);
            source.insert("diffuse".into(), d.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        }
        let manifest = WearableManifest {
            format: WEARABLE_FORMAT.into(),
            name: binding.keys.get("name").cloned().unwrap_or_else(|| "wearable".into()),
            kind: kind.into(),
            binding: hash::reference(&self.put(binding.to_text().as_bytes())?),
            mesh: hash::reference(&self.put_file(obj_path)?),
            vertices: binding.bound.len(),
            hidden: binding.hidden.len(),
            material,
            transparent,
            double_sided: extras.double_sided,
            source,
            notes,
            attach: None,
            levels: extras.levels,
        };
        Ok((self.put_manifest("wearables", &manifest)?, manifest))
    }

    pub fn load_binding(&self, reference: &str) -> Result<(WearableManifest, Binding), String> {
        let manifest: WearableManifest = self.get_manifest("wearables", reference)?;
        if manifest.format != WEARABLE_FORMAT {
            return Err(format!("wearable {reference} has format {:?}", manifest.format));
        }
        let binding = if manifest.attach.is_some() {
            Binding::default()
        } else {
            Binding::parse(&String::from_utf8_lossy(&self.get_ref(&manifest.binding)?)).map_err(|e| format!("wearable {reference}: {e}"))?
        };
        Ok((manifest, binding))
    }

    pub fn load_wearable(&self, reference: &str) -> Result<Wearable, String> {
        let (manifest, binding) = self.load_binding(reference)?;
        let mesh = crate::body::parse_obj(&String::from_utf8_lossy(&self.get_ref(&manifest.mesh)?), None).map_err(|e| format!("wearable {reference}: {e}"))?;
        if manifest.attach.is_none() && mesh.positions.len() != binding.bound.len() {
            return Err(format!("wearable {reference}: binding and mesh disagree on the vertex count"));
        }
        let (faces, levels) = split_levels(mesh.faces, &manifest.levels).map_err(|e| format!("wearable {reference}: {e}"))?;
        Ok(Wearable { manifest, binding, positions: mesh.positions, uvs: mesh.uvs, faces, levels })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_split_a_mesh_into_face_sets_and_refuse_nonsense() {
        let faces: Vec<Vec<(u32, Option<u32>)>> = (0..6).map(|k| vec![(k, None), (k + 1, None), (k + 2, None)]).collect();
        let (first, sets) = split_levels(faces.clone(), &[Level { from: 2, first_face: 3 }, Level { from: 4, first_face: 5 }]).unwrap();
        assert_eq!(first, faces[..3].to_vec());
        assert_eq!(sets, vec![(2, faces[3..5].to_vec()), (4, faces[5..].to_vec())]);
        assert_eq!(split_levels(faces.clone(), &[]).unwrap(), (faces.clone(), Vec::new()));
        for bad in [
            vec![Level { from: 0, first_face: 3 }],
            vec![Level { from: 2, first_face: 0 }],
            vec![Level { from: 2, first_face: 6 }],
            vec![Level { from: 3, first_face: 2 }, Level { from: 3, first_face: 4 }],
            vec![Level { from: 2, first_face: 4 }, Level { from: 3, first_face: 4 }],
        ] {
            assert!(split_levels(faces.clone(), &bad).is_err(), "{bad:?}");
        }
    }
}
