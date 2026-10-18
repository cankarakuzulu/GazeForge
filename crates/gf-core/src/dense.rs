use std::path::{Path, PathBuf};

use ndarray::{Array1, Array2};
use ndarray_npy::read_npy;
use serde::Deserialize;

pub type Uv = [f32; 2];

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
    Ok(array.rows().into_iter().map(|r| [r[0] as f64, r[1] as f64, r[2] as f64]).collect())
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
}
