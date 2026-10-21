use std::collections::HashMap;
use std::path::Path;

use gf_core::game_mesh::{self as gm, Point};
use gf_core::gates::{self, DenseSurface, ShellLimits};
use gf_core::hash;
use gf_core::placement::{DenseMesh, Placed, Placement, PlacementVersion};
use gf_geom::Uv;
use gf_geom::decimate::Decimated;
use gf_geom::lids;
use gf_io::character as ch;
use gf_io::dense::{self, Dense};
use gf_io::gltf::{self, Lod, MorphData, Sparse};
use gf_io::store::Store;
use gf_io::{game_base, obj};
use serde_json::{Value, json};

use super::{Options, count, need, number, only, placement_keys, placer, placer_with};

pub const SHELL_KEYS: &[&str] = &["dense", "before", "after", "fold-tolerance", "inner-tolerance", "rough-mm", "rough-rel", "rough-tolerance"];
pub const GAME_KEYS: &[&str] = &["dense", "base", "face", "obj", "tip-vertex", "gates", "seam-limit-mm", "triangles", "cells"];
pub const LIDS_GATE_KEYS: &[&str] = &["glb", "eyes", "states", "front-deg", "tolerance-mm", "cells", "dense", "base"];

fn cells_flag(opts: &Options) -> Result<bool, String> {
    match opts.get("cells").map(String::as_str) {
        None | Some("off") => Ok(false),
        Some("on") => Ok(true),
        Some(other) => Err(format!("--cells takes on or off, got {other}")),
    }
}
pub const PATH_KEYS: &[&str] = &["dense", "base", "start", "delta", "steps", "placement", "pop-limit", "plant"];
pub const GLB_KEYS: &[&str] = &[
    "store",
    "spec",
    "macro",
    "out",
    "morphs",
    "sparse",
    "lod-screen",
    "lod-pixels",
    "lod-height",
    "facing",
    "rig",
    "rig-jaw",
    "rig-rounds",
    "rig-correctives",
    "morph-dir",
    "morph-floor",
    "lids",
    "lod0-out",
];
pub const LID_EXPORT_KEYS: &[&str] = &["lids-step-mm", "lids-max-mm", "lids-front-deg", "lids-tolerance-mm", "lids-smooth"];
pub const EYES_KEY: &[&str] = &["eyes"];
pub const BODY_KEYS: &[&str] = &["body", "body-blend-mm", "body-neck-offset-mm", "head-scale", "body-scale", "body-smooth", "body-tangent"];
pub const BODY_RIG_KEYS: &[&str] = &["body-rig", "body-weights", "body-gates"];
const GENITAL_SINK_M: f64 = 0.002;
const GENITAL_MIN_BODY_VERTICES: usize = 2500;

fn face_normals(positions: &[Point], triangles: &[[u32; 3]]) -> Vec<Point> {
    let mut n = vec![[0.0; 3]; positions.len()];
    for t in triangles {
        let (a, b, c) = (positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let f = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        for &k in t {
            for d in 0..3 {
                n[k as usize][d] += f[d];
            }
        }
    }
    n.into_iter().map(|p| {
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-30);
        p.map(|x| x / l)
    }).collect()
}

pub struct GenitalMesh {
    pub level: gf_core::subdiv::Level,
    pub positions: Vec<Point>,
    pub normals: Vec<Point>,
    pub triangles: Vec<[u32; 3]>,
    pub uvs: Vec<[Uv; 3]>,
    pub rim: usize,
    pub gap_mm: f64,
    pub flipped: bool,
}

pub fn genital_mesh(pack: &gf_core::body::BodyPack, controls: &gf_core::body::Controls, level: usize, body_positions: &[Point], body_normals: &[Point]) -> Result<GenitalMesh, String> {
    let g = pack.genital.as_ref().ok_or("the spec asks for genitals but the body has no helper-genital patch: re-import it (body.import)")?;
    if pack.helpers.positions.is_empty() {
        return Err("the body carries no helper vertices: re-import it (body.import, then body.targets)".into());
    }
    let control = pack.control_joint(controls);
    let base = g.positions(&pack.helpers);
    let l = gf_core::subdiv::subdivide(&g.quads, &g.quad_uvs, base.len(), level)?;
    let mut positions: Vec<Point> = l.stencil.apply(&base).into_iter().map(&control).collect();
    let (mut triangles, mut uvs) = gf_core::subdiv::triangles(&l.quads, &l.uvs);
    let mut count: HashMap<(u32, u32), u32> = HashMap::new();
    for q in &l.quads {
        for k in 0..4 {
            *count.entry((q[k].min(q[(k + 1) % 4]), q[k].max(q[(k + 1) % 4]))).or_default() += 1;
        }
    }
    let mut rim: Vec<u32> = count.into_iter().filter(|e| e.1 == 1).flat_map(|e| [e.0.0, e.0.1]).collect();
    rim.sort_unstable();
    rim.dedup();
    let nearest = |p: Point| -> usize {
        let d = |q: &Point| (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2);
        (0..body_positions.len()).min_by(|&a, &b| d(&body_positions[a]).total_cmp(&d(&body_positions[b]))).unwrap_or(0)
    };
    let before = face_normals(&positions, &triangles);
    let mut facing = 0.0;
    let mut gap_mm: f64 = 0.0;
    for &v in &rim {
        let j = nearest(positions[v as usize]);
        let (q, n) = (body_positions[j], body_normals[j]);
        facing += before[v as usize][0] * n[0] + before[v as usize][1] * n[1] + before[v as usize][2] * n[2];
        let p = positions[v as usize];
        gap_mm = gap_mm.max(((p[0] - q[0]) * n[0] + (p[1] - q[1]) * n[1] + (p[2] - q[2]) * n[2]).abs() * 1000.0);
        positions[v as usize] = [q[0] - GENITAL_SINK_M * n[0], q[1] - GENITAL_SINK_M * n[1], q[2] - GENITAL_SINK_M * n[2]];
    }
    let flipped = facing < 0.0;
    if flipped {
        for (t, u) in triangles.iter_mut().zip(uvs.iter_mut()) {
            t.swap(1, 2);
            u.swap(1, 2);
        }
    }
    let normals = face_normals(&positions, &triangles);
    Ok(GenitalMesh { level: l, positions, normals, triangles, uvs, rim: rim.len(), gap_mm, flipped })
}

fn genital_lod(
    pack: &gf_core::body::BodyPack,
    controls: &gf_core::body::Controls,
    level: usize,
    body_positions: &[Point],
    body_normals: &[Point],
    by_obj: Option<&HashMap<u32, gf_core::body_rig::Influences>>,
    name: String,
) -> Result<(Lod, Value), String> {
    let GenitalMesh { level: l, positions, normals, triangles, uvs, rim, gap_mm, flipped } = genital_mesh(pack, controls, level, body_positions, body_normals)?;
    let g = pack.genital.as_ref().ok_or("the body has no helper-genital patch")?;
    let surface = gm::split_seams(&triangles, &uvs);
    let skin = match by_obj {
        Some(w) => {
            let none: Vec<(usize, f64)> = Vec::new();
            let infl: Vec<gf_core::body_rig::Influences> = surface
                .source
                .iter()
                .map(|&v| {
                    let parts: Vec<(f64, &[(usize, f64)])> = l.stencil.rows[v as usize]
                        .iter()
                        .map(|&(c, k)| (k, w.get(&pack.helpers.source[g.helpers[c as usize] as usize]).map(Vec::as_slice).unwrap_or(&none)))
                        .collect();
                    let x = gf_core::body_rig::strongest(&gf_core::body_rig::mix(&parts), 4);
                    if x.is_empty() { vec![(0, 1.0)] } else { x }
                })
                .collect();
            Some(skin_data(&infl.iter().map(Vec::as_slice).collect::<Vec<_>>()))
        }
        None => None,
    };
    let report = json!({"node": name, "subdivision_level": level, "vertices": surface.source.len(), "triangles": triangles.len(), "rim_vertices": rim,
                        "rim_off_body_before_mm": gap_mm, "winding_flipped": flipped});
    let lod = Lod {
        name,
        positions: surface.source.iter().map(|&v| gm::to_gltf(positions[v as usize])).collect(),
        normals: surface.source.iter().map(|&v| gm::to_gltf(normals[v as usize])).collect(),
        uvs: surface.uvs.clone(),
        indices: surface.indices,
        morphs: Vec::new(),
        skin,
        material: None,
    };
    Ok((lod, report))
}

fn wearable_material(store: &Store, item: &gf_io::wearable_store::Wearable) -> Result<gltf::Material, String> {
    let texture = |slot: &str| item.manifest.material.get(slot).map(|r| store.get_ref(r)).transpose();
    Ok(gltf::Material {
        name: item.manifest.name.clone(),
        base_color: [1.0, 1.0, 1.0, 1.0],
        base_texture: texture("diffuse")?,
        normal_texture: texture("normal")?,
        orm_texture: None,
        metallic: 0.0,
        roughness: 0.85,
        transmission: None,
        ior: None,
        blend: false,
        alpha_cutoff: item.manifest.transparent.then_some(0.5),
        double_sided: item.manifest.double_sided,
        extension: None,
    })
}

const WEARABLE_MIN_VERTICES: usize = 32;

type GarmentLevel = (Vec<u32>, Vec<[u32; 3]>, Vec<[gf_geom::Uv; 3]>);

fn wearable_ladder(item: &gf_io::wearable_store::Wearable, garment: &[Point], body_counts: &[usize]) -> Vec<GarmentLevel> {
    let mut triangles = Vec::new();
    let mut uvs: Vec<[gf_geom::Uv; 3]> = Vec::new();
    let uv = |c: Option<u32>| c.and_then(|t| item.uvs.get(t as usize)).map_or([0.0f32, 0.0], |u| [u[0] as f32, 1.0 - u[1] as f32]);
    for f in &item.faces {
        for k in 1..f.len().saturating_sub(1) {
            triangles.push([f[0].0, f[k].0, f[k + 1].0]);
            uvs.push([uv(f[0].1), uv(f[k].1), uv(f[k + 1].1)]);
        }
    }
    let n = garment.len();
    let mut out: Vec<GarmentLevel> = vec![((0..n as u32).collect(), triangles.clone(), uvs.clone())];
    if body_counts.len() > 1 && body_counts[0] > 0 {
        let mut targets = Vec::new();
        let mut above = n;
        for &b in &body_counts[1..] {
            let t = ((n as f64 * b as f64 / body_counts[0] as f64).round() as usize).max(WEARABLE_MIN_VERTICES).min(above.saturating_sub(1));
            if t < WEARABLE_MIN_VERTICES {
                break;
            }
            targets.push(t);
            above = t;
        }
        if !targets.is_empty() {
            let settings = gf_geom::decimate::Settings {
                target_vertices: targets[0],
                min_angle_deg: 4.0,
                max_angle_deg: 170.0,
                max_normal_turn_deg: 45.0,
                boundary_weight: 100.0,
                improve_below_deg: 20.0,
                flip_flat_deg: 15.0,
            };
            for level in gf_geom::decimate::decimate_levels(vec![garment.to_vec()], &triangles, &uvs, &settings, None, &targets[1..], None) {
                out.push((level.kept, level.triangles, level.triangle_uvs));
            }
        }
    }
    out
}

fn garment_lod(item: &gf_io::wearable_store::Wearable, positions: &[Point], kept: &[u32], triangles: &[[u32; 3]], uvs: &[[gf_geom::Uv; 3]], by_obj: Option<&HashMap<u32, gf_core::body_rig::Influences>>, material: usize) -> Lod {
    let normals = super::wearable::vertex_normals(&positions, triangles);
    let surface = gm::split_seams(triangles, uvs);
    let skin = by_obj.map(|w| {
        let none: Vec<(usize, f64)> = Vec::new();
        let infl: Vec<gf_core::body_rig::Influences> = surface
            .source
            .iter()
            .map(|&v| {
                let b = &item.binding.bound[kept[v as usize] as usize];
                let parts: Vec<(f64, &[(usize, f64)])> = (0..3).map(|k| (b.weights[k], w.get(&b.vertices[k]).map(Vec::as_slice).unwrap_or(&none))).collect();
                let x = gf_core::body_rig::strongest(&gf_core::body_rig::mix(&parts), 4);
                if x.is_empty() { vec![(0, 1.0)] } else { x }
            })
            .collect();
        skin_data(&infl.iter().map(Vec::as_slice).collect::<Vec<_>>())
    });
    let name: String = item.manifest.name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    Lod {
        name,
        positions: surface.source.iter().map(|&v| gm::to_gltf(positions[v as usize])).collect(),
        normals: surface.source.iter().map(|&v| gm::to_gltf(normals[v as usize])).collect(),
        uvs: surface.uvs.clone(),
        indices: surface.indices,
        morphs: Vec::new(),
        skin,
        material: Some(material),
    }
}

