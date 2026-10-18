use std::path::{Path, PathBuf};

use ndarray::{Array1, Array2};
use ndarray_npy::read_npy;
use serde::Deserialize;

pub type Uv = [f32; 2];

pub const NOSE_TIP_VERTEX: usize = 185_894;

#[derive(Deserialize)]
pub struct ShapeEntry {
    pub name: String,
}

#[derive(Deserialize)]
pub struct Manifest {
    pub vertices: usize,
    pub faces: usize,
    pub shapes: Vec<ShapeEntry>,
}

pub struct Dense {
    pub dir: PathBuf,
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    pub triangle_uvs: Vec<[Uv; 3]>,
    pub manifest: Manifest,
}

fn read_points(path: &Path) -> Result<Vec<[f64; 3]>, String> {
    let array: Array2<f32> = read_npy(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if array.ncols() != 3 {
        return Err(format!("{}: expected N x 3, got {:?}", path.display(), array.shape()));
    }
    let points: Vec<[f64; 3]> = array.rows().into_iter().map(|r| [r[0] as f64, r[1] as f64, r[2] as f64]).collect();
    check_finite(&points).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(points)
}

pub fn write_points(path: &Path, points: &[[f64; 3]]) -> Result<(), String> {
    let flat: Vec<f32> = points.iter().flat_map(|p| p.map(|x| x as f32)).collect();
    let array = Array2::from_shape_vec((points.len(), 3), flat).map_err(|e| e.to_string())?;
    ndarray_npy::write_npy(path, &array).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn check_finite(points: &[[f64; 3]]) -> Result<(), String> {
    let bad: Vec<usize> = points.iter().enumerate().filter(|(_, p)| p.iter().any(|x| !x.is_finite())).map(|(i, _)| i).collect();
    match bad.first() {
        None => Ok(()),
        Some(first) => Err(format!("{} vertices have a NaN or infinite coordinate (first: vertex {first})", bad.len())),
    }
}

impl Dense {
    pub fn load(dir: &Path) -> Result<Dense, String> {
        let manifest: Manifest = serde_json::from_str(
            &std::fs::read_to_string(dir.join("manifest.json")).map_err(|e| format!("manifest.json: {e}"))?,
        )
        .map_err(|e| format!("manifest.json: {e}"))?;
        let positions = read_points(&dir.join("positions.npy"))?;
        let sizes: Array1<u32> = read_npy(dir.join("face_sizes.npy")).map_err(|e| format!("face_sizes.npy: {e}"))?;
        let corners: Array1<u32> = read_npy(dir.join("corner_vertices.npy")).map_err(|e| format!("corner_vertices.npy: {e}"))?;
        let uvs: Array2<f32> = read_npy(dir.join("corner_uvs.npy")).map_err(|e| format!("corner_uvs.npy: {e}"))?;
        if positions.len() != manifest.vertices || sizes.len() != manifest.faces {
            return Err("the arrays disagree with manifest.json".into());
        }
        let mut triangles = Vec::with_capacity(sizes.len() * 2);
        let mut triangle_uvs = Vec::with_capacity(sizes.len() * 2);
        let mut start = 0usize;
        for &size in sizes.iter() {
            let size = size as usize;
            for k in 1..size - 1 {
                let loops = [start, start + k, start + k + 1];
                triangles.push(loops.map(|l| corners[l]));
                triangle_uvs.push(loops.map(|l| [uvs[[l, 0]], uvs[[l, 1]]]));
            }
            start += size;
        }
        if start != corners.len() {
            return Err(format!("face sizes cover {start} corners, the file has {}", corners.len()));
        }
        Ok(Dense { dir: dir.to_path_buf(), positions, triangles, triangle_uvs, manifest })
    }

    pub fn shape(&self, name: &str) -> Result<Vec<[f64; 3]>, String> {
        let delta = read_points(&self.dir.join(format!("shape_{name}.npy")))?;
        Ok(self.positions.iter().zip(delta).map(|(p, d)| [p[0] + d[0], p[1] + d[1], p[2] + d[2]]).collect())
    }

    pub fn positions_from(&self, spec: &str) -> Result<Vec<[f64; 3]>, String> {
        match spec {
            "base" => Ok(self.positions.clone()),
            _ if spec.starts_with("shape:") => self.shape(&spec[6..]),
            _ => {
                let points = read_points(Path::new(spec))?;
                if points.len() != self.positions.len() {
                    return Err(format!("{spec} has {} vertices, the dense base {}", points.len(), self.positions.len()));
                }
                Ok(points)
            }
        }
    }

    pub fn vertex_weights(&self, path: &str) -> Result<Vec<f64>, String> {
        let array: Array1<f32> = read_npy(path).map_err(|e| format!("{path}: {e}"))?;
        if array.len() != self.positions.len() {
            return Err(format!("{path} has {} values, the dense base {} vertices", array.len(), self.positions.len()));
        }
        let bad = array.iter().filter(|w| !w.is_finite() || **w <= 0.0).count();
        if bad > 0 {
            return Err(format!("{path}: {bad} weights are not finite and positive"));
        }
        Ok(array.iter().map(|&w| w as f64).collect())
    }

    pub fn nose_tip(&self, positions: &[[f64; 3]], vertex: usize) -> Result<[f64; 3], String> {
        positions.get(vertex).copied().ok_or_else(|| {
            format!("nose tip vertex {vertex} is outside the dense base ({} vertices)", self.positions.len())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_points_pass() {
        assert!(check_finite(&[[0.0, 1.0, -2.0], [1e-9, 3.0, 4.0]]).is_ok());
    }

    #[test]
    fn nan_and_infinite_points_are_refused() {
        let error = check_finite(&[[0.0; 3], [0.0, f64::NAN, 0.0], [f64::INFINITY, 0.0, 0.0]]).unwrap_err();
        assert!(error.contains("2 vertices") && error.contains("vertex 1"), "{error}");
        assert!(check_finite(&[[f64::NEG_INFINITY, 0.0, 0.0]]).is_err());
    }
}
