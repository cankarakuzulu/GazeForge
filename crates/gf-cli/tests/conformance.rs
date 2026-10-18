use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use ndarray::{Array1, arr2};
use ndarray_npy::write_npy;
use serde_json::{Value, json};

const EXE: &str = env!("CARGO_BIN_EXE_gazeforge");

fn argv(args: &[&str]) -> Value {
    let output = Command::new(EXE).args(args).output().unwrap();
    assert!(output.status.success(), "{args:?}: {}", String::from_utf8_lossy(&output.stdout));
    serde_json::from_slice(&output.stdout).unwrap()
}

fn fixture(name: &str) -> (PathBuf, String, String) {
    let root = std::env::temp_dir().join(format!("gazeforge-conf-{name}-{}", std::process::id()));
    let dense = root.join("dense");
    let targets = root.join("targets");
    std::fs::create_dir_all(&dense).unwrap();
    std::fs::create_dir_all(targets.join("nose")).unwrap();
    std::fs::write(dense.join("manifest.json"), r#"{"vertices": 4, "faces": 2, "shapes": []}"#).unwrap();
    write_npy(dense.join("positions.npy"), &arr2(&[[0.0f32, 0.0, 0.0], [0.01, 0.0, 0.0], [0.01, 0.0, 0.01], [0.0, 0.0, 0.01]])).unwrap();
    write_npy(dense.join("face_sizes.npy"), &Array1::from(vec![3u32, 3])).unwrap();
    write_npy(dense.join("corner_vertices.npy"), &Array1::from(vec![0u32, 1, 2, 0, 2, 3])).unwrap();
    write_npy(dense.join("corner_uvs.npy"), &arr2(&[[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0], [1.0, 1.0], [0.0, 1.0]])).unwrap();
    std::fs::write(
        targets.join("catalog.json"),
        r#"{"meta": {}, "not_on_head": {}, "targets": {"nose/a": {"name": "a", "category": "nose", "file": "nose/a.npz", "max_mm": 2.0}}}"#,
    )
    .unwrap();
    let mut npz = ndarray_npy::NpzWriter::new(std::fs::File::create(targets.join("nose/a.npz")).unwrap());
    npz.add_array("indices", &Array1::from(vec![1u32, 2])).unwrap();
    npz.add_array("deltas", &arr2(&[[0.002f32, 0.0, 0.0], [0.0, 0.0, -0.002]])).unwrap();
    npz.finish().unwrap();
    let store = root.join("store");
    let s = store.to_str().unwrap();
    let pack = argv(&["store", "import-pack", "--store", s, "--dense", dense.to_str().unwrap()])["pack"].as_str().unwrap().to_string();
    let catalog = argv(&["store", "import-catalog", "--store", s, "--targets", targets.to_str().unwrap(), "--pack", &pack])["catalog"]
        .as_str()
        .unwrap()
        .to_string();
    (root, pack, catalog)
}

struct Surfaces {
    server: Child,
    port: u16,
    mcp: Child,
    mcp_in: ChildStdin,
    mcp_out: BufReader<ChildStdout>,
    next_id: u64,
}

impl Surfaces {
    fn start(jobs: &str) -> Surfaces {
        let mut server = Command::new(EXE).args(["serve", "--port", "0"]).env("GAZEFORGE_JOBS", jobs).stdout(Stdio::piped()).spawn().unwrap();
        let mut first = String::new();
        BufReader::new(server.stdout.take().unwrap()).read_line(&mut first).unwrap();
        let listening: Value = serde_json::from_str(&first).unwrap();
        let port = listening["listening"].as_str().unwrap().rsplit(':').next().unwrap().parse().unwrap();
        let mut mcp = Command::new(EXE).arg("mcp").env("GAZEFORGE_JOBS", jobs).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let mcp_in = mcp.stdin.take().unwrap();
        let mcp_out = BufReader::new(mcp.stdout.take().unwrap());
        let mut surfaces = Surfaces { server, port, mcp, mcp_in, mcp_out, next_id: 1 };
        let init = surfaces.rpc("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "conformance", "version": "0"}}));
        assert_eq!(init["result"]["serverInfo"]["name"], "gazeforge", "{init}");
        writeln!(surfaces.mcp_in, "{}", json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).unwrap();
        surfaces
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(self.mcp_in, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        self.mcp_in.flush().unwrap();
        let mut line = String::new();
        self.mcp_out.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], json!(id), "{reply}");
        reply
    }

    fn cli(&self, name: &str, input: &Value, jobs: &str) -> (String, u8) {
        let output = Command::new(EXE).args(["call", name, "--input", &input.to_string()]).env("GAZEFORGE_JOBS", jobs).output().unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert_eq!(text.lines().count(), 1, "{text}");
        (text.trim_end_matches('\n').to_string(), output.status.code().unwrap() as u8)
    }

    fn http(&self, name: &str, input: &Value) -> (String, u16) {
        let body = input.to_string();
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(stream, "POST /v1/{name} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", self.port, body.len())
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (body.to_string(), status)
    }

    fn mcp(&mut self, name: &str, input: &Value) -> (String, bool) {
        let tool = gf_api::find(name).map_or_else(|| name.to_string(), |c| c.tool_name());
        let reply = self.rpc("tools/call", json!({"name": tool, "arguments": input}));
        let result = &reply["result"];
        (result["content"][0]["text"].as_str().unwrap().to_string(), result["isError"].as_bool().unwrap())
    }
}

impl Drop for Surfaces {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.mcp.kill();
        let _ = self.server.wait();
        let _ = self.mcp.wait();
    }
}

