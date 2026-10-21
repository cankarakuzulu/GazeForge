use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use ndarray::{Array1, Array2};
use ndarray_npy::{read_npy, write_npy};
use serde_json::{Value, json};

use gf_core::body::{self as b, BodyLod, BodyPack, Mesh, Point, RingVertex};
use gf_core::hash;
use gf_geom::Uv;
use gf_geom::decimate::{self, Decimated, Settings, Stats};

pub const FORMAT: &str = "gazeforge.body/1";

pub struct Polys {
    pub positions: Vec<Point>,
    pub uvs: Vec<[f64; 2]>,
    pub faces: Vec<Vec<(u32, Option<u32>)>>,
}

pub fn read_obj_groups(path: &Path, prefix: &str) -> Result<BTreeMap<String, Vec<u32>>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out: BTreeMap<String, Vec<u32>> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut count: i64 = 0;
    for (n, line) in text.lines().enumerate() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("v") => count += 1,
            Some("g") | Some("o") => current = parts.next().filter(|g| g.starts_with(prefix)).map(str::to_string),
            Some("f") => {
                let Some(g) = &current else { continue };
                let ids = out.entry(g.clone()).or_default();
                for corner in parts {
                    let v: i64 = corner.split('/').next().unwrap_or("").parse().map_err(|_| format!("{} line {}: bad f", path.display(), n + 1))?;
                    let k = if v < 0 { count + v } else { v - 1 };
                    if k < 0 || k >= count {
                        return Err(format!("{} line {}: face index out of range", path.display(), n + 1));
                    }
                    ids.push(k as u32);
                }
            }
            _ => {}
        }
    }
    for ids in out.values_mut() {
        ids.sort_unstable();
        ids.dedup();
    }
    Ok(out)
}

pub fn helpers_of(groups: &BTreeMap<String, Vec<u32>>, extra: &[u32], positions: &[Point]) -> Result<b::Helpers, String> {
    let mut source: Vec<u32> = groups.values().flatten().chain(extra).copied().collect();
    source.sort_unstable();
    source.dedup();
    let at: HashMap<u32, u32> = source.iter().enumerate().map(|(i, &v)| (v, i as u32)).collect();
    let positions = source.iter().map(|&v| positions.get(v as usize).copied().ok_or_else(|| format!("helper vertex {v} is outside the mesh"))).collect::<Result<Vec<_>, _>>()?;
    let groups = groups.iter().map(|(g, ids)| (g.clone(), ids.iter().map(|v| at[v]).collect())).collect();
    Ok(b::Helpers { source, positions, groups })
}

pub fn read_obj(path: &Path, group: Option<&str>) -> Result<Polys, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out = Polys { positions: Vec::new(), uvs: Vec::new(), faces: Vec::new() };
    let mut current: Option<String> = None;
    let bad = |n: usize, what: &str| format!("{} line {}: {what}", path.display(), n + 1);
    for (n, line) in text.lines().enumerate() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("v") => {
                let v: Vec<f64> = parts.take(3).map(str::parse).collect::<Result<_, _>>().map_err(|_| bad(n, "bad v"))?;
                if v.len() != 3 || v.iter().any(|x| !x.is_finite()) {
                    return Err(bad(n, "v needs 3 finite numbers"));
                }
                out.positions.push([v[0], v[1], v[2]]);
            }
            Some("vt") => {
                let v: Vec<f64> = parts.take(2).map(str::parse).collect::<Result<_, _>>().map_err(|_| bad(n, "bad vt"))?;
                if v.len() != 2 {
                    return Err(bad(n, "vt needs 2 numbers"));
                }
                out.uvs.push([v[0], v[1]]);
            }
            Some("g") | Some("o") => current = parts.next().map(str::to_string),
            Some("f") => {
                if group.is_some_and(|g| current.as_deref() != Some(g)) {
                    continue;
                }
                let mut face = Vec::new();
                for corner in parts {
                    let mut ids = corner.split('/');
                    let v: i64 = ids.next().unwrap_or("").parse().map_err(|_| bad(n, "bad f"))?;
                    let t = ids.next().filter(|s| !s.is_empty()).map(str::parse::<i64>).transpose().map_err(|_| bad(n, "bad f"))?;
                    let fix = |i: i64, len: usize| -> Result<u32, String> {
                        let k = if i < 0 { len as i64 + i } else { i - 1 };
                        if k < 0 || k as usize >= len { Err(bad(n, "face index out of range")) } else { Ok(k as u32) }
                    };
                    face.push((fix(v, out.positions.len())?, t.map(|t| fix(t, out.uvs.len())).transpose()?));
                }
                if face.len() < 3 {
                    return Err(bad(n, "a face needs 3 corners"));
                }
                out.faces.push(face);
            }
            _ => {}
        }
    }
    if out.faces.is_empty() {
        return Err(format!("{}: no faces{}", path.display(), group.map(|g| format!(" in group {g}")).unwrap_or_default()));
    }
    Ok(out)
}

