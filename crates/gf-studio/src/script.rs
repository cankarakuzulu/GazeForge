use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{Value, json};

use crate::godot;
use crate::render::{Gpu, Renderer, Source, TestPattern};
use crate::shape::{Edit, ShapeEvent};
use crate::viewer::{self, Layout, mode_index};
use crate::worker::stamp;
use crate::world::World;

pub struct Offscreen {
    pub gpu: Gpu,
    pub renderer: Renderer,
    pub world: World,
    pub width: u32,
    pub height: u32,
    pub godot: Option<Arc<godot::Host>>,
    pub(crate) ui: Option<crate::ui_script::OffUi>,
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

pub(crate) fn stats(mut v: Vec<f64>) -> Value {
    if v.is_empty() {
        return json!(null);
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    let at = |q: f64| v[((n - 1) as f64 * q).round() as usize];
    json!({"count": n, "mean": v.iter().sum::<f64>() / n as f64, "min": v[0], "p50": at(0.5), "p95": at(0.95), "max": v[n - 1]})
}

pub fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
    let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| format!("png: {e}"))?;
    writer.write_image_data(&rgb).map_err(|e| format!("png: {e}"))?;
    writer.finish().map_err(|e| format!("png: {e}"))
}

fn slots_of(step: &Value, world: &World) -> Vec<usize> {
    match step.get("slot") {
        Some(Value::String(s)) if s == "all" => (0..4).collect(),
        Some(v) if v.is_u64() => vec![(v.as_u64().unwrap_or(0) as usize).min(3)],
        _ => vec![world.state.active.min(world.slot_count() - 1)],
    }
}

impl Offscreen {
    pub fn new(width: u32, height: u32) -> Result<Offscreen, String> {
        let gpu = Gpu::headless()?;
        let renderer = Renderer::new(&gpu.device);
        Ok(Offscreen { gpu, renderer, world: World::default(), width, height, godot: None, ui: None })
    }

