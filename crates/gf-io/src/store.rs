use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use gf_core::catalog::{
    BlobStore, CATALOG_FORMAT, CatalogEntry, CatalogManifest, GAME_FORMAT, GameLod, GameManifest, HeadOnlySettings, PACK_FORMAT,
    PackManifest, SparseTarget,
};
use gf_core::eyerig::{EyeRigManifest, FORMAT as EYERIG_FORMAT, Sides};
use gf_core::hash;
use gf_geom::decimate::{Decimated, Stats};
use ndarray::{Array1, Array2};
use ndarray_npy::{NpzReader, ReadNpyExt};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::dense::{self, Dense, Manifest};

pub struct Store {
    pub root: PathBuf,
}

pub const KINDS: [&str; 4] = ["packs", "catalogs", "game", "eyerigs"];
const HELPER_GROUPS: [&str; 2] = ["helper-l-eye", "helper-r-eye"];

fn makehuman_axes(x: f64, y: f64, z: f64) -> [f64; 3] {
    [x * 0.1 + 0.0, -z * 0.1 + 0.0, y * 0.1 + 0.0]
}

type Helpers = ([Vec<usize>; 2], Vec<[f64; 3]>);

fn read_helpers(obj: &Path) -> Result<Helpers, String> {
    let text = std::fs::read_to_string(obj).map_err(io(obj))?;
    let mut vertices = Vec::new();
    let mut members: [std::collections::BTreeSet<usize>; 2] = Default::default();
    let mut group: Option<usize> = None;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("v") => {
                let p: Vec<f64> = parts.take(3).map(|t| t.parse::<f64>().map_err(|e| format!("{}: {e}", obj.display()))).collect::<Result<_, _>>()?;
                vertices.push([p[0], p[1], p[2]]);
            }
            Some("g") => group = parts.next().and_then(|g| HELPER_GROUPS.iter().position(|h| *h == g)),
            Some("f") => {
                if let Some(side) = group {
                    for t in parts {
                        let i: usize = t.split('/').next().unwrap_or("").parse().map_err(|e| format!("{}: face index {t}: {e}", obj.display()))?;
                        members[side].insert(i - 1);
                    }
                }
            }
            _ => {}
        }
    }
    let [l, r] = members.map(|m| m.into_iter().collect::<Vec<_>>());
    if l.is_empty() || r.is_empty() {
        return Err(format!("{} has no {} or {} faces", obj.display(), HELPER_GROUPS[0], HELPER_GROUPS[1]));
    }
    if l.iter().chain(&r).any(|&i| i >= vertices.len()) {
        return Err(format!("{}: a helper face indexes past the vertices", obj.display()));
    }
    Ok(([l, r], vertices))
}

fn read_gz_target(path: &Path) -> Result<Vec<(usize, [f64; 3])>, String> {
    use std::io::Read;
    let mut text = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(path).map_err(io(path))?).read_to_string(&mut text).map_err(io(path))?;
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() != 4 || line.starts_with('#') {
            continue;
        }
        let i: usize = parts[0].parse().map_err(|e| format!("{}: {line:?}: {e}", path.display()))?;
        let d: Vec<f64> = parts[1..].iter().map(|t| t.parse::<f64>().map_err(|e| format!("{}: {line:?}: {e}", path.display()))).collect::<Result<_, _>>()?;
        out.push((i, [d[0], d[1], d[2]]));
    }
    Ok(out)
}
const PACK_ARRAYS: [&str; 4] = ["positions", "face_sizes", "corner_vertices", "corner_uvs"];
const GAME_ARRAYS: [&str; 4] = ["kept", "owner", "triangles", "triangle_uvs"];

fn io(path: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io(dir))?;
    }
    let partial = path.with_extension("partial");
    std::fs::write(&partial, bytes).map_err(io(&partial))?;
    std::fs::rename(&partial, path).map_err(io(path))
}

