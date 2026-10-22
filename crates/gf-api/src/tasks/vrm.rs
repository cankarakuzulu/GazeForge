use std::collections::HashMap;

use gf_io::gltf;
use serde_json::{Value, json};

pub const BONES: &[&str] = &[
    "hips", "spine", "chest", "upperChest", "neck", "head", "leftEye", "rightEye", "jaw", "leftUpperLeg", "leftLowerLeg", "leftFoot", "leftToes",
    "rightUpperLeg", "rightLowerLeg", "rightFoot", "rightToes", "leftShoulder", "leftUpperArm", "leftLowerArm", "leftHand", "rightShoulder",
    "rightUpperArm", "rightLowerArm", "rightHand", "leftThumbMetacarpal", "leftThumbProximal", "leftThumbDistal", "leftIndexProximal",
    "leftIndexIntermediate", "leftIndexDistal", "leftMiddleProximal", "leftMiddleIntermediate", "leftMiddleDistal", "leftRingProximal",
    "leftRingIntermediate", "leftRingDistal", "leftLittleProximal", "leftLittleIntermediate", "leftLittleDistal", "rightThumbMetacarpal",
    "rightThumbProximal", "rightThumbDistal", "rightIndexProximal", "rightIndexIntermediate", "rightIndexDistal", "rightMiddleProximal",
    "rightMiddleIntermediate", "rightMiddleDistal", "rightRingProximal", "rightRingIntermediate", "rightRingDistal", "rightLittleProximal",
    "rightLittleIntermediate", "rightLittleDistal",
];

pub const REQUIRED: &[&str] = &[
    "hips", "spine", "head", "leftUpperLeg", "leftLowerLeg", "leftFoot", "rightUpperLeg", "rightLowerLeg", "rightFoot", "leftUpperArm", "leftLowerArm",
    "leftHand", "rightUpperArm", "rightLowerArm", "rightHand",
];

pub const LICENSE: &str = "https://vrm.dev/licenses/1.0/";

pub const EXPRESSIONS: &[(&str, &[&str])] = &[
    ("blink", &["eyeBlinkLeft", "eyeBlinkRight"]),
    ("blinkLeft", &["eyeBlinkLeft"]),
    ("blinkRight", &["eyeBlinkRight"]),
    ("aa", &["jawOpen"]),
    ("happy", &["mouthSmileLeft", "mouthSmileRight"]),
    ("sad", &["mouthFrownLeft", "mouthFrownRight", "browInnerUp"]),
    ("angry", &["browDownLeft", "browDownRight", "mouthPressLeft", "mouthPressRight"]),
    ("surprised", &["browInnerUp", "eyeWideLeft", "eyeWideRight", "jawOpen"]),
];

pub fn vrm_bone(godot: &str) -> Option<String> {
    let mut c = godot.chars();
    let first = c.next()?;
    let name: String = first.to_lowercase().chain(c).collect();
    BONES.contains(&name.as_str()).then_some(name)
}

type M3 = [[f64; 3]; 3];
const IDENTITY: M3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn mul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

fn apply(m: &M3, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| (0..3).map(|k| m[i][k] * v[k]).sum())
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    v.map(|x| x / l)
}

pub fn from_to(a: [f64; 3], b: [f64; 3]) -> M3 {
    let (a, b) = (unit(a), unit(b));
    let c = a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let v = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    if c < -1.0 + 1e-9 {
        let axis = unit(if a[0].abs() < 0.9 { [0.0, -a[2], a[1]] } else { [-a[2], 0.0, a[0]] });
        return std::array::from_fn(|i| std::array::from_fn(|j| 2.0 * axis[i] * axis[j] - if i == j { 1.0 } else { 0.0 }));
    }
    let k: M3 = [[0.0, -v[2], v[1]], [v[2], 0.0, -v[0]], [-v[1], v[0], 0.0]];
    let k2 = mul(&k, &k);
    let f = 1.0 / (1.0 + c);
    std::array::from_fn(|i| std::array::from_fn(|j| IDENTITY[i][j] + k[i][j] + k2[i][j] * f))
}

pub struct Turn {
    pub joint: usize,
    pub toward: usize,
    pub target: [f64; 3],
}

