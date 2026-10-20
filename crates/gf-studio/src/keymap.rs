use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eframe::egui;
use serde_json::{Value, json};

pub const ACTIONS: &[(&str, &str, &[&str])] = &[
    ("frame", "frame the head in the active viewport", &["Alt+F"]),
    ("layout", "cycle the layout: single, side by side, quad", &["Alt+L"]),
    ("next_viewport", "make the next viewport active", &["Alt+N"]),
    ("mode_next", "next render mode", &["Alt+M"]),
    ("mode_prev", "previous render mode", &["Alt+Shift+M"]),
    ("wire", "wireframe overlay", &["Alt+W"]),
    ("xray", "hidden edges through the surface", &["Alt+X"]),
    ("points", "vertex points", &["Alt+P"]),
    ("back_faces", "back-face highlight", &["Alt+B"]),
    ("seams", "UV seams and open boundaries", &["Alt+E"]),
    ("clip", "clip plane 1", &["Alt+C"]),
    ("eyes", "eye meshes", &["Alt+Y"]),
    ("light_gizmo", "light direction gizmo", &["Alt+G"]),
    ("theme", "dark or light viewport", &["Alt+T"]),
    ("interface_theme", "dark or light interface", &["Alt+Shift+T"]),
    ("screenshot", "save the active viewport as PNG", &["Alt+K"]),
    ("screenshot_layout", "save the whole layout as PNG", &["Alt+Shift+K"]),
    ("fov_down", "narrower field of view", &["Alt+OpenBracket"]),
    ("fov_up", "wider field of view", &["Alt+CloseBracket"]),
    ("lod_coarser", "coarser level of detail", &["Alt+Comma"]),
    ("lod_finer", "finer level of detail", &["Alt+Period"]),
    ("view_front", "front view", &["Alt+1"]),
    ("view_back", "back view", &["Alt+Shift+1"]),
    ("view_left", "left view", &["Alt+3"]),
    ("view_right", "right view", &["Alt+Shift+3"]),
    ("view_top", "top view", &["Alt+7"]),
    ("view_bottom", "bottom view", &["Alt+Shift+7"]),
    ("view_three_quarter", "three-quarter view", &["Alt+9"]),
    ("view_home", "home view of the active viewport, framed", &["Alt+Home"]),
    ("ortho", "orthographic or perspective", &["Alt+5", "Alt+O"]),
    ("undo", "undo the last shape change", &["Ctrl+Z"]),
    ("redo", "redo the last undone shape change", &["Ctrl+Y", "Ctrl+Shift+Z"]),
    ("save_spec", "save the character spec", &["Ctrl+S"]),
    ("reset_shape", "every slider back to the start values", &[]),
    ("hud", "timings overlay", &["Alt+H"]),
    ("help", "show the key map", &["F1"]),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub key: egui::Key,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

pub fn parse(text: &str) -> Result<Binding, String> {
    let mut b = Binding { key: egui::Key::Escape, ctrl: false, shift: false, alt: false };
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (last, mods) = parts.split_last().ok_or_else(|| format!("empty binding {text:?}"))?;
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" | "cmd" | "command" => b.ctrl = true,
            "shift" => b.shift = true,
            "alt" | "option" => b.alt = true,
            other => return Err(format!("unknown modifier {other:?} in {text:?}")),
        }
    }
    b.key = egui::Key::from_name(last).ok_or_else(|| format!("unknown key {last:?} in {text:?}"))?;
    Ok(b)
}

pub struct Keymap {
    pub bindings: Vec<(String, Binding)>,
    pub source: String,
    pub problems: Vec<String>,
}

pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| if cfg!(target_os = "macos") { PathBuf::from(h).join("Library/Application Support") } else { PathBuf::from(h).join(".config") }))?;
    Some(base.join("gazeforge").join("studio-keymap.json"))
}

pub fn defaults_json() -> Value {
    let mut m = serde_json::Map::new();
    for (id, _, keys) in ACTIONS {
        m.insert(id.to_string(), json!(keys));
    }
    json!({"format": "gazeforge.keymap/1", "bindings": m,
           "notes": "each action takes a list of bindings such as \"F\", \"Shift+M\", \"Ctrl+Alt+1\"; an empty list unbinds it; actions left out keep their defaults"})
}

