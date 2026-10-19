use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::session::Character;

pub struct TargetInfo {
    pub id: String,
    pub category: String,
    pub label: String,
}

pub struct MacroInfo {
    pub name: String,
    pub base: f64,
    pub start: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Values {
    pub weights: BTreeMap<String, f64>,
    pub macros: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Weight(String, f64),
    Macro(String, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ShapeEvent {
    Begin(Vec<String>),
    Preview(Vec<Edit>),
    Commit(String),
    Reset,
}

pub struct Step {
    pub label: String,
    pub before: Values,
    pub after: Values,
}

#[derive(Default)]
pub struct History {
    pub done: Vec<Step>,
    pub undone: Vec<Step>,
}

pub const HISTORY_LIMIT: usize = 300;

#[derive(Default, Debug, PartialEq)]
pub struct Outcome {
    pub preview: bool,
    pub settle: bool,
    pub begin: Vec<String>,
    pub committed: Option<String>,
}

pub struct ShapeModel {
    pub targets: Vec<TargetInfo>,
    pub categories: Vec<(String, usize)>,
    pub macros: Vec<MacroInfo>,
    pub macros_available: bool,
    pub macros_reason: Option<String>,
    pub macro_mode: String,
    pub head_scale: f64,
    pub start_had_macros: bool,
    pub start: Values,
    pub committed: Values,
    pub live: Values,
    pub history: History,
    pub generation: u64,
}

impl Values {
    fn set(&mut self, e: &Edit) -> bool {
        match e {
            Edit::Weight(id, x) => {
                let x = x.clamp(0.0, 1.0);
                let before = self.weights.get(id).copied().unwrap_or(0.0);
                if x == 0.0 {
                    self.weights.remove(id);
                } else {
                    self.weights.insert(id.clone(), x);
                }
                before != x
            }
            Edit::Macro(name, x) => {
                let x = x.clamp(0.0, 1.0);
                self.macros.insert(name.clone(), x) != Some(x)
            }
        }
    }

    pub fn weight(&self, id: &str) -> f64 {
        self.weights.get(id).copied().unwrap_or(0.0)
    }
}

impl ShapeModel {
    pub fn from_sliders(sl: &Value) -> ShapeModel {
        let mut targets = Vec::new();
        let mut categories: Vec<(String, usize)> = Vec::new();
        let mut list: Vec<&Value> = sl["targets"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
        list.sort_by(|a, b| (a["category"].as_str(), a["id"].as_str()).cmp(&(b["category"].as_str(), b["id"].as_str())));
        for t in list {
            let id = t["id"].as_str().unwrap_or_default().to_string();
            let category = t["category"].as_str().unwrap_or_default().to_string();
            match categories.last_mut() {
                Some((c, n)) if *c == category => *n += 1,
                _ => categories.push((category.clone(), 1)),
            }
            targets.push(TargetInfo { label: id.rsplit('/').next().unwrap_or(&id).to_string(), id, category });
        }
        let m = &sl["macros"];
        let start = &sl["start"];
        let macros: Vec<MacroInfo> = m["sliders"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|s| MacroInfo { name: s["name"].as_str().unwrap_or_default().to_string(), base: s["base"].as_f64().unwrap_or(0.5), start: s["value"].as_f64().or(s["base"].as_f64()).unwrap_or(0.5) })
                    .collect()
            })
            .unwrap_or_default();
        let weights: BTreeMap<String, f64> = start["weights"].as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_f64().filter(|x| *x != 0.0).map(|x| (k.clone(), x))).collect()).unwrap_or_default();
        let start_values = Values { weights, macros: macros.iter().map(|m| (m.name.clone(), m.start)).collect() };
        ShapeModel {
            targets,
            categories,
            macros,
            macros_available: m["available"].as_bool().unwrap_or(false),
            macros_reason: m["reason"].as_str().map(str::to_string),
            macro_mode: start["macros"]["mode"].as_str().unwrap_or("gf").to_string(),
            head_scale: start["macros"]["head_scale"].as_f64().unwrap_or(1.0),
            start_had_macros: !start["macros"].is_null(),
            committed: start_values.clone(),
            live: start_values.clone(),
            start: start_values,
            history: History::default(),
            generation: 0,
        }
    }

