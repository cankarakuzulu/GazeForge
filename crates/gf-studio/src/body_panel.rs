use std::collections::BTreeMap;

use eframe::egui;
use serde_json::{Map, Value, json};

use crate::i18n::{tr, trf};
use crate::world::World;

pub struct Pair {
    pub id: String,
    pub family: String,
    pub incr: Option<String>,
    pub decr: Option<String>,
}

#[derive(Default)]
pub struct BodyUi {
    pub key: String,
    pub pairs: Vec<Pair>,
    pub compounds: Vec<String>,
    pub genitals: Vec<String>,
    pub error: Option<String>,
    pub filter: String,
    live: BTreeMap<String, f64>,
}

fn weight(body: &Value, id: &Option<String>) -> f64 {
    id.as_ref().and_then(|t| body["weights"][t].as_f64()).unwrap_or(0.0)
}

pub fn pair_value(body: &Value, p: &Pair) -> f64 {
    weight(body, &p.incr) - weight(body, &p.decr)
}

fn put(map: &mut Map<String, Value>, key: &str, v: f64) {
    if v == 0.0 {
        map.remove(key);
    } else {
        map.insert(key.to_string(), json!(v));
    }
}

impl World {
    pub fn body_sliders_load(&mut self) {
        let Some(s) = &self.session else { return };
        let Some(targets) = s.character.body["targets"].as_str() else { return };
        let key = format!("{}|{targets}|{}", s.store, s.catalog);
        if self.body_ui.key == key {
            return;
        }
        self.body_ui.key = key;
        let env = gf_api::call(&gf_api::Ctx::from_env(), "body.sliders", &json!({"store": s.store, "targets": targets, "catalog": s.catalog}));
        if env["ok"] != true {
            self.body_ui.pairs.clear();
            self.body_ui.compounds.clear();
            self.body_ui.error = Some(env["error"]["message"].as_str().unwrap_or("").to_string());
            return;
        }
        let r = &env["result"];
        let text = |v: &Value| v.as_str().map(str::to_string);
        self.body_ui.pairs = r["pairs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| Pair { id: text(&p["id"]).unwrap_or_default(), family: text(&p["family"]).unwrap_or_default(), incr: text(&p["incr"]), decr: text(&p["decr"]) })
            .collect();
        self.body_ui.compounds = r["compounds"].as_array().into_iter().flatten().filter(|c| c["missing"].as_array().is_none_or(Vec::is_empty)).filter_map(|c| text(&c["name"])).collect();
        self.body_ui.genitals = r["genitals"].as_array().into_iter().flatten().filter_map(text).collect();
        self.body_ui.error = None;
    }

    pub fn body_edit(&mut self, edit: impl FnOnce(&mut Map<String, Value>)) -> bool {
        let Some(s) = self.session.as_mut() else { return false };
        let Some(body) = s.character.body.as_object() else { return false };
        let mut next = body.clone();
        edit(&mut next);
        for field in ["weights", "compounds"] {
            if next.get(field).and_then(Value::as_object).is_some_and(Map::is_empty) {
                next.remove(field);
            }
        }
        let next = Value::Object(next);
        if next == s.character.body {
            return false;
        }
        s.character.body = next;
        self.stitch.wanted = true;
        self.request_settle();
        true
    }

    pub fn body_set_pair(&mut self, id: &str, v: f64) -> Result<bool, String> {
        let p = self.body_ui.pairs.iter().find(|p| p.id == id).ok_or_else(|| format!("{id} is not a body slider"))?;
        let (incr, decr) = (p.incr.clone(), p.decr.clone());
        let lo = if decr.is_some() { -1.0 } else { 0.0 };
        let v = v.clamp(lo, if incr.is_some() { 1.0 } else { 0.0 });
        Ok(self.body_edit(|b| {
            let w = b.entry("weights").or_insert_with(|| json!({}));
            let Some(w) = w.as_object_mut() else { return };
            if let Some(t) = &incr {
                put(w, t, v.max(0.0));
            }
            if let Some(t) = &decr {
                put(w, t, (-v).max(0.0));
            }
        }))
    }

    pub fn body_set_compound(&mut self, name: &str, v: f64) -> Result<bool, String> {
        if !self.body_ui.compounds.iter().any(|c| c == name) {
            return Err(format!("{name} is not a compound parameter of this body"));
        }
        let v = v.clamp(-1.0, 1.0);
        Ok(self.body_edit(|b| {
            let c = b.entry("compounds").or_insert_with(|| json!({}));
            if let Some(c) = c.as_object_mut() {
                put(c, name, v);
            }
        }))
    }

    pub fn body_set_genitals(&mut self, kind: &str) -> Result<bool, String> {
        if !(kind.is_empty() || kind == "none" || self.body_ui.genitals.iter().any(|g| g == kind)) {
            return Err(format!("{kind} is not a genitals kind this body can wear"));
        }
        Ok(self.body_edit(|b| {
            if kind.is_empty() || kind == "none" {
                b.remove("genitals");
            } else {
                b.insert("genitals".into(), json!(kind));
            }
        }))
    }

    pub fn body_reset(&mut self) -> bool {
        self.body_edit(|b| {
            b.remove("weights");
            b.remove("compounds");
        })
    }
}

fn slider(ui: &mut egui::Ui, live: &mut BTreeMap<String, f64>, key: String, value: f64, lo: f64, hi: f64, text: String) -> Option<f64> {
    let mut v = live.get(&key).copied().unwrap_or(value);
    let r = ui.add(egui::Slider::new(&mut v, lo..=hi).step_by(0.01).text(text));
    if r.changed() {
        live.insert(key.clone(), v);
    }
    if r.drag_stopped() || (r.changed() && !r.dragged()) {
        live.remove(&key);
        return Some(v);
    }
    None
}

impl crate::studio::Studio<'_> {
    pub fn body_panel(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(tr("panel.body")).id_salt("body").default_open(false).show(ui, |ui| {
            let Some(s) = &self.world.session else {
                ui.label(egui::RichText::new(tr("studio.load_first")).weak());
                return;
            };
            if s.character.body.is_null() {
                ui.label(egui::RichText::new(tr("body.no_body")).weak());
                return;
            }
            self.world.body_sliders_load();
            if let Some(e) = &self.world.body_ui.error {
                ui.colored_label(ui.visuals().error_fg_color, trf("body.sliders_failed", &[("error", e)]));
                return;
            }
            let body = self.world.session.as_ref().map_or(Value::Null, |s| s.character.body.clone());
            let ui_state = &mut self.world.body_ui;
            let mut pair_commit = None;
            let mut compound_commit = None;
            let mut genitals_commit = None;
            if !ui_state.genitals.is_empty() {
                let current = body["genitals"].as_str().unwrap_or("none").to_string();
                let label = |k: &str| tr(&format!("body.genitals.{k}")).to_string();
                egui::ComboBox::from_label(tr("body.genitals")).selected_text(label(&current)).show_ui(ui, |ui| {
                    for kind in std::iter::once("none").chain(ui_state.genitals.iter().map(String::as_str)) {
                        if ui.selectable_label(current == kind, label(kind)).clicked() && current != kind {
                            genitals_commit = Some(kind.to_string());
                        }
                    }
                });
                ui.separator();
            }
            ui.label(egui::RichText::new(tr("body.compounds")).strong());
            for name in &ui_state.compounds {
                let value = body["compounds"][name].as_f64().unwrap_or(0.0);
                if let Some(v) = slider(ui, &mut ui_state.live, format!("c:{name}"), value, -1.0, 1.0, tr(&format!("body.compound.{name}")).to_string()) {
                    compound_commit = Some((name.clone(), v));
                }
            }
            ui.separator();
            ui.label(egui::RichText::new(tr("body.regional")).strong());
            ui.add(egui::TextEdit::singleline(&mut ui_state.filter).hint_text(tr("body.filter")).desired_width(f32::INFINITY));
            let needle = ui_state.filter.trim().to_lowercase();
            let mut families: Vec<&str> = ui_state.pairs.iter().map(|p| p.family.as_str()).collect();
            families.dedup();
            for family in families {
                let rows: Vec<(&Pair, String)> = ui_state
                    .pairs
                    .iter()
                    .filter(|p| p.family == family)
                    .map(|p| (p, tr(&format!("body.slider.{}", p.id)).to_string()))
                    .filter(|(p, label)| needle.is_empty() || label.to_lowercase().contains(&needle) || p.id.contains(&needle))
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                let set = rows.iter().filter(|(p, _)| pair_value(&body, p) != 0.0).count();
                let title = trf("body.family_line", &[("family", &tr(&format!("body.family.{family}"))), ("set", &set), ("count", &rows.len())]);
                egui::CollapsingHeader::new(title).id_salt(("body_family", family)).open((!needle.is_empty()).then_some(true)).show(ui, |ui| {
                    for (p, label) in rows {
                        let lo = if p.decr.is_some() { -1.0 } else { 0.0 };
                        if let Some(v) = slider(ui, &mut ui_state.live, format!("p:{}", p.id), pair_value(&body, p), lo, if p.incr.is_some() { 1.0 } else { 0.0 }, label) {
                            pair_commit = Some((p.id.clone(), v));
                        }
                    }
                });
            }
            ui.separator();
            let shaped = body.get("weights").is_some() || body.get("compounds").is_some();
            let reset = ui.add_enabled(shaped, egui::Button::new(tr("body.reset"))).clicked();
            if let Some((id, v)) = pair_commit {
                let _ = self.world.body_set_pair(&id, v);
            }
            if let Some((name, v)) = compound_commit {
                let _ = self.world.body_set_compound(&name, v);
            }
            if let Some(kind) = genitals_commit {
                let _ = self.world.body_set_genitals(&kind);
            }
            if reset {
                self.world.body_reset();
            }
        });
    }
}
