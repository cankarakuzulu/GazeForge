use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use eframe::egui;
use gf_api::Ctx;
use gf_geom::topology::min_angle;
use serde_json::{Value, json};

use crate::icons;
use crate::session::{Head, Lod, run_settle, vertex_normals};
use crate::studio::Studio;
use crate::world::World;

pub const BAD_FLAG: u32 = 64;
const POLL: Duration = Duration::from_millis(250);

pub struct TopologyUi {
    pub below_deg: f64,
    pub bad_counts: Vec<usize>,
    flagged: Option<(u64, u64, bool, bool)>,
    inspect: Option<Receiver<Result<Value, String>>>,
    pub info: Option<Result<Value, String>>,
    info_game: String,
    pub job: Option<String>,
    polled: Option<Instant>,
    pub record: Option<Value>,
    pub error: Option<String>,
    after_rx: Option<Receiver<Result<Look, String>>>,
    after: Option<Look>,
    before: Option<Look>,
    after_revision: u64,
    session_epoch: u64,
    pub showing_after: bool,
    pub after_game: String,
    pub job_source: String,
    pub shapes_dir: String,
}

#[derive(Clone)]
pub struct Look {
    lods: Vec<Lod>,
    positions: Vec<f32>,
    normals: Vec<f32>,
}

impl Default for TopologyUi {
    fn default() -> TopologyUi {
        TopologyUi {
            below_deg: 10.0,
            bad_counts: Vec::new(),
            flagged: None,
            inspect: None,
            info: None,
            info_game: String::new(),
            job: None,
            polled: None,
            record: None,
            error: None,
            after_rx: None,
            after: None,
            before: None,
            after_revision: 0,
            session_epoch: 0,
            showing_after: false,
            after_game: String::new(),
            job_source: String::new(),
            shapes_dir: String::new(),
        }
    }
}

fn err(e: gf_api::ApiError) -> String {
    e.message
}

pub fn flag_bad(head: &mut Head, below: f64, show: bool) -> Vec<usize> {
    let p = |i: u32| {
        let k = i as usize * 3;
        [head.positions[k] as f64, head.positions[k + 1] as f64, head.positions[k + 2] as f64]
    };
    let mut counts = Vec::with_capacity(head.lods.len());
    let flags: Vec<Vec<u32>> = head
        .lods
        .iter()
        .map(|l| {
            let mut n = 0;
            let out = l
                .indices
                .chunks_exact(3)
                .zip(&l.flags)
                .map(|(t, &f)| {
                    let bad = min_angle([p(t[0]), p(t[1]), p(t[2])]) < below;
                    n += bad as usize;
                    if bad && show { f | BAD_FLAG } else { f & !BAD_FLAG }
                })
                .collect();
            counts.push(n);
            out
        })
        .collect();
    for (l, f) in head.lods.iter_mut().zip(flags) {
        l.flags = f;
    }
    counts
}

fn look_of(store: &str, pack: &str, catalog: &str, game: &str, mut settle: Value) -> Result<Look, String> {
    let scene = gf_api::designer::scene_mesh(&json!({"store": store, "pack": pack, "catalog": catalog, "game": game})).map_err(err)?;
    let mut lods = vec![Lod::new("LOD0".into(), scene.triangles.iter().flatten().copied().collect(), scene.triangle_uvs.iter().flatten().flatten().copied().collect(), 0)];
    for l in &scene.lods {
        lods.push(Lod::new(format!("LOD {}", l.target), l.indices.clone(), l.triangle_uvs.iter().map(|&x| x as f32).collect(), l.dropped_triangles));
    }
    settle["game"] = json!(game);
    let placed = run_settle(&settle)?;
    if placed.positions.len() != scene.vertices * 3 {
        return Err(format!("the settle on {game} returned {} vertices, the base has {}", placed.positions.len() / 3, scene.vertices));
    }
    if let Some(uvs) = placed.uvs.filter(|u| u.len() == lods[0].uvs.len()) {
        lods[0].set_uvs(uvs);
    }
    let normals = vertex_normals(&placed.positions, &lods[0].indices, scene.vertices);
    Ok(Look { lods, positions: placed.positions, normals })
}

