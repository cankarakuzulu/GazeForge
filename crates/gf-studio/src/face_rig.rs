use std::collections::BTreeMap;

use serde_json::Value;

pub const SHAPES: [&str; 51] = [
    "browDownLeft", "browDownRight", "browInnerUp", "browOuterUpLeft", "browOuterUpRight", "cheekPuff", "cheekSquintLeft", "cheekSquintRight",
    "eyeBlinkLeft", "eyeBlinkRight", "eyeLookDownLeft", "eyeLookDownRight", "eyeLookInLeft", "eyeLookInRight", "eyeLookOutLeft", "eyeLookOutRight",
    "eyeLookUpLeft", "eyeLookUpRight", "eyeSquintLeft", "eyeSquintRight", "eyeWideLeft", "eyeWideRight", "jawForward", "jawLeft", "jawOpen",
    "jawRight", "mouthClose", "mouthDimpleLeft", "mouthDimpleRight", "mouthFrownLeft", "mouthFrownRight", "mouthFunnel", "mouthLeft",
    "mouthLowerDownLeft", "mouthLowerDownRight", "mouthPressLeft", "mouthPressRight", "mouthPucker", "mouthRight", "mouthRollLower",
    "mouthRollUpper", "mouthShrugLower", "mouthShrugUpper", "mouthSmileLeft", "mouthSmileRight", "mouthStretchLeft", "mouthStretchRight",
    "mouthUpperUpLeft", "mouthUpperUpRight", "noseSneerLeft", "noseSneerRight",
];

pub const GROUPS: [(&str, &[&str]); 5] = [("brow", &["brow"]), ("eye", &["eye"]), ("cheek_nose", &["cheek", "nose"]), ("jaw", &["jaw"]), ("mouth", &["mouth"])];

pub const PRESETS: [(&str, &[(&str, f64)]); 6] = [
    ("neutral", &[]),
    ("smile", &[("mouthSmileLeft", 1.0), ("mouthSmileRight", 1.0), ("cheekSquintLeft", 0.5), ("cheekSquintRight", 0.5), ("eyeSquintLeft", 0.3), ("eyeSquintRight", 0.3)]),
    ("blink", &[("eyeBlinkLeft", 1.0), ("eyeBlinkRight", 1.0)]),
    ("surprise", &[("jawOpen", 0.4), ("browInnerUp", 1.0), ("browOuterUpLeft", 1.0), ("browOuterUpRight", 1.0), ("eyeWideLeft", 1.0), ("eyeWideRight", 1.0)]),
    ("frown", &[("mouthFrownLeft", 1.0), ("mouthFrownRight", 1.0), ("browDownLeft", 0.8), ("browDownRight", 0.8), ("mouthShrugLower", 0.4)]),
    ("jaw_open", &[("jawOpen", 1.0)]),
];

pub const MANUAL: [(&str, f64); 6] = [("rx", 30.0), ("ry", 30.0), ("rz", 30.0), ("tx", 10.0), ("ty", 10.0), ("tz", 10.0)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Blend,
    Joints,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Blend => "blend",
            Mode::Joints => "joints",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    pub position: [f64; 3],
    pub posed: bool,
}

#[derive(Clone, Debug)]
pub struct Rig {
    pub n: usize,
    pub joints: Vec<Joint>,
    pub poses: Vec<Vec<[f64; 6]>>,
    pub fit: Vec<Value>,
    pub rest: Vec<f32>,
    pub ij: Vec<u8>,
    pub iw: Vec<f32>,
    pub offsets: Vec<u32>,
    pub index: Vec<u32>,
    pub delta: Vec<f32>,
    pub ms: f64,
}

fn bytes(v: &Value, what: &str) -> Result<Vec<u8>, String> {
    gf_api::designer::from_base64(v.as_str().ok_or_else(|| format!("rig.face returned no {what}"))?).map_err(|e| format!("{what}: {e}"))
}

