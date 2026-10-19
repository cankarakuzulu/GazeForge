use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::render::{ExternalFrames, FrameRequest};
use crate::viewer::Matrices;

pub const PROTOCOL: u16 = 1;
const MAGIC: u32 = 0x4852_4647;
const HELLO: u16 = 1;
const HELLO_ACK: u16 = 2;
const MESH: u16 = 3;
const POSITIONS: u16 = 4;
const RENDER: u16 = 5;
const FRAME: u16 = 6;
const READBACK: u16 = 7;
const VERTS: u16 = 8;
const CONFIG: u16 = 9;
const CONFIG_ACK: u16 = 10;
const ERROR: u16 = 12;
const MESH_ACK: u16 = 13;
const POSITIONS_ACK: u16 = 14;
const RING: usize = 4096;
const MAX_PAYLOAD: usize = 8192 * 8192 * 4 + 65536;

#[derive(Clone, Debug)]
pub struct Config {
    pub godot: Option<PathBuf>,
    pub project: Option<PathBuf>,
    pub script: String,
    pub material: Option<String>,
    pub environment: Option<String>,
    pub scene: Option<String>,
    pub background: Option<[f64; 3]>,
    pub orbit_scale: f64,
    pub still_ms: f64,
    pub start_timeout_ms: u64,
    pub frame_timeout_ms: u64,
    pub visible: bool,
    pub host_args: Vec<String>,
    pub log: Option<PathBuf>,
    pub source: String,
    pub problems: Vec<String>,
}

pub fn default_path() -> Option<PathBuf> {
    crate::keymap::default_path().map(|p| p.with_file_name("studio-godot.json"))
}

fn find_project() -> Option<PathBuf> {
    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        starts.extend(exe.parent().map(Path::to_path_buf));
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    for s in starts {
        for dir in s.ancestors() {
            let p = dir.join("clients").join("godot-render");
            if p.join("project.godot").is_file() {
                return Some(p);
            }
        }
    }
    None
}

impl Default for Config {
    fn default() -> Config {
        Config {
            godot: std::env::var_os("GAZEFORGE_GODOT").map(PathBuf::from),
            project: std::env::var_os("GAZEFORGE_GODOT_PROJECT").map(PathBuf::from).or_else(find_project),
            script: "res://host.gd".into(),
            material: None,
            environment: None,
            scene: None,
            background: None,
            orbit_scale: 0.5,
            still_ms: 120.0,
            start_timeout_ms: 30000,
            frame_timeout_ms: 3000,
            visible: false,
            host_args: Vec::new(),
            log: None,
            source: "defaults".into(),
            problems: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(file: Option<&Path>) -> Config {
        let mut c = Config::default();
        let path = file.map(Path::to_path_buf).or_else(default_path);
        if let Some(p) = path.filter(|p| file.is_some() || p.is_file()) {
            match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string())) {
                Ok(v) => {
                    c.apply(&v);
                    c.source = p.display().to_string();
                }
                Err(e) => c.problems.push(format!("{}: {e}", p.display())),
            }
        }
        c
    }

    pub fn apply(&mut self, v: &Value) {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        let num = |k: &str| v.get(k).and_then(Value::as_f64);
        if let Some(s) = text("godot") {
            self.godot = Some(PathBuf::from(s));
        }
        if let Some(s) = text("project") {
            self.project = Some(PathBuf::from(s));
        }
        if let Some(s) = text("script") {
            self.script = s;
        }
        if let Some(s) = text("log") {
            self.log = Some(PathBuf::from(s));
        }
        for (k, slot) in [("material", &mut self.material), ("environment", &mut self.environment), ("scene", &mut self.scene)] {
            if let Some(s) = v.get(k).and_then(Value::as_str) {
                *slot = Some(s.to_string());
            }
        }
        if let Some(b) = v.get("background").and_then(Value::as_array).filter(|a| a.len() == 3) {
            self.background = Some([0, 1, 2].map(|i| b[i].as_f64().unwrap_or(0.0)));
        }
        if let Some(x) = num("orbit_scale") {
            self.orbit_scale = x.clamp(0.1, 1.0);
        }
        if let Some(x) = num("still_ms") {
            self.still_ms = x.max(0.0);
        }
        if let Some(x) = num("start_timeout_ms") {
            self.start_timeout_ms = x.max(100.0) as u64;
        }
        if let Some(x) = num("frame_timeout_ms") {
            self.frame_timeout_ms = x.max(50.0) as u64;
        }
        if let Some(b) = v.get("visible").and_then(Value::as_bool) {
            self.visible = b;
        }
        if let Some(a) = v.get("host_args").and_then(Value::as_array) {
            self.host_args = a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect();
        }
    }