fn skin_data(influences: &[&[(usize, f64)]]) -> gltf::SkinData {
    let mut joints = Vec::with_capacity(influences.len());
    let mut weights = Vec::with_capacity(influences.len());
    for inf in influences {
        let (mut j, mut w) = ([0u8; 4], [0.0f32; 4]);
        for (k, &(joint, weight)) in inf.iter().take(4).enumerate() {
            j[k] = joint as u8;
            w[k] = weight as f32;
        }
        joints.push(j);
        weights.push(w);
    }
    gltf::SkinData { joints, weights, joints_1: Vec::new(), weights_1: Vec::new() }
}

fn body_options(opts: &Options) -> Result<Option<(gf_core::body::BodyPack, Value, gf_core::body::Controls)>, String> {
    let Some(dir) = opts.get("body") else {
        if let Some(k) = BODY_KEYS.iter().find(|k| opts.contains_key(**k)) {
            return Err(format!("--{k} needs --body PACK"));
        }
        return Ok(None);
    };
    let (manifest, pack) = gf_io::body::load(Path::new(dir))?;
    let controls = gf_core::body::Controls {
        blend_m: number(opts, "body-blend-mm", 60.0)? / 1000.0,
        neck_offset_m: number(opts, "body-neck-offset-mm", 0.0)? / 1000.0,
        head_scale: number(opts, "head-scale", 1.0)?,
        body_scale: number(opts, "body-scale", 1.0)?,
        smooth: number(opts, "body-smooth", 0.0)?.round().max(0.0) as usize,
        tangent: number(opts, "body-tangent", 0.5)?,
    };
    controls.check()?;
    Ok(Some((pack, manifest, controls)))
}
const LID_FRONT_DEG: f64 = 75.0;
const FACING_ROUNDS: usize = 8;
const UNTANGLE_ITERATIONS: usize = 200;
const UNTANGLE_MAX_MM: f64 = 2.0;
const STATE_UNTANGLE_ITERATIONS: usize = 400;
const STATE_UNTANGLE_MAX_MM: f64 = 4.0;
const TOUCH_ROUNDS: usize = 4;
const SETTLE_ROUNDS: usize = 4;
const TOUCH_MARGIN: f64 = 1e-5;
const WEIGHT_STEPS: f64 = 65536.0;
const EYE_RINGS: u32 = 16;
const EYE_SEGMENTS: u32 = 24;
const NORMAL_FLOOR: f32 = 1e-3;
pub const GAME_GATES: &[&str] = &["triq", "uv", "census", "seams"];

pub fn verdict(command: &str, gates: Value, failed: Vec<String>, extra: Value) -> Value {
    let pass = failed.is_empty();
    let mut out = json!({"ok": pass, "command": command, "pass": pass, "gates": gates, "failed": failed});
    if !pass {
        out["error"] = json!(format!("gates failed: {}", out["failed"].as_array().unwrap().iter().map(|v| v.as_str().unwrap_or("")).collect::<Vec<_>>().join(", ")));
    }
    if let Value::Object(map) = extra {
        for (k, v) in map {
            out[k] = v;
        }
    }
    out
}

pub fn gates_shell(opts: &Options) -> Result<Value, String> {
    only(opts, "gates shell", &[SHELL_KEYS])?;
    let limits = ShellLimits {
        fold_tolerance: count(opts, "fold-tolerance", 10)?,
        inner_tolerance: count(opts, "inner-tolerance", 15)?,
        rough_mm: number(opts, "rough-mm", 0.02)?,
        rough_relative: number(opts, "rough-rel", 0.05)?,
        rough_tolerance: count(opts, "rough-tolerance", 200)?,
    };
    let dense = Dense::load(Path::new(need(opts, "dense")?))?;
    let quads = dense.quads()?;
    let before = dense.positions_from(need(opts, "before")?)?;
    let after = dense.positions_from(need(opts, "after")?)?;
    let report = gates::shell(&before, &after, &quads, &limits)?;
    gates::check_frame(&before, report.tip)?;
    let failed = if report.pass { Vec::new() } else { vec!["shell".to_string()] };
    let failures = report.failures.clone();
    Ok(verdict("gates shell", json!({"shell": report}), failed, json!({"failures": failures})))
}

pub fn gates_game(opts: &Options) -> Result<Value, String> {
    only(opts, "gates game", &[GAME_KEYS])?;
    let chosen: Vec<String> = match opts.get("gates") {
        Some(list) => list.split(',').map(str::to_string).collect(),
        None => GAME_GATES.iter().map(|s| s.to_string()).collect(),
    };
    if let Some(bad) = chosen.iter().find(|g| !GAME_GATES.contains(&g.as_str())) {
        return Err(format!("--gates takes a list of {}, got {bad}", GAME_GATES.join(",")));
    }
    let seam_limit = number(opts, "seam-limit-mm", 0.05)?;
    let dense = Dense::load(Path::new(need(opts, "dense")?))?;
    let base = game_base::load(Path::new(need(opts, "base")?))?;
    let face = dense.positions_from(need(opts, "face")?)?;
    let tip = dense.nose_tip(&face, count(opts, "tip-vertex", dense::NOSE_TIP_VERTEX)?)?;
    gates::check_frame(&face, tip)?;
    let path = Path::new(need(opts, "obj")?);
    let mesh = obj::read(path)?;
    if mesh.positions.len() != base.kept.len() {
        return Err(format!("{} has {} vertices, the game base keeps {}", path.display(), mesh.positions.len(), base.kept.len()));
    }
    let own_triangles = match opts.get("triangles").map(String::as_str).unwrap_or("base") {
        "base" => false,
        "obj" => true,
        other => return Err(format!("--triangles must be base or obj, got {other}")),
    };
    let triangles = if own_triangles {
        let own = mesh.triangles();
        if own.iter().flatten().any(|&i| i as usize >= mesh.positions.len()) {
            return Err(format!("a triangle of {} indexes past its vertices", path.display()));
        }
        own
    } else {
        if mesh.triangles() != base.triangles {
            return Err(format!("the triangles of {} differ from the game base's (--triangles obj reads a re-triangulated mesh)", path.display()));
        }
        base.triangles.clone()
    };
    let uvs = mesh.corner_uvs();
    let game = &mesh.positions;
    let mut out = serde_json::Map::new();
    let mut failed = Vec::new();
    for name in &chosen {
        let (value, pass) = match name.as_str() {
            "triq" => {
                let r = gates::triangle_quality(game, &triangles, tip);
                (json!(r), r.pass)
            }
            "uv" => {
                let r = gates::uv_health(game, &triangles, &uvs);
                (json!(r), r.pass)
            }
            "census" => {
                let r = gates::census_on(&face, &dense.triangles, &base, &triangles, game, tip);
                (json!(r), r.pass)
            }
            _ => {
                let surface = DenseSurface { positions: &face, triangles: &dense.triangles, triangle_uvs: &dense.triangle_uvs };
                let r = gates::seams(game, &triangles, &uvs, (&base.triangles, &base.triangle_uvs), &surface, tip, seam_limit);
                (json!(r), r.pass)
            }
        };
        if !pass {
            failed.push(name.clone());
        }
        out.insert(name.clone(), value);
    }
    let mut extra = json!({"tip_m": tip, "obj": path.display().to_string()});
    if own_triangles {
        extra["triangles"] = json!({"source": "obj", "count": triangles.len(), "differ_from_base": triangles != base.triangles});
    }
    let mut report = verdict("gates game", Value::Object(out), failed, extra);
    if cells_flag(opts)? {
        let cells = crate::grid::Cells::new(&dense.positions, &base.kept, &triangles)?;
        let n = cells.annotate(&mut report["gates"]);
        report["cells"] = json!({"chart": cells.chart.definition(), "annotated": n, "triangles": if own_triangles { "obj" } else { "base" }});
    }
    Ok(report)
}

fn glb_game_vertices(indices: &[u32], vertices: usize, triangles: &[[u32; 3]]) -> Result<Vec<u32>, String> {
    if indices.len() != triangles.len() * 3 {
        return Err(format!("{} triangles, the game base has {}", indices.len() / 3, triangles.len()));
    }
    let mut out = vec![u32::MAX; vertices];
    for (k, &i) in indices.iter().enumerate() {
        let g = triangles[k / 3][k % 3];
        let slot = out.get_mut(i as usize).ok_or("an index runs past the vertices")?;
        if *slot != u32::MAX && *slot != g {
            return Err(format!("vertex {i} sits on game vertices {} and {g}: the triangles do not follow the game base's order", *slot));
        }
        *slot = g;
    }
    Ok(out)
}

pub fn gates_path(opts: &Options) -> Result<Value, String> {
    let default = gf_core::placement::DEFAULT.to_string();
    let given = opts.get("placement").map(String::as_str).unwrap_or(&default);
    let version = PlacementVersion::parse(given)?;
    let mut allowed: Vec<&[&str]> = vec![PATH_KEYS];
    allowed.extend_from_slice(placement_keys(version.name));
    only(opts, &format!("gates path --placement {given}"), &allowed)?;
    let placement = placer(opts, version)?;
    let steps = count(opts, "steps", 20)?;
    if steps == 0 {
        return Err("--steps must be at least 1".into());
    }
    let limit = count(opts, "pop-limit", 10)?;
    let dense = Dense::load(Path::new(need(opts, "dense")?))?;
    let base = game_base::load(Path::new(need(opts, "base")?))?;
    let start = dense.positions_from(need(opts, "start")?)?;
    let delta = dense::read_points(Path::new(need(opts, "delta")?))?;
    if delta.len() != start.len() {
        return Err(format!("--delta has {} vertices, the dense base {}", delta.len(), start.len()));
    }
    let plant = match opts.get("plant") {
        Some(text) => {
            let parts: Vec<&str> = text.split(',').map(str::trim).collect();
            let parsed = match parts.as_slice() {
                [s, v, mm] => s.parse::<usize>().ok().zip(v.parse::<usize>().ok()).zip(mm.parse::<f64>().ok().filter(|x| x.is_finite())),
                _ => None,
            };
            let ((s, v), mm) = parsed.ok_or_else(|| format!("--plant takes STEP,GAME_VERTEX,MM, got {text}"))?;
            if s > steps || v >= base.kept.len() {
                return Err(format!("--plant {text}: the step must be at most {steps} and the vertex below {}", base.kept.len()));
            }
            Some((s, v, mm))
        }
        None => None,
    };
    let mesh = DenseMesh { triangles: &dense.triangles, triangle_uvs: &dense.triangle_uvs };
    let mut placed = Vec::new();
    let mut subset = Vec::new();
    for k in 0..=steps {
        crate::jobs::progress("place", k as u64, steps as u64 + 1, "path step");
        let t = k as f64 / steps as f64;
        let shape: Vec<Point> = start.iter().zip(&delta).map(|(s, d)| [0, 1, 2].map(|i| (s[i] + t * d[i]) as f32 as f64)).collect();
        let mut game = placement.place(&shape, &mesh, &base).game;
        if let Some((_, v, mm)) = plant.filter(|p| p.0 == k) {
            game[v][1] -= mm / 1000.0;
        }
        placed.push(game);
        subset.push(base.kept.iter().map(|&i| shape[i as usize]).collect::<Vec<_>>());
    }
    crate::jobs::progress("place", steps as u64 + 1, steps as u64 + 1, "path step");
    let report = gates::path_pops(&placed, &subset, limit)?;
    let failed = if report.pass { Vec::new() } else { vec!["path".to_string()] };
    let planted = plant.map(|(s, v, mm)| json!({"step": s, "game_vertex": v, "mm_forward": mm}));
    Ok(verdict("gates path", json!({"path": report}), failed, json!({"placement_version": placement.version().to_string(), "plant": planted})))
}

fn list_arg(text: &str) -> Result<Vec<String>, String> {
    if let Some(path) = text.strip_prefix('@') {
        let body = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        return serde_json::from_str(&body).map_err(|e| format!("{path}: expected a JSON list of target ids: {e}"));
    }
    Ok(text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
}

struct Level<'a> {
    name: String,
    mesh: &'a Decimated,
    map: Vec<u32>,
    uvs: Vec<[Uv; 3]>,
    triangles: Vec<[u32; 3]>,
}

fn lod_state(level: &Level, game0: &[Point]) -> (Vec<Point>, Vec<Point>) {
    let game: Vec<Point> = level.map.iter().map(|&j| game0[j as usize]).collect();
    let normals = gm::game_normals(&game, &level.triangles);
    (game, normals)
}

fn uv_slip(neutral: &[[Uv; 3]], moved: &[[Uv; 3]]) -> Value {
    let mut d: Vec<f64> = neutral
        .iter()
        .zip(moved)
        .flat_map(|(a, b)| (0..3).map(move |c| (((a[c][0] - b[c][0]) as f64).powi(2) + ((a[c][1] - b[c][1]) as f64).powi(2)).sqrt()))
        .collect();
    d.sort_by(f64::total_cmp);
    let n = d.len().max(1);
    json!({"corners": d.len(), "over_1e-4": d.iter().filter(|x| **x > 1e-4).count(), "max_uv": d.last().copied().unwrap_or(0.0),
           "p99_uv": d.get(((n - 1) as f64 * 0.99) as usize).copied().unwrap_or(0.0),
           "max_texels_2048": d.last().copied().unwrap_or(0.0) * 2048.0})
}

