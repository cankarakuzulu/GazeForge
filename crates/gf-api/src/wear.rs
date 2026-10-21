use std::path::Path;

use gf_io::store::Store;
use gf_io::wearable_store::WearableManifest;
use serde_json::{Value, json};

use crate::commands::{obj, shape, text};
use crate::error::ApiError;
use crate::Ctx;

pub fn list_input() -> Value {
    obj(json!({"store": {"type": "string", "minLength": 1}}), &["store"])
}

pub fn list_output() -> Value {
    shape(json!({"wearables": {"type": "array", "description": "per wearable: reference, name, kind, vertices, the base vertices it hides, transparent, its material maps, z_depth (the layer it takes unless the spec's wearables[].layer sets one: lower is closer to the skin)"}}))
}

pub fn list(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = Path::new(text(input, "store"));
    let store = Store::open(root).map_err(ApiError::not_found)?;
    let dir = root.join("wearables");
    let mut rows = Vec::new();
    if dir.is_dir() {
        let mut names: Vec<String> = std::fs::read_dir(&dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?.filter_map(Result::ok)
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(str::to_string)).collect();
        names.sort();
        for hex in names {
            let reference = format!("sha256:{hex}");
            let m: WearableManifest = store.get_manifest("wearables", &reference).map_err(ApiError::internal)?;
            let z_depth = store.load_wearable(&reference).map(|w| crate::tasks::wearable::depth_of(&w.binding)).map_err(ApiError::internal)?;
            rows.push(json!({"wearable": reference, "name": m.name, "kind": m.kind, "vertices": m.vertices, "hides_base_vertices": m.hidden,
                             "transparent": m.transparent, "material": m.material.keys().collect::<Vec<_>>(), "z_depth": z_depth}));
        }
        rows.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    }
    Ok(json!({"wearables": rows}))
}

pub fn import_input() -> Value {
    obj(json!({"store": {"type": "string", "minLength": 1},
               "mhclo": {"type": "string", "minLength": 1, "description": "a MakeHuman clothes file (.mhclo) beside its .obj, .mhmat and textures; several separated by commas"}}),
        &["store", "mhclo"])
}

pub fn import_output() -> Value {
    shape(json!({"wearables": {"type": "array", "description": "per file: the wearable reference to put in body.wearables, its name and counts"}}))
}

pub fn import(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::not_found)?;
    let mut rows = Vec::new();
    for file in text(input, "mhclo").split(',') {
        let (reference, m) = store.import_mhclo(Path::new(file.trim())).map_err(ApiError::invalid)?;
        rows.push(json!({"wearable": reference, "name": m.name, "file": file, "vertices": m.vertices, "hides_base_vertices": m.hidden, "transparent": m.transparent}));
    }
    Ok(json!({"wearables": rows}))
}