pub fn read_target(path: &Path) -> Result<Vec<(u32, Point)>, String> {
    let raw = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = if raw.starts_with(&[0x1f, 0x8b]) {
        let mut s = String::new();
        flate2::read::GzDecoder::new(&raw[..]).read_to_string(&mut s).map_err(|e| format!("{}: {e}", path.display()))?;
        s
    } else {
        String::from_utf8(raw).map_err(|e| format!("{}: {e}", path.display()))?
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let p: Vec<&str> = line.split_whitespace().collect();
        if p.len() < 4 {
            return Err(format!("{}: bad target line {line}", path.display()));
        }
        let i: u32 = p[0].parse().map_err(|_| format!("{}: bad index", path.display()))?;
        let d: Vec<f64> = p[1..4].iter().map(|x| x.parse()).collect::<Result<_, _>>().map_err(|_| format!("{}: bad delta", path.display()))?;
        out.push((i, [d[0], d[1], d[2]]));
    }
    Ok(out)
}

#[derive(Clone, Copy)]
pub enum Up {
    Y,
    Z,
}

impl Up {
    pub fn parse(text: &str) -> Result<Up, String> {
        match text {
            "y" => Ok(Up::Y),
            "z" => Ok(Up::Z),
            other => Err(format!("up must be y or z, got {other}")),
        }
    }

    pub fn to_frame(self, p: Point, unit: f64) -> Point {
        match self {
            Up::Y => [p[0] * unit, -p[2] * unit, p[1] * unit],
            Up::Z => [p[0] * unit, p[1] * unit, p[2] * unit],
        }
    }
}

fn npy_err(path: &Path) -> impl Fn(ndarray_npy::WriteNpyError) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

fn write_points(path: &Path, points: &[Point]) -> Result<(), String> {
    let a = Array2::from_shape_vec((points.len(), 3), points.iter().flatten().copied().collect()).map_err(|e| e.to_string())?;
    write_npy(path, &a).map_err(npy_err(path))
}

fn read_points(path: &Path) -> Result<Vec<Point>, String> {
    let a: Array2<f64> = read_npy(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(a.rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect())
}

fn read_u32(path: &Path) -> Result<Vec<u32>, String> {
    let a: Array1<u32> = read_npy(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(a.to_vec())
}

pub fn save(dir: &Path, pack: &BodyPack, manifest: &Value) -> Result<Value, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    write_points(&dir.join("positions.npy"), &pack.mesh.positions)?;
    let tris = Array2::from_shape_vec((pack.mesh.triangles.len(), 3), pack.mesh.triangles.iter().flatten().copied().collect()).map_err(|e| e.to_string())?;
    write_npy(dir.join("triangles.npy"), &tris).map_err(|e| e.to_string())?;
    let uvs = Array2::from_shape_vec((pack.mesh.uvs.len(), 6), pack.mesh.uvs.iter().flatten().flatten().copied().collect()).map_err(|e| e.to_string())?;
    write_npy(dir.join("triangle_uvs.npy"), &uvs).map_err(|e| e.to_string())?;
    write_npy(dir.join("ring.npy"), &Array1::from(pack.ring.iter().map(|r| r.vertex).collect::<Vec<_>>())).map_err(|e| e.to_string())?;
    write_npy(dir.join("ring_params.npy"), &Array1::from(pack.ring.iter().map(|r| r.param).collect::<Vec<_>>())).map_err(|e| e.to_string())?;
    write_points(&dir.join("ring_targets.npy"), &pack.ring_targets)?;
    write_npy(dir.join("seam_loop.npy"), &Array1::from(pack.seam_loop.clone())).map_err(|e| e.to_string())?;
    write_points(&dir.join("seam_base.npy"), &pack.seam_base)?;
    if !pack.source.is_empty() {
        write_npy(dir.join("source.npy"), &Array1::from(pack.source.clone())).map_err(|e| e.to_string())?;
    }
    if !pack.cage_source.is_empty() {
        write_npy(dir.join("cage_source.npy"), &Array1::from(pack.cage_source.clone())).map_err(|e| e.to_string())?;
    }
    if let Some(f) = &pack.fine {
        let d = dir.join("fine");
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        let q = Array2::from_shape_vec((f.quads.len(), 4), f.quads.iter().flatten().copied().collect()).map_err(|e| e.to_string())?;
        write_npy(d.join("quads.npy"), &q).map_err(|e| e.to_string())?;
        let u = Array2::from_shape_vec((f.quad_uvs.len(), 8), f.quad_uvs.iter().flatten().flatten().copied().collect()).map_err(|e| e.to_string())?;
        write_npy(d.join("quad_uvs.npy"), &u).map_err(|e| e.to_string())?;
        for (i, lod) in f.lods.iter().enumerate() {
            let dd = Decimated { kept: lod.kept.clone(), owner: Vec::new(), triangles: lod.triangles.clone(), triangle_uvs: lod.uvs.clone(), stats: Stats::default() };
            crate::game_base::save(&d.join(format!("lod{i}")), &dd)?;
        }
    }
    if !pack.helpers.source.is_empty() {
        let h = dir.join("helpers");
        std::fs::create_dir_all(&h).map_err(|e| format!("{}: {e}", h.display()))?;
        write_npy(h.join("source.npy"), &Array1::from(pack.helpers.source.clone())).map_err(|e| e.to_string())?;
        write_points(&h.join("positions.npy"), &pack.helpers.positions)?;
        std::fs::write(h.join("groups.json"), serde_json::to_string(&pack.helpers.groups).map_err(|e| e.to_string())?).map_err(|e| format!("groups.json: {e}"))?;
    }
    for (i, lod) in pack.lods.iter().enumerate() {
        let d = Decimated { kept: lod.kept.clone(), owner: Vec::new(), triangles: lod.triangles.clone(), triangle_uvs: lod.uvs.clone(), stats: Stats::default() };
        crate::game_base::save(&dir.join(format!("lod{}", i + 1)), &d)?;
    }
    let mut m = manifest.clone();
    m["format"] = json!(FORMAT);
    m["mode"] = json!(pack.mode);
    m["per_span"] = json!(pack.per_span);
    if let Some(f) = &pack.fine {
        m["fine"] = json!({"level": f.level, "dir": "fine", "cage_quads": f.quads.len(), "vertices": f.stencil.rows.len(), "triangles": f.triangles.len(),
                           "lods": f.lods.iter().enumerate().map(|(i, l)| json!({"name": format!("Body_LOD{i}"), "dir": format!("fine/lod{i}"), "vertices": l.kept.len(), "triangles": l.triangles.len()})).collect::<Vec<_>>(),
                           "meaning": "the body is shaped on its cage (positions.npy, the targets' vertices) and refined by Catmull-Clark (B-spline boundary) to this level; every game LOD is a decimation of the refined body, the neck band kept fine"});
    }
    if !pack.helpers.source.is_empty() {
        m["helpers"] = json!({"vertices": pack.helpers.source.len(), "groups": pack.helpers.groups.len(), "dir": "helpers",
                              "meaning": "every base.obj vertex the body does not keep (the joint-* helper cubes, the head the dense head replaces, the other helper geometry), in the body's frame at its base shape: a MakeHuman rig's joints are cube means, vertices or vertex means of these, so they are shaped with the body"});
    }
    m["lods"] = json!(std::iter::once(json!({"name": "Body_LOD0", "vertices": pack.mesh.positions.len(), "triangles": pack.mesh.triangles.len()}))
        .chain(pack.lods.iter().enumerate().map(|(i, l)| {
            let kept: HashSet<u32> = l.kept.iter().copied().collect();
            json!({"name": format!("Body_LOD{}", i + 1), "vertices": l.kept.len(), "triangles": l.triangles.len(), "dir": format!("lod{}", i + 1),
                   "ring_vertices_kept": pack.ring.iter().filter(|r| kept.contains(&r.vertex)).count(), "collar_rings_weighted": COLLAR_RINGS})
        }))
        .collect::<Vec<_>>());
    let mut files = BTreeMap::new();
    for name in ["positions.npy", "triangles.npy", "triangle_uvs.npy", "ring.npy", "ring_params.npy", "ring_targets.npy", "seam_loop.npy", "seam_base.npy"] {
        let bytes = std::fs::read(dir.join(name)).map_err(|e| format!("{name}: {e}"))?;
        files.insert(name.to_string(), hash::sha256_hex(&bytes));
    }
    m["sha256"] = json!(files);
    std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?).map_err(|e| format!("manifest.json: {e}"))?;
    Ok(m)
}

