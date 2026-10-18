use std::collections::BTreeMap;

pub const BASE_VERTICES: usize = 1_151_889;

pub struct Landmark {
    pub name: &'static str,
    pub vertex: usize,
    pub rule: &'static str,
}

pub const LANDMARKS: &[Landmark] = &[
    Landmark { name: "alare_l", vertex: 930_597, rule: "lateral-most vertex (+x) among those nose/nose-flaring-incr moves >= 50% of its maximum" },
    Landmark { name: "alare_r", vertex: 426_541, rule: "lateral-most vertex (-x) among those nose/nose-flaring-incr moves >= 50% of its maximum" },
    Landmark { name: "pronasale", vertex: 185_894, rule: "the nose tip vertex" },
    Landmark { name: "nasion", vertex: 72_776, rule: "most posterior vertex of the midline profile (|x| < 0.6 mm) 15 to 45 mm above the tip" },
    Landmark { name: "subnasale", vertex: 124_210, rule: "most posterior vertex of the midline profile 5 to 25 mm below the tip" },
    Landmark { name: "endocanthion_l", vertex: 823_322, rule: "vertex nearest the centroid of the full-weight plateau of eyes/l-eye-corner2-up (inner eye corner)" },
    Landmark { name: "endocanthion_r", vertex: 318_859, rule: "vertex nearest the centroid of the full-weight plateau of eyes/r-eye-corner2-up (inner eye corner)" },
    Landmark { name: "exocanthion_l", vertex: 934_965, rule: "vertex nearest the centroid of the full-weight plateau of eyes/l-eye-corner1-up (outer eye corner)" },
    Landmark { name: "exocanthion_r", vertex: 431_092, rule: "vertex nearest the centroid of the full-weight plateau of eyes/r-eye-corner1-up (outer eye corner)" },
    Landmark { name: "zygion_l", vertex: 674_407, rule: "lateral-most vertex (+x) 0 to 30 mm above the tip, outside the region ears/l-ear-scale-incr moves" },
    Landmark { name: "zygion_r", vertex: 168_396, rule: "lateral-most vertex (-x) 0 to 30 mm above the tip, outside the region ears/r-ear-scale-incr moves" },
];

pub struct Measure {
    pub name: &'static str,
    pub from: &'static str,
    pub to: &'static str,
    pub what: &'static str,
}

pub const MEASURES: &[Measure] = &[
    Measure { name: "nose_width", from: "alare_l", to: "alare_r", what: "nose width at the alae (alar width)" },
    Measure { name: "nose_length", from: "nasion", to: "pronasale", what: "nose length, nasion to tip" },
    Measure { name: "nose_height", from: "nasion", to: "subnasale", what: "nose height, nasion to subnasale" },
    Measure { name: "face_width", from: "zygion_l", to: "zygion_r", what: "face width at the cheekbones" },
    Measure { name: "eye_spacing", from: "endocanthion_l", to: "endocanthion_r", what: "eye spacing, inner corner to inner corner" },
    Measure { name: "eye_spacing_outer", from: "exocanthion_l", to: "exocanthion_r", what: "eye spacing, outer corner to outer corner" },
];

pub fn landmark(name: &str) -> Option<&'static Landmark> {
    LANDMARKS.iter().find(|l| l.name == name)
}

pub fn default_pairs() -> BTreeMap<String, [usize; 2]> {
    MEASURES
        .iter()
        .map(|m| (m.name.to_string(), [landmark(m.from).expect("listed").vertex, landmark(m.to).expect("listed").vertex]))
        .collect()
}

pub fn distance_mm(positions: &[[f64; 3]], pair: [usize; 2]) -> Result<f64, String> {
    let n = positions.len();
    let (a, b) = match (positions.get(pair[0]), positions.get(pair[1])) {
        (Some(a), Some(b)) => (a, b),
        _ => return Err(format!("vertex pair {pair:?} is outside the {n} positions")),
    };
    let d = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt() * 1000.0;
    if !d.is_finite() {
        return Err(format!("vertex pair {pair:?} has a non-finite distance"));
    }
    Ok(d)
}

pub fn round_mm(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_measure_names_two_landmarks_and_pairs_are_distinct() {
        let pairs = default_pairs();
        assert_eq!(pairs.len(), MEASURES.len());
        assert!(pairs.values().all(|p| p[0] != p[1] && p.iter().all(|&v| v < BASE_VERTICES)));
    }

    #[test]
    fn distances_are_in_millimetres_and_bad_pairs_are_refused() {
        let p = vec![[0.0, 0.0, 0.0], [0.003, 0.004, 0.0]];
        assert!((distance_mm(&p, [0, 1]).unwrap() - 5.0).abs() < 1e-12);
        assert!(distance_mm(&p, [0, 2]).is_err());
        assert!(distance_mm(&[[f64::NAN, 0.0, 0.0], [0.0; 3]], [0, 1]).is_err());
    }
}
