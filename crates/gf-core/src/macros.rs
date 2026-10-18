use std::collections::BTreeMap;

use serde_json::Value;

pub const MACRO_SLIDERS: [&str; 8] = ["gender", "age", "muscle", "weight", "proportions", "height", "cupsize", "firmness"];
pub const RACES: [&str; 3] = ["african", "asian", "caucasian"];
pub const KEY_FLOOR: f64 = 0.01;
pub const LABEL_DIGITS: usize = 4;

const FAMILIES: [(&str, &[&str], &str); 5] = [
    ("racegenderage", &["race", "gender", "age"], "macrodetails/"),
    ("genderagemuscleweight", &["gender", "age", "muscle", "weight"], "macrodetails/universal-"),
    ("genderagemuscleweightproportions", &["gender", "age", "muscle", "weight", "proportions"], "macrodetails/proportions/"),
    ("genderagemuscleweightheight", &["gender", "age", "muscle", "weight", "height"], "macrodetails/height/"),
    ("genderagemuscleweightcupsizefirmness", &["gender", "age", "muscle", "weight", "cupsize", "firmness"], "breast/"),
];

fn not_multiplied(family: &str) -> &'static [&'static str] {
    if family == "genderagemuscleweightcupsizefirmness" { &["gender"] } else { &[] }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Gf,
    Mpfb,
}

