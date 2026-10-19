use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use gf_core::catalog::{BlobStore, CatalogManifest, GameManifest, PackManifest, SparseTarget};
use gf_core::character::{FORMAT, MacroSpec, ResidualRef, Spec, SpecBase};
use gf_core::head::{self, HeadOnly};
use gf_core::macros::{MACRO_SLIDERS, Mode, RACES};
use gf_core::measure;
use gf_core::placement::{self as gp, DenseMesh, Placement, PlacementVersion, Prepared};
use gf_core::residual;
use gf_geom::decimate::Decimated;
use gf_geom::lids::ClearSettings;
use gf_geom::placement_f3::{F3Settings, F4Settings};
use gf_geom::placement_f5::{F5Settings, F6Settings};
use gf_io::character::{self as ch, MacroSource};
use gf_io::dense::Dense;
use gf_io::store::Store;
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{obj, shape, text};
use crate::error::ApiError;

pub const HEAD_CATEGORIES: [&str; 10] = ["cheek", "chin", "ears", "eyebrows", "eyes", "forehead", "head", "mouth", "neck", "nose"];
pub const PACK_FORMAT: &str = "gazeforge.preview/1";
pub const DRAFT_ROUNDS: usize = 1;

pub fn f3_defaults() -> F3Settings {
    F3Settings {
        regular: 1e-3,
        max_move: 3.0 / 1000.0,
        smooth: 0.1,
        rounds: 4,
        relax_steps: 24,
        relax_step: 0.5,
        relax_angle_deg: [1.0, 8.0],
        relax_eye_mm: [3.0 / 1000.0, 8.0 / 1000.0],
        backoff_turn_deg: [25.0, 55.0],
        backoff_step: 0.5,
        uv_ring: 2,
        clear: ClearSettings {
            along_gaze: false,
            reach: 2.0 / 1000.0,
            depth: [0.3 / 1000.0, 0.6 / 1000.0],
            clearance: 0.05 / 1000.0,
            front_deg: 75.0,
            rounds: 1,
            samples: 6,
        },
        f4: None,
    }
}

pub fn f4_defaults() -> F4Settings {
    F4Settings { target_deg: 10.0, passes: 12, step: 0.25, gain: 0.5, fold_deg: 60.0, backoff_dense: true, game_plane: true }
}

pub fn f5_defaults() -> F5Settings {
    F5Settings {
        tangential: 0.01,
        smooth: 0.1,
        membrane: 0.1,
        max_move: 3.0 / 1000.0,
        uv_ring: 2,
        subset_normals: true,
        eye_clearance: 0.05 / 1000.0,
        eye_depth: [0.3, 0.6].map(|x: f64| x / 1000.0),
        eye_front_deg: 75.0,
    }
}

pub fn f6_defaults() -> F6Settings {
    gp::f6_defaults(f5_defaults())
}

struct Loaded {
    key: String,
    store: Store,
    pack: PackManifest,
    dense: Dense,
    catalog: CatalogManifest,
    game: Decimated,
    targets: Mutex<HashMap<String, Arc<SparseTarget>>>,
    residuals: Mutex<HashMap<String, Arc<Vec<[f64; 3]>>>>,
    frame: OnceLock<Result<HeadOnly, String>>,
    prepared: OnceLock<Prepared>,
}

static CACHE: OnceLock<Mutex<Option<Arc<Loaded>>>> = OnceLock::new();

fn load(input: &Value) -> Result<(Arc<Loaded>, bool), ApiError> {
    let root = text(input, "store");
    let (pack, catalog, game) = (text(input, "pack"), text(input, "catalog"), text(input, "game"));
    let key = format!("{root}|{pack}|{catalog}|{game}");
    let slot = CACHE.get_or_init(|| Mutex::new(None));
    let mut held = slot.lock().map_err(|_| ApiError::internal("the designer cache lock is poisoned"))?;
    if let Some(l) = held.as_ref().filter(|l| l.key == key) {
        return Ok((l.clone(), true));
    }
    if !Path::new(root).join("packs").is_dir() {
        return Err(ApiError::not_found(format!("{root} is not a store")).details(json!({"store": root})));
    }
    let store = Store::open(Path::new(root)).map_err(ApiError::internal)?;
    let (pack_manifest, dense) = store.load_pack(pack).map_err(ApiError::not_found)?;
    let catalog_manifest = store.load_catalog(catalog).map_err(ApiError::not_found)?;
    let (game_manifest, mesh): (GameManifest, Decimated) = store.load_game(game).map_err(ApiError::not_found)?;
    if catalog_manifest.pack != pack || game_manifest.pack != pack {
        return Err(ApiError::invalid("the catalog and the game base must belong to the pack"));
    }
    let loaded = Arc::new(Loaded {
        key,
        store,
        pack: pack_manifest,
        dense,
        catalog: catalog_manifest,
        game: mesh,
        targets: Mutex::new(HashMap::new()),
        residuals: Mutex::new(HashMap::new()),
        frame: OnceLock::new(),
        prepared: OnceLock::new(),
    });
    *held = Some(loaded.clone());
    Ok((loaded, false))
}

