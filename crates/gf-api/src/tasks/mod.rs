use std::collections::HashMap;
use std::path::Path;

use gf_core::placement::{self as gp, DenseMesh, Placement, PlacementVersion};
use gf_geom::decimate::{self, Settings};
use gf_geom::{lids, placement, placement_f2, placement_f3, placement_f5, topology as topology_geom};
use gf_io::dense::{self, Dense};
use gf_io::{game_base, obj};
use serde_json::{Value, json};

pub mod api;
pub mod bake;
pub mod data;
pub mod mesh;
pub mod rig;
pub mod topology;
pub mod vrm;
pub mod wearable;

pub const BUILD_KEYS: &[&str] = &[
    "dense",
    "out",
    "target",
    "shapes",
    "extra",
    "min-angle",
    "max-angle",
    "max-turn",
    "boundary-weight",
    "improve-below",
    "flip-flat",
    "weights",
    "lods",
];
pub const EXPORT_KEYS: &[&str] = &["dense", "base", "positions", "out", "placement", "tip-vertex", "quads"];
pub const OPTIMAL_KEYS: &[&str] = &["max-move"];
pub const FIT_KEYS: &[&str] = &["max-move", "fit-iterations", "fit-smooth", "fit-min-angle", "fit-guard-steps", "fit-repair-below", "fit-trim"];
pub const F2_KEYS: &[&str] = &[
    "f2-regular",
    "f2-max-turn",
    "f2-turn-margin",
    "f2-confident-spread",
    "f2-confident-above",
    "f2-angle-margin",
    "f2-repair-rounds",
    "f2-repair-max-step",
    "f2-repair-max-step-confident",
    "f2-uv-ring",
    "f2-uvs",
];

pub const F3_KEYS: &[&str] = &[
    "f3-regular",
    "f3-max-move",
    "f3-smooth",
    "f3-rounds",
    "f3-relax-steps",
    "f3-relax-step",
    "f3-relax-angle",
    "f3-relax-eye",
    "f3-backoff-turn",
    "f3-backoff-step",
    "f3-uv-ring",
    "eyes",
    "f3-eye-front",
    "f3-eye-reach",
    "f3-eye-depth",
    "f3-eye-lift",
    "f3-eye-clearance",
    "f3-eye-rounds",
    "f3-eye-samples",
];
pub const F4_KEYS: &[&str] = &["f4-target", "f4-passes", "f4-step", "f4-gain", "f4-fold", "f4-backoff", "f4-plane"];
pub const F5_KEYS: &[&str] = &[
    "f5-tangential",
    "f5-smooth",
    "f5-membrane",
    "f5-max-move",
    "f5-uv-ring",
    "f5-normals",
    "eyes",
    "f5-eye-front",
    "f5-eye-depth",
    "f5-eye-clearance",
];
pub const F6_KEYS: &[&str] = &["f6-sliver-deg", "f6-sliver-rounds", "f6-sliver-relax", "f6-sliver-seam", "f6-sliver-eye"];
pub const COLLIDE_KEYS: &[&str] = &["dense", "positions", "eyes", "out", "clearance", "front-deg", "smooth"];

pub type Options = HashMap<String, String>;

pub fn only(opts: &Options, command: &str, allowed: &[&[&str]]) -> Result<(), String> {
    let mut unknown: Vec<&str> = opts.keys().map(String::as_str).filter(|k| !allowed.iter().any(|set| set.contains(k))).collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    let mut accepted: Vec<&str> = allowed.iter().flat_map(|set| set.iter().copied()).collect();
    accepted.sort_unstable();
    accepted.dedup();
    let flags = |keys: &[&str]| keys.iter().map(|k| format!("--{k}")).collect::<Vec<_>>().join(" ");
    Err(format!("unknown option for {command}: {}; it accepts {}", flags(&unknown), flags(&accepted)))
}

pub fn need<'a>(opts: &'a Options, key: &str) -> Result<&'a str, String> {
    opts.get(key).map(String::as_str).ok_or_else(|| format!("missing --{key}"))
}

