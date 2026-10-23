use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

use crate::face_rig::{Mode, SHAPES};
use crate::i18n::{tr, tr_or, trf};
use crate::stitch::CONTROLS;
use crate::world::World;

pub const LIMIT: usize = 300;

fn first_difference(a: &Value, b: &Value, path: &mut Vec<String>) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                let (u, v) = (x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null));
                if u != v {
                    path.push(k.clone());
                    first_difference(u, v, path);
                    return true;
                }
            }
            false
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (u, v)) in x.iter().zip(y).enumerate() {
                if u != v {
                    path.push(i.to_string());
                    first_difference(u, v, path);
                    return true;
                }
            }
            false
        }
        _ => a != b,
    }
}

fn pins_label(b: &Value, a: &Value) -> Option<String> {
    let (pb, pa) = (b["pins"]["pins"].as_array()?, a["pins"]["pins"].as_array()?);
    let n = |p: &Value| p["n"].as_u64().unwrap_or(0);
    let only = |x: &[Value], y: &[Value]| x.iter().find(|p| !y.iter().any(|q| n(q) == n(p))).map(n);
    if pa.len() > pb.len() {
        return only(pa, pb).map(|k| trf("pins.undo_add_n", &[("n", &k)]));
    }
    if pa.len() < pb.len() {
        return only(pb, pa).map(|k| trf("pins.undo_delete", &[("n", &k)]));
    }
    pa.iter().zip(pb).find(|(x, y)| x != y).map(|(p, _)| trf("pins.undo_note", &[("n", &n(p))]))
}

fn panel_label(before: &Value, after: &Value) -> String {
    let mut path = Vec::new();
    if !first_difference(before, after, &mut path) || path.is_empty() {
        return tr("history.panels").to_string();
    }
    if path[0] == "pins"
        && let Some(l) = pins_label(before, after)
    {
        return l;
    }
    if path[0] == "view" && path.get(1).is_some_and(|p| p == "box") {
        let on = |v: &Value| v["view"]["box"]["on"] == true;
        return tr(match (on(before), on(after)) {
            (false, true) => "designer.undo.box_hide",
            (true, false) => "designer.undo.box_show",
            _ => "designer.undo.box_move",
        })
        .to_string();
    }
    let panel = tr_or(&format!("panel.{}", path[0]), &crate::wizard::humanize(&path[0])).to_string();
    let Some(leaf) = path.iter().skip(1).rev().find(|p| p.parse::<usize>().is_err()) else { return panel };
    let wear = |k: &str| match k {
        "hair" => Some("body.wear_hair"),
        "physics" => Some("body.wear_physics"),
        "layer" => Some("body.wear_layer"),
        "hide_body" => Some("body.wear_hide"),
        "wearables" => Some("body.wearables"),
        "opening_m" => Some("body.wear_opening"),
        "coarse_power" => Some("body.wear_coarse"),
        "scalp_cap" => Some("body.wear_cap"),
        "strands" => Some("body.wear_strands"),
        _ => None,
    };
    let key = if path.iter().any(|p| p == "groom") && leaf != "groom" { Some(format!("body.groom.{leaf}")) } else { wear(leaf).map(str::to_string) };
    let what = key
        .map(|k| tr_or(&k, "").to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| Some(tr_or(&format!("stitch.control.{leaf}"), "").to_string()).filter(|s| !s.is_empty()))
        .or_else(|| Some(tr_or(&format!("body.{leaf}"), "").to_string()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| crate::wizard::humanize(leaf));
    trf("history.panel_change", &[("panel", &panel), ("what", &what)])
}
const SETTLE: Duration = Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Shape,
    Skin,
    Panels,
}

#[derive(Default)]
pub struct Journal {
    pub done: Vec<Domain>,
    pub undone: Vec<Domain>,
    shape_seen: u64,
    panels_done: Vec<(Value, Value)>,
    panels_undone: Vec<(Value, Value)>,
    committed: Option<Value>,
    last: Option<Value>,
    changed_at: Option<Instant>,
    pub pointer_down: bool,
}

impl Journal {
    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn rebase_panels(&mut self) {
        self.committed = None;
        self.last = None;
        self.changed_at = None;
    }
}

impl World {
    pub fn journal_push(&mut self, d: Domain) {
        if d != Domain::Panels {
            self.journal_flush();
        }
        let j = &mut self.journal;
        j.done.push(d);
        let over = j.done.len().saturating_sub(LIMIT * 3);
        j.done.drain(..over);
        j.undone.clear();
        j.panels_undone.clear();
        if let Some(shape) = self.shape.as_mut() {
            shape.history.undone.clear();
        }
        self.skin.undone.clear();
    }

