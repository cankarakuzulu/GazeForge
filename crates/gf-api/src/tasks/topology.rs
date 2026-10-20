use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use gf_core::catalog::{BlobStore, CatalogManifest, GameLod, GameManifest};
use gf_core::hash;
use gf_geom::decimate::Decimated;
use gf_geom::topology::{self as topo, AngleStats, FlipSettings, ShapeSet};
use gf_io::store::Store;
use serde_json::{Value, json};

use super::{Options, count, need, number, only};
use crate::designer::{HEAD_CATEGORIES, base64, f32_base64};

pub const IMPROVE_KEYS: &[&str] = &["store", "game", "catalog", "shape-targets", "samples", "seed", "dense", "below", "min-gain", "max-fold", "max-spread", "rounds", "levels", "out"];
pub const INSPECT_KEYS: &[&str] = &["store", "game"];
const SAMPLE_PICK: f64 = 0.3;
const SAMPLE_EXPRESSION: f64 = 0.5;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.unit() * n as f64) as usize).min(n.saturating_sub(1))
    }
}

struct Level {
    label: String,
    target: Option<usize>,
    reference: String,
    manifest: GameManifest,
    mesh: Decimated,
    index: Vec<u32>,
    set: ShapeSet,
    sparse: HashMap<String, (Vec<u32>, Vec<[f64; 3]>)>,
}

fn open_store(root: &str) -> Result<Store, String> {
    if !Path::new(root).join("game").is_dir() {
        return Err(format!("{root} is not a store (it has no game directory)"));
    }
    Store::open(Path::new(root))
}

fn catalog_for(store: &Store, pack: &str, given: Option<&String>) -> Result<(String, CatalogManifest), String> {
    if let Some(reference) = given {
        let catalog = store.load_catalog(reference)?;
        if catalog.pack != pack {
            return Err(format!("catalog {reference} belongs to pack {}, the game base to {pack}", catalog.pack));
        }
        return Ok((reference.clone(), catalog));
    }
    let dir = store.root.join("catalogs");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let name = entry.map_err(|e| e.to_string())?.file_name().to_string_lossy().to_string();
        let Some(hex) = name.strip_suffix(".json") else { continue };
        let reference = hash::reference(hex);
        if let Ok(c) = store.load_catalog(&reference)
            && c.pack == pack
        {
            found.push((reference, c));
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0));
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(format!("the store has no catalog for pack {pack}; the shape range comes from a catalog")),
        n => Err(format!("the store has {n} catalogs for pack {pack}; name one with --catalog")),
    }
}

fn settings(opts: &Options) -> Result<FlipSettings, String> {
    let d = FlipSettings::default();
    let s = FlipSettings {
        below_deg: number(opts, "below", d.below_deg)?,
        min_gain_deg: number(opts, "min-gain", d.min_gain_deg)?,
        max_fold_deg: number(opts, "max-fold", d.max_fold_deg)?,
        max_rounds: count(opts, "rounds", d.max_rounds)?,
        max_spread_deg: number(opts, "max-spread", d.max_spread_deg)?,
    };
    if s.min_gain_deg <= 0.0 || s.max_fold_deg >= 90.0 || s.below_deg > 60.0 || s.max_rounds == 0 || s.max_spread_deg > 180.0 {
        return Err(format!(
            "--min-gain must be above 0, --max-fold below 90, --below at most 60, --max-spread at most 180 and --rounds at least 1, got {}, {}, {}, {}, {}",
            s.min_gain_deg, s.max_fold_deg, s.below_deg, s.max_spread_deg, s.max_rounds
        ));
    }
    Ok(s)
}

fn combo_target(category: &str) -> bool {
    HEAD_CATEGORIES.contains(&category) || category.starts_with("expression/")
}

fn chosen(mode: &str, category: &str) -> bool {
    match mode {
        "all" => true,
        "head" => combo_target(category) || category.starts_with("macrodetails"),
        _ => false,
    }
}