fn load_helpers(dir: &Path) -> Result<b::Helpers, String> {
    if !dir.join("source.npy").is_file() {
        return Ok(b::Helpers::default());
    }
    let source = read_u32(&dir.join("source.npy"))?;
    let positions = read_points(&dir.join("positions.npy"))?;
    let text = std::fs::read_to_string(dir.join("groups.json")).map_err(|e| format!("helpers/groups.json: {e}"))?;
    let groups: BTreeMap<String, Vec<u32>> = serde_json::from_str(&text).map_err(|e| format!("helpers/groups.json: {e}"))?;
    if positions.len() != source.len() || groups.values().flatten().any(|&i| i as usize >= source.len()) {
        return Err(format!("{}: the helper arrays disagree", dir.display()));
    }
    Ok(b::Helpers { source, positions, groups })
}

pub fn load(dir: &Path) -> Result<(Value, BodyPack), String> {
    let text = std::fs::read_to_string(dir.join("manifest.json")).map_err(|e| format!("{}: {e}", dir.join("manifest.json").display()))?;
    let m: Value = serde_json::from_str(&text).map_err(|e| format!("manifest.json: {e}"))?;
    if m["format"] != json!(FORMAT) {
        return Err(format!("{} is not a body pack ({FORMAT})", dir.display()));
    }
    let positions = read_points(&dir.join("positions.npy"))?;
    let tris: Array2<u32> = read_npy(dir.join("triangles.npy")).map_err(|e| format!("triangles.npy: {e}"))?;
    let uvs: Array2<f32> = read_npy(dir.join("triangle_uvs.npy")).map_err(|e| format!("triangle_uvs.npy: {e}"))?;
    let ring_v = read_u32(&dir.join("ring.npy"))?;
    let ring_p: Array1<f64> = read_npy(dir.join("ring_params.npy")).map_err(|e| format!("ring_params.npy: {e}"))?;
    let mesh = Mesh {
        positions,
        triangles: tris.rows().into_iter().map(|r| [r[0], r[1], r[2]]).collect(),
        uvs: uvs.rows().into_iter().map(|r| [[r[0], r[1]], [r[2], r[3]], [r[4], r[5]]]).collect(),
    };
    let n = mesh.positions.len() as u32;
    if mesh.triangles.iter().flatten().any(|&v| v >= n) || mesh.uvs.len() != mesh.triangles.len() || ring_v.len() != ring_p.len() {
        return Err(format!("{}: the body arrays disagree", dir.display()));
    }
    let mut lods = Vec::new();
    for i in 1.. {
        let d = dir.join(format!("lod{i}"));
        if !d.is_dir() {
            break;
        }
        let l = crate::game_base::load(&d)?;
        lods.push(BodyLod { kept: l.kept, triangles: l.triangles, uvs: l.triangle_uvs });
    }
    let pack = BodyPack {
        mode: m["mode"].as_str().unwrap_or("").to_string(),
        mesh,
        ring: ring_v.iter().zip(ring_p.iter()).map(|(&vertex, &param)| RingVertex { vertex, param }).collect(),
        ring_targets: read_points(&dir.join("ring_targets.npy"))?,
        seam_loop: read_u32(&dir.join("seam_loop.npy"))?,
        seam_base: read_points(&dir.join("seam_base.npy"))?,
        per_span: m["per_span"].as_u64().unwrap_or(16) as usize,
        lods,
        source: if dir.join("source.npy").is_file() { read_u32(&dir.join("source.npy"))? } else { Vec::new() },
        helpers: load_helpers(&dir.join("helpers"))?,
        cage_source: if dir.join("cage_source.npy").is_file() { read_u32(&dir.join("cage_source.npy"))? } else { Vec::new() },
        fine: None,
    };
    let mut pack = pack;
    if let Some(level) = m["fine"]["level"].as_u64().filter(|&l| l > 0) {
        let d = dir.join("fine");
        let q: Array2<u32> = read_npy(d.join("quads.npy")).map_err(|e| format!("fine/quads.npy: {e}"))?;
        let u: Array2<f32> = read_npy(d.join("quad_uvs.npy")).map_err(|e| format!("fine/quad_uvs.npy: {e}"))?;
        let quads = q.rows().into_iter().map(|r| [r[0], r[1], r[2], r[3]]).collect();
        let quad_uvs = u.rows().into_iter().map(|r| [[r[0], r[1]], [r[2], r[3]], [r[4], r[5]], [r[6], r[7]]]).collect();
        let mut fine = b::Fine::new(quads, quad_uvs, pack.mesh.positions.len(), level as usize, &pack.ring, &pack.ring_targets)?;
        for i in 0.. {
            let dd = d.join(format!("lod{i}"));
            if !dd.is_dir() {
                break;
            }
            let l = crate::game_base::load(&dd)?;
            if l.kept.iter().any(|&k| k as usize >= fine.stencil.rows.len()) {
                return Err(format!("{}: a fine LOD keeps a vertex the refined body does not have", dd.display()));
            }
            fine.lods.push(BodyLod { kept: l.kept, triangles: l.triangles, uvs: l.triangle_uvs });
        }
        pack.fine = Some(fine);
    }
    if !pack.source.is_empty() && pack.source.len() != pack.mesh.positions.len() {
        return Err(format!("{}: source.npy has {} entries for {} vertices", dir.display(), pack.source.len(), pack.mesh.positions.len()));
    }
    if pack.ring_targets.len() != pack.ring.len() || pack.seam_base.len() != pack.seam_loop.len() {
        return Err(format!("{}: ring or seam arrays disagree", dir.display()));
    }
    Ok((m, pack))
}