impl Loaded {
    fn target(&self, id: &str) -> Result<Arc<SparseTarget>, ApiError> {
        if let Some(t) = self.targets.lock().map_err(|_| ApiError::internal("lock"))?.get(id) {
            return Ok(t.clone());
        }
        if !self.catalog.targets.contains_key(id) {
            return Err(ApiError::invalid(format!("{id} is not a target of the catalog")).hint("designer.sliders lists the targets"));
        }
        let t = Arc::new(self.store.load_target(&self.catalog, id).map_err(ApiError::internal)?);
        self.targets.lock().map_err(|_| ApiError::internal("lock"))?.insert(id.to_string(), t.clone());
        Ok(t)
    }

    fn residual(&self, r: &ResidualRef) -> Result<Arc<Vec<[f64; 3]>>, ApiError> {
        if let Some(v) = self.residuals.lock().map_err(|_| ApiError::internal("lock"))?.get(&r.blob) {
            return Ok(v.clone());
        }
        let (values, _) = residual::decode(&self.store.get_ref(&r.blob).map_err(ApiError::not_found)?).map_err(ApiError::internal)?;
        if values.len() != self.dense.positions.len() {
            return Err(ApiError::invalid(format!("residual has {} vertices, the pack {}", values.len(), self.dense.positions.len())));
        }
        let v = Arc::new(values);
        self.residuals.lock().map_err(|_| ApiError::internal("lock"))?.insert(r.blob.clone(), v.clone());
        Ok(v)
    }

    fn frame(&self) -> Result<&HeadOnly, ApiError> {
        let settings = self.pack.head_only.as_ref().ok_or_else(|| ApiError::invalid("the pack has no head_only settings"))?;
        self.frame
            .get_or_init(|| HeadOnly::new(&self.dense.positions, &gf_geom::open_boundary(&self.dense.triangles), settings.pin_top_m))
            .as_ref()
            .map_err(|e| ApiError::internal(e.clone()))
    }

    fn prepared(&self) -> &Prepared {
        self.prepared.get_or_init(|| Prepared::new(self.dense.positions.len(), &self.dense.triangles, &self.dense.triangle_uvs, &self.game, f3_defaults().uv_ring))
    }

    fn macro_layer(&self, source: &MacroSource, mode: &str, sliders: &BTreeMap<String, f64>, head_scale: f64) -> Result<Vec<[f64; 3]>, ApiError> {
        let base = self.pack.macro_base.as_ref().ok_or_else(|| ApiError::invalid("the pack has no macro_base"))?;
        let mode = Mode::parse(mode).map_err(ApiError::invalid)?;
        let on_head = |n: &str| self.catalog.targets.contains_key(n);
        let changes = head::weight_changes(&source.definition, mode, sliders, base, &on_head).map_err(ApiError::invalid)?;
        let mut raw = vec![[0.0f64; 3]; self.dense.positions.len()];
        for (id, dw) in &changes {
            self.target(id)?.add_scaled(*dw, &mut raw);
        }
        let frame = self.frame()?;
        let (mut pinned, _) = frame.strip_and_pin(&raw);
        frame.add_head_scale(head_scale, &mut pinned).map_err(ApiError::invalid)?;
        Ok(pinned)
    }