fn conform(cases: &[(&str, Value, bool)], jobs: &str) -> Vec<Value> {
    let mut surfaces = Surfaces::start(jobs);
    let mut report = Vec::new();
    for (name, input, want_ok) in cases {
        let (cli, exit) = surfaces.cli(name, input, jobs);
        let (http, status) = surfaces.http(name, input);
        let (mcp, is_error) = surfaces.mcp(name, input);
        let envelope: Value = serde_json::from_str(&cli).unwrap();
        assert_eq!(envelope["ok"].as_bool(), Some(*want_ok), "{name}: {cli}");
        assert_eq!(cli, http, "{name}: CLI and HTTP bytes differ");
        assert_eq!(cli, mcp, "{name}: CLI and MCP bytes differ");
        assert_eq!(exit, gf_api::exit_code(&envelope), "{name}: exit code");
        assert_eq!(status, gf_api::http_status(&envelope), "{name}: HTTP status");
        assert_eq!(is_error, !want_ok, "{name}: MCP isError");
        report.push(json!({"command": name, "bytes": cli.len(), "exit": exit, "http": status, "ok": want_ok}));
    }
    report
}

#[test]
fn the_same_commands_give_the_same_bytes_on_cli_http_and_mcp() {
    let (root, pack, catalog) = fixture("three");
    let jobs = root.join("jobs");
    let store = root.join("store").to_str().unwrap().to_string();
    let spec = root.join("c.gfchar.json").to_str().unwrap().to_string();
    let dense = root.join("c.npy").to_str().unwrap().to_string();
    let missing = root.join("missing.gfchar.json").to_str().unwrap().to_string();
    let cases = vec![
        ("commands.list", json!({}), true),
        ("store.list", json!({"store": store}), true),
        ("catalog.list", json!({"store": store, "catalog": catalog, "category": "nose"}), true),
        ("character.create", json!({"store": store, "pack": pack, "catalog": catalog, "out": spec, "weights": {"nose/a": 0.5}}), true),
        ("character.validate", json!({"store": store, "spec": spec}), true),
        ("character.evaluate", json!({"store": store, "spec": spec, "out": dense, "precision": "f64"}), true),
        ("character.measure", json!({"store": store, "spec": spec, "pairs": {"across": [1, 3], "up": [0, 2]}}), true),
        ("character.measure", json!({"store": store, "spec": spec}), false),
        ("character.create", json!({"store": store, "pack": pack, "catalog": catalog, "out": spec, "weights": {"nose/a": 1.5}}), false),
        ("character.create", json!({"store": store, "pack": pack, "catalog": catalog, "out": spec, "weights": {"nose/zz": 0.5}}), false),
        ("character.validate", json!({"store": store, "spec": missing}), false),
        ("nope", json!({}), false),
    ];
    let report = conform(&cases, jobs.to_str().unwrap());
    assert_eq!(report.len(), cases.len());
    let measured: Value = serde_json::from_str(&Surfaces::start(jobs.to_str().unwrap()).cli("character.measure", &cases[6].1, jobs.to_str().unwrap()).0).unwrap();
    let across = &measured["result"]["measurements"]["across"];
    assert!((across["base_mm"].as_f64().unwrap() - 14.1421).abs() < 1e-3, "{measured}");
    assert!((across["mm"].as_f64().unwrap() - 0.011f64.hypot(0.01) * 1000.0).abs() < 1e-3, "{measured}");
}