pub struct DenseQuads {
    pub positions: Vec<Point>,
    pub quads: Vec<[u32; 4]>,
}

pub fn read_dense_quads(dir: &Path) -> Result<DenseQuads, String> {
    let sizes: Array1<u32> = read_npy(dir.join("face_sizes.npy")).map_err(|e| format!("face_sizes.npy: {e}"))?;
    let corners: Array1<u32> = read_npy(dir.join("corner_vertices.npy")).map_err(|e| format!("corner_vertices.npy: {e}"))?;
    if sizes.iter().any(|&s| s != 4) || corners.len() != sizes.len() * 4 {
        return Err("the dense base is not all quads".into());
    }
    let quads = corners.as_slice().ok_or("corner array not contiguous")?.chunks(4).map(|c| [c[0], c[1], c[2], c[3]]).collect();
    Ok(DenseQuads { positions: crate::dense::read_points(&dir.join("positions.npy"))?, quads })
}

pub struct Seam {
    pub loop_ids: Vec<u32>,
    pub per_span: usize,
    pub cage: Vec<[u32; 4]>,
}

pub fn dense_seam(dense: &DenseQuads) -> Result<Seam, String> {
    let cage = b::patch_corners(&dense.quads, 256)?;
    let corners: HashSet<u32> = cage.iter().flatten().copied().collect();
    let loops = b::oriented_loops(&dense.quads)?;
    if loops.len() != 1 {
        return Err(format!("the dense base has {} boundary loops; one neck boundary is expected", loops.len()));
    }
    let mut lp = loops.into_iter().next().unwrap();
    let start = lp.iter().enumerate().filter(|(_, v)| corners.contains(v)).min_by_key(|(_, v)| **v).map(|(i, _)| i).ok_or("no cage corner on the boundary")?;
    lp.rotate_left(start);
    let at: Vec<usize> = lp.iter().enumerate().filter(|(_, v)| corners.contains(v)).map(|(i, _)| i).collect();
    let per_span = lp.len() / at.len();
    if at.iter().enumerate().any(|(k, &i)| i != k * per_span) {
        return Err("cage corners are not evenly spaced along the dense boundary".into());
    }
    Ok(Seam { loop_ids: lp, per_span, cage })
}

