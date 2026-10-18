use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{obj, shape};
use crate::error::{ApiError, Code};
use crate::photo;

const KEYS: [&str; 7] = ["front", "profile", "profile_side", "thresholds", "allow", "python", "workers"];

fn gate_props() -> Value {
    json!({
        "thresholds": {"type": "object", "additionalProperties": {"type": "number"},
                       "description": "overrides of the calibrated gate limits by name (photo.check reports every limit it used); a pose limit of 0 turns that axis off"},
        "allow": {"type": "array", "items": {"type": "string"},
                  "description": "check names (e.g. front.pitch) whose failure is recorded as allowed instead of stopping the run"}
    })
}

fn with_gate(schema: Value) -> Value {
    let mut schema = schema;
    if let (Some(props), Some(extra)) = (schema.get_mut("properties").and_then(Value::as_object_mut), gate_props().as_object()) {
        for (k, v) in extra {
            props.insert(k.clone(), v.clone());
        }
    }
    schema
}

pub fn check_input() -> Value {
    let mut props = gate_props();
    for (k, v) in [
        ("front", json!({"type": "string", "minLength": 1, "description": "front photo"})),
        ("profile", json!({"type": "string", "minLength": 1, "description": "profile photo"})),
        ("profile_side", json!({"type": "string", "enum": ["right", "left"], "description": "which side of the face the profile shows (default right)"})),
        ("run", json!({"type": "string", "minLength": 1, "description": "directory that receives gate.json (default a temporary directory)"})),
        ("python", json!({"type": "string", "description": "Python with numpy, opencv and mediapipe (else GAZEFORGE_PYTHON, else python)"})),
        ("workers", json!({"type": "string", "description": "directory of the worker scripts (else GAZEFORGE_WORKERS, else ./workers)"})),
        ("landmarker", json!({"type": "string", "description": "MediaPipe face landmarker .task model with blendshapes (else GAZEFORGE_FACE_LANDMARKER)"})),
    ] {
        props[k] = v;
    }
    obj(props, &[])
}

pub fn check_output() -> Value {
    shape(json!({
        "pass": {"type": "boolean"},
        "failures": {"type": "array", "items": {"type": "string"}},
        "allowed": {"type": "array", "items": {"type": "string"}},
        "photos": {"type": "object", "description": "per photo: pose (yaw, pitch, roll in degrees with a confidence) and checks (name, value, threshold, pass, reason)"},
        "thresholds": {"type": "object"}
    }))
}

fn scratch() -> Result<PathBuf, ApiError> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("gazeforge-photo-check-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?;
    Ok(dir)
}

fn run(input: &Value, dir: &Path) -> Result<Value, ApiError> {
    let mut args = json!({});
    for k in ["front", "profile", "profile_side", "thresholds", "allow"] {
        if let Some(v) = input.get(k) {
            args[k] = v.clone();
        }
    }
    photo::worker(input, "photo_gate.py", "check", args, dir)
}

pub fn check(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    if input.get("front").is_none() && input.get("profile").is_none() {
        return Err(ApiError::invalid("give front, profile or both"));
    }
    match input.get("run").and_then(Value::as_str) {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?;
            run(input, &dir)
        }
        None => {
            let dir = scratch()?;
            let out = run(input, &dir);
            let _ = std::fs::remove_dir_all(&dir);
            out
        }
    }
}

fn require(input: &Value) -> Result<Value, ApiError> {
    let dir = PathBuf::from(input.get("run").and_then(Value::as_str).unwrap_or(""));
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?;
    let started = Instant::now();
    let report = run(input, &dir)?;
    if report.get("pass") == Some(&Value::Bool(true)) {
        return Ok(report);
    }
    let failures: Vec<String> =
        report.get("failures").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let stages = json!([{"stage": "0 photo.gate", "seconds": started.elapsed().as_secs_f64(), "ok": false}]);
    let _ = std::fs::write(dir.join("stages.json"), serde_json::to_string_pretty(&json!({"stages": stages})).unwrap_or_default());
    Err(ApiError::new(Code::GateFailed, format!("the reference photos failed {} check(s): {}", failures.len(), failures.join("; ")))
        .details(json!({"stage": "0 photo.gate", "stages": stages, "gate": report}))
        .hint("retake the photos: head straight (no yaw, pitch or roll), one sharp, evenly lit face, eyes open, mouth closed, plain background; or name a check in allow to record an override"))
}

fn without_gate(input: &Value) -> Value {
    let mut inner = input.clone();
    if let Some(o) = inner.as_object_mut() {
        o.remove("thresholds");
        o.remove("allow");
    }
    inner
}

pub fn ingest_input() -> Value {
    with_gate(photo::ingest_input())
}

pub fn ingest(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let report = require(input)?;
    let mut out = photo::ingest(ctx, &without_gate(input))?;
    out["gate"] = report;
    Ok(out)
}

pub fn from_photos_input() -> Value {
    with_gate(photo::from_photos_input())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gated_inputs_accept_thresholds_and_allow() {
        for schema in [ingest_input(), from_photos_input(), check_input()] {
            assert!(schema.pointer("/properties/thresholds").is_some());
            assert!(schema.pointer("/properties/allow").is_some());
            assert_eq!(schema.get("additionalProperties"), Some(&Value::Bool(false)));
        }
        let errors = crate::schema::validate(&ingest_input(), &json!({"front": "a", "profile": "b", "run": "r", "thresholds": {"front_yaw_deg": 2}}));
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn check_needs_a_photo() {
        let ctx = Ctx { jobs_dir: PathBuf::from("."), exe: PathBuf::from("."), job_id: None };
        let err = check(&ctx, &json!({})).unwrap_err();
        assert_eq!(err.code, Code::InvalidInput);
    }

    #[test]
    fn without_gate_strips_only_gate_keys() {
        let inner = without_gate(&json!({"front": "a", "thresholds": {}, "allow": [], "run": "r"}));
        assert_eq!(inner, json!({"front": "a", "run": "r"}));
    }
}

pub fn from_photos(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let mut gate_in = json!({});
    for k in KEYS.iter().chain(["run", "landmarker"].iter()) {
        if let Some(v) = input.get(*k) {
            gate_in[*k] = v.clone();
        }
    }
    let report = require(&gate_in)?;
    let mut out = photo::from_photos(ctx, &without_gate(input))?;
    out["gate"] = json!({"pass": report.get("pass"), "allowed": report.get("allowed")});
    Ok(out)
}
