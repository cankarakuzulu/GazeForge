use std::path::Path;

use ndarray::{Array1, Array2};
use ndarray_npy::{WriteNpyExt, read_npy, write_npy};

use gf_geom::decimate::{Decimated, Stats};

type Arrays = (Array1<u32>, Array1<u32>, Array2<u32>, Array2<f32>);

fn arrays(mesh: &Decimated) -> Result<Arrays, String> {
    let kept = Array1::from(mesh.kept.clone());
    let owner = Array1::from(mesh.owner.clone());
    let tris = Array2::from_shape_vec((mesh.triangles.len(), 3), mesh.triangles.iter().flatten().copied().collect())
        .map_err(|e| e.to_string())?;
    let uvs = Array2::from_shape_vec((mesh.triangle_uvs.len(), 6), mesh.triangle_uvs.iter().flatten().flatten().copied().collect())
        .map_err(|e| e.to_string())?;
    Ok((kept, owner, tris, uvs))
}

pub fn save(dir: &Path, mesh: &Decimated) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (kept, owner, tris, uvs) = arrays(mesh)?;
    write_npy(dir.join("kept.npy"), &kept).map_err(|e| e.to_string())?;
    write_npy(dir.join("owner.npy"), &owner).map_err(|e| e.to_string())?;
    write_npy(dir.join("triangles.npy"), &tris).map_err(|e| e.to_string())?;
    write_npy(dir.join("triangle_uvs.npy"), &uvs).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("report.json"), serde_json::to_string_pretty(&mesh.stats).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn npy_arrays(mesh: &Decimated) -> Result<[(&'static str, Vec<u8>); 4], String> {
    let (kept, owner, tris, uvs) = arrays(mesh)?;
    let bytes = |write: &dyn Fn(&mut Vec<u8>) -> Result<(), ndarray_npy::WriteNpyError>| {
        let mut out = Vec::new();
        write(&mut out).map_err(|e| e.to_string()).map(|_| out)
    };
    Ok([
        ("kept", bytes(&|w| kept.write_npy(w))?),
        ("owner", bytes(&|w| owner.write_npy(w))?),
        ("triangles", bytes(&|w| tris.write_npy(w))?),
        ("triangle_uvs", bytes(&|w| uvs.write_npy(w))?),
    ])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_arrays_are_the_bytes_save_writes() {
        let mesh = Decimated {
            kept: vec![4, 9, 2],
            owner: vec![0, 1, 2, 2, 0, 1, 1, 2, 0, 1],
            triangles: vec![[0, 1, 2]],
            triangle_uvs: vec![[[0.0, 0.5], [1.0, 0.25], [0.5, 1.0]]],
            stats: Stats::default(),
        };
        let dir = std::env::temp_dir().join(format!("gf-game-base-{}", std::process::id()));
        save(&dir, &mesh).unwrap();
        for (name, bytes) in npy_arrays(&mesh).unwrap() {
            assert_eq!(std::fs::read(dir.join(format!("{name}.npy"))).unwrap(), bytes, "{name}");
        }
        let back = load(&dir).unwrap();
        assert_eq!(back.triangles, mesh.triangles);
        assert_eq!(back.triangle_uvs, mesh.triangle_uvs);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
