use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use eframe::egui;
use serde_json::{Map, Value, json};

use crate::face_rig::{MANUAL, Mode, SHAPES};
use crate::i18n::{tr, trf};
use crate::stitch::CONTROLS;
use crate::world::World;

pub enum Op {
    Open { name: String, recovery: bool },
    Save { name: String, recovery: bool },
    List,
    Export,
}

pub struct Project {
    pub name: Option<String>,
    pub doc: Option<Value>,
    pub active: usize,
    pub notes: String,
    pub status: String,
    pub saved_sig: Option<String>,
    pub list: Vec<Value>,
    pub recoveries: Vec<Value>,
    pub resources: Map<String, Value>,
    pub unresolved: Vec<Value>,
    pub pending_panels: Option<Value>,
    pub job: Option<Receiver<(Op, Result<Value, String>)>>,
    pub last_autosave: Option<Instant>,
    pub autosaved: bool,
    pub recovered: bool,
    pub last_open: Option<String>,
    pub pending_epoch: Option<u64>,
    pub relinked: bool,
    pub last_auto_sig: Option<String>,
    pub autosave_reason: &'static str,
}

impl Default for Project {
    fn default() -> Project {
        Project {
            name: None,
            doc: None,
            active: 0,
            notes: String::new(),
            status: String::new(),
            saved_sig: None,
            list: Vec::new(),
            recoveries: Vec::new(),
            resources: Map::new(),
            unresolved: Vec::new(),
            pending_panels: None,
            job: None,
            last_autosave: None,
            autosaved: false,
            recovered: false,
            last_open: None,
            pending_epoch: None,
            relinked: false,
            last_auto_sig: None,
            autosave_reason: "timer",
        }
    }
}

impl Project {
    pub fn slot(&self) -> String {
        self.name.clone().unwrap_or_else(|| gf_api::project::UNTITLED.to_string())
    }
}

fn root() -> String {
    gf_api::project::default_root().display().to_string()
}

impl World {
    pub fn panel_states(&self) -> Map<String, Value> {
        let mut out = self.project.doc.as_ref().and_then(|d| d["panels"].as_object().cloned()).unwrap_or_default();
        let f = &self.face;
        let weights: Map<String, Value> = SHAPES.iter().zip(&f.weights).filter(|(_, w)| **w != 0.0).map(|(n, w)| (n.to_string(), json!(w))).collect();
        let manual: Map<String, Value> = f.manual.iter().map(|(j, m)| (j.to_string(), Value::Object(MANUAL.iter().enumerate().map(|(c, (k, _))| (k.to_string(), json!(m[c]))).collect()))).collect();
        out.insert("rig".into(), json!({"loaded": f.rig.is_some() || f.wanted, "mode": f.mode.name(), "overlay": f.overlay, "joint": f.joint, "weights": weights, "manual": manual}));
        let s = &self.stitch;
        let mut resources = Map::new();
        if !s.store.trim().is_empty() {
            resources.insert("store".into(), json!(s.store.trim()));
        }
        if !s.body.trim().is_empty() {
            resources.insert("body".into(), json!(s.body.trim()));
        }
        let values: Map<String, Value> = CONTROLS.iter().enumerate().map(|(i, c)| (c.0.to_string(), json!(s.values[i]))).collect();
        out.insert("stitch".into(), json!({"resources": resources, "show": s.show, "seam": s.seam, "auto": s.auto, "values": values, "stitched": s.wanted}));
        let t = &self.timeline;
        out.insert("timeline".into(), json!({"version": 1, "t": t.t, "speed": t.speed, "loop": t.looping, "applyHead": t.apply_head, "playing": t.playing,
                                             "clip": t.clip.as_ref().map_or(Value::Null, |c| c.raw.clone())}));
        out
    }