fn neck_loop(triangles: &[[u32; 3]], game: &[Point]) -> Result<Vec<u32>, String> {
    let loops = gf_core::rig::boundary_loops(triangles);
    let mean_up = |l: &Vec<u32>| l.iter().map(|&v| game[v as usize][2]).sum::<f64>() / l.len() as f64;
    loops.into_iter().min_by(|a, b| mean_up(a).total_cmp(&mean_up(b))).ok_or_else(|| "the game base has no open boundary for the neck".to_string())
}

fn mean_point(points: impl Iterator<Item = Point>) -> Point {
    let mut sum = [0.0; 3];
    let mut n = 0.0;
    for p in points {
        for k in 0..3 {
            sum[k] += p[k];
        }
        n += 1.0;
    }
    sum.map(|s| s / n)
}

pub fn gates_facing(opts: &Options) -> Result<Value, String> {
    only(opts, "gates facing", &[&["glb", "states"]])?;
    let path = need(opts, "glb")?;
    let states = opts.get("states").map(String::as_str).unwrap_or("neutral");
    if !matches!(states, "neutral" | "all") {
        return Err(format!("--states must be neutral or all, got {states}"));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let (_, meshes) = gltf::read_glb(&bytes)?;
    let mut failed = Vec::new();
    let mut rows = Vec::new();
    for mesh in &meshes {
        if mesh.normals.is_empty() {
            continue;
        }
        let tris: Vec<[u32; 3]> = mesh.indices.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
        let to64 = |v: &[[f32; 3]]| -> Vec<Point> { v.iter().map(|p| p.map(f64::from)).collect() };
        let count = |p: &[Point], n: &[Point]| tris.iter().filter(|t| gm::facing_dot(p, n, t) < 0.0).count();
        let (p0, n0) = (to64(&mesh.positions), to64(&mesh.normals));
        let neutral = count(&p0, &n0);
        let mut morph_counts = serde_json::Map::new();
        let mut worst = 0usize;
        for (k, m) in mesh.morphs.iter().enumerate() {
            let p: Vec<Point> = p0.iter().zip(&m.positions).map(|(a, d)| [0, 1, 2].map(|i| a[i] + d[i] as f64)).collect();
            let n: Vec<Point> = n0.iter().zip(&m.normals).map(|(a, d)| [0, 1, 2].map(|i| a[i] + d[i] as f64)).collect();
            let c = count(&p, &n);
            worst = worst.max(c);
            morph_counts.insert(mesh.morph_names.get(k).cloned().unwrap_or_else(|| k.to_string()), json!(c));
        }
        if neutral > 0 || (states == "all" && worst > 0) {
            failed.push(mesh.name.clone());
        }
        rows.push(json!({"mesh": mesh.name, "triangles": tris.len(), "against_vertex_normals": neutral,
                         "morph_states_worst": worst, "morph_states": morph_counts}));
    }
    if rows.is_empty() {
        return Err(format!("{path}: no mesh with normals to check"));
    }
    Ok(verdict("gates facing", json!({"facing": rows}), failed, json!({"glb": path, "states": states})))
}

pub fn from_gltf(p: [f32; 3]) -> Point {
    [f64::from(p[0]), -f64::from(p[2]), f64::from(p[1])]
}

pub fn lid_settings(opts: &Options) -> Result<(f64, f64), String> {
    let front = number(opts, "front-deg", LID_FRONT_DEG)?;
    if front > 90.0 {
        return Err(format!("--front-deg must be at most 90, got {front}"));
    }
    Ok((front, number(opts, "tolerance-mm", 0.0)? / 1000.0))
}

pub fn gates_lids(opts: &Options) -> Result<Value, String> {
    only(opts, "gates lids", &[LIDS_GATE_KEYS])?;
    let path = need(opts, "glb")?;
    let eyes = gf_io::load_eyes(Path::new(need(opts, "eyes")?))?;
    let states = opts.get("states").map(String::as_str).unwrap_or("all");
    if !matches!(states, "neutral" | "all") {
        return Err(format!("--states must be neutral or all, got {states}"));
    }
    let (front, tolerance) = lid_settings(opts)?;
    let cells = if cells_flag(opts)? {
        let dense = Dense::load(Path::new(need(opts, "dense").map_err(|e| format!("--cells on {e}"))?))?;
        let base = game_base::load(Path::new(need(opts, "base").map_err(|e| format!("--cells on {e}"))?))?;
        Some(crate::grid::Cells::new(&dense.positions, &base.kept, &base.triangles)?)
    } else {
        if let Some(k) = ["dense", "base"].iter().find(|k| opts.contains_key(**k)) {
            return Err(format!("--{k} is read only with --cells on"));
        }
        None
    };
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let (_, meshes) = gltf::read_glb(&bytes)?;
    let mut failed = Vec::new();
    let mut rows = Vec::new();
    let mut annotated = 0usize;
    for mesh in meshes.iter().filter(|m| !m.normals.is_empty() && !m.name.starts_with("Eye_")) {
        let mut seen: HashMap<[u32; 3], ()> = HashMap::new();
        let unique: Vec<usize> = (0..mesh.positions.len()).filter(|&i| seen.insert(mesh.positions[i].map(f32::to_bits), ()).is_none()).collect();
        let check = |dp: Option<&MorphData>| {
            let p: Vec<Point> = unique.iter().map(|&i| {
                let d = dp.map_or([0.0; 3], |m| m.positions[i]);
                from_gltf([0, 1, 2].map(|k| mesh.positions[i][k] + d[k]))
            }).collect();
            let n: Vec<Point> = unique.iter().map(|&i| {
                let d = dp.map_or([0.0; 3], |m| m.normals[i]);
                from_gltf([0, 1, 2].map(|k| mesh.normals[i][k] + d[k]))
            }).collect();
            lids::lid_check(&p, &n, &eyes, front, tolerance)
        };
        let neutral = check(None);
        let mut morph_counts = serde_json::Map::new();
        let (mut worst, mut failing, mut deepest) = (0usize, 0usize, 0.0f64);
        let mut morph_inside: Vec<usize> = Vec::new();
        if states == "all" {
            for (k, m) in mesh.morphs.iter().enumerate() {
                let c = check(Some(m));
                worst = worst.max(c.inside);
                failing += (c.inside > 0) as usize;
                deepest = deepest.max(c.deepest_mm);
                morph_counts.insert(mesh.morph_names.get(k).cloned().unwrap_or_else(|| k.to_string()), json!(c.inside));
                if cells.is_some() {
                    morph_inside.extend(&c.inside_vertices);
                }
            }
        }
        if neutral.inside > 0 || worst > 0 {
            failed.push(mesh.name.clone());
        }
        let mut row = json!({"mesh": mesh.name, "vertices": unique.len(), "neutral_inside": neutral.inside, "neutral_deepest_mm": neutral.deepest_mm,
                         "candidates": neutral.candidates, "morph_states_worst": worst, "morph_states_failing": failing,
                         "morph_states_deepest_mm": deepest, "morph_states": morph_counts});
        if let Some(cells) = &cells {
            match glb_game_vertices(&mesh.indices, mesh.positions.len(), &cells.triangles) {
                Ok(game_of) => {
                    let tally = |list: &[usize]| -> Value {
                        let mut m: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
                        for &i in list {
                            let g = game_of[unique[i]];
                            let name = cells.game_cell(g as usize).map_or("none".to_string(), gf_geom::face_chart::cell_name);
                            *m.entry(name).or_default() += 1;
                        }
                        json!(m)
                    };
                    let mut games: Vec<u32> = neutral.inside_vertices.iter().map(|&i| game_of[unique[i]]).collect();
                    games.sort_unstable();
                    games.dedup();
                    annotated += neutral.inside_vertices.len() + morph_inside.len();
                    row["cells"] = json!({"neutral_inside_game_vertices": games, "neutral_inside_cells": tally(&neutral.inside_vertices),
                                          "morph_inside_cells": tally(&morph_inside)});
                }
                Err(why) => row["cells"] = json!({"error": format!("{}: {why}; cells are read for the finest level only", mesh.name)}),
            }
        }
        rows.push(row);
    }
    if rows.is_empty() {
        return Err(format!("{path}: no mesh with normals to check"));
    }
    let rule = "a vertex fails when it lies within 0.5..1.3 eye radii of an eye centre, within front-deg of its gaze, its normal points away from the centre, and it is closer to the centre than the larger of the eye radius and the eye mesh's outermost surface along that direction, by more than tolerance-mm";
    let mut report = verdict("gates lids", json!({"lids": rows}), failed, json!({"glb": path, "states": states, "front_deg": front, "tolerance_mm": tolerance * 1000.0, "rule": rule}));
    if let Some(cells) = &cells {
        report["cells"] = json!({"chart": cells.chart.definition(), "annotated": annotated,
                                 "rule": "a glTF vertex is the game vertex at the same triangle corner (the finest level's triangles follow the game base's order); a level that does not is reported with an error"});
    }
    Ok(report)
}

struct LidRun {
    eyes: Vec<lids::Eye>,
    step: f64,
    max: f64,
    front: f64,
    tolerance: f64,
    smooth: usize,
}

fn lid_check_game(run: &LidRun, game: &[Point], triangles: &[[u32; 3]]) -> lids::LidCheck {
    let normals = gm::game_normals(game, triangles);
    lids::lid_check(game, &normals, &run.eyes, run.front, run.tolerance)
}

fn lid_check_lods(run: &LidRun, game: &[Point], lods: &[(&[u32], &[[u32; 3]])]) -> lids::LidCheck {
    let mut total = lids::LidCheck { front_deg: run.front, tolerance_mm: run.tolerance * 1000.0, ..Default::default() };
    for (map, triangles) in lods {
        let local: Vec<Point> = map.iter().map(|&j| game[j as usize]).collect();
        let c = lid_check_game(run, &local, triangles);
        total.vertices += c.vertices;
        total.candidates += c.candidates;
        total.inside += c.inside;
        total.deepest_mm = total.deepest_mm.max(c.deepest_mm);
        total.inside_vertices.extend(c.inside_vertices.iter().map(|&i| map[i] as usize));
    }
    total.pass = total.inside == 0;
    total
}

fn lid_touch_up(run: &LidRun, game: &mut [Point], lods: &[(&[u32], &[[u32; 3]])]) -> Value {
    let (mut moved, mut largest, mut rounds) = (0usize, 0.0f64, 0usize);
    for _ in 0..TOUCH_ROUNDS {
        let c = lid_check_lods(run, game, lods);
        if c.pass {
            break;
        }
        rounds += 1;
        let mut vertices = c.inside_vertices;
        vertices.sort_unstable();
        vertices.dedup();
        for g in vertices {
            if let Some(q) = lids::push_out(&run.eyes, game[g], run.front, run.tolerance + TOUCH_MARGIN) {
                let p = game[g];
                largest = largest.max(((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt());
                game[g] = q;
                moved += 1;
            }
        }
    }
    let after = lid_check_lods(run, game, lods);
    json!({"rounds": rounds, "moved": moved, "largest_mm": largest * 1000.0, "inside_after": after.inside, "pass": after.pass})
}

fn settle(run: &LidRun, game: &mut [Point], lods: &[(&[u32], &[[u32; 3]])], untangle: Option<(usize, f64)>) -> Value {
    let mut rounds = Vec::new();
    for _ in 0..SETTLE_ROUNDS {
        let touch = lid_touch_up(run, game, lods);
        let mut changed = touch["moved"].as_u64().unwrap_or(0) > 0;
        let mut row = json!({"lids": touch});
        if let Some((iterations, cap)) = untangle {
            let project = |p: Point| lids::push_out(&run.eyes, p, run.front, run.tolerance + TOUCH_MARGIN).unwrap_or(p);
            let (shift, report) = gm::untangle_projected(game, lods, iterations, cap, &project);
            if report.moved_vertices > 0 {
                changed = true;
                for (p, d) in game.iter_mut().zip(&shift) {
                    for k in 0..3 {
                        p[k] += d[k];
                    }
                }
            }
            row["untangle"] = json!(report);
        }
        rounds.push(row);
        if !changed {
            break;
        }
    }
    let last = lid_check_lods(run, game, lods);
    rounds.push(json!({"final_inside": last.inside}));
    json!(rounds)
}

fn place_clear(
    placement: &dyn Placement,
    run: Option<&LidRun>,
    positions: &[Point],
    dmesh: &DenseMesh,
    base: &Decimated,
    lods: &[(&[u32], &[[u32; 3]])],
) -> (Option<Vec<Point>>, Placed, Value) {
    let placed = placement.place(positions, dmesh, base);
    let Some(run) = run else {
        return (None, placed, Value::Null);
    };
    let first = lid_check_lods(run, &placed.game, lods);
    if first.pass {
        return (None, placed, json!({"inside_before": 0, "collided": false, "pass": true}));
    }
    let mut tried = Vec::new();
    let mut forced: Vec<u32> = first.inside_vertices.iter().map(|&g| base.kept[g]).collect();
    for k in 0.. {
        let clearance = k as f64 * run.step;
        if clearance > run.max + 1e-12 {
            break;
        }
        let settings = lids::CollideSettings { clearance, front_deg: run.front, smooth_iterations: run.smooth };
        let (moved, stats) = lids::collide_dense_forced(positions, dmesh.triangles, &run.eyes, &settings, &forced);
        let p = placement.place(&moved, dmesh, base);
        let c = lid_check_lods(run, &p.game, lods);
        tried.push(json!({"clearance_mm": clearance * 1000.0, "inside": c.inside, "deepest_mm": c.deepest_mm, "forced": forced.len()}));
        if c.pass {
            return (
                Some(moved),
                p,
                json!({"inside_before": first.inside, "deepest_before_mm": first.deepest_mm, "collided": true, "clearance_mm": clearance * 1000.0,
                       "dense_pushed": stats.pushed, "dense_deepest_mm": stats.deepest_mm, "dense_smoothed": stats.smoothed, "tried": tried, "pass": true}),
            );
        }
        for &g in &c.inside_vertices {
            let d = base.kept[g];
            if !forced.contains(&d) {
                forced.push(d);
            }
        }
    }
    let report = json!({"inside_before": first.inside, "deepest_before_mm": first.deepest_mm, "collided": false, "tried": tried, "pass": false,
                        "note": "no clearance up to lids-max-mm passed; the dense shape is left untouched"});
    (None, placed, report)
}

fn lid_final(run: &LidRun, levels: &[Level], neutral: &[Point], states: &[Vec<Point>], names: &[String]) -> (Value, bool) {
    let mut rows = Vec::new();
    let mut pass = true;
    for level in levels {
        let pick = |g: &[Point]| -> Vec<Point> { level.map.iter().map(|&j| g[j as usize]).collect() };
        let n = lid_check_game(run, &pick(neutral), &level.triangles);
        let checks: Vec<lids::LidCheck> = states.iter().map(|g| lid_check_game(run, &pick(g), &level.triangles)).collect();
        let worst = checks.iter().map(|c| c.inside).max().unwrap_or(0);
        let failing: Vec<Value> =
            checks.iter().zip(names).filter(|(c, _)| c.inside > 0).map(|(c, name)| json!({"morph": name, "inside": c.inside, "deepest_mm": c.deepest_mm})).collect();
        pass &= n.inside == 0 && worst == 0;
        rows.push(json!({"node": level.name, "neutral_inside": n.inside, "neutral_deepest_mm": n.deepest_mm, "morph_states_worst": worst,
                         "morph_states_failing": failing}));
    }
    (json!({"lods": rows, "pass": pass}), pass)
}

fn delta(a: Point, b: Point) -> [f32; 3] {
    gm::to_gltf([b[0] - a[0], b[1] - a[1], b[2] - a[2]])
}

fn eye_materials(m: &gf_core::eye_inventory::EyeModel) -> Result<Vec<gltf::Material>, String> {
    crate::eyes::look(m)?
        .into_iter()
        .map(|p| {
            Ok(gltf::Material {
                name: format!("{}_{}", m.id, p.part),
                base_color: p.colour,
                base_texture: p.texture.map(|t| gf_io::png::rgba(t.width, t.height, &t.rgba)).transpose()?,
                normal_texture: None,
                orm_texture: None,
                metallic: 0.0,
                roughness: p.roughness,
                transmission: p.transmission,
                ior: p.ior,
                blend: p.blend,
                alpha_cutoff: None,
                double_sided: false,
                extension: None,
            })
        })
        .collect()
}

fn eye_part_mesh(eye: &lids::Eye, name: &str, joint: Option<usize>, origin: Point, vertices: &[Point]) -> gltf::PartMesh {
    let parts = eye
        .parts
        .iter()
        .filter_map(|p| {
            let material = gf_geom::eyeball::PARTS.iter().position(|n| *n == p.name)?;
            let indices: Vec<u32> = eye.triangles.iter().skip(p.first_triangle).take(p.triangles).flatten().copied().collect();
            Some((indices, material))
        })
        .collect();
    gltf::PartMesh {
        name: name.into(),
        positions: vertices.iter().map(|p| gm::to_gltf([p[0] - origin[0], p[1] - origin[1], p[2] - origin[2]])).collect(),
        normals: eye.vertex_normals().iter().map(|n| gm::to_gltf(*n)).collect(),
        uvs: if eye.uvs.len() == vertices.len() { eye.uvs.iter().map(|u| [u[0] as f32, u[1] as f32]).collect() } else { vec![[0.0, 0.0]; vertices.len()] },
        parts,
        joint,
        translation: [0.0; 3],
    }
}

pub fn export_glb(opts: &Options) -> Result<Value, String> {
    let store = Store::open(Path::new(need(opts, "store")?))?;
    let spec = ch::read_spec(Path::new(need(opts, "spec")?))?;
    let version = PlacementVersion::parse(&spec.base.placement)?;
    let mut allowed: Vec<&[&str]> = vec![GLB_KEYS];
    allowed.extend_from_slice(placement_keys(version.name));
    allowed.push(BODY_KEYS);
    allowed.push(BODY_RIG_KEYS);
    if opts.get("lids").map(String::as_str) == Some("auto") {
        allowed.push(LID_EXPORT_KEYS);
        if !placement_keys(version.name).iter().any(|keys| keys.contains(&"eyes")) {
            allowed.push(EYES_KEY);
        }
    }
    only(opts, &format!("export-glb (spec placement {version})"), &allowed)?;
    let mode = Sparse::parse(opts.get("sparse").map(String::as_str).unwrap_or("auto"))?;
    let out = Path::new(need(opts, "out")?);
    let started = std::time::Instant::now();
    let source = match opts.get("macro") {
        Some(p) => Some(ch::read_macro_definition(Path::new(p))?),
        None => None,
    };
    crate::jobs::progress("evaluate", 0, 1, "spec to dense");
    let (positions, _, evaluation, pin) = ch::evaluate_traced(&store, &spec, source.as_ref())?;
    let base_eyes = match ch::model_eyes(&store, &spec)? {
        Some(eyes) => Some(eyes),
        None if spec.eyeballs.is_some() => opts.get("eyes").map(|p| gf_io::load_eyes(Path::new(p))).transpose()?,
        None => None,
    };
    let placed_eyes = match base_eyes {
        Some(eyes) => Some(ch::place_eyes(&store, &spec, pin.as_ref(), eyes)?),
        None => None,
    };
    let moved_eyes = placed_eyes.as_ref().map(|(e, _)| e.clone());
    let eyes_report = placed_eyes.and_then(|(_, r)| r);
    let eye_model = spec.eyeballs.as_ref().and_then(|e| e.model.as_ref()).and_then(|m| gf_core::eye_inventory::model(&m.id));
    let mut eye_parts: Vec<gltf::PartMesh> = Vec::new();
    let placement = placer_with(opts, version, moved_eyes.clone())?;
    let (_, dense) = store.load_pack(&spec.base.pack)?;
    let catalog = store.load_catalog(&spec.base.catalog)?;
    let game_ref = spec.base.game_base.clone().ok_or("export-glb needs a spec with base.game_base")?;
    let (manifest, base) = store.load_game(&game_ref)?;
    let mut world: Option<gf_core::body_shape::Similarity> = None;
    let mut shaped_report = Value::Null;
    let body = match body_options(opts)? {
        Some(b) => Some(b),
        None => match &spec.body {
            Some(_) => {
                let s = crate::body::spec_body(&store, &spec, source.as_ref(), pin.as_ref())?;
                world = Some(s.similarity);
                shaped_report = s.report;
                Some((s.pack, s.manifest, s.controls))
            }
            None => None,
        },
    };
    let head_scale = body.as_ref().map(|(p, _, c)| (p.pivot(), c.head_scale)).filter(|(_, s)| *s != 1.0);
    let facing = opts.get("facing").map(String::as_str).unwrap_or("keep");
    if !matches!(facing, "keep" | "repair" | "repair-states") {
        return Err(format!("--facing must be keep, repair or repair-states, got {facing}"));
    }
    let lid_run = match opts.get("lids").map(String::as_str).unwrap_or("off") {
        "off" => None,
        "auto" => {
            let eyes = match &moved_eyes {
                Some(e) => e.clone(),
                None => gf_io::load_eyes(Path::new(opts.get("eyes").ok_or("--lids auto needs --eyes EYES.json")?))?,
            };
            let step = number(opts, "lids-step-mm", 0.05)? / 1000.0;
            if step <= 0.0 {
                return Err("--lids-step-mm must be positive".into());
            }
            let front = number(opts, "lids-front-deg", LID_FRONT_DEG)?;
            if front > 90.0 {
                return Err(format!("--lids-front-deg must be at most 90, got {front}"));
            }
            Some(LidRun {
                eyes,
                step,
                max: number(opts, "lids-max-mm", 1.0)? / 1000.0,
                front,
                tolerance: number(opts, "lids-tolerance-mm", 0.0)? / 1000.0,
                smooth: count(opts, "lids-smooth", 4)?,
            })
        }
        other => return Err(format!("--lids must be off or auto, got {other}")),
    };
    let lod0_out = opts.get("lod0-out").map(Path::new);
    let (rigged, face_mode) = match opts.get("rig").map(String::as_str).unwrap_or("none") {
        "none" => (false, false),
        "head" => (true, false),
        "face" | "face2" => (true, true),
        other => return Err(format!("--rig must be none, head, face or face2, got {other}")),
    };
    let face_version = if opts.get("rig").map(String::as_str) == Some("face2") { 2 } else { 1 };
    let body_rig = match (opts.get("body-rig"), opts.get("body-weights")) {
        (None, None) => None,
        (None, Some(_)) => return Err("--body-weights goes with --body-rig".into()),
        (Some(r), w) => {
            if rigged {
                return Err("--body-rig cannot be combined with --rig yet: the head and face rig on the body's head bone is the next step".into());
            }
            let (pack, _, controls) = body.as_ref().filter(|_| world.is_some()).ok_or("--body-rig needs the spec's own body (a makehuman-mode body in the spec), not --body")?;
            let (rig_json, weights_json, source) = crate::body::rig_source(r, w.map(String::as_str))?;
            Some(crate::body::body_rig(pack, controls, &rig_json, &weights_json, source)?)
        }
    };
    let head_influences: Option<Vec<gf_core::body_rig::Influences>> = match (&body_rig, &body) {
        (Some(br), Some((pack, _, _))) => {
            let quads = store.load_pack_quads(&spec.base.pack)?;
            let points = gf_core::cage::cage_points(&quads, dense.positions.len(), 256)?;
            let none: Vec<(usize, f64)> = Vec::new();
            let mut out = Vec::with_capacity(base.kept.len());
            for &d in &base.kept {
                let terms = points.get(d as usize).ok_or_else(|| format!("game vertex on dense vertex {d}, outside the dense base"))?.terms();
                let mut parts: Vec<(f64, &[(usize, f64)])> = Vec::with_capacity(4);
                for (c, k) in terms {
                    let o = pack.cage_source.get(c as usize).ok_or_else(|| format!("cage corner {c} is outside the body's cage map"))?;
                    parts.push((k, br.by_obj.get(o).map(Vec::as_slice).unwrap_or(&none)));
                }
                out.push(gf_core::body_rig::strongest(&gf_core::body_rig::mix(&parts), 4));
            }
            Some(out)
        }
        _ => None,
    };
    let body_gltf_rig = body_rig.as_ref().map(|br| gltf::Rig {
        names: br.rig.bones.iter().map(|b| b.name.clone()).collect(),
        parents: br.rig.bones.iter().map(|b| b.parent.as_ref().and_then(|p| br.rig.bones.iter().position(|q| &q.name == p))).collect(),
        global: br.heads.iter().map(|h| gm::to_gltf(*h)).collect(),
        attachments: Vec::new(),
        animations: Vec::new(),
    });
    let correctives = match opts.get("rig-correctives").map(String::as_str) {
        None | Some("off") => false,
        Some("on") if face_version == 2 => true,
        Some("on") => return Err("--rig-correctives goes with --rig face2".into()),
        Some(other) => return Err(format!("--rig-correctives must be on or off, got {other}")),
    };
    if !face_mode && opts.contains_key("rig-rounds") {
        return Err("--rig-rounds goes with --rig face".into());
    }
    let rig_rounds = count(opts, "rig-rounds", 2)?;
    let morph_dir = opts.get("morph-dir").map(Path::new);
    let floor_m = if opts.contains_key("morph-floor") { Some(number(opts, "morph-floor", 0.0)? / 1000.0) } else { None };
    let measured = opts.get("lod-screen").map(String::as_str) == Some("measured");
    let lod_pixels = number(opts, "lod-pixels", 1.0)?;
    let lod_height = number(opts, "lod-height", 1080.0)?;
    if !measured && (opts.contains_key("lod-pixels") || opts.contains_key("lod-height")) {
        return Err("--lod-pixels and --lod-height go with --lod-screen measured".into());
    }
    if measured && (lod_pixels <= 0.0 || lod_height <= 0.0) {
        return Err("--lod-pixels and --lod-height must be positive".into());
    }
    let extended = facing != "keep" || rigged || measured || morph_dir.is_some() || floor_m.is_some() || lid_run.is_some();
    let load = |id: &str| -> Result<gf_core::catalog::SparseTarget, String> {
        if let Some(dir) = morph_dir {
            let path = dir.join(format!("{id}.npz"));
            if path.exists() {
                let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let target = gf_io::store::sparse_from_npz(&bytes).map_err(|e| format!("{id}: {e}"))?;
                target.check(positions.len()).map_err(|e| format!("{id}: {e}"))?;
                return Ok(target);
            }
        }
        store.load_target(&catalog, id)
    };
    let screens: Vec<f64> = match opts.get("lod-screen") {
        _ if measured => Vec::new(),
        Some(text) => text.split(',').map(|s| s.trim().parse::<f64>().map_err(|_| format!("--lod-screen takes numbers, got {text}"))).collect::<Result<_, _>>()?,
        None => (0..manifest.lods.len()).map(|i| [0.25, 0.1, 0.04].get(i).copied().unwrap_or_else(|| 0.04 * 0.4f64.powi(i as i32 - 2))).collect(),
    };
    if !measured && (screens.len() != manifest.lods.len() || screens.iter().any(|s| !s.is_finite() || *s <= 0.0 || *s >= 1.0)) {
        return Err(format!("--lod-screen needs {} numbers in (0, 1), one per LOD below the finest", manifest.lods.len()));
    }
    let ids = match opts.get("morphs") {
        Some(text) => list_arg(text)?,
        None => Vec::new(),
    };
    let mut names: Vec<String> = Vec::new();
    for id in &ids {
        let name = id.rsplit('/').next().unwrap_or(id).to_string();
        if names.contains(&name) {
            return Err(format!("two morphs would both be named {name}; give targets with distinct last path parts"));
        }
        names.push(name);
    }
    let dmesh = DenseMesh { triangles: &dense.triangles, triangle_uvs: &dense.triangle_uvs };
    let mut coarse = Vec::new();
    for lod in &manifest.lods {
        coarse.push(store.load_game(&lod.game)?.1);
    }
    let mut lid_lods: Vec<(Vec<u32>, Vec<[u32; 3]>)> = vec![((0..base.kept.len() as u32).collect(), base.triangles.clone())];
    for mesh in &coarse {
        lid_lods.push((gm::lod_map(&base, mesh)?, mesh.triangles.clone()));
    }
    let lid_refs: Vec<(&[u32], &[[u32; 3]])> = lid_lods.iter().map(|(m, t)| (m.as_slice(), t.as_slice())).collect();
    crate::jobs::progress("place", 0, ids.len() as u64 + 1, "neutral");
    let (collided, mut neutral, neutral_lids) = place_clear(placement.as_ref(), lid_run.as_ref(), &positions, &dmesh, &base, &lid_refs);
    let neutral_dense = collided.unwrap_or_else(|| positions.clone());
    let uvs0 = neutral.triangle_uvs.clone().unwrap_or_else(|| base.triangle_uvs.clone());
    let mut morph_games = Vec::new();
    let mut slips = Vec::new();
    let mut morph_lids = Vec::new();
    for (k, id) in ids.iter().enumerate() {
        crate::jobs::progress("place", k as u64 + 1, ids.len() as u64 + 1, id);
        let target = load(id)?;
        let mut moved = neutral_dense.clone();
        target.add_scaled(1.0, &mut moved);
        let (_, placed, lid_report) = place_clear(placement.as_ref(), lid_run.as_ref(), &moved, &dmesh, &base, &lid_refs);
        morph_lids.push(lid_report);
        if extended {
            slips.push(match (&neutral.triangle_uvs, &placed.triangle_uvs) {
                (Some(a), Some(b)) => uv_slip(a, b),
                _ => json!({"corners": 0, "note": "the placement keeps the base UVs: no slip"}),
            });
        }
        morph_games.push(placed.game);
    }
    if let Some((pivot, s)) = head_scale {
        gf_core::body::scale_about(&mut neutral.game, pivot, s);
        for mg in morph_games.iter_mut() {
            gf_core::body::scale_about(mg, pivot, s);
        }
    }
    let mut levels = vec![Level {
        name: "Head_LOD0".into(),
        map: (0..base.kept.len() as u32).collect(),
        uvs: uvs0.clone(),
        mesh: &base,
        triangles: base.triangles.clone(),
    }];
    let mut lod_uv_report = Vec::new();
    for (i, mesh) in coarse.iter().enumerate() {
        let map = gm::lod_map(&base, mesh)?;
        let (uvs, unmatched, worst) = gm::lod_uvs(&base, &uvs0, mesh, &map);
        lod_uv_report.push(json!({"lod": i + 1, "corners_without_a_matching_chart": unmatched, "worst_chart_distance": worst}));
        levels.push(Level { name: format!("Head_LOD{}", i + 1), mesh, map, uvs, triangles: mesh.triangles.clone() });
    }
    let mut facing_report = Vec::new();
    if facing != "keep" {
        crate::jobs::progress("facing", 0, 1, facing);
        for level in levels.iter_mut() {
            let pick = |g: &[Point]| -> Vec<Point> { level.map.iter().map(|&j| g[j as usize]).collect() };
            let states: Vec<Vec<Point>> = std::iter::once(&neutral.game).chain(&morph_games).map(|g| pick(g)).collect();
            let refs: Vec<&[Point]> = states.iter().map(Vec::as_slice).collect();
            let repair = gm::repair_facing(&refs, &mut level.triangles, &mut level.uvs, FACING_ROUNDS);
            facing_report.push(json!({"node": level.name, "against_neutral_before": repair.against_before[0],
                                      "against_neutral_after": repair.against_after[0],
                                      "against_morph_states_before_worst": repair.against_before[1..].iter().max().copied().unwrap_or(0),
                                      "against_morph_states_after_worst": repair.against_after[1..].iter().max().copied().unwrap_or(0),
                                      "flips": repair.flips, "flips_relaxed": repair.flips_relaxed, "rounds": repair.rounds,
                                      "refused_seam": repair.refused_seam, "refused_check": repair.refused_check,
                                      "refused_neutral": repair.refused_neutral, "refused_states": repair.refused_states,
                                      "refused_angle": repair.refused_angle}));
        }
        let lod_refs: Vec<(&[u32], &[[u32; 3]])> = levels.iter().map(|l| (l.map.as_slice(), l.triangles.as_slice())).collect();
        let (shift, untangled) = gm::untangle(&neutral.game, &lod_refs, UNTANGLE_ITERATIONS, UNTANGLE_MAX_MM / 1000.0);
        for game in std::iter::once(&mut neutral.game).chain(morph_games.iter_mut()) {
            for (p, d) in game.iter_mut().zip(&shift) {
                for k in 0..3 {
                    p[k] += d[k];
                }
            }
        }
        facing_report.push(json!({"untangle": untangled, "note": "positions of the vertices of triangles still facing against their vertex normals after the flips move toward their one-ring centroids in every LOD that has them; every morph state moves by the same amount, so morph deltas are unchanged"}));
        if facing == "repair-states" {
            let mut rows = Vec::new();
            let mut moved_states = 0usize;
            for (game, name) in morph_games.iter_mut().zip(&names) {
                let (shift, report) = gm::untangle(game, &lod_refs, STATE_UNTANGLE_ITERATIONS, STATE_UNTANGLE_MAX_MM / 1000.0);
                if report.moved_vertices == 0 {
                    continue;
                }
                moved_states += 1;
                for (p, d) in game.iter_mut().zip(&shift) {
                    for k in 0..3 {
                        p[k] += d[k];
                    }
                }
                rows.push(json!({"morph": name, "against_before": report.against_before, "against_after": report.against_after,
                                 "moved_vertices": report.moved_vertices, "max_move_mm": report.max_move_mm, "capped_vertices": report.capped_vertices,
                                 "iterations": report.iterations}));
            }
            facing_report.push(json!({"morph_states_untangle": rows, "morph_states_moved": moved_states,
                                      "note": "each morph state at weight 1 whose triangles still face against its own vertex normals is untangled alone (same step, cap 4 mm); the neutral is not touched, so that morph's delta is no longer exactly F(c + e) - F(c) at the moved vertices"}));
        }
    }
    let mut lid_touch = Value::Null;
    if let Some(run) = &lid_run {
        crate::jobs::progress("lids", 0, 1, "");
        let lods: Vec<(&[u32], &[[u32; 3]])> = levels.iter().map(|l| (l.map.as_slice(), l.triangles.as_slice())).collect();
        let neutral_untangle = (facing != "keep").then_some((UNTANGLE_ITERATIONS, UNTANGLE_MAX_MM / 1000.0));
        let state_untangle = (facing == "repair-states").then_some((STATE_UNTANGLE_ITERATIONS, STATE_UNTANGLE_MAX_MM / 1000.0));
        let before = neutral.game.clone();
        let neutral_touch = settle(run, &mut neutral.game, &lods, neutral_untangle);
        let shift: Vec<(usize, Point)> = before
            .iter()
            .zip(&neutral.game)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(g, (a, b))| (g, [b[0] - a[0], b[1] - a[1], b[2] - a[2]]))
            .collect();
        let mut rows = Vec::new();
        for (game, name) in morph_games.iter_mut().zip(&names) {
            for &(g, d) in &shift {
                for k in 0..3 {
                    game[g][k] += d[k];
                }
            }
            let touch = settle(run, game, &lods, state_untangle);
            if touch.as_array().is_some_and(|r| r.iter().any(|x| x["lids"]["moved"].as_u64().unwrap_or(0) > 0 || x["lids"]["pass"] != json!(true))) {
                rows.push(json!({"morph": name, "rounds": touch}));
            }
        }
        lid_touch = json!({"neutral": neutral_touch, "morph_states": rows,
                           "note": "after the facing repair, a vertex the check still finds inside an eye on any LOD is moved along the eye's radius to its surface plus tolerance + 0.01 mm (game level, up to 4 passes); with a facing repair the untangle runs again after it, alternating up to 4 rounds until neither moves a vertex; the neutral's moves are applied to every morph state too"});
    }
    let lid_gate = lid_run.as_ref().map(|run| lid_final(run, &levels, &neutral.game, &morph_games, &names));
    let mut screens = screens;
    let mut lod_selection = Value::Null;
    if measured {
        let fine = &neutral.game;
        let lo = (0..3).map(|k| fine.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min)).collect::<Vec<_>>();
        let hi = (0..3).map(|k| fine.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max)).collect::<Vec<_>>();
        let centre = [0, 1, 2].map(|k| 0.5 * (lo[k] + hi[k]));
        let radius = fine.iter().map(|p| (0..3).map(|k| (p[k] - centre[k]).powi(2)).sum::<f64>().sqrt()).fold(0.0, f64::max);
        let mut rows = Vec::new();
        let mut previous = 1.0f64;
        for level in levels.iter().skip(1) {
            let coarse_points: Vec<Point> = level.map.iter().map(|&j| fine[j as usize]).collect();
            let owner: Vec<u32> = base.kept.iter().map(|&d| level.mesh.owner[d as usize]).collect();
            let error = gm::lod_error(fine, &owner, &coarse_points, &level.triangles);
            let raw = gm::screen_size(error.max_mm / 1000.0, radius, lod_pixels, lod_height);
            let chosen = raw.min(previous * 0.999).clamp(1e-4, 0.999);
            previous = chosen;
            screens.push(chosen);
            rows.push(json!({"node": level.name, "error_to_lod0": error, "screen_size_raw": raw, "screen_size": chosen}));
        }
        lod_selection = json!({
            "method": "switch to a coarser LOD when its largest distance to the finest LOD's vertices projects to at most `pixels` on a screen `screen_height_px` tall",
            "screen_size": "the bounding sphere's diameter as a fraction of the screen height: s = 2 * pixels * radius / (error * screen_height_px); distance d = radius / (s * tan(vfov / 2))",
            "pixels": lod_pixels, "screen_height_px": lod_height, "bounding_radius_m": radius,
            "bounding_centre_gltf_m": gm::to_gltf(centre), "lods": rows});
    }
    let mut rig_meta = Value::Null;
    let mut rig_data: Option<(gltf::Rig, Vec<[f64; 3]>)> = None;
    let mut face_meta = Value::Null;
    let mut face_skin: Option<gf_core::face_rig::Influences> = None;
    let mut face2_correctives: Option<(Vec<Vec<gf_core::face_rig::JointPose>>, Vec<Point>)> = None;
    if rigged {
        crate::jobs::progress("rig", 0, 1, "");
        use gf_core::rig;
        let mut eyes = match &moved_eyes {
            Some(e) => e.clone(),
            None => gf_io::load_eyes(Path::new(opts.get("eyes").ok_or("--rig head needs --eyes EYES.json")?))?,
        };
        if let Some((pivot, s)) = head_scale {
            for e in eyes.iter_mut() {
                let mut c = [e.centre];
                gf_core::body::scale_about(&mut c, pivot, s);
                e.centre = c[0];
                e.radius *= s;
                if spec.eyeballs.is_some() {
                    gf_core::body::scale_about(&mut e.vertices, pivot, s);
                }
            }
        }
        if eyes.len() != 2 {
            return Err(format!("--rig head needs two eyes, the eyes file has {}", eyes.len()));
        }
        let jaw = load(opts.get("rig-jaw").ok_or("--rig head needs --rig-jaw TARGET (a jaw-open target)")?)?;
        let hinge = rig::fit_hinge(&dense.positions, &jaw)?;
        let share = rig::jaw_share(&dense.positions, &jaw, &hinge);
        let seam = neck_loop(&base.triangles, &neutral.game)?;
        let (weights, skin_report) = rig::game_weights(&dense.positions, &base.kept, &seam, &share, &hinge);
        let neck = mean_point(seam.iter().map(|&g| neutral.game[g as usize]));
        let mut hinge_now = [rig::character_hinge(&dense.positions, &positions, &hinge)];
        if let Some((pivot, s)) = head_scale {
            gf_core::body::scale_about(&mut hinge_now, pivot, s);
        }
        let hinge_now = hinge_now[0];
        let (left, right) = if eyes[0].centre[0] >= eyes[1].centre[0] { (&eyes[0], &eyes[1]) } else { (&eyes[1], &eyes[0]) };
        let joints = rig::joints(neck, hinge_now, left.centre, right.centre);
        if eye_model.is_some() {
            for (name, joint, eye) in [("Eye_L", 3usize, left), ("Eye_R", 4usize, right)] {
                eye_parts.push(eye_part_mesh(eye, name, Some(joint), eye.centre, &eye.vertices));
            }
        }
        let attachments = [("Eye_L", 3usize, left), ("Eye_R", 4usize, right)]
            .iter()
            .filter(|_| eye_model.is_none())
            .map(|&(name, joint, eye)| {
                if spec.eyeballs.is_some() {
                    let c = eye.centre;
                    return gltf::Attachment {
                        name: name.into(),
                        joint,
                        positions: eye.vertices.iter().map(|p| gm::to_gltf([p[0] - c[0], p[1] - c[1], p[2] - c[2]])).collect(),
                        normals: eye.vertex_normals().iter().map(|n| gm::to_gltf(*n)).collect(),
                        indices: eye.triangles.iter().flatten().copied().collect(),
                    };
                }
                let s = rig::sphere(eye.radius, EYE_RINGS, EYE_SEGMENTS);
                gltf::Attachment {
                    name: name.into(),
                    joint,
                    positions: s.positions.iter().map(|p| gm::to_gltf(*p)).collect(),
                    normals: s.normals.iter().map(|p| gm::to_gltf(*p)).collect(),
                    indices: s.indices,
                }
            })
            .collect();
        let mut rig_value = gltf::Rig {
            names: rig::JOINTS.iter().map(|s| s.to_string()).collect(),
            parents: rig::PARENTS.to_vec(),
            global: joints.iter().map(|p| gm::to_gltf(*p)).collect(),
            attachments,
            animations: Vec::new(),
        };
        if face_mode && face_version == 2 {
            use gf_core::face_rig as fr;
            use gf_core::face_rig2 as fr2;
            if ids.is_empty() {
                return Err("--rig face2 needs --morphs (the shapes the joint poses are fitted to)".into());
            }
            let fit_started = std::time::Instant::now();
            let targets: Vec<gf_core::catalog::SparseTarget> = ids.iter().map(|id| load(id)).collect::<Result<_, _>>()?;
            let base_rig = fr2::base_rig(&dense.positions, &base.kept, &base.triangles, &jaw, &targets, rig_rounds)?;
            let pivots = fr2::character_pivots([neck, hinge_now, left.centre, right.centre], &base_rig.regions, &positions);
            let deltas: Vec<Vec<Point>> = morph_games.iter().map(|mg| mg.iter().zip(&neutral.game).map(|(a, b)| [0, 1, 2].map(|k| a[k] - b[k])).collect()).collect();
            let emphasis = fr::lid_emphasis(&neutral.game, &[(left.centre, left.radius), (right.centre, right.radius)]);
            let problem = fr::Problem { rest: &neutral.game, influences: &base_rig.influences, pivots: &pivots, emphasis: Some(&emphasis) };
            let bounds = fr2::bounds();
            let bar = fr2::barrier(&neutral.game, &eyes, LID_FRONT_DEG);
            let fits = fr2::fit_shapes(&problem, &deltas, &bounds, Some(&bar));
            rig_value.names = fr2::names();
            rig_value.parents = fr2::parents();
            rig_value.global = pivots.iter().map(|p| gm::to_gltf(*p)).collect();
            rig_value.animations = super::rig::face2_animations(&names, &fits, &pivots);
            let parents = fr2::parents();
            let all_names = fr2::names();
            let spec = fr2::joints();
            let joint_rows: Vec<Value> = (0..fr2::count())
                .map(|j| {
                    let mut row = json!({"name": all_names[j], "parent": parents[j].map(|p| all_names[p].clone()), "position_gltf_m": rig_value.global[j],
                                         "posed": fr::is_free(j), "bound_deg_mm": [bounds[j].0.to_degrees(), bounds[j].1 * 1000.0]});
                    if let Some(f) = j.checked_sub(rig::JOINTS.len()).and_then(|f| spec.get(f)) {
                        row["kind"] = json!(f.kind);
                        if let fr2::Kind::Seed(s) = f.kind {
                            row["seed_dense_vertex"] = json!(fr::FACE[s].seed);
                            row["seed_rule"] = json!(fr::FACE[s].rule);
                            row["falloff_mm"] = json!(fr::FACE[s].radius_mm);
                        }
                    }
                    row
                })
                .collect();
            face_meta = json!({
                "version": fr2::VERSION, "joints": joint_rows, "weights": base_rig.report, "lid_frames": base_rig.lids, "lid_vertices": base_rig.lid_vertices,
                "fit": super::rig::fit_report(&names, &fits), 
                "correctives": if correctives { json!(format!("one corrective morph target per shape, named <shape>.corrective, on every LOD: per vertex the rest-space delta c with sum_j w_j (R_j (x + c - p_j) + p_j + t_j) = the blend shape position at weight 1 (and the normal likewise); each animation also drives its corrective weight 0 -> 1 through a glTF weights channel on every head LOD node, so joints plus corrective reproduce the blend shape; vertices whose blended rotation has |det| < {} take the plain difference", fr2::SINGULAR_DET)) } else { Value::Null },
                "seconds": fit_started.elapsed().as_secs_f64(),
                "pivots": format!("each seeded facial joint sits at the mean of the character's dense positions over the base vertices within {} mm of its seed vertex; the 12 lid joints (3 along each lid margin) sit at the eye centres; every joint position is rounded to 1/{} m", fr::REGION_MM, fr::PIVOT_GRID),
                "poses": format!("per shape, every posed joint gets a rotation about its rest position and a translation, solved by Gauss-Newton on the game LOD0 against F(c + e) - F(c) with the exported weights, bounded per joint (bound_deg_mm); vertices within {} eye radii of an eye centre count {} times, and every vertex outside an eye at rest within {} eye radii of its centre is pushed out of the eye surface (+{} mm) by a barrier term", fr::LID_EMPHASIS_REACH, fr::LID_EMPHASIS, fr2::BARRIER_REACH, fr2::BARRIER_MARGIN_MM),
                "influences": format!("up to {} per vertex: JOINTS_0 / WEIGHTS_0 hold the first 4, JOINTS_1 / WEIGHTS_1 the rest", fr2::MAX_INFLUENCES),
                "animations": "one glTF animation per shape, named as the morph, with LINEAR keys at t = 0 (rest) and t = 1 (the fitted pose) for the posed joints' rotation and translation; neck, head and the eyes are not animated"});
            if correctives {
                face2_correctives = Some((fits.iter().map(|f| f.0.clone()).collect::<Vec<_>>(), pivots.clone()));
                for (k, a) in rig_value.animations.iter_mut().enumerate() {
                    a.morph = Some(names.len() + k);
                }
            }
            face_skin = Some(base_rig.influences);
        } else if face_mode {
            use gf_core::face_rig as fr;
            if ids.is_empty() {
                return Err("--rig face needs --morphs (the shapes the joint poses are fitted to)".into());
            }
            let fit_started = std::time::Instant::now();
            let targets: Vec<gf_core::catalog::SparseTarget> = ids.iter().map(|id| load(id)).collect::<Result<_, _>>()?;
            let base_rig = fr::base_rig(&dense.positions, &base.kept, &base.triangles, &jaw, &targets, rig_rounds)?;
            let pivots = fr::character_pivots([neck, hinge_now, left.centre, right.centre], &base_rig.regions, &positions);
            let deltas: Vec<Vec<Point>> = morph_games.iter().map(|mg| mg.iter().zip(&neutral.game).map(|(a, b)| [0, 1, 2].map(|k| a[k] - b[k])).collect()).collect();
            let emphasis = fr::lid_emphasis(&neutral.game, &[(left.centre, left.radius), (right.centre, right.radius)]);
            let problem = fr::Problem { rest: &neutral.game, influences: &base_rig.influences, pivots: &pivots, emphasis: Some(&emphasis) };
            let fits = fr::fit_shapes(&problem, &deltas);
            rig_value.names = fr::names();
            rig_value.parents = fr::parents();
            rig_value.global = pivots.iter().map(|p| gm::to_gltf(*p)).collect();
            rig_value.animations = super::rig::face_animations(&names, &fits, &pivots);
            let parents = fr::parents();
            let all_names = fr::names();
            let joint_rows: Vec<Value> = (0..fr::count())
                .map(|j| {
                    let mut row = json!({"name": all_names[j], "parent": parents[j].map(|p| all_names[p].clone()), "position_gltf_m": rig_value.global[j],
                                         "posed": fr::is_free(j)});
                    if let Some(f) = j.checked_sub(rig::JOINTS.len()).and_then(|f| fr::FACE.get(f)) {
                        row["seed_dense_vertex"] = json!(f.seed);
                        row["seed_rule"] = json!(f.rule);
                        row["falloff_mm"] = json!(f.radius_mm);
                        row["region_vertices"] = json!(base_rig.regions[j - rig::JOINTS.len()].len());
                    }
                    row
                })
                .collect();
            face_meta = json!({
                "joints": joint_rows, "weights": base_rig.report, "fit": super::rig::fit_report(&names, &fits),
                "seconds": fit_started.elapsed().as_secs_f64(),
                "pivots": format!("each facial joint sits at the mean of the character's dense positions over the base vertices within {} mm of its seed vertex; every joint position is rounded to 1/{} m so that the f32 joint chain and the inverse bind matrices cancel exactly", fr::REGION_MM, fr::PIVOT_GRID),
                "poses": "per shape, every posed joint (jaw and the facial joints) gets a rotation about its rest position and a translation, solved by Gauss-Newton on the game LOD0 against F(c + e) - F(c) with the exported weights, bounded to 35 deg / 15 mm (jaw), 60 deg / 6 mm (lids, which turn about the eye centre) and 30 deg / 12 mm (other facial joints); vertices within 1.4 eye radii of an eye centre count 30 times in the fit",
                "animations": "one glTF animation per shape, named as the morph, with LINEAR keys at t = 0 (rest) and t = 1 (the fitted pose) for the posed joints' rotation and translation; neck, head and the eyes are not animated"});
            face_skin = Some(base_rig.influences);
        }
        rig_meta = json!({
            "joints": rig::JOINTS.iter().zip(rig::PARENTS).zip(&rig_value.global)
                .map(|((n, p), g)| json!({"name": n, "parent": p.map(|i| rig::JOINTS[i]), "position_gltf_m": g})).collect::<Vec<_>>(),
            "jaw_target": opts.get("rig-jaw"), "hinge_fit_on_base": hinge, "skin": skin_report,
            "weights": "per game vertex from the dense base: jaw = the jaw target's displacement projected on the fitted hinge rotation (clamped 0..1), neck = 1 on the neck seam falling (smoothstep) to 0 at half the chin-to-seam distance, head = the rest",
            "eyes": "Eye_L / Eye_R spheres (radius from the eyes file) are children of eye_L / eye_R",
            "bind": "joints are translation-only; each inverse bind matrix is the negated sum of the joint chain's local translations"});
        if face_mode {
            rig_meta["mode"] = json!(if face_version == 2 { "face2" } else { "face" });
            rig_meta["weights"] = json!("face: see face.weights.method; the neck / head / jaw split the facial weights start from is the head rig's");
            rig_meta["face"] = face_meta.clone();
        }
        rig_data = Some((rig_value, weights));
    }
    let mut body_lods = Vec::new();
    let mut seam_normals: Vec<HashMap<u32, Point>> = Vec::new();
    let mut body_meta = Value::Null;
    let genitals = spec.body.as_ref().is_some_and(|b| b.canonical().genitals == "male");
    let mut genital_rows: Vec<Value> = Vec::new();
    let mut wear_slots: Vec<(usize, usize)> = Vec::new();
    let mut wear_materials: Vec<gltf::Material> = Vec::new();
    if let Some((pack, pack_manifest, controls)) = &body {
        crate::jobs::progress("body", 0, 1, "");
        use gf_core::body as gb;
        let limits = if pack.mode == "makehuman" { gb::Limits::exact() } else { gb::Limits::any_body() };
        let worn_refs = spec.body.as_ref().map(|b| b.wearables.clone()).unwrap_or_default();
        if !worn_refs.is_empty() && world.is_none() {
            return Err("body.wearables need the spec's own body (a makehuman-mode body in the spec), not --body".into());
        }
        let frame = super::wearable::BaseFrame::new(pack, controls, pack_manifest, &positions);
        let mut pending: Vec<(gf_io::wearable_store::Wearable, Vec<Point>, usize, Value)> = Vec::new();
        let mut worn: Vec<(gf_io::wearable_store::Wearable, Vec<Point>, usize, Vec<GarmentLevel>, Value)> = Vec::new();
        let body_counts: Vec<usize> = (0..levels.len()).map(|i| pack.lod_mesh(i.min(pack.lod_count() - 1)).map(|m| m.1.len())).collect::<Result<_, _>>()?;
        let mut covered: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for w in &worn_refs {
            let item = store.load_wearable(&w.asset)?;
            let garment = frame.evaluate(&item.binding).map_err(|e| format!("wearable {}: {e}", w.asset))?;
            if w.hide_body {
                covered.extend(item.binding.hidden.iter().copied());
            }
            let slot = wear_materials.len();
            wear_materials.push(wearable_material(&store, &item)?);
            let row = json!({"asset": w.asset, "name": item.manifest.name, "hide_body": w.hide_body, "vertices": garment.len(), "hides_base_vertices": item.binding.hidden.len(), "material": item.manifest.material.keys().collect::<Vec<_>>()});
            pending.push((item, garment, slot, row));
        }
        let mut rows = Vec::new();
        let mut failures = Vec::new();
        for (i, level) in levels.iter().enumerate() {
            let (game, _) = lod_state(level, &neutral.game);
            let dense_of: Vec<u32> = level.map.iter().map(|&g| base.kept[g as usize]).collect();
            let seam = gb::head_seam(pack, &game, &level.triangles, &dense_of)?;
            let body_lod = i.min(pack.lod_count() - 1);
            let st = gb::stitch_lod(pack, body_lod, &seam, controls)?;
            let (report, failed) = gb::gates(&game, &level.triangles, &seam, &st, &gb::Limits { relative_dihedral: i > 0, ..limits });
            failures.extend(failed.into_iter().map(|f| format!("{}: {f}", level.name)));
            let (shared, normals) = gb::two_sided_normals(&game, &level.triangles, &st);
            seam_normals.push(shared.into_iter().collect());
            if i == 0 && !pending.is_empty() {
                let mut points = game.clone();
                points.extend_from_slice(&st.positions);
                let h = game.len() as u32;
                let tris: Vec<[u32; 3]> = level.triangles.iter().copied().chain(st.triangles.iter().map(|t| t.map(|v| v + h))).collect();
                let surface = super::wearable::Surface::new(points, &tris);
                let mut dressed = std::mem::take(&mut pending);
                for (item, garment, _, row) in dressed.iter_mut() {
                    row["pushed"] = if item.manifest.transparent { json!("no: alpha cards cross the skin on purpose") } else { super::wearable::push_out(garment, &super::wearable::triangles_of(&item.faces), &surface, super::wearable::CLEARANCE_M) };
                }
                let reports = {
                    let mut layers: Vec<super::wearable::Layered> = dressed
                        .iter_mut()
                        .map(|(item, garment, _, _)| super::wearable::Layered { triangles: super::wearable::triangles_of(&item.faces), depth: super::wearable::depth_of(&item.binding), opaque: !item.manifest.transparent, positions: garment })
                        .collect();
                    super::wearable::layer(&mut layers)
                };
                for ((item, garment, slot, mut row), layered) in dressed.into_iter().zip(reports) {
                    row["layered"] = layered;
                    let ladder = wearable_ladder(&item, &garment, &body_counts);
                    row["lod_vertices"] = json!(ladder.iter().map(|l| l.0.len()).collect::<Vec<_>>());
                    row["lod_pushed"] = json!([]);
                    worn.push((item, garment, slot, ladder, row));
                }
            }
            let (body_triangles, body_uvs): (Vec<[u32; 3]>, Vec<[gf_geom::Uv; 3]>) = if covered.is_empty() {
                (st.triangles.clone(), st.uvs.clone())
            } else {
                let hidden = super::wearable::hidden_body(pack, body_lod, &st, &covered)?;
                st.triangles.iter().zip(&st.uvs).filter(|(t, _)| !t.iter().all(|&v| hidden[v as usize])).map(|(t, u)| (*t, *u)).unzip()
            };
            let surface = gm::split_seams(&body_triangles, &body_uvs);
            let name = format!("Body_LOD{i}");
            let body_skin = match (&body_rig, &head_influences) {
                (Some(br), Some(hi)) => {
                    let (_, kept) = pack.lod_mesh(body_lod)?;
                    let copies: HashMap<u32, u32> = st.seam_copies.iter().copied().collect();
                    let mut infl: Vec<gf_core::body_rig::Influences> = Vec::with_capacity(surface.source.len());
                    for &v in &surface.source {
                        let x = match (st.source[v as usize], copies.get(&v)) {
                            (Some(u), _) => {
                                let k = *kept.get(u as usize).ok_or_else(|| format!("{name}: body vertex {u} is outside its LOD"))?;
                                let none: Vec<(usize, f64)> = Vec::new();
                                let of_cage = |c: u32| br.by_obj.get(&pack.source[c as usize]).map(Vec::as_slice).unwrap_or(&none);
                                match &pack.fine {
                                    Some(f) => {
                                        let parts: Vec<(f64, &[(usize, f64)])> = f.stencil.rows[k as usize].iter().map(|&(c, w)| (w, of_cage(c))).collect();
                                        gf_core::body_rig::strongest(&gf_core::body_rig::mix(&parts), 4)
                                    }
                                    None => gf_core::body_rig::strongest(of_cage(k), 4),
                                }
                            }
                            (None, Some(&h)) => hi[level.map[h as usize] as usize].clone(),
                            (None, None) => return Err(format!("{name}: body vertex {v} is neither the body's nor a seam copy")),
                        };
                        infl.push(if x.is_empty() { vec![(0, 1.0)] } else { x });
                    }
                    Some(skin_data(&infl.iter().map(Vec::as_slice).collect::<Vec<_>>()))
                }
                _ => None,
            };
            rows.push(json!({"node": name, "body_lod": body_lod, "head_node": level.name, "vertices": surface.source.len(),
                             "triangles": st.triangles.len(), "seam_points": seam.len(), "gates": report}));
            body_lods.push(Lod {
                name,
                positions: surface.source.iter().map(|&v| gm::to_gltf(st.positions[v as usize])).collect(),
                normals: surface.source.iter().map(|&v| gm::to_gltf(normals[v as usize])).collect(),
                uvs: surface.uvs.clone(),
                indices: surface.indices,
                morphs: Vec::new(),
                skin: body_skin.or_else(|| rig_data.as_ref().map(|_| gltf::SkinData {
                    joints: vec![[0, 1, 2, 0]; surface.source.len()],
                    weights: vec![[1.0, 0.0, 0.0, 0.0]; surface.source.len()],
                    joints_1: Vec::new(),
                    weights_1: Vec::new(),
                })),
                material: None,
            });
            if genitals && st.positions.len() >= GENITAL_MIN_BODY_VERTICES {
                let level = pack.fine.as_ref().map_or(2, |f| f.level).saturating_sub(i);
                let (lod, report) = genital_lod(pack, controls, level, &st.positions, &normals, body_rig.as_ref().map(|br| &br.by_obj), format!("Genitals_LOD{i}"))?;
                genital_rows.push(report);
                body_lods.push(lod);
            }
            if !worn.is_empty() {
                let mut points = game.clone();
                points.extend_from_slice(&st.positions);
                let h = game.len() as u32;
                let tris: Vec<[u32; 3]> = level.triangles.iter().copied().chain(st.triangles.iter().map(|t| t.map(|v| v + h))).collect();
                let surface = super::wearable::Surface::new(points, &tris);
                let mut placed: Vec<Vec<Point>> = Vec::with_capacity(worn.len());
                for (item, garment, _, ladder, row) in worn.iter_mut() {
                    let (kept, tris, _) = &ladder[i.min(ladder.len() - 1)];
                    let mut positions: Vec<Point> = kept.iter().map(|&k| garment[k as usize]).collect();
                    if i > 0 && !item.manifest.transparent {
                        let pushed = super::wearable::push_out(&mut positions, tris, &surface, super::wearable::CLEARANCE_M);
                        if let Some(list) = row["lod_pushed"].as_array_mut() {
                            list.push(pushed["moved"].clone());
                        }
                    }
                    placed.push(positions);
                }
                if i > 0 {
                    let mut layers: Vec<super::wearable::Layered> = worn
                        .iter()
                        .zip(placed.iter_mut())
                        .map(|((item, _, _, ladder, _), positions)| super::wearable::Layered {
                            triangles: ladder[i.min(ladder.len() - 1)].1.clone(),
                            depth: super::wearable::depth_of(&item.binding),
                            opaque: !item.manifest.transparent,
                            positions,
                        })
                        .collect();
                    super::wearable::layer(&mut layers);
                }
                for ((item, _, slot, ladder, _), positions) in worn.iter().zip(&placed) {
                    let (kept, tris, uvs) = &ladder[i.min(ladder.len() - 1)];
                    let lod = garment_lod(item, positions, kept, tris, uvs, body_rig.as_ref().map(|br| &br.by_obj), *slot);
                    wear_slots.push((body_lods.len(), *slot));
                    body_lods.push(Lod { name: format!("{}_LOD{i}", lod.name), ..lod });
                }
            }
        }
        let report_only = match opts.get("body-gates").map(String::as_str).unwrap_or("refuse") {
            "refuse" => false,
            "report" => true,
            other => return Err(format!("--body-gates must be refuse or report, got {other}")),
        };
        if !failures.is_empty() && !report_only {
            return Err(format!("body stitch gates failed: {}", failures.join("; ")));
        }
        body_meta = json!({"mode": pack.mode, "pack": opts.get("body"), "source": pack_manifest["source"], "controls": controls, "lods": rows,
                           "mesh": "one mesh node per body LOD (Body_LODn beside Head_LODn), its own material slot and the body's UV layout; the seam vertices are copies of the head's with the same position, normal and skin weights",
                           "skin": if let Some(br) = &body_rig { br.meta.clone() } else if rig_data.is_some() { json!("every body vertex weighted 1.0 to the neck joint, like the head's seam vertices") } else { Value::Null }});
        if !failures.is_empty() {
            body_meta["gates_failed"] = json!({"exported_anyway": "--body-gates report", "failures": failures});
        }
        if !worn.is_empty() {
            body_meta["wearables"] = json!({"items": worn.iter().map(|w| w.4.clone()).collect::<Vec<_>>(), "hidden_base_vertices": covered.len(),
                                            "method": "MakeHuman clothes bindings evaluated on the character (head cage on the dense head, body and helpers on the shaped body with its controls), outward-facing vertices of opaque wearables inside or within 2 mm of the stitched LOD0 character pushed out along its normal (alpha cards are left as they are), then each opaque wearable pushed out of the opaque ones with a lower z_depth (shoes 5 under trousers 50); one node per wearable per LOD (<name>_LODn): LOD0 the full garment, each coarser level a nested decimation sized by the body level's vertex ratio (at least 32 vertices), every kept vertex an original one so the binding and skin carry over, each coarser level pushed out of its own body level (lod_pushed: vertices moved per level) and layered again by z_depth; body triangles whose corners are all covered by a hide_body wearable are left out of every body LOD; skinned through the binding's base vertices when a rig is asked"});
        }
        if genitals {
            body_meta["genitals"] = json!({"kind": "male", "lods": genital_rows,
                                           "method": "MakeHuman's helper-genital patch, shaped with the body's helpers (macros and the genital targets), refined by Catmull-Clark (one level less per LOD), its rim set on that LOD's body surface 2 mm inside along the body normal; one mesh per LOD (Genitals_LODn) on every LOD whose body keeps at least 2500 vertices (coarser LODs carry none), skinned like the body when a rig is asked"});
        }
    }
    let mut lods = Vec::new();
    let mut lod_meta = Vec::new();
    for (i, level) in levels.iter().enumerate() {
        let surface = gm::split_seams(&level.triangles, &level.uvs);
        let (game, mut normals) = lod_state(level, &neutral.game);
        let shared = seam_normals.get(i);
        if let Some(over) = shared {
            for (&v, n) in over {
                normals[v as usize] = *n;
            }
        }
        let pick = |v: &[Point]| -> Vec<Point> { surface.source.iter().map(|&g| v[g as usize]).collect() };
        let (gp, gn) = (pick(&game), pick(&normals));
        let mut morphs = Vec::new();
        let mut corrective_morphs = Vec::new();
        for mg in &morph_games {
            let (eg, mut en) = lod_state(level, mg);
            if let Some(over) = shared {
                for (&v, n) in over {
                    en[v as usize] = *n;
                }
            }
            let (ep, enn) = (pick(&eg), pick(&en));
            let mut positions_delta: Vec<[f32; 3]> = gp.iter().zip(&ep).map(|(a, b)| delta(*a, *b)).collect();
            let mut normals_delta: Vec<[f32; 3]> = gn.iter().zip(&enn).map(|(a, b)| delta(*a, *b)).collect();
            if let Some(floor) = floor_m {
                let length = |v: &[f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                for (p, q) in positions_delta.iter_mut().zip(normals_delta.iter_mut()) {
                    if (length(p) as f64) < floor {
                        *p = [0.0; 3];
                    }
                    if length(q) < NORMAL_FLOOR {
                        *q = [0.0; 3];
                    }
                }
            }
            morphs.push(MorphData { positions: positions_delta, normals: normals_delta });
            if let (Some((poses, pivots)), Some(influences)) = (&face2_correctives, &face_skin) {
                let inf: Vec<&[(usize, f64)]> = surface.source.iter().map(|&g| influences[level.map[g as usize] as usize].as_slice()).collect();
                let c = gf_core::face_rig2::corrective(&gp, &gn, &inf, pivots, &poses[corrective_morphs.len()], &ep, &enn);
                let mut positions: Vec<[f32; 3]> = c.positions.iter().map(|d| gm::to_gltf(*d)).collect();
                let mut normals: Vec<[f32; 3]> = c.normals.iter().map(|d| gm::to_gltf(*d)).collect();
                if let Some(floor) = floor_m {
                    let length = |v: &[f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                    for (p, q) in positions.iter_mut().zip(normals.iter_mut()) {
                        if (length(p) as f64) < floor {
                            *p = [0.0; 3];
                        }
                        if length(q) < NORMAL_FLOOR {
                            *q = [0.0; 3];
                        }
                    }
                }
                corrective_morphs.push(MorphData { positions, normals });
            }
        }
        morphs.extend(corrective_morphs);
        lod_meta.push(json!({"node": level.name, "game_vertices": level.mesh.kept.len(), "vertices": surface.source.len(),
                             "triangles": level.triangles.len(),
                             "screen_size": if i == 0 { 1.0 } else { screens[i - 1] }}));
        let face_lod_skin = face_skin.as_ref().map(|influences| {
            let wide = influences.iter().any(|i| i.len() > 4);
            let mut joints = Vec::with_capacity(surface.source.len());
            let mut weights = Vec::with_capacity(surface.source.len());
            let mut joints_1 = Vec::new();
            let mut weights_1 = Vec::new();
            for &g in &surface.source {
                let (mut j, mut w) = ([0u8; 8], [0.0f32; 8]);
                for (k, &(joint, weight)) in influences[level.map[g as usize] as usize].iter().enumerate() {
                    j[k] = joint as u8;
                    w[k] = weight as f32;
                }
                joints.push([j[0], j[1], j[2], j[3]]);
                weights.push([w[0], w[1], w[2], w[3]]);
                if wide {
                    joints_1.push([j[4], j[5], j[6], j[7]]);
                    weights_1.push([w[4], w[5], w[6], w[7]]);
                }
            }
            gltf::SkinData { joints, weights, joints_1, weights_1 }
        });
        let body_rig_skin = head_influences.as_ref().map(|hi| skin_data(&surface.source.iter().map(|&g| hi[level.map[g as usize] as usize].as_slice()).collect::<Vec<_>>()));
        let skin = if face_lod_skin.is_some() { face_lod_skin } else if body_rig_skin.is_some() { body_rig_skin } else { rig_data.as_ref().map(|(_, weights)| {
            let w: Vec<[f32; 4]> = surface
                .source
                .iter()
                .map(|&g| {
                    let x = weights[level.map[g as usize] as usize];
                    let q = |v: f64| ((v * WEIGHT_STEPS).round() / WEIGHT_STEPS) as f32;
                    let neck = q(x[0]);
                    let jaw = q(x[2]).min(1.0 - neck);
                    [neck, 1.0 - neck - jaw, jaw, 0.0]
                })
                .collect();
            gltf::SkinData { joints: vec![[0, 1, 2, 0]; w.len()], weights: w, joints_1: Vec::new(), weights_1: Vec::new() }
        }) };
        lods.push(Lod {
            name: level.name.clone(),
            positions: gp.iter().map(|p| gm::to_gltf(*p)).collect(),
            normals: gn.iter().map(|p| gm::to_gltf(*p)).collect(),
            uvs: surface.uvs.clone(),
            indices: surface.indices,
            morphs,
            skin,
            material: None,
        });
    }
    let morph_meta: Vec<Value> = ids
        .iter()
        .zip(&names)
        .zip(&morph_games)
        .map(|((id, name), mg)| {
            let d = gm::displacement(&neutral.game, mg);
            json!({"name": name, "target": id, "game_vertices_moved": d.entries, "max_mm": d.max_mm,
                   "over_0p01_mm": d.over_0p01_mm, "over_0p1_mm": d.over_0p1_mm})
        })
        .collect();
    let tool = format!("gazeforge {}", env!("CARGO_PKG_VERSION"));
    let extras = json!({"spec": spec.revision(), "pack": spec.base.pack, "game_base": game_ref,
                        "placement": placement.version().to_string(), "tool": tool});
    let glb_names: Vec<String> = if face2_correctives.is_some() { names.iter().cloned().chain(names.iter().map(|n| format!("{n}.corrective"))).collect() } else { names.clone() };
    if let (false, true, Some(eyes)) = (rigged, spec.eyeballs.is_some(), &moved_eyes) {
        for eye in eyes {
            let mut vertices = eye.vertices.clone();
            if let Some((pivot, s)) = head_scale {
                gf_core::body::scale_about(&mut vertices, pivot, s);
            }
            if eye_model.is_some() {
                let (joint, origin) = match &body_rig {
                    Some(br) => (Some(br.head_joint), br.heads[br.head_joint]),
                    None => (None, [0.0; 3]),
                };
                eye_parts.push(eye_part_mesh(eye, if eye.centre[0] < 0.0 { "Eye_R" } else { "Eye_L" }, joint, origin, &vertices));
                continue;
            }
            body_lods.push(Lod {
                name: if eye.centre[0] < 0.0 { "Eye_R" } else { "Eye_L" }.into(),
                positions: vertices.iter().map(|p| gm::to_gltf(*p)).collect(),
                normals: eye.vertex_normals().iter().map(|n| gm::to_gltf(*n)).collect(),
                uvs: vec![[0.0, 0.0]; vertices.len()],
                indices: eye.triangles.iter().flatten().copied().collect(),
                morphs: Vec::new(),
                skin: body_rig.as_ref().map(|br| skin_data(&vec![[(br.head_joint, 1.0)].as_slice(); vertices.len()])),
                material: None,
            });
        }
    }
    let mut materials = match &eye_model {
        Some(m) if !eye_parts.is_empty() => eye_materials(m)?,
        _ => Vec::new(),
    };
    let first_wear = materials.len();
    for (index, slot) in wear_slots {
        body_lods[index].material = Some(first_wear + slot);
    }
    materials.extend(wear_materials);
    let skin_report = match &spec.skin {
        Some(skin) => {
            crate::jobs::progress("skin", 0, 1, "textures and layers");
            let (material, report) = crate::skin::head_material(&store, skin, &dense.positions, &base)?;
            for lod in lods.iter_mut() {
                lod.material = Some(materials.len());
            }
            materials.push(material);
            Some(report)
        }
        None => None,
    };
    crate::jobs::progress("write", 0, 1, "glb");
    let root = world.map(|w| {
        let t = w.translation_for_origin();
        let q = w.quaternion();
        gltf::Root { name: "Character".into(), translation: [t[0], t[2], -t[1]], rotation: [q[0], q[2], -q[1], q[3]], scale: w.scale }
    });
    let (bytes, report) = gltf::glb_with_root(&lods, &body_lods, &glb_names, mode, extras.clone(), &tool, rig_data.as_ref().map(|(r, _)| r).or(body_gltf_rig.as_ref()), &eye_parts, &materials, root.as_ref())?;
    std::fs::write(out, &bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    let digest = hash::sha256_hex(&bytes);
    let file_name = out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stem = file_name.strip_suffix(".glb").unwrap_or(&file_name).to_string();
    let sidecar_path = out.with_file_name(format!("{stem}.gazeforge.json"));
    let mut sidecar = json!({
        "format": "gazeforge.sidecar/1",
        "glb": file_name,
        "glb_sha256": digest,
        "spec": extras["spec"], "pack": extras["pack"], "game_base": extras["game_base"], "placement": extras["placement"], "tool": extras["tool"],
        "frame": {"glb": "metres, +Y up, the face looks along +Z", "from_world": "(x, y, z) -> (x, z, -y)"},
        "uv_layer": "head",
        "lods": lod_meta,
        "morphs": morph_meta,
        "morph_definition": "F(c + e) - F(c): the placed game positions of the character with the target at weight 1, minus without",
    });
    if extended {
        for (m, slip) in sidecar["morphs"].as_array_mut().into_iter().flatten().zip(&slips) {
            m["uv_slip_lod0"] = slip.clone();
        }
        sidecar["uv_policy"] = json!("neutral: every LOD carries the neutral character's UVs and glTF morphs carry no UVs, so under a morph the texture rides with the vertices; uv_slip_lod0 is how far F's own re-derived UVs for that morph at weight 1 sit from the neutral ones");
        sidecar["morph_floor_mm"] = json!(floor_m.map(|f| f * 1000.0));
        sidecar["morph_normal_floor"] = json!(floor_m.map(|_| NORMAL_FLOOR));
        sidecar["facing"] = json!({"mode": facing, "lods": facing_report});
        sidecar["lod_selection"] = lod_selection.clone();
        sidecar["rig"] = rig_meta.clone();
    }
    if let Some(run) = &lid_run {
        let morphs: Vec<Value> = names.iter().zip(&morph_lids).map(|(n, r)| json!({"morph": n, "placement": r})).collect();
        let collided: Vec<&String> = names.iter().zip(&morph_lids).filter(|(_, r)| r["collided"] == json!(true)).map(|(n, _)| n).collect();
        sidecar["lids"] = json!({
            "mode": "auto", "step_mm": run.step * 1000.0, "max_mm": run.max * 1000.0, "front_deg": run.front, "tolerance_mm": run.tolerance * 1000.0,
            "smooth": run.smooth,
            "method": "after F, the finest LOD is checked against the eyes; a state that fails is collided at dense level (collide-lids) with clearance 0, step, 2 x step, ... up to max_mm, F re-run each time, and the first clearance that passes is kept; a state that passes is left untouched; morph states start from the neutral's dense shape as exported",
            "neutral": neutral_lids, "morph_states_collided": collided, "morphs": morphs, "touch_up": lid_touch,
            "final": lid_gate.as_ref().map(|g| g.0.clone())});
    }
    if let Some(stem) = lod0_out {
        let obj_path = stem.with_extension("obj");
        let npy_path = stem.with_extension("dense.npy");
        obj::write_triangles(&obj_path, &neutral.game, &levels[0].triangles, &levels[0].uvs)?;
        dense::write_points(&npy_path, &neutral_dense)?;
        sidecar["lod0_out"] = json!({"obj": obj_path.display().to_string(), "dense": npy_path.display().to_string()});
    }
    if !body_meta.is_null() {
        sidecar["body"] = body_meta.clone();
        if !shaped_report.is_null() {
            sidecar["body"]["shaped"] = shaped_report.clone();
            sidecar["body"]["root"] = json!(root.as_ref().map(|r| json!({"node": r.name, "translation_gltf": r.translation, "rotation_gltf": r.rotation, "scale": r.scale})));
        }
    }
    if let Some(skin) = &skin_report {
        sidecar["skin"] = skin.clone();
    }
    let text = serde_json::to_string_pretty(&sidecar).map_err(|e| e.to_string())?;
    std::fs::write(&sidecar_path, text).map_err(|e| format!("{}: {e}", sidecar_path.display()))?;
    let mut result = json!({"ok": true, "command": "export-glb", "out": out.display().to_string(), "sidecar": sidecar_path.display().to_string(),
              "glb_sha256": digest, "bytes": bytes.len(), "placement_version": placement.version().to_string(),
              "spec_revision": extras["spec"], "evaluation": evaluation, "lods": report, "lod_uvs": lod_uv_report,
              "morphs": sidecar["morphs"], "seconds": started.elapsed().as_secs_f64()});
    if extended {
        result["facing"] = sidecar["facing"].clone();
        result["lod_selection"] = lod_selection;
        result["rig"] = rig_meta;
    }
    if lid_run.is_some() {
        result["lids"] = sidecar["lids"].clone();
    }
    if !body_meta.is_null() {
        result["body"] = body_meta;
    }
    if lod0_out.is_some() {
        result["lod0_out"] = sidecar["lod0_out"].clone();
    }
    if let Some(eyes) = eyes_report {
        result["eyes"] = json!(eyes);
    }
    if let Some(skin) = skin_report {
        result["skin"] = skin;
    }
    Ok(result)
}
