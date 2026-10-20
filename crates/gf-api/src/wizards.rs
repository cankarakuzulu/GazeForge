use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::commands::{obj, shape, text};
use crate::error::ApiError;
use crate::{Ctx, Kind, check_input, find, jobs, project};

pub const FORMAT: &str = "gazeforge.wizard-run/1";

pub fn definitions() -> Vec<Value> {
    vec![export()]
}

fn export() -> Value {
    json!({
        "id": "export",
        "title": "Export a character",
        "group": "Export",
        "developer": false,
        "summary": "A character spec to a glb (and optionally its finest level as an obj), checked and opened back in the viewport.",
        "steps": [
            {"id": "character", "title": "Character", "command": null,
             "help": "The character to export: the spec file the designer saved (the current character by default) and its store.",
             "fields": [
                 {"name": "store", "label": "store", "widget": "folder", "bind": "$run.store", "common": true},
                 {"name": "spec", "label": "character spec", "widget": "file_open", "ext": ".gfchar.json", "bind": "$run.spec", "common": true},
             ],
             "shows": []},
            {"id": "export", "title": "Export", "command": "export.glb",
             "help": "Where to write the glb and what goes in it. Everything under advanced keeps its default.",
             "fields": [
                 {"name": "store", "bind": "$character.input.store", "hidden": true},
                 {"name": "spec", "bind": "$character.input.spec", "hidden": true},
                 {"name": "out", "label": "glb file", "widget": "file_save", "ext": ".glb", "bind": "$run.out", "common": true},
                 {"name": "lod0_out", "label": "also an obj of the finest level (file stem)", "widget": "file_save", "ext": "", "common": true},
                 {"name": "morphs", "label": "morph targets", "widget": "targets"},
                 {"name": "eyes", "label": "eyes file", "widget": "file_open", "ext": ".json"},
                 {"name": "body", "label": "body pack", "widget": "folder"},
                 {"name": "lids", "label": "eyelid fitting"},
                 {"name": "rig", "label": "face rig"},
                 {"name": "sparse", "label": "sparse morphs"},
                 {"name": "lod_screen", "label": "LOD switch screen sizes"},
             ],
             "shows": [{"kind": "report"}, {"kind": "glb", "from": "$export.result.out"}]},
            {"id": "check", "title": "Check", "command": "gates.facing", "optional": true,
             "help": "Every triangle of the exported file faces the way its vertex normals say, in the neutral and in each morph state.",
             "fields": [
                 {"name": "glb", "bind": "$export.result.out", "hidden": true},
                 {"name": "states", "label": "states", "bind": "=all", "common": true},
             ],
             "shows": [{"kind": "report"}]},
        ],
    })
}

pub fn list_input() -> Value {
    obj(json!({"developer": {"type": "boolean", "description": "include the developer wizards (default false)"}}), &[])
}

pub fn list_output() -> Value {
    shape(json!({"wizards": {"type": "array", "description": "id, title, group, developer, summary, steps"}}))
}

pub fn list(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let developer = input.get("developer").and_then(Value::as_bool).unwrap_or(false);
    let wizards: Vec<Value> = definitions()
        .into_iter()
        .filter(|w| developer || w["developer"] != Value::Bool(true))
        .map(|w| json!({"id": w["id"], "title": w["title"], "group": w["group"], "developer": w["developer"], "summary": w["summary"],
                        "steps": w["steps"].as_array().map_or(0, Vec::len)}))
        .collect();
    Ok(json!({"wizards": wizards}))
}

fn definition(id: &str) -> Result<Value, ApiError> {
    definitions().into_iter().find(|w| w["id"] == id).ok_or_else(|| ApiError::not_found(format!("no wizard named {id:?}")).hint("wizards.list names every wizard"))
}

pub fn get_input() -> Value {
    obj(json!({"id": {"type": "string", "minLength": 1, "description": "a wizard id, as wizards.list prints it"}}), &["id"])
}

pub fn get_output() -> Value {
    shape(json!({"id": {"type": "string"}, "steps": {"type": "array", "description": "each step with its command, kind, input schema, fields and what it shows"}}))
}

pub fn get(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let mut w = definition(text(input, "id"))?;
    for step in w["steps"].as_array_mut().into_iter().flatten() {
        if let Some(def) = step["command"].as_str().and_then(find) {
            step["kind"] = json!(def.kind.as_str());
            step["input_schema"] = (def.input)();
            step["summary"] = json!(def.summary);
        }
    }
    Ok(w)
}

