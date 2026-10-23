use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use gf_io::store::Store;
use gf_io::wearable_store::{CATEGORIES, WearableManifest};
use serde_json::{Value, json};

use crate::commands::{obj, shape, text};
use crate::error::ApiError;
use crate::Ctx;

pub fn list_input() -> Value {
    obj(json!({"store": {"type": "string", "minLength": 1}}), &["store"])
}

pub fn list_output() -> Value {
    shape(json!({"wearables": {"type": "array", "description": "per wearable: reference, name, kind (the import's source: makehuman, obj or attach), category (what it is: one of categories) with category_source (store: kept in the store at import or by wearable.categorize; inferred: from its name, tags and transparency), vertices, the base vertices it hides, transparent, its material maps, z_depth (the layer it takes unless the spec's wearables[].layer sets one: lower is closer to the skin)"},
                 "categories": {"type": "array", "description": "the categories in display order"}}))
}

fn rows_seen() -> &'static Mutex<HashMap<String, (Value, String)>> {
    static SEEN: OnceLock<Mutex<HashMap<String, (Value, String)>>> = OnceLock::new();
    SEEN.get_or_init(Default::default)
}

pub fn list(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let root = Path::new(text(input, "store"));
    let store = Store::open(root).map_err(ApiError::not_found)?;
    let dir = root.join("wearables");
    let mut rows = Vec::new();
    let known = store.wearable_categories().map_err(ApiError::internal)?;
    if dir.is_dir() {
        let mut names: Vec<String> = std::fs::read_dir(&dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?.filter_map(Result::ok)
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").map(str::to_string)).collect();
        names.sort();
        for hex in names {
            let reference = format!("sha256:{hex}");
            let cached = rows_seen().lock().ok().and_then(|seen| seen.get(&reference).cloned());
            let (mut row, inferred) = match cached {
                Some(row) => row,
                None => {
                    let (m, binding): (WearableManifest, _) = store.load_binding(&reference).map_err(ApiError::internal)?;
                    let z_depth = crate::tasks::wearable::depth_of(&binding);
                    let (inferred, _) = store.category_of("", &m, &Default::default());
                    let row = json!({"wearable": reference, "name": m.name, "kind": m.kind, "vertices": m.vertices, "hides_base_vertices": m.hidden,
                                     "transparent": m.transparent, "material": m.material.keys().collect::<Vec<_>>(), "z_depth": z_depth});
                    if let Ok(mut seen) = rows_seen().lock() {
                        seen.insert(reference.clone(), (row.clone(), inferred.clone()));
                    }
                    (row, inferred)
                }
            };
            let (category, source) = known.get(&reference).filter(|c| CATEGORIES.contains(&c.as_str())).map_or((inferred, "inferred"), |c| (c.clone(), "store"));
            row["category"] = json!(category);
            row["category_source"] = json!(source);
            rows.push(row);
        }
        rows.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    }
    Ok(json!({"wearables": rows, "categories": CATEGORIES}))
}

pub fn categorize_input() -> Value {
    obj(json!({"store": {"type": "string", "minLength": 1},
               "wearable": {"type": "string", "minLength": 1, "description": "the wearable's reference (sha256:...)"},
               "category": {"type": "string", "description": "what it is: one of the categories, or empty to forget the kept one so wearable.list infers it again"}}),
        &["store", "wearable", "category"])
}

pub fn categorize_output() -> Value {
    shape(json!({"wearable": {"type": "string"}, "category": {"type": "string", "description": "its category now"}, "category_source": {"type": "string", "description": "store or inferred"},
                 "before": {"description": "the category kept before, or null when it was inferred"}}))
}

pub fn categorize(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::not_found)?;
    let reference = text(input, "wearable");
    let wanted = Some(text(input, "category").trim()).filter(|c| !c.is_empty());
    let before = store.set_wearable_category(reference, wanted).map_err(ApiError::invalid)?;
    let m: WearableManifest = store.get_manifest("wearables", reference).map_err(ApiError::internal)?;
    let (category, source) = store.category_of(reference, &m, &store.wearable_categories().map_err(ApiError::internal)?);
    Ok(json!({"wearable": reference, "category": category, "category_source": source, "before": before}))
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