    pub fn apply_panel_states(&mut self, panels: &Value) {
        let res = |role: &str| self.project.resources.get(role).and_then(|r| r["path"].as_str()).map(str::to_string);
        let (stitch_store, stitch_body) = (res("panels.stitch.store"), res("panels.stitch.body"));
        let r = &panels["rig"];
        if r.is_object() {
            self.face.weights = SHAPES.iter().map(|n| r["weights"][*n].as_f64().unwrap_or(0.0)).collect();
            self.face.mode = if r["mode"] == "joints" { Mode::Joints } else { Mode::Blend };
            self.face.overlay = r["overlay"] != false;
            if let Some(j) = r["joint"].as_u64() {
                self.face.joint = j as usize;
            }
            self.face.manual = r["manual"].as_object().into_iter().flatten().filter_map(|(k, m)| Some((k.parse().ok()?, std::array::from_fn(|c| m[MANUAL[c].0].as_f64().unwrap_or(0.0))))).collect();
            if r["loaded"] == true {
                self.face_load();
            }
        }
        let s = &panels["stitch"];
        if s.is_object() {
            let src = if s["resources"].is_object() { &s["resources"] } else { s };
            self.stitch.store = stitch_store.or_else(|| src["store"].as_str().map(str::to_string)).unwrap_or_default();
            self.stitch.body = stitch_body.or_else(|| src["body"].as_str().map(str::to_string)).unwrap_or_default();
            self.stitch.show = s["show"] != false;
            self.stitch.seam = s["seam"] != false;
            self.stitch.auto = s["auto"] == true;
            for (i, c) in CONTROLS.iter().enumerate() {
                self.stitch.values[i] = s["values"][c.0].as_f64().map_or(c.4, |v| v.clamp(c.1, c.2));
            }
            self.stitch.last = None;
            self.stitch.stale = false;
            self.stitch_draw();
            if s["stitched"] == true && !self.stitch.body.is_empty() {
                self.stitch_go();
            }
        }
        let t = &panels["timeline"];
        if t.is_object() {
            if t["clip"].is_object() {
                self.timeline_clip(t["clip"].clone(), tr("timeline.label_restored"));
            }
            if let Some(v) = t["speed"].as_f64() {
                self.timeline.speed = v;
            }
            self.timeline.looping = t["loop"] != false;
            self.timeline.apply_head = t["applyHead"] != false;
            if let Some(v) = t["t"].as_f64() {
                self.timeline.t = v;
            }
            self.timeline_show();
            if t["playing"] == true {
                self.timeline_play(true);
            }
        }
    }

    pub fn character_input(&self) -> Option<Value> {
        let s = self.session.as_ref()?;
        let character = self.shape.as_ref().map_or_else(|| s.character.clone(), |sh| sh.character(&s.base(), &sh.live));
        let mut c = s.settle_input(&character, false);
        if let Some(o) = c.as_object_mut() {
            for k in ["store", "eyes", "uvs", "fast"] {
                o.remove(k);
            }
        }
        Some(c)
    }

    pub fn project_sig(&self) -> String {
        gf_api::text(&json!({"c": self.character_input(), "p": self.panel_states(), "n": self.project.notes}))
    }

    pub fn project_dirty(&self) -> bool {
        self.session.is_some() && (self.project.recovered || self.project.relinked || self.project.saved_sig.as_deref() != Some(&self.project_sig()))
    }

    fn project_input(&self, name: &str) -> Option<Value> {
        let s = self.session.as_ref()?;
        let p = &self.project;
        let a = p.active;
        let current = json!({"designer": self.character_input()?});
        let characters: Vec<Value> = match &p.doc {
            Some(d) => d["characters"].as_array().into_iter().flatten().enumerate()
                .map(|(i, c)| if i == a { let mut x = current.clone(); x["id"] = c["id"].clone(); x["history"] = c["history"].clone(); x } else { json!({"id": c["id"], "spec": c["spec"], "history": c["history"]}) })
                .collect(),
            None => vec![{ let mut x = current; x["id"] = json!("c1"); x }],
        };
        let mut options = Map::new();
        if let Some(e) = &s.eyes_file {
            options.insert("eyes".into(), json!(e));
        }
        if let Some(m) = &s.macro_file {
            options.insert("macro".into(), json!(m));
        }
        if let Some(d) = self.face_morphs() {
            options.insert("face_morphs".into(), json!(d));
        }
        if let Some(p) = self.face_prefix.clone() {
            options.insert("face_prefix".into(), json!(p));
        }
        let mut input = json!({"name": name, "root": root(), "store": s.store, "characters": characters, "active": a, "panels": self.panel_states(), "notes": p.notes, "options": options});
        if let Some(v) = p.doc.as_ref().map(|d| d["viewer"].clone()).filter(|v| v.is_object()) {
            input["viewer"] = v;
        }
        if let Some(m) = &s.macro_file {
            input["macro"] = json!(m);
        }
        Some(input)
    }