impl Keymap {
    pub fn defaults() -> Keymap {
        let mut bindings = Vec::new();
        for (id, _, keys) in ACTIONS {
            for k in *keys {
                bindings.push((id.to_string(), parse(k).expect("default bindings parse")));
            }
        }
        Keymap { bindings, source: "defaults".into(), problems: Vec::new() }
    }

    pub fn load(path: Option<&Path>) -> Keymap {
        let mut map = Keymap::defaults();
        let Some(path) = path.map(Path::to_path_buf).or_else(default_path) else { return map };
        let Ok(text) = std::fs::read_to_string(&path) else { return map };
        map.source = path.display().to_string();
        let doc: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                map.problems.push(format!("{}: {e}", path.display()));
                return map;
            }
        };
        let Some(given) = doc.get("bindings").and_then(Value::as_object) else {
            map.problems.push(format!("{}: no bindings object", path.display()));
            return map;
        };
        let mut over: BTreeMap<String, Vec<Binding>> = BTreeMap::new();
        for (id, keys) in given {
            if !ACTIONS.iter().any(|a| a.0 == id) {
                map.problems.push(format!("unknown action {id:?}"));
                continue;
            }
            let list = keys.as_array().cloned().unwrap_or_default();
            let mut parsed = Vec::new();
            for k in list {
                match k.as_str().ok_or_else(|| format!("{id}: bindings are strings")).and_then(parse) {
                    Ok(b) => parsed.push(b),
                    Err(e) => map.problems.push(e),
                }
            }
            over.insert(id.clone(), parsed);
        }
        map.bindings.retain(|(id, _)| !over.contains_key(id));
        for (id, list) in over {
            for b in list {
                map.bindings.push((id.clone(), b));
            }
        }
        map
    }

    pub fn actions(&self, ctx: &egui::Context) -> Vec<String> {
        let mut out = Vec::new();
        ctx.input(|i| {
            for e in &i.events {
                if let egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } = e {
                    for (id, b) in &self.bindings {
                        let hit = b.key == *key || physical_key.is_some_and(|p| p == b.key);
                        if hit && b.ctrl == modifiers.command && b.shift == modifiers.shift && b.alt == modifiers.alt {
                            out.push(id.clone());
                        }
                    }
                }
            }
        });
        out
    }

    pub fn describe(&self, id: &str) -> String {
        let keys: Vec<String> = self
            .bindings
            .iter()
            .filter(|(a, _)| a == id)
            .map(|(_, b)| format!("{}{}{}{}", if b.ctrl { "Ctrl+" } else { "" }, if b.alt { "Alt+" } else { "" }, if b.shift { "Shift+" } else { "" }, b.key.name()))
            .collect();
        keys.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_binding_parses_and_actions_are_unique() {
        let map = Keymap::defaults();
        assert!(map.bindings.len() >= ACTIONS.len());
        let mut ids: Vec<&str> = ACTIONS.iter().map(|a| a.0).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ACTIONS.len());
    }

    #[test]
    fn no_default_is_a_bare_printable_key() {
        for (id, b) in &Keymap::defaults().bindings {
            let bare = !b.ctrl && !b.alt && !b.shift;
            let function_key = b.key.name().starts_with('F') && b.key.name().len() > 1;
            assert!(!bare || function_key, "{id} is bound to a bare {}", b.key.name());
            assert!(b.ctrl || b.alt || function_key, "{id} is bound to {} without Ctrl or Alt", b.key.name());
        }
    }

    #[test]
    fn no_two_actions_share_a_default_binding() {
        let map = Keymap::defaults();
        for (i, (a, x)) in map.bindings.iter().enumerate() {
            for (b, y) in &map.bindings[i + 1..] {
                assert!(a == b || x != y, "{a} and {b} share {x:?}");
            }
        }
    }

    #[test]
    fn bindings_parse_modifiers_and_refuse_unknown_keys() {
        let b = parse("Ctrl+Shift+K").unwrap();
        assert!(b.ctrl && b.shift && !b.alt && b.key == egui::Key::K);
        assert!(parse("Hyper+K").is_err());
        assert!(parse("NoSuchKey").is_err());
    }
}
