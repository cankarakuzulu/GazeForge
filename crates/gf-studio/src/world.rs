use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::hud::Hud;
use crate::preview::{Normals, Preview, Slope};
use crate::session::Session;
use crate::shape::{ShapeEvent, ShapeModel};
use crate::viewer::{self, Bounds, DISPLACEMENT, MODES, ViewerState};
use crate::worker::{Done, Job, Jobs, SettleRequest};

pub struct World {
    pub state: ViewerState,
    pub session: Option<Session>,
    pub shape: Option<ShapeModel>,
    pub preview: Option<Preview>,
    normals: Option<Normals>,
    lod0: Option<Arc<Vec<u32>>>,
    pub uvs_dirty: bool,
    pub epoch: u64,
    jobs: Jobs,
    load_seq: u64,
    pub loading: Option<Instant>,
    pub load_result: Option<Result<Value, String>>,
    pub settle_wanted: Option<u64>,
    settle_release: Option<Instant>,
    pub settles_stale: u64,
    pub last_settle: Option<Value>,
    pub settle_error: Option<String>,
    preview_dirty: bool,
    edit_at: Option<Instant>,
    latency_mark: Option<Instant>,
    slope_wanted: BTreeSet<String>,
    reference_wanted: bool,
    pub saved: Option<Result<Value, String>>,
    pub log: Vec<String>,
    pub hud: Hud,
    pub preview_threads: usize,
    pub last_preview: Value,
    pub prefetch: Vec<String>,
}

