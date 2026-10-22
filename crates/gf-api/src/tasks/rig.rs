use std::collections::HashMap;
use std::path::Path;

use gf_core::face_rig::{self as fr, JointPose, Mat, ShapeError};
use gf_core::game_mesh::{self as gm, Point};
use gf_geom::lids;
use gf_io::gltf;
use serde_json::{Value, json};

use super::mesh::{from_gltf, lid_settings, verdict};
use super::{Options, need, only};

fn frame(p: Point) -> Point {
    [p[0], p[2], -p[1]]
}

pub fn face_animations(names: &[String], fits: &[(Vec<JointPose>, ShapeError)], pivots: &[Point]) -> Vec<gltf::Animation> {
    let parents = fr::parents();
    let pg: Vec<Point> = pivots.iter().map(|p| frame(*p)).collect();
    names
        .iter()
        .zip(fits)
        .map(|(name, fit)| {
            let poses: Vec<JointPose> = fit.0.iter().map(|p| JointPose { rotation: frame(p.rotation), translation: frame(p.translation) }).collect();
            let locals = fr::local_pose(&parents, &pg, &poses);
            let channels = (0..fr::count())
                .filter(|&j| fr::is_free(j))
                .map(|j| {
                    let parent = parents[j].map(|p| pg[p]).unwrap_or([0.0; 3]);
                    let rest = [0, 1, 2].map(|k| (pg[j][k] - parent[k]) as f32);
                    let (q, t) = locals[j];
                    gltf::Channel { joint: j, rotation: [[0.0, 0.0, 0.0, 1.0], q.map(|x| x as f32)], translation: [rest, t.map(|x| x as f32)] }
                })
                .collect();
            gltf::Animation { name: name.clone(), channels, morph: None }
        })
        .collect()
}

pub struct OnBody {
    pub neck: usize,
    pub head: usize,
    pub jaw: Option<usize>,
    pub eyes: [Option<usize>; 2],
}

pub fn merge_head_rig(body: &mut gltf::Rig, head: gltf::Rig, on: &OnBody) -> (Vec<usize>, Value) {
    let rest_local = |global: &[[f32; 3]], parents: &[Option<usize>], j: usize| match parents[j] {
        Some(p) => [0, 1, 2].map(|k| global[j][k] - global[p][k]),
        None => global[j],
    };
    let head_local: Vec<[f32; 3]> = (0..head.names.len()).map(|j| rest_local(&head.global, &head.parents, j)).collect();
    let given = [Some(on.neck), Some(on.head), on.jaw, on.eyes[0], on.eyes[1]];
    let mut map = vec![usize::MAX; head.names.len()];
    let mut taken = Vec::new();
    let mut added = Vec::new();
    for j in 0..head.names.len() {
        if let Some(Some(b)) = given.get(j) {
            map[j] = *b;
            if j > gf_core::rig::HEAD {
                body.global[*b] = head.global[j];
                taken.push(json!({"joint": head.names[j], "bone": body.names[*b]}));
            }
            continue;
        }
        let mut name = head.names[j].clone();
        while body.names.contains(&name) {
            name.push_str("_face");
        }
        map[j] = body.names.len();
        body.names.push(name.clone());
        body.parents.push(head.parents[j].map(|p| map[p]));
        body.global.push(head.global[j]);
        added.push(name);
    }
    for mut a in head.attachments {
        a.joint = map[a.joint];
        body.attachments.push(a);
    }
    let mut dropped_channels = 0;
    for mut anim in head.animations {
        let before = anim.channels.len();
        anim.channels.retain(|c| c.joint != gf_core::rig::NECK && c.joint != gf_core::rig::HEAD);
        dropped_channels += before - anim.channels.len();
        for c in anim.channels.iter_mut() {
            let now = rest_local(&body.global, &body.parents, map[c.joint]);
            let was = head_local[c.joint];
            for t in c.translation.iter_mut() {
                for k in 0..3 {
                    t[k] += now[k] - was[k];
                }
            }
            c.joint = map[c.joint];
        }
        body.animations.push(anim);
    }
    let report = json!({"neck": body.names[on.neck], "head": body.names[on.head], "taken_over": taken, "added": added.len(), "added_joints": added,
                        "neck_head_channels_dropped": dropped_channels});
    (map, report)
}