    fn evaluate(&self, spec: &Spec, source: Option<&MacroSource>) -> Result<Vec<[f64; 3]>, ApiError> {
        let spec = spec.canonical();
        let mut loaded = Vec::new();
        for (id, &w) in &spec.weights {
            loaded.push((w, self.target(id)?));
        }
        let terms: Vec<(f64, &SparseTarget)> = loaded.iter().map(|(w, t)| (*w, t.as_ref())).collect();
        let mut layers: Vec<Vec<[f64; 3]>> = Vec::new();
        if let Some(m) = &spec.macros {
            let source = source.ok_or_else(|| ApiError::invalid("the character has macros: pass macro, the macro definition file"))?;
            if source.reference != m.definition {
                return Err(ApiError::invalid(format!("macros.definition is {}, the file given hashes to {}", m.definition, source.reference)));
            }
            layers.push(self.macro_layer(source, &m.mode, &m.sliders, m.head_scale)?);
        }
        let refs: Vec<&[[f64; 3]]> = layers.iter().map(|l| l.as_slice()).collect();
        let mut dense = gf_core::character::evaluate_dense(&self.dense.positions, &terms, &refs).map_err(ApiError::invalid)?;
        if let Some(r) = &spec.residual {
            let values = self.residual(r)?;
            for (p, d) in dense.iter_mut().zip(values.iter()) {
                for k in 0..3 {
                    p[k] += d[k];
                }
            }
        }
        Ok(dense)
    }
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn f32_base64(values: impl Iterator<Item = f64>) -> String {
    let bytes: Vec<u8> = values.flat_map(|v| (v as f32).to_le_bytes()).collect();
    base64(&bytes)
}

fn macro_source(input: &Value) -> Result<Option<MacroSource>, ApiError> {
    match input.get("macro").and_then(Value::as_str) {
        Some(path) if Path::new(path).is_file() => ch::read_macro_definition(Path::new(path)).map(Some).map_err(ApiError::invalid),
        Some(path) => Err(ApiError::not_found(format!("macro definition {path} does not exist"))),
        None => Ok(None),
    }
}

fn number_map(value: Option<&Value>) -> BTreeMap<String, f64> {
    value.and_then(Value::as_object).map(|m| m.iter().filter_map(|(k, v)| v.as_f64().map(|x| (k.clone(), x))).collect()).unwrap_or_default()
}

fn spec_of(input: &Value, loaded: &Loaded, source: Option<&MacroSource>) -> Result<Spec, ApiError> {
    let placement = match input.get("placement").and_then(Value::as_str) {
        Some(given) => PlacementVersion::parse(given).map_err(ApiError::invalid)?,
        None => gp::DEFAULT,
    };
    let macros = match input.get("macros").filter(|m| !m.is_null()) {
        Some(m) => {
            let source = source.ok_or_else(|| ApiError::invalid("macros need macro, the macro definition file"))?;
            let mut sliders = loaded.pack.macro_base.clone().ok_or_else(|| ApiError::invalid("the pack has no macro_base"))?;
            sliders.extend(number_map(m.get("sliders")));
            Some(MacroSpec {
                mode: m.get("mode").and_then(Value::as_str).unwrap_or("gf").to_string(),
                sliders,
                head_scale: m.get("head_scale").and_then(Value::as_f64).unwrap_or(1.0),
                definition: source.reference.clone(),
            })
        }
        None => None,
    };
    let residual = match input.get("residual").filter(|r| !r.is_null()) {
        Some(r) => Some(serde_json::from_value::<ResidualRef>(r.clone()).map_err(|e| ApiError::invalid(format!("residual: {e}")))?),
        None => None,
    };
    let spec = Spec {
        format: FORMAT.into(),
        name: input.get("name").and_then(Value::as_str).unwrap_or("character").to_string(),
        base: SpecBase {
            pack: text(input, "pack").to_string(),
            catalog: text(input, "catalog").to_string(),
            game_base: Some(text(input, "game").to_string()),
            placement: placement.to_string(),
        },
        weights: number_map(input.get("weights")),
        macros,
        residual,
    }
    .canonical();
    let mut problems = spec.problems();
    problems.extend(spec.weights.keys().filter(|id| !loaded.catalog.targets.contains_key(*id)).map(|id| format!("weight {id} is not a target of the catalog")));
    if !problems.is_empty() {
        return Err(ApiError::invalid(format!("the character does not validate: {}", problems.join("; "))).details(json!({"problems": problems})));
    }
    Ok(spec)
}

pub fn draft_defaults() -> F3Settings {
    F3Settings { rounds: DRAFT_ROUNDS, ..f3_defaults() }
}

fn placer(version: PlacementVersion, eyes: Vec<gf_geom::lids::Eye>, draft: bool) -> Result<Box<dyn Placement>, ApiError> {
    let f3 = if draft { draft_defaults() } else { f3_defaults() };
    Ok(match version.name {
        "f3" => Box::new(gp::F3 { settings: f3, eyes }),
        "f4" => Box::new(gp::F3 { settings: F3Settings { f4: Some(f4_defaults()), ..f3 }, eyes }),
        "f5" => Box::new(gp::F5 { settings: f5_defaults(), eyes }),
        "f6" => Box::new(gp::F6 { settings: f6_defaults(), eyes }),
        "subset" => Box::new(gp::Subset),
        "optimal" => Box::new(gp::Optimal { max_move: 3.0 / 1000.0 }),
        other => return Err(ApiError::invalid(format!("designer.settle places with f3, f4, f5, f6, subset or optimal, not {other}"))),
    })
}

fn base_props() -> Value {
    json!({"store": {"type": "string", "minLength": 1, "description": "path of the store directory"},
           "pack": {"type": "string", "minLength": 1, "description": "dense pack reference"},
           "catalog": {"type": "string", "minLength": 1, "description": "target catalog reference"},
           "game": {"type": "string", "minLength": 1, "description": "game base reference"}})
}

fn with(mut base: Value, extra: Value) -> Value {
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

fn character_props() -> Value {
    json!({"placement": {"type": "string", "description": format!("placement name/version, default {}", gp::DEFAULT)},
           "name": {"type": "string", "minLength": 1},
           "weights": {"type": "object", "additionalProperties": {"type": "number", "minimum": 0, "maximum": 1}, "description": "target id -> weight in [0, 1]"},
           "macros": {"type": ["object", "null"], "description": "{sliders: {name: value}, mode: gf|mpfb, head_scale}; sliders not given keep the pack's macro_base"},
           "macro": {"type": "string", "minLength": 1, "description": "path of the macro definition file, needed when macros are set"},
           "residual": {"type": ["object", "null"], "description": "a spec's residual reference, as designer.sliders returns it in start"}})
}

pub fn sliders_input() -> Value {
    obj(
        json!({"store": {"type": "string", "minLength": 1}, "spec": {"type": "string", "minLength": 1, "description": "optional spec file to start from"},
               "macro": {"type": "string", "minLength": 1, "description": "optional macro definition file; without it the macro group is unavailable"}}),
        &["store"],
    )
}

pub fn sliders_output() -> Value {
    shape(json!({"pack": {"type": "string"}, "catalog": {"type": "string"}, "game": {"type": "string"}, "targets": {"type": "array"},
                        "categories": {"type": "object"}, "macros": {"type": "object"}, "start": {"type": "object"}}))
}

pub fn pack_input() -> Value {
    obj(with(base_props(), json!({"targets": {"type": "array", "items": {"type": "string", "minLength": 1}, "description": "default: every head target"}})), &["store", "pack", "catalog", "game"])
}

pub fn pack_output() -> Value {
    shape(json!({"blob": {"type": "string"}, "bytes": {"type": "integer"}, "vertices": {"type": "integer"}, "triangles": {"type": "integer"},
                        "targets": {"type": "array"}, "entries": {"type": "integer"}, "layout": {"type": "object"}, "quantization_max_mm": {"type": "number"}}))
}

pub fn settle_input() -> Value {
    obj(
        with(with(base_props(), character_props()), json!({"eyes": {"type": "string", "minLength": 1, "description": "optional eyes file for the eyelid clearance of f3 and f5"},
                                                           "uvs": {"type": "boolean", "description": "also return the placed triangle UVs (default false)"},
                                                           "fast": {"type": "boolean", "description": "opt-in designer draft for f3/f4: DRAFT_ROUNDS fit rounds instead of 4, so the result is approximate (flagged approximate); designer.save and every export still place with the full released placement (default false)"}})),
        &["store", "pack", "catalog", "game"],
    )
}

pub fn settle_output() -> Value {
    shape(json!({"positions_f32_b64": {"type": "string", "description": "game vertices x,y,z in metres, little-endian f32, base64"},
                        "triangle_uvs_f32_b64": {"type": "string"}, "vertices": {"type": "integer"}, "revision": {"type": "string"},
                        "placement": {"type": "string"}, "timing_ms": {"type": "object"}, "cached": {"type": "boolean"}}))
}

pub fn macro_delta_input() -> Value {
    obj(
        with(with(base_props(), character_props()), json!({"slider": {"type": "string", "minLength": 1}, "step": {"type": "number", "minimum": 0.001, "maximum": 0.5}})),
        &["store", "pack", "catalog", "game", "macro", "slider"],
    )
}

pub fn macro_delta_output() -> Value {
    shape(json!({"up_f32_b64": {"type": "string"}, "down_f32_b64": {"type": "string"}, "value": {"type": "number"}, "step": {"type": "number"}}))
}

pub fn scene_input() -> Value {
    obj(
        with(base_props(), json!({"eyes": {"type": "string", "minLength": 1, "description": "optional eyes file; its eyeballs come back as spheres"}})),
        &["store", "pack", "catalog", "game"],
    )
}

pub fn scene_output() -> Value {
    shape(json!({"vertices": {"type": "integer"}, "triangles": {"type": "integer"}, "lods": {"type": "array"}, "eyes": {"type": "array"}}))
}

pub fn scene(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let (loaded, cached) = load(input)?;
    let game = text(input, "game");
    let manifest: GameManifest = loaded.store.get_manifest("game", game).map_err(ApiError::not_found)?;
    let mut lod0_of = vec![u32::MAX; loaded.dense.positions.len()];
    for (g, &d) in loaded.game.kept.iter().enumerate() {
        lod0_of[d as usize] = g as u32;
    }
    let mut lods = Vec::new();
    for lod in &manifest.lods {
        let (m, mesh) = loaded.store.load_game(&lod.game).map_err(ApiError::not_found)?;
        let mut dropped = 0usize;
        let mut indices: Vec<u32> = Vec::with_capacity(mesh.triangles.len() * 3);
        let mut uvs = Vec::with_capacity(mesh.triangles.len() * 6);
        for (t, uv) in mesh.triangles.iter().zip(&mesh.triangle_uvs) {
            let mapped = t.map(|v| mesh.kept.get(v as usize).map_or(u32::MAX, |&d| lod0_of.get(d as usize).copied().unwrap_or(u32::MAX)));
            if mapped.contains(&u32::MAX) {
                dropped += 1;
            } else {
                indices.extend(mapped);
                uvs.extend(uv.iter().flatten().map(|&x| x as f64));
            }
        }
        let bytes: Vec<u8> = indices.iter().flat_map(|v| v.to_le_bytes()).collect();
        lods.push(json!({"target": lod.target, "game": lod.game, "vertices": m.vertices, "triangles": m.triangles, "dropped_triangles": dropped,
                         "triangles_u32_b64": base64(&bytes), "triangle_uvs_f32_b64": f32_base64(uvs.into_iter())}));
    }
    let eyes: Vec<Value> = match input.get("eyes").and_then(Value::as_str) {
        Some(p) => gf_io::load_eyes(Path::new(p)).map_err(ApiError::invalid)?.iter().map(|e| json!({"centre": e.centre, "radius": e.radius})).collect(),
        None => Vec::new(),
    };
    let base_uvs = f32_base64(loaded.game.triangle_uvs.iter().flatten().flatten().map(|&x| x as f64));
    Ok(json!({"vertices": loaded.game.kept.len(), "triangles": loaded.game.triangles.len(), "triangle_uvs_f32_b64": base_uvs, "lods": lods, "eyes": eyes, "cached": cached,
              "definition": "each coarser level's triangles as indices into the finest game base's vertex order (the levels are nested subsets of it); a triangle with a vertex outside the finest level is dropped and counted"}))
}

pub fn save_input() -> Value {
    obj(with(with(base_props(), character_props()), json!({"out": {"type": "string", "minLength": 1, "description": "path of the spec file to write"}})), &["store", "pack", "catalog", "game", "out"])
}

pub fn save_output() -> Value {
    shape(json!({"out": {"type": "string"}, "revision": {"type": "string"}, "spec": {"type": "object"}}))
}

pub fn sliders(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = text(input, "store");
    if !Path::new(root).join("packs").is_dir() {
        return Err(ApiError::not_found(format!("{root} is not a store")).details(json!({"store": root})));
    }
    let store = Store::open(Path::new(root)).map_err(ApiError::internal)?;
    let spec = match input.get("spec").and_then(Value::as_str) {
        Some(p) => Some(ch::read_spec(Path::new(p)).map_err(ApiError::invalid)?),
        None => None,
    };
    let list = |kind: &str| -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(store.root.join(kind))
            .map(|d| d.filter_map(|e| e.ok()).filter_map(|e| e.file_name().into_string().ok()).filter_map(|n| n.strip_suffix(".json").map(|h| format!("sha256:{h}"))).collect())
            .unwrap_or_default();
        names.sort();
        names
    };
    let (pack, catalog, game) = match &spec {
        Some(s) => (s.base.pack.clone(), s.base.catalog.clone(), s.base.game_base.clone().unwrap_or_default()),
        None => {
            let pack = list("packs").into_iter().next().ok_or_else(|| ApiError::not_found("the store has no pack"))?;
            let catalog = list("catalogs")
                .into_iter()
                .find(|c| store.get_manifest::<CatalogManifest>("catalogs", c).is_ok_and(|m| m.pack == pack))
                .ok_or_else(|| ApiError::not_found("the store has no catalog for its pack"))?;
            let game = list("game")
                .into_iter()
                .filter_map(|g| store.get_manifest::<GameManifest>("game", &g).ok().filter(|m| m.pack == pack).map(|m| (m.vertices, g)))
                .max()
                .map(|(_, g)| g)
                .unwrap_or_default();
            (pack, catalog, game)
        }
    };
    if game.is_empty() {
        return Err(ApiError::not_found("no game base: the spec names none and the store has none for its pack"));
    }
    let pack_manifest: PackManifest = store.get_manifest("packs", &pack).map_err(ApiError::not_found)?;
    let catalog_manifest = store.load_catalog(&catalog).map_err(ApiError::not_found)?;
    let mut categories = BTreeMap::new();
    let targets: Vec<Value> = catalog_manifest
        .targets
        .iter()
        .filter(|(_, e)| HEAD_CATEGORIES.contains(&e.category.as_str()))
        .map(|(id, e)| {
            *categories.entry(e.category.clone()).or_insert(0usize) += 1;
            json!({"id": id, "category": e.category, "slider": e.slider, "side": e.side, "opposite": e.opposite, "mirror": e.mirror,
                   "max_mm": gf_core::measure::round_mm(e.max_mm)})
        })
        .collect();
    let source = macro_source(input)?;
    let reason = match (&pack_manifest.macro_base, &pack_manifest.head_only, &source) {
        (None, _, _) | (_, None, _) => Some("the pack has no macro_base or head_only settings"),
        (_, _, None) => Some("no macro definition file was given (macro)"),
        _ => None,
    };
    let base = pack_manifest.macro_base.clone().unwrap_or_default();
    let now = spec.as_ref().and_then(|s| s.macros.as_ref()).map(|m| m.sliders.clone()).unwrap_or_else(|| base.clone());
    let macro_sliders: Vec<Value> =
        MACRO_SLIDERS.iter().chain(RACES.iter()).map(|n| json!({"name": n, "base": base.get(*n), "value": now.get(*n).or(base.get(*n))})).collect();
    let start = match &spec {
        Some(s) => json!({"name": s.name, "placement": s.base.placement, "weights": s.weights, "macros": s.macros, "residual": s.residual}),
        None => json!({"name": "character", "placement": gp::DEFAULT.to_string(), "weights": {}, "macros": null, "residual": null}),
    };
    Ok(json!({"pack": pack, "catalog": catalog, "game": game, "head_categories": HEAD_CATEGORIES, "categories": categories,
              "targets": targets,
              "macros": {"available": reason.is_none(), "reason": reason, "mode": "gf", "sliders": macro_sliders,
                         "head_only": "option E: the similarity is stripped and the neck seam pinned, so macros change the head only"},
              "start": start}))
}

pub fn pack(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let started = Instant::now();
    let (loaded, cached) = load(input)?;
    let ids: Vec<String> = match input.get("targets").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => loaded.catalog.targets.iter().filter(|(_, e)| HEAD_CATEGORIES.contains(&e.category.as_str())).map(|(id, _)| id.clone()).collect(),
    };
    let mesh = &loaded.game;
    let mut game_of = vec![u32::MAX; loaded.dense.positions.len()];
    for (g, &d) in mesh.kept.iter().enumerate() {
        game_of[d as usize] = g as u32;
    }
    let (mut offsets, mut index, mut delta, mut scale) = (vec![0u32], Vec::new(), Vec::new(), Vec::new());
    let mut worst = 0.0f64;
    for id in &ids {
        let target = loaded.target(id)?;
        let mut rows: Vec<(u32, [f32; 3])> = target
            .indices
            .iter()
            .zip(&target.deltas)
            .filter(|(d, v)| game_of[**d as usize] != u32::MAX && v.iter().any(|x| *x != 0.0))
            .map(|(&d, v)| (game_of[d as usize], *v))
            .collect();
        rows.sort_by_key(|r| r.0);
        let largest = rows.iter().flat_map(|r| r.1).fold(0.0f32, |m, x| m.max(x.abs()));
        let s = if largest > 0.0 { largest / 32767.0 } else { 1.0 };
        for (g, v) in &rows {
            index.push(*g);
            for x in v {
                let q = (x / s).round().clamp(-32767.0, 32767.0) as i16;
                worst = worst.max(((q as f32 * s) - x).abs() as f64);
                delta.push(q);
            }
        }
        scale.push(s);
        offsets.push(index.len() as u32);
    }
    let mut bytes: Vec<u8> = Vec::new();
    let mut layout = serde_json::Map::new();
    let mut section = |name: &str, data: Vec<u8>, kind: &str, count: usize, bytes: &mut Vec<u8>| {
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        layout.insert(name.into(), json!({"offset": bytes.len(), "type": kind, "count": count}));
        bytes.extend(data);
    };
    let tris: Vec<u8> = mesh.triangles.iter().flatten().flat_map(|v| v.to_le_bytes()).collect();
    section("triangles", tris, "u32", mesh.triangles.len() * 3, &mut bytes);
    section("offsets", offsets.iter().flat_map(|v| v.to_le_bytes()).collect(), "u32", offsets.len(), &mut bytes);
    section("index", index.iter().flat_map(|v| v.to_le_bytes()).collect(), "u32", index.len(), &mut bytes);
    section("delta", delta.iter().flat_map(|v| v.to_le_bytes()).collect(), "i16", delta.len(), &mut bytes);
    section("scale", scale.iter().flat_map(|v| v.to_le_bytes()).collect(), "f32", scale.len(), &mut bytes);
    let blob = gf_core::hash::reference(&loaded.store.put(&bytes).map_err(ApiError::internal)?);
    Ok(json!({"format": PACK_FORMAT, "blob": blob, "bytes": bytes.len(), "vertices": mesh.kept.len(), "triangles": mesh.triangles.len(),
              "targets": ids, "entries": index.len(), "layout": layout, "quantization_max_mm": worst * 1000.0,
              "definition": "delta[t] = the target's dense delta at each game vertex's dense index (the game vertices are a subset of the dense ones), i16 times scale[t] metres",
              "cached": cached, "seconds": started.elapsed().as_secs_f64()}))
}

