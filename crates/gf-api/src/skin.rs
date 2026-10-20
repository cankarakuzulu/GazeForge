use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use gf_core::skin_texture::{self as st, SkinLook};
use gf_core::{game_mesh, skin_map, skin_regions};
use gf_io::store::Store;
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{obj, shape, text};
use crate::error::ApiError;

pub const DEFAULT_SIZE: usize = 2048;
const PAD: usize = 8;

pub fn models_input() -> Value {
    obj(json!({}), &[])
}

pub fn models_output() -> Value {
    shape(json!({"default": {"type": "string"}, "looks": {"type": "array"}}))
}

pub fn models(_: &Ctx, _: &Value) -> Result<Value, ApiError> {
    Ok(json!({"default": st::DEFAULT_LOOK, "looks": st::looks()}))
}

pub fn generate_input() -> Value {
    obj(
        json!({"store": {"type": "string", "minLength": 1},
               "game": {"type": "string", "minLength": 1, "description": "the game base whose UVs the textures follow"},
               "look": {"type": "string", "minLength": 1, "description": "a preset of skin.models; default medium-warm"},
               "skin": {"type": "object", "description": "fields that override the preset's (melanin, haemoglobin, undertone, redness, lip_tint, under_eye, freckles, freckle_size_mm, moles, pores, age, stubble, hair, oil, seed)"},
               "landmarks": {"type": "string", "minLength": 1, "description": "the base's landmark table (base_landmarks.json: MediaPipe landmark -> dense vertex)"},
               "size": {"type": "integer", "minimum": 256, "maximum": 8192, "description": "texture edge in pixels, default 2048"},
               "regions": {"type": "boolean", "description": "also write <id>.regions.png: each texel coloured by its skin regions, for inspection"},
               "out": {"type": "string", "minLength": 1, "description": "folder; writes <id>.albedo.png, <id>.normal.png and <id>.orm.png"}}),
        &["store", "game", "landmarks", "out"],
    )
}

pub fn generate_output() -> Value {
    shape(json!({"look": {"type": "object"}, "albedo": {"type": "string"}, "normal": {"type": "string"}, "orm": {"type": "string"},
                 "size": {"type": "integer"}, "covered_fraction": {"type": "number"}, "seconds": {"type": "number"}}))
}

pub fn resolve_look(input: &Value) -> Result<SkinLook, ApiError> {
    let id = input.get("look").and_then(Value::as_str).unwrap_or(st::DEFAULT_LOOK);
    let base = st::look(id).ok_or_else(|| ApiError::invalid(format!("skin look {id:?} is not a preset of skin.models")))?;
    let Some(over) = input.get("skin") else { return Ok(base) };
    let mut merged = serde_json::to_value(&base).map_err(|e| ApiError::internal(e.to_string()))?;
    let over = over.as_object().ok_or_else(|| ApiError::invalid("skin must be an object"))?;
    for (k, v) in over {
        if k == "id" || merged.get(k).is_none() {
            return Err(ApiError::invalid(format!("skin has no field {k:?}")));
        }
        merged[k] = v.clone();
    }
    merged["id"] = json!(format!("{id}+custom"));
    serde_json::from_value(merged).map_err(|e| ApiError::invalid(format!("skin: {e}")))
}

pub fn read_landmarks(path: &Path, dense: &[[f64; 3]]) -> Result<BTreeMap<usize, [f64; 3]>, ApiError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
    let v: Value = serde_json::from_str(&raw).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
    let table = v.get("vertices").and_then(Value::as_object).ok_or_else(|| ApiError::invalid(format!("{}: no vertices table", path.display())))?;
    let mut out = BTreeMap::new();
    for (k, idx) in table {
        let lm: usize = k.parse().map_err(|_| ApiError::invalid(format!("{}: landmark key {k:?} is not a number", path.display())))?;
        let i = idx.as_u64().ok_or_else(|| ApiError::invalid(format!("{}: landmark {k} has no vertex index", path.display())))? as usize;
        let p = dense.get(i).ok_or_else(|| ApiError::invalid(format!("{}: landmark {k} names vertex {i}, beyond the dense base", path.display())))?;
        out.insert(lm, *p);
    }
    Ok(out)
}

pub fn generate(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let started = Instant::now();
    let look = resolve_look(input)?;
    let size = input.get("size").and_then(Value::as_u64).map(|s| s as usize).unwrap_or(DEFAULT_SIZE);
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::not_found)?;
    let (manifest, mesh) = store.load_game(text(input, "game")).map_err(ApiError::not_found)?;
    let (_, dense) = store.load_pack(&manifest.pack).map_err(ApiError::not_found)?;
    let landmarks = read_landmarks(Path::new(text(input, "landmarks")), &dense.positions)?;
    let positions: Vec<[f64; 3]> = mesh.kept.iter().map(|&k| dense.positions[k as usize]).collect();
    let normals = game_mesh::game_normals(&positions, &mesh.triangles);
    let regions = skin_regions::build(&positions, &normals, &mesh.triangles, &landmarks).map_err(ApiError::invalid)?;
    let map = skin_map::build(&mesh.triangle_uvs, size, PAD);
    let tex = st::generate(&map, &positions, &normals, &mesh.triangles, &mesh.triangle_uvs, &regions, &look);
    let out = Path::new(text(input, "out"));
    std::fs::create_dir_all(out).map_err(|e| ApiError::invalid(format!("{}: {e}", out.display())))?;
    let rgba = |rgb: &[u8]| -> Vec<u8> { rgb.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect() };
    let mut written = BTreeMap::new();
    for (kind, pixels) in [("albedo", tex.albedo.clone()), ("normal", rgba(&tex.normal)), ("orm", rgba(&tex.orm))] {
        let path = out.join(format!("{}.{kind}.png", look.id));
        let png = gf_io::png::rgba(size, size, &pixels).map_err(ApiError::internal)?;
        std::fs::write(&path, png).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
        written.insert(kind, path.display().to_string());
    }
    if input.get("regions").and_then(Value::as_bool) == Some(true) {
        let path = out.join(format!("{}.regions.png", look.id));
        let png = gf_io::png::rgba(size, size, &st::region_map(&map, &mesh.triangles, &regions)).map_err(ApiError::internal)?;
        std::fs::write(&path, png).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
        written.insert("regions", path.display().to_string());
    }
    Ok(json!({"look": look, "albedo": written["albedo"], "normal": written["normal"], "orm": written["orm"], "regions": written.get("regions"), "size": size,
              "covered_fraction": map.covered() as f64 / (size * size) as f64, "seconds": started.elapsed().as_secs_f64()}))
}