fn lod_settings(target: usize) -> Settings {
    Settings { target_vertices: target, min_angle_deg: 4.0, max_angle_deg: 170.0, max_normal_turn_deg: 45.0, boundary_weight: 100.0, improve_below_deg: 20.0, flip_flat_deg: 15.0 }
}

fn compact(positions: &[Point], triangles: &[[u32; 3]]) -> (Vec<Point>, Vec<[u32; 3]>, Vec<u32>) {
    let mut map: HashMap<u32, u32> = HashMap::new();
    let mut used: Vec<u32> = triangles.iter().flatten().copied().collect();
    used.sort_unstable();
    used.dedup();
    for (i, &v) in used.iter().enumerate() {
        map.insert(v, i as u32);
    }
    (used.iter().map(|&v| positions[v as usize]).collect(), triangles.iter().map(|t| t.map(|v| map[&v])).collect(), used)
}

pub const COLLAR_RINGS: usize = 2;
pub const FINE_COLLAR_ROWS: usize = 2;
const COLLAR_WEIGHT: f64 = 1e9;

fn make_lods(mesh: &Mesh, lods: &[usize], ring: &[u32], collar_rings: usize) -> Vec<BodyLod> {
    let wanted: Vec<usize> = lods.iter().copied().filter(|&t| t < mesh.positions.len()).collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); mesh.positions.len()];
    for t in &mesh.triangles {
        for k in 0..3 {
            neighbours[t[k] as usize].push(t[(k + 1) % 3]);
            neighbours[t[(k + 1) % 3] as usize].push(t[k]);
        }
    }
    let mut weights = vec![1.0; mesh.positions.len()];
    let mut front: Vec<u32> = ring.to_vec();
    for &v in ring {
        weights[v as usize] = COLLAR_WEIGHT;
    }
    for _ in 0..collar_rings {
        let mut next = Vec::new();
        for &v in &front {
            for &u in &neighbours[v as usize] {
                if weights[u as usize] != COLLAR_WEIGHT {
                    weights[u as usize] = COLLAR_WEIGHT;
                    next.push(u);
                }
            }
        }
        front = next;
    }
    decimate::decimate_levels(vec![mesh.positions.clone()], &mesh.triangles, &mesh.uvs, &lod_settings(wanted[0]), Some(&weights), &wanted[1..], None)
        .into_iter()
        .map(|d| BodyLod { kept: d.kept, triangles: d.triangles, uvs: d.triangle_uvs })
        .collect()
}

fn corner_uv(polys: &Polys, c: Option<u32>) -> Uv {
    c.map(|t| polys.uvs[t as usize]).map(|u| [u[0] as f32, u[1] as f32]).unwrap_or([0.0, 0.0])
}

pub struct MakeHumanInput<'a> {
    pub obj: &'a Path,
    pub group: &'a str,
    pub unit: f64,
    pub shaping: Vec<Shaping>,
    pub lods: &'a [usize],
    pub subdivide: usize,
    pub fine_lods: &'a [usize],
}

pub type Shaping = (String, f64, Vec<(u32, Point)>);

