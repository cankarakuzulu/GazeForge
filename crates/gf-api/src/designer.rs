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
               "character": {"type": "object", "description": "optional spec object to start from (instead of spec)"},
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
        with(with(base_props(), character_props()), json!({"slider": {"type": "string", "minLength": 1}, "step": {"type": "number", "minimum": 0.001, "maximum": 0.5},
                                                           "sampled": {"type": "boolean", "description": "return samples instead of the two slopes: at every part boundary of the slider plus a grid of sample_spacing, for piecewise-linear interpolation (default false)"},
                                                           "sample_spacing": {"type": "number", "minimum": 0.01, "maximum": 1, "description": "grid spacing of the samples (default 0.25)"},
                                                           "sample_form": {"type": "string", "enum": ["placed", "layer"], "description": "placed (default): the settled game positions of the character at each sample value, as designer.settle places them; layer: the macro layer alone at the game vertices, cheaper, without the placement"},
                                                           "eyes": {"type": "string", "minLength": 1, "description": "placed samples: the eyes file the settle uses"},
                                                           "fast": {"type": "boolean", "description": "placed samples: the designer draft placement, as in designer.settle"}})),
        &["store", "pack", "catalog", "game", "macro", "slider"],
    )
}

pub fn macro_delta_output() -> Value {
    shape(json!({"up_f32_b64": {"type": "string"}, "down_f32_b64": {"type": "string"}, "value": {"type": "number"}, "step": {"type": "number"},
                 "values": {"type": "array", "description": "sampled: the slider values, ascending, 0 and 1 included"},
                 "layers_f32_b64": {"type": "array", "description": "sampled: per value, the macro layer at every game vertex in metres"}}))
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

pub struct SceneLod {
    pub target: usize,
    pub game: String,
    pub vertices: usize,
    pub triangles: usize,
    pub dropped_triangles: usize,
    pub indices: Vec<u32>,
    pub triangle_uvs: Vec<f64>,
}

pub struct SceneMesh {
    pub vertices: usize,
    pub triangles: Vec<[u32; 3]>,
    pub triangle_uvs: Vec<[gf_geom::Uv; 3]>,
    pub lods: Vec<SceneLod>,
    pub cached: bool,
}

pub fn scene_mesh(input: &Value) -> Result<SceneMesh, ApiError> {
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
        lods.push(SceneLod { target: lod.target, game: lod.game.clone(), vertices: m.vertices, triangles: m.triangles, dropped_triangles: dropped, indices, triangle_uvs: uvs });
    }
    Ok(SceneMesh { vertices: loaded.game.kept.len(), triangles: loaded.game.triangles.clone(), triangle_uvs: loaded.game.triangle_uvs.clone(), lods, cached })
}

