use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use gf_core::decimate::{self, Settings};
use gf_core::dense::Dense;
use gf_core::decimate::Decimated;
use gf_core::{game_base, obj, placement, placement_f2};
use serde_json::{Value, json};

const USAGE: &str = "gazeforge <command> [--key value ...]
  build-base  --dense DIR --out DIR [--target 20000] [--shapes all|none|a,b] [--extra A.npy,B.npy] [--min-angle 4] [--max-angle 170]
              [--max-turn 45] [--boundary-weight 100] [--improve-below 20] [--flip-flat 15]
  export-obj  --dense DIR --base DIR --positions base|shape:NAME|FILE.npy --out FILE.obj [--placement optimal|fit|f2|subset] [--max-move 3]
              [--fit-iterations 4] [--fit-smooth 0.1] [--fit-min-angle 5] [--fit-guard-steps 8] [--fit-repair-below 5] [--fit-trim 0]
              f2 only: [--f2-regular 1e-2] [--f2-max-turn 60] [--f2-turn-margin 2] [--f2-confident-spread 30] [--f2-confident-above 25]
              [--f2-angle-margin 0.5] [--f2-repair-rounds 8] [--f2-repair-max-step 0.4] [--f2-repair-max-step-confident 0.5]
              [--f2-uv-ring 2] [--f2-uvs surface|subset]
Every command prints one JSON object on stdout; a non-zero exit code means it failed.";

fn options(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    let mut iter = args.iter();
    while let Some(key) = iter.next() {
        let name = key.strip_prefix("--").ok_or_else(|| format!("expected --key, got {key}"))?;
        let value = iter.next().ok_or_else(|| format!("--{name} needs a value"))?;
        out.insert(name.to_string(), value.clone());
    }
    Ok(out)
}

fn need<'a>(opts: &'a HashMap<String, String>, key: &str) -> Result<&'a str, String> {
    opts.get(key).map(String::as_str).ok_or_else(|| format!("missing --{key}"))
}

fn number(opts: &HashMap<String, String>, key: &str, default: f64) -> Result<f64, String> {
    opts.get(key).map_or(Ok(default), |v| v.parse().map_err(|_| format!("--{key} is not a number: {v}")))
}

fn build_base(opts: &HashMap<String, String>) -> Result<Value, String> {
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
    let settings = Settings {
        target_vertices: number(opts, "target", 20000.0)? as usize,
        min_angle_deg: number(opts, "min-angle", 4.0)?,
        max_angle_deg: number(opts, "max-angle", 170.0)?,
        max_normal_turn_deg: number(opts, "max-turn", 45.0)?,
        boundary_weight: number(opts, "boundary-weight", 100.0)?,
        improve_below_deg: number(opts, "improve-below", 20.0)?,
        flip_flat_deg: number(opts, "flip-flat", 15.0)?,
    };
    let result = decimate::decimate(shapes, &dense.triangles, &dense.triangle_uvs, &settings);
    let out = Path::new(need(opts, "out")?);
    game_base::save(out, &result)?;
    Ok(json!({"ok": true, "command": "build-base", "out": out.display().to_string(), "shapes": names, "extra": extra,
              "settings": settings, "stats": result.stats}))
}