    pub fn json(&self) -> Value {
        json!({"godot": self.godot.as_ref().map(|p| p.display().to_string()), "project": self.project.as_ref().map(|p| p.display().to_string()), "script": self.script,
               "material": self.material, "environment": self.environment, "scene": self.scene, "background": self.background, "orbit_scale": self.orbit_scale,
               "still_ms": self.still_ms, "start_timeout_ms": self.start_timeout_ms, "frame_timeout_ms": self.frame_timeout_ms, "visible": self.visible, "host_args": self.host_args,
               "source": self.source, "problems": self.problems})
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Status {
    Starting,
    Ready,
    Failed,
    Stopped,
}

#[derive(Clone)]
struct Cam {
    ortho: bool,
    a: f32,
    near: f32,
    far: f32,
    xf: [f32; 12],
}

#[derive(Clone)]
struct Want {
    key: Vec<u32>,
    cam: Cam,
    w: u32,
    h: u32,
    changed: Instant,
    marker: Option<[f32; 4]>,
}

pub struct Frame {
    w: u32,
    h: u32,
    rgba: Arc<Vec<u8>>,
    key: Vec<u32>,
    posted: Instant,
    revision: u64,
    scale: f64,
    seq: u64,
}

struct HeadCopy {
    revision: u64,
    positions: Arc<Vec<f32>>,
    normals: Arc<Vec<f32>>,
}

pub struct Topology {
    pub map: Vec<u32>,
    pub uvs: Vec<f32>,
    pub indices: Vec<u32>,
    pub reversed: bool,
}

struct TopoSource {
    ident: (usize, usize, usize, usize, usize),
    indices: Arc<Vec<u32>>,
    uvs: Arc<Vec<f32>>,
}

enum Cmd {
    Readback(mpsc::Sender<Result<(u64, Vec<f32>), String>>),
}

#[derive(Default)]
struct Rings {
    camera_to_shown: Vec<f64>,
    settle_to_shown: Vec<f64>,
    roundtrip: Vec<f64>,
    render: Vec<f64>,
    readback: Vec<f64>,
    transfer: Vec<f64>,
    positions: Vec<f64>,
    shown: u64,
    shown_full: u64,
    shown_scaled: u64,
    rendered: u64,
    since: Option<Instant>,
}

fn push(v: &mut Vec<f64>, x: f64) {
    if v.len() >= RING {
        v.remove(0);
    }
    v.push(x);
}

struct State {
    status: Status,
    detail: String,
    info: Value,
    want: [Option<Want>; 4],
    head: Option<HeadCopy>,
    head_ident: (usize, usize),
    topo_src: Option<TopoSource>,
    topo: Option<Arc<Topology>>,
    sent_revision: Option<u64>,
    frames: [Option<Frame>; 4],
    shown_seq: [u64; 4],
    shown_rev: u64,
    rev_posted: VecDeque<(u64, Instant)>,
    cmds: VecDeque<Cmd>,
    marker: Option<[f32; 4]>,
    stop: bool,
    pid: Option<u32>,
    rings: Rings,
    started: Instant,
    ready_ms: Option<f64>,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

fn lock(s: &Shared) -> MutexGuard<'_, State> {
    s.0.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Host {
    shared: Shared,
    child: Arc<Mutex<Option<Child>>>,
    pub config: Config,
}

fn plant() -> String {
    std::env::var("GAZEFORGE_GODOT_PLANT").unwrap_or_default()
}

impl Host {
    pub fn start(config: Config) -> Arc<Host> {
        let shared: Shared = Arc::new((
            Mutex::new(State {
                status: Status::Starting,
                detail: "starting Godot".into(),
                info: Value::Null,
                want: Default::default(),
                head: None,
                head_ident: (0, 0),
                topo_src: None,
                topo: None,
                sent_revision: None,
                frames: Default::default(),
                shown_seq: [0; 4],
                shown_rev: 0,
                rev_posted: VecDeque::new(),
                cmds: VecDeque::new(),
                marker: None,
                stop: false,
                pid: None,
                rings: Rings::default(),
                started: Instant::now(),
                ready_ms: None,
            }),
            Condvar::new(),
        ));
        let child = Arc::new(Mutex::new(None));
        let (s, c, cfg) = (shared.clone(), child.clone(), config.clone());
        let spawned = std::thread::Builder::new().name("godot-host".into()).spawn(move || {
            let r = worker(&s, &c, &cfg);
            if let Some(mut ch) = c.lock().unwrap_or_else(|e| e.into_inner()).take() {
                let _ = ch.kill();
                let _ = ch.wait();
            }
            let mut st = lock(&s);
            match r {
                Err(e) if !st.stop => {
                    st.status = Status::Failed;
                    st.detail = e;
                }
                _ => {
                    st.status = Status::Stopped;
                    if st.detail.is_empty() {
                        st.detail = "stopped".into();
                    }
                }
            }
            s.1.notify_all();
        });
        if let Err(e) = spawned {
            let mut st = lock(&shared);
            st.status = Status::Failed;
            st.detail = format!("cannot start the Godot thread: {e}");
        }
        Arc::new(Host { shared, child, config })
    }

    pub fn status(&self) -> (Status, String) {
        let st = lock(&self.shared);
        (st.status, st.detail.clone())
    }

    pub fn status_text(&self) -> String {
        let (s, d) = self.status();
        match s {
            Status::Ready => "Godot".into(),
            Status::Starting => format!("Godot starting ({d})"),
            Status::Failed => format!("Godot failed, native fallback: {d}"),
            Status::Stopped => format!("Godot stopped, native fallback: {d}"),
        }
    }

    pub fn pid(&self) -> Option<u32> {
        lock(&self.shared).pid
    }

    pub fn topology(&self) -> Option<Arc<Topology>> {
        lock(&self.shared).topo.clone()
    }

    pub fn set_marker(&self, m: Option<[f32; 4]>) {
        lock(&self.shared).marker = m;
        self.shared.1.notify_all();
    }

    pub fn reset_stats(&self) {
        let mut st = lock(&self.shared);
        st.rings = Rings { since: Some(Instant::now()), ..Rings::default() };
    }

    pub fn current(&self, slot: usize, revision: u64) -> bool {
        let st = lock(&self.shared);
        match (&st.want[slot], &st.frames[slot]) {
            (Some(w), Some(f)) => f.key == w.key && f.revision == revision && f.scale >= 1.0 && st.shown_seq[slot] == f.seq,
            _ => false,
        }
    }

    pub fn readback(&self, timeout: Duration) -> Result<(u64, Vec<f32>), String> {
        let (tx, rx) = mpsc::channel();
        {
            let mut st = lock(&self.shared);
            if st.status != Status::Ready {
                return Err(format!("Godot is not ready: {}", st.detail));
            }
            st.cmds.push_back(Cmd::Readback(tx));
        }
        self.shared.1.notify_all();
        rx.recv_timeout(timeout).map_err(|e| format!("readback: {e}"))?
    }

    pub fn json(&self) -> Value {
        let st = lock(&self.shared);
        let r = &st.rings;
        let s = crate::script::stats;
        let secs = r.since.map(|t| t.elapsed().as_secs_f64());
        json!({"status": format!("{:?}", st.status).to_lowercase(), "detail": st.detail, "info": st.info, "pid": st.pid, "ready_ms": st.ready_ms,
               "protocol": PROTOCOL, "config": self.config.json(),
               "render_vertices": st.topo.as_ref().map(|t| t.map.len()), "indices_reversed": st.topo.as_ref().map(|t| t.reversed),
               "camera_to_shown_ms": s(r.camera_to_shown.clone()), "settle_to_shown_ms": s(r.settle_to_shown.clone()),
               "roundtrip_ms": s(r.roundtrip.clone()), "godot_render_ms": s(r.render.clone()), "godot_readback_ms": s(r.readback.clone()),
               "frame_transfer_ms": s(r.transfer.clone()), "positions_ms": s(r.positions.clone()),
               "frames_shown": r.shown, "frames_shown_full": r.shown_full, "frames_shown_scaled": r.shown_scaled, "frames_rendered": r.rendered,
               "seconds": secs, "shown_per_s": secs.map(|x| r.shown as f64 / x.max(1e-9))})
    }

    fn post(&self, slot: usize, req: &FrameRequest) -> Option<Arc<Vec<u8>>> {
        let now = Instant::now();
        let mut st = lock(&self.shared);
        if matches!(st.status, Status::Failed | Status::Stopped) {
            return None;
        }
        if let Some(h) = req.head {
            let lod = &h.lods[0];
            let ident = (lod.indices.as_ptr() as usize, lod.indices.len(), lod.uvs.as_ptr() as usize, lod.uvs.len(), h.n);
            if st.topo_src.as_ref().is_none_or(|t| t.ident != ident) {
                st.topo_src = Some(TopoSource { ident, indices: Arc::new(lod.indices.clone()), uvs: Arc::new(lod.uvs.clone()) });
            }
            let hid = (h.positions.as_ptr() as usize, h.positions.len());
            if st.head.as_ref().is_none_or(|c| c.revision != h.revision) || st.head_ident != hid {
                st.head = Some(HeadCopy { revision: h.revision, positions: Arc::new(h.positions.clone()), normals: Arc::new(h.normals.clone()) });
                st.head_ident = hid;
                st.rev_posted.push_back((h.revision, now));
                while st.rev_posted.len() > 512 {
                    st.rev_posted.pop_front();
                }
            }
        }
        let cam = cam_of(req.matrices);
        let marker = st.marker;
        let mut key: Vec<u32> = vec![req.width, req.height, cam.ortho as u32, cam.a.to_bits(), cam.near.to_bits(), cam.far.to_bits()];
        key.extend(cam.xf.iter().map(|x| x.to_bits()));
        key.extend(marker.unwrap_or([0.0; 4]).iter().map(|x| x.to_bits()));
        let changed = match &st.want[slot] {
            Some(w) if w.key == key => w.changed,
            _ => now,
        };
        st.want[slot] = Some(Want { key, cam, w: req.width, h: req.height, changed, marker });
        self.shared.1.notify_all();
        let (seq, w, h) = match &st.frames[slot] {
            Some(f) => (f.seq, f.w, f.h),
            None => return None,
        };
        if w != req.width || h != req.height {
            return None;
        }
        if seq != st.shown_seq[slot] {
            st.shown_seq[slot] = seq;
            let (posted, revision, scale) = st.frames[slot].as_ref().map(|f| (f.posted, f.revision, f.scale)).expect("checked above");
            let shown_rev = st.shown_rev;
            let settle = if revision > shown_rev { st.rev_posted.iter().find(|(r, _)| *r == revision).map(|(_, t)| *t) } else { None };
            let r = &mut st.rings;
            push(&mut r.camera_to_shown, (now - posted).as_secs_f64() * 1000.0);
            r.shown += 1;
            if scale >= 1.0 {
                r.shown_full += 1;
            } else {
                r.shown_scaled += 1;
            }
            if let Some(t) = settle {
                push(&mut r.settle_to_shown, (now - t).as_secs_f64() * 1000.0);
            }
            if revision > shown_rev {
                st.shown_rev = revision;
            }
        }
        st.frames[slot].as_ref().map(|f| f.rgba.clone())
    }

    fn release(&self, slot: usize) {
        let mut st = lock(&self.shared);
        st.want[slot] = None;
        st.frames[slot] = None;
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        lock(&self.shared).stop = true;
        self.shared.1.notify_all();
        if let Some(mut ch) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }
}

fn cam_of(m: &Matrices) -> Cam {
    let a = if m.ortho { 2.0 * m.half_h } else { 2.0 * m.tanh.atan().to_degrees() * if plant() == "fov" { 1.03 } else { 1.0 } };
    let xf = [m.s[0], m.s[1], m.s[2], m.u[0], m.u[1], m.u[2], -m.f[0], -m.f[1], -m.f[2], m.eye[0], m.eye[1], m.eye[2]].map(|x| x as f32);
    Cam { ortho: m.ortho, a: a as f32, near: m.near as f32, far: m.far as f32, xf }
}

pub struct GodotSlot {
    pub host: Arc<Host>,
    pub slot: usize,
}

impl ExternalFrames for GodotSlot {
    fn name(&self) -> &str {
        "godot"
    }
    fn frame(&mut self, request: &FrameRequest) -> Option<Arc<Vec<u8>>> {
        self.host.post(self.slot, request)
    }
    fn status(&self) -> Option<String> {
        Some(self.host.status_text())
    }
}

impl Drop for GodotSlot {
    fn drop(&mut self) {
        self.host.release(self.slot);
    }
}

fn topology(src: &TopoSource, positions: &[f32], normals: &[f32]) -> Topology {
    let idx = &src.indices;
    let has_uv = src.uvs.len() >= idx.len() * 2;
    let mut seen: HashMap<(u32, u32, u32), u32> = HashMap::with_capacity(idx.len());
    let (mut map, mut uvs, mut out) = (Vec::new(), Vec::new(), Vec::with_capacity(idx.len()));
    for (c, &v) in idx.iter().enumerate() {
        let uv = if has_uv { [src.uvs[c * 2], src.uvs[c * 2 + 1]] } else { [0.0, 0.0] };
        let k = (v, uv[0].to_bits(), uv[1].to_bits());
        let id = *seen.entry(k).or_insert_with(|| {
            map.push(v);
            uvs.extend_from_slice(&uv);
            (map.len() - 1) as u32
        });
        out.push(id);
    }
    let p = |i: u32| [0, 1, 2].map(|k| positions[i as usize * 3 + k] as f64);
    let mut facing = 0.0;
    for t in idx.chunks_exact(3) {
        let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
        let (e1, e2) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let g = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let n: f64 = t.iter().map(|&i| (0..3).map(|k| g[k] * normals[i as usize * 3 + k] as f64).sum::<f64>()).sum();
        facing += n.signum();
    }
    let reversed = facing > 0.0;
    if reversed {
        for t in out.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
    }
    Topology { map, uvs, indices: out, reversed }
}

fn gather(t: &Topology, v: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(t.map.len() * 3);
    for &i in &t.map {
        out.extend_from_slice(&v[i as usize * 3..i as usize * 3 + 3]);
    }
    out
}

fn f32_bytes(v: &[f32]) -> impl Iterator<Item = u8> + '_ {
    v.iter().flat_map(|x| x.to_le_bytes())
}

fn send(s: &mut TcpStream, t: u16, payload: &[u8]) -> Result<(), String> {
    let mut h = Vec::with_capacity(16 + payload.len());
    h.extend_from_slice(&MAGIC.to_le_bytes());
    h.extend_from_slice(&PROTOCOL.to_le_bytes());
    h.extend_from_slice(&t.to_le_bytes());
    h.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    h.extend_from_slice(payload);
    s.write_all(&h).map_err(|e| format!("send to Godot: {e}"))
}

fn recv(s: &mut TcpStream) -> Result<(u16, Vec<u8>, Instant), String> {
    let mut h = [0u8; 16];
    s.read_exact(&mut h).map_err(|e| format!("Godot did not answer: {e}"))?;
    let first = Instant::now();
    let u32_at = |o: usize| u32::from_le_bytes([h[o], h[o + 1], h[o + 2], h[o + 3]]);
    if u32_at(0) != MAGIC {
        return Err("Godot sent a bad header".into());
    }
    let version = u16::from_le_bytes([h[4], h[5]]);
    if version != PROTOCOL {
        return Err(format!("Godot render host speaks protocol {version}, the app speaks {PROTOCOL}"));
    }
    let t = u16::from_le_bytes([h[6], h[7]]);
    let len = u32_at(8) as usize;
    if len > MAX_PAYLOAD {
        return Err(format!("Godot sent a {len}-byte payload, over the {MAX_PAYLOAD}-byte limit"));
    }
    let mut p = vec![0u8; len];
    s.read_exact(&mut p).map_err(|e| format!("Godot payload: {e}"))?;
    if t == ERROR {
        return Err(format!("Godot render host: {}", String::from_utf8_lossy(&p)));
    }
    Ok((t, p, first))
}

fn expect(s: &mut TcpStream, want: u16) -> Result<(Vec<u8>, Instant), String> {
    let (t, p, first) = recv(s)?;
    if t != want {
        return Err(format!("Godot answered message type {t}, expected {want}"));
    }
    Ok((p, first))
}

fn rd_u32(p: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]])
}

