use std::path::Path;

use gf_core::game_mesh::{self as gm, Point};
use gf_core::gates::{self, DenseSurface, ShellLimits};
use gf_core::hash;
use gf_core::placement::{DenseMesh, PlacementVersion};
use gf_geom::Uv;
use gf_geom::decimate::Decimated;
use gf_io::character as ch;
use gf_io::dense::{self, Dense};
use gf_io::gltf::{self, Lod, MorphData, Sparse};
use gf_io::store::Store;
use gf_io::{game_base, obj};
use serde_json::{Value, json};

use crate::{Options, count, need, number, only, placement_keys, placer};

const SHELL_KEYS: &[&str] = &["dense", "before", "after", "fold-tolerance", "inner-tolerance", "rough-mm", "rough-rel", "rough-tolerance"];
const GAME_KEYS: &[&str] = &["dense", "base", "face", "obj", "tip-vertex", "gates", "seam-limit-mm"];
const PATH_KEYS: &[&str] = &["dense", "base", "start", "delta", "steps", "placement", "pop-limit", "plant"];
const GLB_KEYS: &[&str] = &["store", "spec", "macro", "out", "morphs", "sparse", "lod-screen"];
const GAME_GATES: &[&str] = &["triq", "uv", "census", "seams"];

fn verdict(command: &str, gates: Value, failed: Vec<String>, extra: Value) -> Value {
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
    if mesh.triangles() != base.triangles {
        return Err(format!("the triangles of {} differ from the game base's", path.display()));
    }
    let uvs = mesh.corner_uvs();
    let game = &mesh.positions;
    let mut out = serde_json::Map::new();
    let mut failed = Vec::new();
    for name in &chosen {
        let (value, pass) = match name.as_str() {
            "triq" => {
                let r = gates::triangle_quality(game, &base.triangles, tip);
                (json!(r), r.pass)
            }
            "uv" => {
                let r = gates::uv_health(game, &base.triangles, &uvs);
                (json!(r), r.pass)
            }
            "census" => {
                let r = gates::census(&face, &dense.triangles, &base, game, tip);
                (json!(r), r.pass)
            }
            _ => {
                let surface = DenseSurface { positions: &face, triangles: &dense.triangles, triangle_uvs: &dense.triangle_uvs };
                let r = gates::seams(game, &base.triangles, &uvs, &base.triangle_uvs, &surface, tip, seam_limit);
                (json!(r), r.pass)
            }
        };
        if !pass {
            failed.push(name.clone());
        }
        out.insert(name.clone(), value);
    }
    Ok(verdict("gates game", Value::Object(out), failed, json!({"tip_m": tip, "obj": path.display().to_string()})))
}

pub fn gates_path(opts: &Options) -> Result<Value, String> {
    let given = opts.get("placement").map(String::as_str).unwrap_or("f3");
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
        let t = k as f64 / steps as f64;
        let shape: Vec<Point> = start.iter().zip(&delta).map(|(s, d)| [0, 1, 2].map(|i| (s[i] + t * d[i]) as f32 as f64)).collect();
        let mut game = placement.place(&shape, &mesh, &base).game;
        if let Some((_, v, mm)) = plant.filter(|p| p.0 == k) {
            game[v][1] -= mm / 1000.0;
        }
        placed.push(game);
        subset.push(base.kept.iter().map(|&i| shape[i as usize]).collect::<Vec<_>>());
    }
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
}

fn lod_state(level: &Level, game0: &[Point]) -> (Vec<Point>, Vec<Point>) {
    let game: Vec<Point> = level.map.iter().map(|&j| game0[j as usize]).collect();
    let normals = gm::game_normals(&game, &level.mesh.triangles);
    (game, normals)
}

fn delta(a: Point, b: Point) -> [f32; 3] {
    gm::to_gltf([b[0] - a[0], b[1] - a[1], b[2] - a[2]])
}

