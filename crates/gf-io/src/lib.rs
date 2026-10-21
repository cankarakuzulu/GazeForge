pub mod body;
pub mod body_store;
pub mod character;
pub mod dense;
pub mod game_base;
pub mod gltf;
pub mod obj;
pub mod png;
pub mod store;
pub mod wearable_store;

use std::collections::BTreeSet;
use std::path::Path;

use gf_geom::lids::{self, Eye};
use ndarray::{Array1, Array2};

pub fn load_eyes(path: &Path) -> Result<Vec<Eye>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    lids::eyes_from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn macro_target_files(root: &Path) -> Result<BTreeSet<String>, String> {
    let mut found = BTreeSet::new();
    for top in ["macrodetails", "breast"] {
        let mut stack = vec![root.join(top)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                for ext in [".target.gz", ".target", ".npz"] {
                    if let Some(stem) = name.strip_suffix(ext) {
                        let rel = path.parent().unwrap().strip_prefix(root).map_err(|e| e.to_string())?;
                        let mut parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
                        parts.push(stem.to_string());
                        found.insert(parts.join("/"));
                        break;
                    }
                }
            }
        }
    }
    Ok(found)
}

pub fn read_values(path: &Path) -> Result<Vec<f64>, String> {
    let array: Array1<f64> = ndarray_npy::read_npy(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(array.to_vec())
}

pub fn write_table(path: &Path, rows: usize, cols: usize, values: Vec<f64>) -> Result<(), String> {
    let array = Array2::from_shape_vec((rows, cols), values).map_err(|e| e.to_string())?;
    ndarray_npy::write_npy(path, &array).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write_table_f32(path: &Path, rows: usize, cols: usize, values: Vec<f32>) -> Result<(), String> {
    let array = Array2::from_shape_vec((rows, cols), values).map_err(|e| e.to_string())?;
    ndarray_npy::write_npy(path, &array).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn write_table_i32(path: &Path, rows: usize, cols: usize, values: Vec<i32>) -> Result<(), String> {
    let array = Array2::from_shape_vec((rows, cols), values).map_err(|e| e.to_string())?;
    ndarray_npy::write_npy(path, &array).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn read_table_f32(path: &Path) -> Result<(usize, usize, Vec<f32>), String> {
    let array: Array2<f32> = ndarray_npy::read_npy(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (r, c) = array.dim();
    Ok((r, c, array.iter().copied().collect()))
}