pub fn number(opts: &Options, key: &str, default: f64) -> Result<f64, String> {
    let Some(text) = opts.get(key) else {
        return Ok(default);
    };
    let value: f64 = text.parse().map_err(|_| format!("--{key} is not a number: {text}"))?;
    if !value.is_finite() {
        return Err(format!("--{key} must be finite, got {text}"));
    }
    if value < 0.0 {
        return Err(format!("--{key} must not be negative, got {text}"));
    }
    Ok(value)
}

pub fn count(opts: &Options, key: &str, default: usize) -> Result<usize, String> {
    let value = number(opts, key, default as f64)?;
    if value.fract() != 0.0 || value > u32::MAX as f64 {
        return Err(format!("--{key} must be a whole number, got {}", opts.get(key).map_or("", String::as_str)));
    }
    Ok(value as usize)
}

pub fn build_base(opts: &Options) -> Result<Value, String> {
    only(opts, "build-base", &[BUILD_KEYS])?;
    let settings = Settings {
        target_vertices: count(opts, "target", 20000)?,
        min_angle_deg: number(opts, "min-angle", 4.0)?,
        max_angle_deg: number(opts, "max-angle", 170.0)?,
        max_normal_turn_deg: number(opts, "max-turn", 45.0)?,
        boundary_weight: number(opts, "boundary-weight", 100.0)?,
        improve_below_deg: number(opts, "improve-below", 20.0)?,
        flip_flat_deg: number(opts, "flip-flat", 15.0)?,
    };
    let out = Path::new(need(opts, "out")?);
    crate::jobs::progress("load", 0, 1, "dense base and shapes");
    let dense = Dense::load(Path::new(need(opts, "dense")?))?;
    let names: Vec<String> = match opts.get("shapes").map(String::as_str).unwrap_or("all") {
        "all" => dense.manifest.shapes.iter().map(|s| s.name.clone()).collect(),
        "none" => Vec::new(),
        list => list.split(',').map(str::to_string).collect(),
    };
    let mut shapes = vec![dense.positions.clone()];
    for name in &names {
        shapes.push(dense.shape(name)?);
    }
    let extra: Vec<String> = opts.get("extra").map(|v| v.split(',').map(str::to_string).collect()).unwrap_or_default();
    for path in &extra {
        shapes.push(dense.positions_from(path)?);
    }
    let lods = lod_targets(opts, settings.target_vertices)?;
    let weights = match opts.get("weights") {
        Some(path) => Some(dense.vertex_weights(path)?),
        None => None,
    };
    crate::jobs::progress("decimate", 0, 1, "");
    let started = std::time::Instant::now();
    let clock = || started.elapsed().as_secs_f64();
    let mut levels =
        decimate::decimate_levels(shapes, &dense.triangles, &dense.triangle_uvs, &settings, weights.as_deref(), &lods, Some(&clock));
    crate::jobs::progress("save", 0, 1, "");
    let result = levels.remove(0);
    game_base::save(out, &result)?;
    let mut coarser = Vec::new();
    for (target, level) in lods.iter().zip(&levels) {
        let dir = out.join(format!("lod_{target}"));
        game_base::save(&dir, level)?;
        coarser.push(json!({"target": target, "out": dir.display().to_string(), "stats": level.stats}));
    }
    let weight_summary = weights.as_ref().map(|w| {
        let (lo, hi) = w.iter().fold((f64::INFINITY, 0.0f64), |(a, b), &x| (a.min(x), b.max(x)));
        json!({"path": opts.get("weights"), "min": lo, "max": hi, "mean": w.iter().sum::<f64>() / w.len() as f64})
    });
    Ok(json!({"ok": true, "command": "build-base", "out": out.display().to_string(), "shapes": names, "extra": extra,
              "settings": settings, "weights": weight_summary, "stats": result.stats, "lods": coarser}))
}

pub const DERIVE_KEYS: &[&str] = &["store", "game", "dense", "out", "lods", "shapes", "weights", "min-angle", "max-angle", "max-turn", "boundary-weight", "improve-below", "flip-flat", "seam-weight"];