fn short(reference: &str) -> String {
    let hex = reference.strip_prefix("sha256:").unwrap_or(reference);
    format!("sha256:{}", &hex[..hex.len().min(10)])
}

fn stat(v: &Value, path: &str) -> String {
    match v.pointer(path) {
        Some(Value::Number(n)) if n.is_f64() => format!("{:.2}", n.as_f64().unwrap_or(0.0)),
        Some(x) if !x.is_null() => x.to_string(),
        _ => "-".into(),
    }
}

impl TopologyUi {
    pub fn result(&self) -> Option<&Value> {
        self.record.as_ref().filter(|r| r["state"] == "done").map(|r| &r["result"]["result"])
    }

    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    pub fn sync(&mut self, world: &mut World, show: bool) {
        if self.session_epoch != world.epoch {
            self.session_epoch = world.epoch;
            self.reset();
            if self.job.is_none() && world.session.as_ref().is_none_or(|s| s.game != self.job_source) {
                self.record = None;
                self.error = None;
                self.after_game.clear();
            }
        }
        let Some(s) = world.session.as_mut() else {
            self.flagged = None;
            return;
        };
        if self.info_game != s.game && self.inspect.is_none() {
            self.info_game = s.game.clone();
            self.info = None;
            let (store, game) = (s.store.clone(), s.game.clone());
            let (tx, rx) = channel();
            std::thread::spawn(move || {
                let mut o = gf_api::tasks::Options::new();
                o.insert("store".into(), store);
                o.insert("game".into(), game);
                let _ = tx.send(gf_api::tasks::topology::inspect(&o));
            });
            self.inspect = Some(rx);
        }
        if let Some(rx) = &self.inspect
            && let Ok(r) = rx.try_recv()
        {
            self.info = Some(r);
            self.inspect = None;
        }
        if let Some(rx) = &self.after_rx
            && let Ok(r) = rx.try_recv()
        {
            self.after_rx = None;
            match r {
                Ok(look) if look.lods.len() == s.head.lods.len() && look.positions.len() == s.head.positions.len() => {
                    self.after = Some(look);
                    self.swap(world, true);
                    return;
                }
                Ok(look) => self.error = Some(format!("the improved base has {} levels, the loaded one {}", look.lods.len(), s.head.lods.len())),
                Err(e) => self.error = Some(e),
            }
        }
        if self.showing_after && s.head.revision != self.after_revision {
            self.showing_after = false;
            if let Some(before) = self.before.take() {
                s.head.lods = before.lods;
                s.head.normals = vertex_normals(&s.head.positions, &s.head.lods[0].indices, s.head.n);
            }
            self.flagged = None;
            world.epoch_bump_head();
            return;
        }
        let key = (s.head.revision, (self.below_deg * 1000.0) as u64, show, self.showing_after);
        if self.flagged != Some(key) {
            self.bad_counts = flag_bad(&mut s.head, self.below_deg, show);
            self.flagged = Some(key);
            world.uvs_dirty = true;
        }
    }

    pub fn start(&mut self, store: &str, game: &str) -> Result<String, String> {
        if self.job.is_some() {
            return Err("an improve is already running".into());
        }
        let mut input = json!({"store": store, "game": game});
        if !self.shapes_dir.trim().is_empty() {
            input["dense"] = json!(self.shapes_dir.trim());
        }
        let env = gf_api::call(&crate::wizard::ctx()?, "jobs.submit", &json!({"command": "topology.improve", "input": input}));
        if env["ok"] != true {
            return Err(env.pointer("/error/message").and_then(Value::as_str).unwrap_or("jobs.submit failed").to_string());
        }
        let id = env["result"]["id"].as_str().unwrap_or_default().to_string();
        self.job = Some(id.clone());
        self.job_source = game.to_string();
        self.record = None;
        self.error = None;
        self.polled = None;
        Ok(format!("improving the topology (job {id})"))
    }