    fn drag(&mut self, step: &Value) -> Result<Value, String> {
        let num = |k: &str| step.get(k).and_then(Value::as_f64);
        let (target, mac) = (step.get("target").and_then(Value::as_str), step.get("macro").and_then(Value::as_str));
        let key = target.or(mac).ok_or("drag needs target or macro")?.to_string();
        let shape = self.world.shape.as_ref().ok_or("nothing is loaded")?;
        let now = if mac.is_some() { shape.live.macros.get(&key).copied().ok_or_else(|| format!("{key} is not a macro slider"))? } else { shape.live.weight(&key) };
        if target.is_some() && !shape.targets.iter().any(|t| t.id == key) {
            return Err(format!("{key} is not a head target"));
        }
        let (from, to) = (num("from").unwrap_or(now), num("to").ok_or("drag needs to")?);
        let frames = num("frames").unwrap_or(30.0).max(1.0) as usize;
        let edit = |v: f64| if mac.is_some() { Edit::Macro(key.clone(), v) } else { Edit::Weight(key.clone(), v) };
        let t0 = Instant::now();
        let samples_wait_ms = if mac.is_some() && step.get("wait_samples").and_then(Value::as_bool).unwrap_or(false) {
            let k = key.clone();
            self.world.wait(move |w| w.samples_ready(&k), 120.0)?;
            Some(ms(t0))
        } else {
            None
        };
        self.world.shape_event(ShapeEvent::Begin(vec![key.clone()]));
        let slope_wait_ms = if mac.is_some() && step.get("wait_slope").and_then(Value::as_bool).unwrap_or(true) {
            self.world.wait(|w| w.idle(), 120.0)?;
            Some(ms(t0))
        } else {
            None
        };
        let (mut latency, mut complete, mut compute, mut missing, mut fallback, mut sampled) = (Vec::new(), Vec::new(), Vec::new(), 0usize, 0usize, 0usize);
        let first = self.world.hud.preview_latency.total;
        let (anchor_at, mut shown, mut stale) = (self.world.anchor_at, None, 0usize);
        for i in 1..=frames {
            let v = from + (to - from) * i as f64 / frames as f64;
            self.world.shape_event(ShapeEvent::Preview(vec![edit(v)]));
            let r = self.render()?;
            complete.push(r["complete_ms"].as_f64().unwrap_or(0.0));
            if self.world.hud.preview_latency.total > first + latency.len() as u64 {
                latency.push(self.world.hud.preview_latency.last().unwrap_or(0.0));
            }
            compute.push(self.world.last_preview["total_ms"].as_f64().unwrap_or(0.0));
            if self.world.last_preview["missing_slopes"].as_array().is_some_and(|a| !a.is_empty()) {
                missing += 1;
            }
            let has = |k: &str| self.world.last_preview[k].as_array().is_some_and(|a| !a.is_empty());
            fallback += has("slope_fallback") as usize;
            sampled += has("sampled") as usize;
            stale += has("stale_slope") as usize;
            if mac.is_some() && shown.is_none() && (has("slope_fallback") || has("sampled")) {
                shown = Some(Instant::now());
            }
        }
        let mut out = json!({"key": key, "from": from, "to": to, "frames": frames, "slope_wait_ms": slope_wait_ms, "samples_wait_ms": samples_wait_ms, "frames_without_macro_preview": missing,
                             "frames_slope_fallback": fallback, "frames_sampled": sampled, "busy_text": self.world.busy_text(),
                             "input_to_preview_ms": stats(latency), "frame_complete_ms": stats(complete), "preview_compute_ms": stats(compute), "last_preview": self.world.last_preview,
                             "frames_stale_slope": stale, "begin_t": stamp(t0), "drag_end_t": stamp(Instant::now()), "anchor_t": anchor_at.map(stamp), "first_macro_preview_t": shown.map(stamp),
                             "begin_to_first_macro_preview_ms": shown.map(|s| (s - t0).as_secs_f64() * 1000.0)});
        if step.get("release").and_then(Value::as_bool).unwrap_or(true) {
            let previewed = self.world.session.as_ref().map(|s| s.head.positions.clone()).unwrap_or_default();
            let t = Instant::now();
            self.world.shape_event(ShapeEvent::Commit(format!("{key} to {to:.3}")));
            let mut during = Vec::new();
            while self.world.settle_wanted.is_some() {
                if t.elapsed().as_secs_f64() > 600.0 {
                    return Err("the settle did not finish in 600 s".into());
                }
                let r = self.render()?;
                during.push(r["complete_ms"].as_f64().unwrap_or(0.0));
                std::thread::sleep(std::time::Duration::from_millis(step.get("frame_ms").and_then(Value::as_u64).unwrap_or(4)));
            }
            self.render()?;
            if let Some(e) = &self.world.settle_error {
                return Err(e.clone());
            }
            out["release_to_settled_ms"] = json!(ms(t));
            if let Some(s) = &self.world.session {
                let mm: Vec<f64> = previewed
                    .chunks_exact(3)
                    .zip(s.head.positions.chunks_exact(3))
                    .map(|(a, b)| (0..3).map(|k| (a[k] as f64 - b[k] as f64).powi(2)).sum::<f64>().sqrt() * 1000.0)
                    .collect();
                out["preview_vs_settled_mm"] = stats(mm);
            }
            out["settle"] = self.world.last_settle.clone().unwrap_or(Value::Null);
            out["frames_during_settle"] = json!(during.len());
            out["frame_complete_during_settle_ms"] = stats(during);
        }
        Ok(out)
    }

    pub fn render(&mut self) -> Result<Value, String> {
        let t = Instant::now();
        self.world.tick();
        self.renderer.sync_world(&self.gpu.device, &self.gpu.queue, &mut self.world);
        let head = self.world.session.as_ref().map(|s| &s.head);
        let f = self.renderer.render(&self.gpu.device, &self.gpu.queue, &self.world.state, head, self.width, self.height);
        let encoded = ms(t);
        self.renderer.wait(&self.gpu.device)?;
        let done = Instant::now();
        let complete = (done - t).as_secs_f64() * 1000.0;
        self.world.frame_done(done);
        self.world.hud.frame_cpu.push(encoded);
        self.world.hud.frame_gpu.push(complete);
        Ok(json!({"encode_ms": f.encode_ms, "submit_ms": encoded, "complete_ms": complete, "passes": f.passes, "draws": f.draws}))
    }

    fn screenshot(&mut self, path: &Path, which: &str) -> Result<Value, String> {
        let r = self.render()?;
        let (x, y, w, h) = if which == "layout" {
            (0, 0, self.width, self.height)
        } else {
            let a = self.world.state.active_rect(self.width, self.height);
            (a.x, a.y, a.w, a.h)
        };
        let t = Instant::now();
        let px = self.renderer.read_rgba(&self.gpu.device, &self.gpu.queue, x, y, w, h)?;
        let read_ms = ms(t);
        let mut sum = [0u64; 3];
        for p in px.chunks_exact(4) {
            for k in 0..3 {
                sum[k] += p[k] as u64;
            }
        }
        let n = (w * h).max(1) as f64;
        write_png(path, w, h, &px)?;
        Ok(json!({"file": path.display().to_string(), "width": w, "height": h, "render": r, "readback_ms": read_ms,
                  "mean_rgb": sum.map(|s| (s as f64 / n * 100.0).round() / 100.0)}))
    }

