use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use gf_core::placement::{self as gp, DenseMesh, Placement, PlacementVersion};
use gf_geom::decimate::{self, Decimated, Settings};
use gf_geom::{lids, placement, placement_f2, placement_f3, placement_f5};
use gf_io::dense::{self, Dense};
use gf_io::{game_base, obj};
use serde_json::{Value, json};

mod data_cmds;
mod front;
mod http;
mod mcp;
mod mesh_cmds;
mod rig_cmds;

const USAGE: &str = "gazeforge <command> [--key value ...]
  build-base  --dense DIR --out DIR [--target 20000] [--shapes all|none|a,b] [--extra A.npy,B.npy] [--min-angle 4] [--max-angle 170]
              [--max-turn 45] [--boundary-weight 100] [--improve-below 20] [--flip-flat 15]
              [--weights W.npy (per dense vertex, > 0, multiplies the collapse cost)] [--lods 8000,3000 (nested, into OUT/lod_N)]
  export-obj  --dense DIR --base DIR --positions base|shape:NAME|FILE.npy --out FILE.obj [--placement f5|f6|subset|optimal|fit|f2|f3|f4|NAME/VERSION]
              [--tip-vertex 185894]
              optimal, fit, f2: [--max-move 3]
              fit, f2: [--fit-iterations 4] [--fit-smooth 0.1] [--fit-min-angle 5] [--fit-guard-steps 8] [--fit-repair-below 5] [--fit-trim 0]
              f2 only: [--f2-regular 1e-2] [--f2-max-turn 60] [--f2-turn-margin 2] [--f2-confident-spread 30] [--f2-confident-above 25]
              [--f2-angle-margin 0.5] [--f2-repair-rounds 8] [--f2-repair-max-step 0.4] [--f2-repair-max-step-confident 0.5]
              [--f2-uv-ring 2] [--f2-uvs surface|subset]
              f3 only: [--f3-regular 1e-3] [--f3-max-move 3] [--f3-smooth 0.1] [--f3-rounds 4] [--f3-relax-steps 24] [--f3-relax-step 0.5]
              [--f3-relax-angle 1,8] [--f3-relax-eye 3,8] [--f3-backoff-turn 25,55] [--f3-backoff-step 0.5] [--f3-uv-ring 2] [--eyes EYES.json]
              [--f3-eye-front 75] [--f3-eye-clearance 0.05] [--f3-eye-reach 2] [--f3-eye-depth 0.3,0.6] [--f3-eye-lift radial|gaze]
              [--f3-eye-rounds 1] [--f3-eye-samples 6]
              f4 = f3 + an angle pass: every f3 option, and [--f4-target 10] [--f4-passes 12] [--f4-step 0.25] [--f4-gain 0.5] [--f4-fold 60]
              [--f4-backoff dense|subset] [--f4-plane game|dense]
              f5 only: [--f5-tangential 0.01] [--f5-smooth 0.1] [--f5-membrane 0.1] [--f5-max-move 3]
              [--f5-uv-ring 2] [--f5-normals subset|dense] [--eyes EYES.json]
              [--f5-eye-front 75] [--f5-eye-clearance 0.05] [--f5-eye-depth 0.3,0.6]
              f6 = f5 + a continuous sliver nudge: every f5 option (f6 defaults --f5-membrane 0.05 --f5-smooth 0.05),
              and [--f6-sliver-deg 6] [--f6-sliver-rounds 32]
              [--f6-sliver-relax 0.5] [--f6-sliver-seam 0.25] [--f6-sliver-eye 4,8]
  collide-lids --dense DIR --positions base|shape:NAME|FILE.npy --eyes EYES.json --out FILE.npy [--clearance 0]
              [--front-deg 75] [--smooth 4]
  store import-pack --store DIR --dense DIR [--macro-base JSON|@FILE] [--pin-top Z_METRES]
  store import-catalog --store DIR --targets DIR --pack sha256:...
  store import-game --store DIR --base DIR --pack sha256:...
  store import-eyerig --store DIR --catalog sha256:... --makehuman MAKEHUMAN_DATA_DIR
  character create --store DIR --pack sha256:... --catalog sha256:... --out FILE [--game sha256:...] [--placement NAME/VERSION]
              [--name NAME] [--weights JSON|@FILE] [--macro MACRO_JSON [--sliders JSON|@FILE] [--macro-mode gf|mpfb] [--head-scale 1]]
              [--residual-from FACE.npy [--quantum-mm 0.005]]
  character validate --store DIR --spec FILE [--macro MACRO_JSON]
  character evaluate --store DIR --spec FILE --out DENSE.npy [--macro MACRO_JSON] [--precision f32|f64]
  macro weights --macro MACRO_JSON --sliders JSON|@FILE [--targets TARGETS_ROOT] [--mode gf|mpfb] [--floor 0.01|none]
  macro labels --macro MACRO_JSON --slider NAME --values VALUES.npy --out TABLE.npy [--mode gf|mpfb]
  export-glb  --store DIR --spec FILE --out FILE.glb [--macro MACRO_JSON] [--morphs ID,ID|@FILE.json] [--sparse auto|always|never]
              [--lod-screen 0.25,0.1] [the options of the spec's placement, e.g. --eyes EYES.json]
  gates shell --dense DIR --before base|shape:NAME|FILE.npy --after base|shape:NAME|FILE.npy [--fold-tolerance 10]
              [--inner-tolerance 15] [--rough-mm 0.02] [--rough-rel 0.05] [--rough-tolerance 200]
  gates game  --dense DIR --base DIR --face base|shape:NAME|FILE.npy --obj GAME.obj [--gates triq,uv,census,seams]
              [--tip-vertex 185894] [--seam-limit-mm 0.05]
  gates path  --dense DIR --base DIR --start base|shape:NAME|FILE.npy --delta FILE.npy [--steps 20] [--pop-limit 10]
              [--placement f3 and its options] [--plant STEP,GAME_VERTEX,MM (a planted forward jump, to prove the gate)]
  A gate that fails prints its report with \"ok\": false and exits non-zero.
JSON|@FILE: inline JSON, or @ and a path to a JSON file. MACRO_JSON is read from the user's MakeHuman/MPFB install.
An option the command does not read is refused; numbers must be finite and not negative; input positions must be finite.
The command registry (the same commands over every front door):
  commands                              list every command with its input schema
  call    COMMAND --input JSON|@FILE    run one command
  serve   [--port N]                    local HTTP API on 127.0.0.1: GET /v1/commands, POST /v1/<command>
  mcp                                   MCP server over stdio
Every command prints one JSON object on stdout; a non-zero exit code means it failed.";

const BUILD_KEYS: &[&str] = &[
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
const EXPORT_KEYS: &[&str] = &["dense", "base", "positions", "out", "placement", "tip-vertex"];
const OPTIMAL_KEYS: &[&str] = &["max-move"];
const FIT_KEYS: &[&str] = &["max-move", "fit-iterations", "fit-smooth", "fit-min-angle", "fit-guard-steps", "fit-repair-below", "fit-trim"];
const F2_KEYS: &[&str] = &[
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

const F3_KEYS: &[&str] = &[
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
const F4_KEYS: &[&str] = &["f4-target", "f4-passes", "f4-step", "f4-gain", "f4-fold", "f4-backoff", "f4-plane"];
const F5_KEYS: &[&str] = &[
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
const F6_KEYS: &[&str] = &["f6-sliver-deg", "f6-sliver-rounds", "f6-sliver-relax", "f6-sliver-seam", "f6-sliver-eye"];
const COLLIDE_KEYS: &[&str] = &["dense", "positions", "eyes", "out", "clearance", "front-deg", "smooth"];

pub(crate) type Options = HashMap<String, String>;

fn options(args: &[String]) -> Result<Options, String> {
    let mut out = HashMap::new();
    let mut iter = args.iter();
    while let Some(key) = iter.next() {
        let name = key.strip_prefix("--").ok_or_else(|| format!("expected --key, got {key}"))?;
        let value = iter.next().ok_or_else(|| format!("--{name} needs a value"))?;
        if out.insert(name.to_string(), value.clone()).is_some() {
            return Err(format!("--{name} is given more than once"));
        }
    }
    Ok(out)
}

pub(crate) fn only(opts: &Options, command: &str, allowed: &[&[&str]]) -> Result<(), String> {
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

pub(crate) fn need<'a>(opts: &'a Options, key: &str) -> Result<&'a str, String> {
    opts.get(key).map(String::as_str).ok_or_else(|| format!("missing --{key}"))
}

pub(crate) fn number(opts: &Options, key: &str, default: f64) -> Result<f64, String> {
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

pub(crate) fn count(opts: &Options, key: &str, default: usize) -> Result<usize, String> {
    let value = number(opts, key, default as f64)?;
    if value.fract() != 0.0 || value > u32::MAX as f64 {
        return Err(format!("--{key} must be a whole number, got {}", opts.get(key).map_or("", String::as_str)));
    }
    Ok(value as usize)
}

fn build_base(opts: &Options) -> Result<Value, String> {
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
    let started = std::time::Instant::now();
    let clock = || started.elapsed().as_secs_f64();
    let mut levels =
        decimate::decimate_levels(shapes, &dense.triangles, &dense.triangle_uvs, &settings, weights.as_deref(), &lods, Some(&clock));
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

fn lod_targets(opts: &Options, finest: usize) -> Result<Vec<usize>, String> {
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

pub(crate) fn placement_keys(name: &str) -> &'static [&'static [&'static str]] {
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

pub(crate) fn placer(opts: &Options, version: PlacementVersion) -> Result<Box<dyn Placement>, String> {
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

pub(crate) fn placer_with(opts: &Options, version: PlacementVersion, eyes: Option<Vec<gf_geom::lids::Eye>>) -> Result<Box<dyn Placement>, String> {
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

fn export_obj(opts: &Options) -> Result<Value, String> {
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
    let dense = Dense::load(Path::new(need(opts, "dense")?))?;
    let base = game_base::load(Path::new(need(opts, "base")?))?;
    let positions = dense.positions_from(need(opts, "positions")?)?;
    let tip = dense.nose_tip(&positions, tip_vertex)?;
    let placed = placer.place(&positions, &DenseMesh { triangles: &dense.triangles, triangle_uvs: &dense.triangle_uvs }, &base);
    match placed.triangle_uvs {
        Some(triangle_uvs) => {
            let character = Decimated {
                kept: base.kept.clone(),
                owner: base.owner.clone(),
                triangles: base.triangles.clone(),
                triangle_uvs,
                stats: Default::default(),
            };
            obj::write(out, &placed.game, &character)?;
        }
        None => obj::write(out, &placed.game, &base)?,
    }
    let measure = placement::measure(&positions, &dense.triangles, &base, &placed.game, tip);
    let mut value = json!({"ok": true, "command": "export-obj", "out": out.display().to_string(), "vertices": base.kept.len(),
                           "triangles": base.triangles.len(), "placement": version.name,
                           "placement_version": placer.version().to_string(), "measure": measure});
    for (k, v) in placed.report {
        value[k] = v;
    }
    Ok(value)
}

fn fit_settings(opts: &Options) -> Result<placement::FitSettings, String> {
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

fn f2_settings(opts: &Options, fit: &placement::FitSettings) -> Result<(placement_f2::F2Settings, String), String> {
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

fn pair(opts: &Options, key: &str, default: [f64; 2]) -> Result<[f64; 2], String> {
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

fn f3_settings(opts: &Options) -> Result<placement_f3::F3Settings, String> {
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

fn f3_placement(opts: &Options, name: &str) -> Result<gp::F3, String> {
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

fn f5_placement(opts: &Options) -> Result<gp::F5, String> {
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

fn f6_placement(opts: &Options) -> Result<gp::F6, String> {
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

fn collide_lids(opts: &Options) -> Result<Value, String> {
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

fn run(args: &[String]) -> Result<Value, String> {
    match args.first().map(String::as_str) {
        Some("build-base") => options(&args[1..]).and_then(|o| build_base(&o)),
        Some("export-obj") => options(&args[1..]).and_then(|o| export_obj(&o)),
        Some("collide-lids") => options(&args[1..]).and_then(|o| collide_lids(&o)),
        Some("export-glb") => options(&args[1..]).and_then(|o| mesh_cmds::export_glb(&o)),
        Some("gates") => match args.get(1).map(String::as_str) {
            Some("shell") => options(&args[2..]).and_then(|o| mesh_cmds::gates_shell(&o)),
            Some("game") => options(&args[2..]).and_then(|o| mesh_cmds::gates_game(&o)),
            Some("path") => options(&args[2..]).and_then(|o| mesh_cmds::gates_path(&o)),
            Some("facing") => options(&args[2..]).and_then(|o| mesh_cmds::gates_facing(&o)),
            Some("lids") => options(&args[2..]).and_then(|o| mesh_cmds::gates_lids(&o)),
            Some("rig") => options(&args[2..]).and_then(|o| rig_cmds::gates_rig(&o)),
            _ => Err(USAGE.to_string()),
        },
        Some(group @ ("store" | "character" | "macro")) => match args.get(1) {
            Some(verb) => options(&args[2..]).and_then(|o| data_cmds::run(group, verb, &o)),
            None => Err(USAGE.to_string()),
        },
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = front::dispatch(&args) {
        return code;
    }
    match run(&args) {
        Ok(value) => {
            println!("{value}");
            if value.get("ok") == Some(&Value::Bool(false)) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
        }
        Err(error) => {
            println!("{}", json!({"ok": false, "error": error}));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(pairs: &[&str]) -> Result<Options, String> {
        options(&pairs.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn export_error(pairs: &[&str]) -> String {
        let mut args = vec!["export-obj".to_string()];
        args.extend(pairs.iter().map(|s| s.to_string()));
        run(&args).unwrap_err()
    }

    #[test]
    fn unknown_flags_are_refused() {
        let e = export_error(&["--placement", "fit", "--max-mvoe", "2"]);
        assert!(e.contains("unknown option") && e.contains("--max-mvoe"), "{e}");
        let e = run(&["build-base".into(), "--dense".into(), "d".into(), "--targte".into(), "1".into()]).unwrap_err();
        assert!(e.contains("--targte"), "{e}");
    }

    #[test]
    fn flags_the_placement_does_not_read_are_refused() {
        assert!(export_error(&["--placement", "fit", "--f2-regular", "0.1"]).contains("--f2-regular"));
        assert!(export_error(&["--placement", "subset", "--max-move", "2"]).contains("--max-move"));
        assert!(export_error(&["--placement", "optimal", "--fit-iterations", "2"]).contains("--fit-iterations"));
    }

    #[test]
    fn known_flags_pass_the_key_check() {
        let o = opts(&["--placement", "f2", "--max-move", "3", "--fit-trim", "0", "--f2-uvs", "subset", "--tip-vertex", "7"]).unwrap();
        assert!(only(&o, "export-obj", &[EXPORT_KEYS, FIT_KEYS, F2_KEYS]).is_ok());
    }

    #[test]
    fn repeated_flags_are_refused() {
        assert!(opts(&["--max-move", "2", "--max-move", "3"]).unwrap_err().contains("more than once"));
    }

    #[test]
    fn negative_magnitudes_are_refused() {
        let e = export_error(&["--placement", "fit", "--max-move", "-1"]);
        assert!(e.contains("--max-move must not be negative"), "{e}");
        assert!(export_error(&["--placement", "f2", "--f2-uv-ring", "-2"]).contains("must not be negative"));
        let o = opts(&["--target", "-20000"]).unwrap();
        assert!(count(&o, "target", 20000).unwrap_err().contains("must not be negative"));
    }

    #[test]
    fn non_finite_and_fractional_numbers_are_refused() {
        for bad in ["NaN", "nan", "inf", "-inf", "infinity"] {
            let o = opts(&["--max-move", bad]).unwrap();
            assert!(number(&o, "max-move", 3.0).is_err(), "{bad} accepted");
        }
        let o = opts(&["--fit-iterations", "2.5"]).unwrap();
        assert!(count(&o, "fit-iterations", 4).unwrap_err().contains("whole number"));
    }

    #[test]
    fn valid_numbers_and_defaults_are_unchanged() {
        let o = opts(&["--max-move", "2.5", "--fit-iterations", "6", "--f2-regular", "1e-2"]).unwrap();
        assert_eq!(number(&o, "max-move", 3.0).unwrap(), 2.5);
        assert_eq!(count(&o, "fit-iterations", 4).unwrap(), 6);
        assert_eq!(number(&o, "f2-regular", 0.0).unwrap(), 0.01);
        assert_eq!(number(&o, "fit-smooth", 0.1).unwrap(), 0.1);
        assert_eq!(count(&o, "tip-vertex", dense::NOSE_TIP_VERTEX).unwrap(), 185_894);
    }

    #[test]
    fn the_designer_places_with_the_cli_default_settings() {
        let none = Options::new();
        let cli = serde_json::to_value(f3_settings(&none).unwrap()).unwrap();
        assert_eq!(cli, serde_json::to_value(gf_api::designer::f3_defaults()).unwrap());
        let f4 = serde_json::to_value(f3_placement(&none, "f4").unwrap().settings.f4).unwrap();
        assert_eq!(f4, serde_json::to_value(Some(gf_api::designer::f4_defaults())).unwrap());
        let f5 = serde_json::to_value(f5_placement(&none).unwrap().settings).unwrap();
        assert_eq!(f5, serde_json::to_value(gf_api::designer::f5_defaults()).unwrap());
    }
}