    fn project_call(&mut self, op: Op, command: &'static str, input: Value) {
        if self.project.job.is_some() {
            self.project.status = tr("project.busy").into();
            return;
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let env = gf_api::call(&gf_api::Ctx::from_env(), command, &input);
            let out = if env["ok"] == true { Ok(env["result"].clone()) } else { Err(gf_api::text(&env["error"])) };
            let _ = tx.send((op, out));
        });
        self.project.job = Some(rx);
    }

    pub fn project_save(&mut self, name: &str, recovery: bool) {
        if self.session.as_ref().is_some_and(|s| !s.character.macros.is_null() && s.macro_file.is_none()) {
            self.project.status = tr("designer.needs_macro").into();
            return;
        }
        let Some(mut input) = self.project_input(name) else {
            self.project.status = tr("studio.load_first").into();
            return;
        };
        if recovery {
            input["recovery"] = json!(true);
        }
        self.project_call(Op::Save { name: name.to_string(), recovery }, "project.save", input);
    }

    pub fn project_open(&mut self, name: &str, recovery: bool) {
        self.project.last_open = Some(name.to_string());
        let mut input = json!({"name": name, "root": root(), "recovery": recovery});
        let given: Map<String, Value> = self.project.unresolved.iter().filter(|u| u["skip"] != true).filter_map(|u| Some((u["role"].as_str()?.to_string(), u["given"].clone()))).filter(|(_, v)| v.as_str().is_some_and(|s| !s.trim().is_empty())).collect();
        let skip: Vec<Value> = self.project.unresolved.iter().filter(|u| u["skip"] == true).map(|u| u["role"].clone()).collect();
        self.project.relinked = !given.is_empty() || !skip.is_empty();
        if !given.is_empty() {
            input["resources"] = Value::Object(given);
        }
        if !skip.is_empty() {
            input["skip"] = Value::Array(skip);
        }
        self.project_call(Op::Open { name: name.to_string(), recovery }, "project.load", input);
    }

    pub fn project_list(&mut self) {
        self.project_call(Op::List, "project.list", json!({"root": root()}));
    }

    pub fn project_export(&mut self) {
        let Some(name) = self.project.name.clone() else { return };
        self.project_call(Op::Export, "project.export", json!({"name": name, "root": root()}));
    }

    pub fn project_detach(&mut self) {
        self.project = Project { list: std::mem::take(&mut self.project.list), recoveries: std::mem::take(&mut self.project.recoveries), ..Project::default() };
        self.project.pending_epoch = Some(self.epoch);
    }

    pub fn project_new(&mut self) {
        let Some(s) = &self.session else { return };
        let (store, mac, eyes) = (s.store.clone(), s.macro_file.clone(), s.eyes_file.clone());
        self.project_detach();
        self.project.status = tr("designer.project_unsaved").into();
        self.request_load_with(&store, None, None, mac.as_deref(), eyes.as_deref(), None);
    }

    pub fn project_pick(&mut self, k: usize) {
        let Some(d) = self.project.doc.clone() else { return };
        let Some(c) = d["characters"].get(k) else { return };
        let Some(s) = &self.session else { return };
        self.project.active = k;
        self.project.saved_sig = None;
        self.project.pending_epoch = Some(self.epoch);
        self.project.pending_panels = d["panels"].as_object().map(|p| Value::Object(p.clone()));
        let (store, mac, eyes) = (s.store.clone(), s.macro_file.clone(), s.eyes_file.clone());
        self.request_load_with(&store, None, Some(c["spec"].clone()), mac.as_deref(), eyes.as_deref(), None);
    }

    pub fn project_poll(&mut self) {
        if let Some(rx) = &self.project.job
            && let Ok((op, out)) = rx.try_recv()
        {
            self.project.job = None;
            match (op, out) {
                (Op::Save { name, recovery }, Ok(r)) => {
                    if recovery {
                        self.project.autosaved = true;
                        self.project.status = trf("designer.autosaved", &[("reason", &tr(&format!("project.frag.{}", self.project.autosave_reason))), ("modified", &when(&r["modified"]))]);
                    } else {
                        let was = self.project.name.clone();
                        self.project.name = Some(name.clone());
                        self.project.doc = Some(r["project"].clone());
                        self.project.recovered = false;
                        self.project.relinked = false;
                        self.project.autosaved = false;
                        self.project.resources = r["project"]["resources"].as_object().into_iter().flatten()
                            .filter_map(|(role, v)| v["hint"].as_str().map(|h| (role.clone(), json!({"path": h}))))
                            .collect();
                        self.project.saved_sig = Some(self.project_sig());
                        self.project.status = trf("designer.project_saved", &[("path", &r["path"].as_str().unwrap_or("")), ("bytes", &r["bytes"]), ("modified", &when(&r["modified"]))]);
                        if was.is_none() && name != gf_api::project::UNTITLED {
                            let _ = gf_api::call(&gf_api::Ctx::from_env(), "project.discard", &json!({"name": gf_api::project::UNTITLED, "root": root()}));
                        }
                    }
                }
                (Op::Open { name, recovery }, Ok(r)) => {
                    let doc = r["project"].clone();
                    self.project.resources = r["checked"]["resources"].as_object().cloned().unwrap_or_default();
                    self.project.unresolved.clear();
                    self.project.name = (name != gf_api::project::UNTITLED).then_some(name);
                    self.project.active = doc["active"].as_u64().unwrap_or(0) as usize;
                    self.project.notes = doc["notes"].as_str().unwrap_or_default().to_string();
                    self.project.recovered = recovery;
                    self.project.pending_panels = Some(doc["panels"].clone());
                    let res = |role: &str| self.project.resources.get(role).and_then(|v| v["path"].as_str()).map(str::to_string);
                    let (eyes, mac) = (res("eyes"), res("macro"));
                    let store = doc["store"].as_str().unwrap_or_default().to_string();
                    let spec = doc["characters"][self.project.active]["spec"].clone();
                    self.project.doc = Some(doc);
                    self.project.saved_sig = None;
                    self.project.status = tr("project.opening").into();
                    self.face_morphs = res("face_morphs").or_else(|| self.project.doc.as_ref().and_then(|d| d["options"]["face_morphs"].as_str()).map(str::to_string));
                    self.face_prefix = self.project.doc.as_ref().and_then(|d| d["options"]["face_prefix"].as_str()).map(str::to_string);
                    self.project.pending_epoch = Some(self.epoch);
                    self.project.last_autosave = Some(Instant::now());
                    self.request_load_with(&store, None, Some(spec), mac.as_deref(), eyes.as_deref(), None);
                }
                (Op::Open { .. }, Err(e)) => {
                    let v = gf_api::exactjson::text(&e).unwrap_or(Value::Null);
                    self.project.unresolved = v["details"]["unresolved"].as_array().cloned().unwrap_or_default();
                    for u in &mut self.project.unresolved {
                        u["given"] = json!("");
                    }
                    self.project.status = v["message"].as_str().map_or(e.clone(), str::to_string);
                }
                (Op::List, Ok(r)) => {
                    self.project.list = r["projects"].as_array().cloned().unwrap_or_default().into_iter().filter(|p| p["error"].is_null()).collect();
                    self.project.recoveries = r["recoveries"].as_array().cloned().unwrap_or_default();
                }
                (Op::Export, Ok(r)) => {
                    self.project.status = trf("designer.portable", &[("out", &r["out"].as_str().unwrap_or("")), ("bytes", &r["bytes"]), ("blobs", &r["character_blobs"])]);
                }
                (Op::Export, Err(e)) => self.project.status = trf("designer.export_failed", &[("error", &message(&e))]),
                (Op::Save { .. }, Err(e)) => self.project.status = trf("designer.save_failed", &[("error", &message(&e))]),
                (Op::List, Err(e)) => self.project.status = message(&e),
            }
        }
        if self.project.saved_sig.is_none() && self.loading.is_none() && matches!(self.load_result, Some(Err(_))) {
            let e = self.load_result.as_ref().and_then(|r| r.as_ref().err()).cloned().unwrap_or_default();
            self.project.pending_panels = None;
            self.project.pending_epoch = None;
            self.project.status = trf("studio.note.load_failed", &[("error", &e)]);
            self.project.doc = None;
            self.project.name = None;
            if self.session.is_some() {
                self.project.saved_sig = Some(self.project_sig());
            }
        }
        if self.session.is_some() && self.project.saved_sig.is_none() && self.loading.is_none() && self.project.pending_epoch.is_none_or(|e| e != self.epoch) {
            self.project.pending_epoch = None;
            if let Some(p) = self.project.pending_panels.take() {
                self.apply_panel_states(&p);
                self.journal.rebase_panels();
            }
            self.project.saved_sig = Some(self.project_sig());
            if !self.project.recovered && self.project.doc.is_some() {
                self.project.status.clear();
            }
        }
    }

    pub fn project_autosave(&mut self, reason: &'static str) {
        if self.session.is_none() || self.project.job.is_some() || self.loading.is_some() || self.project.saved_sig.is_none() {
            return;
        }
        if reason == "timer" && self.project.last_autosave.is_some_and(|t| t.elapsed().as_secs_f64() < 60.0) {
            return;
        }
        self.project.last_autosave = Some(Instant::now());
        if self.project_dirty() {
            let sig = self.project_sig();
            if reason == "settle" && self.project.last_auto_sig.as_deref() == Some(&sig) {
                return;
            }
            self.project.last_auto_sig = Some(sig);
            self.project.autosave_reason = reason;
            let slot = self.project.slot();
            self.project_save(&slot, true);
        } else if self.project.autosaved {
            self.project.autosaved = false;
            let _ = gf_api::call(&gf_api::Ctx::from_env(), "project.discard", &json!({"name": self.project.slot(), "root": root()}));
        }
    }
}