    fn godot_slots(&self) -> Vec<viewer::Rect> {
        viewer::rects(self.world.state.layout, self.width, self.height).into_iter().filter(|r| self.renderer.sources[r.slot].name() == "godot").collect()
    }

    fn godot_step(&mut self, op: &str, step: &Value, out: &Path) -> Result<Value, String> {
        let num = |k: &str| step.get(k).and_then(Value::as_f64);
        let text = |k: &str| step.get(k).and_then(Value::as_str);
        if op == "source" {
            let restart = step.get("restart").and_then(Value::as_bool).unwrap_or(false);
            if self.godot.is_none() || restart {
                let mut cfg = godot::Config::load(text("config").map(Path::new));
                if let Some(v) = step.get("godot") {
                    cfg.apply(v);
                }
                if cfg.log.is_none() {
                    cfg.log = Some(out.join("godot.log"));
                }
                if cfg.background.is_none() {
                    cfg.background = Some(crate::render::background(self.world.state.opts.dark));
                }
                self.godot = Some(godot::Host::start(cfg));
            }
            let host = self.godot.clone().ok_or("no Godot host")?;
            for s in slots_of(step, &self.world) {
                self.renderer.sources[s] = Source::External(Box::new(godot::GodotSlot { host: host.clone(), slot: s }));
            }
            return Ok(json!({"source": "godot", "config": host.config.json()}));
        }
        let host = self.godot.clone().ok_or("no Godot host: run a source step with kind godot first")?;
        match op {
            "godot_wait" => {
                let t = Instant::now();
                let limit = num("seconds").unwrap_or(60.0);
                let mut renders = 0usize;
                loop {
                    self.render()?;
                    renders += 1;
                    let (status, detail) = host.status();
                    let rev = self.world.session.as_ref().map_or(0, |s| s.head.revision);
                    let slots = self.godot_slots();
                    if matches!(status, godot::Status::Failed | godot::Status::Stopped) || (status == godot::Status::Ready && !slots.is_empty() && slots.iter().all(|r| host.current(r.slot, rev))) {
                        return Ok(json!({"ms": ms(t), "renders": renders, "status": format!("{status:?}").to_lowercase(), "detail": detail, "slots": slots.iter().map(|r| r.slot).collect::<Vec<_>>()}));
                    }
                    if t.elapsed().as_secs_f64() > limit {
                        return Err(format!("Godot frames not current after {limit} s: {}", host.status_text()));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(num("frame_ms").unwrap_or(2.0) as u64));
                }
            }
            "godot_status" => {
                let v = host.json();
                if step.get("reset").and_then(Value::as_bool).unwrap_or(false) {
                    host.reset_stats();
                }
                Ok(v)
            }
            "godot_readback" => {
                let stem = text("file").ok_or("godot_readback needs file")?;
                let rb = host.readback(std::time::Duration::from_secs(20))?;
                let (rev, back) = (rb.revision, &rb.positions);
                let topo = host.topology().ok_or("Godot holds no mesh yet")?;
                let s = self.world.session.as_ref().ok_or("nothing is loaded")?;
                let write = |name: String, bytes: Vec<u8>| -> Result<String, String> {
                    let p = out.join(name);
                    std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display()))?;
                    Ok(p.display().to_string())
                };
                let g = write(format!("{stem}.godot.f32"), back.iter().flat_map(|x| x.to_le_bytes()).collect())?;
                let a = write(format!("{stem}.app.f32"), s.head.positions.iter().flat_map(|x| x.to_le_bytes()).collect())?;
                let m = write(format!("{stem}.map.u32"), topo.map.iter().flat_map(|x| x.to_le_bytes()).collect())?;
                let mut worst = 0f64;
                if back.len() == topo.map.len() * 3 {
                    for (i, &v) in topo.map.iter().enumerate() {
                        for k in 0..3 {
                            worst = worst.max((back[i * 3 + k] as f64 - s.head.positions[v as usize * 3 + k] as f64).abs());
                        }
                    }
                }
                Ok(json!({"godot": g, "app": a, "map": m, "render_vertices": topo.map.len(), "godot_vertices": back.len() / 3, "game_vertices": s.head.n,
                          "revision_sent": rev, "head_revision": s.head.revision, "max_abs": worst, "godot_lod": rb.lod, "godot_indices": rb.indices,
                          "app_lod_indices": s.head.lods.iter().map(|l| l.indices.len()).collect::<Vec<_>>(),
                          "slot_lods": (0..4).map(|i| self.world.state.vp[i].lod).collect::<Vec<_>>()}))
            }
            "godot_track" => {
                let mk = host.marker().ok_or("godot_track needs a godot_marker step first")?;
                let pos = [mk[0] as f64, mk[1] as f64, mk[2] as f64];
                let count = num("count").unwrap_or(240.0).max(1.0) as usize;
                let (spin, pace) = (num("spin").unwrap_or(0.5), num("frame_ms").unwrap_or(6.944));
                let a = self.world.state.active_rect(self.width, self.height);
                let (mut last_seq, mut prev_claim): (Option<u64>, Option<[f64; 2]>) = (None, None);
                let (mut rows, mut complete) = (Vec::new(), Vec::new());
                let t = Instant::now();
                for _ in 0..count {
                    let started = Instant::now();
                    self.world.state.orbit(a.cam, spin, 0.0, true);
                    let r = self.render()?;
                    complete.push(r["complete_ms"].as_f64().unwrap_or(0.0));
                    if let Some(f) = host.shown(a.slot).filter(|f| Some(f.seq) != last_seq) {
                        last_seq = Some(f.seq);
                        let (mut n, mut sx, mut sy) = (0u64, 0f64, 0f64);
                        for (i, p) in f.rgba.chunks_exact(4).enumerate() {
                            if p[0] > 200 && p[1] < 80 && p[2] > 200 {
                                n += 1;
                                sx += (i as u32 % f.w) as f64 + 0.5;
                                sy += (i as u32 / f.w) as f64 + 0.5;
                            }
                        }
                        let claim = f.project(pos);
                        let found = (n > 0).then(|| [sx / n as f64, sy / n as f64]);
                        let err = match (found, claim) {
                            (Some(c), Some(p)) => Some(((c[0] - p[0]).powi(2) + (c[1] - p[1]).powi(2)).sqrt()),
                            _ => None,
                        };
                        let step = match (claim, prev_claim) {
                            (Some(p), Some(q)) => Some(((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()),
                            _ => None,
                        };
                        prev_claim = claim;
                        rows.push(json!({"seq": f.seq, "revision": f.revision, "scale": f.scale, "lod": f.lod, "marker_px": n, "centroid": found, "claimed": claim, "error_px": err, "step_from_previous_px": step}));
                    }
                    let left = pace - ms(started);
                    if left > 0.0 {
                        std::thread::sleep(std::time::Duration::from_secs_f64(left / 1000.0));
                    }
                }
                let errs: Vec<f64> = rows.iter().filter_map(|r| r["error_px"].as_f64()).collect();
                let steps: Vec<f64> = rows.iter().filter_map(|r| r["step_from_previous_px"].as_f64()).collect();
                let missing = rows.iter().filter(|r| r["error_px"].is_null()).count();
                Ok(json!({"frames": count, "seconds": ms(t) / 1000.0, "shown_frames": rows.len(), "checked": errs.len(), "missing_marker": missing,
                          "max_error_px": errs.iter().cloned().fold(0.0, f64::max), "error_px": stats(errs), "step_px": stats(steps), "complete_ms": stats(complete), "rows": rows}))
            }
            "godot_marker" => {
                if step.get("off").and_then(Value::as_bool).unwrap_or(false) {
                    host.set_marker(None);
                    return Ok(json!({"marker": null}));
                }
                let s = self.world.session.as_ref().ok_or("nothing is loaded")?;
                let st = &self.world.state;
                let a = st.active_rect(self.width, self.height);
                let am = st.cams[a.cam].matrices(a.w as f64 / a.h as f64);
                let p = |i: usize| [0, 1, 2].map(|k| s.head.positions[i * 3 + k] as f64);
                let vertex = match step.get("vertex") {
                    Some(Value::Number(n)) => n.as_u64().unwrap_or(0) as usize,
                    _ => {
                        let mut front = st.cams[a.cam].cur;
                        front.yaw = 0.0;
                        front.pitch = 0.0;
                        let back = viewer::Camera::basis(&front).back;
                        (0..s.head.n).max_by(|&i, &j| viewer::dot(p(i), back).total_cmp(&viewer::dot(p(j), back))).unwrap_or(0)
                    }
                };
                if vertex >= s.head.n {
                    return Err(format!("vertex {vertex} out of range ({})", s.head.n));
                }
                let pos = p(vertex);
                let depth = viewer::dot(viewer::sub(pos, am.eye), am.f);
                let per_px = if am.ortho { 2.0 * am.half_h / a.h as f64 } else { 2.0 * depth * am.tanh / a.h as f64 };
                let radius = num("radius_px").unwrap_or(3.0) * per_px;
                host.set_marker(Some([pos[0] as f32, pos[1] as f32, pos[2] as f32, radius as f32]));
                let slots: Vec<Value> = viewer::rects(st.layout, self.width, self.height)
                    .iter()
                    .map(|r| {
                        let m = st.cams[r.cam].matrices(r.w as f64 / r.h as f64);
                        let c = |row: usize| (0..3).map(|k| m.mvp[k * 4 + row] as f64 * pos[k]).sum::<f64>() + m.mvp[12 + row] as f64;
                        let (x, y, w) = (c(0), c(1), c(3));
                        let (px, py) = ((x / w * 0.5 + 0.5) * r.w as f64, (0.5 - y / w * 0.5) * r.h as f64);
                        json!({"slot": r.slot, "rect": [r.x, r.y, r.w, r.h], "px": [px, py], "layout_px": [r.x as f64 + px, r.y as f64 + py], "source": self.renderer.sources[r.slot].name()})
                    })
                    .collect();
                Ok(json!({"vertex": vertex, "position": pos, "radius": radius, "slots": slots}))
            }
            "godot_kill" => {
                let pid = host.pid().ok_or("no Godot process")?;
                let status = if cfg!(windows) { Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output() } else { Command::new("kill").args(["-9", &pid.to_string()]).output() };
                let t = Instant::now();
                let mut renders = 0usize;
                let mut frame_ms = Vec::new();
                while t.elapsed().as_secs_f64() < num("watch_seconds").unwrap_or(5.0) {
                    let r = self.render()?;
                    frame_ms.push(r["complete_ms"].as_f64().unwrap_or(0.0));
                    renders += 1;
                    if matches!(host.status().0, godot::Status::Failed | godot::Status::Stopped) && renders > 10 {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Ok(json!({"pid": pid, "kill_ok": status.as_ref().map(|o| o.status.success()).unwrap_or(false), "renders_after_kill": renders, "frame_complete_ms": stats(frame_ms),
                          "status": host.status_text(), "detected_ms": ms(t)}))
            }
            "godot_stop" => {
                let t = Instant::now();
                for s in 0..4 {
                    if self.renderer.sources[s].name() == "godot" {
                        self.renderer.sources[s] = Source::Native;
                    }
                }
                drop(host);
                self.godot = None;
                Ok(json!({"stop_ms": ms(t)}))
            }
            other => Err(format!("unknown op {other}")),
        }
    }

    pub fn step(&mut self, step: &Value, out: &Path) -> Result<Value, String> {
        let op = step.get("op").and_then(Value::as_str).ok_or("each step needs op")?;
        if op.starts_with("ui") || op.starts_with("cube_") {
            return self.ui_step(op, step, out);
        }
        if op.starts_with("godot_") || (op == "source" && step.get("kind").and_then(Value::as_str) == Some("godot")) {
            return self.godot_step(op, step, out);
        }
        let text = |k: &str| step.get(k).and_then(Value::as_str);
        let num = |k: &str| step.get(k).and_then(Value::as_f64);
        let w = &mut self.world;
        let res = match op {
            "load" => {
                let store = text("store").ok_or("load needs store")?;
                if let Some(n) = num("preview_threads") {
                    w.preview_threads = (n as usize).max(1);
                }
                w.request_load(store, text("spec"), text("macro"), text("eyes"));
                w.wait(|w| w.loading.is_none(), 600.0)?;
                let r = w.load_result.clone().ok_or("the load returned nothing")??;
                w.wait(|w| w.idle(), 600.0)?;
                r
            }
            "layout" => {
                w.state.layout = Layout::parse(text("layout").unwrap_or("")).ok_or("layout is single, side or quad")?;
                w.state.active = w.state.active.min(w.slot_count() - 1);
                json!({"layout": w.state.layout.name()})
            }
            "active" => {
                w.state.active = (num("slot").unwrap_or(0.0) as usize).min(w.slot_count() - 1);
                json!({"active": w.state.active})
            }
            "mode" => {
                let name = text("mode").ok_or("mode needs mode")?;
                let m = mode_index(name).ok_or_else(|| format!("unknown mode {name}; modes: {}", viewer::MODES.map(|m| m.0).join(", ")))?;
                let t = Instant::now();
                for s in slots_of(step, w) {
                    w.set_mode(s, m);
                }
                w.wait(|w| w.idle(), 600.0)?;
                json!({"mode": name, "ms": ms(t)})
            }
            "lod" => {
                let lod = (num("lod").unwrap_or(0.0) as usize).min(w.lods() - 1);
                for s in slots_of(step, w) {
                    w.state.vp[s].lod = lod;
                }
                json!({"lod": lod, "lods": w.lods()})
            }
            "view" => {
                let name = text("name").ok_or("view needs name")?;
                if !w.state.named_view(name, true) {
                    return Err(format!("unknown view {name}"));
                }
                json!({"view": name})
            }
            "ortho" => {
                let cam = w.state.active_cam();
                cam.ortho = step.get("on").and_then(Value::as_bool).unwrap_or(!cam.ortho);
                json!({"ortho": cam.ortho})
            }
            "camera" => {
                let idx = num("cam").map(|c| (c as usize).min(3));
                let cam = match idx {
                    Some(i) => &mut w.state.cams[i],
                    None => w.state.active_cam(),
                };
                let target = step.get("target").and_then(Value::as_array).filter(|a| a.len() == 3).map(|a| [0, 1, 2].map(|k| a[k].as_f64().unwrap_or(0.0)));
                cam.set(
                    |g| {
                        if let Some(v) = num("yaw") {
                            g.yaw = v;
                        }
                        if let Some(v) = num("pitch") {
                            g.pitch = v;
                        }
                        if let Some(v) = num("dist") {
                            g.dist = v;
                        }
                        if let Some(v) = num("fov") {
                            g.fov = v;
                        }
                        if let Some(t) = target {
                            g.target = t;
                        }
                    },
                    true,
                );
                if let Some(o) = step.get("ortho").and_then(Value::as_bool) {
                    cam.ortho = o;
                }
                json!({"yaw": cam.goal.yaw, "pitch": cam.goal.pitch, "dist": cam.goal.dist, "fov": cam.goal.fov, "target": cam.goal.target, "ortho": cam.ortho})
            }
            "orbit" | "pan" | "zoom" => {
                let a = w.state.active_rect(self.width, self.height);
                match op {
                    "orbit" => w.state.orbit(a.cam, num("dx").unwrap_or(0.0), num("dy").unwrap_or(0.0), true),
                    "pan" => w.state.pan(a.cam, num("dx").unwrap_or(0.0), num("dy").unwrap_or(0.0), a.h as f64, true),
                    _ => w.state.zoom(a.cam, num("delta").unwrap_or(0.0), true),
                }
                let g = w.state.cams[a.cam].goal;
                json!({"yaw": g.yaw, "pitch": g.pitch, "dist": g.dist, "target": g.target})
            }
            "fov" => {
                w.state.set_fov(num("fov").ok_or("fov needs fov")?, true);
                json!({"fov": w.state.active_cam().goal.fov})
            }
            "focus" => {
                w.frame_head(true);
                w.state.pivot_flash = 0.0;
                let g = w.state.active_cam().goal;
                json!({"target": g.target, "dist": g.dist})
            }
            "action" => json!({"message": w.action(text("id").ok_or("action needs id")?, true)?}),
            "option" => {
                let name = text("name").ok_or("option needs name")?;
                w.state.opts.set(name, step.get("value").unwrap_or(&Value::Null))?;
                json!({"option": name})
            }
            "weights" | "edit" => {
                let shape = w.shape.as_ref().ok_or("nothing is loaded")?;
                let mut edits = Vec::new();
                if step.get("replace").and_then(Value::as_bool).unwrap_or(false) {
                    edits.extend(shape.live.weights.keys().map(|k| Edit::Weight(k.clone(), 0.0)));
                }
                if let Some(given) = step.get("weights").and_then(Value::as_object) {
                    edits.extend(given.iter().map(|(k, v)| Edit::Weight(k.clone(), v.as_f64().unwrap_or(0.0))));
                }
                if let Some(given) = step.get("macros").and_then(Value::as_object) {
                    edits.extend(given.iter().map(|(k, v)| Edit::Macro(k.clone(), v.as_f64().unwrap_or(0.0))));
                }
                if let Some(e) = step.get("eyeballs") {
                    let spec = if e.is_null() { None } else { Some(serde_json::from_value(e.clone()).map_err(|x| format!("eyeballs: {x}"))?) };
                    edits.push(Edit::Eyes(spec));
                }
                w.shape_event(ShapeEvent::Preview(edits));
                let commit = op == "weights" || step.get("commit").and_then(Value::as_bool).unwrap_or(false);
                if commit {
                    w.shape_event(ShapeEvent::Commit(text("label").unwrap_or("script edit").to_string()));
                }
                if !commit || !step.get("wait").and_then(Value::as_bool).unwrap_or(true) {
                    json!({"generation": w.shape.as_ref().map(|s| s.generation), "committed": commit, "settle": w.settle_stats()})
                } else {
                    w.wait(|w| w.settle_wanted.is_none(), 600.0)?;
                    if let Some(e) = &w.settle_error {
                        return Err(e.clone());
                    }
                    w.last_settle.clone().unwrap_or(Value::Null)
                }
            }
            "settle" => {
                w.request_settle();
                w.wait(|w| w.settle_wanted.is_none(), 600.0)?;
                w.last_settle.clone().unwrap_or(Value::Null)
            }
            "wait" => json!({"waited_ms": w.wait(|w| w.idle(), num("seconds").unwrap_or(600.0))?}),
            "drag" => return self.drag(step),
            "save" => {
                let file = text("file").ok_or("save needs file")?;
                let path = if Path::new(file).is_absolute() { PathBuf::from(file) } else { out.join(file) };
                w.save(&path.display().to_string())?;
                w.wait(|w| w.saved.is_some(), 120.0)?;
                w.saved.clone().ok_or("save returned nothing")??
            }
            "dump_settle" => {
                let s = w.session.as_ref().ok_or("nothing is loaded")?;
                let stem = text("file").ok_or("dump_settle needs file")?;
                let mut input = w.settle_input().ok_or("nothing is loaded")?;
                if let Some(m) = &s.macro_file {
                    input["macro"] = json!(m);
                }
                let (pos, inp) = (out.join(format!("{stem}.f32")), out.join(format!("{stem}.input.json")));
                let bytes: Vec<u8> = s.head.positions.iter().flat_map(|x| x.to_le_bytes()).collect();
                std::fs::write(&pos, &bytes).map_err(|e| format!("{}: {e}", pos.display()))?;
                std::fs::write(&inp, serde_json::to_string(&input).unwrap_or_default()).map_err(|e| format!("{}: {e}", inp.display()))?;
                let eyes = match &s.head.eyes {
                    Some(m) => {
                        let file = out.join(format!("{stem}.eyes.f32"));
                        let bytes: Vec<u8> = m.positions.iter().flat_map(|x| x.to_le_bytes()).collect();
                        std::fs::write(&file, &bytes).map_err(|e| format!("{}: {e}", file.display()))?;
                        json!(file.display().to_string())
                    }
                    None => Value::Null,
                };
                json!({"positions": pos.display().to_string(), "input": inp.display().to_string(), "eyes": eyes, "settled_revision": s.settled.as_ref().map(|x| x.revision.clone()), "idle": w.idle()})
            }
            "hud" => json!({"hud": w.hud.json(), "settle": w.settle_stats(), "last_settle": w.last_settle, "last_preview": w.last_preview, "samples": w.samples_info, "trace": w.trace}),
            "source" => {
                let kind = text("kind").unwrap_or("native");
                for s in slots_of(step, w) {
                    self.renderer.sources[s] = match kind {
                        "native" => Source::Native,
                        "test-pattern" => Source::External(Box::new(TestPattern)),
                        other => return Err(format!("unknown source {other}; sources: native, test-pattern, godot")),
                    };
                }
                json!({"source": kind})
            }
            "screenshot" => {
                let file = text("file").ok_or("screenshot needs file")?;
                return self.screenshot(&out.join(file), text("which").unwrap_or("layout"));
            }
            "frames" => {
                let count = num("count").unwrap_or(60.0).max(1.0) as usize;
                let spin = num("spin").unwrap_or(0.0);
                let pace = num("frame_ms");
                let (mut submit, mut complete, mut slow) = (Vec::new(), Vec::new(), Vec::new());
                let t = Instant::now();
                for k in 0..count {
                    let started = Instant::now();
                    if spin != 0.0 {
                        let a = self.world.state.active_rect(self.width, self.height);
                        self.world.state.orbit(a.cam, spin, 0.0, true);
                    }
                    let r = self.render()?;
                    submit.push(r["submit_ms"].as_f64().unwrap_or(0.0));
                    complete.push(r["complete_ms"].as_f64().unwrap_or(0.0));
                    if r["complete_ms"].as_f64().unwrap_or(0.0) > 16.0 {
                        slow.push(json!({"frame": k, "at_ms": ms(t), "encode_ms": r["encode_ms"], "submit_ms": r["submit_ms"], "complete_ms": r["complete_ms"]}));
                    }
                    if let Some(p) = pace {
                        let left = p - ms(started);
                        if left > 0.0 {
                            std::thread::sleep(std::time::Duration::from_secs_f64(left / 1000.0));
                        }
                    }
                }
                let total = ms(t);
                return Ok(json!({"frames": count, "total_ms": total, "fps": count as f64 / (total / 1000.0), "submit_ms": stats(submit), "complete_ms": stats(complete), "frames_over_16ms": slow}));
            }
            "dump_positions" => {
                let s = w.session.as_ref().ok_or("nothing is loaded")?;
                let file = out.join(text("file").ok_or("dump_positions needs file")?);
                let bytes: Vec<u8> = s.head.positions.iter().flat_map(|x| x.to_le_bytes()).collect();
                std::fs::write(&file, &bytes).map_err(|e| format!("{}: {e}", file.display()))?;
                json!({"file": file.display().to_string(), "floats": s.head.positions.len()})
            }
            "summary" => w.session.as_ref().ok_or("nothing is loaded")?.summary(),
            "keymap" => {
                let map = crate::keymap::Keymap::load(text("file").map(Path::new));
                let bound: Vec<String> = crate::keymap::ACTIONS.iter().map(|a| format!("{}={}", a.0, map.describe(a.0))).collect();
                json!({"source": map.source, "bindings": map.bindings.len(), "problems": map.problems, "actions": bound})
            }
            other => return Err(format!("unknown op {other}")),
        };
        Ok(res)
    }
}

pub fn run(script: &str, out: &str) -> Result<Value, (Value, String)> {
    let fail = |msg: String| (json!({}), msg);
    let text = std::fs::read_to_string(script).map_err(|e| fail(format!("{script}: {e}")))?;
    let doc: Value = serde_json::from_str(&text).map_err(|e| fail(format!("{script}: {e}")))?;
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).map_err(|e| fail(format!("{}: {e}", out.display())))?;
    let size = doc.get("size").and_then(Value::as_array).map(|a| [a.first(), a.get(1)].map(|v| v.and_then(Value::as_u64).unwrap_or(0) as u32)).unwrap_or([1280, 800]);
    let t = Instant::now();
    let mut off = Offscreen::new(size[0].max(1), size[1].max(1)).map_err(fail)?;
    let gpu_ms = ms(t);
    if let Some(theme) = doc.get("theme").and_then(Value::as_str) {
        off.world.state.opts.dark = theme != "light";
    }
    if let Some(d) = doc.get("dpr").and_then(Value::as_f64) {
        off.renderer.dpr = d;
    }
    let info = &off.gpu.info;
    let mut report = json!({"ok": true, "adapter": {"name": info.name, "backend": format!("{:?}", info.backend), "driver": info.driver, "driver_info": info.driver_info, "device_type": format!("{:?}", info.device_type)},
                            "size": size, "gpu_init_ms": gpu_ms, "steps": []});
    let steps = doc.get("steps").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut failed = None;
    for (i, step) in steps.iter().enumerate() {
        let t = Instant::now();
        let r = off.step(step, &out);
        let entry = match r {
            Ok(v) => json!({"i": i, "op": step["op"], "ms": ms(t), "result": v}),
            Err(e) => {
                failed = Some(format!("step {i} ({}): {e}", step["op"]));
                json!({"i": i, "op": step["op"], "ms": ms(t), "error": e})
            }
        };
        report["steps"].as_array_mut().expect("array").push(entry);
        if failed.is_some() {
            break;
        }
    }
    report["total_ms"] = json!(ms(t));
    report["hud"] = off.world.hud.json();
    report["settle_queue"] = off.world.settle_stats();
    if let Some(h) = &off.godot {
        report["godot"] = h.json();
    }
    if let Some(e) = &failed {
        report["ok"] = json!(false);
        report["error"] = json!(e);
    }
    let path = out.join("report.json");
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap_or_default());
    match failed {
        None => Ok(report),
        Some(e) => Err((report, e)),
    }
}