fn runs_root(ctx: &Ctx, input: &Value) -> PathBuf {
    let root = input.get("root").and_then(Value::as_str).filter(|r| !r.is_empty()).map_or_else(|| ctx.projects.clone(), PathBuf::from);
    root.join("runs")
}

fn run_dir(ctx: &Ctx, input: &Value) -> Result<PathBuf, ApiError> {
    let run = text(input, "run");
    let ok = run.split('/').count() == 2 && run.split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) && !p.starts_with('.'));
    if !ok {
        return Err(ApiError::invalid(format!("{run:?} is not a wizard run id (PROJECT/WIZARD-N)")));
    }
    let dir = runs_root(ctx, input).join(run);
    if !dir.join("run.json").is_file() {
        return Err(ApiError::not_found(format!("no wizard run {run}")));
    }
    Ok(dir)
}

fn read_json(path: &Path) -> Option<Value> {
    std::fs::read_to_string(path).ok().and_then(|t| crate::exactjson::record(&t).ok())
}

fn write_json(path: &Path, value: &Value) -> Result<(), ApiError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?;
    }
    project::write_atomic(path, crate::text(value).as_bytes())
}

fn run_prop() -> Value {
    json!({"type": "string", "minLength": 3, "description": "the run id wizards.start returned (PROJECT/WIZARD-N)"})
}

fn root_prop() -> Value {
    json!({"type": "string", "minLength": 1, "description": "projects root (default: the server's projects folder); runs live in ROOT/runs"})
}

pub fn start_input() -> Value {
    obj(json!({"wizard": {"type": "string", "minLength": 1}, "project": {"type": "string", "minLength": 1, "description": "the project the run belongs to (default untitled)"},
               "values": {"type": "object", "description": "run values the steps bind as $run.NAME (for example store, spec, out)"}, "root": root_prop()}),
        &["wizard"])
}

pub fn state_input() -> Value {
    obj(json!({"run": run_prop(), "root": root_prop()}), &["run"])
}

pub fn step_input() -> Value {
    obj(json!({"run": run_prop(), "step": {"type": "string", "minLength": 1}, "input": {"type": "object", "description": "the step's fields; hidden and bound fields not given take their bound values"},
               "root": root_prop()}),
        &["run", "step"])
}

pub fn run_output() -> Value {
    shape(json!({"run": {"type": "string"}, "wizard": {"type": "string"}, "values": {"type": "object"},
                 "steps": {"type": "array", "description": "per step: id, state (open, running, done, failed, stale), the proposed input, the input given, the result or error, the job id"}}))
}

pub fn start(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let w = definition(text(input, "wizard"))?;
    let name = input.get("project").and_then(Value::as_str).unwrap_or(project::UNTITLED);
    project::check_name(name, true)?;
    let base = runs_root(ctx, input).join(name);
    std::fs::create_dir_all(&base).map_err(|e| ApiError::internal(format!("{}: {e}", base.display())))?;
    let id = w["id"].as_str().unwrap_or_default();
    let n = (1..).find(|n| !base.join(format!("{id}-{n}")).exists()).unwrap_or(1);
    let run = format!("{name}/{id}-{n}");
    let dir = base.join(format!("{id}-{n}"));
    write_json(&dir.join("run.json"), &json!({"format": FORMAT, "wizard": id, "run": run, "values": input.get("values").cloned().unwrap_or_else(|| json!({})), "started": project::now_utc()}))?;
    state_of(ctx, &dir)
}

pub fn state(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    state_of(ctx, &run_dir(ctx, input)?)
}

fn lookup(path: &str, run: &Value, steps: &Map<String, Value>) -> Option<Value> {
    let mut parts = path.split('.');
    let head = parts.next()?;
    let mut at = if head == "run" { run["values"].clone() } else { steps.get(head)?.clone() };
    for p in parts {
        at = at.get(p)?.clone();
    }
    (!at.is_null()).then_some(at)
}

fn bound(field: &Value, run: &Value, steps: &Map<String, Value>) -> Option<Value> {
    let b = field["bind"].as_str()?;
    if let Some(literal) = b.strip_prefix('=') {
        return Some(json!(literal));
    }
    lookup(b.strip_prefix('$')?, run, steps)
}