fn samples(catalog: &CatalogManifest, count: usize, seed: u64) -> Vec<BTreeMap<String, f64>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut expressions = Vec::new();
    for (id, e) in &catalog.targets {
        if HEAD_CATEGORIES.contains(&e.category.as_str()) {
            groups.entry(e.slider.clone().unwrap_or_else(|| id.clone())).or_default().push(id.clone());
        } else if e.category.starts_with("expression/") {
            expressions.push(id.clone());
        }
    }
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    (0..count)
        .map(|_| {
            let mut w = BTreeMap::new();
            for members in groups.values() {
                if rng.unit() < SAMPLE_PICK {
                    let id = &members[rng.below(members.len())];
                    w.insert(id.clone(), rng.unit());
                }
            }
            if !expressions.is_empty() && rng.unit() < SAMPLE_EXPRESSION {
                for _ in 0..1 + rng.below(2) {
                    let id = &expressions[rng.below(expressions.len())];
                    w.insert(id.clone(), rng.unit());
                }
            }
            w
        })
        .collect()
}

fn level(store: &Store, label: String, target: Option<usize>, reference: &str, positions: &[[f64; 3]]) -> Result<Level, String> {
    let (manifest, mesh) = store.load_game(reference)?;
    let mut index = vec![u32::MAX; positions.len()];
    for (g, &k) in mesh.kept.iter().enumerate() {
        if k as usize >= positions.len() {
            return Err(format!("game base {reference}: kept vertex {k} is outside the pack's {} vertices", positions.len()));
        }
        index[k as usize] = g as u32;
    }
    let set = ShapeSet::new(mesh.kept.iter().map(|&k| positions[k as usize]).collect());
    Ok(Level { label, target, reference: reference.to_string(), manifest, mesh, index, set, sparse: HashMap::new() })
}

fn row(name: &str, before: &AngleStats, after: &AngleStats) -> Value {
    json!({"name": name, "below_5": [before.below_5, after.below_5], "below_10": [before.below_10, after.below_10],
           "min_angle_p1": [before.min_angle_p1, after.min_angle_p1], "min_angle_worst": [before.min_angle_worst, after.min_angle_worst],
           "turned": [before.turned, after.turned]})
}

fn f32le(values: &[f64]) -> Vec<u8> {
    values.iter().flat_map(|&v| (v as f32).to_le_bytes()).collect()
}