fn export_obj(opts: &HashMap<String, String>) -> Result<Value, String> {
    let dense = Dense::load(Path::new(need(opts, "dense")?))?;
    let base = game_base::load(Path::new(need(opts, "base")?))?;
    let positions = dense.positions_from(need(opts, "positions")?)?;
    let out = Path::new(need(opts, "out")?);
    let placement = opts.get("placement").map(String::as_str).unwrap_or("optimal");
    let (game, stats) = match placement {
        "subset" => (base.kept.iter().map(|&d| positions[d as usize]).collect::<Vec<_>>(), None),
        "optimal" => {
            let (game, stats) = placement::optimal(&positions, &dense.triangles, &base, number(opts, "max-move", 3.0)? / 1000.0);
            (game, Some(stats))
        }
        "fit" => {
            let (game, stats) = placement::fit(&positions, &dense.triangles, &base, &fit_settings(opts)?);
            (game, Some(stats))
        }
        "f2" => return export_f2(opts, &dense, &base, &positions, out),
        other => return Err(format!("--placement must be optimal, fit, f2 or subset, got {other}")),
    };
    obj::write(out, &game, &base)?;
    let measure = placement::measure(&positions, &dense.triangles, &base, &game);
    Ok(json!({"ok": true, "command": "export-obj", "out": out.display().to_string(), "vertices": base.kept.len(),
              "triangles": base.triangles.len(), "placement": placement, "placement_stats": stats, "measure": measure}))
}

fn fit_settings(opts: &HashMap<String, String>) -> Result<placement::FitSettings, String> {
    Ok(placement::FitSettings {
        iterations: number(opts, "fit-iterations", 4.0)? as usize,
        smooth: number(opts, "fit-smooth", 0.1)?,
        max_move: number(opts, "max-move", 3.0)? / 1000.0,
        min_angle_deg: number(opts, "fit-min-angle", 5.0)?,
        guard_steps: number(opts, "fit-guard-steps", 8.0)? as usize,
        repair_below_deg: number(opts, "fit-repair-below", 5.0)?,
        trim: number(opts, "fit-trim", 0.0)? / 1000.0,
    })
}

fn export_f2(opts: &HashMap<String, String>, dense: &Dense, base: &Decimated, positions: &[[f64; 3]], out: &Path) -> Result<Value, String> {
    let settings = placement_f2::F2Settings {
        fit: fit_settings(opts)?,
        regular: number(opts, "f2-regular", 1e-2)?,
        max_turn_deg: number(opts, "f2-max-turn", 60.0)?,
        turn_margin_deg: number(opts, "f2-turn-margin", 2.0)?,
        confident_spread_deg: number(opts, "f2-confident-spread", 30.0)?,
        confident_above_deg: number(opts, "f2-confident-above", 25.0)?,
        angle_margin_deg: number(opts, "f2-angle-margin", 0.5)?,
        repair_rounds: number(opts, "f2-repair-rounds", 8.0)? as usize,
        repair_max_step: number(opts, "f2-repair-max-step", 0.4)? / 1000.0,
        repair_max_step_confident: number(opts, "f2-repair-max-step-confident", 0.5)? / 1000.0,
        uv_ring: number(opts, "f2-uv-ring", 2.0)? as usize,
    };
    let uv_mode = opts.get("f2-uvs").map(String::as_str).unwrap_or("surface");
    if uv_mode != "surface" && uv_mode != "subset" {
        return Err(format!("--f2-uvs must be surface or subset, got {uv_mode}"));
    }
    let (game, uvs, stats) = placement_f2::place(positions, &dense.triangles, &dense.triangle_uvs, base, &settings);
    let triangle_uvs = if uv_mode == "surface" { uvs } else { base.triangle_uvs.clone() };
    let character = Decimated {
        kept: base.kept.clone(),
        owner: base.owner.clone(),
        triangles: base.triangles.clone(),
        triangle_uvs,
        stats: Default::default(),
    };
    obj::write(out, &game, &character)?;
    let measure = placement::measure(positions, &dense.triangles, base, &game);
    Ok(json!({"ok": true, "command": "export-obj", "out": out.display().to_string(), "vertices": base.kept.len(),
              "triangles": base.triangles.len(), "placement": "f2", "uvs": uv_mode, "settings": settings,
              "placement_stats": stats, "measure": measure}))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("build-base") => options(&args[1..]).and_then(|o| build_base(&o)),
        Some("export-obj") => options(&args[1..]).and_then(|o| export_obj(&o)),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("{}", json!({"ok": false, "error": error}));
            ExitCode::FAILURE
        }
    }
}
