use std::collections::HashMap;
use std::sync::OnceLock;

const EN: &str = include_str!("../../../web/locales/en.json");

fn table() -> &'static HashMap<String, &'static str> {
    static TABLE: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut out = HashMap::new();
        let mut add = |text: &str| {
            if let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(text) {
                for (k, v) in m {
                    if let Some(s) = v.as_str() {
                        out.insert(k, &*Box::leak(s.to_string().into_boxed_str()));
                    }
                }
            }
        };
        add(EN);
        if let Some(path) = std::env::var_os("GAZEFORGE_LOCALE") {
            match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string()).map(|_| t)) {
                Ok(text) => add(&text),
                Err(e) => eprintln!("GAZEFORGE_LOCALE {}: {e}; English shown", std::path::Path::new(&path).display()),
            }
        }
        out
    })
}

pub fn tr(key: &str) -> &'static str {
    if let Some(s) = table().get(key) {
        return s;
    }
    static MISSING: OnceLock<std::sync::Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut m = MISSING.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    m.entry(key.to_string()).or_insert_with(|| Box::leak(key.to_string().into_boxed_str()))
}

pub fn tr_or<'a>(key: &str, otherwise: &'a str) -> &'a str {
    table().get(key).copied().unwrap_or(otherwise)
}

pub fn readable(id: &str) -> String {
    let words: Vec<&str> = id.split(['-', '_']).filter(|w| !w.is_empty()).map(|w| tr_or(&format!("target.frag.{w}"), w)).collect();
    let s = words.join(" ");
    let mut c = s.chars();
    c.next().map_or(s.clone(), |f| f.to_uppercase().chain(c).collect())
}

pub fn trf(key: &str, vars: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut s = tr(key).to_string();
    for (name, value) in vars {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reads_and_fills_placeholders() {
        assert_eq!(tr("ui.wz-run"), "Run");
        assert_eq!(trf("designer.light_n", &[("n", &3)]), "Light 3");
        assert_eq!(tr_or("no.such.key", "fallback"), "fallback");
    }

    #[test]
    fn a_target_id_reads_as_words() {
        assert_eq!(readable("l-cheek-trans-down"), "Left cheek move down");
        assert_eq!(readable("nose-scale-horiz-incr"), "Nose scale horizontal more");
        assert_eq!(readable("mouth-upperlip-height"), "Mouth upper lip height");
        assert_eq!(readable("cupsize"), "Cup size");
        assert_eq!(readable("gender"), "Gender");
        assert_eq!(readable(""), "");
    }
}
