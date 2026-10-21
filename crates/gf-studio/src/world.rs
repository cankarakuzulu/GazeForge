use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::face_rig::FaceRig;
use crate::stitch::Stitch;
use crate::project::Project;
use crate::timeline::Timeline;
use crate::hud::Hud;
use crate::i18n::{tr, tr_or, trf};
use crate::preview::{Normals, Preview, Sampled, Slope, macro_key};
use crate::session::{EdgePairs, Session};
use crate::shape::{ShapeEvent, ShapeModel, Values};
use crate::topology::TopologyUi;
use crate::viewer::{self, Bounds, DISPLACEMENT, MODES, ViewerState};
use crate::worker::{Done, Job, Jobs, SettleRequest, stamp};

pub struct Extra {
    pub id: String,
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
    pub colour: [f32; 3],
    pub visible: bool,
    pub key: u64,
}

pub struct Overlay {
    pub id: String,
    pub segments: Vec<[f64; 3]>,
    pub colour: [f64; 3],
}

pub struct World {
    pub state: ViewerState,
    pub session: Option<Session>,
    pub shape: Option<ShapeModel>,
    pub preview: Option<Preview>,
    normals: Option<Normals>,
    lod0: Option<Arc<EdgePairs>>,
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
    slope_wanted: BTreeSet<(String, String)>,
    samples_wanted: Option<(u64, String, String)>,
    samples_seq: u64,
    anchor_values: Option<Values>,
    pub samples_info: BTreeMap<String, Value>,
    reference_wanted: bool,
    pub saved: Option<Result<Value, String>>,
    pub log: Vec<String>,
    pub hud: Hud,
    pub preview_threads: usize,
    pub last_preview: Value,
    pub prefetch: Vec<String>,
    pub anchor_at: Option<Instant>,
    pub anchor_release: Option<Instant>,
    pub trace: Vec<Value>,
    pub topo: TopologyUi,
    pub head_generation: u64,
    pub poses: Vec<(String, Vec<f32>)>,
    pose_dirty: bool,
    pub face: FaceRig,
    pub skin: crate::skin::SkinUi,
    pub journal: crate::history::Journal,
    pub extras: Vec<Extra>,
    pub overlays: Vec<Overlay>,
    extra_key: u64,
    pub stitch: Stitch,
    pub body_ui: crate::body_panel::BodyUi,
    pub timeline: Timeline,
    pub eye_pose: Option<([f64; 9], [f64; 3])>,
    pub project: Project,
    pub face_morphs: Option<String>,
    pub face_prefix: Option<String>,
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
            samples_wanted: None,
            samples_seq: 0,
            anchor_values: None,
            samples_info: BTreeMap::new(),
            reference_wanted: false,
            saved: None,
            log: Vec::new(),
            hud: Hud::default(),
            preview_threads: std::env::var("GAZEFORGE_PREVIEW_THREADS").ok().and_then(|t| t.parse::<usize>().ok()).unwrap_or(1).clamp(1, cores),
            last_preview: Value::Null,
            prefetch: vec!["age".into(), "gender".into()],
            anchor_at: None,
            anchor_release: None,
            trace: Vec::new(),
            topo: TopologyUi::default(),
            head_generation: 0,
            poses: Vec::new(),
            pose_dirty: false,
            face: FaceRig::default(),
            skin: crate::skin::SkinUi::default(),
            journal: crate::history::Journal::default(),
            extras: Vec::new(),
            overlays: Vec::new(),
            extra_key: 0,
            stitch: Stitch::default(),
            body_ui: Default::default(),
            timeline: Timeline::default(),
            eye_pose: None,
            project: Project::default(),
            face_morphs: None,
            face_prefix: None,
        }
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

impl World {
    pub fn epoch_bump_head(&mut self) {
        self.head_generation += 1;
    }

    pub fn with_topo<R>(&mut self, f: impl FnOnce(&mut TopologyUi, &mut World) -> R) -> R {
        let mut t = std::mem::take(&mut self.topo);
        let r = f(&mut t, self);
        self.topo = t;
        r
    }

    pub fn set_wake(&self, f: Box<dyn Fn() + Send>) {
        self.jobs.set_wake(f);
    }