impl Default for World {
    fn default() -> World {
        let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
        World {
            state: ViewerState::default(),
            session: None,
            shape: None,
            preview: None,
            normals: None,
            lod0: None,
            uvs_dirty: false,
            epoch: 0,
            jobs: Jobs::start(),
            load_seq: 0,
            loading: None,
            load_result: None,
            settle_wanted: None,
            settle_release: None,
            settles_stale: 0,
            last_settle: None,
            settle_error: None,
            preview_dirty: false,
            edit_at: None,
            latency_mark: None,
            slope_wanted: BTreeSet::new(),
            reference_wanted: false,
            saved: None,
            log: Vec::new(),
            hud: Hud::default(),
            preview_threads: std::env::var("GAZEFORGE_PREVIEW_THREADS").ok().and_then(|t| t.parse::<usize>().ok()).unwrap_or(1).clamp(1, cores),
            last_preview: Value::Null,
            prefetch: vec!["age".into(), "gender".into()],
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

impl World {
    pub fn set_wake(&self, f: Box<dyn Fn() + Send>) {
        self.jobs.set_wake(f);
    }

    fn note(&mut self, text: String) {
        self.log.push(text);
        if self.log.len() > 50 {
            self.log.remove(0);
        }
    }

    pub fn request_load(&mut self, store: &str, spec: Option<&str>, macro_file: Option<&str>) {
        self.load_seq += 1;
        self.loading = Some(Instant::now());
        self.load_result = None;
        self.jobs.send(self.load_seq, Job::Load { store: store.to_string(), spec: spec.map(str::to_string), macro_file: macro_file.map(str::to_string) });
    }

    pub fn idle(&self) -> bool {
        self.loading.is_none() && self.settle_wanted.is_none() && self.slope_wanted.is_empty() && !self.reference_wanted
    }

    pub fn busy_text(&self) -> Option<String> {
        if let Some(t) = self.loading {
            return Some(format!("loading {:.1} s", t.elapsed().as_secs_f64()));
        }
        if let (Some(_), Some(t)) = (self.settle_wanted, self.settle_release) {
            return Some(format!("settling {:.0} ms", ms(t.elapsed())));
        }
        if !self.slope_wanted.is_empty() {
            return Some("preparing the macro preview".into());
        }
        None
    }

    pub fn tick(&mut self) {
        while let Some(done) = self.jobs.try_recv() {
            self.handle(done);
        }
        if self.preview_dirty {
            self.update_preview();
        }
    }

    pub fn wait(&mut self, done: impl Fn(&World) -> bool, seconds: f64) -> Result<f64, String> {
        let t = Instant::now();
        loop {
            self.tick();
            if done(self) {
                return Ok(ms(t.elapsed()));
            }
            if t.elapsed().as_secs_f64() > seconds {
                return Err(format!("timed out after {seconds} s waiting for the worker"));
            }
            std::thread::sleep(Duration::from_micros(500));
        }
    }

    fn handle(&mut self, done: Done) {
        let now = Instant::now();
        match done {
            Done::Loaded { epoch, result, ms: load_ms } => {
                if epoch != self.load_seq {
                    return;
                }
                self.loading = None;
                match result {
                    Ok(o) => {
                        let shape = ShapeModel::from_sliders(&o.sliders);
                        let mut preview = match Preview::new(o.deltas, self.preview_threads) {
                            Ok(p) => p,
                            Err(e) => {
                                self.load_result = Some(Err(e));
                                return;
                            }
                        };
                        let s = o.session;
                        preview.set_anchor(&s.head.positions, &shape.start.weights, &shape.start.macros);
                        self.normals = Some(Normals::new(&s.head.lods[0].indices, s.head.n));
                        self.lod0 = Some(Arc::new(s.head.lods[0].indices.clone()));
                        self.state.bounds = Bounds::of(&s.head.positions);
                        let b = self.state.bounds;
                        for cam in &mut self.state.cams {
                            viewer::frame_box(&b, cam, true);
                        }
                        let mut summary = s.summary();
                        summary["load_ms"] = json!(load_ms);
                        summary["preview"] = json!({"targets": preview.ids.len(), "entries": preview.entries(), "deltas_ms": o.deltas_ms, "threads": preview.threads});
                        self.session = Some(s);
                        self.shape = Some(shape);
                        self.preview = Some(preview);
                        self.uvs_dirty = true;
                        self.epoch += 1;
                        self.settle_wanted = None;
                        self.slope_wanted.clear();
                        self.reference_wanted = false;
                        self.hud.settle_latency.clear();
                        self.note(format!("loaded in {load_ms:.0} ms"));
                        self.load_result = Some(Ok(summary));
                        if self.state.vp.iter().any(|v| v.mode == DISPLACEMENT) {
                            self.want_reference();
                        }
                        self.prefetch_slopes();
                    }
                    Err(e) => {
                        self.note(format!("load failed: {e}"));
                        self.load_result = Some(Err(e));
                    }
                }
            }
            Done::Settled { epoch, generation, sent, result, queued_ms } => {
                if epoch != self.epoch {
                    return;
                }
                let current = self.settle_wanted == Some(generation);
                let out = match result {
                    Ok(o) => o,
                    Err(e) => {
                        if current {
                            self.settle_wanted = None;
                        }
                        self.settle_error = Some(e.clone());
                        self.note(format!("settle failed: {e}"));
                        return;
                    }
                };
                let (Some(s), Some(p)) = (self.session.as_mut(), self.preview.as_mut()) else { return };
                if let Err(e) = s.apply_settle(&out) {
                    self.note(e);
                    return;
                }
                p.set_anchor(&out.positions, &sent.weights, &sent.macros);
                let apply_ms = ms(now.elapsed());
                self.uvs_dirty |= out.uvs.is_some();
                self.preview_dirty = true;
                self.hud.settle_worker.push(out.ms);
                let latency = self.settle_release.map(|t| ms(now - t));
                self.last_settle = Some(json!({"generation": generation, "current": current, "revision": out.revision, "placement": out.placement, "approximate": out.approximate,
                                               "worker_ms": out.ms, "queued_ms": queued_ms, "release_to_applied_ms": latency, "apply_ms": apply_ms, "server_ms": out.server_ms}));
                if current {
                    self.settle_wanted = None;
                    if let Some(l) = latency {
                        self.hud.settle_latency.push(l);
                    }
                    self.note(format!("settled {} in {:.0} ms", out.placement, latency.unwrap_or(out.ms)));
                    self.prefetch_slopes();
                } else {
                    self.settles_stale += 1;
                }
            }
            Done::Slope { epoch, name, at, result, ms: slope_ms } => {
                self.slope_wanted.remove(&name);
                if epoch != self.epoch {
                    return;
                }
                match result {
                    Ok(m) => {
                        self.hud.slope.push(slope_ms);
                        if let Some(p) = self.preview.as_mut()
                            && p.anchor_macros.get(&name) == Some(&at)
                        {
                            p.slopes.insert(name, Slope { up: m.up, down: m.down });
                            self.preview_dirty = true;
                        }
                    }
                    Err(e) => self.note(format!("macro preview for {name} failed: {e}")),
                }
            }
            Done::Reference { epoch, result } => {
                self.reference_wanted = false;
                if epoch != self.epoch {
                    return;
                }
                match result {
                    Ok(r) => {
                        if let Some(s) = self.session.as_mut() {
                            s.head.reference = Some(r);
                        }
                    }
                    Err(e) => self.note(format!("reference failed: {e}")),
                }
            }
            Done::Saved { result } => {
                match &result {
                    Ok(v) => self.note(format!("saved {}", v["out"].as_str().unwrap_or(""))),
                    Err(e) => self.note(format!("save failed: {e}")),
                }
                self.saved = Some(result);
            }
        }
    }

    fn want_slope(&mut self, name: &str) {
        self.slope_job(name, false);
    }

    fn slope_job(&mut self, name: &str, prefetch: bool) {
        let (Some(s), Some(p), Some(shape)) = (&self.session, &self.preview, &self.shape) else { return };
        if !shape.macros_available || p.slopes.contains_key(name) || self.slope_wanted.contains(name) {
            return;
        }
        let Some(&at) = p.anchor_macros.get(name) else { return };
        let base = shape.character(&s.start, &shape.live);
        let mut input = s.settle_input(&base, false);
        input["macros"] = json!({"sliders": p.anchor_macros, "mode": shape.macro_mode, "head_scale": shape.head_scale});
        input["slider"] = json!(name);
        if let Some(m) = &s.macro_file {
            input["macro"] = json!(m);
        }
        input.as_object_mut().map(|o| o.remove("uvs"));
        self.slope_wanted.insert(name.to_string());
        self.jobs.send(self.epoch, Job::Slope { name: name.to_string(), at, input, prefetch });
    }

    fn prefetch_slopes(&mut self) {
        for name in self.prefetch.clone() {
            self.slope_job(&name, true);
        }
    }

    fn want_reference(&mut self) {
        let Some(s) = &self.session else { return };
        if s.head.reference.is_some() || self.reference_wanted {
            return;
        }
        self.reference_wanted = true;
        self.jobs.send(self.epoch, Job::Reference { input: s.reference_input() });
    }

    pub fn shape_event(&mut self, ev: ShapeEvent) {
        let Some(shape) = self.shape.as_mut() else { return };
        let out = shape.apply(ev);
        if out.preview {
            self.preview_dirty = true;
            self.edit_at.get_or_insert_with(Instant::now);
        }
        let macros: Vec<String> = shape.macros.iter().map(|m| m.name.clone()).filter(|n| out.begin.contains(n)).collect();
        for m in macros {
            self.want_slope(&m);
        }
        if out.settle {
            self.request_settle();
        }
    }

    pub fn request_settle(&mut self) {
        let (Some(s), Some(shape)) = (self.session.as_mut(), &self.shape) else { return };
        let c = shape.character(&s.start, &shape.live);
        let input = s.settle_input(&c, true);
        s.character = c;
        let now = Instant::now();
        self.jobs.settle(SettleRequest { epoch: self.epoch, generation: shape.generation, sent: shape.live.clone(), input, at: now, indices: self.lod0.clone() });
        self.settle_wanted = Some(shape.generation);
        self.settle_release = Some(now);
        self.settle_error = None;
    }

    pub fn settle_input(&self) -> Option<Value> {
        let (s, shape) = (self.session.as_ref()?, self.shape.as_ref()?);
        Some(s.settle_input(&shape.character(&s.start, &shape.live), true))
    }

    pub fn settle_stats(&self) -> Value {
        let (queued, running, cancelled) = self.jobs.settle_state();
        json!({"queued": queued, "running": running, "cancelled_before_start": cancelled, "stale_applied_as_anchor": self.settles_stale})
    }

    fn update_preview(&mut self) {
        self.preview_dirty = false;
        let (Some(s), Some(p), Some(shape), Some(nrm)) = (self.session.as_mut(), self.preview.as_ref(), self.shape.as_ref(), self.normals.as_mut()) else { return };
        let t = Instant::now();
        let h = &mut s.head;
        let c = p.compute(&shape.live.weights, &shape.live.macros, &mut h.positions);
        let t_n = Instant::now();
        p.normals(nrm, &h.positions, &h.lods[0].indices, &mut h.normals);
        h.revision += 1;
        let total = ms(t.elapsed());
        self.hud.preview_compute.push(total);
        self.last_preview = json!({"targets": c.targets, "macros": c.macros, "missing_slopes": c.missing_slopes, "positions_ms": ms(t_n - t), "normals_ms": ms(t_n.elapsed()), "total_ms": total});
        for m in c.missing_slopes {
            self.want_slope(&m);
        }
        if let Some(e) = self.edit_at.take() {
            self.latency_mark = Some(e);
        }
    }

    pub fn frame_done(&mut self, at: Instant) {
        if let Some(m) = self.latency_mark.take() {
            self.hud.preview_latency.push(ms(at - m));
        }
    }

    pub fn undo(&mut self, redo: bool) -> Option<String> {
        let shape = self.shape.as_mut()?;
        let label = if redo { shape.redo() } else { shape.undo() }?;
        self.preview_dirty = true;
        self.edit_at.get_or_insert_with(Instant::now);
        self.request_settle();
        Some(format!("{} {label}", if redo { "redo" } else { "undo" }))
    }

    pub fn save(&mut self, out: &str) -> Result<(), String> {
        let mut input = self.settle_input().ok_or("nothing is loaded")?;
        input["out"] = json!(out);
        input.as_object_mut().map(|o| o.remove("uvs"));
        self.saved = None;
        self.jobs.send(self.epoch, Job::Save { input });
        Ok(())
    }

    pub fn slot_count(&self) -> usize {
        self.state.layout.slots().len()
    }

    pub fn set_mode(&mut self, slot: usize, mode: usize) {
        let slot = slot.min(3);
        self.state.vp[slot].mode = mode % MODES.len();
        if self.state.vp[slot].mode == DISPLACEMENT {
            self.want_reference();
        }
    }

    pub fn lods(&self) -> usize {
        self.session.as_ref().map_or(1, |s| s.head.lods.len())
    }

    pub fn frame_head(&mut self, instant: bool) {
        if let Some(s) = &self.session {
            let b = Bounds::of(&s.head.positions);
            viewer::frame_box(&b, self.state.active_cam(), instant);
            self.state.pivot_flash = 0.6;
        }
    }

    pub fn action(&mut self, id: &str, instant: bool) -> Result<String, String> {
        let st = &mut self.state;
        let toggle = |st: &mut ViewerState, path: &str, label: &str| st.opts.toggle(path).map(|on| format!("{label} {}", if on { "on" } else { "off" }));
        Ok(match id {
            "frame" => {
                self.frame_head(instant);
                "framed the head".into()
            }
            "layout" => {
                st.layout = st.layout.next();
                st.active = st.active.min(st.layout.slots().len() - 1);
                format!("layout {}", st.layout.name())
            }
            "next_viewport" => {
                st.active = (st.active + 1) % st.layout.slots().len();
                st.pivot_flash = 0.4;
                format!("viewport {}", st.active + 1)
            }
            "mode_next" | "mode_prev" => {
                let slot = st.active.min(st.layout.slots().len() - 1);
                let now = st.vp[slot].mode;
                let next = if id == "mode_next" { (now + 1) % MODES.len() } else { (now + MODES.len() - 1) % MODES.len() };
                self.set_mode(slot, next);
                MODES[next].1.to_string()
            }
            "wire" => toggle(st, "wire", "wireframe")?,
            "xray" => toggle(st, "xray", "hidden edges")?,
            "points" => toggle(st, "points", "vertex points")?,
            "back_faces" => toggle(st, "backHi", "back-face highlight")?,
            "seams" => toggle(st, "seams", "seams and boundaries")?,
            "clip" => toggle(st, "clip.0.on", "clip plane 1")?,
            "light_gizmo" => toggle(st, "lightGizmo", "light gizmo")?,
            "theme" => toggle(st, "dark", "dark viewport")?,
            "fov_down" | "fov_up" => {
                let f = st.active_cam().goal.fov + if id == "fov_up" { 5.0 } else { -5.0 };
                st.set_fov(f, instant);
                format!("field of view {:.0}", st.active_cam().goal.fov)
            }
            "lod_coarser" | "lod_finer" => {
                let n = self.lods();
                let s = self.state.active_slot();
                s.lod = if id == "lod_coarser" { (s.lod + 1).min(n - 1) } else { s.lod.saturating_sub(1) };
                format!("LOD {}", s.lod)
            }
            "view_front" | "view_back" | "view_left" | "view_right" | "view_top" | "view_bottom" | "view_three_quarter" => {
                let name = id.trim_start_matches("view_").replace('_', "");
                st.named_view(&name, instant);
                format!("{name} view")
            }
            "ortho" => {
                let cam = st.active_cam();
                cam.ortho = !cam.ortho;
                if instant {
                    cam.snap();
                }
                if cam.ortho { "orthographic".into() } else { "perspective".into() }
            }
            "undo" | "redo" => self.undo(id == "redo").unwrap_or_else(|| format!("nothing to {id}")),
            "hud" => {
                self.hud.on = !self.hud.on;
                format!("timings {}", if self.hud.on { "shown" } else { "hidden" })
            }
            "reset_shape" => {
                self.shape_event(ShapeEvent::Reset);
                "reset all sliders".into()
            }
            other => return Err(format!("unknown action {other}")),
        })
    }
}