pub fn derive_lods(opts: &Options) -> Result<Value, String> {
    only(opts, "derive-lods", &[DERIVE_KEYS])?;
    let store = gf_io::store::Store::open(Path::new(need(opts, "store")?))?;
    let reference = need(opts, "game")?;
    let (manifest, base) = store.load_game(reference)?;
    let lods = lod_targets(opts, base.kept.len())?;
    if lods.is_empty() {
        return Err("--lods names the coarser levels to derive, e.g. 10000,5000,2500".into());
    }
    let settings = Settings {
        target_vertices: lods[0],
        min_angle_deg: number(opts, "min-angle", 4.0)?,
        max_angle_deg: number(opts, "max-angle", 170.0)?,
        max_normal_turn_deg: number(opts, "max-turn", 45.0)?,
        boundary_weight: number(opts, "boundary-weight", 100.0)?,
        improve_below_deg: number(opts, "improve-below", 20.0)?,
        flip_flat_deg: number(opts, "flip-flat", 15.0)?,
    };
    crate::jobs::progress("load", 0, 1, "dense base and shapes");
    let dense = Dense::load(Path::new(need(opts, "dense")?))?;
    if base.owner.len() != dense.positions.len() {
        return Err(format!("the game base covers {} dense vertices, the dense base has {}", base.owner.len(), dense.positions.len()));
    }
    let names: Vec<String> = match opts.get("shapes").map(String::as_str).unwrap_or("all") {
        "all" => dense.manifest.shapes.iter().map(|s| s.name.clone()).collect(),
        "none" => Vec::new(),
        list => list.split(',').map(str::to_string).collect(),
    };
    let pick = |all: &[[f64; 3]]| -> Vec<[f64; 3]> { base.kept.iter().map(|&d| all[d as usize]).collect() };
    let mut shapes = vec![pick(&dense.positions)];
    for name in &names {
        shapes.push(pick(&dense.shape(name)?));
    }
    let mut weights = match opts.get("weights") {
        Some(path) => {
            let all = dense.vertex_weights(path)?;
            Some(base.kept.iter().map(|&d| all[d as usize]).collect::<Vec<f64>>())
        }
        None => None,
    };
    let seam_weight = number(opts, "seam-weight", 1.0)?;
    if seam_weight <= 0.0 {
        return Err(format!("--seam-weight multiplies the open boundary's collapse cost and must be > 0, got {seam_weight}"));
    }
    let boundary: Vec<u32> = gf_core::rig::boundary_loops(&base.triangles).into_iter().flatten().collect();
    if seam_weight != 1.0 {
        let w = weights.get_or_insert_with(|| vec![1.0; base.kept.len()]);
        for &v in &boundary {
            w[v as usize] *= seam_weight;
        }
    }
    crate::jobs::progress("decimate", 0, 1, "");
    let started = std::time::Instant::now();
    let clock = || started.elapsed().as_secs_f64();
    let levels = decimate::decimate_levels(shapes, &base.triangles, &base.triangle_uvs, &settings, weights.as_deref(), &lods[1..], Some(&clock));
    let out = Path::new(need(opts, "out")?);
    game_base::save(out, &base)?;
    std::fs::write(out.join("report.json"), serde_json::to_string_pretty(&manifest.build).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut rows = Vec::new();
    for (target, level) in lods.iter().zip(levels) {
        let kept: Vec<u32> = level.kept.iter().map(|&k| base.kept[k as usize]).collect();
        let owner: Vec<u32> = base.owner.iter().map(|&g| level.owner[g as usize]).collect();
        let derived = decimate::Decimated { kept, owner, triangles: level.triangles, triangle_uvs: level.triangle_uvs, stats: level.stats };
        let dir = out.join(format!("lod_{target}"));
        game_base::save(&dir, &derived)?;
        let mut stored = serde_json::to_value(&derived.stats).map_err(|e| e.to_string())?;
        stored.as_object_mut().map(|m| m.remove("seconds"));
        std::fs::write(dir.join("report.json"), serde_json::to_string_pretty(&stored).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", dir.display()))?;
        let seam_points = gf_core::rig::boundary_loops(&derived.triangles).iter().map(Vec::len).sum::<usize>();
        rows.push(json!({"target": target, "vertices": derived.kept.len(), "triangles": derived.triangles.len(), "seam_points": seam_points, "stats": derived.stats}));
    }
    let (game, _) = store.import_game(out, &manifest.pack)?;
    Ok(json!({"ok": true, "command": "derive-lods", "from": reference, "game": game, "out": out.display().to_string(),
              "lod0": {"vertices": base.kept.len(), "triangles": base.triangles.len(), "seam_points": boundary.len(), "same_as": reference},
              "lods": rows, "shapes": names, "seam_weight": seam_weight,
              "method": "the finest level is the given base's, unchanged; every coarser level decimates it (its vertices, triangles and UVs, carried through the dense shapes), so each is a subset of the finest and a new level can be added at any time; the stored level reports leave out the decimation time (it is in lods[].stats here), so the same inputs give the same reference"}))
}

pub fn lod_targets(opts: &Options, finest: usize) -> Result<Vec<usize>, String> {
    let Some(text) = opts.get("lods") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for part in text.split(',') {
        let value: usize = part.trim().parse().map_err(|_| format!("--lods takes whole vertex counts, got {part}"))?;
        let above = out.last().copied().unwrap_or(finest);
        if value == 0 || value >= above {
            return Err(format!("--lods must be strictly decreasing and below --target {finest}, got {text}"));
        }
        out.push(value);
    }
    Ok(out)
}

pub fn placement_keys(name: &str) -> &'static [&'static [&'static str]] {
    match name {
        "subset" => &[],
        "optimal" => &[OPTIMAL_KEYS],
        "fit" => &[FIT_KEYS],
        "f2" => &[FIT_KEYS, F2_KEYS],
        "f3" => &[F3_KEYS],
        "f5" => &[F5_KEYS],
        "f6" => &[F5_KEYS, F6_KEYS],
        _ => &[F3_KEYS, F4_KEYS],
    }
}

pub fn placer(opts: &Options, version: PlacementVersion) -> Result<Box<dyn Placement>, String> {
    Ok(match version.name {
        "f3" | "f4" => Box::new(f3_placement(opts, version.name)?),
        "f5" => Box::new(f5_placement(opts)?),
        "f6" => Box::new(f6_placement(opts)?),
        "subset" => Box::new(gp::Subset),
        "optimal" => Box::new(gp::Optimal { max_move: number(opts, "max-move", 3.0)? / 1000.0 }),
        "fit" => Box::new(gp::Fit { settings: fit_settings(opts)? }),
        _ => {
            let (settings, uv_mode) = f2_settings(opts, &fit_settings(opts)?)?;
            Box::new(gp::F2 { settings, surface_uvs: uv_mode == "surface" })
        }
    })
}

pub fn placer_with(opts: &Options, version: PlacementVersion, eyes: Option<Vec<gf_geom::lids::Eye>>) -> Result<Box<dyn Placement>, String> {
    let Some(eyes) = eyes else {
        return placer(opts, version);
    };
    Ok(match version.name {
        "f3" | "f4" => Box::new(gp::F3 { eyes, ..f3_placement(opts, version.name)? }),
        "f5" => Box::new(gp::F5 { eyes, ..f5_placement(opts)? }),
        "f6" => Box::new(gp::F6 { eyes, ..f6_placement(opts)? }),
        _ => placer(opts, version)?,
    })
}

pub fn export_obj(opts: &Options) -> Result<Value, String> {
    let default = gp::DEFAULT.to_string();
    let given = opts.get("placement").map(String::as_str).unwrap_or(&default);
    let version = PlacementVersion::parse(given)
        .map_err(|_| format!("--placement must be optimal, fit, f2, f3, f4, f5, f6 or subset (optionally with its version, e.g. f3/1), got {given}"))?;
    let mut allowed: Vec<&[&str]> = vec![EXPORT_KEYS];
    allowed.extend_from_slice(placement_keys(version.name));
    only(opts, &format!("export-obj --placement {given}"), &allowed)?;
    let tip_vertex = count(opts, "tip-vertex", dense::NOSE_TIP_VERTEX)?;
    let placer = placer(opts, version)?;
    let out = Path::new(need(opts, "out")?);
    crate::jobs::progress("load", 0, 1, "");
    let dense = Dense::load(Path::new(need(opts, "dense")?))?;
    let base = game_base::load(Path::new(need(opts, "base")?))?;
    let positions = dense.positions_from(need(opts, "positions")?)?;
    let tip = dense.nose_tip(&positions, tip_vertex)?;
    let quads = match opts.get("quads").map(String::as_str) {
        None | Some("off") => false,
        Some("on") => true,
        Some(other) => return Err(format!("--quads takes on or off, got {other}")),
    };
    crate::jobs::progress("place", 0, 1, version.name);
    let placed = placer.place(&positions, &DenseMesh { triangles: &dense.triangles, triangle_uvs: &dense.triangle_uvs }, &base);
    let written_uvs = placed.triangle_uvs.as_ref().unwrap_or(&base.triangle_uvs);
    let mut split = 0;
    let faces = quads.then(|| {
        let rest: Vec<[f64; 3]> = base.kept.iter().map(|&k| dense.positions[k as usize]).collect();
        let settings = topology_geom::QuadSettings::default();
        let paired = topology_geom::quads(&rest, &base.triangles, &base.triangle_uvs, &settings);
        let (faces, n) = topology_geom::clean_quads(&paired, &placed.game, &base.triangles, written_uvs, &settings);
        split = n;
        faces
    });
    match &faces {
        Some(faces) => obj::write_faces(out, &placed.game, &base.triangles, written_uvs, faces)?,
        None => obj::write_triangles(out, &placed.game, &base.triangles, written_uvs)?,
    }
    let measure = placement::measure(&positions, &dense.triangles, &base, &placed.game, tip);
    let mut value = json!({"ok": true, "command": "export-obj", "out": out.display().to_string(), "vertices": base.kept.len(),
                           "triangles": base.triangles.len(), "placement": version.name,
                           "placement_version": placer.version().to_string(), "measure": measure});
    if let Some(faces) = &faces {
        let quad_count = faces.iter().filter(|f| matches!(f, topology_geom::Face::Quad(_))).count();
        value["quads"] = json!({"quads": quad_count, "triangles": faces.len() - quad_count, "faces": faces.len(), "split_on_this_mesh": split,
                                "settings": topology_geom::QuadSettings::default(),
                                "rule": "pairs are chosen on the base's rest shape and texture, so characters of one base share them; a pair that is not clean on the written mesh (folded, not convex in 3D or texture, a corner outside the limits) is written as its two triangles"});
    }
    for (k, v) in placed.report {
        value[k] = v;
    }
    Ok(value)
}

pub fn fit_settings(opts: &Options) -> Result<placement::FitSettings, String> {
    Ok(placement::FitSettings {
        iterations: count(opts, "fit-iterations", 4)?,
        smooth: number(opts, "fit-smooth", 0.1)?,
        max_move: number(opts, "max-move", 3.0)? / 1000.0,
        min_angle_deg: number(opts, "fit-min-angle", 5.0)?,
        guard_steps: count(opts, "fit-guard-steps", 8)?,
        repair_below_deg: number(opts, "fit-repair-below", 5.0)?,
        trim: number(opts, "fit-trim", 0.0)? / 1000.0,
    })
}

pub fn f2_settings(opts: &Options, fit: &placement::FitSettings) -> Result<(placement_f2::F2Settings, String), String> {
    let settings = placement_f2::F2Settings {
        fit: fit.clone(),
        regular: number(opts, "f2-regular", 1e-2)?,
        max_turn_deg: number(opts, "f2-max-turn", 60.0)?,
        turn_margin_deg: number(opts, "f2-turn-margin", 2.0)?,
        confident_spread_deg: number(opts, "f2-confident-spread", 30.0)?,
        confident_above_deg: number(opts, "f2-confident-above", 25.0)?,
        angle_margin_deg: number(opts, "f2-angle-margin", 0.5)?,
        repair_rounds: count(opts, "f2-repair-rounds", 8)?,
        repair_max_step: number(opts, "f2-repair-max-step", 0.4)? / 1000.0,
        repair_max_step_confident: number(opts, "f2-repair-max-step-confident", 0.5)? / 1000.0,
        uv_ring: count(opts, "f2-uv-ring", 2)?,
    };
    let uv_mode = opts.get("f2-uvs").map(String::as_str).unwrap_or("surface");
    if uv_mode != "surface" && uv_mode != "subset" {
        return Err(format!("--f2-uvs must be surface or subset, got {uv_mode}"));
    }
    Ok((settings, uv_mode.to_string()))
}

pub fn pair(opts: &Options, key: &str, default: [f64; 2]) -> Result<[f64; 2], String> {
    let Some(text) = opts.get(key) else {
        return Ok(default);
    };
    let parts: Vec<&str> = text.split(',').collect();
    let values: Vec<f64> = parts.iter().filter_map(|p| p.trim().parse::<f64>().ok()).collect();
    if parts.len() != 2 || values.len() != 2 || values.iter().any(|v| !v.is_finite() || *v < 0.0) || values[0] >= values[1] {
        return Err(format!("--{key} takes two increasing non-negative numbers LOW,HIGH, got {text}"));
    }
    Ok([values[0], values[1]])
}

pub fn f3_settings(opts: &Options) -> Result<placement_f3::F3Settings, String> {
    Ok(placement_f3::F3Settings {
        regular: number(opts, "f3-regular", 1e-3)?,
        max_move: number(opts, "f3-max-move", 3.0)? / 1000.0,
        smooth: number(opts, "f3-smooth", 0.1)?,
        rounds: count(opts, "f3-rounds", 4)?,
        relax_steps: count(opts, "f3-relax-steps", 24)?,
        relax_step: number(opts, "f3-relax-step", 0.5)?,
        relax_angle_deg: pair(opts, "f3-relax-angle", [1.0, 8.0])?,
        relax_eye_mm: pair(opts, "f3-relax-eye", [3.0, 8.0]).map(|p| p.map(|x| x / 1000.0))?,
        backoff_turn_deg: pair(opts, "f3-backoff-turn", [25.0, 55.0])?,
        backoff_step: number(opts, "f3-backoff-step", 0.5)?,
        uv_ring: count(opts, "f3-uv-ring", 2)?,
        clear: lids::ClearSettings {
            along_gaze: match opts.get("f3-eye-lift").map(String::as_str).unwrap_or("radial") {
                "radial" => false,
                "gaze" => true,
                other => return Err(format!("--f3-eye-lift must be radial or gaze, got {other}")),
            },
            reach: number(opts, "f3-eye-reach", 2.0)? / 1000.0,
            depth: pair(opts, "f3-eye-depth", [0.3, 0.6])?.map(|x| x / 1000.0),
            clearance: number(opts, "f3-eye-clearance", 0.05)? / 1000.0,
            front_deg: number(opts, "f3-eye-front", 75.0)?,
            rounds: count(opts, "f3-eye-rounds", 1)?,
            samples: count(opts, "f3-eye-samples", 6)?,
        },
        f4: None,
    })
}

pub fn f3_placement(opts: &Options, name: &str) -> Result<gp::F3, String> {
    let mut settings = f3_settings(opts)?;
    if name == "f4" {
        settings.f4 = Some(placement_f3::F4Settings {
            target_deg: number(opts, "f4-target", 10.0)?,
            passes: count(opts, "f4-passes", 12)?,
            step: number(opts, "f4-step", 0.25)?,
            gain: number(opts, "f4-gain", 0.5)?,
            fold_deg: number(opts, "f4-fold", 60.0)?,
            backoff_dense: match opts.get("f4-backoff").map(String::as_str).unwrap_or("dense") {
                "dense" => true,
                "subset" => false,
                other => return Err(format!("--f4-backoff must be dense or subset, got {other}")),
            },
            game_plane: match opts.get("f4-plane").map(String::as_str).unwrap_or("game") {
                "game" => true,
                "dense" => false,
                other => return Err(format!("--f4-plane must be game or dense, got {other}")),
            },
        });
    }
    let eyes = match opts.get("eyes") {
        Some(path) => gf_io::load_eyes(Path::new(path))?,
        None => Vec::new(),
    };
    Ok(gp::F3 { settings, eyes })
}

pub fn f5_placement(opts: &Options) -> Result<gp::F5, String> {
    let settings = placement_f5::F5Settings {
        tangential: number(opts, "f5-tangential", 0.01)?,
        smooth: number(opts, "f5-smooth", 0.1)?,
        membrane: number(opts, "f5-membrane", 0.1)?,
        max_move: number(opts, "f5-max-move", 3.0)? / 1000.0,
        uv_ring: count(opts, "f5-uv-ring", 2)?,
        subset_normals: match opts.get("f5-normals").map(String::as_str).unwrap_or("subset") {
            "subset" => true,
            "dense" => false,
            other => return Err(format!("--f5-normals must be subset or dense, got {other}")),
        },
        eye_clearance: number(opts, "f5-eye-clearance", 0.05)? / 1000.0,
        eye_depth: pair(opts, "f5-eye-depth", [0.3, 0.6])?.map(|x| x / 1000.0),
        eye_front_deg: number(opts, "f5-eye-front", 75.0)?,
    };
    if settings.eye_front_deg < 10.0 {
        return Err(format!("--f5-eye-front must be at least 10 (its fade starts 10 degrees inside it), got {}", settings.eye_front_deg));
    }
    let eyes = match opts.get("eyes") {
        Some(path) => gf_io::load_eyes(Path::new(path))?,
        None => Vec::new(),
    };
    Ok(gp::F5 { settings, eyes })
}

pub fn f6_placement(opts: &Options) -> Result<gp::F6, String> {
    let gp::F5 { settings: f5, eyes } = f5_placement(opts)?;
    let defaults = gp::f6_defaults(f5.clone());
    let f5 = placement_f5::F5Settings {
        membrane: number(opts, "f5-membrane", defaults.f5.membrane)?,
        smooth: number(opts, "f5-smooth", defaults.f5.smooth)?,
        ..f5
    };
    let settings = placement_f5::F6Settings {
        f5,
        sliver_deg: number(opts, "f6-sliver-deg", defaults.sliver_deg)?,
        sliver_rounds: count(opts, "f6-sliver-rounds", defaults.sliver_rounds)?,
        sliver_relax: number(opts, "f6-sliver-relax", defaults.sliver_relax)?,
        sliver_seam: number(opts, "f6-sliver-seam", defaults.sliver_seam)?,
        sliver_eye: pair(opts, "f6-sliver-eye", defaults.sliver_eye.map(|x| x * 1000.0))?.map(|x| x / 1000.0),
        ..defaults
    };
    if !(0.0..30.0).contains(&settings.sliver_deg)
        || !(settings.sliver_relax > 0.0 && settings.sliver_relax <= 1.0)
        || !(0.0..=1.0).contains(&settings.sliver_seam)
        || settings.sliver_eye[1] <= settings.sliver_eye[0]
    {
        return Err(format!(
            "--f6-sliver-deg must be in [0, 30), --f6-sliver-relax in (0, 1], --f6-sliver-seam in [0, 1] and --f6-sliver-eye increasing, got {}, {}, {}, {:?}",
            settings.sliver_deg, settings.sliver_relax, settings.sliver_seam, settings.sliver_eye
        ));
    }
    Ok(gp::F6 { settings, eyes })
}

pub fn collide_lids(opts: &Options) -> Result<Value, String> {
    only(opts, "collide-lids", &[COLLIDE_KEYS])?;
    let settings = lids::CollideSettings {
        clearance: number(opts, "clearance", 0.0)? / 1000.0,
        front_deg: number(opts, "front-deg", 75.0)?,
        smooth_iterations: count(opts, "smooth", 4)?,
    };
    let out = Path::new(need(opts, "out")?);
    let eyes = gf_io::load_eyes(Path::new(need(opts, "eyes")?))?;
    let dense = Dense::load(Path::new(need(opts, "dense")?))?;
    let positions = dense.positions_from(need(opts, "positions")?)?;
    let (moved, stats) = lids::collide_dense(&positions, &dense.triangles, &eyes, &settings);
    let (before, _) = lids::penetration_area(&positions, &dense.triangles, &eyes, settings.front_deg, 2);
    let (after, _) = lids::penetration_area(&moved, &dense.triangles, &eyes, settings.front_deg, 2);
    dense::write_points(out, &moved)?;
    Ok(json!({"ok": true, "command": "collide-lids", "out": out.display().to_string(), "settings": settings, "stats": stats,
              "inside_mm2_before": before * 1e6, "inside_mm2_after": after * 1e6}))
}
