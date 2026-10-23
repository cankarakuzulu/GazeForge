use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

use eframe::egui;
use gf_api::Ctx;
use serde_json::{Map, Value, json};

use crate::browse::{self, Browse, Pick};
use crate::i18n::{tr, tr_or, trf};

const POLL: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct Wizards {
    pub open: bool,
    list: Option<Vec<Value>>,
    def: Option<Value>,
    state: Option<Value>,
    prepared: Map<String, Value>,
    at: usize,
    shown_for: Option<(usize, String)>,
    text: BTreeMap<String, String>,
    flags: BTreeMap<String, bool>,
    error: Option<String>,
    progress: Option<Value>,
    since: Option<(String, Instant)>,
    polled: Option<Instant>,
    images: BTreeMap<String, Option<egui::TextureHandle>>,
    pub open_spec: Option<(String, String)>,
    developer: bool,
    jobs_cache: Option<(Instant, Value)>,
    store_cache: Option<(String, Value)>,
    runs: Option<Vec<Value>>,
    pending: Option<Receiver<Result<Value, String>>>,
    last_glb: Option<String>,
    glb_epoch: u64,
    glb_live: bool,
    browse: Browse,
}

fn progress_text(p: &Value, secs: u64) -> String {
    let stage = p["stage"].as_str().filter(|s| !s.is_empty());
    let message = p["message"].as_str().filter(|s| !s.is_empty());
    let counted = match (p["done"].as_f64(), p["total"].as_f64()) {
        (Some(d), Some(t)) if t > 0.0 => Some(match stage {
            Some(s) => trf("wizard.progress_of", &[("stage", &s), ("done", &d), ("total", &t)]),
            None => trf("wizard.progress_count", &[("done", &d), ("total", &t)]),
        }),
        _ => None,
    };
    match (counted, stage) {
        (Some(c), _) => message.map_or(c.clone(), |m| format!("{c} · {m}")),
        (None, Some(s)) => trf("wizard.stage_for", &[("stage", &s), ("s", &secs)]),
        (None, None) => trf("wizard.running_for", &[("s", &secs)]),
    }
}

fn load_png(path: &str) -> Option<egui::ColorImage> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let bytes = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => bytes.to_vec(),
        png::ColorType::Rgb => bytes.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}

pub fn run_values(world: &crate::world::World) -> Value {
    match &world.session {
        Some(s) => json!({"store": s.store, "pack": s.pack, "catalog": s.catalog, "game": s.game, "macro": s.macro_file}),
        None => json!({}),
    }
}

fn show_in_folder(path: &str) {
    let p = std::path::Path::new(path);
    let _ = if cfg!(windows) {
        std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn()
    } else {
        std::process::Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" }).arg(p.parent().unwrap_or(p)).spawn()
    };
}

pub fn ctx() -> Result<Ctx, String> {
    let mut ctx = Ctx::from_env();
    let name = if cfg!(windows) { "gazeforge.exe" } else { "gazeforge" };
    let cli = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name(name);
    if cli.is_file() {
        ctx.exe = cli;
    }
    Ok(ctx)
}

fn call(name: &str, input: Value) -> Result<Value, String> {
    let ctx = ctx()?;
    let env = gf_api::call(&ctx, name, &input);
    if env["ok"] == Value::Bool(true) {
        Ok(env["result"].clone())
    } else {
        Err(env["error"]["message"].as_str().unwrap_or("the command failed").to_string())
    }
}

pub(crate) fn humanize(name: &str) -> String {
    let s = name.replace('_', " ");
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
}

