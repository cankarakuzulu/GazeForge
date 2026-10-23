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
        let reference = self.put_manifest("wearables", &manifest)?;
        if let Some(c) = known_category(mhclo_folder(dir).as_deref(), &mhclo_tags(&text)) {
            let _ = self.keep_category(&reference, c);
        }
        Ok((reference, manifest))
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

pub const CATEGORIES: [&str; 10] = ["hair", "facial_hair", "hat", "eyewear", "top", "bottom", "outfit", "shoes", "accessory", "other"];
const CATEGORY_FILE: &str = "wearable_categories.json";

pub fn mhclo_folder(dir: &Path) -> Option<String> {
    dir.ancestors().take(2).filter_map(|p| p.file_name()).map(|n| n.to_string_lossy().to_lowercase()).find(|n| FOLDERS.contains(&n.as_str()))
}

const FOLDERS: [&str; 7] = ["hair", "eyebrows", "eyelashes", "teeth", "tongue", "eyes", "clothes"];

pub fn mhclo_tags(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("tag ")).map(|t| t.trim().to_lowercase()).collect()
}

pub fn known_category(folder: Option<&str>, tags: &[String]) -> Option<&'static str> {
    match folder.map(str::to_lowercase).as_deref() {
        Some("hair") => return Some("hair"),
        Some("eyebrows" | "eyelashes") => return Some("facial_hair"),
        Some("teeth" | "tongue" | "eyes") => return Some("other"),
        _ => {}
    }
    tags.iter().find_map(|t| match t.as_str() {
        "shoes" => Some("shoes"),
        "hat" | "hats" => Some("hat"),
        _ => None,
    })
}

pub fn infer_category(m: &WearableManifest, folder: Option<&str>, tags: &[String]) -> &'static str {
    if let Some(c) = known_category(folder, tags) {
        return c;
    }
    let n = m.name.to_lowercase();
    let tokens: Vec<&str> = n.split(|c: char| !c.is_ascii_alphabetic()).filter(|s| !s.is_empty()).collect();
    let hit = |w: &str| {
        if w.contains('-') {
            n.contains(w)
        } else if let Some(s) = w.strip_prefix('^') {
            tokens.iter().any(|t| t.starts_with(s))
        } else if let Some(s) = w.strip_prefix('*') {
            tokens.iter().any(|t| t.ends_with(s))
        } else if let Some(s) = w.strip_prefix('=') {
            tokens.contains(&s)
        } else if w.len() >= 5 {
            tokens.iter().any(|t| t.starts_with(w) || t.ends_with(w))
        } else {
            tokens.contains(&w)
        }
    };
    let has = |words: &[&str]| words.iter().any(|w| hit(w));
    let rules: [(&str, &[&str]); 10] = [
        ("other", &["teeth", "tooth", "tongue", "eyeball", "eye", "eyes", "*eyes", "cornea", "low-poly", "high-poly"]),
        ("facial_hair", &["beard", "*beard", "moustache", "mustache", "stubble", "goatee", "eyebrow", "eyelash", "brow", "brows"]),
        ("eyewear", &["glasses", "*glasses", "goggles", "spectacles", "monocle"]),
        ("hat", &["hat", "hats", "*hat", "fedora", "helmet", "beanie", "bonnet", "turban", "sombrero"]),
        ("hair", &["hair", "*hair", "^hair", "cards", "=groom", "ponytail", "braid", "afro", "bob", "bun", "dreads"]),
        ("shoes", &["shoe", "shoes", "*shoes", "boot", "*boot", "boots", "sneaker", "sandal", "heels"]),
        ("outfit", &["suit", "*suit", "dress", "*dress", "overall", "jumpsuit", "robe", "*robe", "gown", "*gown"]),
        ("bottom", &["pants", "*pants", "trousers", "jeans", "skirt", "shorts", "leggings"]),
        ("top", &["shirt", "*shirt", "top", "tops", "jacket", "coat", "*coat", "coats", "sweater", "hoodie", "blouse", "vest", "cape", "cloak"]),
        ("accessory", &["sword", "*sword", "shield", "bag", "*bag", "belt", "necklace", "earring", "glove", "gloves", "scarf"]),
    ];
    if let Some((c, _)) = rules.iter().find(|(_, words)| has(words)) {
        return c;
    }
    if m.attach.is_some() {
        "accessory"
    } else if m.transparent {
        "hair"
    } else {
        "other"
    }
}

