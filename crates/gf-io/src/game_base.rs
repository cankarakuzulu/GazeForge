use std::path::Path;

use ndarray::{Array1, Array2};
use ndarray_npy::{read_npy, write_npy};

use gf_geom::decimate::{Decimated, Stats};

pub fn save(dir: &Path, mesh: &Decimated) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let kept = Array1::from(mesh.kept.clone());
    let tris = Array2::from_shape_vec((mesh.triangles.len(), 3), mesh.triangles.iter().flatten().copied().collect())
        .map_err(|e| e.to_string())?;
    let uvs = Array2::from_shape_vec((mesh.triangle_uvs.len(), 6), mesh.triangle_uvs.iter().flatten().flatten().copied().collect())
        .map_err(|e| e.to_string())?;
    write_npy(dir.join("kept.npy"), &kept).map_err(|e| e.to_string())?;
    write_npy(dir.join("owner.npy"), &Array1::from(mesh.owner.clone())).map_err(|e| e.to_string())?;
    write_npy(dir.join("triangles.npy"), &tris).map_err(|e| e.to_string())?;
    write_npy(dir.join("triangle_uvs.npy"), &uvs).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("report.json"), serde_json::to_string_pretty(&mesh.stats).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn load(dir: &Path) -> Result<Decimated, String> {
    let kept: Array1<u32> = read_npy(dir.join("kept.npy")).map_err(|e| format!("kept.npy: {e}"))?;
    let owner: Array1<u32> = read_npy(dir.join("owner.npy")).map_err(|e| format!("owner.npy: {e}"))?;
    let tris: Array2<u32> = read_npy(dir.join("triangles.npy")).map_err(|e| format!("triangles.npy: {e}"))?;
    let uvs: Array2<f32> = read_npy(dir.join("triangle_uvs.npy")).map_err(|e| format!("triangle_uvs.npy: {e}"))?;
    Ok(Decimated {
        kept: kept.to_vec(),
        owner: owner.to_vec(),
        triangles: tris.rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect(),
        triangle_uvs: uvs.rows().into_iter().map(|r| [[r[0], r[1]], [r[2], r[3]], [r[4], r[5]]]).collect(),
        stats: Stats::default(),
    })
}