#[test]
#[ignore = "needs a full store: GAZEFORGE_CONFORMANCE_STORE, _PACK, _CATALOG"]
fn the_full_base_measures_the_same_on_every_surface() {
    let var = |k: &str| std::env::var(format!("GAZEFORGE_CONFORMANCE_{k}")).unwrap_or_else(|_| panic!("GAZEFORGE_CONFORMANCE_{k} is not set"));
    let (store, pack, catalog) = (var("STORE"), var("PACK"), var("CATALOG"));
    let root = std::env::temp_dir().join(format!("gazeforge-conf-full-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let spec = root.join("wide.gfchar.json").to_str().unwrap().to_string();
    let dense = root.join("wide.npy").to_str().unwrap().to_string();
    let cases = vec![
        ("store.list", json!({"store": store}), true),
        ("catalog.list", json!({"store": store, "catalog": catalog, "contains": "width"}), true),
        ("character.create", json!({"store": store, "pack": pack, "catalog": catalog, "out": spec, "weights": {"nose/nose-scale-horiz-incr": 0.5}}), true),
        ("character.evaluate", json!({"store": store, "spec": spec, "out": dense}), true),
        ("character.measure", json!({"store": store, "spec": spec}), true),
        ("character.create", json!({"store": store, "pack": pack, "catalog": catalog, "out": spec, "weights": {"nose/none": 0.5}}), false),
    ];
    let report = conform(&cases, root.join("jobs").to_str().unwrap());
    println!("{}", json!({"cases": report}));
}

#[cfg(windows)]
#[test]
fn a_cancelled_job_leaves_no_worker_processes() {
    let root = std::env::temp_dir().join(format!("gazeforge-conf-jobs-{}", std::process::id()));
    let jobs = root.join("jobs");
    let jobs_s = jobs.to_str().unwrap();
    let call = |name: &str, input: Value| -> Value {
        let output = Command::new(EXE).args(["call", name, "--input", &input.to_string()]).env("GAZEFORGE_JOBS", jobs_s).output().unwrap();
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let submitted = call("jobs.submit", json!({"command": "debug.slow", "input": {"seconds": 120, "children": 3}}));
    let id = submitted["result"]["id"].as_str().unwrap().to_string();
    let mut running = Value::Null;
    for _ in 0..100 {
        running = call("jobs.status", json!({"id": id}));
        if running["result"]["state"] == "running" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert_eq!(running["result"]["state"], "running", "{running}");
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let cancelled = call("jobs.cancel", json!({"id": id, "wait_s": 20}));
    assert_eq!(cancelled["result"]["state"], "cancelled", "{cancelled}");
    assert_eq!(cancelled["result"]["worker_processes_left"], json!(0), "{cancelled}");
    let worker = cancelled["result"]["worker_pid"].as_u64().unwrap();
    let listed = Command::new("tasklist").args(["/FI", &format!("PID eq {worker}"), "/NH"]).output().unwrap();
    assert!(!String::from_utf8_lossy(&listed.stdout).contains("gazeforge"), "worker {worker} is still alive");
    let done = call("jobs.submit", json!({"command": "debug.slow", "input": {"seconds": 0.2, "children": 1}}));
    let id = done["result"]["id"].as_str().unwrap().to_string();
    let finished = call("jobs.status", json!({"id": id, "wait_s": 30}));
    assert_eq!(finished["result"]["state"], "done", "{finished}");
    assert_eq!(finished["result"]["result"]["result"]["children"], json!(1), "{finished}");
    assert_eq!(finished["result"]["worker_processes_left"], json!(0), "{finished}");
}