pub fn settle(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let started = Instant::now();
    let (loaded, cached) = load(input)?;
    let t_load = started.elapsed().as_secs_f64();
    let source = macro_source(input)?;
    let spec = spec_of(input, &loaded, source.as_ref())?;
    let eyes = match input.get("eyes").and_then(Value::as_str) {
        Some(p) => gf_io::load_eyes(Path::new(p)).map_err(ApiError::invalid)?,
        None => Vec::new(),
    };
    let version = PlacementVersion::parse(&spec.base.placement).map_err(ApiError::invalid)?;
    let draft = input.get("fast").and_then(Value::as_bool).unwrap_or(false) && matches!(version.name, "f3" | "f4");
    let placement = placer(version, eyes, draft)?;
    let t0 = started.elapsed().as_secs_f64();
    let dense = loaded.evaluate(&spec, source.as_ref())?;
    let t1 = started.elapsed().as_secs_f64();
    let mut measurements = serde_json::Map::new();
    if dense.len() == measure::BASE_VERTICES {
        for (name, pair) in measure::default_pairs() {
            let (Ok(now), Ok(was)) = (measure::distance_mm(&dense, pair), measure::distance_mm(&loaded.dense.positions, pair)) else {
                continue;
            };
            let what = measure::MEASURES.iter().find(|m| m.name == name).map(|m| m.what);
            measurements.insert(name, json!({"mm": measure::round_mm(now), "base_mm": measure::round_mm(was), "delta_mm": measure::round_mm(now - was), "what": what}));
        }
    }
    let ready = loaded.prepared.get().is_some();
    let prepared = loaded.prepared();
    let t_prepared = started.elapsed().as_secs_f64();
    let placed = placement.place_prepared(&dense, &DenseMesh { triangles: &loaded.dense.triangles, triangle_uvs: &loaded.dense.triangle_uvs }, &loaded.game, prepared);
    let t2 = started.elapsed().as_secs_f64();
    let mut out = json!({"vertices": placed.game.len(), "revision": spec.revision(), "placement": placement.version().to_string(),
                         "positions_f32_b64": f32_base64(placed.game.iter().flatten().copied()), "cached": cached, "approximate": draft,
                         "draft": if draft { json!({"rounds": DRAFT_ROUNDS, "full_rounds": f3_defaults().rounds, "note": "designer-only draft; save and export use the full placement"}) } else { Value::Null }});
    if input.get("uvs").and_then(Value::as_bool).unwrap_or(false) {
        let uvs = placed.triangle_uvs.unwrap_or_else(|| loaded.game.triangle_uvs.clone());
        out["triangle_uvs_f32_b64"] = json!(f32_base64(uvs.iter().flatten().flatten().map(|&x| x as f64)));
    }
    let t3 = started.elapsed().as_secs_f64();
    out["timing_ms"] = json!({"load": t_load * 1000.0, "evaluate": (t1 - t0) * 1000.0, "prepare": (t_prepared - t1) * 1000.0, "place": (t2 - t_prepared) * 1000.0,
                             "encode": (t3 - t2) * 1000.0, "total": t3 * 1000.0, "prepared_cached": ready});
    out["measurements"] = Value::Object(measurements);
    out["report"] = Value::Object(placed.report);
    Ok(out)
}

