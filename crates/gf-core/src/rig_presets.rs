use std::io::Read;

use serde_json::Value;

macro_rules! preset {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!("../makehuman/rigs/rig.", $name, ".json.gz")).as_slice(),
            include_bytes!(concat!("../makehuman/rigs/weights.", $name, ".json.gz")).as_slice(),
        )
    };
}

pub const PRESETS: &[(&str, &[u8], &[u8])] = &[
    preset!("game_engine"),
    preset!("game_engine_with_breast"),
    preset!("default"),
    preset!("mixamo"),
    preset!("mixamo_unity"),
    preset!("cmu_mb"),
    preset!("openpose"),
];

pub fn names() -> Vec<&'static str> {
    PRESETS.iter().map(|p| p.0).collect()
}

fn unpack(name: &str, what: &str, bytes: &[u8]) -> Result<Value, String> {
    let mut text = String::new();
    flate2::read::GzDecoder::new(bytes).read_to_string(&mut text).map_err(|e| format!("rig preset {name} {what}: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("rig preset {name} {what}: {e}"))
}

pub fn preset(name: &str) -> Result<(Value, Value), String> {
    let (_, rig, weights) = PRESETS.iter().find(|p| p.0 == name).ok_or_else(|| format!("{name} is not a rig preset; the presets are {}", names().join(", ")))?;
    Ok((unpack(name, "rig", rig)?, unpack(name, "weights", weights)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body_rig::{RigDef, rig_weights};

    #[test]
    fn every_preset_parses_and_its_weights_name_only_its_bones() {
        for name in names() {
            let (rig, weights) = preset(name).unwrap();
            let r = RigDef::parse(&rig).unwrap();
            let (w, unknown) = rig_weights(&weights, &r).unwrap();
            assert!(r.bones.len() >= 20, "{name}: {} bones", r.bones.len());
            assert!(w.len() > 10_000, "{name}: {} weighted vertices", w.len());
            assert!(unknown.is_empty(), "{name}: weights for bones not in the rig: {unknown:?}");
        }
        assert!(preset("nope").unwrap_err().contains("game_engine"));
    }
}
