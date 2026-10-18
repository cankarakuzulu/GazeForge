use std::collections::HashMap;

use gf_geom::Uv;
use gf_geom::decimate::Decimated;
use gf_geom::placement::angle_weighted_normals;

pub type Point = [f64; 3];

pub fn to_gltf(p: Point) -> [f32; 3] {
    [p[0] as f32, p[2] as f32, (-p[1]) as f32]
}

pub struct Surface {
    pub source: Vec<u32>,
    pub uvs: Vec<Uv>,
    pub indices: Vec<u32>,
}

pub fn split_seams(triangles: &[[u32; 3]], triangle_uvs: &[[Uv; 3]]) -> Surface {
    let mut seen: HashMap<(u32, u32, u32), u32> = HashMap::new();
    let mut s = Surface { source: Vec::new(), uvs: Vec::new(), indices: Vec::with_capacity(triangles.len() * 3) };
    for (tri, uvs) in triangles.iter().zip(triangle_uvs) {
        for c in 0..3 {
            let key = (tri[c], uvs[c][0].to_bits(), uvs[c][1].to_bits());
            let next = s.source.len() as u32;
            let index = *seen.entry(key).or_insert(next);
            if index == next {
                s.source.push(tri[c]);
                s.uvs.push(uvs[c]);
            }
            s.indices.push(index);
        }
    }
    s
}

pub fn lod_map(lod0: &Decimated, lod: &Decimated) -> Result<Vec<u32>, String> {
    let index: HashMap<u32, u32> = lod0.kept.iter().enumerate().map(|(i, &d)| (d, i as u32)).collect();
    let mut out = Vec::with_capacity(lod.kept.len());
    let mut missing = 0usize;
    for d in &lod.kept {
        match index.get(d) {
            Some(&i) => out.push(i),
            None => missing += 1,
        }
    }
    if missing > 0 {
        return Err(format!("the LOD is not nested: {missing} of its {} vertices are not vertices of the finest level", lod.kept.len()));
    }
    Ok(out)
}

pub fn lod_uvs(lod0: &Decimated, lod0_uvs: &[[Uv; 3]], lod: &Decimated, map: &[u32]) -> (Vec<[Uv; 3]>, usize, f64) {
    let mut keys: HashMap<u32, Vec<(Uv, Uv)>> = HashMap::new();
    for ((tri, base), placed) in lod0.triangles.iter().zip(&lod0.triangle_uvs).zip(lod0_uvs) {
        for c in 0..3 {
            keys.entry(tri[c]).or_default().push((base[c], placed[c]));
        }
    }
    let mut unmatched = 0usize;
    let mut worst = 0.0f64;
    let out = lod
        .triangles
        .iter()
        .zip(&lod.triangle_uvs)
        .map(|(tri, base)| {
            [0, 1, 2].map(|c| {
                let list = &keys[&map[tri[c] as usize]];
                let distance = |k: &(Uv, Uv)| ((k.0[0] - base[c][0]).abs().max((k.0[1] - base[c][1]).abs())) as f64;
                let best = list.iter().min_by(|a, b| distance(a).total_cmp(&distance(b))).unwrap();
                let d = distance(best);
                worst = worst.max(d);
                if d > 1e-4 {
                    unmatched += 1;
                }
                best.1
            })
        })
        .collect();
    (out, unmatched, worst)
}

pub fn game_normals(game: &[Point], triangles: &[[u32; 3]]) -> Vec<Point> {
    angle_weighted_normals(game, triangles)
}

pub struct Displacement {
    pub entries: usize,
    pub max_mm: f64,
    pub over_0p01_mm: usize,
    pub over_0p1_mm: usize,
}

pub fn displacement(base: &[Point], moved: &[Point]) -> Displacement {
    let mut d = Displacement { entries: 0, max_mm: 0.0, over_0p01_mm: 0, over_0p1_mm: 0 };
    for (a, b) in base.iter().zip(moved) {
        let delta = to_gltf([b[0] - a[0], b[1] - a[1], b[2] - a[2]]);
        if delta.iter().any(|x| *x != 0.0) {
            d.entries += 1;
        }
        let mm = (0..3).map(|k| (b[k] - a[k]).powi(2)).sum::<f64>().sqrt() * 1000.0;
        d.max_mm = d.max_mm.max(mm);
        d.over_0p01_mm += (mm > 0.01) as usize;
        d.over_0p1_mm += (mm > 0.1) as usize;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use gf_geom::decimate::Stats;

    fn mesh(kept: Vec<u32>, triangles: Vec<[u32; 3]>, uvs: Vec<[Uv; 3]>) -> Decimated {
        Decimated { kept, owner: Vec::new(), triangles, triangle_uvs: uvs, stats: Stats::default() }
    }

    #[test]
    fn seams_split_only_where_uvs_differ() {
        let tris = [[0, 1, 2], [2, 1, 3]];
        let uvs = [[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], [[0.0, 1.0], [1.0, 0.0], [1.0, 1.0]]];
        assert_eq!(split_seams(&tris, &uvs).source.len(), 4);
        let cut = [[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], [[0.5, 1.0], [1.0, 0.0], [1.0, 1.0]]];
        let s = split_seams(&tris, &cut);
        assert_eq!(s.source, vec![0, 1, 2, 2, 3]);
        assert_eq!(s.indices, vec![0, 1, 2, 3, 1, 4]);
    }

    #[test]
    fn a_lod_that_is_not_nested_is_refused() {
        let uv = [[0.0f32, 0.0]; 3];
        let fine = mesh(vec![10, 11, 12, 13], vec![[0, 1, 2]], vec![uv]);
        assert_eq!(lod_map(&fine, &mesh(vec![13, 10, 11], vec![[0, 1, 2]], vec![uv])).unwrap(), vec![3, 0, 1]);
        assert!(lod_map(&fine, &mesh(vec![10, 99, 11], vec![[0, 1, 2]], vec![uv])).is_err());
    }

    #[test]
    fn the_gltf_frame_is_y_up_and_faces_plus_z() {
        assert_eq!(to_gltf([1.0, -2.0, 3.0]), [1.0, 3.0, 2.0]);
    }
}