pub fn macro_delta(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let started = Instant::now();
    let (loaded, _) = load(input)?;
    let source = macro_source(input)?.ok_or_else(|| ApiError::invalid("macro_delta needs macro, the macro definition file"))?;
    let spec = spec_of(&with(input.clone(), json!({"macros": input.get("macros").cloned().filter(|m| !m.is_null()).unwrap_or(json!({}))})), &loaded, Some(&source))?;
    let m = spec.macros.clone().expect("set above");
    let slider = text(input, "slider");
    let value = *m.sliders.get(slider).ok_or_else(|| ApiError::invalid(format!("{slider} is not a macro slider of the pack")))?;
    let step = input.get("step").and_then(Value::as_f64).unwrap_or(0.05);
    let at = |v: f64| -> Result<Vec<[f64; 3]>, ApiError> {
        let mut sliders = m.sliders.clone();
        sliders.insert(slider.to_string(), v);
        let layer = loaded.macro_layer(&source, &m.mode, &sliders, m.head_scale)?;
        Ok(loaded.game.kept.iter().map(|&d| layer[d as usize]).collect())
    };
    let (hi, lo) = ((value + step).min(1.0), (value - step).max(0.0));
    let mid = at(value)?;
    let slope = |far: &[[f64; 3]], span: f64| -> String {
        if span <= 0.0 {
            return f32_base64(std::iter::repeat_n(0.0, mid.len() * 3));
        }
        f32_base64(far.iter().zip(&mid).flat_map(|(f, c)| [0, 1, 2].map(|k| (f[k] - c[k]) / span)))
    };
    let up = slope(&at(hi)?, hi - value);
    let down = slope(&at(lo)?, value - lo);
    Ok(json!({"slider": slider, "value": value, "step": step, "up_f32_b64": up, "down_f32_b64": down, "vertices": mid.len(),
              "definition": "metres per unit slider at each game vertex: (L(v + step) - L(v)) / step and (L(v) - L(v - step)) / step, clamped to [0, 1]",
              "seconds": started.elapsed().as_secs_f64()}))
}

pub fn save(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let (loaded, _) = load(input)?;
    let source = macro_source(input)?;
    let spec = spec_of(input, &loaded, source.as_ref())?;
    let (problems, _) = ch::problems(&loaded.store, &spec, source.as_ref());
    if !problems.is_empty() {
        return Err(ApiError::invalid(format!("the spec does not validate: {}", problems.join("; "))).details(json!({"problems": problems})));
    }
    let out = text(input, "out");
    let revision = ch::write_spec(Path::new(out), &spec).map_err(ApiError::internal)?;
    Ok(json!({"out": out, "revision": revision, "spec": serde_json::to_value(&spec).map_err(|e| ApiError::internal(e.to_string()))?}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (raw, enc) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(raw.as_bytes()), enc);
        }
    }

    #[test]
    fn the_head_set_names_ten_categories() {
        assert_eq!(HEAD_CATEGORIES.len(), 10);
        assert!(!HEAD_CATEGORIES.contains(&"asym"));
    }
}