impl BlobStore for Store {
    fn get(&self, hex: &str) -> Result<Vec<u8>, String> {
        let path = self.blob_path(hex);
        let bytes = std::fs::read(&path).map_err(io(&path))?;
        let found = hash::sha256_hex(&bytes);
        if found != hex {
            return Err(format!("blob {hex} is corrupt: its bytes hash to {found}"));
        }
        Ok(bytes)
    }

    fn put(&self, bytes: &[u8]) -> Result<String, String> {
        let hex = hash::sha256_hex(bytes);
        let path = self.blob_path(&hex);
        if !path.exists() {
            write_atomic(&path, bytes)?;
        }
        Ok(hex)
    }
}

impl Store {
    pub fn open(root: &Path) -> Result<Store, String> {
        for kind in KINDS.iter().chain(["blobs"].iter()) {
            let dir = root.join(kind);
            std::fs::create_dir_all(&dir).map_err(io(&dir))?;
        }
        Ok(Store { root: root.to_path_buf() })
    }

    pub fn blob_path(&self, hex: &str) -> PathBuf {
        self.root.join("blobs").join(&hex[..2.min(hex.len())]).join(hex)
    }

    pub fn put_file(&self, path: &Path) -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(io(path))?;
        self.put(&bytes)
    }

    pub fn get_ref(&self, reference: &str) -> Result<Vec<u8>, String> {
        self.get(&hash::parse_reference(reference)?)
    }

    pub fn put_manifest<T: Serialize>(&self, kind: &str, value: &T) -> Result<String, String> {
        let bytes = hash::canonical_bytes(&serde_json::to_value(value).map_err(|e| e.to_string())?);
        let hex = hash::sha256_hex(&bytes);
        let path = self.root.join(kind).join(format!("{hex}.json"));
        if !path.exists() {
            write_atomic(&path, &bytes)?;
        }
        Ok(hash::reference(&hex))
    }

    pub fn get_manifest<T: DeserializeOwned>(&self, kind: &str, reference: &str) -> Result<T, String> {
        let hex = hash::parse_reference(reference)?;
        let path = self.root.join(kind).join(format!("{hex}.json"));
        let bytes = std::fs::read(&path).map_err(|e| format!("{kind} {reference} is not in the store ({}: {e})", path.display()))?;
        let found = hash::sha256_hex(&bytes);
        if found != hex {
            return Err(format!("{kind} manifest {hex} is corrupt: its bytes hash to {found}"));
        }
        serde_json::from_slice(&bytes).map_err(|e| format!("{kind} {reference}: {e}"))
    }

    pub fn import_pack(&self, dir: &Path, macro_base: Option<BTreeMap<String, f64>>, pin_top: Option<f64>) -> Result<(String, PackManifest), String> {
        let loaded = Dense::load(dir)?;
        if let Some(sliders) = &macro_base {
            gf_core::macros::check_sliders(sliders).map_err(|e| format!("macro base: {e}"))?;
        }
        let head_only = match pin_top {
            Some(top) => {
                let boundary = gf_geom::open_boundary(&loaded.triangles);
                gf_core::head::HeadOnly::new(&loaded.positions, &boundary, top)?;
                Some(HeadOnlySettings { pin_top_m: top })
            }
            None => None,
        };
        let mut arrays = BTreeMap::new();
        for name in PACK_ARRAYS {
            arrays.insert(name.to_string(), hash::reference(&self.put_file(&dir.join(format!("{name}.npy")))?));
        }
        let source: Value = serde_json::from_str::<Value>(&std::fs::read_to_string(dir.join("manifest.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?
            .get("source")
            .cloned()
            .unwrap_or(Value::Null);
        let manifest = PackManifest {
            format: PACK_FORMAT.into(),
            vertices: loaded.positions.len(),
            faces: loaded.manifest.faces,
            uv_layer: "head".into(),
            arrays,
            frame: json!({"axes": "x right, -y forward, z up", "units": "m", "nose_tip_vertex": dense::NOSE_TIP_VERTEX}),
            boundary_vertices: gf_geom::open_boundary(&loaded.triangles).len(),
            macro_base,
            head_only,
            source,
        };
        Ok((self.put_manifest("packs", &manifest)?, manifest))
    }

    pub fn load_pack(&self, reference: &str) -> Result<(PackManifest, Dense), String> {
        let manifest: PackManifest = self.get_manifest("packs", reference)?;
        if manifest.format != PACK_FORMAT {
            return Err(format!("pack {reference} has format {:?}", manifest.format));
        }
        let blob = |name: &str| -> Result<Vec<u8>, String> {
            self.get_ref(manifest.arrays.get(name).ok_or_else(|| format!("pack {reference} has no array {name}"))?)
        };
        let npy = |e: ndarray_npy::ReadNpyError| e.to_string();
        let positions: Array2<f32> = Array2::read_npy(Cursor::new(blob("positions")?)).map_err(npy)?;
        let sizes: Array1<u32> = Array1::read_npy(Cursor::new(blob("face_sizes")?)).map_err(npy)?;
        let corners: Array1<u32> = Array1::read_npy(Cursor::new(blob("corner_vertices")?)).map_err(npy)?;
        let uvs: Array2<f32> = Array2::read_npy(Cursor::new(blob("corner_uvs")?)).map_err(npy)?;
        let (positions, triangles, triangle_uvs) = dense::assemble(dense::points_from(&positions)?, &sizes, &corners, &uvs)?;
        if positions.len() != manifest.vertices || sizes.len() != manifest.faces {
            return Err(format!("pack {reference}: the arrays disagree with the manifest"));
        }
        let shell = Manifest { vertices: manifest.vertices, faces: manifest.faces, shapes: Vec::new() };
        Ok((manifest, Dense { dir: self.root.clone(), positions, triangles, triangle_uvs, manifest: shell }))
    }

    pub fn import_catalog(&self, dir: &Path, pack: &str) -> Result<(String, CatalogManifest, u64), String> {
        let (pack_manifest, _) = self.load_pack(pack)?;
        let path = dir.join("catalog.json");
        let source: Value = serde_json::from_str(&std::fs::read_to_string(&path).map_err(io(&path))?).map_err(|e| e.to_string())?;
        let listed = source.get("targets").and_then(Value::as_object).ok_or("catalog.json has no targets object")?;
        let mut targets = BTreeMap::new();
        let mut bytes_in = 0u64;
        for (id, entry) in listed {
            let text = |k: &str| entry.get(k).and_then(Value::as_str).map(str::to_string);
            let file = text("file").ok_or_else(|| format!("{id}: no file"))?;
            let bytes = std::fs::read(dir.join(&file)).map_err(|e| format!("{file}: {e}"))?;
            bytes_in += bytes.len() as u64;
            let target = sparse_from_npz(&bytes).map_err(|e| format!("{file}: {e}"))?;
            target.check(pack_manifest.vertices).map_err(|e| format!("{file}: {e}"))?;
            let blob = hash::reference(&self.put(&bytes)?);
            targets.insert(
                id.clone(),
                CatalogEntry {
                    blob,
                    category: text("category").unwrap_or_default(),
                    name: text("name").unwrap_or_else(|| id.clone()),
                    moved: target.indices.len() as u64,
                    max_mm: entry.get("max_mm").and_then(Value::as_f64).unwrap_or(f64::NAN),
                    units: "mm".into(),
                    license: "CC0".into(),
                    slider: text("slider"),
                    side: text("side"),
                    opposite: text("opposite"),
                    mirror: text("mirror"),
                    macro_family: entry.get("macro").cloned(),
                },
            );
        }
        let not_on_head = source.get("not_on_head").and_then(Value::as_object).map(|m| m.clone().into_iter().collect()).unwrap_or_default();
        let manifest = CatalogManifest {
            format: CATALOG_FORMAT.into(),
            pack: pack.to_string(),
            vertices: pack_manifest.vertices,
            targets,
            not_on_head,
            source_meta: source.get("meta").cloned().unwrap_or(Value::Null),
        };
        Ok((self.put_manifest("catalogs", &manifest)?, manifest, bytes_in))
    }

    pub fn load_catalog(&self, reference: &str) -> Result<CatalogManifest, String> {
        let manifest: CatalogManifest = self.get_manifest("catalogs", reference)?;
        if manifest.format != CATALOG_FORMAT {
            return Err(format!("catalog {reference} has format {:?}", manifest.format));
        }
        Ok(manifest)
    }

    pub fn import_eyerig(&self, makehuman: &Path, catalog: &str) -> Result<(String, EyeRigManifest), String> {
        let catalog_manifest = self.load_catalog(catalog)?;
        let ([left, right], vertices) = read_helpers(&makehuman.join("3dobjs").join("base.obj"))?;
        let slot: BTreeMap<usize, (usize, usize)> = left.iter().enumerate().map(|(k, &i)| (i, (0, k))).chain(right.iter().enumerate().map(|(k, &i)| (i, (1, k)))).collect();
        let base = |ids: &[usize]| ids.iter().map(|&i| makehuman_axes(vertices[i][0], vertices[i][1], vertices[i][2])).collect::<Vec<_>>();
        let mut targets = BTreeMap::new();
        let mut missing = Vec::new();
        for id in catalog_manifest.targets.keys() {
            let path = makehuman.join("targets").join(format!("{id}.target.gz"));
            if !path.is_file() {
                missing.push(id.clone());
                continue;
            }
            let mut sides = [vec![[0.0; 3]; left.len()], vec![[0.0; 3]; right.len()]];
            for (i, d) in read_gz_target(&path)? {
                if let Some(&(side, k)) = slot.get(&i) {
                    sides[side][k] = makehuman_axes(d[0], d[1], d[2]);
                }
            }
            let [l, r] = sides.map(|s| s.iter().any(|d| *d != [0.0; 3]).then_some(s));
            if l.is_some() || r.is_some() {
                targets.insert(id.clone(), Sides { left: l, right: r });
            }
        }
        let manifest = EyeRigManifest {
            format: EYERIG_FORMAT.into(),
            catalog: catalog.to_string(),
            source: format!("MakeHuman base.obj groups {} and {} (CC0), moved by each catalog target's MakeHuman file", HELPER_GROUPS[0], HELPER_GROUPS[1]),
            axes: "metres, MakeHuman (x, -z, y) times 0.1".into(),
            helpers: Sides { left: base(&left), right: base(&right) },
            targets,
            missing,
        };
        Ok((self.put_manifest("eyerigs", &manifest)?, manifest))
    }

    pub fn load_eyerig(&self, reference: &str) -> Result<EyeRigManifest, String> {
        let manifest: EyeRigManifest = self.get_manifest("eyerigs", reference)?;
        if manifest.format != EYERIG_FORMAT {
            return Err(format!("eye rig {reference} has format {:?}", manifest.format));
        }
        Ok(manifest)
    }

    pub fn find_eyerig(&self, catalog: &str) -> Result<Option<String>, String> {
        let dir = self.root.join("eyerigs");
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&dir).map_err(io(&dir))? {
            let path = entry.map_err(io(&dir))?.path();
            let Some(hex) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".json")) else { continue };
            let reference = hash::reference(hex);
            if self.load_eyerig(&reference).is_ok_and(|m| m.catalog == catalog) {
                found.push(reference);
            }
        }
        found.sort();
        if found.len() > 1 {
            return Err(format!("the store holds {} eye rigs for catalog {catalog} ({}); name one", found.len(), found.join(", ")));
        }
        Ok(found.pop())
    }

    pub fn load_target(&self, catalog: &CatalogManifest, id: &str) -> Result<SparseTarget, String> {
        let entry = catalog.targets.get(id).ok_or_else(|| format!("target {id} is not in the catalog"))?;
        let target = sparse_from_npz(&self.get_ref(&entry.blob)?).map_err(|e| format!("{id}: {e}"))?;
        target.check(catalog.vertices).map_err(|e| format!("{id}: {e}"))?;
        Ok(target)
    }

    pub fn import_game(&self, dir: &Path, pack: &str) -> Result<(String, GameManifest), String> {
        let (pack_manifest, _) = self.load_pack(pack)?;
        let mut lods = Vec::new();
        let mut entries: Vec<(usize, PathBuf)> = std::fs::read_dir(dir)
            .map_err(io(dir))?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.strip_prefix("lod_").and_then(|n| n.parse::<usize>().ok()).map(|n| (n, e.path()))
            })
            .collect();
        entries.sort_by(|a, b| b.0.cmp(&a.0));
        for (target, path) in entries {
            let (game, _) = self.import_game(&path, pack)?;
            lods.push(GameLod { target, game });
        }
        let mesh = crate::game_base::load(dir)?;
        if let Some(bad) = mesh.kept.iter().find(|&&k| k as usize >= pack_manifest.vertices) {
            return Err(format!("{}: kept vertex {bad} is outside the pack's {} vertices", dir.display(), pack_manifest.vertices));
        }
        let mut arrays = BTreeMap::new();
        for name in GAME_ARRAYS {
            arrays.insert(name.to_string(), hash::reference(&self.put_file(&dir.join(format!("{name}.npy")))?));
        }
        let report = dir.join("report.json");
        let build = if report.exists() {
            serde_json::from_str(&std::fs::read_to_string(&report).map_err(io(&report))?).map_err(|e| e.to_string())?
        } else {
            Value::Null
        };
        let manifest = GameManifest {
            format: GAME_FORMAT.into(),
            pack: pack.to_string(),
            vertices: mesh.kept.len(),
            triangles: mesh.triangles.len(),
            arrays,
            lods,
            build,
        };
        Ok((self.put_manifest("game", &manifest)?, manifest))
    }

    pub fn load_game(&self, reference: &str) -> Result<(GameManifest, Decimated), String> {
        let manifest: GameManifest = self.get_manifest("game", reference)?;
        let blob = |name: &str| -> Result<Cursor<Vec<u8>>, String> {
            Ok(Cursor::new(self.get_ref(manifest.arrays.get(name).ok_or_else(|| format!("game base {reference} has no {name}"))?)?))
        };
        let npy = |e: ndarray_npy::ReadNpyError| e.to_string();
        let kept: Array1<u32> = Array1::read_npy(blob("kept")?).map_err(npy)?;
        let owner: Array1<u32> = Array1::read_npy(blob("owner")?).map_err(npy)?;
        let tris: Array2<u32> = Array2::read_npy(blob("triangles")?).map_err(npy)?;
        let uvs: Array2<f32> = Array2::read_npy(blob("triangle_uvs")?).map_err(npy)?;
        let mesh = Decimated {
            kept: kept.to_vec(),
            owner: owner.to_vec(),
            triangles: tris.rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect(),
            triangle_uvs: uvs.rows().into_iter().map(|r| [[r[0], r[1]], [r[2], r[3]], [r[4], r[5]]]).collect(),
            stats: Stats::default(),
        };
        Ok((manifest, mesh))
    }
}

pub fn sparse_from_npz(bytes: &[u8]) -> Result<SparseTarget, String> {
    let mut npz = NpzReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let indices: Array1<u32> = npz.by_name("indices.npy").map_err(|e| format!("indices: {e}"))?;
    let deltas: Array2<f32> = npz.by_name("deltas.npy").map_err(|e| format!("deltas: {e}"))?;
    if deltas.ncols() != 3 {
        return Err(format!("deltas are {:?}, expected N x 3", deltas.shape()));
    }
    Ok(SparseTarget { indices: indices.to_vec(), deltas: deltas.rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect() })
}