pub fn tpose(rig: &mut gltf::Rig, turns: &[Turn], meshes: &mut [&mut gltf::Lod], points: &mut [(usize, [f64; 3])]) -> Value {
    let n = rig.names.len();
    let rest: Vec<[f64; 3]> = rig.global.iter().map(|g| g.map(|x| x as f64)).collect();
    let mut depth = vec![0usize; n];
    for (j, d) in depth.iter_mut().enumerate() {
        let mut p = rig.parents[j];
        while let Some(q) = p {
            *d += 1;
            p = rig.parents[q];
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&j| depth[j]);
    let by_joint: HashMap<usize, &Turn> = turns.iter().map(|t| (t.joint, t)).collect();
    let mut affine: Vec<(M3, [f64; 3])> = vec![(IDENTITY, [0.0; 3]); n];
    let mut turned = Vec::new();
    for &j in &order {
        let (r, t) = rig.parents[j].map_or((IDENTITY, [0.0; 3]), |p| affine[p]);
        let place = |x: [f64; 3]| {
            let y = apply(&r, x);
            [y[0] + t[0], y[1] + t[1], y[2] + t[2]]
        };
        affine[j] = (r, t);
        if let Some(turn) = by_joint.get(&j) {
            let (pj, pc) = (place(rest[j]), place(rest[turn.toward]));
            let d = [pc[0] - pj[0], pc[1] - pj[1], pc[2] - pj[2]];
            let q = from_to(d, turn.target);
            let qt = apply(&q, [t[0] - pj[0], t[1] - pj[1], t[2] - pj[2]]);
            affine[j] = (mul(&q, &r), [qt[0] + pj[0], qt[1] + pj[1], qt[2] + pj[2]]);
            let cos = unit(d).iter().zip(unit(turn.target)).map(|(a, b)| a * b).sum::<f64>().clamp(-1.0, 1.0);
            turned.push(json!({"joint": rig.names[j], "deg": (cos.acos().to_degrees() * 10.0).round() / 10.0}));
        }
    }
    let moved = |a: &(M3, [f64; 3]), x: [f64; 3]| {
        let y = apply(&a.0, x);
        [y[0] + a.1[0], y[1] + a.1[1], y[2] + a.1[2]]
    };
    for j in 0..n {
        rig.global[j] = moved(&affine[j], rest[j]).map(|x| x as f32);
    }
    for (j, p) in points.iter_mut() {
        if let Some(a) = affine.get(*j) {
            *p = moved(a, *p);
        }
    }
    let mut vertices = 0usize;
    for lod in meshes.iter_mut() {
        let Some(skin) = &lod.skin else { continue };
        for v in 0..lod.positions.len() {
            let mut influences: Vec<(usize, f64)> = skin.joints[v].iter().zip(skin.weights[v]).map(|(&j, w)| (j as usize, w as f64)).collect();
            if let (Some(j1), Some(w1)) = (skin.joints_1.get(v), skin.weights_1.get(v)) {
                influences.extend(j1.iter().zip(w1).map(|(&j, &w)| (j as usize, w as f64)));
            }
            let p = lod.positions[v].map(|x| x as f64);
            let nm = lod.normals[v].map(|x| x as f64);
            let (mut np, mut nn) = ([0.0; 3], [0.0; 3]);
            for (j, w) in influences.into_iter().filter(|e| e.1 > 0.0) {
                let a = &affine[j];
                let x = moved(a, p);
                let y = apply(&a.0, nm);
                for k in 0..3 {
                    np[k] += w * x[k];
                    nn[k] += w * y[k];
                }
            }
            lod.positions[v] = np.map(|x| x as f32);
            lod.normals[v] = unit(nn).map(|x| x as f32);
            vertices += 1;
        }
    }
    json!({"turned": turned, "vertices_reposed": vertices})
}

pub fn spring_bone(doc: &Value) -> Result<Option<(Value, Value)>, String> {
    let physics = &doc["extensions"]["GAZEFORGE_physics"];
    let springs = physics["springs"].as_array().cloned().unwrap_or_default();
    if springs.is_empty() {
        return Ok(None);
    }
    let nodes = doc["nodes"].as_array().ok_or("the glb has no nodes")?;
    let joints: Vec<usize> = doc["skins"][0]["joints"].as_array().ok_or("the glb has no skin")?.iter().filter_map(Value::as_u64).map(|j| j as usize).collect();
    let mut parent = vec![None; nodes.len()];
    for (i, n) in nodes.iter().enumerate() {
        for c in n["children"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
            parent[c as usize] = Some(i);
        }
    }
    let in_skin = |n: usize| joints.contains(&n);
    let place = |node: usize| -> Result<[f64; 3], String> {
        let mut p = [0.0; 3];
        let mut at = Some(node);
        while let Some(n) = at.filter(|&n| in_skin(n)) {
            if nodes[n].get("rotation").is_some_and(|r| r != &json!([0.0, 0.0, 0.0, 1.0])) || nodes[n].get("scale").is_some() || nodes[n].get("matrix").is_some() {
                return Err(format!("joint node {} is rotated or scaled; VRMC_springBone colliders are written for translation-only joints", nodes[n]["name"]));
            }
            let t = nodes[n]["translation"].as_array().map(|a| [0, 1, 2].map(|k| a[k].as_f64().unwrap_or(0.0))).unwrap_or([0.0; 3]);
            p = [p[0] + t[0], p[1] + t[1], p[2] + t[2]];
            at = parent[n];
        }
        Ok(p)
    };
    let node_of = |j: u64| joints.get(j as usize).copied().ok_or(format!("GAZEFORGE_physics names joint {j}, the skin has {}", joints.len()));
    let v3 = |v: &Value| -> [f64; 3] { v.as_array().map(|a| [0, 1, 2].map(|k| a.get(k).and_then(Value::as_f64).unwrap_or(0.0))).unwrap_or([0.0; 3]) };
    let mut colliders = Vec::new();
    let mut base = Vec::new();
    for c in physics["colliders"].as_array().into_iter().flatten() {
        let node = node_of(c["joint"].as_u64().ok_or("a collider has no joint")?)?;
        let at = place(node)?;
        let (h, t) = (v3(&c["head"]), v3(&c["tail"]));
        let local = |p: [f64; 3]| [p[0] - at[0], p[1] - at[1], p[2] - at[2]].map(|x| x as f32);
        base.push((node, local(h), local(t)));
        colliders.push(json!({"node": node, "shape": {"capsule": {"offset": local(h), "radius": c["radius"], "tail": local(t)}}}));
    }
    let d = &physics["spring_defaults"];
    let (mut groups, mut out_springs) = (Vec::new(), Vec::new());
    for (k, s) in springs.iter().enumerate() {
        let mut members: Vec<usize> = (0..base.len()).collect();
        for r in s["collider_radii"].as_array().into_iter().flatten() {
            let c = r["collider"].as_u64().ok_or("a collider_radii entry has no collider")? as usize;
            let (node, h, t) = *base.get(c).ok_or("a collider_radii entry names a collider the file does not have")?;
            colliders.push(json!({"node": node, "shape": {"capsule": {"offset": h, "radius": r["radius"], "tail": t}}}));
            members[c] = colliders.len() - 1;
        }
        groups.push(json!({"name": format!("{}_chain{}", s["wearable"].as_str().unwrap_or("spring"), s["chain"]), "colliders": members}));
        let hit = s["hit_radius_m"].as_f64().or(d["hit_radius_m"].as_f64()).unwrap_or(0.02);
        let chain: Vec<Value> = s["joints"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|j| -> Result<Value, String> {
                Ok(json!({"node": node_of(j.as_u64().ok_or("a spring joint is not an index")?)?, "hitRadius": hit, "stiffness": d["stiffness"].as_f64().unwrap_or(1.0),
                          "gravityPower": d["gravity"].as_f64().unwrap_or(0.1), "gravityDir": [0.0, -1.0, 0.0], "dragForce": d["drag"].as_f64().unwrap_or(0.4)}))
            })
            .collect::<Result<_, _>>()?;
        out_springs.push(json!({"name": format!("{}_chain{}", s["wearable"].as_str().unwrap_or("spring"), s["chain"]), "joints": chain, "colliderGroups": [k]}));
    }
    let cloth = physics["cloth"].as_array().map_or(0, Vec::len);
    let report = json!({"springs": out_springs.len(), "colliders": colliders.len(), "shrunk_copies": colliders.len() - base.len(),
                        "cloth_left_out": cloth,
                        "method": "GAZEFORGE_physics as VRMC_springBone 1.0: one capsule collider per body capsule on its bone (offset and tail in the bone's space), one more per shrunk per-chain radius, one collider group and one spring per chain with the file's spring defaults and the chain's hit radius; GAZEFORGE_physics itself is left out of the .vrm"});
    Ok(Some((json!({"specVersion": "1.0", "colliders": colliders, "colliderGroups": groups, "springs": out_springs}), report)))
}

pub fn extension(doc: &Value, bones: &[(String, String)], meta: Value, head_node: &str, morphs: &[String]) -> Result<(Value, Value), String> {
    let nodes = doc["nodes"].as_array().ok_or("the glb has no nodes")?;
    let node_of = |name: &str| nodes.iter().position(|n| n["name"] == name);
    let mut human = serde_json::Map::new();
    for (vrm, bone) in bones {
        if let Some(i) = node_of(bone) {
            human.insert(vrm.clone(), json!({"node": i}));
        }
    }
    let missing: Vec<&&str> = REQUIRED.iter().filter(|b| !human.contains_key(**b)).collect();
    if !missing.is_empty() {
        return Err(format!("VRM needs the humanoid bones {missing:?}, which this rig's map does not give"));
    }
    let mut presets = serde_json::Map::new();
    let mut made = Vec::new();
    if let Some(head) = node_of(head_node) {
        for (preset, parts) in EXPRESSIONS {
            let binds: Vec<Value> = parts
                .iter()
                .filter_map(|m| morphs.iter().position(|x| x.rsplit('/').next() == Some(*m)))
                .map(|index| json!({"node": head, "index": index, "weight": 1.0}))
                .collect();
            if binds.len() == parts.len() {
                presets.insert(preset.to_string(), json!({"morphTargetBinds": binds, "isBinary": false}));
                made.push(*preset);
            }
        }
    }
    let mut ext = json!({"specVersion": "1.0", "meta": meta, "humanoid": {"humanBones": human}});
    if !presets.is_empty() {
        ext["expressions"] = json!({"preset": presets});
    }
    let report = json!({"human_bones": ext["humanoid"]["humanBones"].as_object().map_or(0, |m| m.len()), "expressions": made});
    Ok((ext, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physics_becomes_spring_bone_with_per_chain_collider_copies() {
        let doc = json!({
            "nodes": [
                {"name": "Character", "children": [1]},
                {"name": "hips", "translation": [0.0, 1.0, 0.0], "children": [2]},
                {"name": "head", "translation": [0.0, 0.6, 0.0], "children": [3]},
                {"name": "hair_chain0_0", "translation": [0.0, 0.0, -0.1], "children": [4]},
                {"name": "hair_chain0_1", "translation": [0.0, -0.1, 0.0]}
            ],
            "skins": [{"joints": [1, 2, 3, 4]}],
            "extensions": {"GAZEFORGE_physics": {
                "colliders": [{"joint": 1, "bone": "head", "head": [0.0, 1.6, 0.0], "tail": [0.0, 1.8, 0.0], "radius": 0.08},
                              {"joint": 0, "bone": "hips", "head": [0.0, 1.0, 0.0], "tail": [0.0, 1.2, 0.0], "radius": 0.1}],
                "springs": [{"wearable": "hair", "chain": 0, "joints": [2, 3], "hit_radius_m": 0.015, "collider_radii": [{"collider": 0, "radius": 0.05}]}],
                "spring_defaults": {"stiffness": 1.0, "drag": 0.4, "gravity": 0.1, "hit_radius_m": 0.02},
                "cloth": []
            }}
        });
        let (ext, report) = spring_bone(&doc).unwrap().unwrap();
        let caps = ext["colliders"].as_array().unwrap();
        assert_eq!(caps.len(), 3, "two body capsules and one shrunk copy");
        assert_eq!(caps[0]["node"], 2);
        assert_eq!(caps[0]["shape"]["capsule"]["offset"], json!([0.0, 0.0, 0.0]), "in the head bone's space");
        let tail = caps[0]["shape"]["capsule"]["tail"].as_array().unwrap();
        assert!((tail[1].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert_eq!(caps[2]["node"], 2);
        assert!((caps[2]["shape"]["capsule"]["radius"].as_f64().unwrap() - 0.05).abs() < 1e-6);
        assert_eq!(ext["colliderGroups"][0]["colliders"], json!([2, 1]), "the chain sees its own copy instead of the full head capsule");
        let joints = ext["springs"][0]["joints"].as_array().unwrap();
        assert_eq!(joints.iter().map(|j| j["node"].as_u64().unwrap()).collect::<Vec<_>>(), vec![3, 4]);
        assert!((joints[0]["hitRadius"].as_f64().unwrap() - 0.015).abs() < 1e-9);
        assert_eq!(joints[0]["gravityPower"], 0.1);
        assert_eq!(report["shrunk_copies"], 1);
        let mut rotated = doc.clone();
        rotated["nodes"][2]["rotation"] = json!([0.0, 0.7071, 0.0, 0.7071]);
        assert!(spring_bone(&rotated).is_err(), "a rotated joint is refused, not written wrong");
    }

    #[test]
    fn godot_profile_names_map_to_vrm_bone_names() {
        assert_eq!(vrm_bone("LeftUpperArm").as_deref(), Some("leftUpperArm"));
        assert_eq!(vrm_bone("UpperChest").as_deref(), Some("upperChest"));
        assert_eq!(vrm_bone("LeftThumbMetacarpal").as_deref(), Some("leftThumbMetacarpal"));
        assert_eq!(vrm_bone("Root"), None);
        assert!(REQUIRED.iter().all(|r| BONES.contains(r)));
    }

    #[test]
    fn a_hanging_arm_turns_out_level_and_its_skin_follows() {
        let mut rig = gltf::Rig {
            names: ["root", "upper", "lower", "hand"].map(String::from).to_vec(),
            parents: vec![None, Some(0), Some(1), Some(2)],
            global: vec![[0.0, 1.0, 0.0], [0.2, 1.4, 0.0], [0.4, 1.2, 0.1], [0.5, 1.0, 0.1]],
            attachments: Vec::new(),
            animations: Vec::new(),
        };
        let mut lod = gltf::Lod {
            name: "Body_LOD0".into(),
            positions: vec![[0.45, 1.1, 0.1], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0], [0.0, 0.0, 1.0]],
            uvs: vec![[0.0; 2]; 2],
            indices: vec![],
            morphs: vec![],
            skin: Some(gltf::SkinData { joints: vec![[2, 0, 0, 0], [0, 0, 0, 0]], weights: vec![[1.0, 0.0, 0.0, 0.0]; 2], joints_1: vec![], weights_1: vec![] }),
            material: None,
            custom: vec![],
        };
        let turns = [Turn { joint: 1, toward: 2, target: [1.0, 0.0, 0.0] }, Turn { joint: 2, toward: 3, target: [1.0, 0.0, 0.0] }];
        let report = tpose(&mut rig, &turns, &mut [&mut lod], &mut []);
        let (u, l, h) = (rig.global[1], rig.global[2], rig.global[3]);
        let near = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(near(l[1], u[1]) && near(l[2], u[2]) && near(h[1], u[1]) && near(h[2], u[2]), "the chain lies along +X: {:?} {report}", rig.global);
        assert!(near(l[0] - u[0], (0.2f32 * 0.2 + 0.2 * 0.2 + 0.1 * 0.1).sqrt()), "bone lengths keep");
        assert_eq!(rig.global[0], [0.0, 1.0, 0.0], "the root stays");
        let v = lod.positions[0];
        let half = (0.1f32 * 0.1 + 0.2 * 0.2).sqrt() / 2.0;
        assert!(near(v[0], l[0] + half) && near(v[1], l[1]) && near(v[2], l[2]), "the vertex halfway along the forearm stays halfway along it: {v:?} {l:?}");
        assert_eq!(lod.positions[1], [0.0, 1.0, 0.0], "a vertex on the root stays");
    }
}
