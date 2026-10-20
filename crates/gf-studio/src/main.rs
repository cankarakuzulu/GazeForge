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
mod studio;
mod ui_script;
mod viewer;
mod worker;
mod world;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

const USAGE: &str = "gazeforge-studio [--store DIR] [--spec FILE] [--macro FILE] [--eyes FILE] [--keymap FILE] [--input sliders] [--godot-config FILE] [--bench-window SECONDS | --bench-idle SECONDS] [--present vsync|immediate]
  opens the native designer window; the core runs in-process; --input picks the shape input layer (default sliders); --godot-config names the Godot render mode's settings
gazeforge-studio --script STEPS.json --out DIR
  runs the steps against an offscreen GPU target with no window and prints one JSON report (also written to DIR/report.json)
gazeforge-studio --write-keymap FILE
  writes the default key map, to edit and pass back with --keymap (or place at the default location)
Every scripted or keymap run prints one JSON object on stdout; a non-zero exit code means it failed.";

const KEYS: &[&str] = &["store", "spec", "macro", "eyes", "keymap", "input", "bench-window", "present", "script", "out", "write-keymap", "godot-config", "bench-idle"];

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

struct StderrLog;

static STDERR_LOG: StderrLog = StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::max_level()
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("[{} {}] {}", r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

fn install_log() {
    let Some(level) = std::env::var("GAZEFORGE_LOG").ok().and_then(|v| v.parse::<log::LevelFilter>().ok()) else { return };
    if log::set_logger(&STDERR_LOG).is_ok() {
        log::set_max_level(level);
    }
}

fn main() -> ExitCode {
    install_log();
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
    let mut bench = None;
    for (key, idle) in [("bench-window", false), ("bench-idle", true)] {
        match o.get(key).map(|s| s.parse::<f64>()) {
            Some(Ok(s)) if s > 0.0 && bench.is_none() => bench = Some((s, idle)),
            Some(Ok(s)) if s > 0.0 => return emit(json!({"ok": false, "error": "--bench-window and --bench-idle exclude each other"}), 2),
            Some(_) => return emit(json!({"ok": false, "error": format!("--{key} needs a positive number of seconds")}), 2),
            None => {}
        }
    }
    let a = app::Args {
        store: o.get("store").cloned(),
        spec: o.get("spec").cloned(),
        macro_file: o.get("macro").cloned(),
        eyes: o.get("eyes").cloned(),
        keymap: o.get("keymap").map(PathBuf::from),
        bench,
        present: o.get("present").cloned(),
        input: o.get("input").cloned(),
        godot_config: o.get("godot-config").map(PathBuf::from),
    };
    match app::run(a) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => emit(json!({"ok": false, "error": e}), 1),
    }
}
