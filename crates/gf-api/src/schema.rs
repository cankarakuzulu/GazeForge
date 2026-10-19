use serde_json::Value;

pub fn validate(schema: &Value, value: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    check(schema, value, "input", &mut problems);
    problems
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn type_matches(want: &str, value: &Value) -> bool {
    let got = type_name(value);
    match want {
        "number" => got == "number" || got == "integer",
        "integer" => got == "integer" || value.as_f64().is_some_and(|x| x.fract() == 0.0),
        other => got == other,
    }
}

fn check(schema: &Value, value: &Value, path: &str, out: &mut Vec<String>) {
    if let Some(want) = schema.get("type").and_then(Value::as_str)
        && !type_matches(want, value)
    {
        out.push(format!("{path}: expected {want}, got {}", type_name(value)));
        return;
    }
    if let Some(options) = schema.get("enum").and_then(Value::as_array)
        && !options.contains(value)
    {
        let list: Vec<String> = options.iter().map(Value::to_string).collect();
        out.push(format!("{path}: must be one of {}, got {value}", list.join(", ")));
    }
    if let Some(x) = value.as_f64() {
        if let Some(lo) = schema.get("minimum").and_then(Value::as_f64)
            && x < lo
        {
            out.push(format!("{path}: {x} is below the minimum {lo}"));
        }
        if let Some(hi) = schema.get("maximum").and_then(Value::as_f64)
            && x > hi
        {
            out.push(format!("{path}: {x} is above the maximum {hi}"));
        }
    }
    if let (Some(s), Some(min)) = (value.as_str(), schema.get("minLength").and_then(Value::as_u64))
        && (s.chars().count() as u64) < min
    {
        out.push(format!("{path}: shorter than {min} characters"));
    }
    if let Some(items) = value.as_array() {
        if let Some(min) = schema.get("minItems").and_then(Value::as_u64)
            && (items.len() as u64) < min
        {
            out.push(format!("{path}: needs at least {min} items, got {}", items.len()));
        }
        if let Some(max) = schema.get("maxItems").and_then(Value::as_u64)
            && (items.len() as u64) > max
        {
            out.push(format!("{path}: takes at most {max} items, got {}", items.len()));
        }
        if let Some(item_schema) = schema.get("items") {
            for (i, item) in items.iter().enumerate() {
                check(item_schema, item, &format!("{path}[{i}]"), out);
            }
        }
    }
    if let Some(object) = value.as_object() {
        let properties = schema.get("properties").and_then(Value::as_object);
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    out.push(format!("{path}.{key}: is required"));
                }
            }
        }
        for (key, item) in object {
            let child = format!("{path}.{key}");
            match properties.and_then(|p| p.get(key)) {
                Some(s) => check(s, item, &child, out),
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => {
                        let mut known: Vec<&str> = properties.map(|p| p.keys().map(String::as_str).collect()).unwrap_or_default();
                        known.sort_unstable();
                        out.push(format!("{child}: unknown field; known fields: {}", known.join(", ")));
                    }
                    Some(s @ Value::Object(_)) => check(s, item, &child, out),
                    _ => {}
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_validator_refuses_what_the_schema_excludes() {
        let s = json!({"type": "object", "required": ["a"], "additionalProperties": false, "properties": {
            "a": {"type": "string", "minLength": 1},
            "w": {"type": "object", "additionalProperties": {"type": "number", "minimum": 0, "maximum": 1}},
            "n": {"type": "integer", "minimum": 0}, "e": {"enum": ["f32", "f64"]},
            "p": {"type": "array", "items": {"type": "integer"}, "minItems": 2, "maxItems": 2}}});
        assert!(validate(&s, &json!({"a": "x", "w": {"t": 0.5}, "n": 3, "e": "f32", "p": [1, 2]})).is_empty());
        let bad = validate(&s, &json!({"w": {"t": 1.5}, "n": 2.5, "e": "f16", "p": [1], "z": 1}));
        assert_eq!(bad.len(), 6, "{bad:?}");
        assert!(validate(&s, &json!([])).iter().any(|p| p.contains("expected object")));
    }
}