fn when(v: &Value) -> String {
    let iso = v.as_str().unwrap_or("");
    let b = iso.as_bytes();
    let shaped = b.len() >= 20 && b[10] == b'T' && iso.ends_with('Z') && iso.is_char_boundary(19);
    if !shaped {
        return iso.to_string();
    }
    trf("project.utc_time", &[("date", &&iso[..10]), ("time", &&iso[11..19])])
}

fn message(e: &str) -> String {
    gf_api::exactjson::text(e).ok().and_then(|v| v["message"].as_str().map(str::to_string)).unwrap_or_else(|| e.to_string())
}

#[derive(Clone)]
pub enum Confirm {
    New,
    Open(String),
    Recover(String),
    Pick(usize),
}

#[derive(Default)]
pub struct ProjectUi {
    pub name: String,
    pub confirm: Option<Confirm>,
    pub list_open: bool,
}

impl crate::studio::Studio<'_> {
    pub fn project_panel(&mut self, ui: &mut egui::Ui) {
        let dirty = self.world.project_dirty();
        let title = format!("{} {}{}", tr("ui.project"), self.world.project.name.clone().unwrap_or_else(|| tr("designer.untitled").into()), if dirty { " *" } else { "" });
        egui::CollapsingHeader::new(title).id_salt("project").default_open(false).show(ui, |ui| {
            let busy = self.world.project.job.is_some();
            if self.world.project.list.is_empty() && !self.s.project.list_open && !busy {
                self.s.project.list_open = true;
                self.world.project_list();
            }
            let mut act: Option<&str> = None;
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr("project.new")).on_hover_text(tr("ui.p-new.title")).clicked() {
                    act = Some("new");
                }
                if ui.add_enabled(!busy, egui::Button::new(tr("project.save"))).on_hover_text(tr("ui.p-save.title")).clicked() {
                    act = Some("save");
                }
                if ui.add_enabled(self.world.project.name.is_some() && !busy, egui::Button::new(tr("ui.p-export"))).on_hover_text(tr("ui.p-export.title")).clicked() {
                    act = Some("export");
                }
                if busy {
                    ui.spinner();
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.s.project.name).hint_text(tr("designer.save_as_title")).desired_width(150.0)).on_hover_text(tr("designer.name_rule"));
                if ui.add_enabled(!busy && !self.s.project.name.trim().is_empty(), egui::Button::new(tr("project.save_as"))).clicked() {
                    act = Some("save_as");
                }
            });
            egui::CollapsingHeader::new(tr("project.open")).id_salt("project_open").show(ui, |ui| {
                if ui.small_button(tr("store.refresh")).clicked() {
                    self.world.project_list();
                }
                if self.world.project.list.is_empty() {
                    ui.label(egui::RichText::new(tr("designer.no_projects")).weak());
                }
                for p in self.world.project.list.clone() {
                    let name = p["name"].as_str().unwrap_or_default().to_string();
                    let label = trf("designer.project_row", &[("name", &name), ("modified", &when(&p["modified"])), ("count", &p["characters"])]);
                    if ui.selectable_label(self.world.project.name.as_deref() == Some(&name), label).clicked() {
                        self.s.project.confirm = Some(Confirm::Open(name.clone()));
                    }
                }
                for r in self.world.project.recoveries.clone().into_iter().filter(|r| r["newer_than_saved"] == true) {
                    let name = r["name"].as_str().unwrap_or_default().to_string();
                    ui.horizontal(|ui| {
                        ui.label(trf("designer.recover", &[("name", &name), ("modified", &when(&r["modified"]))]));
                        if ui.small_button(tr("ui.p-restore")).clicked() {
                            self.s.project.confirm = Some(Confirm::Recover(name.clone()));
                        }
                    });
                }
            });
            if let Some(c) = self.s.project.confirm.clone() {
                if dirty {
                    ui.colored_label(ui.visuals().warn_fg_color, tr("designer.unsaved"));
                    ui.horizontal(|ui| {
                        if ui.button(tr("designer.discard_changes")).clicked() {
                            self.s.project.confirm = None;
                            self.run_confirmed(&c);
                        }
                        if ui.button(tr("ui.p-dlg-cancel")).clicked() {
                            self.s.project.confirm = None;
                        }
                    });
                } else {
                    self.s.project.confirm = None;
                    self.run_confirmed(&c);
                }
            }
            if !self.world.project.unresolved.is_empty() {
                ui.label(egui::RichText::new(tr("project.relink")).strong());
                for u in &mut self.world.project.unresolved {
                    ui.label(format!("{} ({})", u["role"].as_str().unwrap_or(""), u["reason"].as_str().unwrap_or("")));
                    let mut given = u["given"].as_str().unwrap_or_default().to_string();
                    if ui.add(egui::TextEdit::singleline(&mut given).hint_text(tr("designer.relink_path")).desired_width(f32::INFINITY)).changed() {
                        u["given"] = json!(given);
                    }
                    if u["optional"] == true {
                        let mut skip = u["skip"] == true;
                        if ui.checkbox(&mut skip, tr("designer.open_without")).changed() {
                            u["skip"] = json!(skip);
                        }
                    }
                }
                if ui.button(tr("ui.p-relink-ok")).clicked()
                    && let Some(name) = self.world.project.doc.as_ref().and_then(|d| d["name"].as_str()).map(str::to_string).or_else(|| self.world.project.last_open.clone())
                {
                    self.world.project_open(&name, false);
                }
            }
            if let Some(doc) = self.world.project.doc.clone() {
                ui.label(egui::RichText::new(tr("ui.characters")).strong());
                for (k, c) in doc["characters"].as_array().into_iter().flatten().enumerate() {
                    let label = format!("{} · {}", c["name"].as_str().unwrap_or(""), trf("designer.revisions", &[("n", &c["history"].as_array().map_or(0, Vec::len))]));
                    if ui.selectable_label(k == self.world.project.active, label).clicked() && k != self.world.project.active {
                        self.s.project.confirm = Some(Confirm::Pick(k));
                    }
                }
            }
            ui.label(egui::RichText::new(tr("ui.notes")).strong());
            ui.add(egui::TextEdit::multiline(&mut self.world.project.notes).hint_text(tr("ui.p-notes.placeholder")).desired_rows(3).desired_width(f32::INFINITY));
            if !self.world.project.status.is_empty() {
                ui.label(egui::RichText::new(&self.world.project.status).weak());
            }
            match act {
                Some("new") => self.s.project.confirm = Some(Confirm::New),
                Some("save") => match self.world.project.name.clone() {
                    Some(n) => self.world.project_save(&n, false),
                    None => self.world.project.status = tr("project.name_first").into(),
                },
                Some("save_as") => {
                    let n = self.s.project.name.trim().to_string();
                    match gf_api::project::check_name(&n, false) {
                        Ok(()) => self.world.project_save(&n, false),
                        Err(e) => self.world.project.status = e.message,
                    }
                }
                Some("export") => self.world.project_export(),
                _ => {}
            }
        });
    }

    fn run_confirmed(&mut self, c: &Confirm) {
        match c {
            Confirm::New => self.world.project_new(),
            Confirm::Open(n) => self.world.project_open(n, false),
            Confirm::Recover(n) => self.world.project_open(n, true),
            Confirm::Pick(k) => self.world.project_pick(*k),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_time_is_shown_without_the_iso_punctuation_and_names_its_zone() {
        assert_eq!(when(&json!("2026-10-23T01:20:19.992Z")), "2026-10-23 01:20:19 UTC");
        assert_eq!(when(&json!("2026-10-23T01:20:19Z")), "2026-10-23 01:20:19 UTC");
        assert_eq!(when(&json!("yesterday")), "yesterday");
        assert_eq!(when(&Value::Null), "");
    }
}