type Weights = Vec<(usize, f64)>;

pub fn merge_influences(body: &[Weights], face: &[Weights], map: &[usize], head_bones: &[usize], cap: usize) -> (Vec<Weights>, Vec<Weights>) {
    let mut merged = Vec::with_capacity(body.len());
    let mut corrective = Vec::with_capacity(body.len());
    for (b, f) in body.iter().zip(face) {
        let share: f64 = b.iter().filter(|e| head_bones.contains(&e.0)).map(|e| e.1).sum();
        let own: Vec<(usize, f64)> = f.iter().copied().filter(|e| e.0 != gf_core::rig::NECK && e.1 > 0.0).collect();
        let total: f64 = own.iter().map(|e| e.1).sum();
        if share <= 0.0 || total <= 0.0 {
            merged.push(b.clone());
            corrective.push(vec![(gf_core::rig::NECK, 1.0)]);
            continue;
        }
        let mut out: Vec<(usize, f64)> = b.iter().copied().filter(|e| !head_bones.contains(&e.0)).collect();
        let mut corr: Vec<(usize, f64)> = vec![(gf_core::rig::NECK, 1.0 - share)];
        for &(j, w) in &own {
            let part = share * w / total;
            match out.iter_mut().find(|e| e.0 == map[j]) {
                Some(e) => e.1 += part,
                None => out.push((map[j], part)),
            }
            corr.push((j, part));
        }
        merged.push(gf_core::body_rig::strongest(&out, cap));
        corrective.push(corr.into_iter().filter(|e| e.1 > 0.0).collect());
    }
    (merged, corrective)
}

pub fn face2_animations(names: &[String], fits: &[(Vec<JointPose>, ShapeError)], pivots: &[Point]) -> Vec<gltf::Animation> {
    use gf_core::face_rig2 as fr2;
    let parents = fr2::parents();
    let pg: Vec<Point> = pivots.iter().map(|p| frame(*p)).collect();
    names
        .iter()
        .zip(fits)
        .map(|(name, fit)| {
            let poses: Vec<JointPose> = fit.0.iter().map(|p| JointPose { rotation: frame(p.rotation), translation: frame(p.translation) }).collect();
            let locals = fr::local_pose(&parents, &pg, &poses);
            let channels = (0..fr2::count())
                .filter(|&j| fr::is_free(j))
                .map(|j| {
                    let parent = parents[j].map(|p| pg[p]).unwrap_or([0.0; 3]);
                    let rest = [0, 1, 2].map(|k| (pg[j][k] - parent[k]) as f32);
                    let (q, t) = locals[j];
                    gltf::Channel { joint: j, rotation: [[0.0, 0.0, 0.0, 1.0], q.map(|x| x as f32)], translation: [rest, t.map(|x| x as f32)] }
                })
                .collect();
            gltf::Animation { name: name.clone(), channels, morph: None }
        })
        .collect()
}

pub fn fit_report(names: &[String], fits: &[(Vec<JointPose>, ShapeError)]) -> Value {
    let rows: Vec<Value> = names
        .iter()
        .zip(fits)
        .map(|(n, f)| {
            let mut row = serde_json::to_value(&f.1).unwrap_or(Value::Null);
            row["name"] = json!(n);
            row["poorly_expressed"] = json!(fr::poorly_expressed(&f.1));
            row
        })
        .collect();
    let poor: Vec<&String> = names.iter().zip(fits).filter(|(_, f)| fr::poorly_expressed(&f.1)).map(|(n, _)| n).collect();
    let mut p95: Vec<f64> = fits.iter().map(|f| f.1.p95_mm).collect();
    p95.sort_by(f64::total_cmp);
    let median = p95.get(p95.len() / 2).copied().unwrap_or(0.0);
    let explained = fits.iter().map(|f| f.1.explained).sum::<f64>() / fits.len().max(1) as f64;
    json!({"shapes": rows, "poorly_expressed": poor, "median_p95_mm": median,
           "worst_max_mm": fits.iter().map(|f| f.1.max_mm).fold(0.0, f64::max), "mean_explained": explained,
           "rule": format!("a shape is poorly expressed when the joints explain less than {} of its squared displacement or its p95 error is over {} mm; errors are over the vertices the shape or the joints move more than 0.1 mm", fr::POOR_EXPLAINED, fr::POOR_P95_MM)})
}

