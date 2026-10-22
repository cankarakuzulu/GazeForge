use serde_json::Value;

pub const FORMAT: &str = "gazeforge.groom/1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    Left,
    Right,
    Centre,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tie {
    None,
    Low,
    High,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fringe {
    None,
    Straight,
    Curtain,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Direction {
    Part,
    Back,
    Forward,
    Up,
}

#[derive(Clone, Debug)]
pub struct Groom {
    pub name: String,
    pub strands: usize,
    pub points: usize,
    pub length_m: f64,
    pub part: Part,
    pub tie: Tie,
    pub fringe: Fringe,
    pub fringe_m: f64,
    pub frame_m: f64,
    pub direction: Direction,
    pub sides_m: f64,
    pub fade_m: f64,
    pub lift: f64,
    pub forehead: f64,
    pub droop: f64,
    pub curl_radius_m: f64,
    pub curl_turns_per_m: f64,
    pub clumps: usize,
    pub clump: f64,
    pub frizz_m: f64,
    pub colour: [f32; 3],
    pub tip_colour: [f32; 3],
    pub highlight_colour: [f32; 3],
    pub highlights: f64,
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
    Field { key: "sides_m", default: 0.0, lo: 0.0, hi: 2.0, integer: false },
    Field { key: "fade_m", default: 0.03, lo: 0.002, hi: 0.2, integer: false },
    Field { key: "lift", default: 0.35, lo: 0.0, hi: 1.0, integer: false },
    Field { key: "forehead", default: 1.0, lo: 0.3, hi: 1.6, integer: false },
    Field { key: "fringe_m", default: 0.1, lo: 0.01, hi: 0.5, integer: false },
    Field { key: "frame_m", default: 0.0, lo: 0.0, hi: 2.0, integer: false },
    Field { key: "highlights", default: 0.0, lo: 0.0, hi: 1.0, integer: false },
    Field { key: "droop", default: 0.5, lo: 0.0, hi: 1.0, integer: false },
    Field { key: "curl_radius_m", default: 0.0, lo: 0.0, hi: 0.1, integer: false },
    Field { key: "curl_turns_per_m", default: 0.0, lo: 0.0, hi: 200.0, integer: false },
    Field { key: "clumps", default: 0.0, lo: 0.0, hi: 5000.0, integer: true },
    Field { key: "clump", default: 0.0, lo: 0.0, hi: 1.0, integer: false },
    Field { key: "frizz_m", default: 0.0, lo: 0.0, hi: 0.05, integer: false },
    Field { key: "seed", default: 1.0, lo: 0.0, hi: u32::MAX as f64, integer: true },
];

pub const PARTS: &[&str] = &["centre", "left", "right", "none"];

pub const TIES: &[&str] = &["none", "low", "high"];
pub const FRINGES: &[&str] = &["none", "straight", "curtain"];
pub const HIGHLIGHT_DEFAULT: &str = "#b8925a";

pub const DIRECTIONS: &[&str] = &["part", "back", "forward", "up"];

pub struct Style {
    pub name: &'static str,
    pub tie: &'static str,
    pub direction: &'static str,
    pub fringe: &'static str,
    pub settings: &'static [(&'static str, f64)],
}