fn shown(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

impl Wizards {
    pub fn state(&self) -> Value {
        json!({"open": self.open, "wizard": self.def.as_ref().map(|d| d["id"].clone()), "at": self.at, "error": self.error, "run": self.state})
    }

    fn remember_glb(&mut self) {
        let Some(st) = self.state.as_ref().filter(|_| self.glb_live) else { return };
        if self.def.as_ref().is_none_or(|d| d["id"] != "export") {
            return;
        }
        let found = st["steps"].as_array().into_iter().flatten().filter_map(|s| s["result"]["out"].as_str()).rfind(|o| o.ends_with(".glb"));
        if let Some(o) = found {
            self.last_glb = Some(o.to_string());
        }
    }

    pub fn start(&mut self, id: &str, mut values: Value, character: Option<Value>, project: &str) -> Result<(), String> {
        if self.pending.is_some() {
            return Err(tr("wizard.already_running").into());
        }
        if let Some(glb) = &self.last_glb {
            values["glb"] = json!(glb);
        }
        let store = values["store"].as_str().unwrap_or_default().to_string();
        let def = call("wizards.get", json!({"id": id}))?;
        let mut state = call("wizards.start", json!({"wizard": id, "project": project, "values": values}))?;
        let dir = PathBuf::from(state["dir"].as_str().unwrap_or_default());
        let mut prepared = Map::new();
        if let Some(mut input) = character {
            let spec = dir.join(gf_api::wizards::SAVED_CHARACTER).display().to_string();
            input["out"] = json!(spec);
            input.as_object_mut().map(|o| o.remove("uvs"));
            call("designer.save", input)?;
            prepared.insert("character".into(), json!({"store": store, "spec": spec}));
            state = call("wizards.state", json!({"run": state["run"]}))?;
        }
        prepared.insert("export".into(), json!({"out": dir.join("export.glb").display().to_string()}));
        self.def = Some(def);
        self.state = Some(state);
        self.glb_live = true;
        self.prepared = prepared;
        self.at = 0;
        self.shown_for = None;
        self.error = None;
        Ok(())
    }

    pub fn resume(&mut self, wizard: &str, run: &str) -> Result<(), String> {
        let def = call("wizards.get", json!({"id": wizard}))?;
        let state = call("wizards.state", json!({"run": run}))?;
        let steps = state["steps"].as_array().cloned().unwrap_or_default();
        self.at = steps.iter().position(|s| s["state"] != "done").unwrap_or(steps.len().saturating_sub(1));
        self.def = Some(def);
        self.state = Some(state);
        self.glb_live = true;
        self.prepared = Map::new();
        self.shown_for = None;
        self.error = None;
        Ok(())
    }

    fn step_def(&self, i: usize) -> &Value {
        &self.def.as_ref().expect("a wizard")["steps"][i]
    }

    fn step_state(&self, i: usize) -> &Value {
        &self.state.as_ref().expect("a run")["steps"][i]
    }

    fn value_of(&self, i: usize, name: &str) -> Value {
        let st = self.step_state(i);
        for key in ["input", "proposed"] {
            if let Some(v) = st[key].get(name).filter(|v| !v.is_null()) {
                return v.clone();
            }
        }
        self.prepared.get(self.step_def(i)["id"].as_str().unwrap_or_default()).and_then(|p| p.get(name)).cloned().unwrap_or(Value::Null)
    }

    fn fields(&self, i: usize) -> Vec<(String, String, bool, Value)> {
        let s = self.step_def(i);
        let props = s["input_schema"]["properties"].as_object().cloned().unwrap_or_default();
        let defined: Vec<&Value> = s["fields"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
        let mut out: Vec<(String, String, bool, Value)> = defined
            .iter()
            .filter(|f| f["hidden"] != Value::Bool(true))
            .map(|f| {
                let name = f["name"].as_str().unwrap_or_default().to_string();
                let schema = props.get(&name).cloned().or_else(|| f.get("schema").cloned()).unwrap_or_else(|| json!({"type": "string"}));
                (name.clone(), tr_or(&format!("wizard.{}.{}.{name}", self.def.as_ref().map_or("", |d| d["id"].as_str().unwrap_or("")), s["id"].as_str().unwrap_or("")), f["label"].as_str().unwrap_or(&name)).to_string(), f["common"] == Value::Bool(true), schema)
            })
            .collect();
        for (name, schema) in props {
            if self.developer && !defined.iter().any(|f| f["name"] == name.as_str()) {
                out.push((name.clone(), tr_or(&format!("wizard.field.{name}"), &humanize(&name)).to_string(), false, schema));
            }
        }
        out
    }

    pub fn load_form(&mut self) {
        let key = (self.at, self.step_state(self.at)["state"].as_str().unwrap_or_default().to_string());
        if self.shown_for.as_ref() == Some(&key) {
            return;
        }
        self.text.clear();
        self.flags.clear();
        for (name, _, _, schema) in self.fields(self.at) {
            let v = self.value_of(self.at, &name);
            if schema["type"] == "boolean" {
                self.flags.insert(name, v.as_bool().unwrap_or(false));
            } else {
                self.text.insert(name, shown(&v));
            }
        }
        self.shown_for = Some(key);
    }

    fn input(&self) -> Result<Value, String> {
        let mut out = Map::new();
        for (name, _, _, schema) in self.fields(self.at) {
            if schema["type"] == "boolean" {
                if self.flags.get(&name).copied().unwrap_or(false) {
                    out.insert(name, json!(true));
                }
                continue;
            }
            let t = self.text.get(&name).map(|s| s.trim().to_string()).unwrap_or_default();
            if t.is_empty() {
                continue;
            }
            let structured = t.starts_with('{') || t.starts_with('[');
            let v = match schema["type"].as_str() {
                _ if schema["enum"].is_array() => json!(t),
                Some("integer") => json!(t.parse::<i64>().map_err(|_| trf("wizard.not_number", &[("field", &name), ("text", &t)]))?),
                Some("number") => json!(t.parse::<f64>().map_err(|_| trf("wizard.not_number", &[("field", &name), ("text", &t)]))?),
                Some("object") | Some("array") => gf_api::exactjson::text(&t).map_err(|e| trf("wizard.not_json", &[("field", &name), ("text", &e)]))?,
                None if structured => gf_api::exactjson::text(&t).map_err(|e| trf("wizard.not_json", &[("field", &name), ("text", &e)]))?,
                None => gf_api::exactjson::text(&t).unwrap_or(json!(t)),
                _ => json!(t),
            };
            out.insert(name, v);
        }
        Ok(Value::Object(out))
    }

    pub fn submit_step(&mut self) -> Result<(), String> {
        if self.pending.is_some() {
            return Err(tr("wizard.already_running").into());
        }
        let run = self.state.as_ref().ok_or(tr("wizard.no_run"))?["run"].clone();
        let id = self.step_def(self.at)["id"].clone();
        let input = self.input()?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(call("wizards.step", json!({"run": run, "step": id, "input": input})));
        });
        self.pending = Some(rx);
        self.error = None;
        Ok(())
    }

    pub fn show_error(&mut self, e: String) {
        self.error = Some(e);
    }

    pub fn run_step(&mut self) -> Result<(), String> {
        self.submit_step()?;
        let outcome = self.pending.take().expect("submitted").recv().map_err(|e| e.to_string())?;
        self.finish_step(outcome)
    }

    fn collect_step(&mut self) -> Result<(), String> {
        let Some(rx) = &self.pending else { return Ok(()) };
        match rx.try_recv() {
            Ok(outcome) => {
                self.pending = None;
                self.finish_step(outcome)
            }
            Err(TryRecvError::Empty) => Ok(()),
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                Err(tr("wizard.thread_lost").into())
            }
        }
    }

    fn cancel_job(&mut self) -> Result<(), String> {
        let job = self.step_state(self.at)["job"].as_str().ok_or(tr("wizard.no_job"))?.to_string();
        let run = self.state.as_ref().ok_or(tr("wizard.no_run"))?["run"].clone();
        let _ = call("jobs.cancel", json!({"id": job, "wait_s": 0.5}));
        self.state = Some(call("wizards.state", json!({"run": run}))?);
        self.shown_for = None;
        Ok(())
    }

    fn finish_step(&mut self, outcome: Result<Value, String>) -> Result<(), String> {
        self.state = Some(outcome?);
        self.shown_for = None;
        let st = self.step_state(self.at);
        if st["state"] == "done" && self.step_def(self.at)["command"].is_null() && self.at + 1 < self.steps() {
            self.at += 1;
        }
        Ok(())
    }

    fn steps(&self) -> usize {
        self.def.as_ref().and_then(|d| d["steps"].as_array()).map_or(0, Vec::len)
    }

    pub fn poll(&mut self) -> Result<bool, String> {
        self.collect_step()?;
        if self.pending.is_some() {
            return Ok(true);
        }
        let Some(state) = &self.state else { return Ok(false) };
        if self.step_state(self.at)["state"] != "running" {
            return Ok(false);
        }
        if self.polled.is_some_and(|t| t.elapsed() < POLL) {
            return Ok(true);
        }
        self.polled = Some(Instant::now());
        let run = state["run"].clone();
        if let Some(job) = self.step_state(self.at)["job"].as_str() {
            self.progress = call("jobs.status", json!({"id": job})).ok().and_then(|r| r.get("progress").cloned());
        }
        self.state = Some(call("wizards.state", json!({"run": run}))?);
        Ok(true)
    }

    pub fn running(&self) -> bool {
        self.pending.is_some() || (self.state.is_some() && self.step_state(self.at)["state"] == "running")
    }

    pub fn set_field(&mut self, name: &str, value: &Value) {
        match value {
            Value::Bool(b) => {
                self.flags.insert(name.to_string(), *b);
            }
            other => {
                self.text.insert(name.to_string(), shown(other));
            }
        }
    }

    pub fn go(&mut self, delta: isize) {
        let next = self.at as isize + delta;
        if next >= 0 && (next as usize) < self.steps() {
            self.at = next as usize;
            self.shown_for = None;
        }
    }

    pub fn sync_epoch(&mut self, epoch: u64) {
        if self.glb_epoch != epoch {
            self.glb_epoch = epoch;
            self.last_glb = None;
            self.glb_live = false;
        }
    }

    pub fn show(&mut self, ui_ctx: &egui::Context, world: &crate::world::World, project: &str) {
        if !self.open {
            return;
        }
        match self.poll() {
            Ok(true) => ui_ctx.request_repaint_after(POLL),
            Ok(false) => {}
            Err(e) => self.error = Some(e),
        }
        self.sync_epoch(world.epoch);
        self.remember_glb();
        let mut open = self.open;
        let title = self.def.as_ref().map_or_else(|| tr("ui.wz-title").to_string(), |d| tr_or(&format!("wizard.{}.title", d["id"].as_str().unwrap_or("")), d["title"].as_str().unwrap_or("")).to_string());
        egui::Window::new(tr("ui.wz-title")).id(egui::Id::new("wizards")).title_bar(true).open(&mut open).resizable(true).default_width(720.0).default_height(560.0).show(ui_ctx, |ui| {
            if self.def.is_some() {
                ui.horizontal(|ui| {
                    if self.pending.is_none() && ui.button(tr("wizard.all")).clicked() {
                        self.def = None;
                        self.state = None;
                        self.runs = None;
                    }
                    ui.strong(&title);
                });
                ui.separator();
            }
            if let Some(e) = &self.error {
                ui.colored_label(ui.visuals().warn_fg_color, e);
            }
            if self.def.is_none() {
                egui::ScrollArea::vertical().id_salt("wizard_list").auto_shrink([false, false]).show(ui, |ui| self.list_ui(ui, world, project));
            } else {
                self.run_ui(ui);
            }
        });
        self.open = open;
    }

    fn list_ui(&mut self, ui: &mut egui::Ui, world: &crate::world::World, project: &str) {
        if ui.checkbox(&mut self.developer, tr("wizard.dev")).on_hover_text(tr("wizard.dev_title")).changed() {
            self.list = None;
        }
        if self.list.is_none() {
            match call("wizards.list", json!({"developer": self.developer})) {
                Ok(r) => self.list = r["wizards"].as_array().cloned(),
                Err(e) => self.error = Some(e),
            }
        }
        let mut chosen = None;
        let mut heading = None;
        for w in self.list.clone().unwrap_or_default() {
            let wid = w["id"].as_str().unwrap_or_default();
            let group = w["group"].as_str().unwrap_or_default().to_string();
            if heading.as_ref() != Some(&group) {
                ui.label(egui::RichText::new(tr_or(&format!("wizard.group.{group}"), &group)).strong());
                heading = Some(group);
            }
            let r = ui.add(egui::Button::new(egui::RichText::new(tr_or(&format!("wizard.{wid}.title"), w["title"].as_str().unwrap_or_default())).strong()).min_size(egui::vec2(ui.available_width(), 28.0)));
            ui.label(egui::RichText::new(tr_or(&format!("wizard.{wid}.summary"), w["summary"].as_str().unwrap_or_default())).weak());
            if r.clicked() {
                chosen = w["id"].as_str().map(str::to_string);
            }
            ui.add_space(6.0);
        }
        if let Some(id) = chosen
            && let Err(e) = self.start(&id, run_values(world), world.settle_input(), project)
        {
            self.error = Some(e);
        }
        if self.runs.is_none() {
            self.runs = Some(call("wizards.runs", json!({"project": project, "limit": 8})).map(|r| r["runs"].as_array().cloned().unwrap_or_default()).unwrap_or_default());
        }
        let mut resumed = None;
        let runs = self.runs.clone().unwrap_or_default();
        if !runs.is_empty() {
            ui.separator();
            ui.label(egui::RichText::new(trf("wizard.recent", &[("project", &project)])).weak());
            for r in &runs {
                let steps: Vec<String> = r["steps"].as_array().into_iter().flatten().map(|s| format!("{} {}", shown(&s["id"]), tr_or(&format!("wizard.state.{}", shown(&s["state"])), &shown(&s["state"])))).collect();
                if ui.button(format!("{}  {}", tr_or(&format!("wizard.{}.title", shown(&r["wizard"])), &shown(&r["title"])), shown(&r["run"]))).on_hover_text(format!("{} · {}", shown(&r["started"]), steps.join(", "))).clicked() {
                    resumed = Some((shown(&r["wizard"]), shown(&r["run"])));
                }
            }
        }
        if let Some((wizard, run)) = resumed
            && let Err(e) = self.resume(&wizard, &run)
        {
            self.error = Some(e);
        }
        ui.separator();
        egui::CollapsingHeader::new(tr("panel.jobs")).id_salt("wz_jobs").show(ui, |ui| self.jobs_ui(ui));
        egui::CollapsingHeader::new(tr("panel.store")).id_salt("wz_store").show(ui, |ui| self.store_ui(ui, world));
    }

    fn jobs_ui(&mut self, ui: &mut egui::Ui) {
        if self.jobs_cache.as_ref().is_none_or(|(at, _)| at.elapsed() >= Duration::from_secs(1)) {
            match call("jobs.list", json!({"limit": 20})) {
                Ok(r) => self.jobs_cache = Some((Instant::now(), r)),
                Err(e) => {
                    ui.label(e);
                    return;
                }
            }
        }
        let listed = self.jobs_cache.as_ref().map(|(_, v)| v.clone()).unwrap_or_default();
        let jobs = listed["jobs"].as_array().cloned().unwrap_or_default();
        if jobs.is_empty() {
            ui.label(egui::RichText::new(trf("jobs.none_in", &[("dir", &shown(&listed["jobs_dir"]))])).weak());
        }
        let live = jobs.iter().any(|j| matches!(j["state"].as_str(), Some("running") | Some("queued")));
        egui::Grid::new("wz_jobs_grid").num_columns(4).striped(true).show(ui, |ui| {
            for j in &jobs {
                let command = shown(&j["command"]);
                ui.label(tr_or(&format!("job.cmd.{command}"), &command)).on_hover_text(&command);
                ui.label(tr_or(&format!("job.state.{}", shown(&j["state"])), &shown(&j["state"])));
                let p = &j["progress"];
                match (p["done"].as_f64(), p["total"].as_f64()) {
                    (Some(d), Some(t)) if t > 0.0 && j["state"] == "running" => {
                        ui.add(egui::ProgressBar::new((d / t) as f32).desired_width(120.0).text(shown(&p["stage"])));
                    }
                    _ => {
                        ui.label(egui::RichText::new(shown(&j["error"])).weak());
                    }
                }
                if matches!(j["state"].as_str(), Some("running") | Some("queued")) && ui.small_button(tr("jobs.cancel")).clicked()
                    && let Err(e) = call("jobs.cancel", json!({"id": j["id"]}))
                {
                    self.error = Some(e);
                }
                ui.end_row();
            }
        });
        if live {
            ui.ctx().request_repaint_after(Duration::from_secs(1));
        }
    }

    fn store_ui(&mut self, ui: &mut egui::Ui, world: &crate::world::World) {
        let Some(s) = &world.session else {
            ui.label(tr("studio.load_first"));
            return;
        };
        let refresh = ui.small_button(tr("store.refresh")).clicked();
        if refresh || self.store_cache.as_ref().is_none_or(|(path, _)| *path != s.store) {
            match call("store.list", json!({"store": s.store})) {
                Ok(r) => self.store_cache = Some((s.store.clone(), r)),
                Err(e) => {
                    ui.label(e);
                    return;
                }
            }
        }
        let listed = self.store_cache.as_ref().map(|(_, v)| v.clone()).unwrap_or_default();
        let short = |v: &Value| shown(v).trim_start_matches("sha256:").chars().take(12).collect::<String>();
        let day = |v: &Value| shown(v).chars().take(16).collect::<String>().replace('T', " ");
        for (title, key, id, used, cols) in [
            (tr("store.packs"), "packs", "pack", s.pack.clone(), vec!["vertices", "macro_base"]),
            (tr("store.catalogs"), "catalogs", "catalog", s.catalog.clone(), vec!["targets"]),
            (tr("store.games"), "game_bases", "game", s.game.clone(), vec!["vertices", "triangles", "lods"]),
            (tr("store.eyerigs"), "eyerigs", "eyerig", String::new(), vec!["format"]),
        ] {
            ui.strong(title);
            egui::Grid::new(("wz_store", key)).num_columns(cols.len() + 2).striped(true).show(ui, |ui| {
                for item in listed[key].as_array().cloned().unwrap_or_default() {
                    ui.label(day(&item["imported"]));
                    for c in &cols {
                        ui.label(shown(&item[*c]));
                    }
                    let mark = if item[id] == used.as_str() { " ●" } else { "" };
                    ui.label(format!("{}{mark}", short(&item[id]))).on_hover_text(shown(&item[id]));
                    ui.end_row();
                }
            });
        }
        ui.label(egui::RichText::new(tr("store.note_short")).weak());
    }

    fn run_ui(&mut self, ui: &mut egui::Ui) {
        self.load_form();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(170.0);
                for i in 0..self.steps() {
                    let (wid, sid) = (self.def.as_ref().map_or(String::new(), |d| shown(&d["id"])), shown(&self.step_def(i)["id"]));
                    let title = tr_or(&format!("wizard.{wid}.{sid}.title"), self.step_def(i)["title"].as_str().unwrap_or_default()).to_string();
                    let raw = self.step_state(i)["state"].as_str().unwrap_or_default().to_string();
                    let state = tr_or(&format!("wizard.state.{raw}"), &raw).to_string();
                    if ui.selectable_label(i == self.at, format!("{}. {title}\n    {state}", i + 1)).clicked() {
                        self.at = i;
                        self.shown_for = None;
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                let s = self.step_def(self.at).clone();
                let st = self.step_state(self.at).clone();
                if let Some(h) = s["help"].as_str() {
                    let wid = self.def.as_ref().map_or(String::new(), |d| shown(&d["id"]));
                    ui.label(egui::RichText::new(tr_or(&format!("wizard.{wid}.{}.help", shown(&s["id"])), h)).weak());
                }
                if let Some(c) = s["command"].as_str() {
                    ui.label(egui::RichText::new(trf("wizard.runs", &[("command", &c), ("kind", &s["kind"].as_str().unwrap_or_default())])).weak().monospace());
                }
                let fields = self.fields(self.at);
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    egui::Grid::new(("wz_common", self.at)).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                        for (name, label, _, schema) in fields.iter().filter(|f| f.2) {
                            self.field_ui(ui, name, label, schema);
                        }
                    });
                    let advanced: Vec<_> = fields.iter().filter(|f| !f.2).collect();
                    if !advanced.is_empty() {
                        egui::CollapsingHeader::new(trf("wizard.advanced", &[("n", &advanced.len())])).id_salt(("wz_adv", self.at)).show(ui, |ui| {
                            egui::Grid::new(("wz_advanced", self.at)).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                                for (name, label, _, schema) in advanced {
                                    self.field_ui(ui, name, label, schema);
                                }
                            });
                        });
                    }
                    if let Some(e) = st["error"].as_object() {
                        ui.colored_label(ui.visuals().warn_fg_color, format!("{}: {}", e.get("code").and_then(Value::as_str).unwrap_or(""), e.get("message").and_then(Value::as_str).unwrap_or("")));
                    }
                    if self.pending.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(tr("wizard.running_step"));
                        });
                    }
                    if st["state"] == "running" {
                        let p = self.progress.clone().unwrap_or(Value::Null);
                        let job = st["job"].as_str().unwrap_or_default().to_string();
                        if self.since.as_ref().is_none_or(|(j, _)| *j != job) {
                            self.since = Some((job, Instant::now()));
                        }
                        let secs = self.since.as_ref().map_or(0, |(_, t)| t.elapsed().as_secs());
                        let text = progress_text(&p, secs);
                        match (p["done"].as_f64(), p["total"].as_f64()) {
                            (Some(d), Some(t)) if t > 0.0 => ui.add(egui::ProgressBar::new((d / t) as f32).text(text)),
                            _ => ui.add(egui::ProgressBar::new(0.0).animate(true).text(text)),
                        };
                    }
                    if !st["result"].is_null() {
                        self.shows_ui(ui, &s);
                        egui::CollapsingHeader::new(tr("wizard.report")).id_salt(("wz_report", self.at)).default_open(false).show(ui, |ui| {
                            ui.label(egui::RichText::new(serde_json::to_string_pretty(&st["result"]).unwrap_or_default()).monospace().small());
                        });
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let state = st["state"].as_str().unwrap_or_default();
                    ui.label(if state == "stale" { tr("wizard.stale_note") } else { tr_or(&format!("wizard.state.{state}"), state) });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let last = self.at + 1 >= self.steps();
                        if ui.add_enabled(!last && (state == "done" || s["optional"] == Value::Bool(true)), egui::Button::new(tr("ui.wz-next"))).clicked() {
                            self.at += 1;
                            self.shown_for = None;
                        }
                        let label = if s["command"].is_null() { tr("wizard.use") } else if state == "done" { tr("wizard.run_again") } else { tr("ui.wz-run") };
                        if ui.add_enabled(state != "running" && self.pending.is_none(), egui::Button::new(label)).clicked()
                            && let Err(e) = self.submit_step()
                        {
                            self.error = Some(e);
                        }
                        if state == "running" && st["job"].is_string() && ui.button(tr("ui.wz-cancel")).on_hover_text(tr("ui.wz-cancel.title")).clicked()
                            && let Err(e) = self.cancel_job()
                        {
                            self.error = Some(e);
                        }
                        if ui.add_enabled(self.at > 0, egui::Button::new(tr("ui.wz-back"))).clicked() {
                            self.at -= 1;
                            self.shown_for = None;
                        }
                    });
                });
            });
        });
    }

    fn resolve(&self, binding: &str) -> Value {
        let path: Vec<&str> = binding.trim_start_matches('$').split('.').collect();
        let Some(state) = &self.state else { return Value::Null };
        if path[0] == "run" {
            return path[1..].iter().fold(state["values"].clone(), |at, p| at[*p].clone());
        }
        let Some(k) = (0..self.steps()).find(|&i| self.step_def(i)["id"] == path[0]) else { return Value::Null };
        let start = match path.get(1) {
            Some(&"input") => self.step_state(k)["input"].clone(),
            _ => self.step_state(k)["result"].clone(),
        };
        path.iter().skip(2).fold(start, |at, p| at[*p].clone())
    }

    fn shows_ui(&mut self, ui: &mut egui::Ui, s: &Value) {
        for sh in s["shows"].as_array().cloned().unwrap_or_default() {
            match sh["kind"].as_str() {
                Some("images") => {
                    let list = sh["from"].as_str().map(|b| self.resolve(b)).unwrap_or(Value::Null);
                    let key = sh["path"].as_str().unwrap_or("path");
                    ui.horizontal_wrapped(|ui| {
                        for item in list.as_array().cloned().unwrap_or_default() {
                            let Some(path) = item[key].as_str() else { continue };
                            let tex = self.images.entry(path.to_string()).or_insert_with(|| load_png(path).map(|img| ui.ctx().load_texture(path, img, Default::default())));
                            ui.vertical(|ui| {
                                match tex {
                                    Some(t) => {
                                        let size = t.size_vec2();
                                        let w = 260.0_f32.min(size.x);
                                        ui.image((t.id(), egui::vec2(w, w * size.y / size.x.max(1.0))));
                                    }
                                    None => {
                                        ui.label(trf("wizard.cannot_read", &[("path", &path)]));
                                    }
                                }
                                ui.label(egui::RichText::new(shown(&item[sh["label"].as_str().unwrap_or("name")])).weak());
                            });
                        }
                    });
                }
                Some("spec") => {
                    let spec = sh["from"].as_str().map(|b| self.resolve(b)).unwrap_or(Value::Null);
                    let store = sh["store"].as_str().map(|b| self.resolve(b)).unwrap_or(Value::Null);
                    if let Some(spec) = spec.as_str() {
                        ui.horizontal(|ui| {
                            ui.label(trf("wizard.wrote", &[("path", &spec)]));
                            if ui.button(tr("wizard.open_spec")).clicked() {
                                self.open_spec = Some((shown(&store), spec.to_string()));
                            }
                        });
                    }
                }
                Some("glb") | Some("file") => {
                    if let Some(path) = sh["from"].as_str().map(|b| self.resolve(b)).and_then(|v| v.as_str().map(str::to_string)) {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(trf("wizard.wrote", &[("path", &path)]));
                            if ui.small_button(tr("wizard.show_in_folder")).clicked() {
                                show_in_folder(&path);
                            }
                            if ui.small_button(tr("wizard.copy_path")).clicked() {
                                ui.ctx().copy_text(path.clone());
                            }
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn field_ui(&mut self, ui: &mut egui::Ui, name: &str, label: &str, schema: &Value) {
        let (short, more) = label.split_once(" (").map_or((label, ""), |(s, m)| (s, m.trim_end_matches(')')));
        let described = schema["description"].as_str().unwrap_or_default();
        let tip = [more, described].iter().filter(|t| !t.is_empty()).copied().collect::<Vec<_>>().join("\n");
        ui.label(short).on_hover_text(tip);
        if let Some(options) = schema["enum"].as_array() {
            let cur = self.text.entry(name.to_string()).or_default();
            egui::ComboBox::from_id_salt(("wz_enum", name)).selected_text(cur.clone()).show_ui(ui, |ui| {
                ui.selectable_value(cur, String::new(), tr("wizard.default"));
                for o in options {
                    let o = shown(o);
                    ui.selectable_value(cur, o.clone(), o);
                }
            });
        } else if schema["type"] == "boolean" {
            ui.checkbox(self.flags.entry(name.to_string()).or_default(), "");
        } else {
            let field = self.step_def(self.at)["fields"].as_array().and_then(|a| a.iter().find(|f| f["name"] == name));
            let ext = field.and_then(|f| f["ext"].as_str()).unwrap_or_default().to_string();
            let pick = match field.and_then(|f| f["widget"].as_str()) {
                Some("file_open") => Some(Pick::Open(ext)),
                Some("file_save") => Some(Pick::Save(ext)),
                Some("folder") => Some(Pick::Folder),
                _ => None,
            };
            let value = self.text.entry(name.to_string()).or_default();
            match pick {
                Some(pick) => {
                    ui.allocate_ui(egui::vec2(410.0, ui.available_height()), |ui| browse::field(ui, &mut self.browse, &format!("wizard.{name}"), value, "", pick));
                }
                None => {
                    ui.add(egui::TextEdit::singleline(value).desired_width(380.0));
                }
            }
        }
        ui.end_row();
    }
}