pub fn improve(opts: &Options) -> Result<Value, String> {
    only(opts, "topology improve", &[IMPROVE_KEYS])?;
    let flip = settings(opts)?;
    let mode = opts.get("shape-targets").map(String::as_str).unwrap_or("all");
    if !["all", "head", "none"].contains(&mode) {
        return Err(format!("--shape-targets must be all, head or none, got {mode}"));
    }
    let levels_mode = opts.get("levels").map(String::as_str).unwrap_or("all");
    if !["all", "finest"].contains(&levels_mode) {
        return Err(format!("--levels must be all or finest, got {levels_mode}"));
    }
    let sample_count = count(opts, "samples", 32)?;
    let seed = count(opts, "seed", 1)? as u64;
    let store = open_store(need(opts, "store")?)?;
    let source = need(opts, "game")?.to_string();
    crate::jobs::progress("load", 0, 1, "game base, pack and catalog");
    let (finest_manifest, _) = store.load_game(&source)?;
    let (catalog_ref, catalog) = catalog_for(&store, &finest_manifest.pack, opts.get("catalog"))?;
    let (_, dense) = store.load_pack(&finest_manifest.pack)?;
    let mut levels = vec![level(&store, "finest".into(), None, &source, &dense.positions)?];
    if levels_mode == "all" {
        for lod in &finest_manifest.lods {
            levels.push(level(&store, format!("lod_{}", lod.target), Some(lod.target), &lod.game, &dense.positions)?);
        }
    }
    crate::jobs::progress("load", 0, 1, "rest surface normals");
    let normals = gf_geom::placement::angle_weighted_normals(&dense.positions, &dense.triangles);
    for l in &mut levels {
        l.set.set_normals(l.mesh.kept.iter().map(|&k| normals[k as usize]).collect())?;
    }
    let mut dense_shapes = Vec::new();
    if let Some(dir) = opts.get("dense") {
        let path = Path::new(dir).join("manifest.json");
        let manifest: gf_io::dense::Manifest =
            serde_json::from_str(&std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?).map_err(|e| format!("{}: {e}", path.display()))?;
        if manifest.vertices != dense.positions.len() {
            return Err(format!("{dir} has {} vertices, the pack {}", manifest.vertices, dense.positions.len()));
        }
        for shape in &manifest.shapes {
            let deltas = gf_io::dense::read_points(&Path::new(dir).join(format!("shape_{}.npy", shape.name)))?;
            if deltas.len() != dense.positions.len() {
                return Err(format!("shape {} has {} vertices, the pack {}", shape.name, deltas.len(), dense.positions.len()));
            }
            for l in &mut levels {
                let indices: Vec<u32> = (0..l.mesh.kept.len() as u32).collect();
                let d: Vec<[f64; 3]> = l.mesh.kept.iter().map(|&k| deltas[k as usize]).collect();
                l.set.add_deltas(&format!("dense:{}", shape.name), &indices, &d)?;
            }
            dense_shapes.push(shape.name.clone());
        }
    }
    let drawn = samples(&catalog, sample_count, seed);
    let in_samples: std::collections::BTreeSet<&String> = drawn.iter().flat_map(|w| w.keys()).collect();
    let mut by_blob: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
    for (id, e) in &catalog.targets {
        if chosen(mode, &e.category) || in_samples.contains(id) {
            by_blob.entry(e.blob.as_str()).or_default().push(id);
        }
    }
    let mut individual = 0;
    let mut aliases = 0;
    let total = by_blob.len() as u64;
    for (i, ids) in by_blob.values().enumerate() {
        let first = ids[0];
        crate::jobs::progress("targets", i as u64, total, first);
        let target = store.load_target(&catalog, first)?;
        let alone = ids.iter().any(|id| chosen(mode, &catalog.targets[*id].category));
        for l in &mut levels {
            let mut indices = Vec::new();
            let mut deltas = Vec::new();
            for (&d, delta) in target.indices.iter().zip(&target.deltas) {
                let g = l.index[d as usize];
                if g != u32::MAX {
                    indices.push(g);
                    deltas.push(delta.map(|x| x as f64));
                }
            }
            if alone {
                l.set.add_deltas(&format!("target:{first}"), &indices, &deltas)?;
            }
            for id in ids.iter().filter(|id| in_samples.contains(**id)) {
                l.sparse.insert((*id).clone(), (indices.clone(), deltas.clone()));
            }
        }
        if alone {
            individual += 1;
            aliases += ids.len() - 1;
        }
    }
    for l in &mut levels {
        let n = l.mesh.kept.len();
        for (s, weights) in drawn.iter().enumerate() {
            let mut sum = vec![[0.0f64; 3]; n];
            let mut touched = vec![false; n];
            for (id, &w) in weights {
                let (indices, deltas) = &l.sparse[id];
                for (&g, d) in indices.iter().zip(deltas) {
                    for k in 0..3 {
                        sum[g as usize][k] += w * d[k];
                    }
                    touched[g as usize] = true;
                }
            }
            let indices: Vec<u32> = (0..n as u32).filter(|&g| touched[g as usize]).collect();
            let deltas: Vec<[f64; 3]> = indices.iter().map(|&g| sum[g as usize]).collect();
            l.set.add_deltas(&format!("sample:{s}"), &indices, &deltas)?;
        }
        l.sparse.clear();
    }
    let shape_set = json!({"rest": 1, "dense_shapes": dense_shapes, "targets": individual, "target_aliases": aliases, "target_mode": mode,
                           "samples": sample_count, "seed": seed, "sample_rule": format!("per head slider (designer categories) a random side with probability {SAMPLE_PICK} at a uniform weight in [0, 1]; with probability {SAMPLE_EXPRESSION} also one or two expression units at uniform weights"),
                           "total": levels[0].set.len(), "entries": levels[0].set.entries()});
    let mut new_lods: Vec<GameLod> = Vec::new();
    let mut level_reports = Vec::new();
    let mut finest = None;
    for l in levels.iter().rev() {
        let stage = format!("improve {}", l.label);
        crate::jobs::progress(&stage, 0, flip.max_rounds as u64, "scoring the shape range");
        let (before_rows, before_all) = topo::shape_stats(&l.set, &l.mesh.triangles);
        let worst_before = topo::worst_angles(&l.set, &l.mesh.triangles);
        let out = topo::improve(&l.set, &l.mesh.triangles, &l.mesh.triangle_uvs, &flip, &mut |r| {
            crate::jobs::progress(&stage, r.round as u64, flip.max_rounds as u64, &format!("round {}: {} flips ({} in all), {} triangles below 10 deg over the shape range", r.round, r.flips, r.total_flips, r.worst_below_10));
        });
        crate::jobs::progress(&stage, flip.max_rounds as u64, flip.max_rounds as u64, "scoring the result");
        let (after_rows, after_all) = topo::shape_stats(&l.set, &out.triangles);
        let worst_after = topo::worst_angles(&l.set, &out.triangles);
        let changed = l.mesh.triangles.iter().zip(&out.triangles).filter(|(a, b)| a != b).count();
        let rows: Vec<Value> = l.set.names().iter().zip(before_rows.iter().zip(&after_rows)).map(|(n, (b, a))| row(n, b, a)).collect();
        let mut named = serde_json::Map::new();
        for (i, name) in l.set.names().iter().enumerate() {
            if i == 0 || name.starts_with("dense:") {
                named.insert(name.clone(), json!({"before": before_rows[i], "after": after_rows[i]}));
            }
        }
        let worst_before_ref = hash::reference(&store.put(&f32le(&worst_before))?);
        let worst_after_ref = hash::reference(&store.put(&f32le(&worst_after))?);
        let summary = json!({"from": l.reference, "catalog": catalog_ref, "settings": flip, "shape_set": shape_set, "flips": out.flips,
                             "rounds": out.rounds.len(), "converged": out.converged, "changed_triangles": changed,
                             "rest": {"before": before_rows[0], "after": after_rows[0]}, "shape_range": {"before": before_all, "after": after_all},
                             "worst_deg_f32le": {"before": worst_before_ref, "after": worst_after_ref}});
        let build = json!({"source": l.reference, "source_build": l.manifest.build, "topology": summary});
        let mesh = Decimated { kept: l.mesh.kept.clone(), owner: l.mesh.owner.clone(), triangles: out.triangles, triangle_uvs: out.triangle_uvs, stats: Default::default() };
        crate::jobs::progress("save", 0, 1, &l.label);
        let lods = match l.target {
            Some(_) => Vec::new(),
            None if levels_mode == "all" => new_lods.iter().rev().cloned().collect(),
            None => l.manifest.lods.clone(),
        };
        let (reference, manifest) = store.put_game(&mesh, &l.manifest.pack, lods, build)?;
        if let Some(dir) = opts.get("out") {
            let dir = match l.target {
                Some(target) => Path::new(dir).join(format!("lod_{target}")),
                None => Path::new(dir).to_path_buf(),
            };
            gf_io::game_base::save(&dir, &mesh)?;
        }
        if manifest.arrays.get("kept") != l.manifest.arrays.get("kept") || manifest.arrays.get("owner") != l.manifest.arrays.get("owner") {
            let (_, check) = store.load_game(&reference)?;
            if check.kept != l.mesh.kept || check.owner != l.mesh.owner {
                return Err(format!("the improved {} changed kept or owner; nothing that names {reference} should be used", l.label));
            }
        }
        let report = json!({"level": l.label, "target": l.target, "from": l.reference, "game": reference, "triangles": mesh.triangles.len(),
                            "flips": out.flips, "changed_triangles": changed, "converged": out.converged, "rounds": out.rounds,
                            "kept_blob_same": manifest.arrays.get("kept") == l.manifest.arrays.get("kept"),
                            "rest": {"before": before_rows[0], "after": after_rows[0]}, "shape_range": {"before": before_all, "after": after_all},
                            "named": named, "shapes": if l.target.is_none() { Value::Array(rows) } else { Value::Null }});
        match l.target {
            Some(target) => new_lods.push(GameLod { target, game: reference.clone() }),
            None => finest = Some(reference.clone()),
        }
        level_reports.push(report);
    }
    level_reports.reverse();
    Ok(json!({"ok": true, "command": "topology improve", "game": finest, "from": source, "catalog": catalog_ref, "settings": flip,
              "shape_set": shape_set, "levels": level_reports,
              "note": "flips only: every game vertex stays an original dense vertex (kept and owner unchanged); the old game base is untouched and characters that name it are unchanged"}))
}

