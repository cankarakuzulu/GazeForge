use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::commands::text;
use crate::error::{ApiError, Code};
use crate::process::{self, JobGuard};
use crate::{Ctx, call, check_input, find};

const TERMINAL: [&str; 3] = ["done", "failed", "cancelled"];
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn job_dir(jobs_dir: &Path, id: &str) -> Result<PathBuf, ApiError> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(ApiError::invalid(format!("{id:?} is not a job id")));
    }
    let dir = jobs_dir.join(id);
    if !dir.join("job.json").is_file() {
        return Err(ApiError::not_found(format!("no job {id} in {}", jobs_dir.display())).details(json!({"jobs_dir": jobs_dir.display().to_string()})));
    }
    Ok(dir)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let partial = path.with_extension("partial");
    std::fs::write(&partial, bytes).map_err(|e| format!("{}: {e}", partial.display()))?;
    for _ in 0..50 {
        if std::fs::rename(&partial, path).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_record(dir: &Path, record: &Value) -> Result<(), String> {
    write_atomic(&dir.join("job.json"), crate::text(record).as_bytes())
}

pub fn read_record(dir: &Path) -> Result<Value, ApiError> {
    let path = dir.join("job.json");
    let mut last = String::new();
    for _ in 0..50 {
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
            Ok(v) => return Ok(v),
            Err(e) => last = e,
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(ApiError::internal(format!("{}: {last}", path.display())))
}

fn terminal(record: &Value) -> bool {
    record.get("state").and_then(Value::as_str).is_some_and(|s| TERMINAL.contains(&s))
}

fn wait_for(dir: &Path, wait_s: f64) -> Result<Value, ApiError> {
    let deadline = Instant::now() + Duration::from_secs_f64(wait_s);
    loop {
        let record = read_record(dir)?;
        if terminal(&record) || Instant::now() >= deadline {
            return Ok(record);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

pub fn submit(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let name = text(input, "command");
    let inner = input.get("input").cloned().unwrap_or_else(|| json!({}));
    let def = find(name).ok_or_else(|| ApiError::not_found(format!("no command named {name}")).hint("commands.list names every command"))?;
    if def.name.starts_with("jobs.") {
        return Err(ApiError::invalid(format!("{name} cannot run as a job")));
    }
    check_input(def, &inner)?;
    let id = format!("j{}-{}-{}", now_ms(), std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let dir = ctx.jobs_dir.join(&id);
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::internal(format!("{}: {e}", dir.display())))?;
    let mut record = json!({"id": id, "command": name, "input": inner, "state": "queued", "submitted_ms": now_ms(),
                            "jobs_dir": ctx.jobs_dir.display().to_string()});
    write_record(&dir, &record).map_err(ApiError::internal)?;
    let mut supervisor = Command::new(&ctx.exe);
    supervisor.arg("__job-supervise").arg(&ctx.jobs_dir).arg(&id).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    match process::spawn_detached(&mut supervisor) {
        Ok(child) => {
            record["supervisor_pid"] = json!(child.id());
            Ok(record)
        }
        Err(e) => {
            record["state"] = json!("failed");
            record["error"] = ApiError::new(Code::WorkerFailed, format!("could not start the supervisor: {e}")).to_json();
            write_record(&dir, &record).map_err(ApiError::internal)?;
            Err(ApiError::new(Code::WorkerFailed, format!("could not start the supervisor: {e}")))
        }
    }
}

pub fn status(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let dir = job_dir(&ctx.jobs_dir, text(input, "id"))?;
    wait_for(&dir, input.get("wait_s").and_then(Value::as_f64).unwrap_or(0.0))
}

pub fn cancel(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let dir = job_dir(&ctx.jobs_dir, text(input, "id"))?;
    let record = read_record(&dir)?;
    if terminal(&record) {
        return Ok(record);
    }
    std::fs::write(dir.join("cancel"), b"cancel").map_err(|e| ApiError::internal(e.to_string()))?;
    let record = wait_for(&dir, input.get("wait_s").and_then(Value::as_f64).unwrap_or(10.0))?;
    if !terminal(&record) {
        return Err(ApiError::new(Code::WorkerFailed, "cancel was requested; the job has not stopped yet").details(record));
    }
    Ok(record)
}

pub fn slow(ctx: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let seconds = input.get("seconds").and_then(Value::as_f64).unwrap_or(0.0);
    let count = input.get("children").and_then(Value::as_u64).unwrap_or(0);
    let tag = ctx.job_id.clone().unwrap_or_default();
    let mut children = Vec::new();
    for _ in 0..count {
        let mut cmd = Command::new(&ctx.exe);
        cmd.arg("__sleep").arg(seconds.to_string()).arg(&tag).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        process::hide_window(&mut cmd);
        children.push(cmd.spawn().map_err(|e| ApiError::new(Code::WorkerFailed, format!("could not start a child: {e}")))?);
    }
    let started = Instant::now();
    while started.elapsed().as_secs_f64() < seconds {
        std::thread::sleep(Duration::from_millis(50));
    }
    for child in &mut children {
        let _ = child.wait();
    }
    Ok(json!({"slept_s": seconds, "children": count}))
}

pub fn sleep_child(seconds: f64) {
    std::thread::sleep(Duration::from_secs_f64(seconds.clamp(0.0, 600.0)));
}

pub fn supervise(jobs_dir: &Path, id: &str) -> Result<(), String> {
    let dir = job_dir(jobs_dir, id).map_err(|e| e.message)?;
    let mut record = read_record(&dir).map_err(|e| e.message)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut guard = JobGuard::new()?;
    let log = std::fs::File::create(dir.join("worker.log")).map_err(|e| e.to_string())?;
    let mut worker = Command::new(exe);
    worker.arg("__job-work").arg(jobs_dir).arg(id).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(log);
    guard.prepare(&mut worker);
    let mut child = worker.spawn().map_err(|e| e.to_string())?;
    guard.assign(&child)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(b"go\n").map_err(|e| e.to_string())?;
    }
    record["state"] = json!("running");
    record["started_ms"] = json!(now_ms());
    record["worker_pid"] = json!(child.id());
    record["supervisor_pid"] = json!(std::process::id());
    write_record(&dir, &record)?;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            guard.terminate();
            record["worker_processes_left"] = json!(guard.wait_empty(Duration::from_secs(5)));
            record["worker_exit"] = json!(status.code());
            match std::fs::read_to_string(dir.join("result.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
                Some(result) => {
                    let ok = result.get("ok") == Some(&Value::Bool(true));
                    record["state"] = json!(if ok { "done" } else { "failed" });
                    record["result"] = result;
                }
                None => {
                    record["state"] = json!("failed");
                    record["error"] =
                        ApiError::new(Code::WorkerFailed, format!("the worker exited ({:?}) without a result", status.code())).to_json();
                }
            }
            break;
        }
        if dir.join("cancel").exists() {
            guard.terminate();
            record["worker_processes_left"] = json!(guard.wait_empty(Duration::from_secs(5)));
            let _ = child.wait();
            record["state"] = json!("cancelled");
            record["error"] = ApiError::new(Code::Cancelled, "cancelled; the worker's process tree was killed").to_json();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    record["finished_ms"] = json!(now_ms());
    write_record(&dir, &record)
}

pub fn work(jobs_dir: &Path, id: &str) -> Result<(), String> {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
    if line.trim() != "go" {
        return Err("the supervisor did not release the worker".into());
    }
    let dir = job_dir(jobs_dir, id).map_err(|e| e.message)?;
    let record = read_record(&dir).map_err(|e| e.message)?;
    let ctx = Ctx {
        jobs_dir: jobs_dir.to_path_buf(),
        exe: std::env::current_exe().map_err(|e| e.to_string())?,
        job_id: Some(id.to_string()),
        projects: crate::project::default_root(),
    };
    let envelope = call(&ctx, text(&record, "command"), record.get("input").unwrap_or(&json!({})));
    write_atomic(&dir.join("result.json"), crate::text(&envelope).as_bytes())
}