impl Mode {
    pub fn parse(text: &str) -> Result<Mode, String> {
        match text {
            "gf" => Ok(Mode::Gf),
            "mpfb" => Ok(Mode::Mpfb),
            other => Err(format!("macro mode {other:?} is neither \"gf\" nor \"mpfb\"")),
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Mode::Gf => "gf",
            Mode::Mpfb => "mpfb",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Part {
    pub lowest: f64,
    pub highest: f64,
    pub low: Option<String>,
    pub high: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MacroDefinition {
    pub parts: BTreeMap<String, Vec<Part>>,
    pub combinations: BTreeMap<String, Vec<String>>,
}

fn label(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

impl MacroDefinition {
    pub fn from_json(root: &Value) -> Result<MacroDefinition, String> {
        let targets = root.get("macrotargets").and_then(Value::as_object).ok_or("macro definition has no macrotargets object")?;
        let mut parts = BTreeMap::new();
        for name in MACRO_SLIDERS {
            let list = targets
                .get(name)
                .and_then(|t| t.get("parts"))
                .and_then(Value::as_array)
                .ok_or_else(|| format!("macro definition has no parts for {name}"))?;
            let mut out = Vec::new();
            for part in list {
                let number = |key: &str| part.get(key).and_then(Value::as_f64).ok_or_else(|| format!("{name}: a part has no number {key}"));
                let (lowest, highest) = (number("lowest")?, number("highest")?);
                if lowest.partial_cmp(&highest) != Some(std::cmp::Ordering::Less) {
                    return Err(format!("{name}: a part has lowest {lowest} >= highest {highest}"));
                }
                out.push(Part { lowest, highest, low: label(part.get("low")), high: label(part.get("high")) });
            }
            if out.is_empty() {
                return Err(format!("{name}: no parts"));
            }
            parts.insert(name.to_string(), out);
        }
        let combos = root.get("combinations").and_then(Value::as_object).ok_or("macro definition has no combinations object")?;
        let mut combinations = BTreeMap::new();
        for (family, variables, _) in FAMILIES {
            let found: Vec<String> = combos
                .get(family)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .ok_or_else(|| format!("macro definition has no combination {family}"))?;
            if found != variables {
                return Err(format!("macro definition combination {family} is {found:?}, expected {variables:?}"));
            }
            combinations.insert(family.to_string(), found);
        }
        Ok(MacroDefinition { parts, combinations })
    }

    pub fn labels_of(&self, name: &str) -> Vec<String> {
        let mut out: Vec<String> =
            self.parts.get(name).map(|ps| ps.iter().flat_map(|p| [p.low.clone(), p.high.clone()]).flatten().collect()).unwrap_or_default();
        out.sort();
        out.dedup();
        out
    }
}

pub fn round_digits(x: f64, digits: usize) -> f64 {
    format!("{x:.digits$}").parse().expect("a formatted float parses")
}

fn part_labels(part: &Part, t: f64, digits: Option<usize>) -> BTreeMap<String, f64> {
    let mut labels = BTreeMap::new();
    let value = |x: f64| digits.map_or(x, |d| round_digits(x, d));
    if let Some(low) = &part.low {
        labels.insert(low.clone(), value(1.0 - t));
    }
    if let Some(high) = &part.high {
        labels.insert(high.clone(), value(t));
    }
    labels
}

pub fn slider_labels(def: &MacroDefinition, name: &str, value: f64, mode: Mode) -> Result<BTreeMap<String, f64>, String> {
    let parts = def.parts.get(name).ok_or_else(|| format!("{name} is not a macro slider"))?;
    for part in parts {
        if part.lowest <= value && value <= part.highest {
            let t = (value - part.lowest) / (part.highest - part.lowest);
            return Ok(part_labels(part, t, (mode == Mode::Mpfb).then_some(LABEL_DIGITS)));
        }
    }
    if mode == Mode::Mpfb {
        return Ok(BTreeMap::new());
    }
    let distance = |p: &Part| (value - p.lowest).abs().min((value - p.highest).abs());
    let mut nearest = &parts[0];
    for part in &parts[1..] {
        if distance(part) < distance(nearest) {
            nearest = part;
        }
    }
    let t = ((value - nearest.lowest) / (nearest.highest - nearest.lowest)).clamp(0.0, 1.0);
    Ok(part_labels(nearest, t, None))
}

pub fn check_sliders(sliders: &BTreeMap<String, f64>) -> Result<(), String> {
    for name in MACRO_SLIDERS.iter().chain(RACES.iter()) {
        let value = sliders.get(*name).ok_or_else(|| format!("slider {name} is missing"))?;
        if !(0.0..=1.0).contains(value) {
            return Err(format!("slider {name}={value} outside [0, 1]"));
        }
    }
    if let Some(extra) = sliders.keys().find(|k| !MACRO_SLIDERS.contains(&k.as_str()) && !RACES.contains(&k.as_str())) {
        return Err(format!("{extra} is not a macro slider; the sliders are {} and {}", MACRO_SLIDERS.join(", "), RACES.join(", ")));
    }
    Ok(())
}

pub fn target_weights(
    def: &MacroDefinition,
    sliders: &BTreeMap<String, f64>,
    exists: Option<&dyn Fn(&str) -> bool>,
    mode: Mode,
) -> Result<BTreeMap<String, f64>, String> {
    check_sliders(sliders)?;
    let mut labels: BTreeMap<&str, Vec<(String, f64)>> = BTreeMap::new();
    for name in MACRO_SLIDERS {
        labels.insert(name, slider_labels(def, name, sliders[name], mode)?.into_iter().collect());
    }
    labels.insert("race", RACES.iter().map(|r| (r.to_string(), sliders[*r])).collect());
    let mut weights = BTreeMap::new();
    for (family, variables, prefix) in FAMILIES {
        if def.combinations.get(family).map(Vec::as_slice) != Some(&variables.iter().map(|v| v.to_string()).collect::<Vec<_>>()[..]) {
            return Err(format!("macro definition combination {family} does not match {variables:?}"));
        }
        let skip = not_multiplied(family);
        let lists: Vec<&Vec<(String, f64)>> = variables.iter().map(|v| &labels[v]).collect();
        let mut combo = vec![0usize; lists.len()];
        if lists.iter().any(|l| l.is_empty()) {
            continue;
        }
        loop {
            let mut weight = 1.0f64;
            for (k, variable) in variables.iter().enumerate() {
                if !skip.contains(variable) {
                    weight *= lists[k][combo[k]].1;
                }
            }
            let name = format!("{prefix}{}", (0..lists.len()).map(|k| lists[k][combo[k]].0.as_str()).collect::<Vec<_>>().join("-"));
            if exists.is_none_or(|f| f(&name)) {
                *weights.entry(name).or_insert(0.0) += weight;
            }
            let mut k = lists.len();
            let done = loop {
                if k == 0 {
                    break true;
                }
                k -= 1;
                combo[k] += 1;
                if combo[k] < lists[k].len() {
                    break false;
                }
                combo[k] = 0;
            };
            if done {
                break;
            }
        }
    }
    Ok(weights)
}

pub fn mpfb_keys(weights: &BTreeMap<String, f64>, floor: f64) -> BTreeMap<String, f64> {
    weights.iter().filter(|(_, w)| **w > floor).map(|(k, w)| (k.clone(), *w)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn definition() -> MacroDefinition {
        let two = |a: &str, b: &str, c: &str| {
            json!({"parts": [{"lowest": -0.01, "highest": 0.49998, "low": a, "high": b},
                             {"lowest": 0.49999, "highest": 1.01, "low": b, "high": c}]})
        };
        MacroDefinition::from_json(&json!({
            "macrotargets": {
                "gender": {"parts": [{"lowest": -0.01, "highest": 1.01, "low": "female", "high": "male"}]},
                "age": {"parts": [{"lowest": -0.01, "highest": 0.1874998, "low": "baby", "high": "child"},
                                  {"lowest": 0.1874999, "highest": 0.49998, "low": "child", "high": "young"},
                                  {"lowest": 0.49999, "highest": 1.01, "low": "young", "high": "old"}]},
                "muscle": two("minmuscle", "averagemuscle", "maxmuscle"),
                "weight": two("minweight", "averageweight", "maxweight"),
                "proportions": {"parts": [{"lowest": -0.01, "highest": 0.4999, "low": "uncommonproportions", "high": ""},
                                          {"lowest": 0.5, "highest": 1.01, "low": "", "high": "idealproportions"}]},
                "height": {"parts": [{"lowest": -0.01, "highest": 0.49, "low": "minheight", "high": ""},
                                     {"lowest": 0.51, "highest": 1.01, "low": "", "high": "maxheight"}]},
                "cupsize": two("mincup", "averagecup", "maxcup"),
                "firmness": two("minfirmness", "averagefirmness", "maxfirmness"),
            },
            "combinations": {
                "racegenderage": ["race", "gender", "age"],
                "genderagemuscleweight": ["gender", "age", "muscle", "weight"],
                "genderagemuscleweightproportions": ["gender", "age", "muscle", "weight", "proportions"],
                "genderagemuscleweightheight": ["gender", "age", "muscle", "weight", "height"],
                "genderagemuscleweightcupsizefirmness": ["gender", "age", "muscle", "weight", "cupsize", "firmness"],
            }
        }))
        .unwrap()
    }

    fn sliders(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
        let mut out: BTreeMap<String, f64> = MACRO_SLIDERS.iter().chain(RACES.iter()).map(|n| (n.to_string(), 0.5)).collect();
        for (k, v) in pairs {
            out.insert(k.to_string(), *v);
        }
        out
    }

    #[test]
    fn rounding_matches_the_reference_half_even_rule() {
        assert_eq!(round_digits(0.03125, 4), 0.0312);
        assert_eq!(round_digits(0.03135, 4), 0.0314);
        assert_eq!(round_digits(2.675, 2), 2.67);
        assert_eq!(round_digits(0.990196, 4), 0.9902);
    }

    #[test]
    fn gender_zero_is_never_pure() {
        let labels = slider_labels(&definition(), "gender", 0.0, Mode::Mpfb).unwrap();
        assert_eq!(labels["female"], 0.9902);
        assert_eq!(labels["male"], 0.0098);
    }

    #[test]
    fn a_gap_drops_the_family_in_mpfb_mode_and_is_continuous_in_gf_mode() {
        let def = definition();
        assert!(slider_labels(&def, "age", 0.18749985, Mode::Mpfb).unwrap().is_empty());
        let s = sliders(&[("age", 0.18749985)]);
        assert!(target_weights(&def, &s, None, Mode::Mpfb).unwrap().is_empty());
        let inside = slider_labels(&def, "age", 0.1874998, Mode::Gf).unwrap();
        let gap = slider_labels(&def, "age", 0.18749985, Mode::Gf).unwrap();
        assert_eq!(inside, gap);
    }

    #[test]
    fn breast_weights_omit_gender_and_missing_files_are_skipped() {
        let def = definition();
        let s = sliders(&[("gender", 0.3), ("cupsize", 0.0), ("firmness", 0.0)]);
        let all = target_weights(&def, &s, None, Mode::Gf).unwrap();
        let breast = all.get("breast/female-young-averagemuscle-averageweight-mincup-minfirmness").copied().unwrap();
        let without_gender = all.get("macrodetails/universal-female-young-averagemuscle-averageweight").copied().unwrap();
        assert!(breast > without_gender);
        let only_breast = |n: &str| n.starts_with("breast/");
        let some = target_weights(&def, &s, Some(&only_breast), Mode::Gf).unwrap();
        assert!(some.keys().all(|k| k.starts_with("breast/")) && !some.is_empty());
    }

    #[test]
    fn sliders_out_of_range_missing_or_unknown_are_refused() {
        let def = definition();
        assert!(target_weights(&def, &sliders(&[("age", 1.5)]), None, Mode::Gf).is_err());
        let mut s = sliders(&[]);
        s.remove("asian");
        assert!(target_weights(&def, &s, None, Mode::Gf).unwrap_err().contains("asian"));
        let mut s = sliders(&[]);
        s.insert("toes".into(), 0.5);
        assert!(target_weights(&def, &s, None, Mode::Gf).unwrap_err().contains("toes"));
        assert!(Mode::parse("MPFB").is_err());
    }

    #[test]
    fn the_floor_keeps_only_weights_above_it() {
        let w: BTreeMap<String, f64> = [("a".to_string(), 0.01), ("b".to_string(), 0.0101)].into();
        assert_eq!(mpfb_keys(&w, KEY_FLOOR).keys().collect::<Vec<_>>(), ["b"]);
    }
}