pub fn export_glb(opts: &Options) -> Result<Value, String> {
    let store = Store::open(Path::new(need(opts, "store")?))?;
    let spec = ch::read_spec(Path::new(need(opts, "spec")?))?;
    let version = PlacementVersion::parse(&spec.base.placement)?;
    let mut allowed: Vec<&[&str]> = vec![GLB_KEYS];
    allowed.extend_from_slice(placement_keys(version.name));
    only(opts, &format!("export-glb (spec placement {version})"), &allowed)?;
    let placement = placer(opts, version)?;
    let mode = Sparse::parse(opts.get("sparse").map(String::as_str).unwrap_or("auto"))?;
    let out = Path::new(need(opts, "out")?);
    let started = std::time::Instant::now();
    let source = match opts.get("macro") {
        Some(p) => Some(ch::read_macro_definition(Path::new(p))?),
        None => None,
    };
    let (positions, evaluation) = ch::evaluate(&store, &spec, source.as_ref())?;
    let (_, dense) = store.load_pack(&spec.base.pack)?;
    let catalog = store.load_catalog(&spec.base.catalog)?;
    let game_ref = spec.base.game_base.clone().ok_or("export-glb needs a spec with base.game_base")?;
    let (manifest, base) = store.load_game(&game_ref)?;
    let screens: Vec<f64> = match opts.get("lod-screen") {
        Some(text) => text.split(',').map(|s| s.trim().parse::<f64>().map_err(|_| format!("--lod-screen takes numbers, got {text}"))).collect::<Result<_, _>>()?,
        None => (0..manifest.lods.len()).map(|i| [0.25, 0.1, 0.04].get(i).copied().unwrap_or(0.02)).collect(),
    };
    if screens.len() != manifest.lods.len() || screens.iter().any(|s| !s.is_finite() || *s <= 0.0 || *s >= 1.0) {
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
    let neutral = placement.place(&positions, &dmesh, &base);
    let uvs0 = neutral.triangle_uvs.clone().unwrap_or_else(|| base.triangle_uvs.clone());
    let mut morph_games = Vec::new();
    for id in &ids {
        let target = store.load_target(&catalog, id)?;
        let mut moved = positions.clone();
        target.add_scaled(1.0, &mut moved);
        morph_games.push(placement.place(&moved, &dmesh, &base).game);
    }
    let mut coarse = Vec::new();
    for lod in &manifest.lods {
        coarse.push(store.load_game(&lod.game)?.1);
    }
    let mut levels = vec![Level { name: "Head_LOD0".into(), map: (0..base.kept.len() as u32).collect(), uvs: uvs0.clone(), mesh: &base }];
    let mut lod_uv_report = Vec::new();
    for (i, mesh) in coarse.iter().enumerate() {
        let map = gm::lod_map(&base, mesh)?;
        let (uvs, unmatched, worst) = gm::lod_uvs(&base, &uvs0, mesh, &map);
        lod_uv_report.push(json!({"lod": i + 1, "corners_without_a_matching_chart": unmatched, "worst_chart_distance": worst}));
        levels.push(Level { name: format!("Head_LOD{}", i + 1), mesh, map, uvs });
    }
    let mut lods = Vec::new();
    let mut lod_meta = Vec::new();
    for (i, level) in levels.iter().enumerate() {
        let surface = gm::split_seams(&level.mesh.triangles, &level.uvs);
        let (game, normals) = lod_state(level, &neutral.game);
        let pick = |v: &[Point]| -> Vec<Point> { surface.source.iter().map(|&g| v[g as usize]).collect() };
        let (gp, gn) = (pick(&game), pick(&normals));
        let mut morphs = Vec::new();
        for mg in &morph_games {
            let (eg, en) = lod_state(level, mg);
            let (ep, enn) = (pick(&eg), pick(&en));
            morphs.push(MorphData {
                positions: gp.iter().zip(&ep).map(|(a, b)| delta(*a, *b)).collect(),
                normals: gn.iter().zip(&enn).map(|(a, b)| delta(*a, *b)).collect(),
            });
        }
        lod_meta.push(json!({"node": level.name, "game_vertices": level.mesh.kept.len(), "vertices": surface.source.len(),
                             "triangles": level.mesh.triangles.len(),
                             "screen_size": if i == 0 { 1.0 } else { screens[i - 1] }}));
        lods.push(Lod {
            name: level.name.clone(),
            positions: gp.iter().map(|p| gm::to_gltf(*p)).collect(),
            normals: gn.iter().map(|p| gm::to_gltf(*p)).collect(),
            uvs: surface.uvs.clone(),
            indices: surface.indices,
            morphs,
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
    let (bytes, report) = gltf::glb(&lods, &names, mode, extras.clone(), &tool)?;
    std::fs::write(out, &bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    let digest = hash::sha256_hex(&bytes);
    let file_name = out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stem = file_name.strip_suffix(".glb").unwrap_or(&file_name).to_string();
    let sidecar_path = out.with_file_name(format!("{stem}.gazeforge.json"));
    let sidecar = json!({
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
    let text = serde_json::to_string_pretty(&sidecar).map_err(|e| e.to_string())?;
    std::fs::write(&sidecar_path, text).map_err(|e| format!("{}: {e}", sidecar_path.display()))?;
    Ok(json!({"ok": true, "command": "export-glb", "out": out.display().to_string(), "sidecar": sidecar_path.display().to_string(),
              "glb_sha256": digest, "bytes": bytes.len(), "placement_version": placement.version().to_string(),
              "spec_revision": extras["spec"], "evaluation": evaluation, "lods": report, "lod_uvs": lod_uv_report,
              "morphs": sidecar["morphs"], "seconds": started.elapsed().as_secs_f64()}))
}