pub fn scene(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let mesh = scene_mesh(input)?;
    let lods: Vec<Value> = mesh
        .lods
        .iter()
        .map(|l| {
            let bytes: Vec<u8> = l.indices.iter().flat_map(|v| v.to_le_bytes()).collect();
            json!({"target": l.target, "game": l.game, "vertices": l.vertices, "triangles": l.triangles, "dropped_triangles": l.dropped_triangles,
                   "triangles_u32_b64": base64(&bytes), "triangle_uvs_f32_b64": f32_base64(l.triangle_uvs.iter().copied())})
        })
        .collect();
    let eyes: Vec<Value> = match input.get("eyes").and_then(Value::as_str) {
        Some(p) => gf_io::load_eyes(Path::new(p)).map_err(ApiError::invalid)?.iter().map(|e| json!({"centre": e.centre, "radius": e.radius})).collect(),
        None => Vec::new(),
    };
    let base_uvs = f32_base64(mesh.triangle_uvs.iter().flatten().flatten().map(|&x| x as f64));
    Ok(json!({"vertices": mesh.vertices, "triangles": mesh.triangles.len(), "triangle_uvs_f32_b64": base_uvs, "lods": lods, "eyes": eyes, "cached": mesh.cached,
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
    let spec = match (input.get("character").filter(|c| c.is_object()), input.get("spec").and_then(Value::as_str)) {
        (Some(c), _) => Some(serde_json::from_value::<Spec>(c.clone()).map_err(|e| ApiError::invalid(format!("character: {e}")))?),
        (None, Some(p)) => Some(ch::read_spec(Path::new(p)).map_err(ApiError::invalid)?),
        (None, None) => None,
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

pub struct TargetDeltas {
    pub vertices: usize,
    pub ids: Vec<String>,
    pub offsets: Vec<u32>,
    pub index: Vec<u32>,
    pub delta: Vec<[f32; 3]>,
}

pub fn preview_deltas(input: &Value) -> Result<TargetDeltas, ApiError> {
    let (loaded, _) = load(input)?;
    let ids: Vec<String> = match input.get("targets").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => loaded.catalog.targets.iter().filter(|(_, e)| HEAD_CATEGORIES.contains(&e.category.as_str())).map(|(id, _)| id.clone()).collect(),
    };
    let mesh = &loaded.game;
    let mut game_of = vec![u32::MAX; loaded.dense.positions.len()];
    for (g, &d) in mesh.kept.iter().enumerate() {
        game_of[d as usize] = g as u32;
    }
    let per_target = gf_geom::map_range(ids.len(), |i| -> Result<Vec<(u32, [f32; 3])>, ApiError> {
        let target = loaded.target(&ids[i])?;
        let mut rows: Vec<(u32, [f32; 3])> = target
            .indices
            .iter()
            .zip(&target.deltas)
            .filter(|(d, v)| game_of[**d as usize] != u32::MAX && v.iter().any(|x| *x != 0.0))
            .map(|(&d, v)| (game_of[d as usize], *v))
            .collect();
        rows.sort_by_key(|r| r.0);
        Ok(rows)
    });
    let (mut offsets, mut index, mut delta) = (vec![0u32], Vec::new(), Vec::new());
    for rows in per_target {
        for (g, v) in rows? {
            index.push(g);
            delta.push(v);
        }
        offsets.push(index.len() as u32);
    }
    Ok(TargetDeltas { vertices: mesh.kept.len(), ids, offsets, index, delta })
}

pub struct MacroSlope {
    pub value: f64,
    pub step: f64,
    pub up: Vec<f32>,
    pub down: Vec<f32>,
    pub vertices: usize,
}

struct MacroAt {
    loaded: Arc<Loaded>,
    source: MacroSource,
    macros: MacroSpec,
    slider: String,
    value: f64,
}

impl MacroAt {
    fn new(input: &Value) -> Result<MacroAt, ApiError> {
        let (loaded, _) = load(input)?;
        let source = macro_source(input)?.ok_or_else(|| ApiError::invalid("macro_delta needs macro, the macro definition file"))?;
        let spec = spec_of(&with(input.clone(), json!({"macros": input.get("macros").cloned().filter(|m| !m.is_null()).unwrap_or(json!({}))})), &loaded, Some(&source))?;
        let macros = spec.macros.clone().expect("set above");
        let slider = text(input, "slider").to_string();
        let value = *macros.sliders.get(&slider).ok_or_else(|| ApiError::invalid(format!("{slider} is not a macro slider of the pack")))?;
        Ok(MacroAt { loaded, source, macros, slider, value })
    }

    fn layer(&self, v: f64) -> Result<Vec<[f64; 3]>, ApiError> {
        let m = &self.macros;
        let mut sliders = m.sliders.clone();
        sliders.insert(self.slider.clone(), v);
        let layer = self.loaded.macro_layer(&self.source, &m.mode, &sliders, m.head_scale)?;
        Ok(self.loaded.game.kept.iter().map(|&d| layer[d as usize]).collect())
    }
}

pub const MACRO_SAMPLE_SPACING: f64 = 0.25;
const SAMPLE_MERGE: f64 = 1e-4;

pub fn macro_sample_values(source: &MacroSource, slider: &str, spacing: f64) -> Vec<f64> {
    let mut knots: Vec<f64> = vec![0.0, 1.0];
    if let Some(parts) = source.definition.parts.get(slider) {
        knots.extend(parts.iter().flat_map(|p| [p.lowest, p.highest]).filter(|x| *x > 0.0 && *x < 1.0));
    }
    knots.sort_by(f64::total_cmp);
    let mut values: Vec<f64> = Vec::new();
    for k in knots {
        if values.last().is_none_or(|l| k - l > SAMPLE_MERGE) {
            values.push(k);
        }
    }
    let n = (1.0 / spacing.clamp(0.01, 1.0)).round().max(1.0) as usize;
    for g in (1..n).map(|i| i as f64 / n as f64) {
        if values.iter().all(|v| (v - g).abs() > SAMPLE_MERGE) {
            values.push(g);
        }
    }
    values.sort_by(f64::total_cmp);
    values
}

pub struct MacroSamples {
    pub value: f64,
    pub spacing: f64,
    pub placed: bool,
    pub values: Vec<f64>,
    pub layers: Vec<Vec<f32>>,
    pub vertices: usize,
}

pub fn macro_samples(input: &Value) -> Result<MacroSamples, ApiError> {
    let at = MacroAt::new(input)?;
    let spacing = input.get("sample_spacing").and_then(Value::as_f64).unwrap_or(MACRO_SAMPLE_SPACING);
    let placed = match input.get("sample_form").and_then(Value::as_str).unwrap_or("placed") {
        "placed" => true,
        "layer" => false,
        other => return Err(ApiError::invalid(format!("sample_form {other} is neither placed nor layer"))),
    };
    let values = macro_sample_values(&at.source, &at.slider, spacing);
    let mut layers = Vec::with_capacity(values.len());
    for &v in &values {
        let rows = if placed {
            let mut sliders = at.macros.sliders.clone();
            sliders.insert(at.slider.clone(), v);
            let mut one = input.clone();
            one["macros"] = json!({"sliders": sliders, "mode": at.macros.mode, "head_scale": at.macros.head_scale});
            if let Some(o) = one.as_object_mut() {
                o.remove("uvs");
            }
            settle_mesh(&one)?.positions
        } else {
            at.layer(v)?
        };
        layers.push(rows.iter().flatten().map(|&x| x as f32).collect::<Vec<f32>>());
    }
    Ok(MacroSamples { value: at.value, spacing, placed, values, layers, vertices: at.loaded.game.kept.len() })
}

pub fn macro_slope(input: &Value) -> Result<MacroSlope, ApiError> {
    let m = MacroAt::new(input)?;
    let value = m.value;
    let step = input.get("step").and_then(Value::as_f64).unwrap_or(0.05);
    let at = |v: f64| m.layer(v);
    let (hi, lo) = ((value + step).min(1.0), (value - step).max(0.0));
    let mid = at(value)?;
    let slope = |far: &[[f64; 3]], span: f64| -> Vec<f32> {
        if span <= 0.0 {
            return vec![0.0; mid.len() * 3];
        }
        far.iter().zip(&mid).flat_map(|(f, c)| [0, 1, 2].map(|k| ((f[k] - c[k]) / span) as f32)).collect()
    };
    let up = slope(&at(hi)?, hi - value);
    let down = slope(&at(lo)?, value - lo);
    Ok(MacroSlope { value, step, up, down, vertices: mid.len() })
}

pub struct Settled {
    pub positions: Vec<[f64; 3]>,
    pub triangle_uvs: Option<Vec<[gf_geom::Uv; 3]>>,
    pub report: serde_json::Map<String, Value>,
    pub measurements: serde_json::Map<String, Value>,
    pub revision: String,
    pub placement: String,
    pub cached: bool,
    pub approximate: bool,
    pub prepared_cached: bool,
    pub seconds: SettleSeconds,
    pub started: Instant,
}

pub struct SettleSeconds {
    pub load: f64,
    pub placer: f64,
    pub evaluated: f64,
    pub prepared: f64,
    pub placed: f64,
}

pub fn settle_mesh(input: &Value) -> Result<Settled, ApiError> {
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
    let triangle_uvs = if input.get("uvs").and_then(Value::as_bool).unwrap_or(false) {
        Some(placed.triangle_uvs.unwrap_or_else(|| loaded.game.triangle_uvs.clone()))
    } else {
        None
    };
    Ok(Settled {
        positions: placed.game,
        triangle_uvs,
        report: placed.report,
        measurements,
        revision: spec.revision(),
        placement: placement.version().to_string(),
        cached,
        approximate: draft,
        prepared_cached: ready,
        seconds: SettleSeconds { load: t_load, placer: t0, evaluated: t1, prepared: t_prepared, placed: t2 },
        started,
    })
}

pub fn settle(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let s = settle_mesh(input)?;
    let draft = s.approximate;
    let mut out = json!({"vertices": s.positions.len(), "revision": s.revision, "placement": s.placement,
                         "positions_f32_b64": f32_base64(s.positions.iter().flatten().copied()), "cached": s.cached, "approximate": draft,
                         "draft": if draft { json!({"rounds": DRAFT_ROUNDS, "full_rounds": f3_defaults().rounds, "note": "designer-only draft; save and export use the full placement"}) } else { Value::Null }});
    if let Some(uvs) = &s.triangle_uvs {
        out["triangle_uvs_f32_b64"] = json!(f32_base64(uvs.iter().flatten().flatten().map(|&x| x as f64)));
    }
    let t3 = s.started.elapsed().as_secs_f64();
    let SettleSeconds { load: t_load, placer: t0, evaluated: t1, prepared: t_prepared, placed: t2 } = s.seconds;
    out["timing_ms"] = json!({"load": t_load * 1000.0, "evaluate": (t1 - t0) * 1000.0, "prepare": (t_prepared - t1) * 1000.0, "place": (t2 - t_prepared) * 1000.0,
                             "encode": (t3 - t2) * 1000.0, "total": t3 * 1000.0, "prepared_cached": s.prepared_cached});
    out["measurements"] = Value::Object(s.measurements);
    out["report"] = Value::Object(s.report);
    Ok(out)
}

pub fn macro_delta(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let started = Instant::now();
    if input.get("sampled").and_then(Value::as_bool).unwrap_or(false) {
        let s = macro_samples(input)?;
        let layers: Vec<String> = s.layers.iter().map(|l| f32_base64(l.iter().map(|&x| x as f64))).collect();
        let what = if s.placed { "the settled game positions P(x) in metres (designer.settle of the character with the slider at x)" } else { "the macro layer P(x) in metres at each game vertex" };
        return Ok(json!({"slider": text(input, "slider"), "value": s.value, "sampled": true, "sample_form": if s.placed { "placed" } else { "layer" }, "sample_spacing": s.spacing,
                         "values": s.values, "layers_f32_b64": layers, "vertices": s.vertices,
                         "definition": format!("{what} for every sample value x (0, 1, every part boundary of the slider, a grid of sample_spacing); a preview interpolates P piecewise-linearly and moves the anchor by P(v) - P(v0)"),
                         "seconds": started.elapsed().as_secs_f64()}));
    }
    let s = macro_slope(input)?;
    let encode = |v: &[f32]| f32_base64(v.iter().map(|&x| x as f64));
    Ok(json!({"slider": text(input, "slider"), "value": s.value, "step": s.step, "up_f32_b64": encode(&s.up), "down_f32_b64": encode(&s.down), "vertices": s.vertices,
              "definition": "metres per unit slider at each game vertex: (L(v + step) - L(v)) / step and (L(v) - L(v - step)) / step, clamped to [0, 1]",
              "seconds": started.elapsed().as_secs_f64()}))
}

pub fn spec_from_input(input: &Value) -> Result<Spec, ApiError> {
    let (loaded, _) = load(input)?;
    let source = macro_source(input)?;
    let spec = spec_of(input, &loaded, source.as_ref())?;
    let (problems, _) = ch::problems(&loaded.store, &spec, source.as_ref());
    if !problems.is_empty() {
        return Err(ApiError::invalid(format!("the spec does not validate: {}", problems.join("; "))).details(json!({"problems": problems})));
    }
    Ok(spec)
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

type BaseRigSlot = Mutex<Option<(String, Arc<gf_core::face_rig::BaseRig>)>>;
static RIG_CACHE: OnceLock<BaseRigSlot> = OnceLock::new();

pub fn rig_face_input() -> Value {
    obj(
        with(with(base_props(), character_props()), json!({
            "morphs": {"type": "array", "items": {"type": "string", "minLength": 1}, "minItems": 1, "description": "the expression target ids, in slider order"},
            "morph_dir": {"type": "string", "minLength": 1, "description": "directory of <id>.npz sparse targets on the dense topology; an id not found there comes from the catalog"},
            "jaw": {"type": "string", "minLength": 1, "description": "the jaw-open target id the hinge is fitted to (default: the morph whose id ends in jawOpen)"},
            "eyes": {"type": "string", "minLength": 1, "description": "optional eyes file: eye joint positions and the placement's eyelid clearance; without it the eye joints sit between the lid joints"},
            "rounds": {"type": "integer", "minimum": 0, "maximum": 6, "description": "weight refinement rounds on the base (default 2)"},
            "payload": {"type": "boolean", "description": "return the per-vertex arrays (default true); false returns the joints, poses and report only"}})),
        &["store", "pack", "catalog", "game", "morphs"],
    )
}

pub fn rig_face_output() -> Value {
    shape(json!({"names": {"type": "array"}, "joints": {"type": "array"}, "poses": {"type": "array"}, "fit": {"type": "array"}, "weights": {"type": "object"},
                 "rest_f32_b64": {"type": "string"}, "influence_joints_u8_b64": {"type": "string"}, "influence_weights_f32_b64": {"type": "string"},
                 "blend": {"type": "object"}, "timing_ms": {"type": "object"}}))
}

pub fn rig_face(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    use gf_core::face_rig as fr;
    let started = Instant::now();
    let (loaded, cached) = load(input)?;
    let ids: Vec<String> = input.get("morphs").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    if ids.is_empty() {
        return Err(ApiError::invalid("morphs must name at least one target"));
    }
    let dir = input.get("morph_dir").and_then(Value::as_str);
    let fetch = |id: &str| -> Result<SparseTarget, ApiError> {
        if let Some(d) = dir {
            let path = Path::new(d).join(format!("{id}.npz"));
            if path.is_file() {
                let bytes = std::fs::read(&path).map_err(|e| ApiError::internal(format!("{}: {e}", path.display())))?;
                let t = gf_io::store::sparse_from_npz(&bytes).map_err(|e| ApiError::invalid(format!("{id}: {e}")))?;
                t.check(loaded.dense.positions.len()).map_err(|e| ApiError::invalid(format!("{id}: {e}")))?;
                return Ok(t);
            }
        }
        loaded.target(id).map(|t| (*t).clone())
    };
    let targets: Vec<SparseTarget> = ids.iter().map(|id| fetch(id)).collect::<Result<_, _>>()?;
    let jaw_id = match input.get("jaw").and_then(Value::as_str) {
        Some(j) => j.to_string(),
        None => ids.iter().find(|i| i.ends_with("jawOpen")).cloned().ok_or_else(|| ApiError::invalid("no morph id ends in jawOpen; name the jaw target with jaw"))?,
    };
    let rounds = input.get("rounds").and_then(Value::as_u64).unwrap_or(2).min(6) as usize;
    let key = format!("{}|{}|{}|{}|{}", loaded.key, dir.unwrap_or(""), ids.join(","), jaw_id, rounds);
    let slot = RIG_CACHE.get_or_init(|| Mutex::new(None));
    let (base_rig, rig_cached) = {
        let mut held = slot.lock().map_err(|_| ApiError::internal("the rig cache lock is poisoned"))?;
        match held.as_ref().filter(|(k, _)| *k == key) {
            Some((_, r)) => (r.clone(), true),
            None => {
                let jaw = fetch(&jaw_id)?;
                let r = Arc::new(fr::base_rig(&loaded.dense.positions, &loaded.game.kept, &loaded.game.triangles, &jaw, &targets, rounds).map_err(ApiError::invalid)?);
                *held = Some((key, r.clone()));
                (r, false)
            }
        }
    };
    let t_rig = started.elapsed().as_secs_f64();
    let source = macro_source(input)?;
    let spec = spec_of(input, &loaded, source.as_ref())?;
    let eyes = match input.get("eyes").and_then(Value::as_str) {
        Some(p) => gf_io::load_eyes(Path::new(p)).map_err(ApiError::invalid)?,
        None => Vec::new(),
    };
    let eye_spheres: Vec<([f64; 3], f64)> = eyes.iter().map(|e| (e.centre, e.radius)).collect();
    let mut eye_centres: Vec<[f64; 3]> = eyes.iter().map(|e| e.centre).collect();
    eye_centres.sort_by(|a, b| b[0].total_cmp(&a[0]));
    let version = PlacementVersion::parse(&spec.base.placement).map_err(ApiError::invalid)?;
    let placement = placer(version, eyes, false)?;
    let dense = loaded.evaluate(&spec, source.as_ref())?;
    let placed = placement.place_prepared(&dense, &DenseMesh { triangles: &loaded.dense.triangles, triangle_uvs: &loaded.dense.triangle_uvs }, &loaded.game, loaded.prepared());
    let t_place = started.elapsed().as_secs_f64();
    let rest = placed.game;
    let seam = fr::neck_seam(&loaded.game.triangles, &rest).map_err(ApiError::invalid)?;
    let neck = fr::mean(seam.iter().map(|&g| rest[g as usize]));
    let hinge = gf_core::rig::character_hinge(&loaded.dense.positions, &dense, &base_rig.hinge);
    let (eye_left, eye_right, eye_source) = if eye_centres.len() == 2 {
        (eye_centres[0], eye_centres[1], "eyes file")
    } else {
        let (l, r) = fr::eye_estimate(&dense);
        (l, r, "estimated from the eye corners (no eyes file)")
    };
    let pivots = fr::character_pivots([neck, hinge, eye_left, eye_right], &base_rig.regions, &dense);
    let deltas = fr::game_deltas(&targets, &loaded.game.kept, dense.len());
    let emphasis = fr::lid_emphasis(&rest, &eye_spheres);
    let problem = fr::Problem { rest: &rest, influences: &base_rig.influences, pivots: &pivots, emphasis: Some(&emphasis) };
    let fits = fr::fit_shapes(&problem, &deltas);
    let t_fit = started.elapsed().as_secs_f64();
    let names: Vec<String> = ids.iter().map(|id| id.rsplit('/').next().unwrap_or(id).to_string()).collect();
    let all = fr::names();
    let parents = fr::parents();
    let joints: Vec<Value> = (0..fr::count()).map(|j| json!({"name": all[j], "parent": parents[j], "position": pivots[j], "posed": fr::is_free(j)})).collect();
    let poses: Vec<Value> = fits.iter().map(|f| json!(f.0.iter().map(|p| [p.rotation[0], p.rotation[1], p.rotation[2], p.translation[0], p.translation[1], p.translation[2]]).collect::<Vec<_>>())).collect();
    let fit: Vec<Value> = names.iter().zip(&fits).map(|(n, f)| {
        let mut row = serde_json::to_value(&f.1).unwrap_or(Value::Null);
        row["name"] = json!(n);
        row["poorly_expressed"] = json!(fr::poorly_expressed(&f.1));
        row
    }).collect();
    let mut out = json!({"names": names, "targets": ids, "jaw": jaw_id, "joints": joints, "eye_joints": eye_source, "poses": poses, "fit": fit,
                         "weights": base_rig.report, "vertices": rest.len(), "revision": spec.revision(), "placement": placement.version().to_string(),
                         "cached": cached, "rig_cached": rig_cached,
                         "definitions": {"poses": "per shape and joint [rx, ry, rz, tx, ty, tz] in the dense frame (metres, z up): x' = R(r) (x - p) + p + t with p the joint position and R(r) the rotation by the rotation vector r",
                                         "skinning": "x' = sum over a vertex's influences of weight * (joint transform applied to x)",
                                         "blend": "delta[s] = the target's dense delta at each game vertex's dense index (as designer.pack), f32 metres; the export's morphs are F(c + e) - F(c) through the placement instead",
                                         "fit": "the poses are fitted to these blend deltas on the settled game mesh; errors in mm over the vertices the shape or the joints move more than 0.1 mm"}});
    if input.get("payload").and_then(Value::as_bool).unwrap_or(true) {
        let (mut jb, mut wb) = (Vec::with_capacity(rest.len() * 4), Vec::with_capacity(rest.len() * 16));
        for inf in &base_rig.influences {
            for k in 0..4 {
                let (j, w) = inf.get(k).copied().unwrap_or((0, 0.0));
                jb.push(j as u8);
                wb.extend((w as f32).to_le_bytes());
            }
        }
        let (mut offsets, mut index, mut delta) = (vec![0u32], Vec::new(), Vec::new());
        for d in &deltas {
            for (g, v) in d.iter().enumerate() {
                if v.iter().any(|x| *x != 0.0) {
                    index.extend((g as u32).to_le_bytes());
                    delta.extend(v.iter().flat_map(|x| (*x as f32).to_le_bytes()));
                }
            }
            offsets.push((index.len() / 4) as u32);
        }
        out["rest_f32_b64"] = json!(f32_base64(rest.iter().flatten().copied()));
        out["influence_joints_u8_b64"] = json!(base64(&jb));
        out["influence_weights_f32_b64"] = json!(base64(&wb));
        out["blend"] = json!({"offsets": offsets, "index_u32_b64": base64(&index), "delta_f32_b64": base64(&delta)});
    }
    let t_end = started.elapsed().as_secs_f64();
    out["timing_ms"] = json!({"base_rig": t_rig * 1000.0, "place": (t_place - t_rig) * 1000.0, "fit": (t_fit - t_place) * 1000.0, "encode": (t_end - t_fit) * 1000.0, "total": t_end * 1000.0});
    Ok(out)
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