    fn mark(&mut self, v: Value) {
        self.trace.push(v);
        if self.trace.len() > 400 {
            self.trace.remove(0);
        }
    }

    fn mark_ready(&mut self, kind: &str, name: &str, now: Instant) {
        let since = |t: Option<Instant>| t.map(|t| ms(now - t));
        let v = json!({"kind": kind, "name": name, "t": stamp(now), "since_anchor_ms": since(self.anchor_at), "since_release_ms": since(self.anchor_release)});
        self.mark(v);
    }

    fn note(&mut self, text: String) {
        self.log.push(text);
        if self.log.len() > 50 {
            self.log.remove(0);
        }
    }

    pub fn request_load(&mut self, store: &str, spec: Option<&str>, macro_file: Option<&str>, eyes: Option<&str>, game: Option<&str>) {
        self.request_load_with(store, spec, None, macro_file, eyes, game);
    }

    pub fn request_load_with(&mut self, store: &str, spec: Option<&str>, character: Option<Value>, macro_file: Option<&str>, eyes: Option<&str>, game: Option<&str>) {
        self.load_seq += 1;
        self.loading = Some(Instant::now());
        self.load_result = None;
        let owned = |s: Option<&str>| s.map(str::to_string);
        self.jobs.send(self.load_seq, Job::Load { store: store.to_string(), spec: owned(spec), character, macro_file: owned(macro_file), eyes: owned(eyes), game: owned(game) });
    }

    pub fn idle(&self) -> bool {
        self.loading.is_none() && self.settle_wanted.is_none() && self.slope_wanted.is_empty() && !self.reference_wanted
    }

    pub fn busy_text(&self) -> Option<String> {
        if let Some(t) = self.loading {
            return Some(trf("studio.busy.loading", &[("s", &format!("{:.1}", t.elapsed().as_secs_f64()))]));
        }
        if let (Some(_), Some(t)) = (self.settle_wanted, self.settle_release) {
            return Some(trf("studio.busy.settling", &[("ms", &format!("{:.0}", ms(t.elapsed())))]));
        }
        if !self.slope_wanted.is_empty() {
            return Some(tr("studio.busy.macro").into());
        }
        if let Some(f) = self.last_preview["slope_fallback"].as_array().filter(|a| !a.is_empty()) {
            let names: Vec<&str> = f.iter().filter_map(Value::as_str).collect();
            return Some(trf("studio.busy.macro_slope", &[("names", &names.join(", "))]));
        }
        None
    }

    pub fn tick(&mut self) {
        while let Some(done) = self.jobs.try_recv() {
            self.handle(done);
        }
        self.face_poll();
        self.skin_poll();
        self.journal_tick();
        self.stitch_poll();
        self.timeline_poll();
        self.project_poll();
        self.project_autosave("timer");
        if self.preview_dirty {
            self.update_preview();
        }
        if self.pose_dirty {
            self.present_pose();
        }
    }

    pub fn set_pose(&mut self, id: &str, positions: Option<Vec<f32>>) -> Result<(), String> {
        let n = self.session.as_ref().map_or(0, |s| s.head.n);
        if let Some(p) = &positions
            && p.len() != n * 3
        {
            return Err(format!("set_pose needs {} floats (game LOD0 order), got {}", n * 3, p.len()));
        }
        self.poses.retain(|(k, _)| k != id);
        if let Some(p) = positions {
            self.poses.push((id.to_string(), p));
        } else {
            self.preview_dirty = true;
        }
        self.pose_dirty = true;
        Ok(())
    }

    pub fn set_extra(&mut self, id: &str, mesh: Option<(Vec<f32>, Vec<f32>, Vec<u32>, [f32; 3])>) {
        self.extras.retain(|e| e.id != id);
        if let Some((positions, normals, indices, colour)) = mesh {
            self.extra_key += 1;
            self.extras.push(Extra { id: id.to_string(), positions, normals, indices, colour, visible: true, key: self.extra_key });
        }
    }

    pub fn set_overlay(&mut self, id: &str, lines: Option<(Vec<[f64; 3]>, [f64; 3])>) {
        self.overlays.retain(|o| o.id != id);
        if let Some((segments, colour)) = lines {
            self.overlays.push(Overlay { id: id.to_string(), segments, colour });
        }
    }