fn rd_f32(p: &[u8], o: usize) -> f32 {
    f32::from_le_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]])
}

fn launch(cfg: &Config, child: &Arc<Mutex<Option<Child>>>, shared: &Shared) -> Result<TcpStream, String> {
    let godot = cfg.godot.clone().ok_or("no Godot binary configured: set \"godot\" in the Godot config file or GAZEFORGE_GODOT")?;
    if !godot.is_file() {
        return Err(format!("Godot binary not found: {}", godot.display()));
    }
    let project = cfg.project.clone().ok_or("render host project not found: set \"project\" in the Godot config file or GAZEFORGE_GODOT_PROJECT")?;
    if !project.join("project.godot").is_file() {
        return Err(format!("no project.godot in {}", project.display()));
    }
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("listen: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let mut cmd = Command::new(&godot);
    cmd.arg("--path").arg(&project).arg("--script").arg(&cfg.script).arg("--").arg("--port").arg(port.to_string());
    if cfg.visible {
        cmd.arg("--visible");
    }
    cmd.args(&cfg.host_args);
    cmd.stdin(Stdio::null());
    match &cfg.log {
        Some(p) => {
            let f = std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?;
            let f2 = f.try_clone().map_err(|e| e.to_string())?;
            cmd.stdout(f).stderr(f2);
        }
        None => {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let ch = cmd.spawn().map_err(|e| format!("cannot start {}: {e}", godot.display()))?;
    let pid = ch.id();
    *child.lock().unwrap_or_else(|e| e.into_inner()) = Some(ch);
    lock(shared).pid = Some(pid);
    let t0 = Instant::now();
    loop {
        match listener.accept() {
            Ok((s, _)) => {
                s.set_nonblocking(false).map_err(|e| e.to_string())?;
                s.set_nodelay(true).map_err(|e| e.to_string())?;
                return Ok(s);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if lock(shared).stop {
                    return Err("stopped".into());
                }
                if let Some(Ok(Some(code))) = child.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|c| c.try_wait()) {
                    return Err(format!("Godot exited ({code}) before it connected"));
                }
                if t0.elapsed().as_millis() as u64 > cfg.start_timeout_ms {
                    return Err(format!("Godot did not connect within {} ms", cfg.start_timeout_ms));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => return Err(format!("accept: {e}")),
        }
    }
}

enum Job {
    Stop,
    Cmd(Cmd),
    Mesh(TopoSource, Arc<Vec<f32>>, Arc<Vec<f32>>, u64),
    Positions(Arc<Vec<f32>>, Arc<Vec<f32>>, u64),
    Render(usize, Want, u64, f64),
}

fn upscale(src: &[u8], sw: u32, sh: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (w * h * 4) as usize];
    let xs: Vec<usize> = (0..w).map(|x| ((x as u64 * sw as u64 / w as u64) as usize).min(sw as usize - 1) * 4).collect();
    for y in 0..h as usize {
        let sy = ((y as u64 * sh as u64 / h as u64) as usize).min(sh as usize - 1);
        let row = &src[sy * sw as usize * 4..(sy + 1) * sw as usize * 4];
        let dst = &mut out[y * w as usize * 4..(y + 1) * w as usize * 4];
        for (x, &sx) in xs.iter().enumerate() {
            dst[x * 4..x * 4 + 4].copy_from_slice(&row[sx..sx + 4]);
        }
    }
    out
}

fn worker(shared: &Shared, child: &Arc<Mutex<Option<Child>>>, cfg: &Config) -> Result<(), String> {
    if !cfg.problems.is_empty() {
        return Err(cfg.problems.join("; "));
    }
    let mut s = launch(cfg, child, shared)?;
    s.set_read_timeout(Some(Duration::from_millis(cfg.start_timeout_ms))).map_err(|e| e.to_string())?;
    send(&mut s, HELLO, &(PROTOCOL as u32).to_le_bytes())?;
    let (p, _) = expect(&mut s, HELLO_ACK)?;
    let info: Value = serde_json::from_slice(&p[4..]).unwrap_or(Value::Null);
    let conf = json!({"material": cfg.material, "environment": cfg.environment, "scene": cfg.scene, "background": cfg.background});
    send(&mut s, CONFIG, conf.to_string().as_bytes())?;
    let (p, _) = expect(&mut s, CONFIG_ACK)?;
    let applied: Value = serde_json::from_slice(&p).unwrap_or(Value::Null);
    s.set_read_timeout(Some(Duration::from_millis(cfg.frame_timeout_ms))).map_err(|e| e.to_string())?;
    {
        let mut st = lock(shared);
        st.status = Status::Ready;
        st.detail = if applied["ok"].as_bool().unwrap_or(false) { String::new() } else { format!("config problems: {}", applied["problems"]) };
        st.info = json!({"host": info, "config": applied});
        st.ready_ms = Some(st.started.elapsed().as_secs_f64() * 1000.0);
    }
    shared.1.notify_all();
    let mut done: [Option<(Vec<u32>, u64, f64)>; 4] = Default::default();
    let mut topo_ident = None;
    let mut seq = 0u64;
    let mut rr = 0usize;
    let mut built: Option<Arc<Topology>> = None;
    loop {
        let job = {
            let mut st = lock(shared);
            loop {
                if st.stop {
                    break Job::Stop;
                }
                if let Some(c) = st.cmds.pop_front() {
                    break Job::Cmd(c);
                }
                if let (Some(src), Some(h)) = (&st.topo_src, &st.head) {
                    if topo_ident != Some(src.ident) {
                        topo_ident = Some(src.ident);
                        let src = TopoSource { ident: src.ident, indices: src.indices.clone(), uvs: src.uvs.clone() };
                        break Job::Mesh(src, h.positions.clone(), h.normals.clone(), h.revision);
                    }
                    if st.sent_revision != Some(h.revision) {
                        break Job::Positions(h.positions.clone(), h.normals.clone(), h.revision);
                    }
                }
                let now = Instant::now();
                let rev = st.sent_revision.unwrap_or(0);
                let mut pick = None;
                for k in 0..4 {
                    let i = (rr + k) % 4;
                    if let Some(w) = &st.want[i] {
                        let moving = (now - w.changed).as_secs_f64() * 1000.0 < cfg.still_ms;
                        let fresh = matches!(&done[i], Some((key, r, sc)) if *key == w.key && *r == rev && (*sc >= 1.0 || moving));
                        if !fresh && built.is_some() {
                            pick = Some((i, w.clone(), if moving { cfg.orbit_scale } else { 1.0 }));
                            break;
                        }
                    }
                }
                if let Some((i, w, sc)) = pick {
                    rr = i + 1;
                    break Job::Render(i, w, rev, sc);
                }
                st = shared.1.wait_timeout(st, Duration::from_millis(10)).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0);
            }
        };
        match job {
            Job::Stop => return Ok(()),
            Job::Cmd(Cmd::Readback(tx)) => {
                let r = (|| {
                    send(&mut s, READBACK, &[])?;
                    let (p, _) = expect(&mut s, VERTS)?;
                    let n = rd_u32(&p, 0) as usize;
                    let v: Vec<f32> = (0..n * 3).map(|i| rd_f32(&p, 4 + i * 4)).collect();
                    Ok((lock(shared).sent_revision.unwrap_or(0), v))
                })();
                let failed = r.is_err();
                let _ = tx.send(r.clone());
                if failed {
                    return r.map(|_| ());
                }
            }
            Job::Mesh(src, pos, nrm, rev) => {
                let t0 = Instant::now();
                let t = Arc::new(topology(&src, &pos, &nrm));
                lock(shared).topo = Some(t.clone());
                let mut gp = gather(&t, &pos);
                if plant() == "corrupt" && !gp.is_empty() {
                    gp[0] += 0.01;
                }
                let gn = gather(&t, &nrm);
                let mut p: Vec<u8> = Vec::with_capacity(8 + t.map.len() * 32 + t.indices.len() * 4);
                p.extend_from_slice(&(t.map.len() as u32).to_le_bytes());
                p.extend_from_slice(&(t.indices.len() as u32).to_le_bytes());
                p.extend(f32_bytes(&gp));
                p.extend(f32_bytes(&gn));
                p.extend(f32_bytes(&t.uvs));
                p.extend(t.indices.iter().flat_map(|x| x.to_le_bytes()));
                send(&mut s, MESH, &p)?;
                expect(&mut s, MESH_ACK)?;
                built = Some(t);
                let mut st = lock(shared);
                st.sent_revision = Some(rev);
                push(&mut st.rings.positions, t0.elapsed().as_secs_f64() * 1000.0);
            }
            Job::Positions(pos, nrm, rev) => {
                let Some(t) = built.clone() else { continue };
                let t0 = Instant::now();
                let mut gp = gather(&t, &pos);
                if plant() == "corrupt" && !gp.is_empty() {
                    gp[0] += 0.01;
                }
                let gn = gather(&t, &nrm);
                let mut p: Vec<u8> = Vec::with_capacity(12 + t.map.len() * 24);
                p.extend_from_slice(&rev.to_le_bytes());
                p.extend_from_slice(&(t.map.len() as u32).to_le_bytes());
                p.extend(f32_bytes(&gp));
                p.extend(f32_bytes(&gn));
                send(&mut s, POSITIONS, &p)?;
                expect(&mut s, POSITIONS_ACK)?;
                let mut st = lock(shared);
                st.sent_revision = Some(rev);
                push(&mut st.rings.positions, t0.elapsed().as_secs_f64() * 1000.0);
            }
            Job::Render(slot, w, rev, scale) => {
                let (sw, sh) = (((w.w as f64 * scale).round() as u32).max(1), ((w.h as f64 * scale).round() as u32).max(1));
                let flags = w.cam.ortho as u32 | if w.marker.is_some() { 2 } else { 0 };
                let mut p: Vec<u8> = Vec::with_capacity(96);
                for x in [seq as u32, slot as u32, sw, sh, flags] {
                    p.extend_from_slice(&x.to_le_bytes());
                }
                p.extend(f32_bytes(&[w.cam.a, w.cam.near, w.cam.far]));
                p.extend(f32_bytes(&w.cam.xf));
                p.extend(f32_bytes(&w.marker.unwrap_or([0.0; 4])));
                let t0 = Instant::now();
                send(&mut s, RENDER, &p)?;
                let (f, first) = expect(&mut s, FRAME)?;
                let done_at = Instant::now();
                let (fw, fh) = (rd_u32(&f, 4), rd_u32(&f, 8));
                if (fw, fh) != (sw, sh) || f.len() != 20 + (fw * fh * 4) as usize {
                    return Err(format!("Godot returned a {fw}x{fh} frame for a {sw}x{sh} request"));
                }
                let px = if (sw, sh) == (w.w, w.h) { f[20..].to_vec() } else { upscale(&f[20..], sw, sh, w.w, w.h) };
                seq += 1;
                done[slot] = Some((w.key.clone(), rev, scale));
                let mut st = lock(shared);
                let r = &mut st.rings;
                r.rendered += 1;
                push(&mut r.roundtrip, (done_at - t0).as_secs_f64() * 1000.0);
                push(&mut r.render, rd_f32(&f, 12) as f64);
                push(&mut r.readback, rd_f32(&f, 16) as f64);
                push(&mut r.transfer, (done_at - first).as_secs_f64() * 1000.0);
                st.frames[slot] = Some(Frame { w: w.w, h: w.h, rgba: Arc::new(px), key: w.key, posted: w.changed, revision: rev, scale, seq });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_splits_at_uv_seams_and_keeps_every_corner_on_its_vertex() {
        let positions = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0];
        let normals = vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        let indices = vec![0, 1, 2, 2, 1, 3];
        let uvs = vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.5, 1.0, 1.0, 0.0, 1.0, 1.0];
        let src = TopoSource { ident: (0, 0, 0, 0, 0), indices: Arc::new(indices.clone()), uvs: Arc::new(uvs) };
        let t = topology(&src, &positions, &normals);
        assert_eq!(t.map.len(), 5);
        assert!(t.reversed);
        for (tri, out) in indices.chunks_exact(3).zip(t.indices.chunks_exact(3)) {
            assert_eq!([t.map[out[0] as usize], t.map[out[2] as usize], t.map[out[1] as usize]], [tri[0], tri[1], tri[2]]);
        }
        let gathered = gather(&t, &positions);
        for (i, &v) in t.map.iter().enumerate() {
            assert_eq!(&gathered[i * 3..i * 3 + 3], &positions[v as usize * 3..v as usize * 3 + 3]);
        }
    }

    #[test]
    fn upscale_repeats_source_pixels() {
        let src: Vec<u8> = (0..16u8).collect();
        let out = upscale(&src, 2, 2, 4, 4);
        assert_eq!(&out[0..8], &[0, 1, 2, 3, 0, 1, 2, 3]);
        assert_eq!(&out[out.len() - 4..], &[12, 13, 14, 15]);
        assert_eq!(upscale(&src, 2, 2, 2, 2), src);
    }

    #[test]
    fn a_missing_binary_fails_without_blocking() {
        let cfg = Config { godot: Some(PathBuf::from("no/such/godot.exe")), ..Config::default() };
        let t = Instant::now();
        let host = Host::start(cfg);
        while host.status().0 == Status::Starting && t.elapsed().as_secs() < 5 {
            std::thread::sleep(Duration::from_millis(1));
        }
        let (s, d) = host.status();
        assert_eq!(s, Status::Failed);
        assert!(d.contains("not found"), "{d}");
    }
}
