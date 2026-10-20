use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use gf_api::Ctx;
use serde_json::{Map, Value, json};

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
    polled: Option<Instant>,
    images: BTreeMap<String, Option<egui::TextureHandle>>,
    pub open_spec: Option<(String, String)>,
    developer: bool,
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
        Some(s) => json!({"store": s.store, "pack": s.pack, "catalog": s.catalog, "game": s.game}),
        None => json!({}),
    }
}

pub fn ctx() -> Result<Ctx, String> {
    let mut ctx = Ctx::from_env();
    let name = if cfg!(windows) { "gazeforge.exe" } else { "gazeforge" };
    let cli = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name(name);
    if !cli.is_file() {
        return Err(format!("job steps run the command line tool, which is not next to the studio: {}", cli.display()));
    }
    ctx.exe = cli;
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

    pub fn start(&mut self, id: &str, values: Value, character: Option<Value>, project: &str) -> Result<(), String> {
        let store = values["store"].as_str().unwrap_or_default().to_string();
        let def = call("wizards.get", json!({"id": id}))?;
        let state = call("wizards.start", json!({"wizard": id, "project": project, "values": values}))?;
        let dir = PathBuf::from(state["dir"].as_str().unwrap_or_default());
        let mut prepared = Map::new();
        if let Some(mut input) = character {
            let spec = dir.join("character.gfchar.json").display().to_string();
            input["out"] = json!(spec);
            input.as_object_mut().map(|o| o.remove("uvs"));
            call("designer.save", input)?;
            prepared.insert("character".into(), json!({"store": store, "spec": spec}));
        }
        prepared.insert("export".into(), json!({"out": dir.join("export.glb").display().to_string()}));
        self.def = Some(def);
        self.state = Some(state);
        self.prepared = prepared;
        self.at = 0;
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
                (name.clone(), f["label"].as_str().unwrap_or(&name).to_string(), f["common"] == Value::Bool(true), schema)
            })
            .collect();
        for (name, schema) in props {
            if !defined.iter().any(|f| f["name"] == name.as_str()) {
                out.push((name.clone(), name, false, schema));
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

    fn input(&self) -> Value {
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
            let v = match schema["type"].as_str() {
                Some("number") | Some("integer") => t.parse::<f64>().map(|x| json!(x)).unwrap_or(json!(t)),
                None => gf_api::exactjson::text(&t).unwrap_or(json!(t)),
                _ => json!(t),
            };
            out.insert(name, v);
        }
        Value::Object(out)
    }

    pub fn run_step(&mut self) -> Result<(), String> {
        let run = self.state.as_ref().ok_or("no run")?["run"].clone();
        let id = self.step_def(self.at)["id"].clone();
        let state = call("wizards.step", json!({"run": run, "step": id, "input": self.input()}))?;
        self.state = Some(state);
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
        self.state.is_some() && self.step_state(self.at)["state"] == "running"
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

    pub fn show(&mut self, ui_ctx: &egui::Context, world: &crate::world::World, project: &str) {
        if !self.open {
            return;
        }
        match self.poll() {
            Ok(true) => ui_ctx.request_repaint_after(POLL),
            Ok(false) => {}
            Err(e) => self.error = Some(e),
        }
        let mut open = self.open;
        let title = self.def.as_ref().and_then(|d| d["title"].as_str()).unwrap_or("Tasks").to_string();
        egui::Window::new("Tasks").id(egui::Id::new("wizards")).title_bar(true).open(&mut open).resizable(true).default_width(720.0).default_height(560.0).show(ui_ctx, |ui| {
            ui.horizontal(|ui| {
                if self.def.is_some() && ui.button("all tasks").clicked() {
                    self.def = None;
                    self.state = None;
                }
                ui.strong(&title);
            });
            ui.separator();
            if self.def.is_none() {
                self.list_ui(ui, world, project);
            } else {
                self.run_ui(ui);
            }
            if let Some(e) = &self.error {
                ui.colored_label(ui.visuals().warn_fg_color, e);
            }
        });
        self.open = open;
    }

    fn list_ui(&mut self, ui: &mut egui::Ui, world: &crate::world::World, project: &str) {
        if ui.checkbox(&mut self.developer, "developer tasks").on_hover_text("tasks that serve building GazeForge itself: macro tools, raw dense-base gates").changed() {
            self.list = None;
        }
        if self.list.is_none() {
            match call("wizards.list", json!({"developer": self.developer})) {
                Ok(r) => self.list = r["wizards"].as_array().cloned(),
                Err(e) => self.error = Some(e),
            }
        }
        let mut chosen = None;
        for w in self.list.clone().unwrap_or_default() {
            ui.label(egui::RichText::new(w["group"].as_str().unwrap_or_default()).weak());
            let r = ui.add(egui::Button::new(egui::RichText::new(w["title"].as_str().unwrap_or_default()).strong()).min_size(egui::vec2(ui.available_width(), 28.0)));
            ui.label(egui::RichText::new(w["summary"].as_str().unwrap_or_default()).weak());
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
    }

    fn run_ui(&mut self, ui: &mut egui::Ui) {
        self.load_form();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(170.0);
                for i in 0..self.steps() {
                    let (title, state) = (self.step_def(i)["title"].as_str().unwrap_or_default().to_string(), self.step_state(i)["state"].as_str().unwrap_or_default().to_string());
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
                    ui.label(egui::RichText::new(h).weak());
                }
                if let Some(c) = s["command"].as_str() {
                    ui.label(egui::RichText::new(format!("runs {c} ({})", s["kind"].as_str().unwrap_or_default())).weak().monospace());
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
                        egui::CollapsingHeader::new(format!("advanced ({})", advanced.len())).id_salt(("wz_adv", self.at)).show(ui, |ui| {
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
                    if st["state"] == "running" {
                        let p = self.progress.clone().unwrap_or(Value::Null);
                        let frac = match (p["done"].as_f64(), p["total"].as_f64()) {
                            (Some(d), Some(t)) if t > 0.0 => (d / t) as f32,
                            _ => 0.0,
                        };
                        ui.add(egui::ProgressBar::new(frac).text(format!("{} {}/{} {}", shown(&p["stage"]), shown(&p["done"]), shown(&p["total"]), shown(&p["message"]))));
                    }
                    if !st["result"].is_null() {
                        self.shows_ui(ui, &s);
                        egui::CollapsingHeader::new("report").id_salt(("wz_report", self.at)).default_open(true).show(ui, |ui| {
                            ui.label(egui::RichText::new(serde_json::to_string_pretty(&st["result"]).unwrap_or_default()).monospace().small());
                        });
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let state = st["state"].as_str().unwrap_or_default();
                    ui.label(if state == "stale" { "an earlier step changed: run this step again" } else { state });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let last = self.at + 1 >= self.steps();
                        if ui.add_enabled(!last && (state == "done" || s["optional"] == Value::Bool(true)), egui::Button::new("next")).clicked() {
                            self.at += 1;
                            self.shown_for = None;
                        }
                        let label = if s["command"].is_null() { "use these" } else if state == "done" { "run again" } else { "run" };
                        if ui.add_enabled(state != "running", egui::Button::new(label)).clicked()
                            && let Err(e) = self.run_step()
                        {
                            self.error = Some(e);
                        }
                        if ui.add_enabled(self.at > 0, egui::Button::new("back")).clicked() {
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
                                        ui.label(format!("cannot read {path}"));
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
                            ui.label(format!("wrote {spec}"));
                            if ui.button("open in the designer").clicked() {
                                self.open_spec = Some((shown(&store), spec.to_string()));
                            }
                        });
                    }
                }
                Some("glb") | Some("file") => {
                    if let Some(path) = sh["from"].as_str().map(|b| self.resolve(b)).and_then(|v| v.as_str().map(str::to_string)) {
                        ui.label(format!("wrote {path}"));
                    }
                }
                _ => {}
            }
        }
    }

    fn field_ui(&mut self, ui: &mut egui::Ui, name: &str, label: &str, schema: &Value) {
        let tip = schema["description"].as_str().unwrap_or_default();
        ui.label(label).on_hover_text(tip);
        if let Some(options) = schema["enum"].as_array() {
            let cur = self.text.entry(name.to_string()).or_default();
            egui::ComboBox::from_id_salt(("wz_enum", name)).selected_text(cur.clone()).show_ui(ui, |ui| {
                ui.selectable_value(cur, String::new(), "(default)");
                for o in options {
                    let o = shown(o);
                    ui.selectable_value(cur, o.clone(), o);
                }
            });
        } else if schema["type"] == "boolean" {
            ui.checkbox(self.flags.entry(name.to_string()).or_default(), "");
        } else {
            ui.add(egui::TextEdit::singleline(self.text.entry(name.to_string()).or_default()).desired_width(380.0));
        }
        ui.end_row();
    }
}