fn f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn u32s(b: &[u8]) -> Vec<u32> {
    b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

impl Rig {
    pub fn from_json(r: &Value) -> Result<Rig, String> {
        let n = r["vertices"].as_u64().ok_or("rig.face returned no vertex count")? as usize;
        let joints = r["joints"].as_array().ok_or("rig.face returned no joints")?.iter().map(|j| {
            let p = |k: usize| j["position"][k].as_f64().unwrap_or(0.0);
            Joint { name: j["name"].as_str().unwrap_or_default().to_string(), parent: j["parent"].as_u64().map(|x| x as usize), position: [p(0), p(1), p(2)], posed: j["posed"] == true }
        }).collect::<Vec<_>>();
        let poses = r["poses"].as_array().ok_or("rig.face returned no poses")?.iter().map(|s| {
            s.as_array().into_iter().flatten().map(|q| std::array::from_fn(|c| q[c].as_f64().unwrap_or(0.0))).collect()
        }).collect::<Vec<Vec<[f64; 6]>>>();
        let rest = f32s(&bytes(&r["rest_f32_b64"], "rest positions")?);
        let ij = bytes(&r["influence_joints_u8_b64"], "influence joints")?;
        let iw = f32s(&bytes(&r["influence_weights_f32_b64"], "influence weights")?);
        let offsets = r["blend"]["offsets"].as_array().ok_or("rig.face returned no blend offsets")?.iter().map(|x| x.as_u64().unwrap_or(0) as u32).collect::<Vec<_>>();
        let index = u32s(&bytes(&r["blend"]["index_u32_b64"], "blend index")?);
        let delta = f32s(&bytes(&r["blend"]["delta_f32_b64"], "blend deltas")?);
        if rest.len() != n * 3 || ij.len() != n * 4 || iw.len() != n * 4 || delta.len() != index.len() * 3 || offsets.len() != poses.len() + 1 {
            return Err(format!("rig.face arrays do not agree: {} vertices, rest {}, influences {}/{}, blend {}/{}, offsets {} for {} shapes",
                               n, rest.len(), ij.len(), iw.len(), index.len(), delta.len(), offsets.len(), poses.len()));
        }
        if offsets.last().copied().unwrap_or(0) as usize != index.len() || offsets.windows(2).any(|w| w[0] > w[1]) {
            return Err("rig.face blend offsets do not rise to the end of the blend index".into());
        }
        if joints.iter().any(|j| j.parent.is_some_and(|p| p >= joints.len())) || ij.iter().any(|&j| j as usize >= joints.len().max(1)) || index.iter().any(|&g| g as usize >= n) {
            return Err("rig.face names a joint or a vertex that does not exist".into());
        }
        if poses.iter().any(|s| s.len() != joints.len()) {
            return Err("rig.face poses do not cover every joint".into());
        }
        Ok(Rig { n, joints, poses, fit: r["fit"].as_array().cloned().unwrap_or_default(), rest, ij, iw, offsets, index, delta, ms: r["timing_ms"]["total"].as_f64().unwrap_or(0.0) })
    }

    pub fn joint_poses(&self, weights: &[f64], manual: &BTreeMap<usize, [f64; 6]>) -> Vec<[f64; 6]> {
        let mut out = vec![[0.0f64; 6]; self.joints.len()];
        for (s, &w) in weights.iter().enumerate() {
            if w == 0.0 {
                continue;
            }
            let Some(pose) = self.poses.get(s) else { continue };
            for (j, o) in out.iter_mut().enumerate() {
                for c in 0..6 {
                    o[c] += w * pose.get(j).map_or(0.0, |p| p[c]);
                }
            }
        }
        let d = std::f64::consts::PI / 180.0;
        for (&j, m) in manual {
            if let Some(o) = out.get_mut(j) {
                o[0] += m[0] * d;
                o[1] += m[1] * d;
                o[2] += m[2] * d;
                o[3] += m[3] / 1000.0;
                o[4] += m[4] / 1000.0;
                o[5] += m[5] / 1000.0;
            }
        }
        out
    }

    pub fn evaluate(&self, mode: Mode, weights: &[f64], manual: &BTreeMap<usize, [f64; 6]>) -> Vec<f32> {
        let mut p = self.rest.clone();
        if mode == Mode::Blend {
            for (s, &w) in weights.iter().enumerate() {
                if w == 0.0 || s + 1 >= self.offsets.len() {
                    continue;
                }
                for e in self.offsets[s] as usize..self.offsets[s + 1] as usize {
                    let g = self.index[e] as usize * 3;
                    for k in 0..3 {
                        p[g + k] = (p[g + k] as f64 + w * self.delta[3 * e + k] as f64) as f32;
                    }
                }
            }
            return p;
        }
        let poses = self.joint_poses(weights, manual);
        let mats: Vec<[f64; 9]> = poses.iter().map(|q| rotation([q[0], q[1], q[2]])).collect();
        for v in 0..self.n {
            let (x, y, z) = (self.rest[3 * v] as f64, self.rest[3 * v + 1] as f64, self.rest[3 * v + 2] as f64);
            let (mut ox, mut oy, mut oz) = (0.0f64, 0.0f64, 0.0f64);
            for k in 0..4 {
                let w = self.iw[4 * v + k] as f64;
                if w == 0.0 {
                    continue;
                }
                let j = self.ij[4 * v + k] as usize;
                let (m, c, t) = (&mats[j], self.joints[j].position, poses[j]);
                let (dx, dy, dz) = (x - c[0], y - c[1], z - c[2]);
                ox += w * (m[0] * dx + m[1] * dy + m[2] * dz + c[0] + t[3]);
                oy += w * (m[3] * dx + m[4] * dy + m[5] * dz + c[1] + t[4]);
                oz += w * (m[6] * dx + m[7] * dy + m[8] * dz + c[2] + t[5]);
            }
            p[3 * v] = ox as f32;
            p[3 * v + 1] = oy as f32;
            p[3 * v + 2] = oz as f32;
        }
        p
    }

    pub fn joint_overlay(&self, mode: Mode, weights: &[f64], manual: &BTreeMap<usize, [f64; 6]>) -> (Vec<[f64; 3]>, Vec<[[f64; 3]; 2]>) {
        let poses = (mode == Mode::Joints).then(|| self.joint_poses(weights, manual));
        let at = |j: usize| {
            let c = self.joints[j].position;
            let t = poses.as_ref().map_or([0.0; 6], |p| p[j]);
            [c[0] + t[3], c[1] + t[4], c[2] + t[5]]
        };
        let points = (0..self.joints.len()).map(at).collect();
        let lines = self.joints.iter().enumerate().filter_map(|(j, jt)| jt.parent.map(|p| [at(p), at(j)])).collect();
        (points, lines)
    }
}

pub struct FaceRig {
    pub rig: Option<Rig>,
    pub mode: Mode,
    pub weights: Vec<f64>,
    pub manual: BTreeMap<usize, [f64; 6]>,
    pub joint: usize,
    pub overlay: bool,
    pub wanted: bool,
    pub status: String,
    pub job: Option<std::sync::mpsc::Receiver<Result<Rig, String>>>,
    pub stale: bool,
    pub overlay_off: bool,
}

impl Default for FaceRig {
    fn default() -> FaceRig {
        FaceRig {
            rig: None,
            mode: Mode::Blend,
            weights: vec![0.0; SHAPES.len()],
            manual: BTreeMap::new(),
            joint: 0,
            overlay: true,
            wanted: false,
            status: crate::i18n::tr("rig.not_loaded").to_string(),
            job: None,
            stale: false,
            overlay_off: false,
        }
    }
}

impl FaceRig {
    pub fn active(&self) -> bool {
        self.weights.iter().any(|w| *w != 0.0) || (self.mode == Mode::Joints && !self.manual.is_empty())
    }

    pub fn pose(&self) -> Option<Vec<f32>> {
        let rig = self.rig.as_ref()?;
        self.active().then(|| rig.evaluate(self.mode, &self.weights, &self.manual))
    }

    pub fn loaded_text(&self) -> String {
        let Some(r) = &self.rig else { return self.status.clone() };
        let poor = r.fit.iter().filter(|f| f["poorly_expressed"] == true).count();
        crate::i18n::trf("rig.loaded", &[("joints", &r.joints.len()), ("shapes", &SHAPES.len()), ("poor", &poor), ("ms", &r.ms.round())])
    }
}

pub fn rotation(r: [f64; 3]) -> [f64; 9] {
    let t = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
    if t < 1e-12 {
        return [1.0, -r[2], r[1], r[2], 1.0, -r[0], -r[1], r[0], 1.0];
    }
    let k = [r[0] / t, r[1] / t, r[2] / t];
    let (s, c) = t.sin_cos();
    let v = 1.0 - c;
    [
        c + k[0] * k[0] * v, k[0] * k[1] * v - k[2] * s, k[0] * k[2] * v + k[1] * s,
        k[1] * k[0] * v + k[2] * s, c + k[1] * k[1] * v, k[1] * k[2] * v - k[0] * s,
        k[2] * k[0] * v - k[1] * s, k[2] * k[1] * v + k[0] * s, c + k[2] * k[2] * v,
    ]
}

pub fn preset_weights(name: &str) -> Option<Vec<f64>> {
    let (_, values) = PRESETS.iter().find(|(n, _)| *n == name)?;
    Some(SHAPES.iter().map(|s| values.iter().find(|(n, _)| n == s).map_or(0.0, |v| v.1)).collect())
}

pub fn group_of(shape: &str) -> &'static str {
    GROUPS.iter().find(|(_, prefixes)| prefixes.iter().any(|p| shape.starts_with(p))).map_or("mouth", |g| g.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn b64(bytes: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        bytes.chunks(3).flat_map(|c| {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            (0..4).map(move |i| if i <= c.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' })
        }).collect()
    }

    fn tiny() -> Rig {
        let rest: Vec<u8> = [0.0f32, 0.0, 0.0, 0.1, 0.0, 0.0].iter().flat_map(|x| x.to_le_bytes()).collect();
        let ij = [0u8, 0, 0, 0, 1, 0, 0, 0];
        let iw: Vec<u8> = [1.0f32, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0].iter().flat_map(|x| x.to_le_bytes()).collect();
        let index: Vec<u8> = 1u32.to_le_bytes().to_vec();
        let delta: Vec<u8> = [0.0f32, 0.01, 0.0].iter().flat_map(|x| x.to_le_bytes()).collect();
        Rig::from_json(&json!({"vertices": 2, "joints": [{"name": "root", "parent": null, "position": [0, 0, 0], "posed": false}, {"name": "tip", "parent": 0, "position": [0.1, 0, 0], "posed": true}],
            "poses": [[[0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0.002]]], "fit": [], "rest_f32_b64": b64(&rest), "influence_joints_u8_b64": b64(&ij), "influence_weights_f32_b64": b64(&iw),
            "blend": {"offsets": [0, 1], "index_u32_b64": b64(&index), "delta_f32_b64": b64(&delta)}})).unwrap()
    }

    #[test]
    fn blend_adds_the_weighted_delta_and_joints_move_by_the_pose() {
        let r = tiny();
        let none = BTreeMap::new();
        assert_eq!(r.evaluate(Mode::Blend, &[0.5], &none), vec![0.0, 0.0, 0.0, 0.1, 0.005, 0.0]);
        let p = r.evaluate(Mode::Joints, &[1.0], &none);
        assert!((p[5] - 0.002).abs() < 1e-7 && p[0] == 0.0);
        let mut m = BTreeMap::new();
        m.insert(1, [0.0, 0.0, 90.0, 0.0, 0.0, 0.0]);
        let q = r.evaluate(Mode::Joints, &[0.0], &m);
        assert!((q[3] - 0.1).abs() < 1e-7 && q[4].abs() < 1e-7, "rotating about the tip joint keeps the tip");
        assert_eq!(r.joint_overlay(Mode::Blend, &[1.0], &none).1.len(), 1);
    }

    #[test]
    fn a_reply_naming_missing_joints_or_vertices_is_refused() {
        let good = tiny();
        let mut cases = Vec::new();
        let mut r = json!({"vertices": 2, "joints": [{"name": "root", "parent": null, "position": [0, 0, 0]}, {"name": "tip", "parent": 7, "position": [0.1, 0, 0]}],
                           "poses": [[[0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0]]], "rest_f32_b64": b64(&good.rest.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>()),
                           "influence_joints_u8_b64": b64(&good.ij), "influence_weights_f32_b64": b64(&good.iw.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<_>>()),
                           "blend": {"offsets": [0, 1], "index_u32_b64": b64(&1u32.to_le_bytes()), "delta_f32_b64": b64(&[0u8; 12])}});
        cases.push(r.clone());
        r["joints"][1]["parent"] = json!(0);
        r["influence_joints_u8_b64"] = json!(b64(&[5, 0, 0, 0, 1, 0, 0, 0]));
        cases.push(r.clone());
        r["influence_joints_u8_b64"] = json!(b64(&good.ij));
        r["blend"]["index_u32_b64"] = json!(b64(&9u32.to_le_bytes()));
        cases.push(r.clone());
        r["blend"]["index_u32_b64"] = json!(b64(&1u32.to_le_bytes()));
        r["poses"] = json!([[[0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0]], [[0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0]]]);
        r["blend"]["offsets"] = json!([0, 5, 1]);
        cases.push(r.clone());
        for (i, c) in cases.iter().enumerate() {
            assert!(Rig::from_json(c).is_err(), "case {i} was accepted");
        }
        r["poses"] = json!([[[0, 0, 0, 0, 0, 0], [0, 0, 0, 0, 0, 0]]]);
        r["blend"]["offsets"] = json!([0, 1]);
        assert!(Rig::from_json(&r).is_ok());
    }

    #[test]
    fn a_short_array_is_refused_and_presets_follow_the_shape_order() {
        let mut v = json!({"vertices": 3});
        assert!(Rig::from_json(&v).is_err());
        v["joints"] = json!([]);
        assert!(Rig::from_json(&v).is_err());
        let smile = preset_weights("smile").unwrap();
        assert_eq!(smile.len(), SHAPES.len());
        assert_eq!(smile[SHAPES.iter().position(|s| *s == "mouthSmileLeft").unwrap()], 1.0);
        assert_eq!(group_of("cheekPuff"), "cheek_nose");
        assert_eq!(rotation([0.0, 0.0, 0.0]), [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    }
}
