use serde_json::Value;

pub const FORMAT: &str = "gazeforge.groom/1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Left,
    Right,
    Centre,
    None,
}

#[derive(Clone, Debug)]
pub struct Groom {
    pub name: String,
    pub strands: usize,
    pub points: usize,
    pub length_m: f64,
    pub part: Part,
    pub droop: f64,
    pub curl_radius_m: f64,
    pub curl_turns_per_m: f64,
    pub clumps: usize,
    pub clump: f64,
    pub frizz_m: f64,
    pub colour: [f32; 3],
    pub seed: u64,
}

fn linear(c: f64) -> f32 {
    let s = c.clamp(0.0, 1.0);
    (if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }) as f32
}

pub struct Field {
    pub key: &'static str,
    pub default: f64,
    pub lo: f64,
    pub hi: f64,
    pub integer: bool,
}

pub const FIELDS: &[Field] = &[
    Field { key: "strands", default: 20000.0, lo: 1.0, hi: 500_000.0, integer: true },
    Field { key: "points", default: 16.0, lo: 3.0, hi: 64.0, integer: true },
    Field { key: "length_m", default: 0.15, lo: 0.005, hi: 2.0, integer: false },
    Field { key: "droop", default: 0.5, lo: 0.0, hi: 1.0, integer: false },
    Field { key: "curl_radius_m", default: 0.0, lo: 0.0, hi: 0.1, integer: false },
    Field { key: "curl_turns_per_m", default: 0.0, lo: 0.0, hi: 200.0, integer: false },
    Field { key: "clumps", default: 0.0, lo: 0.0, hi: 5000.0, integer: true },
    Field { key: "clump", default: 0.0, lo: 0.0, hi: 1.0, integer: false },
    Field { key: "frizz_m", default: 0.0, lo: 0.0, hi: 0.05, integer: false },
    Field { key: "seed", default: 1.0, lo: 0.0, hi: u32::MAX as f64, integer: true },
];

pub const PARTS: &[&str] = &["centre", "left", "right", "none"];

pub fn starter() -> Value {
    serde_json::json!({"format": FORMAT, "name": "groom", "strands": 20000, "length_m": 0.15, "part": "centre", "droop": 0.5, "clumps": 300, "clump": 0.4, "frizz_m": 0.002, "colour": "#3b2416", "seed": 1})
}

pub fn parse(v: &Value) -> Result<Groom, String> {
    if v["format"].as_str() != Some(FORMAT) {
        return Err(format!("a groom file says \"format\": \"{FORMAT}\""));
    }
    if let Some(o) = v.as_object()
        && let Some(k) = o.keys().find(|k| !["format", "name", "part", "colour"].contains(&k.as_str()) && !FIELDS.iter().any(|f| f.key == k.as_str()))
    {
        return Err(format!("groom {k} is not a groom setting"));
    }
    let num = |key: &str| -> Result<f64, String> {
        let f = FIELDS.iter().find(|f| f.key == key).ok_or_else(|| format!("groom {key} is not a groom setting"))?;
        let x = match v.get(key) {
            None | Some(Value::Null) => f.default,
            Some(x) => x.as_f64().ok_or_else(|| format!("groom {key} must be a number"))?,
        };
        if !(f.lo..=f.hi).contains(&x) {
            return Err(format!("groom {key} must be from {} to {}, got {x}", f.lo, f.hi));
        }
        Ok(x)
    };
    let part = match v["part"].as_str().unwrap_or("centre") {
        "left" => Part::Left,
        "right" => Part::Right,
        "centre" | "center" => Part::Centre,
        "none" => Part::None,
        other => return Err(format!("groom part is left, right, centre or none, got {other}")),
    };
    let colour = match v["colour"].as_str() {
        None => [0.18, 0.11, 0.07].map(linear),
        Some(hex) => {
            let h = hex.trim_start_matches('#');
            let c = (h.len() == 6).then(|| (0..3).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect::<Option<Vec<u8>>>()).flatten();
            let c = c.ok_or_else(|| format!("groom colour is #rrggbb, got {hex}"))?;
            [0, 1, 2].map(|i| linear(c[i] as f64 / 255.0))
        }
    };
    Ok(Groom {
        name: v["name"].as_str().unwrap_or("groom").to_string(),
        strands: num("strands")? as usize,
        points: num("points")? as usize,
        length_m: num("length_m")?,
        part,
        droop: num("droop")?,
        curl_radius_m: num("curl_radius_m")?,
        curl_turns_per_m: num("curl_turns_per_m")?,
        clumps: num("clumps")? as usize,
        clump: num("clump")?,
        frizz_m: num("frizz_m")?,
        colour,
        seed: num("seed")? as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_groom_file_reads_with_defaults_and_refuses_nonsense() {
        let g = parse(&json!({"format": FORMAT, "length_m": 0.3, "part": "left", "colour": "#806040"})).unwrap();
        assert_eq!((g.length_m, g.part, g.points, g.strands), (0.3, Part::Left, 16, 20000));
        assert!((g.colour[0] - linear(128.0 / 255.0)).abs() < 1e-6);
        assert!(parse(&json!({"length_m": 0.3})).is_err(), "the format is required");
        assert!(parse(&json!({"format": FORMAT, "part": "middle"})).is_err());
        assert!(parse(&json!({"format": FORMAT, "droop": 2.0})).is_err());
        assert!(parse(&json!({"format": FORMAT, "colour": "brown"})).is_err());
        assert!(parse(&json!({"format": FORMAT, "lenght_m": 0.3})).is_err(), "a misspelt setting is refused, not ignored");
        let s = parse(&starter()).unwrap();
        assert_eq!((s.strands, s.clumps, s.part), (20000, 300, Part::Centre));
    }
}