    pub fn apply(&mut self, ev: ShapeEvent) -> Outcome {
        let mut out = Outcome::default();
        match ev {
            ShapeEvent::Begin(keys) => out.begin = keys,
            ShapeEvent::Preview(edits) => {
                let mut changed = false;
                for e in &edits {
                    changed |= self.live.set(e);
                }
                if changed {
                    self.generation += 1;
                    out.preview = true;
                }
            }
            ShapeEvent::Commit(label) => {
                if self.live != self.committed {
                    self.push(label.clone(), self.committed.clone(), self.live.clone());
                    self.committed = self.live.clone();
                    out.settle = true;
                    out.committed = Some(label);
                }
            }
            ShapeEvent::Reset => {
                let before = self.live.clone();
                if before != self.start {
                    self.push("reset all sliders".into(), self.committed.clone(), self.start.clone());
                    self.live = self.start.clone();
                    self.committed = self.start.clone();
                    self.generation += 1;
                    out.preview = true;
                    out.settle = true;
                    out.committed = Some("reset all sliders".into());
                }
            }
        }
        out
    }

    fn push(&mut self, label: String, before: Values, after: Values) {
        self.history.done.push(Step { label, before, after });
        if self.history.done.len() > HISTORY_LIMIT {
            self.history.done.remove(0);
        }
        self.history.undone.clear();
    }

    fn jump(&mut self, to: Values) {
        self.live = to.clone();
        self.committed = to;
        self.generation += 1;
    }

    pub fn undo(&mut self) -> Option<String> {
        let s = self.history.done.pop()?;
        self.jump(s.before.clone());
        let label = s.label.clone();
        self.history.undone.push(s);
        Some(label)
    }

    pub fn redo(&mut self) -> Option<String> {
        let s = self.history.undone.pop()?;
        self.jump(s.after.clone());
        let label = s.label.clone();
        self.history.done.push(s);
        Some(label)
    }

    pub fn macros_on(&self, v: &Values) -> bool {
        self.macros_available && (self.start_had_macros || self.macros.iter().any(|m| v.macros.get(&m.name).copied() != Some(m.base)))
    }

    pub fn character(&self, base: &Character, v: &Values) -> Character {
        let macros = if self.macros_on(v) { json!({"sliders": v.macros, "mode": self.macro_mode, "head_scale": self.head_scale}) } else { Value::Null };
        Character { weights: v.weights.iter().filter(|(_, x)| **x > 0.0).map(|(k, x)| (k.clone(), *x)).collect(), macros, ..base.clone() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> ShapeModel {
        ShapeModel::from_sliders(&json!({
            "targets": [{"id": "nose/b", "category": "nose"}, {"id": "chin/a", "category": "chin"}, {"id": "nose/a", "category": "nose"}],
            "macros": {"available": true, "sliders": [{"name": "age", "base": 0.5, "value": 0.5}]},
            "start": {"weights": {"chin/a": 0.25}, "macros": null}
        }))
    }

    #[test]
    fn targets_group_by_category_and_start_values_load() {
        let m = model();
        assert_eq!(m.categories, vec![("chin".to_string(), 1), ("nose".to_string(), 2)]);
        assert_eq!(m.targets[1].label, "a");
        assert_eq!(m.live.weight("chin/a"), 0.25);
        assert!(!m.macros_on(&m.live));
    }

    #[test]
    fn a_drag_previews_then_one_commit_makes_one_undo_step() {
        let mut m = model();
        assert!(m.apply(ShapeEvent::Preview(vec![Edit::Weight("nose/a".into(), 0.3)])).preview);
        assert!(m.apply(ShapeEvent::Preview(vec![Edit::Weight("nose/a".into(), 0.6)])).preview);
        assert_eq!(m.committed.weight("nose/a"), 0.0);
        let o = m.apply(ShapeEvent::Commit("nose/a".into()));
        assert!(o.settle);
        assert_eq!(m.history.done.len(), 1);
        assert!(!m.apply(ShapeEvent::Commit("again".into())).settle);
        m.apply(ShapeEvent::Preview(vec![Edit::Macro("age".into(), 0.9)]));
        m.apply(ShapeEvent::Commit("age".into()));
        assert!(m.macros_on(&m.live));
        assert_eq!(m.undo().as_deref(), Some("age"));
        assert_eq!(m.live.macros["age"], 0.5);
        assert_eq!(m.undo().as_deref(), Some("nose/a"));
        assert_eq!(m.live.weight("nose/a"), 0.0);
        assert_eq!(m.redo().as_deref(), Some("nose/a"));
        assert_eq!(m.live.weight("nose/a"), 0.6);
        let o = m.apply(ShapeEvent::Reset);
        assert!(o.settle && m.history.undone.is_empty());
        assert_eq!(m.live, m.start);
        let c = m.character(&Character { name: "x".into(), placement: "f5/1".into(), weights: BTreeMap::new(), macros: Value::Null, residual: Value::Null }, &m.live);
        assert_eq!(c.weights.len(), 1);
        assert!(c.macros.is_null());
    }
}