pub const STYLES: &[Style] = &[
    Style { name: "short", tie: "none", direction: "part", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.06), ("droop", 0.3), ("clumps", 200.0), ("clump", 0.3), ("frizz_m", 0.001), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.0), ("fade_m", 0.03), ("lift", 0.35)] },
    Style { name: "bob", tie: "none", direction: "part", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.18), ("droop", 0.6), ("clumps", 300.0), ("clump", 0.4), ("frizz_m", 0.002), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.0), ("fade_m", 0.03), ("lift", 0.35)] },
    Style { name: "long", tie: "none", direction: "part", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.4), ("droop", 0.7), ("clumps", 400.0), ("clump", 0.5), ("frizz_m", 0.002), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.0), ("fade_m", 0.03), ("lift", 0.35)] },
    Style { name: "wavy", tie: "none", direction: "part", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.3), ("droop", 0.6), ("clumps", 400.0), ("clump", 0.5), ("frizz_m", 0.002), ("curl_radius_m", 0.015), ("curl_turns_per_m", 8.0), ("sides_m", 0.0), ("fade_m", 0.03), ("lift", 0.35)] },
    Style { name: "curly", tie: "none", direction: "part", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.25), ("droop", 0.5), ("clumps", 600.0), ("clump", 0.6), ("frizz_m", 0.003), ("curl_radius_m", 0.01), ("curl_turns_per_m", 20.0), ("sides_m", 0.0), ("fade_m", 0.03), ("lift", 0.35)] },
    Style { name: "ponytail", tie: "high", direction: "part", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.45), ("droop", 0.8), ("clumps", 300.0), ("clump", 0.3), ("frizz_m", 0.001), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.0), ("fade_m", 0.03), ("lift", 0.35)] },
    Style { name: "buzz", tie: "none", direction: "back", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.01), ("droop", 0.2), ("clumps", 0.0), ("clump", 0.0), ("frizz_m", 0.0005), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.005), ("fade_m", 0.03), ("lift", 0.2)] },
    Style { name: "crop", tie: "none", direction: "forward", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.05), ("droop", 0.4), ("clumps", 300.0), ("clump", 0.4), ("frizz_m", 0.003), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.008), ("fade_m", 0.02), ("lift", 0.3)] },
    Style { name: "undercut", tie: "none", direction: "back", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.14), ("droop", 0.4), ("clumps", 300.0), ("clump", 0.5), ("frizz_m", 0.001), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.004), ("fade_m", 0.005), ("lift", 0.35)] },
    Style { name: "quiff", tie: "none", direction: "up", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.1), ("droop", 0.15), ("clumps", 300.0), ("clump", 0.5), ("frizz_m", 0.002), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.01), ("fade_m", 0.03), ("lift", 0.9)] },
    Style { name: "slick_back", tie: "none", direction: "back", fringe: "none", settings: &[("frame_m", 0.0), ("length_m", 0.12), ("droop", 0.3), ("clumps", 400.0), ("clump", 0.6), ("frizz_m", 0.0005), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.015), ("fade_m", 0.03), ("lift", 0.25)] },
    Style { name: "bob_bangs", tie: "none", direction: "part", fringe: "straight", settings: &[("frame_m", 0.0), ("fringe_m", 0.1), ("length_m", 0.18), ("droop", 0.6), ("clumps", 300.0), ("clump", 0.4), ("frizz_m", 0.002), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.0), ("fade_m", 0.03), ("lift", 0.35)] },
    Style { name: "curtain_bangs", tie: "none", direction: "part", fringe: "curtain", settings: &[("frame_m", 0.2), ("fringe_m", 0.14), ("length_m", 0.4), ("droop", 0.7), ("clumps", 400.0), ("clump", 0.5), ("frizz_m", 0.002), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.0), ("fade_m", 0.03), ("lift", 0.35)] },
    Style { name: "layered", tie: "none", direction: "part", fringe: "none", settings: &[("frame_m", 0.15), ("length_m", 0.4), ("droop", 0.7), ("clumps", 400.0), ("clump", 0.5), ("frizz_m", 0.002), ("curl_radius_m", 0.0), ("curl_turns_per_m", 0.0), ("sides_m", 0.0), ("fade_m", 0.03), ("lift", 0.35)] },
];

pub fn with_style(groom: &Value, name: &str) -> Option<Value> {
    let style = STYLES.iter().find(|s| s.name == name)?;
    let mut out = groom.as_object()?.clone();
    for (key, value, plain) in [("tie", style.tie, "none"), ("direction", style.direction, "part"), ("fringe", style.fringe, "none")] {
        if value == plain {
            out.remove(key);
        } else {
            out.insert(key.into(), serde_json::json!(value));
        }
    }
    for &(key, v) in style.settings.iter() {
        let integer = FIELDS.iter().any(|f| f.key == key && f.integer);
        out.insert(key.into(), if integer { serde_json::json!(v as u64) } else { serde_json::json!(v) });
    }
    Some(Value::Object(out))
}

pub fn style_of(groom: &Value) -> Option<&'static str> {
    let value = |key: &str| groom.get(key).and_then(Value::as_f64).or_else(|| FIELDS.iter().find(|f| f.key == key).map(|f| f.default));
    let word = |key: &str, plain: &'static str| groom.get(key).and_then(Value::as_str).unwrap_or(plain).to_string();
    let (tie, direction, fringe) = (word("tie", "none"), word("direction", "part"), word("fringe", "none"));
    STYLES
        .iter()
        .find(|s| s.tie == tie && s.direction == direction && s.fringe == fringe && s.settings.iter().all(|&(key, v)| value(key).is_some_and(|x| (x - v).abs() < 1e-9)))
        .map(|s| s.name)
}