pub fn import_makehuman(input: &MakeHumanInput, dense_dir: &Path, out: &Path) -> Result<Value, String> {
    let dense = read_dense_quads(dense_dir)?;
    let seam = dense_seam(&dense)?;
    let polys = read_obj(input.obj, Some(input.group))?;
    if polys.faces.iter().any(|f| f.len() != 4) {
        return Err("the MakeHuman body group must be all quads".into());
    }
    let body_quads: Vec<[u32; 4]> = polys.faces.iter().map(|f| [f[0].0, f[1].0, f[2].0, f[3].0]).collect();
    let embedding = b::embed_quads(&seam.cage, &body_quads)?;
    let mut raw = polys.positions.clone();
    let mut shaped = Vec::new();
    for (name, w, deltas) in &input.shaping {
        for (i, d) in deltas {
            if let Some(p) = raw.get_mut(*i as usize) {
                for k in 0..3 {
                    p[k] += w * d[k];
                }
            }
        }
        shaped.push(json!({"target": name, "weight": w, "entries": deltas.len()}));
    }
    let world: Vec<Point> = raw.iter().map(|p| Up::Y.to_frame(*p, input.unit)).collect();
    let head_faces: HashSet<usize> = embedding.faces.iter().copied().collect();
    let mut triangles = Vec::new();
    let mut uvs = Vec::new();
    for (f, face) in polys.faces.iter().enumerate() {
        if head_faces.contains(&f) {
            continue;
        }
        for t in [[0, 1, 2], [0, 2, 3]] {
            triangles.push(t.map(|k| face[k].0));
            uvs.push(t.map(|k| corner_uv(&polys, face[k].1)));
        }
    }
    let seam_base: Vec<Point> = seam.loop_ids.iter().map(|&d| dense.positions[d as usize]).collect();
    let (controls, spline_residual) = b::bspline_controls(&seam_base, seam.per_span)?;
    let ring_obj: Vec<u32> = (0..controls.len()).map(|i| embedding.vertices[&seam.loop_ids[i * seam.per_span]]).collect();
    let ring_world: Vec<Point> = ring_obj.iter().map(|&v| world[v as usize]).collect();
    let shift = b::centroid(&controls.iter().zip(&ring_world).map(|(c, r)| [c[0] - r[0], c[1] - r[1], c[2] - r[2]]).collect::<Vec<_>>());
    let moved: Vec<Point> = world.iter().map(|p| [p[0] + shift[0], p[1] + shift[1], p[2] + shift[2]]).collect();
    let mut misfit: Vec<f64> = controls.iter().zip(&ring_obj).map(|(c, &v)| {
        let p = moved[v as usize];
        ((c[0] - p[0]).powi(2) + (c[1] - p[1]).powi(2) + (c[2] - p[2]).powi(2)).sqrt()
    }).collect();
    let (positions, triangles, used) = compact(&moved, &triangles);
    let local: HashMap<u32, u32> = used.iter().enumerate().map(|(i, &v)| (v, i as u32)).collect();
    let mesh = Mesh { positions, triangles, uvs };
    let ring: Vec<RingVertex> = ring_obj.iter().enumerate().map(|(i, v)| RingVertex { vertex: local[v], param: i as f64 }).collect();
    let quads: Vec<[u32; 4]> = polys.faces.iter().enumerate().filter(|(f, _)| !head_faces.contains(f)).map(|(_, face)| [0, 1, 2, 3].map(|k| local[&face[k].0])).collect();
    let quad_uvs: Vec<[gf_geom::Uv; 4]> = polys.faces.iter().enumerate().filter(|(f, _)| !head_faces.contains(f)).map(|(_, face)| [0, 1, 2, 3].map(|k| corner_uv(&polys, face[k].1))).collect();
    let (lods, fine) = if input.subdivide == 0 {
        (make_lods(&mesh, input.lods, &ring.iter().map(|r| r.vertex).collect::<Vec<_>>(), COLLAR_RINGS), None)
    } else {
        let mut fine = b::Fine::new(quads, quad_uvs, mesh.positions.len(), input.subdivide, &ring, &controls)?;
        let all = fine.stencil.apply(&mesh.positions);
        for (i, &target) in input.fine_lods.iter().enumerate() {
            let level = if i == 0 { input.subdivide } else { input.subdivide.min(1) };
            let (triangles, uvs) = if level == input.subdivide {
                (fine.triangles.clone(), fine.uvs.clone())
            } else {
                let l = gf_core::subdiv::subdivide(&fine.quads, &fine.quad_uvs, mesh.positions.len(), level)?;
                gf_core::subdiv::triangles(&l.quads, &l.uvs)
            };
            let n = triangles.iter().flatten().map(|&v| v as usize + 1).max().unwrap_or(0);
            let source = Mesh { positions: all[..n].to_vec(), triangles, uvs };
            let ring: Vec<u32> = fine.ring.iter().map(|r| r.vertex).filter(|&v| (v as usize) < n).collect();
            let mut made = make_lods(&source, &[target], &ring, FINE_COLLAR_ROWS << level);
            if made.len() != 1 {
                return Err(format!("fine LOD{i}: {target} vertices is not below the {n} vertices of subdivision level {level}"));
            }
            fine.lods.push(made.remove(0));
        }
        (Vec::new(), Some(fine))
    };
    let kept: HashSet<u32> = used.iter().copied().collect();
    let cut: Vec<u32> = (0..moved.len() as u32).filter(|v| !kept.contains(v)).collect();
    let helpers = helpers_of(&read_obj_groups(input.obj, "joint-")?, &cut, &moved)?;
    let cage_count = embedding.vertices.len();
    let cage_source: Vec<u32> = (0..cage_count as u32).map(|d| embedding.vertices.get(&d).copied().ok_or_else(|| format!("cage vertex {d} has no base.obj vertex: the cage is not the dense base's first {cage_count} vertices"))).collect::<Result<_, _>>()?;
    let pack = BodyPack { mode: "makehuman".into(), mesh, ring, ring_targets: controls, seam_loop: seam.loop_ids.clone(), seam_base, per_span: seam.per_span, lods, source: used.clone(), helpers, cage_source, fine };
    misfit.sort_by(f64::total_cmp);
    let obj_bytes = std::fs::read(input.obj).map_err(|e| e.to_string())?;
    let manifest = json!({
        "source": {"obj": input.obj.file_name().map(|n| n.to_string_lossy().to_string()), "obj_sha256": hash::sha256_hex(&obj_bytes), "group": input.group, "unit_m": input.unit, "up": "y", "shaping": shaped},
        "correspondence": {"method": "topology: the dense base's level-4 patches give the cage quads; the cage embeds in the body quads in exactly one orientation-preserving way",
                           "cage_faces": seam.cage.len(), "cage_vertices": embedding.vertices.len(), "seeds_tried": embedding.seeds_tried,
                           "seam_dense_vertices": seam.loop_ids.len(), "ring_vertices": pack.ring.len(), "per_span": seam.per_span},
        "seam_spline_residual_m": spline_residual,
        "positions": "the shaped base mesh's own vertices (cage level), moved by one translation so its neck ring meets the head cage's ring (the spline controls of the head's seam)",
        "translation_m": shift,
        "ring_misfit_mm": {"median": misfit[misfit.len() / 2] * 1000.0, "max": misfit[misfit.len() - 1] * 1000.0,
                           "meaning": "distance from the body's neck ring to the head cage's ring after the translation: the shape difference the blend band absorbs"},
        "frame": "metres, z up, the dense base's frame",
    });
    save(out, &pack, &manifest)
}

