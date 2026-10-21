use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::body::{Helpers, Point};

#[derive(Clone, Debug, PartialEq)]
pub enum JointRef {
    Cube(String),
    Mean(Vec<u32>),
    Vertex(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoneDef {
    pub name: String,
    pub parent: Option<String>,
    pub head: JointRef,
    pub tail: JointRef,
    pub roll: f64,
    pub default_head: Point,
    pub default_tail: Point,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RigDef {
    pub bones: Vec<BoneDef>,
}

fn joint(bone: &str, end: &str, v: &Value) -> Result<(JointRef, Point), String> {
    let what = || format!("bone {bone} {end}");
    let o = v.as_object().ok_or_else(|| format!("{}: not an object", what()))?;
    let d = o.get("default_position").and_then(Value::as_array).filter(|a| a.len() == 3).ok_or_else(|| format!("{}: no default_position", what()))?;
    let default = [0, 1, 2].map(|k| d[k].as_f64().unwrap_or(f64::NAN));
    let index = |x: &Value| x.as_u64().filter(|&i| i <= u32::MAX as u64).map(|i| i as u32);
    let r = match o.get("strategy").and_then(Value::as_str) {
        Some("CUBE") => JointRef::Cube(o.get("cube_name").and_then(Value::as_str).ok_or_else(|| format!("{}: CUBE without cube_name", what()))?.to_string()),
        Some("MEAN") => {
            let ids: Option<Vec<u32>> = o.get("vertex_indices").and_then(Value::as_array).map(|a| a.iter().map(index).collect()).unwrap_or(None);
            JointRef::Mean(ids.filter(|i| !i.is_empty()).ok_or_else(|| format!("{}: MEAN without vertex_indices", what()))?)
        }
        Some("VERTEX") => JointRef::Vertex(o.get("vertex_index").and_then(index).ok_or_else(|| format!("{}: VERTEX without vertex_index", what()))?),
        other => return Err(format!("{}: strategy {other:?} is not CUBE, MEAN or VERTEX", what())),
    };
    if default.iter().any(|x| !x.is_finite()) {
        return Err(format!("{}: default_position is not 3 numbers", what()));
    }
    Ok((r, default))
}

impl RigDef {
    pub fn parse(v: &Value) -> Result<RigDef, String> {
        let o = v.as_object().ok_or("a rig file is a JSON object of bones")?;
        let o = o.get("bones").and_then(Value::as_object).unwrap_or(o);
        let mut defs = BTreeMap::new();
        for (name, b) in o {
            let Some(b) = b.as_object().filter(|b| b.contains_key("head") || b.contains_key("tail")) else { continue };
            let (head, default_head) = joint(name, "head", b.get("head").unwrap_or(&Value::Null))?;
            let (tail, default_tail) = joint(name, "tail", b.get("tail").unwrap_or(&Value::Null))?;
            let parent = b.get("parent").and_then(Value::as_str).filter(|p| !p.is_empty()).map(str::to_string);
            let roll = b.get("roll").and_then(Value::as_f64).unwrap_or(0.0);
            defs.insert(name.clone(), BoneDef { name: name.clone(), parent, head, tail, roll, default_head, default_tail });
        }
        if defs.is_empty() {
            return Err("the rig file has no bones".into());
        }
        let mut bones = Vec::with_capacity(defs.len());
        let mut placed: HashMap<String, ()> = HashMap::new();
        while bones.len() < defs.len() {
            let before = bones.len();
            for (name, d) in &defs {
                if placed.contains_key(name) {
                    continue;
                }
                let ready = match &d.parent {
                    None => true,
                    Some(p) if !defs.contains_key(p) => return Err(format!("bone {name}: parent {p} is not a bone of the rig")),
                    Some(p) => placed.contains_key(p),
                };
                if ready {
                    placed.insert(name.clone(), ());
                    bones.push(d.clone());
                }
            }
            if bones.len() == before {
                return Err("the rig's parents form a cycle".into());
            }
        }
        Ok(RigDef { bones })
    }

    pub fn evaluate(&self, at: &Resolver) -> Result<Vec<(Point, Point)>, String> {
        self.bones
            .iter()
            .map(|b| Ok((at.joint(&b.head).map_err(|e| format!("bone {} head: {e}", b.name))?, at.joint(&b.tail).map_err(|e| format!("bone {} tail: {e}", b.name))?)))
            .collect()
    }
}

pub type Influences = Vec<(usize, f64)>;

pub fn rig_weights(v: &Value, rig: &RigDef) -> Result<(HashMap<u32, Influences>, Vec<String>), String> {
    let w = v.get("weights").and_then(Value::as_object).ok_or("a weights file has a \"weights\" object of bones")?;
    let index: HashMap<&str, usize> = rig.bones.iter().enumerate().map(|(i, b)| (b.name.as_str(), i)).collect();
    let mut out: HashMap<u32, Influences> = HashMap::new();
    let mut unknown = Vec::new();
    for (bone, list) in w {
        let Some(&j) = index.get(bone.as_str()) else {
            if list.as_array().is_some_and(|a| !a.is_empty()) {
                unknown.push(bone.clone());
            }
            continue;
        };
        for e in list.as_array().ok_or_else(|| format!("weights of {bone}: not a list"))? {
            let (Some(vertex), Some(weight)) = (e.get(0).and_then(Value::as_u64), e.get(1).and_then(Value::as_f64)) else {
                return Err(format!("weights of {bone}: an entry is not [vertex, weight]"));
            };
            if weight.is_finite() && weight > 0.0 {
                out.entry(vertex as u32).or_default().push((j, weight));
            }
        }
    }
    unknown.sort();
    Ok((out, unknown))
}

pub fn mix(parts: &[(f64, &[(usize, f64)])]) -> Influences {
    let mut sum: BTreeMap<usize, f64> = BTreeMap::new();
    for (k, list) in parts {
        for &(j, w) in list.iter() {
            *sum.entry(j).or_default() += k * w;
        }
    }
    sum.into_iter().filter(|(_, w)| *w > 0.0).collect()
}

pub fn strongest(influences: &[(usize, f64)], keep: usize) -> Influences {
    let mut v: Influences = influences.iter().copied().filter(|(_, w)| *w > 0.0).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    v.truncate(keep);
    let total: f64 = v.iter().map(|x| x.1).sum();
    if total > 0.0 {
        for x in v.iter_mut() {
            x.1 /= total;
        }
    }
    v
}

pub struct Resolver<'a> {
    body: &'a [Point],
    body_index: HashMap<u32, u32>,
    helpers: &'a Helpers,
    helper_index: HashMap<u32, u32>,
}

impl<'a> Resolver<'a> {
    pub fn new(body: &'a [Point], source: &[u32], helpers: &'a Helpers) -> Resolver<'a> {
        Resolver {
            body,
            body_index: source.iter().enumerate().map(|(i, &o)| (o, i as u32)).collect(),
            helpers,
            helper_index: helpers.source.iter().enumerate().map(|(i, &o)| (o, i as u32)).collect(),
        }
    }

    pub fn vertex(&self, original: u32) -> Result<Point, String> {
        if let Some(&i) = self.body_index.get(&original) {
            return self.body.get(i as usize).copied().ok_or_else(|| format!("vertex {original}: outside the body"));
        }
        if let Some(&i) = self.helper_index.get(&original) {
            return self.helpers.positions.get(i as usize).copied().ok_or_else(|| format!("vertex {original}: outside the helpers"));
        }
        Err(format!("base.obj vertex {original} is neither a body vertex nor a helper of this pack"))
    }

    pub fn joint(&self, r: &JointRef) -> Result<Point, String> {
        match r {
            JointRef::Cube(g) => self.helpers.mean(g).ok_or_else(|| format!("helper group {g} is not in the pack (re-import the body to carry its helpers)")),
            JointRef::Vertex(v) => self.vertex(*v),
            JointRef::Mean(ids) => {
                let mut m = [0.0; 3];
                for &v in ids {
                    let p = self.vertex(v)?;
                    for k in 0..3 {
                        m[k] += p[k];
                    }
                }
                Ok(m.map(|x| x / ids.len() as f64))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_rig_parses_parents_first_and_resolves_every_strategy() {
        let rig = json!({
            "version": 110,
            "spine": {"parent": "root", "roll": 0.5, "head": {"strategy": "VERTEX", "vertex_index": 7, "default_position": [0, 0, 1]},
                      "tail": {"strategy": "MEAN", "vertex_indices": [7, 100], "default_position": [0, 0, 2]}},
            "root": {"parent": "", "head": {"strategy": "CUBE", "cube_name": "joint-ground", "default_position": [0, 0, 0]},
                     "tail": {"strategy": "VERTEX", "vertex_index": 100, "default_position": [0, 0, 1]}}
        });
        let r = RigDef::parse(&rig).unwrap();
        assert_eq!(r.bones.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["root", "spine"]);
        let body = [[0.0, 0.0, 1.0], [0.0, 2.0, 3.0]];
        let helpers = Helpers { source: vec![100, 101], positions: vec![[1.0, 0.0, 0.0], [3.0, 0.0, 0.0]], groups: [("joint-ground".to_string(), vec![0, 1])].into() };
        let at = Resolver::new(&body, &[5, 7], &helpers);
        let j = r.evaluate(&at).unwrap();
        assert_eq!(j[0], ([2.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
        assert_eq!(j[1], ([0.0, 2.0, 3.0], [0.5, 1.0, 1.5]));
        let cyclic = json!({"a": {"parent": "b", "head": rig["root"]["head"], "tail": rig["root"]["tail"]}, "b": {"parent": "a", "head": rig["root"]["head"], "tail": rig["root"]["tail"]}});
        assert!(RigDef::parse(&cyclic).unwrap_err().contains("cycle"));
        assert!(at.vertex(9).unwrap_err().contains("neither"));
        let nested = json!({"version": 100, "scale_factor": 0.1, "bones": {"root": rig["root"], "spine": rig["spine"]}});
        assert_eq!(RigDef::parse(&nested).unwrap(), r);
        let (w, unknown) = rig_weights(&json!({"weights": {"spine": [[7, 0.25], [9, 1.0]], "root": [[7, 0.75]], "tail": [[1, 1.0]], "empty": []}}), &r).unwrap();
        assert_eq!(unknown, ["tail"]);
        let mut w7 = w[&7].clone();
        w7.sort_by_key(|e| e.0);
        assert_eq!(w7, vec![(0, 0.75), (1, 0.25)]);
        assert_eq!(w[&9], vec![(1, 1.0)]);
        let m = mix(&[(0.5, &[(0, 1.0)]), (0.5, &[(0, 0.5), (1, 0.5)])]);
        assert_eq!(m, vec![(0, 0.75), (1, 0.25)]);
        let s = strongest(&[(0, 0.1), (1, 0.4), (2, 0.3), (3, 0.15), (4, 0.05)], 4);
        assert_eq!(s.iter().map(|x| x.0).collect::<Vec<_>>(), [1, 2, 3, 0]);
        assert!((s.iter().map(|x| x.1).sum::<f64>() - 1.0).abs() < 1e-12);
    }
}