pub fn starter() -> Value {
    serde_json::json!({"format": FORMAT, "name": "groom", "strands": 20000, "length_m": 0.15, "part": "centre", "droop": 0.5, "clumps": 300, "clump": 0.4, "frizz_m": 0.002, "colour": "#3b2416", "seed": 1})
}

pub fn parse(v: &Value) -> Result<Groom, String> {
    if v["format"].as_str() != Some(FORMAT) {
        return Err(format!("a groom file says \"format\": \"{FORMAT}\""));
    }
    if let Some(o) = v.as_object()
        && let Some(k) = o.keys().find(|k| !["format", "name", "part", "tie", "fringe", "direction", "colour", "tip_colour", "highlight_colour"].contains(&k.as_str()) && !FIELDS.iter().any(|f| f.key == k.as_str()))
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
    let tie = match v["tie"].as_str().unwrap_or("none") {
        "none" => Tie::None,
        "low" => Tie::Low,
        "high" => Tie::High,
        other => return Err(format!("groom tie is none, low or high, got {other}")),
    };
    let fringe = match v["fringe"].as_str().unwrap_or("none") {
        "none" => Fringe::None,
        "straight" => Fringe::Straight,
        "curtain" => Fringe::Curtain,
        other => return Err(format!("groom fringe is none, straight or curtain, got {other}")),
    };
    let direction = match v["direction"].as_str().unwrap_or("part") {
        "part" => Direction::Part,
        "back" => Direction::Back,
        "forward" => Direction::Forward,
        "up" => Direction::Up,
        other => return Err(format!("groom direction is part, back, forward or up, got {other}")),
    };
    let hex_colour = |key: &str, hex: &str| -> Result<[f32; 3], String> {
        let h = hex.trim_start_matches('#');
        let c = (h.len() == 6).then(|| (0..3).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).ok()).collect::<Option<Vec<u8>>>()).flatten();
        let c = c.ok_or_else(|| format!("groom {key} is #rrggbb, got {hex}"))?;
        Ok([0, 1, 2].map(|i| linear(c[i] as f64 / 255.0)))
    };
    let colour = match v["colour"].as_str() {
        None => [0.18, 0.11, 0.07].map(linear),
        Some(hex) => hex_colour("colour", hex)?,
    };
    let tip_colour = match v["tip_colour"].as_str() {
        None => colour,
        Some(hex) => hex_colour("tip_colour", hex)?,
    };
    let highlight_colour = hex_colour("highlight_colour", v["highlight_colour"].as_str().unwrap_or(HIGHLIGHT_DEFAULT))?;
    Ok(Groom {
        name: v["name"].as_str().unwrap_or("groom").to_string(),
        strands: num("strands")? as usize,
        points: num("points")? as usize,
        length_m: num("length_m")?,
        part,
        tie,
        fringe,
        fringe_m: num("fringe_m")?,
        frame_m: num("frame_m")?,
        direction,
        sides_m: num("sides_m")?,
        fade_m: num("fade_m")?,
        lift: num("lift")?,
        forehead: num("forehead")?,
        droop: num("droop")?,
        curl_radius_m: num("curl_radius_m")?,
        curl_turns_per_m: num("curl_turns_per_m")?,
        clumps: num("clumps")? as usize,
        clump: num("clump")?,
        frizz_m: num("frizz_m")?,
        colour,
        tip_colour,
        highlight_colour,
        highlights: num("highlights")?,
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
        for name in STYLES.iter().map(|s| s.name) {
            let styled = with_style(&starter(), name).unwrap();
            assert!(parse(&styled).is_ok(), "{name}");
            assert_eq!(style_of(&styled), Some(name));
            assert_eq!(styled["colour"], starter()["colour"], "a style keeps the colour, part and seed");
        }
        assert_eq!(style_of(&json!({"format": FORMAT})), None, "the defaults are no style");
        assert!(with_style(&starter(), "mohawk").is_none());
        let back = with_style(&with_style(&starter(), "curtain_bangs").unwrap(), "long").unwrap();
        assert!(back.get("fringe").is_none(), "a style without a fringe takes the fringe away");
        assert_eq!(back["frame_m"], json!(0.0));
        assert_eq!(style_of(&back), Some("long"));
        let mut edited = with_style(&starter(), "bob").unwrap();
        edited["length_m"] = json!(0.2);
        assert_eq!(style_of(&edited), None, "an edited style is custom");
        let s = parse(&starter()).unwrap();
        assert_eq!((s.strands, s.clumps, s.part), (20000, 300, Part::Centre));
    }
}