pub fn inspect(opts: &Options) -> Result<Value, String> {
    only(opts, "topology inspect", &[INSPECT_KEYS])?;
    let store = open_store(need(opts, "store")?)?;
    let reference = need(opts, "game")?;
    let (manifest, mesh) = store.load_game(reference)?;
    let (_, dense) = store.load_pack(&manifest.pack)?;
    let mut set_rest = Vec::with_capacity(mesh.kept.len());
    for &k in &mesh.kept {
        set_rest.push(*dense.positions.get(k as usize).ok_or_else(|| format!("kept vertex {k} is outside the pack"))?);
    }
    let set = ShapeSet::new(set_rest);
    let rest = topo::worst_angles(&set, &mesh.triangles);
    let (stats, _) = topo::shape_stats(&set, &mesh.triangles);
    let topology = manifest.build.get("topology").cloned().unwrap_or(Value::Null);
    let stored = |which: &str| -> Result<Value, String> {
        match topology.pointer(&format!("/worst_deg_f32le/{which}")).and_then(Value::as_str) {
            Some(r) => {
                let bytes = store.get_ref(r)?;
                if bytes.len() != mesh.triangles.len() * 4 {
                    return Err(format!("{r} holds {} bytes for {} triangles", bytes.len(), mesh.triangles.len()));
                }
                Ok(json!(base64(&bytes)))
            }
            None => Ok(Value::Null),
        }
    };
    let tris: Vec<u8> = mesh.triangles.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    Ok(json!({"ok": true, "command": "topology inspect", "game": reference, "vertices": mesh.kept.len(), "triangles": mesh.triangles.len(),
              "lods": manifest.lods, "rest": stats[0], "rest_min_angle_f32_b64": f32_base64(rest.iter().copied()),
              "triangles_u32_b64": base64(&tris), "triangle_uvs_f32_b64": f32_base64(mesh.triangle_uvs.iter().flatten().flatten().map(|&x| x as f64)),
              "topology": topology, "worst_after_f32_b64": stored("after")?,
              "definition": "rest: the game base's own vertices at the pack's base positions; topology: the record an improve wrote (absent on a base that was not improved), whose worst_after is each triangle's smallest angle over the shape range it was improved for"}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_are_seeded_and_take_one_side_per_slider() {
        let entry = |category: &str, slider: &str| {
            serde_json::from_value::<gf_core::catalog::CatalogEntry>(json!({"blob": "sha256:00", "category": category, "name": "x", "moved": 1,
                "max_mm": 1.0, "units": "mm", "license": "CC0", "slider": slider}))
            .unwrap()
        };
        let mut targets = BTreeMap::new();
        targets.insert("nose/a-incr".to_string(), entry("nose", "a"));
        targets.insert("nose/a-decr".to_string(), entry("nose", "a"));
        targets.insert("ears/b-incr".to_string(), entry("ears", "b"));
        targets.insert("expression/units/x/smile".to_string(), entry("expression/units/x", "smile"));
        targets.insert("torso/c".to_string(), entry("torso", "c"));
        let catalog = CatalogManifest { format: "f".into(), pack: "p".into(), vertices: 1, targets, not_on_head: BTreeMap::new(), source_meta: Value::Null };
        let a = samples(&catalog, 200, 7);
        assert_eq!(a, samples(&catalog, 200, 7));
        assert_ne!(a, samples(&catalog, 200, 8));
        for w in &a {
            assert!(!(w.contains_key("nose/a-incr") && w.contains_key("nose/a-decr")));
            assert!(!w.contains_key("torso/c"));
            assert!(w.values().all(|&x| (0.0..1.0).contains(&x)));
        }
        assert!(a.iter().any(|w| w.contains_key("expression/units/x/smile")));
        assert!(a.iter().any(|w| w.contains_key("ears/b-incr")));
    }

    #[test]
    fn bad_settings_and_modes_are_refused() {
        let mut o = Options::new();
        o.insert("max-fold".into(), "95".into());
        assert!(settings(&o).is_err());
        let mut o = Options::new();
        o.insert("min-gain".into(), "0".into());
        assert!(settings(&o).is_err());
        assert!(settings(&Options::new()).is_ok());
        let mut o = Options::new();
        o.insert("store".into(), "x".into());
        o.insert("game".into(), "sha256:00".into());
        o.insert("shape-targets".into(), "some".into());
        assert!(improve(&o).unwrap_err().contains("--shape-targets"));
        o.insert("bogus".into(), "1".into());
        assert!(improve(&o).unwrap_err().contains("--bogus"));
    }
}