type Affine = (Mat, Point);

fn compose(a: &Affine, b: &Affine) -> Affine {
    let r = fr::mul(&a.0, &b.0);
    let t = fr::apply(&a.0, b.1);
    (r, [t[0] + a.1[0], t[1] + a.1[1], t[2] + a.1[2]])
}

fn quat_matrix(q: [f64; 4]) -> Mat {
    let [x, y, z, w] = q;
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

fn f32_of(v: &Value) -> f64 {
    v.as_f64().map(|x| x as f32 as f64).unwrap_or(0.0)
}

struct Skinned {
    name: String,
    node: usize,
    mesh: usize,
    joints: Vec<[usize; 8]>,
    weights: Vec<[f32; 8]>,
}

fn globals(doc: &Value, parent: &[Option<usize>], local: &[Affine], joints: &[usize]) -> Vec<Affine> {
    let _ = doc;
    joints
        .iter()
        .map(|&node| {
            let mut chain = vec![node];
            while let Some(p) = parent[*chain.last().unwrap_or(&node)] {
                chain.push(p);
            }
            let mut g: Affine = (fr::IDENTITY, [0.0; 3]);
            for &n in chain.iter().rev() {
                g = compose(&g, &local[n]);
            }
            g
        })
        .collect()
}

fn skin_points(skin: &Skinned, mats: &[Affine], positions: &[[f32; 3]], normals: &[[f32; 3]]) -> (Vec<Point>, Vec<Point>) {
    let mut p = Vec::with_capacity(positions.len());
    let mut n = Vec::with_capacity(positions.len());
    for (v, position) in positions.iter().enumerate() {
        let x = position.map(f64::from);
        let nn = normals.get(v).map(|q| q.map(f64::from)).unwrap_or([0.0; 3]);
        let (mut y, mut m) = ([0.0; 3], [0.0; 3]);
        for k in 0..8 {
            let w = skin.weights[v][k] as f64;
            if w == 0.0 {
                continue;
            }
            let Some(a) = mats.get(skin.joints[v][k]) else {
                continue;
            };
            let r = fr::apply(&a.0, x);
            let rn = fr::apply(&a.0, nn);
            for c in 0..3 {
                y[c] += w * (r[c] + a.1[c]);
                m[c] += w * rn[c];
            }
        }
        p.push(y);
        n.push(m);
    }
    (p, n)
}

pub fn gates_rig(opts: &Options) -> Result<Value, String> {
    only(opts, "gates rig", &[&["glb", "eyes", "front-deg", "tolerance-mm"]])?;
    let path = need(opts, "glb")?;
    let eyes = match opts.get("eyes") {
        Some(e) => Some(gf_io::load_eyes(Path::new(e))?),
        None => None,
    };
    let (front, tolerance) = lid_settings(opts)?;
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let (doc, bin) = gltf::chunks(&bytes)?;
    let (_, meshes) = gltf::read_glb(&bytes)?;
    let skin = doc["skins"].get(0).ok_or("the glb has no skin")?;
    let joint_nodes: Vec<usize> = skin["joints"].as_array().ok_or("the skin has no joints")?.iter().filter_map(|v| v.as_u64().map(|x| x as usize)).collect();
    let ibm_raw = gltf::read_plain(&doc, bin, skin["inverseBindMatrices"].as_u64().ok_or("the skin has no inverse bind matrices")?)?;
    let ibm: Vec<Affine> = ibm_raw
        .chunks_exact(16)
        .map(|m| {
            let m: Vec<f64> = m.iter().map(|&x| x as f64).collect();
            ([[m[0], m[4], m[8]], [m[1], m[5], m[9]], [m[2], m[6], m[10]]], [m[12], m[13], m[14]])
        })
        .collect();
    let nodes = doc["nodes"].as_array().ok_or("the glb has no nodes")?;
    let mut parent = vec![None; nodes.len()];
    for (i, node) in nodes.iter().enumerate() {
        for c in node["children"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(c) = c.as_u64() {
                parent[c as usize] = Some(i);
            }
        }
    }
    let rest: Vec<Affine> = nodes
        .iter()
        .map(|n| {
            let t = n.get("translation").map(|t| [f32_of(&t[0]), f32_of(&t[1]), f32_of(&t[2])]).unwrap_or([0.0; 3]);
            let r = n.get("rotation").map(|q| quat_matrix([f32_of(&q[0]), f32_of(&q[1]), f32_of(&q[2]), f32_of(&q[3])])).unwrap_or(fr::IDENTITY);
            (r, t)
        })
        .collect();
    let mats = |local: &[Affine]| -> Vec<Affine> { globals(&doc, &parent, local, &joint_nodes).iter().zip(&ibm).map(|(g, b)| compose(g, b)).collect() };
    let mut skinned = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let (Some(m), Some(_)) = (node["mesh"].as_u64(), node["skin"].as_u64()) else {
            continue;
        };
        let attributes = &doc["meshes"][m as usize]["primitives"][0]["attributes"];
        let j = gltf::read_plain(&doc, bin, attributes["JOINTS_0"].as_u64().ok_or("a skinned mesh has no JOINTS_0")?)?;
        let w = gltf::read_plain(&doc, bin, attributes["WEIGHTS_0"].as_u64().ok_or("a skinned mesh has no WEIGHTS_0")?)?;
        let (j1, w1) = match (attributes["JOINTS_1"].as_u64(), attributes["WEIGHTS_1"].as_u64()) {
            (Some(a), Some(b)) => (gltf::read_plain(&doc, bin, a)?, gltf::read_plain(&doc, bin, b)?),
            (None, None) => (vec![0.0; j.len()], vec![0.0; w.len()]),
            _ => return Err("a skinned mesh has only one of JOINTS_1 / WEIGHTS_1".into()),
        };
        if j1.len() != j.len() || w1.len() != w.len() {
            return Err("JOINTS_1 / WEIGHTS_1 do not match JOINTS_0 / WEIGHTS_0".into());
        }
        skinned.push(Skinned {
            name: node["name"].as_str().unwrap_or("").to_string(),
            node: i,
            mesh: m as usize,
            joints: j.chunks_exact(4).zip(j1.chunks_exact(4)).map(|(c, d)| [c[0], c[1], c[2], c[3], d[0], d[1], d[2], d[3]].map(|x| x as usize)).collect(),
            weights: w.chunks_exact(4).zip(w1.chunks_exact(4)).map(|(c, d)| [c[0], c[1], c[2], c[3], d[0], d[1], d[2], d[3]]).collect(),
        });
    }
    if skinned.is_empty() {
        return Err(format!("{path}: no skinned mesh"));
    }
    let rest_mats = mats(&rest);
    let identity_dev = rest_mats
        .iter()
        .map(|a| {
            let mut d = a.1.iter().map(|x| x.abs()).fold(0.0, f64::max);
            for (r, row) in a.0.iter().enumerate() {
                for (c, x) in row.iter().enumerate() {
                    d = d.max((x - if r == c { 1.0 } else { 0.0 }).abs());
                }
            }
            d
        })
        .fold(0.0, f64::max);
    let mut failed = Vec::new();
    let mut bind_rows = Vec::new();
    for s in &skinned {
        let mesh = &meshes[s.mesh];
        let (p, _) = skin_points(s, &rest_mats, &mesh.positions, &mesh.normals);
        let bind = p.iter().zip(&mesh.positions).map(|(a, b)| (0..3).map(|k| (a[k] - b[k] as f64).abs()).fold(0.0, f64::max)).fold(0.0, f64::max);
        let sum_error = s.weights.iter().map(|w| ((w[0] + w[1] + w[2] + w[3] + w[4] + w[5] + w[6] + w[7]) as f64 - 1.0).abs()).fold(0.0, f64::max);
        let negative = s.weights.iter().flatten().filter(|w| **w < 0.0).count();
        let bad_joint = s.joints.iter().flatten().filter(|j| **j >= joint_nodes.len()).count();
        if bind > 0.0 {
            failed.push(format!("{}: bind", s.name));
        }
        if sum_error > 0.0 || negative > 0 || bad_joint > 0 {
            failed.push(format!("{}: weights", s.name));
        }
        bind_rows.push(json!({"mesh": s.name, "vertices": mesh.positions.len(), "bind_max_m": bind, "weight_sum_max_error": sum_error,
                              "negative_weights": negative, "joint_out_of_range": bad_joint}));
    }
    let mut node_of_joint: HashMap<usize, usize> = HashMap::new();
    for (j, &n) in joint_nodes.iter().enumerate() {
        node_of_joint.insert(n, j);
    }
    let mut pose_rows = Vec::new();
    let mut worse_facing = 0usize;
    let mut worse_lids = 0usize;
    for animation in doc["animations"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let name = animation["name"].as_str().unwrap_or("").to_string();
        let mut local = rest.clone();
        let mut morph_weights: HashMap<usize, Vec<f32>> = HashMap::new();
        for ch in animation["channels"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let node = ch["target"]["node"].as_u64().ok_or("a channel has no node")? as usize;
            let sampler = &animation["samplers"][ch["sampler"].as_u64().ok_or("a channel has no sampler")? as usize];
            let out = gltf::read_plain(&doc, bin, sampler["output"].as_u64().ok_or("a sampler has no output")?)?;
            match ch["target"]["path"].as_str() {
                Some("rotation") => {
                    let q = &out[out.len() - 4..];
                    local[node].0 = quat_matrix([q[0] as f64, q[1] as f64, q[2] as f64, q[3] as f64]);
                }
                Some("translation") => {
                    let t = &out[out.len() - 3..];
                    local[node].1 = [t[0] as f64, t[1] as f64, t[2] as f64];
                }
                Some("weights") => {
                    morph_weights.insert(node, out[out.len() / 2..].to_vec());
                }
                _ => {}
            }
        }
        let posed = mats(&local);
        let mut rows = Vec::new();
        for s in &skinned {
            let mesh = &meshes[s.mesh];
            let (p, n) = match morph_weights.get(&s.node) {
                Some(w) => {
                    let mut mp = mesh.positions.clone();
                    let mut mn = mesh.normals.clone();
                    for (k, m) in mesh.morphs.iter().enumerate() {
                        let wk = w.get(k).copied().unwrap_or(0.0);
                        if wk == 0.0 {
                            continue;
                        }
                        for (a, d) in mp.iter_mut().zip(&m.positions) {
                            *a = [0, 1, 2].map(|c| a[c] + wk * d[c]);
                        }
                        for (a, d) in mn.iter_mut().zip(&m.normals) {
                            *a = [0, 1, 2].map(|c| a[c] + wk * d[c]);
                        }
                    }
                    skin_points(s, &posed, &mp, &mn)
                }
                None => skin_points(s, &posed, &mesh.positions, &mesh.normals),
            };
            let tris: Vec<[u32; 3]> = mesh.indices.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
            let joint_facing = tris.iter().filter(|t| gm::facing_dot(&p, &n, t) < 0.0).count();
            let mut weld: HashMap<[u32; 3], u32> = HashMap::new();
            let id: Vec<u32> = mesh.positions.iter().map(|q| {
                let next = weld.len() as u32;
                *weld.entry(q.map(f32::to_bits)).or_insert(next)
            }).collect();
            let welded: Vec<[u32; 3]> = tris.iter().map(|t| t.map(|v| id[v as usize])).collect();
            let folds = |pp: &[Point]| {
                let mut wp = vec![[0.0; 3]; weld.len()];
                for (v, q) in pp.iter().enumerate() {
                    wp[id[v] as usize] = *q;
                }
                let wn = gm::game_normals(&wp, &welded);
                welded.iter().filter(|t| gm::facing_dot(&wp, &wn, t) < 0.0).count()
            };
            let joint_folds = folds(&p);
            let k = mesh.morph_names.iter().position(|m| *m == name);
            let blend = k.map(|k| {
                let m = &mesh.morphs[k];
                let bp: Vec<Point> = mesh.positions.iter().zip(&m.positions).map(|(a, d)| [0, 1, 2].map(|i| a[i] as f64 + d[i] as f64)).collect();
                let bn: Vec<Point> = mesh.normals.iter().zip(&m.normals).map(|(a, d)| [0, 1, 2].map(|i| a[i] as f64 + d[i] as f64)).collect();
                (bp, bn)
            });
            let blend_facing = blend.as_ref().map(|(bp, bn)| tris.iter().filter(|t| gm::facing_dot(bp, bn, t) < 0.0).count());
            let blend_folds = blend.as_ref().map(|(bp, _)| folds(bp));
            let mut row = json!({"mesh": s.name, "joint_folds": joint_folds, "blend_folds": blend_folds,
                                 "joint_against_skinned_normals": joint_facing, "blend_against_morph_normals": blend_facing});
            if blend_folds.is_some_and(|b| joint_folds > b) {
                worse_facing += 1;
                failed.push(format!("{}: {name} folds", s.name));
            }
            if let Some(eyes) = &eyes {
                let mut seen: HashMap<[u32; 3], ()> = HashMap::new();
                let unique: Vec<usize> = (0..mesh.positions.len()).filter(|&i| seen.insert(mesh.positions[i].map(f32::to_bits), ()).is_none()).collect();
                let check = |pp: &[Point], nn: &[Point]| {
                    let wp: Vec<Point> = unique.iter().map(|&i| from_gltf(pp[i].map(|x| x as f32))).collect();
                    let wn: Vec<Point> = unique.iter().map(|&i| from_gltf(nn[i].map(|x| x as f32))).collect();
                    lids::lid_check(&wp, &wn, eyes, front, tolerance)
                };
                let joint_lids = check(&p, &n);
                let blend_lids = blend.as_ref().map(|(bp, bn)| check(bp, bn));
                row["joint_lid_inside"] = json!(joint_lids.inside);
                row["joint_lid_deepest_mm"] = json!(joint_lids.deepest_mm);
                row["blend_lid_inside"] = json!(blend_lids.as_ref().map(|b| b.inside));
                if joint_lids.inside > blend_lids.as_ref().map_or(0, |b| b.inside) {
                    worse_lids += 1;
                    failed.push(format!("{}: {name} lids", s.name));
                }
            }
            rows.push(row);
        }
        pose_rows.push(json!({"animation": name, "meshes": rows}));
    }
    let rule = "bind: every skinned vertex at the rest pose lands exactly (0.0 m in f64) on its exported position; weights: every vertex's f32 weights sum to exactly 1, none negative, every joint index in range; per animation (sampled at its last key) and mesh: the joint pose has no more folded triangles (facing against the area-weighted vertex normals recomputed on the position-welded posed mesh) than the blend shape of the same name at weight 1 measured the same way (the counts against the skinned normals and the morph normals are reported, not gated), and with --eyes no more vertices inside an eye (the gates lids rule) than that blend shape";
    Ok(verdict(
        "gates rig",
        json!({"bind": bind_rows, "skin_times_bind_identity_max_deviation": identity_dev, "animations": pose_rows.len(),
               "poses_worse_folds": worse_facing, "poses_worse_lids": worse_lids, "poses": pose_rows}),
        failed,
        json!({"glb": path, "joints": joint_nodes.len(), "eyes": opts.get("eyes"), "front_deg": front, "tolerance_mm": tolerance * 1000.0, "rule": rule}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig(names: &[&str], parents: Vec<Option<usize>>) -> gltf::Rig {
        gltf::Rig { names: names.iter().map(|s| s.to_string()).collect(), parents, global: (0..names.len()).map(|k| [k as f32, 0.0, 0.0]).collect(), attachments: Vec::new(), animations: Vec::new() }
    }

    #[test]
    fn a_head_rig_joins_the_body_skeleton_at_its_neck_and_head() {
        let mut body = rig(&["hips", "neck_01", "head", "jaw"], vec![None, Some(0), Some(1), Some(2)]);
        let mut head = rig(&["neck", "head", "jaw", "eye_L", "eye_R", "brow"], vec![None, Some(0), Some(1), Some(1), Some(1), Some(1)]);
        head.global[2] = [9.0, 9.0, 9.0];
        head.animations.push(gltf::Animation {
            name: "smile".into(),
            channels: vec![
                gltf::Channel { joint: 1, rotation: [[0.0, 0.0, 0.0, 1.0]; 2], translation: [[0.0; 3]; 2] },
                gltf::Channel { joint: 5, rotation: [[0.0, 0.0, 0.0, 1.0]; 2], translation: [[4.0, 0.0, 0.0]; 2] },
            ],
            morph: None,
        });
        let (map, report) = merge_head_rig(&mut body, head, &OnBody { neck: 1, head: 2, jaw: Some(3), eyes: [None, None] });
        assert_eq!(map, vec![1, 2, 3, 4, 5, 6], "{report}");
        assert_eq!(body.names, ["hips", "neck_01", "head", "jaw", "eye_L", "eye_R", "brow"]);
        assert_eq!(body.parents[4..], [Some(2), Some(2), Some(2)], "added joints hang from the body's head bone");
        assert_eq!(body.global[3], [9.0, 9.0, 9.0], "the preset's jaw moves to the head rig's hinge");
        assert_eq!(body.animations[0].channels.len(), 1, "the head bone belongs to the body: its channel is dropped");
        assert_eq!(body.animations[0].channels[0].joint, 6);
        assert_eq!(body.animations[0].channels[0].translation, [[3.0, 0.0, 0.0]; 2], "keys keep the pose delta over the new parent: brow at 5 under the body's head at 2");
        let mut clash = rig(&["hips", "brow"], vec![None, Some(0)]);
        let (_, _) = merge_head_rig(&mut clash, rig(&["neck", "head", "brow"], vec![None, Some(0), Some(1)]), &OnBody { neck: 0, head: 1, jaw: None, eyes: [None, None] });
        assert_eq!(clash.names, ["hips", "brow", "brow_face"], "a name the body already has gets a suffix");
    }

    #[test]
    fn the_head_bone_s_share_is_split_by_the_face_rig_and_the_seam_keeps_the_body_s() {
        let map = vec![1, 2, 3, 4, 5, 6];
        let body = vec![vec![(2, 0.8), (1, 0.2)], vec![(1, 1.0)]];
        let face = vec![vec![(0, 0.1), (1, 0.5), (5, 0.4)], vec![(0, 1.0)]];
        let (merged, corrective) = merge_influences(&body, &face, &map, &[2], 8);
        let (with_jaw, _) = merge_influences(&[vec![(2, 0.5), (3, 0.3), (1, 0.2)]], &[vec![(1, 0.5), (2, 0.5)]], &map, &[2, 3], 8);
        let (capped, _) = merge_influences(&[vec![(2, 1.0)]], &[vec![(1, 0.3), (2, 0.25), (3, 0.2), (4, 0.15), (5, 0.1)]], &map, &[2], 2);
        assert_eq!(capped[0].len(), 2, "the body's influence cap holds after the split: {:?}", capped[0]);
        assert_eq!(with_jaw[0].iter().find(|e| e.0 == 1).map(|e| e.1), Some(0.2), "only the neck share stays the body's: {:?}", with_jaw[0]);
        assert!((with_jaw[0].iter().find(|e| e.0 == 3).map_or(0.0, |e| e.1) - 0.4).abs() < 1e-12, "the preset jaw's own share is re-split with the head's: {:?}", with_jaw[0]);
        let get = |v: &[(usize, f64)], j: usize| v.iter().find(|e| e.0 == j).map_or(0.0, |e| e.1);
        assert!((get(&merged[0], 1) - 0.2).abs() < 1e-12, "the neck share is the body's: {:?}", merged[0]);
        assert!((get(&merged[0], 2) - 0.8 * 0.5 / 0.9).abs() < 1e-12);
        assert!((get(&merged[0], 6) - 0.8 * 0.4 / 0.9).abs() < 1e-12);
        assert!((merged[0].iter().map(|e| e.1).sum::<f64>() - 1.0).abs() < 1e-12);
        assert!((get(&corrective[0], 0) - 0.2).abs() < 1e-12 && (get(&corrective[0], 5) - 0.8 * 0.4 / 0.9).abs() < 1e-12, "{:?}", corrective[0]);
        assert_eq!(merged[1], body[1], "a vertex the face rig gives wholly to the neck stays exactly the body's");
        assert_eq!(corrective[1], vec![(0, 1.0)]);
    }
}