    fn panel_snapshot(&self) -> Value {
        let f = &self.face;
        let weights: Map<String, Value> = SHAPES.iter().zip(&f.weights).filter(|(_, w)| **w != 0.0).map(|(n, w)| (n.to_string(), json!(w))).collect();
        let manual: Map<String, Value> = f.manual.iter().map(|(j, m)| (j.to_string(), json!(m))).collect();
        let s = &self.stitch;
        let t = &self.timeline;
        json!({"rig": {"mode": f.mode.name(), "weights": weights, "manual": manual},
               "stitch": {"values": s.values, "seam": s.seam, "auto": s.auto, "store": s.store, "body": s.body},
               "timeline": {"speed": t.speed, "loop": t.looping, "apply_head": t.apply_head},
               "pins": self.pins_state(),
               "view": {"head_hidden": self.state.head_hidden, "eyes": self.state.opts.eyes, "regions": self.state.hidden_regions,
                        "hidden": self.hidden_extras, "overlays": self.hidden_overlays,
                        "box": {"on": self.state.opts.clip_box.on, "lo": self.state.opts.clip_box.lo, "hi": self.state.opts.clip_box.hi}},
               "notes": self.project.notes, "body": self.session.as_ref().map_or(Value::Null, |s| s.character.body.clone())})
    }