    fn present_pose(&mut self) {
        self.pose_dirty = false;
        let (Some(s), Some((_, p))) = (self.session.as_mut(), self.poses.last()) else { return };
        s.head.positions.copy_from_slice(p);
        s.head.normals = crate::session::vertex_normals(&s.head.positions, &s.head.lods[0].indices, s.head.n);
        s.head.revision += 1;
        s.preview_eyes(self.shape.as_ref().and_then(|sh| sh.live.eyeballs.as_ref()));
        if let (Some((m, c)), Some(e)) = (self.eye_pose, s.head.eyes.as_mut()) {
            for v in e.positions.chunks_exact_mut(3) {
                let (x, y, z) = (v[0] as f64 - c[0], v[1] as f64 - c[1], v[2] as f64 - c[2]);
                v[0] = (m[0] * x + m[1] * y + m[2] * z + c[0]) as f32;
                v[1] = (m[3] * x + m[4] * y + m[5] * z + c[1]) as f32;
                v[2] = (m[6] * x + m[7] * y + m[8] * z + c[2]) as f32;
            }
            for v in e.normals.chunks_exact_mut(3) {
                let (x, y, z) = (v[0] as f64, v[1] as f64, v[2] as f64);
                v[0] = (m[0] * x + m[1] * y + m[2] * z) as f32;
                v[1] = (m[3] * x + m[4] * y + m[5] * z) as f32;
                v[2] = (m[6] * x + m[7] * y + m[8] * z) as f32;
            }
        }
    }

