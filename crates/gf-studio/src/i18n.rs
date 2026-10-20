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
        if let Some(path) = std::env::var_os("GAZEFORGE_LOCALE")
            && let Ok(text) = std::fs::read_to_string(path)
        {
            add(&text);
        }
        out
    })
}

pub fn tr(key: &str) -> &'static str {
    table().get(key).copied().unwrap_or_else(|| Box::leak(key.to_string().into_boxed_str()))
}

pub fn tr_or<'a>(key: &str, otherwise: &'a str) -> &'a str {
    table().get(key).copied().unwrap_or(otherwise)
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
}