fn step_record(dir: &Path, id: &str) -> Value {
    let at = dir.join(id);
    let mut r = json!({"input": read_json(&at.join("input.json")), "job": read_json(&at.join("job.json")).map(|j| j["id"].clone())});
    if let Some(out) = read_json(&at.join("output.json")) {
        r["result"] = out["result"].clone();
        r["error"] = out["error"].clone();
    }
    r["stale"] = json!(at.join("stale").exists());
    r
}

fn finish_job(ctx: &Ctx, dir: &Path, id: &str) -> Result<(), ApiError> {
    let at = dir.join(id);
    let Some(job) = read_json(&at.join("job.json")) else { return Ok(()) };
    if at.join("output.json").exists() {
        return Ok(());
    }
    let jctx = Ctx { jobs_dir: PathBuf::from(job["jobs_dir"].as_str().unwrap_or_default()), ..ctx.clone() };
    let record = jobs::status(&jctx, &json!({"id": job["id"]}))?;
    match record["state"].as_str() {
        Some("done") | Some("failed") | Some("cancelled") => {
            let envelope = record.get("result").cloned().unwrap_or_else(|| json!({"ok": false, "error": record["error"]}));
            write_json(&at.join("output.json"), &json!({"result": envelope.get("result"), "error": envelope.get("error"), "job_state": record["state"]}))
        }
        _ => Ok(()),
    }
}

fn state_of(ctx: &Ctx, dir: &Path) -> Result<Value, ApiError> {
    let run = read_json(&dir.join("run.json")).ok_or_else(|| ApiError::internal(format!("{}: the run record does not read", dir.display())))?;
    let w = definition(run["wizard"].as_str().unwrap_or_default())?;
    let mut done: Map<String, Value> = Map::new();
    let mut rows = Vec::new();
    for step in w["steps"].as_array().into_iter().flatten() {
        let id = step["id"].as_str().unwrap_or_default();
        finish_job(ctx, dir, id)?;
        let mut rec = step_record(dir, id);
        let mut proposed = Map::new();
        for f in step["fields"].as_array().into_iter().flatten() {
            if let Some(v) = bound(f, &run, &done) {
                proposed.insert(f["name"].as_str().unwrap_or_default().to_string(), v);
            }
        }
        let st = if rec["stale"] == Value::Bool(true) {
            "stale"
        } else if !rec["error"].is_null() {
            "failed"
        } else if rec.get("result").is_some_and(|r| !r.is_null()) || (step["command"].is_null() && !rec["input"].is_null()) {
            "done"
        } else if !rec["job"].is_null() {
            "running"
        } else {
            "open"
        };
        rec["id"] = json!(id);
        rec["state"] = json!(st);
        rec["proposed"] = Value::Object(proposed);
        done.insert(id.to_string(), json!({"input": rec["input"], "result": rec.get("result").cloned().unwrap_or(Value::Null)}));
        rows.push(rec);
    }
    Ok(json!({"run": run["run"], "wizard": run["wizard"], "values": run["values"], "steps": rows}))
}