    pub fn face_load(&mut self) {
        let Some(s) = &self.session else {
            self.face.status = tr("studio.load_first").into();
            return;
        };
        if self.face.job.is_some() {
            self.face.stale = true;
            return;
        }
        let mut input = s.settle_input(&s.character, false);
        let prefix = self.face_prefix();
        input["morphs"] = json!(crate::face_rig::SHAPES.iter().map(|n| format!("{prefix}{n}")).collect::<Vec<_>>());
        if let Some(dir) = self.face_morphs() {
            input["morph_dir"] = json!(dir);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let env = gf_api::call(&gf_api::Ctx::from_env(), "rig.face", &input);
            let out = if env["ok"] == true { crate::face_rig::Rig::from_json(&env["result"]) } else { Err(env["error"]["message"].as_str().unwrap_or("rig.face failed").to_string()) };
            let _ = tx.send(out);
        });
        self.face.job = Some(rx);
        self.face.wanted = true;
        self.face.stale = false;
        self.face.status = tr("rig.fitting").into();
    }

    fn face_poll(&mut self) {
        let Some(rx) = &self.face.job else { return };
        let Ok(out) = rx.try_recv() else { return };
        self.face.job = None;
        match out {
            Ok(rig) => {
                if !rig.joints.get(self.face.joint).is_some_and(|j| j.posed) {
                    self.face.joint = rig.joints.iter().position(|j| j.posed).unwrap_or(0);
                }
                self.face.rig = Some(rig);
                self.face.overlay_off = false;
                self.face.status = self.face.loaded_text();
            }
            Err(e) => self.face.status = trf("rig.error", &[("error", &e)]),
        }
        if self.face.stale {
            self.face_load();
        } else {
            self.face_update();
        }
    }

    pub fn face_morphs(&self) -> Option<String> {
        self.face_morphs.clone().or_else(|| std::env::var("GAZEFORGE_FACE_MORPHS").ok())
    }

    pub fn face_prefix(&self) -> String {
        self.face_prefix.clone().or_else(|| std::env::var("GAZEFORGE_FACE_PREFIX").ok()).unwrap_or_else(|| "faceunits/".into())
    }

    pub fn face_update(&mut self) {
        let pose = self.face.pose();
        let _ = self.set_pose("rig", pose);
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
                        self.lod0 = Some(Arc::new(EdgePairs::new(&s.head.lods[0].indices)));
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
                        self.poses.clear();
                        self.face = FaceRig::default();
                        self.skin_from_session();
                        self.journal_reset();
                        self.extras.clear();
                        self.overlays.clear();
                        let old = std::mem::take(&mut self.timeline);
                        self.timeline = Timeline { file: old.file, source: old.source, glb: old.glb, out: old.out, speed: old.speed, looping: old.looping, apply_head: old.apply_head, ..Timeline::default() };
                        self.eye_pose = None;
                        self.stitch = Stitch { store: std::mem::take(&mut self.stitch.store), body: std::mem::take(&mut self.stitch.body), values: self.stitch.values, auto: self.stitch.auto, ..Stitch::default() };
                        self.settle_wanted = None;
                        self.slope_wanted.clear();
                        if let Some((id, _, _)) = self.samples_wanted.take() {
                            self.jobs.drop_samples(id);
                        }
                        self.anchor_values = self.shape.as_ref().map(|sh| sh.start.clone());
                        self.refresh_state_keys();
                        self.anchor_at = Some(now);
                        self.anchor_release = None;
                        self.reference_wanted = false;
                        self.hud.settle_latency.clear();
                        self.note(trf("studio.note.loaded", &[("ms", &format!("{load_ms:.0}"))]));
                        self.load_result = Some(Ok(summary));
                        if self.state.vp.iter().any(|v| v.mode == DISPLACEMENT) {
                            self.want_reference();
                        }
                        self.prefetch_slopes();
                        self.prefetch_samples();
                    }
                    Err(e) => {
                        self.note(trf("studio.note.load_failed", &[("error", &e)]));
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
                        self.note(trf("designer.status.settle_failed", &[("error", &e)]));
                        return;
                    }
                };
                let (Some(s), Some(p)) = (self.session.as_mut(), self.preview.as_mut()) else { return };
                if let Err(e) = s.apply_settle(&out) {
                    self.note(e);
                    return;
                }
                p.set_anchor(&out.positions, &sent.weights, &sent.macros);
                self.anchor_values = Some((*sent).clone());
                self.refresh_state_keys();
                self.anchor_at = Some(now);
                self.anchor_release = if current { self.settle_release } else { None };
                self.mark(json!({"kind": "anchor", "t": stamp(now), "current": current}));
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
                    self.note(trf("studio.settled", &[("placement", &out.placement), ("ms", &format!("{:.0}", latency.unwrap_or(out.ms)))]));
                    if self.face.wanted {
                        self.face_load();
                    }
                    self.stitch_changed(false);
                    if self.timeline.rig.is_some() {
                        self.timeline_deltas();
                    }
                    self.project_autosave("settle");
                    self.prefetch_slopes();
                    self.prefetch_samples();
                } else {
                    self.settles_stale += 1;
                }
            }
            Done::Slope { epoch, name, key, result, ms: slope_ms } => {
                if epoch != self.epoch {
                    return;
                }
                self.slope_wanted.remove(&(name.clone(), key.clone()));
                match result {
                    Ok(m) => {
                        self.hud.slope.push(slope_ms);
                        if let Some(p) = self.preview.as_mut() {
                            p.insert_slope(&name, &key, Slope { up: m.up, down: m.down.iter().map(|x| -x).collect() });
                            self.preview_dirty = true;
                            if p.slope_key == key {
                                self.mark_ready("slope_ready", &name, now);
                            }
                        }
                    }
                    Err(e) => self.note(trf("studio.note.macro_failed", &[("name", &name), ("error", &e)])),
                }
            }
            Done::Samples { epoch, id, name, key, result, ms: samples_ms } => {
                if self.samples_wanted.as_ref().is_some_and(|w| w.0 == id) {
                    self.samples_wanted = None;
                }
                if epoch != self.epoch {
                    return;
                }
                match result {
                    Ok(None) => self.prefetch_samples(),
                    Ok(Some(m)) => {
                        self.hud.samples.push(samples_ms);
                        if let Some(p) = self.preview.as_mut() {
                            let n = m.values.len();
                            p.add_samples(&name, Sampled { key, values: m.values, layers: m.layers });
                            self.samples_info.insert(name.clone(), json!({"values": n, "ms": samples_ms}));
                            self.preview_dirty = true;
                        }
                        self.mark_ready("samples_ready", &name, now);
                        self.prefetch_samples();
                    }
                    Err(e) => self.note(trf("studio.note.samples_failed", &[("name", &name), ("error", &e)])),
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
                    Err(e) => self.note(trf("designer.reference_failed", &[("error", &e)])),
                }
            }
            Done::Trace(t) => {
                self.mark(json!({"kind": t.kind, "name": t.name, "sent": t.sent, "start": t.start, "run": t.run, "end": t.end}));
            }
            Done::Saved { result } => {
                match &result {
                    Ok(v) => self.note(trf("studio.saved", &[("path", &v["out"].as_str().unwrap_or(""))])),
                    Err(e) => self.note(trf("designer.save_failed", &[("error", &e)])),
                }
                self.saved = Some(result);
            }
        }
    }

    fn want_slope(&mut self, name: &str) {
        let Some(macros) = self.preview.as_ref().map(|p| p.anchor_macros.clone()) else { return };
        self.slope_job(name, &macros, true);
    }

    fn slope_job(&mut self, name: &str, macros: &BTreeMap<String, f64>, urgent: bool) {
        let (Some(s), Some(p), Some(shape)) = (&self.session, &self.preview, &self.shape) else { return };
        let key = macro_key(macros);
        let id = (name.to_string(), key.clone());
        if !shape.macros_available || !macros.contains_key(name) || p.has_slope(name, &key) || self.slope_wanted.contains(&id) || (key == p.slope_key && p.sampled_for(name).is_some()) {
            return;
        }
        let base = shape.character(&s.base(), &shape.live);
        let mut input = s.settle_input(&base, false);
        input["macros"] = json!({"sliders": macros, "mode": shape.macro_mode, "head_scale": shape.head_scale});
        input["slider"] = json!(name);
        if let Some(m) = &s.macro_file {
            input["macro"] = json!(m);
        }
        input.as_object_mut().map(|o| o.remove("uvs"));
        self.slope_wanted.insert(id);
        self.jobs.send(self.epoch, Job::Slope { name: name.to_string(), key, input, urgent });
    }

    fn prefetch_slopes(&mut self) {
        let Some(macros) = self.preview.as_ref().map(|p| p.anchor_macros.clone()) else { return };
        self.prefetch_slopes_at(&macros);
    }

    fn prefetch_slopes_at(&mut self, macros: &BTreeMap<String, f64>) {
        for name in self.prefetch.clone() {
            self.slope_job(&name, macros, false);
        }
    }

    fn sample_input(&self, name: &str) -> Option<(Value, String)> {
        let (s, p, shape, values) = (self.session.as_ref()?, self.preview.as_ref()?, self.shape.as_ref()?, self.anchor_values.as_ref()?);
        if !shape.macros_available || !p.anchor_macros.contains_key(name) {
            return None;
        }
        let mut input = s.settle_input(&shape.character(&s.base(), values), false);
        input["macros"] = json!({"sliders": p.anchor_macros, "mode": shape.macro_mode, "head_scale": shape.head_scale});
        if let Some(m) = &s.macro_file {
            input["macro"] = json!(m);
        }
        let mut keyed = input.clone();
        keyed["macros"]["sliders"].as_object_mut()?.remove(name);
        input["slider"] = json!(name);
        input["sampled"] = json!(true);
        Some((input, format!("{name} {keyed}")))
    }

    fn refresh_state_keys(&mut self) {
        let names: Vec<String> = self.preview.as_ref().map(|p| p.anchor_macros.keys().cloned().collect()).unwrap_or_default();
        let keys: Vec<(String, String)> = names.into_iter().filter_map(|n| self.sample_input(&n).map(|(_, k)| (n, k))).collect();
        if let Some(p) = self.preview.as_mut() {
            p.state_keys = keys.into_iter().collect();
            if let Some((id, name, key)) = &self.samples_wanted
                && p.state_keys.get(name) != Some(key)
            {
                self.jobs.drop_samples(*id);
            }
        }
    }

    fn samples_job(&mut self, name: &str) {
        let Some(p) = &self.preview else { return };
        if p.sampled_for(name).is_some() || self.samples_wanted.is_some() {
            return;
        }
        let Some((input, key)) = self.sample_input(name) else { return };
        self.samples_seq += 1;
        self.samples_wanted = Some((self.samples_seq, name.to_string(), key.clone()));
        self.jobs.send(self.epoch, Job::Samples { id: self.samples_seq, name: name.to_string(), key, input });
    }

    fn prefetch_samples(&mut self) {
        let mut names: Vec<String> = self.last_preview["slope_fallback"].as_array().map(|f| f.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        names.extend(self.prefetch.clone());
        for name in names {
            self.samples_job(&name);
        }
    }

    pub fn samples_ready(&self, name: &str) -> bool {
        self.preview.as_ref().is_some_and(|p| p.sampled_for(name).is_some())
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
        let ends = matches!(ev, ShapeEvent::Commit(_) | ShapeEvent::Reset);
        if matches!(ev, ShapeEvent::Begin(_)) {
            self.jobs.set_dragging(true);
        }
        let out = shape.apply(ev);
        if out.preview {
            self.preview_dirty = true;
            self.edit_at.get_or_insert_with(Instant::now);
            if self.face.rig.is_some() && self.poses.iter().any(|(k, _)| k == "rig") {
                self.poses.retain(|(k, _)| k != "rig");
                self.face.overlay_off = true;
                self.face.status = tr("rig.changed").into();
            }
        }
        let macros: Vec<String> = shape.macros.iter().map(|m| m.name.clone()).filter(|n| out.begin.contains(n)).collect();
        for m in macros {
            self.want_slope(&m);
        }
        if out.preview {
            self.stitch_changed(true);
        }
        if out.settle {
            self.request_settle();
        }
        if ends {
            self.jobs.set_dragging(false);
        }
    }

    pub fn release_drag(&self) {
        self.jobs.set_dragging(false);
    }

    pub fn request_settle(&mut self) {
        let (Some(s), Some(shape)) = (self.session.as_mut(), &self.shape) else { return };
        let c = shape.character(&s.base(), &shape.live);
        let input = s.settle_input(&c, true);
        s.character = c;
        let now = Instant::now();
        let macros = shape.live.macros.clone();
        self.jobs.settle(SettleRequest { epoch: self.epoch, generation: shape.generation, sent: shape.live.clone(), input, at: now, edges: self.lod0.clone() });
        self.settle_wanted = Some(shape.generation);
        self.settle_release = Some(now);
        self.settle_error = None;
        self.prefetch_slopes_at(&macros);
    }

    pub fn settle_input(&self) -> Option<Value> {
        let (s, shape) = (self.session.as_ref()?, self.shape.as_ref()?);
        Some(s.settle_input(&shape.character(&s.base(), &shape.live), true))
    }

    pub fn settle_stats(&self) -> Value {
        let st = self.jobs.settle_state();
        json!({"queued": st.queued, "running": st.running, "cancelled_before_start": st.cancelled, "aborted_while_running": st.aborted, "requeued_after_drag": st.requeued,
               "stale_applied_as_anchor": self.settles_stale})
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
        s.preview_eyes(shape.live.eyeballs.as_ref());
        let total = ms(t.elapsed());
        self.hud.preview_compute.push(total);
        self.last_preview = json!({"targets": c.targets, "macros": c.macros, "missing_slopes": c.missing_slopes, "sampled": c.sampled, "slope_fallback": c.slope_fallback, "stale_slope": c.stale_slope,
                                   "positions_ms": ms(t_n - t), "normals_ms": ms(t_n.elapsed()), "total_ms": total});
        for m in c.missing_slopes.iter().chain(&c.stale_slope) {
            self.want_slope(m);
        }
        for m in c.slope_fallback {
            self.samples_job(&m);
        }
        if let Some(e) = self.edit_at.take() {
            self.latency_mark = Some(e);
        }
        if !self.poses.is_empty() {
            self.pose_dirty = true;
        }
    }

    pub fn frame_done(&mut self, at: Instant) {
        if let Some(m) = self.latency_mark.take() {
            self.hud.preview_latency.push(ms(at - m));
        }
    }

    pub fn shape_step(&mut self, redo: bool) -> Option<String> {
        let shape = self.shape.as_mut()?;
        let label = if redo { shape.redo() } else { shape.undo() }?;
        self.preview_dirty = true;
        self.edit_at.get_or_insert_with(Instant::now);
        self.request_settle();
        Some(label)
    }

    pub fn save(&mut self, out: &str) -> Result<(), String> {
        let mut input = self.settle_input().ok_or(tr("studio.nothing_loaded"))?;
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
        let toggle = |st: &mut ViewerState, path: &str, label: &str| st.opts.toggle(path).map(|on| trf(if on { "designer.on" } else { "designer.off" }, &[("what", &label)]));
        Ok(match id {
            "frame" => {
                self.frame_head(instant);
                tr("studio.framed").into()
            }
            "layout" => {
                st.layout = st.layout.next();
                st.active = st.active.min(st.layout.slots().len() - 1);
                trf("studio.layout_status", &[("name", &tr(&format!("studio.layout.{}", st.layout.name())))])
            }
            "next_viewport" => {
                st.active = (st.active + 1) % st.layout.slots().len();
                st.pivot_flash = 0.4;
                trf("studio.viewport_n", &[("n", &(st.active + 1))])
            }
            "mode_next" | "mode_prev" => {
                let slot = st.active.min(st.layout.slots().len() - 1);
                let now = st.vp[slot].mode;
                let next = if id == "mode_next" { (now + 1) % MODES.len() } else { (now + MODES.len() - 1) % MODES.len() };
                self.set_mode(slot, next);
                tr(&format!("mode.{}", MODES[next])).to_string()
            }
            "wire" => toggle(st, "wire", tr("designer.opt.wire"))?,
            "xray" => toggle(st, "xray", tr("designer.opt.xray"))?,
            "points" => toggle(st, "points", tr("designer.opt.points"))?,
            "back_faces" => toggle(st, "backHi", tr("designer.opt.back"))?,
            "seams" => toggle(st, "seams", tr("designer.opt.seams"))?,
            "clip" => toggle(st, "clip.0.on", tr("designer.opt.clip"))?,
            "light_gizmo" => toggle(st, "lightGizmo", tr("designer.opt.gizmo"))?,
            "theme" => toggle(st, "dark", tr("studio.opt.dark"))?,
            "bad_triangles" => toggle(st, "badTriangles", tr("studio.opt.bad"))?,
            "fov_down" | "fov_up" => {
                let f = st.active_cam().goal.fov + if id == "fov_up" { 5.0 } else { -5.0 };
                st.set_fov(f, instant);
                trf("studio.fov", &[("deg", &format!("{:.0}", st.active_cam().goal.fov))])
            }
            "lod_coarser" | "lod_finer" => {
                let n = self.lods();
                let s = self.state.active_slot();
                s.lod = if id == "lod_coarser" { (s.lod + 1).min(n - 1) } else { s.lod.saturating_sub(1) };
                trf("studio.lod_status", &[("n", &s.lod)])
            }
            "view_front" | "view_back" | "view_left" | "view_right" | "view_top" | "view_bottom" | "view_three_quarter" => {
                let name = id.trim_start_matches("view_").replace('_', "");
                st.named_view(&name, instant);
                trf("studio.view_status", &[("view", &tr_or(&format!("view.{name}"), &name))])
            }
            "view_home" => {
                st.home_view(instant);
                self.frame_head(instant);
                tr("studio.home").into()
            }
            "eyes" => toggle(st, "eyes", tr("studio.opt.eyes"))?,
            "ortho" => {
                let cam = st.active_cam();
                cam.ortho = !cam.ortho;
                if instant {
                    cam.snap();
                }
                if cam.ortho { tr("studio.orthographic").into() } else { tr("view.perspective").into() }
            }
            "undo" | "redo" => self.undo(id == "redo").unwrap_or_else(|| if id == "redo" { tr("studio.nothing_redo").into() } else { tr("studio.nothing_undo").into() }),
            "hud" => {
                self.hud.on = !self.hud.on;
                if self.hud.on { tr("studio.timings_shown").into() } else { tr("studio.timings_hidden").into() }
            }
            "reset_shape" => {
                self.shape_event(ShapeEvent::Reset);
                tr("designer.undo.reset").into()
            }
            other => return Err(format!("unknown action {other}")),
        })
    }
}
