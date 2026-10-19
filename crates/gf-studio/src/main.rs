mod app;
mod godot;
mod hud;
mod input;
mod keymap;
mod preview;
mod render;
mod script;
mod session;
mod shape;
mod viewer;
mod worker;
mod world;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

const USAGE: &str = "gazeforge-studio [--store DIR] [--spec FILE] [--macro FILE] [--keymap FILE] [--input sliders] [--godot-config FILE] [--bench-window SECONDS] [--present vsync|immediate]
  opens the native designer window; the core runs in-process; --input picks the shape input layer (default sliders); --godot-config names the Godot render mode's settings
gazeforge-studio --script STEPS.json --out DIR
  runs the steps against an offscreen GPU target with no window and prints one JSON report (also written to DIR/report.json)
gazeforge-studio --write-keymap FILE
  writes the default key map, to edit and pass back with --keymap (or place at the default location)
Every scripted or keymap run prints one JSON object on stdout; a non-zero exit code means it failed.";

const KEYS: &[&str] = &["store", "spec", "macro", "keymap", "input", "bench-window", "present", "script", "out", "write-keymap", "godot-config"];

fn options(args: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let key = args[i].strip_prefix("--").filter(|k| KEYS.contains(k)).ok_or_else(|| format!("unknown argument {}", args[i]))?;
        let value = args.get(i + 1).ok_or_else(|| format!("--{key} needs a value"))?;
        out.insert(key.to_string(), value.clone());
        i += 2;
    }
    Ok(out)
}

fn emit(v: Value, code: u8) -> ExitCode {
    println!("{v}");
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        return emit(json!({"ok": true, "usage": USAGE, "keymap_default_path": keymap::default_path().map(|p| p.display().to_string()), "godot_config_default_path": godot::default_path().map(|p| p.display().to_string())}), 0);
    }
    let o = match options(&args) {
        Ok(o) => o,
        Err(e) => return emit(json!({"ok": false, "error": e, "usage": USAGE}), 2),
    };
    if let Some(path) = o.get("write-keymap") {
        let text = serde_json::to_string_pretty(&keymap::defaults_json()).unwrap_or_default();
        return match std::fs::write(path, text) {
            Ok(()) => emit(json!({"ok": true, "written": path, "actions": keymap::ACTIONS.len()}), 0),
            Err(e) => emit(json!({"ok": false, "error": format!("{path}: {e}")}), 1),
        };
    }
    if let Some(script) = o.get("script") {
        let Some(out) = o.get("out") else { return emit(json!({"ok": false, "error": "--script needs --out DIR"}), 2) };
        return match script::run(script, out) {
            Ok(report) => emit(report, 0),
            Err((mut report, e)) => {
                if report.get("ok").is_none() {
                    report = json!({"ok": false, "error": e});
                }
                emit(report, 1)
            }
        };
    }
    let bench_seconds = match o.get("bench-window").map(|s| s.parse::<f64>()) {
        Some(Ok(s)) if s > 0.0 => Some(s),
        Some(_) => return emit(json!({"ok": false, "error": "--bench-window needs a positive number of seconds"}), 2),
        None => None,
    };
    let a = app::Args {
        store: o.get("store").cloned(),
        spec: o.get("spec").cloned(),
        macro_file: o.get("macro").cloned(),
        keymap: o.get("keymap").map(PathBuf::from),
        bench_seconds,
        present: o.get("present").cloned(),
        input: o.get("input").cloned(),
        godot_config: o.get("godot-config").map(PathBuf::from),
    };
    match app::run(a) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => emit(json!({"ok": false, "error": e}), 1),
    }
}