    fn panel_apply(&mut self, v: &Value) {
        let r = &v["rig"];
        let rig_changed = *r != self.panel_snapshot()["rig"];
        self.face.weights = SHAPES.iter().map(|n| r["weights"][*n].as_f64().unwrap_or(0.0)).collect();
        self.face.mode = if r["mode"] == "joints" { Mode::Joints } else { Mode::Blend };
        self.face.manual = r["manual"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(j, m)| Some((j.parse::<usize>().ok()?, std::array::from_fn(|c| m.get(c).and_then(Value::as_f64).unwrap_or(0.0)))))
            .collect();
        if rig_changed {
            self.face_update();
        }
        let s = &v["stitch"];
        let before = (self.stitch.values, self.stitch.seam);
        for (i, c) in CONTROLS.iter().enumerate() {
            self.stitch.values[i] = s["values"][i].as_f64().map_or(c.4, |x| x.clamp(c.1, c.2));
        }
        self.stitch.seam = s["seam"] == true;
        self.stitch.auto = s["auto"] != false;
        self.stitch.store = s["store"].as_str().unwrap_or_default().to_string();
        self.stitch.body = s["body"].as_str().unwrap_or_default().to_string();
        if before != (self.stitch.values, self.stitch.seam) {
            self.stitch_changed(false);
        }
        let t = &v["timeline"];
        self.timeline.speed = t["speed"].as_f64().unwrap_or(self.timeline.speed);
        self.timeline.looping = t["loop"] != false;
        self.timeline.apply_head = t["apply_head"] != false;
        (self.pins, self.pins_next) = crate::pins::read(&v["pins"]);
        let view = &v["view"];
        self.state.head_hidden = view["head_hidden"] == true;
        self.state.opts.eyes = view["eyes"] != false;
        self.state.hidden_regions = view["regions"].as_u64().unwrap_or(0) as u32;
        self.hidden_extras = view["hidden"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        let names: Vec<String> = self.extras.iter().map(|e| self.extra_name(&e.id)).collect();
        for (e, name) in self.extras.iter_mut().zip(&names) {
            e.visible = !self.hidden_extras.contains(name);
        }
        self.hidden_overlays = view["overlays"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        let b = &view["box"];
        if b.is_object() {
            let three = |v: &Value, d: [f64; 3]| [0, 1, 2].map(|k| v[k].as_f64().unwrap_or(d[k]));
            let cb = &mut self.state.opts.clip_box;
            cb.on = b["on"] == true;
            cb.lo = three(&b["lo"], cb.lo);
            cb.hi = three(&b["hi"], cb.hi);
        }
        self.project.notes = v["notes"].as_str().unwrap_or_default().to_string();
        let body = v["body"].clone();
        if self.session.as_ref().is_some_and(|s| s.character.body != body) {
            if let Some(s) = self.session.as_mut() {
                s.character.body = body;
            }
            self.request_settle();
        }
    }

    pub fn journal_flush(&mut self) {
        let now = self.panel_snapshot();
        match &self.journal.committed {
            None => self.journal.committed = Some(now),
            Some(c) if *c != now => {
                let before = c.clone();
                self.journal.panels_done.push((before, now.clone()));
                let over = self.journal.panels_done.len().saturating_sub(LIMIT);
                self.journal.panels_done.drain(..over);
                self.journal.committed = Some(now.clone());
                self.journal.last = Some(now);
                self.journal_push(Domain::Panels);
            }
            Some(_) => {}
        }
    }

    pub fn journal_tick(&mut self) {
        if let Some(shape) = &self.shape
            && shape.history.pushes != self.journal.shape_seen
        {
            let grown = shape.history.pushes.saturating_sub(self.journal.shape_seen);
            self.journal.shape_seen = shape.history.pushes;
            for _ in 0..grown {
                self.journal_push(Domain::Shape);
            }
        }
        let now = self.panel_snapshot();
        if self.journal.committed.is_none() {
            self.journal.committed = Some(now.clone());
            self.journal.last = Some(now);
            return;
        }
        if self.journal.last.as_ref() != Some(&now) {
            self.journal.last = Some(now);
            self.journal.changed_at = Some(Instant::now());
            return;
        }
        if !self.journal.pointer_down && self.journal.changed_at.is_some_and(|t| t.elapsed() >= SETTLE) {
            self.journal.changed_at = None;
            self.journal_flush();
        }
    }

    pub fn journal_state(&self) -> Value {
        let name = |d: &Domain| format!("{d:?}").to_lowercase();
        json!({"weights": self.shape.as_ref().map(|s| s.live.weights.clone()), "skin": self.skin.spec.as_ref().map(gf_core::skin_layers::SkinSpec::canonical),
               "panels": self.panel_snapshot(), "done": self.journal.done.iter().map(name).collect::<Vec<_>>(), "undone": self.journal.undone.iter().map(name).collect::<Vec<_>>()})
    }

    pub fn journal_reset(&mut self) {
        let pointer_down = self.journal.pointer_down;
        self.journal = Journal { pointer_down, shape_seen: self.shape.as_ref().map_or(0, |s| s.history.pushes), ..Journal::default() };
    }

    fn panel_step(&mut self, redo: bool) -> Option<String> {
        let (before, after) = if redo { self.journal.panels_undone.pop()? } else { self.journal.panels_done.pop()? };
        let target = if redo { after.clone() } else { before.clone() };
        self.panel_apply(&target);
        self.journal.committed = Some(target.clone());
        self.journal.last = Some(target);
        self.journal.changed_at = None;
        let label = panel_label(&before, &after);
        if redo { self.journal.panels_done.push((before, after)) } else { self.journal.panels_undone.push((before, after)) }
        Some(label)
    }

    pub fn undo(&mut self, redo: bool) -> Option<String> {
        self.journal_tick();
        self.journal_flush();
        loop {
            let d = if redo { self.journal.undone.pop()? } else { self.journal.done.pop()? };
            let label = match d {
                Domain::Shape => self.shape_step(redo),
                Domain::Skin => self.skin_step(redo),
                Domain::Panels => self.panel_step(redo),
            };
            if let Some(label) = label {
                if redo { self.journal.done.push(d) } else { self.journal.undone.push(d) }
                return Some(if redo { trf("designer.redone", &[("what", &label)]) } else { trf("designer.undone", &[("what", &label)]) });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_undo_names_the_setting_it_reverts() {
        let hair = |l: f64| json!({"body": {"groom": {"length_m": l, "seed": 1}}, "notes": ""});
        assert_eq!(panel_label(&hair(0.15), &hair(0.2)), format!("{}: {}", tr("panel.body"), tr("body.groom.length_m")));
        let worn = |h: &str| json!({"body": {"wearables": [{"asset": "a"}, {"asset": "b", "hair": h}]}});
        assert_eq!(panel_label(&worn("tuck"), &worn("hide")), format!("{}: {}", tr("panel.body"), tr("body.wear_hair")));
        let stitch = |v: f64| json!({"stitch": {"values": {"blend_mm": v}}});
        assert_eq!(panel_label(&stitch(60.0), &stitch(40.0)), format!("{}: {}", tr("panel.stitch"), tr("stitch.control.blend_mm")));
        assert_eq!(panel_label(&hair(0.2), &hair(0.2)), tr("history.panels"));
    }
}