struct IndexLock(std::path::PathBuf);

impl Drop for IndexLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn parse_categories(bytes: &[u8]) -> Result<BTreeMap<String, String>, String> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Ok(BTreeMap::new());
    }
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let o = v.as_object().ok_or("not a JSON object")?;
    Ok(o.iter().filter_map(|(k, c)| c.as_str().map(|c| (k.clone(), c.to_string()))).collect())
}

impl Store {
    pub fn wearable_categories(&self) -> Result<BTreeMap<String, String>, String> {
        let path = self.root.join(CATEGORY_FILE);
        match std::fs::read(&path) {
            Ok(bytes) => parse_categories(&bytes).map(|all| all.into_iter().filter(|(_, c)| CATEGORIES.contains(&c.as_str())).collect()).map_err(|e| format!("{} is unreadable ({e}); fix or delete it", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    fn index_lock(&self) -> Result<IndexLock, String> {
        let path = self.root.join(format!("{CATEGORY_FILE}.lock"));
        for _ in 0..240 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(IndexLock(path)),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => std::thread::sleep(std::time::Duration::from_millis(25)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() >= 5);
                    if stale {
                        let _ = std::fs::remove_file(&path);
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        }
        Err(format!("{} is held by another writer", path.display()))
    }

    pub fn set_wearable_category(&self, reference: &str, category: Option<&str>) -> Result<Option<String>, String> {
        if let Some(c) = category.filter(|c| !CATEGORIES.contains(c)) {
            return Err(format!("category {c:?} is not one of {CATEGORIES:?}"));
        }
        let _: WearableManifest = self.get_manifest("wearables", reference)?;
        self.update_index(|all, _| match category {
            Some(c) => all.insert(reference.to_string(), c.to_string()),
            None => all.remove(reference),
        })
    }

    fn update_index<T>(&self, f: impl FnOnce(&mut BTreeMap<String, String>, &BTreeMap<String, String>) -> T) -> Result<T, String> {
        let _lock = self.index_lock()?;
        let path = self.root.join(CATEGORY_FILE);
        let raw = match std::fs::read(&path) {
            Ok(bytes) => parse_categories(&bytes).map_err(|e| format!("{} is unreadable ({e}); fix or delete it", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let (mut all, unknown): (BTreeMap<String, String>, BTreeMap<String, String>) = raw.into_iter().partition(|(_, c)| CATEGORIES.contains(&c.as_str()));
        let out = f(&mut all, &unknown);
        for (k, c) in unknown {
            all.entry(k).or_insert(c);
        }
        let bytes = serde_json::to_vec_pretty(&all).map_err(|e| e.to_string())?;
        crate::store::write_atomic(&self.root.join(CATEGORY_FILE), &bytes)?;
        Ok(out)
    }

    pub fn category_of(&self, reference: &str, m: &WearableManifest, known: &BTreeMap<String, String>) -> (String, &'static str) {
        if let Some(c) = known.get(reference).filter(|c| CATEGORIES.contains(&c.as_str())) {
            return (c.clone(), "store");
        }
        let tags = if m.kind == "makehuman" { self.get_ref(&m.binding).map(|b| mhclo_tags(&String::from_utf8_lossy(&b))).unwrap_or_default() } else { Vec::new() };
        (infer_category(m, None, &tags).to_string(), "inferred")
    }

    pub fn keep_category(&self, reference: &str, category: &'static str) -> Result<(), String> {
        self.update_index(|all, unknown| {
            if !unknown.contains_key(reference) {
                all.entry(reference.to_string()).or_insert_with(|| category.to_string());
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(name: &str, kind: &str, transparent: bool, attach: bool) -> WearableManifest {
        WearableManifest {
            format: WEARABLE_FORMAT.into(),
            name: name.into(),
            kind: kind.into(),
            binding: String::new(),
            mesh: String::new(),
            vertices: 0,
            hidden: 0,
            material: BTreeMap::new(),
            transparent,
            double_sided: false,
            source: BTreeMap::new(),
            notes: Vec::new(),
            attach: attach.then(|| Attach { bone: "head".into(), rig: "default".into(), length_m: 0.1, scale_with_bone: false }),
            levels: Vec::new(),
        }
    }

    #[test]
    fn categories_come_from_the_folder_the_tags_then_the_name() {
        let m = |n: &str, t: bool, a: bool| manifest(n, "obj", t, a);
        assert_eq!(infer_category(&m("long01", true, false), Some("hair"), &[]), "hair");
        assert_eq!(infer_category(&m("eyebrow001", false, false), Some("eyebrows"), &[]), "facial_hair");
        assert_eq!(infer_category(&m("shoes01", false, false), Some("clothes"), &["shoes".into()]), "shoes");
        assert_eq!(infer_category(&m("Scruffy_Beard", true, false), None, &[]), "facial_hair");
        assert_eq!(infer_category(&m("cap check", true, false), None, &[]), "hair");
        assert_eq!(infer_category(&m("gate cards", true, false), None, &[]), "hair");
        assert_eq!(infer_category(&m("fedora", false, false), None, &[]), "hat");
        assert_eq!(infer_category(&m("fedora_rigid", false, true), None, &[]), "hat");
        assert_eq!(infer_category(&m("sword", false, true), None, &[]), "accessory");
        assert_eq!(infer_category(&m("male_casualsuit01", false, false), None, &[]), "outfit");
        assert_eq!(infer_category(&m("cape", false, false), None, &[]), "top");
        assert_eq!(infer_category(&m("garment_x", false, false), None, &[]), "other");
        for (name, t) in [("Teeth_Shape02", false), ("tongue01", false), ("HighPolyEyes", true), ("Low-Poly", true)] {
            assert_eq!(infer_category(&m(name, t, false), None, &[]), "other", "{name}");
        }
        assert_eq!(infer_category(&m("eyelashes03", true, false), None, &[]), "facial_hair");
        assert_eq!(infer_category(&m("whatever_shirt", false, false), None, &[]), "top");
        assert_eq!(infer_category(&m("hatchet", false, true), None, &[]), "accessory");
        assert_eq!(infer_category(&m("round glasses", false, true), None, &[]), "eyewear");
        assert_eq!(infer_category(&m("see-through veil", true, false), None, &[]), "hair");
        assert_eq!(known_category(None, &["young".into()]), None);
        for (name, want) in [("brown_jacket", "top"), ("bridegroom_suit", "outfit"), ("longhair01", "hair"), ("sunglasses", "eyewear"), ("cowboyhat", "hat"), ("male_casualsuit01", "outfit"),
                             ("miniskirt01", "bottom"), ("longskirt", "bottom"), ("bluejeans", "bottom"), ("cowboyboots", "shoes"), ("rainboots", "shoes"), ("highheels", "shoes"),
                             ("ballgown", "outfit"), ("nightgown", "outfit"), ("bathrobe", "outfit"), ("leatherjacket", "top"), ("denimjacket", "top"), ("coats", "top"), ("groom cards", "hair")] {
            assert_eq!(infer_category(&m(name, false, false), None, &[]), want, "{name}");
        }
        let kept = parse_categories(b"{\"a\": \"hair\", \"b\": \"cloak_of_tomorrow\", \"c\": null}").unwrap();
        assert_eq!(kept.get("b").map(String::as_str), Some("cloak_of_tomorrow"));
        assert!(!kept.contains_key("c"));
        assert_eq!(mhclo_folder(Path::new("assets/hair/bob01")).as_deref(), Some("hair"));
        assert_eq!(mhclo_folder(Path::new("assets/Eyelashes")).as_deref(), Some("eyelashes"));
        assert_eq!(mhclo_folder(Path::new("downloads/bob01")), None);
        assert_eq!(mhclo_tags("# x\ntag Hats\ntag young\nname y\n"), vec!["hats".to_string(), "young".into()]);
    }

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