    pub fn poll(&mut self) -> Option<String> {
        let id = self.job.clone()?;
        if self.polled.is_some_and(|t| t.elapsed() < POLL) {
            return None;
        }
        self.polled = Some(Instant::now());
        let env = gf_api::call(&crate::wizard::ctx().unwrap_or_else(|_| Ctx::from_env()), "jobs.status", &json!({"id": id}));
        if env["ok"] != true {
            self.job = None;
            self.error = Some(env.pointer("/error/message").and_then(Value::as_str).unwrap_or("jobs.status failed").to_string());
            return self.error.clone();
        }
        let record = env["result"].clone();
        let state = record["state"].as_str().unwrap_or("").to_string();
        self.record = Some(record);
        if ["done", "failed", "cancelled"].contains(&state.as_str()) {
            self.job = None;
            if state != "done" {
                let r = self.record.as_ref().expect("set above");
                let message = r.pointer("/result/error/message").or(r.pointer("/error/message")).and_then(Value::as_str).unwrap_or("the job failed");
                self.error = Some(message.to_string());
                return Some(format!("topology improve {state}: {message}"));
            }
            return Some(format!("topology improved: new game base {}", self.result().and_then(|r| r["game"].as_str()).map(short).unwrap_or_default()));
        }
        None
    }

