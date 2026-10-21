use std::collections::BTreeMap;

use gf_core::eyerig::EyeballSpec;
use serde_json::{Value, json};

use crate::session::Character;

pub struct TargetInfo {
    pub id: String,
    pub category: String,
    pub label: String,
}

pub type Ends = (&'static str, &'static str);

pub const OPPOSITES: [Ends; 6] = [("decr", "incr"), ("down", "up"), ("in", "out"), ("backward", "forward"), ("concave", "convex"), ("compress", "uncompress")];

#[derive(Clone, Debug, PartialEq)]
pub enum Slider {
    One(usize),
    Pair { label: String, neg: usize, pos: usize, ends: Ends },
}

impl Slider {
    pub fn first(&self) -> usize {
        match self {
            Slider::One(i) => *i,
            Slider::Pair { neg, pos, .. } => *neg.min(pos),
        }
    }

    pub fn matches(&self, targets: &[TargetInfo], q: &str) -> bool {
        match self {
            Slider::One(i) => targets[*i].id.to_lowercase().contains(q),
            Slider::Pair { neg, pos, .. } => targets[*neg].id.to_lowercase().contains(q) || targets[*pos].id.to_lowercase().contains(q),
        }
    }
}

pub fn pair_sliders(targets: &[TargetInfo]) -> Vec<Slider> {
    let at: BTreeMap<&str, usize> = targets.iter().enumerate().map(|(i, t)| (t.id.as_str(), i)).collect();
    let mut partner: Vec<Option<(usize, bool, Ends)>> = vec![None; targets.len()];
    for (i, t) in targets.iter().enumerate() {
        let Some((stem, end)) = t.id.rsplit_once('-') else { continue };
        for &(neg, pos) in &OPPOSITES {
            if let (true, Some(&j)) = (end == neg, at.get(format!("{stem}-{pos}").as_str())) {
                partner[i] = Some((j, true, (neg, pos)));
                partner[j] = Some((i, false, (neg, pos)));
            }
        }
    }
    let mut out = Vec::new();
    for (i, t) in targets.iter().enumerate() {
        match partner[i] {
            None => out.push(Slider::One(i)),
            Some((j, is_neg, ends)) if i < j => {
                let (neg, pos) = if is_neg { (i, j) } else { (j, i) };
                let stem = t.label.rsplit_once('-').map_or(t.label.as_str(), |(s, _)| s).to_string();
                out.push(Slider::Pair { label: stem, neg, pos, ends });
            }
            Some(_) => {}
        }
    }
    out
}

pub fn pair_edits(neg: &str, pos: &str, x: f64) -> Vec<Edit> {
    vec![Edit::Weight(neg.to_string(), (-x).max(0.0)), Edit::Weight(pos.to_string(), x.max(0.0))]
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
    pub eyeballs: Option<EyeballSpec>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Weight(String, f64),
    Macro(String, f64),
    Eyes(Option<EyeballSpec>),
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
    pub pushes: u64,
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
    pub sliders: Vec<Slider>,
    pub categories: Vec<(String, usize)>,
    pub macros: Vec<MacroInfo>,
    pub macros_available: bool,
    pub macros_reason: Option<String>,
    pub macro_mode: String,
    pub head_scale: f64,
    pub start_had_macros: bool,
    pub eyerig: Option<String>,
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
            Edit::Eyes(e) => {
                let changed = self.eyeballs != *e;
                self.eyeballs = e.clone();
                changed
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
        let eyeballs = serde_json::from_value::<EyeballSpec>(start["eyeballs"].clone()).ok();
        let start_values = Values { weights, macros: macros.iter().map(|m| (m.name.clone(), m.start)).collect(), eyeballs };
        let sliders = pair_sliders(&targets);
        ShapeModel {
            targets,
            sliders,
            categories,
            macros,
            macros_available: m["available"].as_bool().unwrap_or(false),
            macros_reason: m["reason"].as_str().map(str::to_string),
            macro_mode: start["macros"]["mode"].as_str().unwrap_or("gf").to_string(),
            head_scale: start["macros"]["head_scale"].as_f64().unwrap_or(1.0),
            start_had_macros: !start["macros"].is_null(),
            eyerig: sl["eyerig"].as_str().map(str::to_string),
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
                    self.push(crate::i18n::tr("designer.undo.reset").into(), self.committed.clone(), self.start.clone());
                    self.live = self.start.clone();
                    self.committed = self.start.clone();
                    self.generation += 1;
                    out.preview = true;
                    out.settle = true;
                    out.committed = Some(crate::i18n::tr("designer.undo.reset").into());
                }
            }
        }
        out
    }

    fn push(&mut self, label: String, before: Values, after: Values) {
        self.history.done.push(Step { label, before, after });
        self.history.pushes += 1;
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
        let eyeballs = v.eyeballs.as_ref().and_then(|e| serde_json::to_value(e).ok()).unwrap_or(Value::Null);
        Character { weights: v.weights.iter().filter(|(_, x)| **x > 0.0).map(|(k, x)| (k.clone(), *x)).collect(), macros, eyeballs, ..base.clone() }
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
    fn opposite_targets_pair_into_one_slider_and_the_rest_stay_single() {
        let m = ShapeModel::from_sliders(&json!({
            "targets": [{"id": "cheek/l-cheek-bones-incr", "category": "cheek"}, {"id": "cheek/l-cheek-bones-decr", "category": "cheek"},
                        {"id": "nose/nose-trans-in", "category": "nose"}, {"id": "nose/nose-trans-out", "category": "nose"},
                        {"id": "nose/nose-scale-incr", "category": "nose"}, {"id": "chin/chin-triangle", "category": "chin"},
                        {"id": "chin/chin-width-incr", "category": "chin"}],
            "macros": {"available": false}, "start": {"weights": {}, "macros": null}
        }));
        let id = |i: usize| m.targets[i].id.as_str();
        let pairs: Vec<(String, &str, &str, (&str, &str))> =
            m.sliders.iter().filter_map(|s| if let Slider::Pair { label, neg, pos, ends } = s { Some((label.clone(), id(*neg), id(*pos), *ends)) } else { None }).collect();
        assert_eq!(pairs, vec![
            ("l-cheek-bones".to_string(), "cheek/l-cheek-bones-decr", "cheek/l-cheek-bones-incr", ("decr", "incr")),
            ("nose-trans".to_string(), "nose/nose-trans-in", "nose/nose-trans-out", ("in", "out")),
        ]);
        let singles: Vec<&str> = m.sliders.iter().filter_map(|s| if let Slider::One(i) = s { Some(id(*i)) } else { None }).collect();
        assert_eq!(singles, vec!["chin/chin-triangle", "chin/chin-width-incr", "nose/nose-scale-incr"]);
        assert_eq!(pair_edits("a", "b", -0.4), vec![Edit::Weight("a".into(), 0.4), Edit::Weight("b".into(), 0.0)]);
        assert_eq!(pair_edits("a", "b", 0.7), vec![Edit::Weight("a".into(), 0.0), Edit::Weight("b".into(), 0.7)]);
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
        let c = m.character(&Character { name: "x".into(), placement: "f5/1".into(), weights: BTreeMap::new(), macros: Value::Null, residual: Value::Null, eyeballs: Value::Null, skin: Value::Null, body: Value::Null }, &m.live);
        assert_eq!(c.weights.len(), 1);
        assert!(c.macros.is_null());
    }
}
