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

pub struct StoreWear {
    pub reference: String,
    pub name: String,
    pub z_depth: f64,
    pub transparent: bool,
}

#[derive(Default)]
pub struct BodyUi {
    pub key: String,
    pub pairs: Vec<Pair>,
    pub compounds: Vec<String>,
    pub genitals: Vec<String>,
    pub physics: Vec<String>,
    pub wear_key: String,
    pub store_wear: Vec<StoreWear>,
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
        self.body_ui.physics = r["physics"].as_array().into_iter().flatten().filter_map(text).collect();
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
        if next.get("wearables").and_then(Value::as_array).is_some_and(Vec::is_empty) {
            next.remove("wearables");
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

    pub fn wearables_load(&mut self) {
        let Some(s) = &self.session else { return };
        if self.body_ui.wear_key == s.store {
            return;
        }
        self.body_ui.wear_key = s.store.clone();
        let env = gf_api::call(&gf_api::Ctx::from_env(), "wearable.list", &json!({"store": s.store}));
        self.body_ui.store_wear = env["result"]["wearables"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|w| {
                Some(StoreWear {
                    reference: w["wearable"].as_str()?.to_string(),
                    name: w["name"].as_str().unwrap_or("").to_string(),
                    z_depth: w["z_depth"].as_f64().unwrap_or(50.0),
                    transparent: w["transparent"].as_bool().unwrap_or(false),
                })
            })
            .collect();
    }

    pub fn body_wear(&mut self, asset: &str) -> Result<bool, String> {
        if !asset.starts_with("sha256:") {
            return Err(format!("{asset} is not a wearable reference (sha256:...)"));
        }
        Ok(self.body_edit(|b| {
            let list = b.entry("wearables").or_insert_with(|| json!([]));
            if let Some(list) = list.as_array_mut()
                && !list.iter().any(|w| w["asset"] == asset)
            {
                list.push(json!({"asset": asset, "hide_body": true}));
            }
        }))
    }

    pub fn body_unwear(&mut self, asset: &str) -> bool {
        self.body_edit(|b| {
            if let Some(list) = b.get_mut("wearables").and_then(Value::as_array_mut) {
                list.retain(|w| w["asset"] != asset);
            }
        })
    }

    pub fn body_wear_physics(&mut self, asset: &str, kind: &str) -> Result<bool, String> {
        if !(kind == "none" || gf_core::body_shape::WEARABLE_PHYSICS.contains(&kind)) {
            return Err(format!("{kind} is not none or one of {:?}", gf_core::body_shape::WEARABLE_PHYSICS));
        }
        Ok(self.body_edit(|b| {
            for w in b.get_mut("wearables").and_then(Value::as_array_mut).into_iter().flatten() {
                if w["asset"] == asset
                    && let Some(o) = w.as_object_mut()
                {
                    if kind == "none" {
                        o.remove("physics");
                    } else {
                        o.insert("physics".into(), json!(kind));
                    }
                }
            }
        }))
    }

    pub fn body_wear_layer(&mut self, asset: &str, layer: Option<f64>) -> Result<bool, String> {
        if let Some(l) = layer
            && !(l.is_finite() && (0.0..=1000.0).contains(&l))
        {
            return Err(format!("layer {l} is not a number from 0 to 1000"));
        }
        Ok(self.body_edit(|b| {
            for w in b.get_mut("wearables").and_then(Value::as_array_mut).into_iter().flatten() {
                if w["asset"] == asset
                    && let Some(o) = w.as_object_mut()
                {
                    match layer {
                        Some(l) => o.insert("layer".into(), json!(l)),
                        None => o.remove("layer"),
                    };
                }
            }
        }))
    }

    pub fn body_wear_opening(&mut self, asset: &str, metres: f64) -> Result<bool, String> {
        if !(metres.is_finite() && (0.0..=0.2).contains(&metres)) {
            return Err(format!("opening reach {metres} m is not a distance from 0 to 0.2 m"));
        }
        Ok(self.body_edit(|b| {
            for w in b.get_mut("wearables").and_then(Value::as_array_mut).into_iter().flatten() {
                if w["asset"] == asset
                    && let Some(o) = w.as_object_mut()
                {
                    if metres == gf_core::body_shape::OPENING_DEFAULT_M {
                        o.remove("opening_m");
                    } else {
                        o.insert("opening_m".into(), json!(metres));
                    }
                }
            }
        }))
    }

    pub fn body_wear_coarse(&mut self, asset: &str, power: Option<f64>) -> Result<bool, String> {
        if let Some(p) = power
            && !(p.is_finite() && (0.25..=1.0).contains(&p))
        {
            return Err(format!("coarse detail {p} is not from 0.25 to 1"));
        }
        Ok(self.body_edit(|b| {
            for w in b.get_mut("wearables").and_then(Value::as_array_mut).into_iter().flatten() {
                if w["asset"] == asset
                    && let Some(o) = w.as_object_mut()
                {
                    match power {
                        Some(p) => o.insert("coarse_power".into(), json!(p)),
                        None => o.remove("coarse_power"),
                    };
                }
            }
        }))
    }

    pub fn body_wear_hide(&mut self, asset: &str, hide: bool) -> bool {
        self.body_edit(|b| {
            for w in b.get_mut("wearables").and_then(Value::as_array_mut).into_iter().flatten() {
                if w["asset"] == asset {
                    w["hide_body"] = json!(hide);
                }
            }
        })
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
            self.world.wearables_load();
            if let Some(e) = &self.world.body_ui.error {
                ui.colored_label(ui.visuals().error_fg_color, trf("body.sliders_failed", &[("error", e)]));
                return;
            }
            let body = self.world.session.as_ref().map_or(Value::Null, |s| s.character.body.clone());
            let ui_state = &mut self.world.body_ui;
            let mut pair_commit = None;
            let mut compound_commit = None;
            let mut genitals_commit = None;
            let mut wear_commit: Option<(String, Option<bool>)> = None;
            let mut physics_commit: Option<(String, String)> = None;
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
            ui.label(egui::RichText::new(tr("body.wearables")).strong());
            let worn: Vec<(String, bool)> = body["wearables"].as_array().into_iter().flatten().filter_map(|w| Some((w["asset"].as_str()?.to_string(), w["hide_body"].as_bool().unwrap_or(true)))).collect();
            let physics_of = |asset: &str| body["wearables"].as_array().into_iter().flatten().find(|w| w["asset"] == asset).and_then(|w| w["physics"].as_str()).unwrap_or("none").to_string();
            let layer_of = |asset: &str| body["wearables"].as_array().into_iter().flatten().find(|w| w["asset"] == asset).and_then(|w| w["layer"].as_f64());
            let mut layer_commit: Option<(String, Option<f64>)> = None;
            let opening_of = |asset: &str| {
                body["wearables"].as_array().into_iter().flatten().find(|w| w["asset"] == asset).and_then(|w| w["opening_m"].as_f64()).unwrap_or(gf_core::body_shape::OPENING_DEFAULT_M)
            };
            let mut opening_commit: Option<(String, f64)> = None;
            let coarse_of = |asset: &str| body["wearables"].as_array().into_iter().flatten().find(|w| w["asset"] == asset).and_then(|w| w["coarse_power"].as_f64());
            let mut coarse_commit: Option<(String, Option<f64>)> = None;
            let label = |asset: &str| {
                let name = ui_state.store_wear.iter().find(|w| w.reference == asset).map_or("?", |w| w.name.as_str());
                format!("{name} ({})", asset.trim_start_matches("sha256:").get(..8).unwrap_or(asset))
            };
            if worn.is_empty() {
                ui.label(egui::RichText::new(tr("body.wear_none")).weak());
            }
            for (asset, hide) in &worn {
                ui.label(label(asset));
                ui.horizontal_wrapped(|ui| {
                    let mut h = *hide;
                    if ui.checkbox(&mut h, tr("body.wear_hide")).on_hover_text(tr("body.wear_hide_tip")).changed() {
                        wear_commit = Some((asset.clone(), Some(h)));
                    }
                    let current = physics_of(asset);
                    let kind_label = |k: &str| tr(&format!("body.physics.{k}")).to_string();
                    egui::ComboBox::from_id_salt(("body_wear_physics", asset.as_str()))
                        .selected_text(kind_label(&current))
                        .show_ui(ui, |ui| {
                            for kind in std::iter::once("none").chain(ui_state.physics.iter().map(String::as_str)) {
                                if ui.selectable_label(current == kind, kind_label(kind)).clicked() && current != kind {
                                    physics_commit = Some((asset.clone(), kind.to_string()));
                                }
                            }
                        })
                        .response
                        .on_hover_text(tr("body.wear_physics_tip"));
                    let own = layer_of(asset);
                    let z = ui_state.store_wear.iter().find(|w| w.reference == *asset).map_or(50.0, |w| w.z_depth);
                    let mut on = own.is_some();
                    if ui.checkbox(&mut on, tr("body.wear_layer")).on_hover_text(tr("body.wear_layer_tip")).changed() {
                        layer_commit = Some((asset.clone(), on.then_some(z)));
                    }
                    match own {
                        Some(l) => {
                            let key = format!("l:{asset}");
                            let mut v = ui_state.live.get(&key).copied().unwrap_or(l);
                            let r = ui.add(egui::DragValue::new(&mut v).range(0.0..=1000.0).speed(0.5)).on_hover_text(tr("body.wear_layer_value_tip"));
                            if r.changed() {
                                ui_state.live.insert(key.clone(), v);
                            }
                            if (r.drag_stopped() || r.lost_focus() || (r.changed() && !r.dragged() && !r.has_focus())) && ui_state.live.remove(&key).is_some() {
                                layer_commit = Some((asset.clone(), Some(v)));
                            }
                        }
                        None => {
                            ui.add(egui::Label::new(egui::RichText::new(trf("body.wear_layer_auto", &[("z", &z)])).weak()).wrap_mode(egui::TextWrapMode::Extend))
                                .on_hover_text(tr("body.wear_layer_value_tip"));
                        }
                    }
                    let key = format!("o:{asset}");
                    let mut cm = ui_state.live.get(&key).copied().unwrap_or(opening_of(asset) * 100.0);
                    ui.label(tr("body.wear_opening")).on_hover_text(tr("body.wear_opening_tip"));
                    let r = ui.add(egui::DragValue::new(&mut cm).range(0.0..=20.0).speed(0.1).max_decimals(1)).on_hover_text(tr("body.wear_opening_tip"));
                    if r.changed() {
                        ui_state.live.insert(key.clone(), cm);
                    }
                    if (r.drag_stopped() || r.lost_focus() || (r.changed() && !r.dragged() && !r.has_focus())) && ui_state.live.remove(&key).is_some() {
                        opening_commit = Some((asset.clone(), (cm * 10.0).round() / 1000.0));
                    }
                    let kind_default = if ui_state.store_wear.iter().find(|w| w.reference == *asset).is_some_and(|w| w.transparent) { gf_core::body_shape::COARSE_POWER_CARDS } else { 1.0 };
                    let key = format!("k:{asset}");
                    let mut p = ui_state.live.get(&key).copied().unwrap_or(coarse_of(asset).unwrap_or(kind_default));
                    ui.label(tr("body.wear_coarse")).on_hover_text(tr("body.wear_coarse_tip"));
                    let r = ui.add(egui::DragValue::new(&mut p).range(0.25..=1.0).speed(0.01).max_decimals(2)).on_hover_text(tr("body.wear_coarse_tip"));
                    if r.changed() {
                        ui_state.live.insert(key.clone(), p);
                    }
                    if (r.drag_stopped() || r.lost_focus() || (r.changed() && !r.dragged() && !r.has_focus())) && ui_state.live.remove(&key).is_some() {
                        let p = (p * 100.0).round() / 100.0;
                        coarse_commit = Some((asset.clone(), (p != kind_default).then_some(p)));
                    }
                    if ui.small_button(tr("body.wear_remove")).clicked() {
                        wear_commit = Some((asset.clone(), None));
                    }
                });
            }
            let free: Vec<&StoreWear> = ui_state.store_wear.iter().filter(|w| !worn.iter().any(|x| x.0 == w.reference)).collect();
            if free.is_empty() {
                ui.label(egui::RichText::new(tr("body.wear_store_empty")).weak());
            } else {
                egui::ComboBox::from_id_salt("body_wear_add").selected_text(tr("body.wear_add")).show_ui(ui, |ui| {
                    for w in free {
                        if ui.selectable_label(false, label(&w.reference)).clicked() {
                            wear_commit = Some((w.reference.clone(), Some(true)));
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
            if let Some((asset, kind)) = physics_commit
                && let Err(e) = self.world.body_wear_physics(&asset, &kind)
            {
                self.world.body_ui.error = Some(e);
            }
            if let Some((asset, layer)) = layer_commit
                && let Err(e) = self.world.body_wear_layer(&asset, layer)
            {
                self.world.body_ui.error = Some(e);
            }
            if let Some((asset, metres)) = opening_commit
                && let Err(e) = self.world.body_wear_opening(&asset, metres)
            {
                self.world.body_ui.error = Some(e);
            }
            if let Some((asset, power)) = coarse_commit
                && let Err(e) = self.world.body_wear_coarse(&asset, power)
            {
                self.world.body_ui.error = Some(e);
            }
            if let Some((asset, hide)) = wear_commit {
                let worn_now = worn.iter().any(|w| w.0 == asset);
                match (worn_now, hide) {
                    (true, Some(h)) => {
                        self.world.body_wear_hide(&asset, h);
                    }
                    (true, None) => {
                        self.world.body_unwear(&asset);
                    }
                    (false, _) => {
                        let _ = self.world.body_wear(&asset);
                    }
                }
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