    pub fn wait(&mut self, seconds: f64) -> Result<(), String> {
        let until = Instant::now() + Duration::from_secs_f64(seconds);
        while self.job.is_some() {
            if Instant::now() > until {
                return Err(format!("the improve job did not finish in {seconds} s"));
            }
            self.polled = None;
            self.poll();
            std::thread::sleep(Duration::from_millis(100));
        }
        match &self.error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    pub fn compare(&mut self, world: &mut World, after: bool) -> Result<String, String> {
        let s = world.session.as_ref().ok_or("nothing is loaded")?;
        if !after {
            self.swap(world, false);
            return Ok("triangulation: before".into());
        }
        let game = self.result().and_then(|r| r["game"].as_str()).ok_or("run an improve first")?.to_string();
        if s.game != self.job_source {
            return Err(format!("that improve was of another game base ({}); run an improve on this one", short(&self.job_source)));
        }
        if self.after.is_some() && self.after_game == game {
            self.swap(world, true);
            return Ok("triangulation: after".into());
        }
        if self.after_rx.is_some() {
            return Ok("loading the improved triangulation".into());
        }
        let (store, pack, catalog) = (s.store.clone(), s.pack.clone(), s.catalog.clone());
        let settle = s.settle_input(&s.character, true);
        self.after_game = game.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(look_of(&store, &pack, &catalog, &game, settle));
        });
        self.after_rx = Some(rx);
        Ok("loading the improved triangulation".into())
    }

    pub fn wait_after(&mut self, world: &mut World, seconds: f64) -> Result<(), String> {
        let until = Instant::now() + Duration::from_secs_f64(seconds);
        while self.after_rx.is_some() {
            if Instant::now() > until {
                return Err("the improved triangulation did not load".into());
            }
            self.sync(world, false);
            std::thread::sleep(Duration::from_millis(50));
        }
        match &self.error {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    fn swap(&mut self, world: &mut World, after: bool) {
        let Some(s) = world.session.as_mut() else { return };
        if after == self.showing_after {
            return;
        }
        let Some(look) = (if after { self.after.clone() } else { self.before.take() }) else { return };
        let old = Look {
            lods: std::mem::replace(&mut s.head.lods, look.lods),
            positions: std::mem::replace(&mut s.head.positions, look.positions),
            normals: std::mem::replace(&mut s.head.normals, look.normals),
        };
        if after {
            self.before = Some(old);
        }
        s.head.revision += 1;
        self.after_revision = s.head.revision;
        self.showing_after = after;
        self.flagged = None;
        world.epoch_bump_head();
    }

    pub fn reset(&mut self) {
        self.after = None;
        self.before = None;
        self.showing_after = false;
        self.after_rx = None;
        self.flagged = None;
    }

    pub fn summary(&self) -> Value {
        json!({"below_deg": self.below_deg, "bad_counts": self.bad_counts, "info": self.info.as_ref().map(|r| r.as_ref().map(|v| json!({"game": v["game"], "rest": v["rest"], "topology": v["topology"].get("rest")})).unwrap_or_else(|e| json!({"error": e}))),
               "job": self.job, "state": self.record.as_ref().map(|r| r["state"].clone()), "progress": self.record.as_ref().map(|r| r["progress"].clone()),
               "error": self.error, "showing_after": self.showing_after, "after_game": self.after_game,
               "result": self.result().map(|r| json!({"game": r["game"], "levels": r["levels"].as_array().map(|ls| ls.iter().map(|l| json!({"level": l["level"], "game": l["game"], "flips": l["flips"], "rest": l["rest"], "shape_range": l["shape_range"]})).collect::<Vec<_>>())}))})
    }
}

impl Studio<'_> {
    pub fn topology_panel(&mut self, ui: &mut egui::Ui) {
        let loaded = self.world.session.is_some();
        egui::CollapsingHeader::new("Topology").id_salt("topology").default_open(false).show(ui, |ui| {
            if !loaded {
                ui.label(egui::RichText::new("load a store first").weak());
                return;
            }
            let o = &mut self.world.state.opts;
            ui.horizontal(|ui| {
                ui.checkbox(&mut o.bad, "show bad triangles").on_hover_text(format!("Triangles whose smallest angle is under the limit, in magenta ({})", self.s.keymap.describe("bad_triangles")));
                ui.add(egui::Slider::new(&mut self.world.topo.below_deg, 2.0..=30.0).suffix(" deg").fixed_decimals(0)).on_hover_text("the smallest-angle limit");
            });
            let t = &self.world.topo;
            if let Some(n) = t.bad_counts.first() {
                let on = if t.showing_after { "the improved base" } else { "the loaded base" };
                ui.label(egui::RichText::new(format!("{n} triangles under {:.0} deg on this character, settled on {on} (LOD0)", t.below_deg)).weak());
            }
            match &t.info {
                None => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("measuring the base").weak());
                    });
                }
                Some(Err(e)) => {
                    ui.colored_label(ui.visuals().error_fg_color, e);
                }
                Some(Ok(v)) => {
                    ui.label(format!("base {} at rest: {} under 5 deg, {} under 10 deg, p1 {} deg", short(v["game"].as_str().unwrap_or("")), stat(v, "/rest/below_5"), stat(v, "/rest/below_10"), stat(v, "/rest/min_angle_p1")));
                    if let Some(from) = v.pointer("/topology/from").and_then(Value::as_str) {
                        ui.label(egui::RichText::new(format!("improved from {}: {} flips", short(from), stat(v, "/topology/flips"))).weak());
                    }
                }
            }
            ui.horizontal(|ui| {
                let running = self.world.topo.running();
                let tip = format!("Re-cut the triangles of this game base by edge flips over the same vertices, scored over the shape range, and store the result as a new game base ({})", self.s.keymap.describe("topology_improve"));
                if ui.add_enabled_ui(!running, |ui| icons::button(ui, "wand-sparkles", running, &tip)).inner.clicked() {
                    self.act("topology_improve");
                }
                ui.label(if running { "improving" } else { "improve" });
            });
            ui.horizontal(|ui| {
                ui.label("extra shapes").on_hover_text("Optional: a dense base folder whose shape keys are scored too (its manifest.json and shape_NAME.npy files); the result depends on the shapes scored");
                ui.add_enabled(!self.world.topo.running(), egui::TextEdit::singleline(&mut self.world.topo.shapes_dir).hint_text("dense base folder (optional)").desired_width(180.0));
            });
            let t = &self.world.topo;
            if let Some(r) = &t.record
                && t.job.is_some()
            {
                let p = &r["progress"];
                let (done, total) = (p["done"].as_f64().unwrap_or(0.0), p["total"].as_f64().unwrap_or(1.0).max(1.0));
                ui.add(egui::ProgressBar::new((done / total) as f32).text(format!("{} {}/{}", p["stage"].as_str().unwrap_or(r["state"].as_str().unwrap_or("queued")), done, total)));
                if let Some(m) = p["message"].as_str().filter(|m| !m.is_empty()) {
                    ui.label(egui::RichText::new(m).weak());
                }
                ui.ctx().request_repaint_after(POLL);
            }
            if let Some(e) = &t.error {
                ui.colored_label(ui.visuals().error_fg_color, e);
            }
            let result = t.result().cloned();
            if let Some(r) = result {
                let l = &r["levels"][0];
                egui::Grid::new("topology_numbers").num_columns(3).striped(true).spacing([10.0, 2.0]).show(ui, |ui| {
                    ui.label(egui::RichText::new("LOD0").strong());
                    ui.label("before");
                    ui.label("after");
                    ui.end_row();
                    for (label, path) in [
                        ("rest under 5 deg", "/rest/{}/below_5"),
                        ("rest under 10 deg", "/rest/{}/below_10"),
                        ("rest smallest p1", "/rest/{}/min_angle_p1"),
                        ("range under 5 deg", "/shape_range/{}/below_5"),
                        ("range under 10 deg", "/shape_range/{}/below_10"),
                        ("range smallest p1", "/shape_range/{}/min_angle_p1"),
                        ("range turned over", "/shape_range/{}/turned"),
                    ] {
                        ui.label(label);
                        ui.label(stat(l, &path.replace("{}", "before")));
                        ui.label(stat(l, &path.replace("{}", "after")));
                        ui.end_row();
                    }
                });
                ui.label(egui::RichText::new(format!("{} flips over {} scored shapes; new base {}", stat(l, "/flips"), stat(&r, "/shape_set/total"), short(r["game"].as_str().unwrap_or("")))).weak());
                ui.horizontal(|ui| {
                    ui.label("triangulation");
                    let mut after = self.world.topo.showing_after;
                    ui.selectable_value(&mut after, false, "before");
                    ui.selectable_value(&mut after, true, "after");
                    if after != self.world.topo.showing_after {
                        self.act(if after { "topology_after" } else { "topology_before" });
                    }
                });
                if ui.button("design on the improved base").on_hover_text("Reload the studio on the new game base (the character keeps its weights; saving it names the new base)").clicked() {
                    self.act("topology_use");
                }
            }
        });
    }

    pub fn topology_act(&mut self, id: &str) -> Option<String> {
        Some(match id {
            "topology_improve" => {
                let Some(s) = self.world.session.as_ref() else { return Some("load a store first".into()) };
                let (store, game) = (s.store.clone(), s.game.clone());
                self.world.topo.start(&store, &game).unwrap_or_else(|e| e)
            }
            "topology_after" | "topology_before" => self.world.with_topo(|t, w| t.compare(w, id == "topology_after")).unwrap_or_else(|e| e),
            "topology_use" => match self.world.topo.result().and_then(|r| r["game"].as_str()).map(str::to_string) {
                Some(g) => {
                    self.s.game = g.clone();
                    self.world.topo.reset();
                    self.load();
                    format!("loading the improved base {}", short(&g))
                }
                None => "run an improve first".into(),
            },
            _ => return None,
        })
    }
}