pub struct MeshInput<'a> {
    pub obj: &'a Path,
    pub group: Option<&'a str>,
    pub unit: f64,
    pub up: Up,
    pub cut_z: Option<f64>,
    pub align: &'a str,
    pub lods: &'a [usize],
}

fn largest_component(n: usize, triangles: &[[u32; 3]]) -> Vec<bool> {
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    for t in triangles {
        let a = find(&mut parent, t[0]);
        for &v in &t[1..] {
            let r = find(&mut parent, v);
            parent[r as usize] = a;
        }
    }
    let mut count: HashMap<u32, usize> = HashMap::new();
    let roots: Vec<u32> = triangles.iter().map(|t| find(&mut parent, t[0])).collect();
    for r in &roots {
        *count.entry(*r).or_default() += 1;
    }
    let best = count.iter().max_by_key(|(r, c)| (**c, std::cmp::Reverse(**r))).map(|(r, _)| *r).unwrap_or(0);
    roots.iter().map(|r| *r == best).collect()
}

pub fn import_mesh(input: &MeshInput, dense_dir: &Path, out: &Path) -> Result<Value, String> {
    let dense = read_dense_quads(dense_dir)?;
    let seam = dense_seam(&dense)?;
    let seam_base: Vec<Point> = seam.loop_ids.iter().map(|&d| dense.positions[d as usize]).collect();
    let polys = read_obj(input.obj, input.group)?;
    let world: Vec<Point> = polys.positions.iter().map(|p| input.up.to_frame(*p, input.unit)).collect();
    let mut triangles = Vec::new();
    let mut uvs = Vec::new();
    for face in &polys.faces {
        for k in 1..face.len() - 1 {
            let c = [0, k, k + 1];
            triangles.push(c.map(|j| face[j].0));
            uvs.push(c.map(|j| corner_uv(&polys, face[j].1)));
        }
    }
    let total = triangles.len();
    if let Some(z) = input.cut_z {
        let keep: Vec<bool> = triangles.iter().map(|t| t.iter().all(|&v| world[v as usize][2] <= z)).collect();
        triangles = triangles.into_iter().zip(&keep).filter(|(_, k)| **k).map(|(t, _)| t).collect();
        uvs = uvs.into_iter().zip(&keep).filter(|(_, k)| **k).map(|(u, _)| u).collect();
    }
    let keep = largest_component(world.len(), &triangles);
    let triangles: Vec<[u32; 3]> = triangles.into_iter().zip(&keep).filter(|(_, k)| **k).map(|(t, _)| t).collect();
    let uvs: Vec<[Uv; 3]> = uvs.into_iter().zip(&keep).filter(|(_, k)| **k).map(|(u, _)| u).collect();
    let loops = b::oriented_loops(&triangles)?;
    if loops.is_empty() {
        return Err("the body is closed: pass cut_z to cut it at the neck".into());
    }
    let top = loops.iter().map(|l| b::centroid(&l.iter().map(|&v| world[v as usize]).collect::<Vec<_>>())[2]).fold(f64::NEG_INFINITY, f64::max);
    let perimeter = |l: &Vec<u32>| -> f64 {
        (0..l.len()).map(|i| {
            let (p, q) = (world[l[i] as usize], world[l[(i + 1) % l.len()] as usize]);
            ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
        }).sum()
    };
    let neck = loops
        .iter()
        .filter(|l| b::centroid(&l.iter().map(|&v| world[v as usize]).collect::<Vec<_>>())[2] >= top - 0.05)
        .max_by(|a, b| perimeter(a).total_cmp(&perimeter(b)))
        .cloned()
        .ok_or("no neck loop")?;
    let ring_points: Vec<Point> = neck.iter().map(|&v| world[v as usize]).collect();
    let ((cb, rb), (ch, rh)) = (b::loop_centre_radius(&ring_points), b::loop_centre_radius(&seam_base));
    let (scale, shift) = match input.align {
        "similarity" => (rh / rb, [ch[0] - cb[0], ch[1] - cb[1], ch[2] - cb[2]]),
        "translate" => (1.0, [ch[0] - cb[0], ch[1] - cb[1], ch[2] - cb[2]]),
        "none" => (1.0, [0.0; 3]),
        other => return Err(format!("align must be translate, similarity or none, got {other}")),
    };
    let aligned: Vec<Point> = world.iter().map(|p| [cb[0] + shift[0] + scale * (p[0] - cb[0]), cb[1] + shift[1] + scale * (p[1] - cb[1]), cb[2] + shift[2] + scale * (p[2] - cb[2])]).collect();
    let mut ring_ids = neck.clone();
    let ring_aligned: Vec<Point> = ring_ids.iter().map(|&v| aligned[v as usize]).collect();
    if b::signed_area_xy(&ring_aligned).signum() != b::signed_area_xy(&seam_base).signum() {
        ring_ids.reverse();
    }
    let angle = |p: Point| (p[1] - ch[1]).atan2(p[0] - ch[0]);
    let a0 = angle(seam_base[0]);
    let start = (0..ring_ids.len())
        .min_by(|&i, &j| {
            let di = b::cyclic_delta(a0, angle(aligned[ring_ids[i] as usize]), std::f64::consts::TAU).abs();
            let dj = b::cyclic_delta(a0, angle(aligned[ring_ids[j] as usize]), std::f64::consts::TAU).abs();
            di.total_cmp(&dj)
        })
        .unwrap_or(0);
    ring_ids.rotate_left(start);
    let ring_aligned: Vec<Point> = ring_ids.iter().map(|&v| aligned[v as usize]).collect();
    let (fitted, offset, fit_rms) = b::fit_params(&ring_aligned, &seam_base, seam.per_span as f64);
    let first = (0..fitted.len()).min_by(|&a, &c| fitted[a].total_cmp(&fitted[c])).unwrap_or(0);
    let order: Vec<usize> = (0..fitted.len()).map(|k| (k + first) % fitted.len()).collect();
    let ring_ids: Vec<u32> = order.iter().map(|&k| ring_ids[k]).collect();
    let ring_aligned: Vec<Point> = order.iter().map(|&k| ring_aligned[k]).collect();
    let params: Vec<f64> = order.iter().map(|&k| fitted[k]).collect();
    let targets: Vec<Point> = params.iter().map(|&p| b::loop_point(&seam_base, p, seam.per_span as f64)).collect();
    let mut misfit: Vec<f64> = targets.iter().zip(&ring_aligned).map(|(t, p)| ((t[0] - p[0]).powi(2) + (t[1] - p[1]).powi(2) + (t[2] - p[2]).powi(2)).sqrt()).collect();
    let (positions, triangles, used) = compact(&aligned, &triangles);
    let local: HashMap<u32, u32> = used.iter().enumerate().map(|(i, &v)| (v, i as u32)).collect();
    let mesh = Mesh { positions, triangles, uvs };
    let ring: Vec<RingVertex> = ring_ids.iter().zip(&params).map(|(v, &p)| RingVertex { vertex: local[v], param: p }).collect();
    let lods = make_lods(&mesh, input.lods, &ring.iter().map(|r| r.vertex).collect::<Vec<_>>(), COLLAR_RINGS);
    if ring.windows(2).any(|w| w[1].param < w[0].param) {
        return Err("the neck ring's parameters are not monotone".into());
    }
    let pack = BodyPack { mode: "mesh".into(), mesh, ring, ring_targets: targets, seam_loop: seam.loop_ids.clone(), seam_base, per_span: seam.per_span, lods, source: Vec::new(), helpers: b::Helpers::default(), cage_source: Vec::new(), fine: None };
    misfit.sort_by(f64::total_cmp);
    let obj_bytes = std::fs::read(input.obj).map_err(|e| e.to_string())?;
    let manifest = json!({
        "source": {"obj": input.obj.file_name().map(|n| n.to_string_lossy().to_string()), "obj_sha256": hash::sha256_hex(&obj_bytes),
                   "group": input.group, "unit_m": input.unit, "up": match input.up { Up::Y => "y", Up::Z => "z" }, "cut_z_m": input.cut_z},
        "neck": {"method": if input.cut_z.is_some() { "faces above the cut plane removed; the longest top boundary loop" } else { "the longest top boundary loop" },
                 "loops": loops.len(), "ring_vertices": pack.ring.len(), "triangles_in": total, "triangles_kept": pack.mesh.triangles.len()},
        "align": {"mode": input.align, "scale": scale, "translation_m": shift, "ring_to_seam_radius_ratio": rh / rb,
                  "arc_offset": offset, "arc_fit_rms_mm": fit_rms * 1000.0,
                  "method": "translate: ring centre onto the seam centre; similarity: also scale by the ratio of horizontal rms radii (both weighted by arc length); ring parameters: arc length with the start offset that best fits the seam, then each ring vertex moved to its closest seam point within 0.75 cage edges, kept increasing"},
        "ring_misfit_mm": {"median": misfit[misfit.len() / 2] * 1000.0, "max": misfit[misfit.len() - 1] * 1000.0},
        "frame": "metres, z up, the dense base's frame",
    });
    save(out, &pack, &manifest)
}