pub fn step(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let dir = run_dir(ctx, input)?;
    let run = read_json(&dir.join("run.json")).ok_or_else(|| ApiError::internal("the run record does not read"))?;
    let w = definition(run["wizard"].as_str().unwrap_or_default())?;
    let steps = w["steps"].as_array().cloned().unwrap_or_default();
    let id = text(input, "step");
    let at = steps.iter().position(|s| s["id"] == id).ok_or_else(|| ApiError::not_found(format!("wizard {} has no step {id:?}", run["wizard"].as_str().unwrap_or(""))))?;
    let state = state_of(ctx, &dir)?;
    if let Some(open) = state["steps"].as_array().into_iter().flatten().take(at).find(|s| !matches!(s["state"].as_str(), Some("done")) && !steps.iter().any(|d| d["id"] == s["id"] && d["optional"] == Value::Bool(true))) {
        return Err(ApiError::invalid(format!("step {id} needs step {} first ({})", open["id"].as_str().unwrap_or(""), open["state"].as_str().unwrap_or(""))));
    }
    let def = &steps[at];
    let mut given = input.get("input").and_then(Value::as_object).cloned().unwrap_or_default();
    let proposed = state["steps"][at]["proposed"].as_object().cloned().unwrap_or_default();
    for f in def["fields"].as_array().into_iter().flatten() {
        let name = f["name"].as_str().unwrap_or_default();
        if let Some(v) = proposed.get(name).filter(|_| !given.contains_key(name) || f["hidden"] == Value::Bool(true)) {
            given.insert(name.to_string(), v.clone());
        }
    }
    let given = Value::Object(given);
    let here = dir.join(id);
    let _ = std::fs::remove_dir_all(&here);
    for later in &steps[at + 1..] {
        let d = dir.join(later["id"].as_str().unwrap_or_default());
        if d.join("input.json").exists() {
            write_json(&d.join("stale"), &json!({"because": id}))?;
        }
    }
    let Some(name) = def["command"].as_str() else {
        write_json(&here.join("input.json"), &given)?;
        return state_of(ctx, &dir);
    };
    let cmd = find(name).ok_or_else(|| ApiError::internal(format!("wizard step {id} names an unknown command {name}")))?;
    check_input(cmd, &given)?;
    write_json(&here.join("input.json"), &given)?;
    if cmd.kind == Kind::Job {
        let record = jobs::submit(ctx, &json!({"command": name, "input": given}))?;
        write_json(&here.join("job.json"), &json!({"id": record["id"], "jobs_dir": ctx.jobs_dir.display().to_string()}))?;
    } else {
        let outcome = (cmd.run)(ctx, &given);
        let out = match outcome {
            Ok(r) => json!({"result": r, "error": null}),
            Err(e) => json!({"result": null, "error": e.to_json()}),
        };
        write_json(&here.join("output.json"), &out)?;
    }
    state_of(ctx, &dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(name: &str) -> Value {
        (find(name).unwrap().input)()["properties"].clone()
    }

    #[test]
    fn every_definition_names_real_commands_fields_and_earlier_bindings() {
        for w in definitions() {
            let mut seen: Vec<String> = Vec::new();
            for step in w["steps"].as_array().unwrap() {
                let id = step["id"].as_str().unwrap();
                let schema = step["command"].as_str().map(|c| {
                    assert!(find(c).is_some(), "{} names {c}", w["id"]);
                    props(c)
                });
                for f in step["fields"].as_array().unwrap() {
                    let name = f["name"].as_str().unwrap();
                    if let Some(p) = &schema {
                        assert!(p.get(name).is_some(), "{}.{id}.{name} is not an input of {}", w["id"], step["command"]);
                    }
                    if let Some(b) = f["bind"].as_str().and_then(|b| b.strip_prefix('$')) {
                        let head = b.split('.').next().unwrap();
                        assert!(head == "run" || seen.iter().any(|s| s == head), "{}.{id}.{name} binds {b} before that step", w["id"]);
                    }
                }
                seen.push(id.to_string());
            }
        }
    }

    #[test]
    fn a_run_steps_through_bound_values_and_marks_later_steps_stale() {
        let root = std::env::temp_dir().join(format!("gf-wizard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ctx = Ctx { jobs_dir: root.join("jobs"), exe: PathBuf::from("x"), job_id: None, projects: root.clone() };
        let s = start(&ctx, &json!({"wizard": "export", "project": "demo", "values": {"store": "S", "spec": "C.gfchar.json", "out": "o.glb"}})).unwrap();
        assert_eq!(s["run"], "demo/export-1");
        assert_eq!(s["steps"][0]["proposed"]["store"], "S");
        assert!(step(&ctx, &json!({"run": "demo/export-1", "step": "export", "input": {}})).unwrap_err().message.contains("needs step character"));
        let s = step(&ctx, &json!({"run": "demo/export-1", "step": "character", "input": {}})).unwrap();
        assert_eq!(s["steps"][0]["state"], "done");
        assert_eq!(s["steps"][1]["proposed"]["spec"], "C.gfchar.json");
        assert_eq!(s["steps"][1]["proposed"]["out"], "o.glb");
        let s = step(&ctx, &json!({"run": "demo/export-1", "step": "character", "input": {"spec": "D.gfchar.json"}})).unwrap();
        assert_eq!(s["steps"][1]["proposed"]["spec"], "D.gfchar.json");
        assert_eq!(s["steps"][2]["proposed"]["states"], "all");
        assert_eq!(start(&ctx, &json!({"wizard": "export", "project": "demo"})).unwrap()["run"], "demo/export-2");
        assert!(state(&ctx, &json!({"run": "../x"})).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }
}
