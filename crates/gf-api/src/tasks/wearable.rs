use std::collections::{HashMap, HashSet};
use std::path::Path;

use gf_core::game_mesh::Point;
use gf_core::macros::{self, Mode};
use gf_core::wearable::Binding;
use gf_io::character as ch;
use serde_json::{Value, json};

use super::data::{json_arg, number_map};
use super::{Options, need, only};

pub const CHECK_KEYS: &[&str] = &["base", "mhclo", "macro", "targets", "sliders"];
pub const IMPORT_KEYS: &[&str] = &["store", "mhclo"];
pub const ATTACH_KEYS: &[&str] = &["store", "spec", "macro", "obj", "bone", "rig", "name", "diffuse", "frame", "scale"];

fn source_mesh(store: &gf_io::store::Store, path: &Path, diffuse: Option<&String>) -> Result<(std::path::PathBuf, Option<std::path::PathBuf>, Value), String> {
    let given = diffuse.map(std::path::PathBuf::from);
    if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("glb")) {
        return Ok((path.to_path_buf(), given, Value::Null));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let scene = gf_io::gltf::read_glb_scene(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let dir = store.root.join("cache").join("glb").join(gf_core::hash::sha256_hex(&bytes));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stem = path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "glb".into());
    let obj = dir.join(format!("{stem}.obj"));
    std::fs::write(&obj, gf_io::gltf::scene_obj_text(&scene)).map_err(|e| format!("{}: {e}", obj.display()))?;
    let (texture, note) = match (&given, &scene.base_color) {
        (Some(_), _) => (given.clone(), "the colour texture given".to_string()),
        (None, Some((mime, data))) if mime == "image/png" => {
            let png = dir.join(format!("{stem}_color.png"));
            std::fs::write(&png, data).map_err(|e| format!("{}: {e}", png.display()))?;
            (Some(png), "the glb's PNG colour texture".to_string())
        }
        (None, Some((mime, data))) if mime == "image/jpeg" => {
            let (w, h, rgba) = gf_io::jpeg::decode_rgba(data).map_err(|e| format!("{}: the colour texture: {e}", path.display()))?;
            let png = dir.join(format!("{stem}_color.png"));
            std::fs::write(&png, gf_io::png::rgba(w, h, &rgba)?).map_err(|e| format!("{}: {e}", png.display()))?;
            (Some(png), "the glb's JPEG colour texture, converted to PNG".to_string())
        }
        (None, Some((mime, _))) => (None, format!("the glb's colour texture is {mime}, which is not read yet: give a PNG with diffuse")),
        (None, None) => match &scene.base_color_uri {
            Some(uri) => (None, format!("the glb's colour texture is outside the glb ({uri}), which is not read: give it with diffuse")),
            None => (None, "the glb has no colour texture".to_string()),
        },
    };
    Ok((obj, texture, json!({"glb": path.display().to_string(), "vertices": scene.positions.len(), "triangles": scene.triangles.len(), "texture": note,
                             "frame": "glTF y up converted to z up; node transforms applied"})))
}

pub const PIN_NEAR_M: f64 = 0.005;
pub const PIN_FAR_M: f64 = 0.03;
const COLLIDER_SKIP: &[&str] = &["index", "middle", "ring", "pinky", "thumb", "ball", "toe", "Root"];
pub const COLLIDER_PERCENTILE: f64 = 0.1;

pub fn pin_weight(distance_out: f64) -> f32 {
    let t = ((distance_out - PIN_NEAR_M) / (PIN_FAR_M - PIN_NEAR_M)).clamp(0.0, 1.0);
    (1.0 - t * t * (3.0 - 2.0 * t)) as f32
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = sub(b, a);
    let t = (dot(sub(p, a), ab) / dot(ab, ab).max(1e-18)).clamp(0.0, 1.0);
    let q = [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]];
    dot(sub(p, q), sub(p, q)).sqrt()
}

pub struct Capsule {
    pub joint: usize,
    pub name: String,
    pub head: Point,
    pub tail: Point,
    pub radius: f64,
    pub vertices: usize,
}

pub fn capsules(pack: &gf_core::body::BodyPack, controls: &gf_core::body::Controls, rig: &gf_core::body_rig::RigDef, by_obj: &HashMap<u32, gf_core::body_rig::Influences>, frame: &BaseFrame) -> Result<Vec<Capsule>, String> {
    let at = gf_core::body_rig::Resolver::new(&pack.mesh.positions, &pack.source, &pack.helpers);
    let ends = rig.evaluate(&at)?;
    let control = pack.control_joint(controls);
    let mut owned: Vec<Vec<Point>> = vec![Vec::new(); rig.bones.len()];
    for (id, p) in frame.skin_points() {
        if let Some(&(j, _)) = by_obj.get(&id).and_then(|w| w.iter().max_by(|a, b| a.1.total_cmp(&b.1))) {
            owned[j].push(p);
        }
    }
    let mut out = Vec::new();
    for (j, bone) in rig.bones.iter().enumerate() {
        if COLLIDER_SKIP.iter().any(|k| bone.name.contains(k)) || owned[j].len() < 8 {
            continue;
        }
        let (h, t) = (control(ends[j].0), control(ends[j].1));
        let mut d: Vec<f64> = owned[j].iter().map(|&p| segment_distance(p, h, t)).collect();
        d.sort_by(f64::total_cmp);
        let radius = d[((d.len() as f64 * COLLIDER_PERCENTILE) as usize).min(d.len() - 1)];
        out.push(Capsule { joint: j, name: bone.name.clone(), head: h, tail: t, radius, vertices: d.len() });
    }
    Ok(out)
}

pub const CHAIN_SEGMENTS: usize = 4;
const CHAIN_VERTICES: usize = 300;
const CHAIN_MAX: usize = 12;
const CHAIN_MIN_REACH_M: f64 = 0.03;
const CHAIN_MIN_SEGMENT_M: f64 = 0.002;
const CHAIN_MERGE_M: f64 = 0.03;
const PINNED: f32 = 0.99;
const SEED_PIN: f32 = 0.5;

pub struct Chain {
    pub root_vertices: Vec<u32>,
    pub joints: Vec<Point>,
}

pub struct Chains {
    pub chains: Vec<Chain>,
    pub along: Vec<Option<(usize, f64)>>,
}

pub fn spring_seeds(garment: &[Point], pin: &[f32], dominant: &[usize]) -> (Vec<bool>, Option<usize>) {
    let mut pinned: Vec<usize> = (0..garment.len()).filter(|&v| pin[v] >= PINNED).collect();
    pinned.sort_by(|&a, &b| garment[b][2].total_cmp(&garment[a][2]).then(a.cmp(&b)));
    let mut votes: HashMap<usize, usize> = HashMap::new();
    for &v in &pinned[..pinned.len().div_ceil(10)] {
        *votes.entry(dominant[v]).or_insert(0) += 1;
    }
    let anchor = votes.into_iter().max_by_key(|&(bone, n)| (n, std::cmp::Reverse(bone))).map(|(bone, _)| bone);
    ((0..garment.len()).map(|v| pin[v] >= SEED_PIN && Some(dominant[v]) == anchor).collect(), anchor)
}

pub fn build_chains(garment: &[Point], triangles: &[[u32; 3]], seed: &[bool]) -> Chains {
    let n = garment.len();
    let mut adj: Vec<Vec<(u32, f64)>> = vec![Vec::new(); n];
    for t in triangles {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let d = dot(sub(garment[a as usize], garment[b as usize]), sub(garment[a as usize], garment[b as usize])).sqrt();
            adj[a as usize].push((b, d));
            adj[b as usize].push((a, d));
        }
    }
    let mut dist = vec![f64::INFINITY; n];
    let mut root = vec![u32::MAX; n];
    let mut heap = std::collections::BinaryHeap::new();
    for v in 0..n {
        if seed[v] {
            dist[v] = 0.0;
            root[v] = v as u32;
            heap.push((std::cmp::Reverse(0u64), v as u32));
        }
    }
    for bridge in [false, true] {
        if bridge {
            let reached: Vec<usize> = (0..n).filter(|&v| dist[v].is_finite()).collect();
            if reached.is_empty() {
                break;
            }
            let unreached: Vec<usize> = (0..n).filter(|&v| !dist[v].is_finite()).collect();
            for v in unreached {
                let gap = |u: usize| dot(sub(garment[v], garment[u]), sub(garment[v], garment[u])).sqrt();
                if let Some(u) = reached.iter().copied().min_by(|&a, &b| gap(a).total_cmp(&gap(b))) {
                    dist[v] = dist[u] + gap(u);
                    root[v] = root[u];
                    heap.push((std::cmp::Reverse(dist[v].to_bits()), v as u32));
                }
            }
        }
        while let Some((std::cmp::Reverse(dk), v)) = heap.pop() {
            let d = f64::from_bits(dk);
            if d > dist[v as usize] {
                continue;
            }
            for &(u, w) in &adj[v as usize] {
                let nd = d + w;
                if nd < dist[u as usize] {
                    dist[u as usize] = nd;
                    root[u as usize] = root[v as usize];
                    heap.push((std::cmp::Reverse(nd.to_bits()), u));
                }
            }
        }
    }
    let free: Vec<usize> = (0..n).filter(|&v| !seed[v] && dist[v].is_finite()).collect();
    let mut along = vec![None; n];
    if free.is_empty() {
        return Chains { chains: Vec::new(), along };
    }
    let k = (free.len() / CHAIN_VERTICES).clamp(1, CHAIN_MAX);
    let anchor = |v: usize| garment[root[v] as usize];
    let mut centres: Vec<Point> = vec![anchor(free[0])];
    while centres.len() < k {
        let far = free.iter().copied().max_by(|&a, &b| {
            let da = centres.iter().map(|c| dot(sub(anchor(a), *c), sub(anchor(a), *c))).fold(f64::INFINITY, f64::min);
            let db = centres.iter().map(|c| dot(sub(anchor(b), *c), sub(anchor(b), *c))).fold(f64::INFINITY, f64::min);
            da.total_cmp(&db)
        });
        centres.push(anchor(far.unwrap_or(free[0])));
    }
    let mut label = vec![0usize; n];
    for _ in 0..12 {
        for &v in &free {
            label[v] = (0..k).min_by(|&a, &b| dot(sub(anchor(v), centres[a]), sub(anchor(v), centres[a])).total_cmp(&dot(sub(anchor(v), centres[b]), sub(anchor(v), centres[b])))).unwrap_or(0);
        }
        let mut sum = vec![([0.0; 3], 0usize); k];
        for &v in &free {
            let (acc, count) = &mut sum[label[v]];
            for (s, a) in acc.iter_mut().zip(anchor(v)) {
                *s += a;
            }
            *count += 1;
        }
        for c in 0..k {
            if sum[c].1 > 0 {
                centres[c] = sum[c].0.map(|x| x / sum[c].1 as f64);
            }
        }
    }
    let mut chains = Vec::new();
    let mut tracks: Vec<Option<Vec<Point>>> = vec![None; k];
    for (pass, c) in (0..2).flat_map(|pass| (0..k).map(move |c| (pass, c))) {
        if pass == 1 && c == 0 {
            let mut parent: Vec<usize> = (0..k).collect();
            fn find(parent: &mut [usize], x: usize) -> usize {
                let mut r = x;
                while parent[r] != r {
                    r = parent[r];
                }
                parent[x] = r;
                r
            }
            for a in 0..k {
                for b in a + 1..k {
                    if let (Some(ta), Some(tb)) = (&tracks[a], &tracks[b]) {
                        let to_line = |p: Point, line: &[Point]| {
                            line.windows(2)
                                .map(|w| {
                                    let (d, e) = (sub(w[1], w[0]), sub(p, w[0]));
                                    let s = (dot(e, d) / dot(d, d).max(1e-12)).clamp(0.0, 1.0);
                                    let r = sub(e, d.map(|x| x * s));
                                    dot(r, r).sqrt()
                                })
                                .fold(f64::INFINITY, f64::min)
                        };
                        let along = |x: &[Point], y: &[Point]| x.iter().map(|&p| to_line(p, y)).sum::<f64>() / x.len() as f64;
                        if along(ta, tb).min(along(tb, ta)) < CHAIN_MERGE_M {
                            let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                            parent[ra.max(rb)] = ra.min(rb);
                        }
                    }
                }
            }
            for &v in &free {
                label[v] = find(&mut parent, label[v]);
            }
        }
        let mut members: Vec<usize> = free.iter().copied().filter(|&v| label[v] == c).collect();
        let reach = members.iter().map(|&v| dist[v]).fold(0.0f64, f64::max);
        if reach < CHAIN_MIN_REACH_M || members.len() < CHAIN_SEGMENTS {
            continue;
        }
        members.sort_by(|&a, &b| dist[a].total_cmp(&dist[b]).then(a.cmp(&b)));
        let mut roots: Vec<u32> = members.iter().map(|&v| root[v]).collect();
        roots.sort_unstable();
        roots.dedup();
        let mean = |vs: &mut dyn Iterator<Item = Point>| -> Option<Point> {
            let (mut acc, mut cnt) = ([0.0; 3], 0usize);
            for p in vs {
                for i in 0..3 {
                    acc[i] += p[i];
                }
                cnt += 1;
            }
            (cnt > 0).then(|| acc.map(|x| x / cnt as f64))
        };
        let base = mean(&mut roots.iter().map(|&r| garment[r as usize])).unwrap_or(garment[members[0]]);
        let mut joints = vec![base];
        let mut station = vec![0.0];
        for seg in 1..=CHAIN_SEGMENTS {
            let ring = &members[(seg - 1) * members.len() / CHAIN_SEGMENTS..seg * members.len() / CHAIN_SEGMENTS];
            joints.push(mean(&mut ring.iter().map(|&v| garment[v])).unwrap_or(base));
            station.push(ring.iter().map(|&v| dist[v]).sum::<f64>() / ring.len() as f64);
        }
        let short = joints.windows(2).any(|w| dot(sub(w[1], w[0]), sub(w[1], w[0])).sqrt() < CHAIN_MIN_SEGMENT_M);
        if short || station.windows(2).any(|w| w[1] <= w[0]) {
            continue;
        }
        if pass == 0 {
            tracks[c] = Some(joints);
            continue;
        }
        let index = chains.len();
        for &v in &members {
            let seg = (1..=CHAIN_SEGMENTS).find(|&s| dist[v] < station[s]).unwrap_or(CHAIN_SEGMENTS);
            let f = ((dist[v] - station[seg - 1]) / (station[seg] - station[seg - 1])).clamp(0.0, 1.0);
            along[v] = Some((index, (seg - 1) as f64 + f));
        }
        chains.push(Chain { root_vertices: roots, joints });
    }
    Chains { chains, along }
}

pub struct BoneFrame {
    pub origin: Point,
    pub axes: [Point; 3],
    pub length: f64,
}

pub fn bone_frame(pack: &gf_core::body::BodyPack, controls: &gf_core::body::Controls, rig: &str, bone: &str) -> Result<BoneFrame, String> {
    let (rig_json, _, _) = crate::body::rig_source(rig, None)?;
    let def = gf_core::body_rig::RigDef::parse(&rig_json)?;
    let i = def.bones.iter().position(|b| b.name == bone).ok_or_else(|| format!("{rig} has no bone {bone}"))?;
    let at = gf_core::body_rig::Resolver::new(&pack.mesh.positions, &pack.source, &pack.helpers);
    let ends = def.evaluate(&at)?;
    let control = pack.control_joint(controls);
    let (h, t) = (control(ends[i].0), control(ends[i].1));
    let along = sub(t, h);
    let length = dot(along, along).sqrt();
    if length < 1e-9 {
        return Err(format!("bone {bone} has no length on this character"));
    }
    let y = along.map(|x| x / length);
    let forward = [0.0, -1.0, 0.0];
    let mut x = cross(y, forward);
    if dot(x, x) < 1e-12 {
        x = cross(y, [0.0, 0.0, 1.0]);
    }
    let x = normalized(x);
    let z = cross(x, y);
    Ok(BoneFrame { origin: h, axes: [x, y, z], length })
}

impl BoneFrame {
    pub fn to_local(&self, p: Point) -> Point {
        let d = sub(p, self.origin);
        self.axes.map(|a| dot(d, a) / self.length)
    }

    pub fn from_local(&self, l: Point) -> Point {
        [0, 1, 2].map(|k| self.origin[k] + self.length * (l[0] * self.axes[0][k] + l[1] * self.axes[1][k] + l[2] * self.axes[2][k]))
    }
}

pub fn place(item: &gf_io::wearable_store::Wearable, frame: &BaseFrame, pack: &gf_core::body::BodyPack, controls: &gf_core::body::Controls) -> Result<Vec<Point>, String> {
    match &item.manifest.attach {
        Some(a) => {
            let mut bf = bone_frame(pack, controls, &a.rig, &a.bone)?;
            if !a.scale_with_bone {
                bf.length = a.length_m;
            }
            Ok(item.positions.iter().map(|&l| bf.from_local(l)).collect())
        }
        None => frame.evaluate(&item.binding),
    }
}

pub fn attach(opts: &Options) -> Result<Value, String> {
    only(opts, "wearable attach", &[ATTACH_KEYS])?;
    let store = gf_io::store::Store::open(Path::new(need(opts, "store")?))?;
    let spec = ch::read_spec(Path::new(need(opts, "spec")?))?;
    let source = opts.get("macro").map(|p| ch::read_macro_definition(Path::new(p))).transpose()?;
    let (_, _, _, pin) = ch::evaluate_traced(&store, &spec, source.as_ref())?;
    let shaped = crate::body::spec_body(&store, &spec, source.as_ref(), pin.as_ref())?;
    let rig = opts.get("rig").map(String::as_str).unwrap_or("game_engine");
    let bone = need(opts, "bone")?;
    let bf = bone_frame(&shaped.pack, &shaped.controls, rig, bone)?;
    let (obj_buf, texture, from_glb) = source_mesh(&store, Path::new(need(opts, "obj")?), opts.get("diffuse"))?;
    let obj_path = obj_buf.as_path();
    let mesh = gf_io::body::read_obj(obj_path, None)?;
    let world = match opts.get("frame").map(String::as_str).unwrap_or("world") {
        "world" => true,
        "canonical" => false,
        other => return Err(format!("--frame takes world or canonical, got {other}")),
    };
    let local: Vec<Point> = mesh.positions.iter().map(|&p| bf.to_local(if world { shaped.similarity.invert(p) } else { p })).collect();
    let name = opts.get("name").cloned().unwrap_or_else(|| obj_path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "solid".into()));
    let scale_with_bone = match opts.get("scale").map(String::as_str).unwrap_or("none") {
        "none" => false,
        "bone" => true,
        other => return Err(format!("--scale takes none (the item keeps its size) or bone (it scales with the bone's length), got {other}")),
    };
    let a = gf_io::wearable_store::Attach { bone: bone.to_string(), rig: rig.to_string(), length_m: bf.length, scale_with_bone };
    let (reference, m) = store.import_attached(&name, a, &local, obj_path, texture.as_deref(), vec![format!("attached to {bone} ({rig}) of {}", spec.name)])?;
    Ok(json!({"ok": true, "command": "wearable attach", "wearable": reference, "name": m.name, "vertices": m.vertices, "bone": bone, "rig": rig, "bone_length_m": bf.length, "from_glb": from_glb,
              "meaning": "a rigid item: its vertices are kept in the bone's frame on the character it was modelled on (origin at the bone's head, y along the bone, x from the character's forward) and placed by the same frame on any character - at its modelled size, or scaled with the bone's length with --scale bone; skinned wholly to that bone", "scale": if scale_with_bone { "bone" } else { "none" }}))
}
pub const BIND_KEYS: &[&str] = &["store", "spec", "macro", "obj", "base", "name", "diffuse", "frame", "out-mhclo", "hide-under", "compare-hidden"];
const HIDE_REACH_M: f64 = 0.15;

fn covered_base(base: &std::collections::BTreeMap<u32, Point>, body: &HashMap<u32, Point>, faces: &[[u32; 3]], garment: &[Point], garment_tris: &[[u32; 3]], unit: f64) -> Vec<u32> {
    let mut normals: std::collections::BTreeMap<u32, Point> = std::collections::BTreeMap::new();
    for f in faces {
        let Some(p) = f.iter().map(|v| base.get(v).copied()).collect::<Option<Vec<Point>>>() else { continue };
        let n = cross(sub(p[1], p[0]), sub(p[2], p[0]));
        for v in f {
            let e = normals.entry(*v).or_insert([0.0; 3]);
            for k in 0..3 {
                e[k] += n[k];
            }
        }
    }
    let tris: Vec<[Point; 3]> = garment_tris.iter().map(|t| t.map(|v| garment[v as usize])).collect();
    let reach = HIDE_REACH_M / unit;
    let behind = 0.005 / unit;
    let mut out: Vec<u32> = normals
        .iter()
        .filter(|(v, _)| body.contains_key(v))
        .filter(|(v, n)| {
            let (o, d) = (base[v], normalized(**n));
            tris.iter().any(|t| ray_hits(o, d, t).is_some_and(|x| (-behind..=reach).contains(&x)))
        })
        .map(|(v, _)| *v)
        .collect();
    out.sort_unstable();
    out
}
const SCALE_PAIRS: [(u32, u32); 3] = [(5399, 11998), (791, 881), (962, 5320)];

pub fn bind(opts: &Options) -> Result<Value, String> {
    only(opts, "wearable bind", &[BIND_KEYS])?;
    let store = gf_io::store::Store::open(Path::new(need(opts, "store")?))?;
    let spec = ch::read_spec(Path::new(need(opts, "spec")?))?;
    let source = opts.get("macro").map(|p| ch::read_macro_definition(Path::new(p))).transpose()?;
    let (dense, _, _, pin) = ch::evaluate_traced(&store, &spec, source.as_ref())?;
    let shaped = crate::body::spec_body(&store, &spec, source.as_ref(), pin.as_ref())?;
    let frame = BaseFrame::new(&shaped.pack, &shaped.controls, &shaped.manifest, &dense);
    let base = frame.surface_ids();
    let polys = gf_io::body::read_obj(Path::new(need(opts, "base")?), Some("body"))?;
    let faces: Vec<[u32; 3]> = polys.faces.iter().flat_map(|f| (1..f.len().saturating_sub(1)).map(move |k| [f[0].0, f[k].0, f[k + 1].0])).collect();
    let mut scales = Vec::new();
    for (axis, (a, b)) in SCALE_PAIRS.iter().enumerate() {
        let (pa, pb) = (base.get(a).ok_or(format!("scale vertex {a} is not on the character"))?, base.get(b).ok_or(format!("scale vertex {b} is not on the character"))?);
        scales.push(gf_core::wearable::Scale { a: *a, b: *b, reference: (pa[axis] - pb[axis]).abs() });
    }
    let (obj_buf, texture, from_glb) = source_mesh(&store, Path::new(need(opts, "obj")?), opts.get("diffuse"))?;
    let obj_path = obj_buf.as_path();
    let garment = gf_io::body::read_obj(obj_path, None)?;
    let world = match opts.get("frame").map(String::as_str).unwrap_or("world") {
        "world" => true,
        "canonical" => false,
        other => return Err(format!("--frame takes world (the character as exported or fitted) or canonical, got {other}")),
    };
    let canonical: Vec<Point> = garment.positions.iter().map(|&p| if world { shaped.similarity.invert(p) } else { p }).collect();
    let mh: Vec<Point> = canonical.iter().map(|&p| frame.to_makehuman(p)).collect();
    let mut binding = gf_core::wearable::bind_to_surface(&mh, &base, &faces, [scales[0], scales[1], scales[2]])?;
    let name = opts.get("name").cloned().unwrap_or_else(|| obj_path.file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "wearable".into()));
    binding.keys.insert("name".into(), name.clone());
    binding.keys.insert("obj_file".into(), obj_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let hide = match opts.get("hide-under").map(String::as_str).unwrap_or("on") {
        "on" => true,
        "off" => false,
        other => return Err(format!("--hide-under takes on or off, got {other}")),
    };
    if hide {
        let tris = triangles_of(&garment.faces);
        binding.hidden = covered_base(&base, &frame.body, &faces, &mh, &tris, frame.unit);
    }
    let compared = match opts.get("compare-hidden") {
        Some(path) => {
            let theirs = Binding::parse(&std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?)?.hidden;
            let (a, b): (std::collections::BTreeSet<u32>, std::collections::BTreeSet<u32>) = (binding.hidden.iter().copied().collect(), theirs.iter().copied().collect());
            let b: std::collections::BTreeSet<u32> = b.into_iter().filter(|v| frame.body.contains_key(v)).collect();
            let both = a.intersection(&b).count();
            json!({"ours": a.len(), "theirs_on_the_body": b.len(), "theirs_all": theirs.len(), "both": both, "only_ours": a.len() - both, "only_theirs": b.len() - both,
                   "jaccard": both as f64 / a.union(&b).count().max(1) as f64,
                   "meaning": "MakeHuman's lists also hide helper geometry (tights, skirt, hair, tongue, eyes); only their body vertices are compared"})
        }
        None => Value::Null,
    };
    let back = frame.evaluate(&binding)?;
    let worst = back.iter().zip(&canonical).map(|(p, q)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()).fold(0.0f64, f64::max);
    let offsets: Vec<f64> = binding.bound.iter().map(|b| (b.offset[0].powi(2) + b.offset[1].powi(2) + b.offset[2].powi(2)).sqrt() * frame.unit * 1000.0).collect();
    if let Some(path) = opts.get("out-mhclo") {
        std::fs::write(path, binding.to_text()).map_err(|e| format!("{path}: {e}"))?;
    }
    let (reference, m) = store.import_generated(&binding, obj_path, texture.as_deref(), "obj", vec![format!("bound to {} on {}", obj_path.display(), spec.name)])?;
    Ok(json!({"ok": true, "command": "wearable bind", "wearable": reference, "name": m.name, "vertices": m.vertices, "transparent": m.transparent,
              "reprojection_max_mm": worst * 1000.0, "offset_max_mm": offsets.iter().copied().fold(0.0, f64::max), "hides_base_vertices": m.hidden, "hidden_compared": compared, "from_glb": from_glb,
              "meaning": "each garment vertex bound to the nearest point of the character's base surface (body and head), with its offset; the reprojection is the binding evaluated back on that character against the garment"}))
}

pub fn import(opts: &Options) -> Result<Value, String> {
    only(opts, "wearable import", &[IMPORT_KEYS])?;
    let store = gf_io::store::Store::open(Path::new(need(opts, "store")?))?;
    let mut rows = Vec::new();
    for file in need(opts, "mhclo")?.split(',') {
        let (reference, m) = store.import_mhclo(Path::new(file))?;
        rows.push(json!({"wearable": reference, "name": m.name, "file": file, "vertices": m.vertices, "hides_base_vertices": m.hidden,
                         "material": m.material.keys().collect::<Vec<_>>(), "transparent": m.transparent, "notes": m.notes}));
    }
    Ok(json!({"ok": true, "command": "wearable import", "wearables": rows,
              "meaning": "each MakeHuman clothes binding with its mesh and material maps, content-addressed in the store; wear one with body.wearables: [{\"asset\": REF, \"hide_body\": true}] in a character spec"}))
}

pub const FIT_KEYS: &[&str] = &["store", "spec", "macro", "mhclo", "out", "hide", "push"];
const INSIDE_TOLERANCE_M: f64 = 0.001;
const COVER_REACH_M: f64 = 0.15;
const COVER_BEHIND_M: f64 = 0.005;

pub struct BaseFrame {
    body: HashMap<u32, Point>,
    helpers: HashMap<u32, Point>,
    head: HashMap<u32, Point>,
    unit: f64,
}

impl BaseFrame {
    fn resolve(&self, v: u32) -> Option<Point> {
        self.head.get(&v).or_else(|| self.body.get(&v)).or_else(|| self.helpers.get(&v)).copied()
    }

    pub fn new(pack: &gf_core::body::BodyPack, controls: &gf_core::body::Controls, pack_manifest: &Value, dense: &[Point]) -> BaseFrame {
        let control = pack.control(controls);
        let control_joint = pack.control_joint(controls);
        let mut head = dense.to_vec();
        gf_core::body::scale_about(&mut head, pack.pivot(), controls.head_scale);
        BaseFrame {
            body: pack.source.iter().enumerate().map(|(i, &o)| (o, control(pack.mesh.positions[i]))).collect(),
            helpers: pack.helpers.source.iter().zip(&pack.helpers.positions).map(|(&o, &p)| (o, control_joint(p))).collect(),
            head: pack.cage_source.iter().enumerate().filter_map(|(d, &o)| head.get(d).map(|p| (o, *p))).collect(),
            unit: pack_unit(pack_manifest),
        }
    }

    pub fn to_makehuman(&self, p: Point) -> Point {
        [p[0] / self.unit, p[2] / self.unit, -p[1] / self.unit]
    }

    pub fn skin_points(&self) -> impl Iterator<Item = (u32, Point)> + '_ {
        self.body.iter().chain(self.head.iter().filter(|(k, _)| !self.body.contains_key(k))).map(|(&k, &p)| (k, p))
    }

    pub fn surface_ids(&self) -> std::collections::BTreeMap<u32, Point> {
        self.body.iter().chain(&self.head).map(|(&k, &p)| (k, self.to_makehuman(p))).collect()
    }

    pub fn evaluate(&self, binding: &Binding) -> Result<Vec<Point>, String> {
        let top = binding.highest_vertex() as usize;
        let mut mh = vec![[0.0; 3]; top + 1];
        let mut missing = Vec::new();
        let used = binding.bound.iter().flat_map(|b| b.vertices).chain(binding.scales.iter().flatten().flat_map(|s| [s.a, s.b]));
        for v in used {
            match self.resolve(v) {
                Some(p) => mh[v as usize] = [p[0] / self.unit, p[2] / self.unit, -p[1] / self.unit],
                None => missing.push(v),
            }
        }
        if !missing.is_empty() {
            missing.sort_unstable();
            missing.dedup();
            return Err(format!("{} base.obj vertices the garment binds to are not on this character (first {:?})", missing.len(), &missing[..missing.len().min(5)]));
        }
        Ok(binding.evaluate(&mh)?.into_iter().map(|q| [q[0] * self.unit, -q[2] * self.unit, q[1] * self.unit]).collect())
    }
}

pub const CLEARANCE_M: f64 = 0.002;
const PUSH_REACH_M: f64 = 0.1;
const PUSH_SPREAD_PASSES: usize = 2;
const LIFT_ROUNDS: usize = 4;
const LIFT_MAX_M: f64 = 0.01;

pub struct Surface {
    pub points: Vec<Point>,
    pub normals: Vec<Point>,
    cell: f64,
    grid: HashMap<(i64, i64, i64), Vec<u32>>,
}

impl Surface {
    pub fn new(points: Vec<Point>, triangles: &[[u32; 3]]) -> Surface {
        let normals = vertex_normals(&points, triangles);
        let cell = 0.02;
        let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
        for (i, p) in points.iter().enumerate() {
            grid.entry(key(*p, cell)).or_default().push(i as u32);
        }
        Surface { points, normals, cell, grid }
    }

    pub fn nearest(&self, p: Point, reach: f64) -> Option<usize> {
        let c = key(p, self.cell);
        let mut best: Option<(f64, usize)> = None;
        let rings = (reach / self.cell).ceil() as i64;
        for r in 0..=rings {
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != r {
                            continue;
                        }
                        for &i in self.grid.get(&(c.0 + dx, c.1 + dy, c.2 + dz)).into_iter().flatten() {
                            let q = self.points[i as usize];
                            let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2);
                            if best.is_none_or(|b| d < b.0) {
                                best = Some((d, i as usize));
                            }
                        }
                    }
                }
            }
            if let Some((d, i)) = best
                && d.sqrt() <= r as f64 * self.cell
            {
                return Some(i);
            }
        }
        best.filter(|b| b.0.sqrt() <= reach).map(|b| b.1)
    }

    pub fn signed(&self, p: Point, j: usize) -> f64 {
        let (q, n) = (self.points[j], self.normals[j]);
        (p[0] - q[0]) * n[0] + (p[1] - q[1]) * n[1] + (p[2] - q[2]) * n[2]
    }
}

fn key(p: Point, cell: f64) -> (i64, i64, i64) {
    ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64, (p[2] / cell).floor() as i64)
}

pub fn push_out(garment: &mut [Point], triangles: &[[u32; 3]], surface: &Surface, clearance: f64) -> Value {
    let mut report = push_out_to(garment, triangles, surface, &vec![Some(clearance); garment.len()], true);
    report["clearance_mm"] = json!(clearance * 1000.0);
    report
}

pub fn clearances(garment: &[Point], surface: &Surface) -> Vec<Option<f64>> {
    garment.iter().map(|&p| surface.nearest(p, PUSH_REACH_M).map(|j| surface.signed(p, j))).collect()
}

pub fn keep_outside(garment: &mut [Point], kept: &[u32], triangles: &[[u32; 3]], surface: &Surface, clear0: &[Option<f64>]) -> Value {
    let target: Vec<Option<f64>> = kept.iter().map(|&k| clear0[k as usize].filter(|&s| s > 0.0).map(|s| s.min(CLEARANCE_M))).collect();
    let mut report = push_out_to(garment, triangles, surface, &target, false);
    let mut lifted = 0usize;
    for _ in 0..LIFT_ROUNDS {
        let mut lift = vec![0.0f64; garment.len()];
        for t in triangles {
            let Some(want) = t.iter().map(|&v| target[v as usize]).try_fold(f64::INFINITY, |m, x| x.map(|x| m.min(x))) else { continue };
            let [a, b, c] = t.map(|v| garment[v as usize]);
            let mid = |p: Point, q: Point| [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0, (p[2] + q[2]) / 2.0];
            let centre = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0];
            let dip = [centre, mid(a, b), mid(b, c), mid(c, a)]
                .into_iter()
                .filter_map(|p| surface.nearest(p, PUSH_REACH_M).map(|j| want - surface.signed(p, j)))
                .fold(0.0f64, f64::max);
            if dip > 0.0 && dip <= LIFT_MAX_M {
                for &v in t {
                    lift[v as usize] = lift[v as usize].max(dip);
                }
            }
        }
        let moved: Vec<usize> = (0..garment.len()).filter(|&v| lift[v] > 0.0).collect();
        if moved.is_empty() {
            break;
        }
        lifted += moved.len();
        for v in moved {
            if let Some(j) = surface.nearest(garment[v], PUSH_REACH_M) {
                let n = surface.normals[j];
                for k in 0..3 {
                    garment[v][k] += lift[v] * n[k];
                }
            }
        }
    }
    report["lifted"] = json!(lifted);
    report
}

fn push_out_to(garment: &mut [Point], triangles: &[[u32; 3]], surface: &Surface, target: &[Option<f64>], skip_inner: bool) -> Value {
    let normals = vertex_normals(garment, triangles);
    let mut push = vec![0.0f64; garment.len()];
    let mut dir = vec![[0.0f64; 3]; garment.len()];
    let mut inner = 0usize;
    for (i, p) in garment.iter().enumerate() {
        let Some(want) = target[i] else { continue };
        let Some(j) = surface.nearest(*p, PUSH_REACH_M) else { continue };
        let s = surface.signed(*p, j);
        if s >= want {
            continue;
        }
        let n = surface.normals[j];
        if skip_inner && normals[i][0] * n[0] + normals[i][1] * n[1] + normals[i][2] * n[2] <= 0.0 {
            inner += 1;
            continue;
        }
        push[i] = want - s;
        dir[i] = n;
    }
    let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); garment.len()];
    for t in triangles {
        for k in 0..3 {
            neighbours[t[k] as usize].push(t[(k + 1) % 3]);
            neighbours[t[k] as usize].push(t[(k + 2) % 3]);
        }
    }
    for _ in 0..PUSH_SPREAD_PASSES {
        let before = push.clone();
        for i in 0..garment.len() {
            if before[i] > 0.0 || neighbours[i].is_empty() {
                continue;
            }
            let (sum, from) = neighbours[i].iter().fold((0.0, None::<usize>), |(s, f), &k| (s + before[k as usize], if before[k as usize] > 0.0 { Some(k as usize) } else { f }));
            if let Some(f) = from {
                push[i] = 0.5 * sum / neighbours[i].len() as f64;
                dir[i] = dir[f];
            }
        }
    }
    let moved = push.iter().filter(|&&d| d > 0.0).count();
    let max = push.iter().copied().fold(0.0f64, f64::max);
    for (p, (d, n)) in garment.iter_mut().zip(push.iter().zip(&dir)) {
        for k in 0..3 {
            p[k] += d * n[k];
        }
    }
    json!({"moved": moved, "max_mm": max * 1000.0, "inner_faces_left": inner})
}

pub struct Layered<'a> {
    pub positions: &'a mut Vec<Point>,
    pub triangles: Vec<[u32; 3]>,
    pub depth: f64,
    pub opaque: bool,
}

pub fn depth_of(binding: &Binding) -> f64 {
    binding.keys.get("z_depth").and_then(|z| z.parse().ok()).unwrap_or(50.0)
}

pub fn layer(garments: &mut [Layered]) -> Vec<Value> {
    let mut order: Vec<usize> = (0..garments.len()).collect();
    order.sort_by(|&a, &b| garments[a].depth.total_cmp(&garments[b].depth));
    let mut reports = vec![Value::Null; garments.len()];
    for &g in &order {
        if !garments[g].opaque {
            continue;
        }
        let inner: Vec<usize> = order.iter().copied().filter(|&k| garments[k].opaque && garments[k].depth < garments[g].depth).collect();
        if inner.is_empty() {
            continue;
        }
        let mut points: Vec<Point> = Vec::new();
        let mut tris: Vec<[u32; 3]> = Vec::new();
        for &k in &inner {
            let offset = points.len() as u32;
            points.extend_from_slice(garments[k].positions);
            tris.extend(garments[k].triangles.iter().map(|t| t.map(|v| v + offset)));
        }
        let surface = Surface::new(points, &tris);
        let triangles = garments[g].triangles.clone();
        let mut report = push_out(garments[g].positions, &triangles, &surface, CLEARANCE_M);
        report["over"] = json!(inner.len());
        reports[g] = report;
    }
    reports
}

pub fn triangles_of(faces: &[Vec<(u32, Option<u32>)>]) -> Vec<[u32; 3]> {
    faces.iter().flat_map(|f| (1..f.len().saturating_sub(1)).map(move |k| [f[0].0, f[k].0, f[k + 1].0])).collect()
}

pub fn hidden_body(pack: &gf_core::body::BodyPack, lod: usize, st: &gf_core::body::Stitched, covered: &HashSet<u32>) -> Result<Vec<bool>, String> {
    let (_, kept) = pack.lod_mesh(lod)?;
    let cage = |c: u32| pack.source.get(c as usize).is_some_and(|o| covered.contains(o));
    st.source
        .iter()
        .map(|s| {
            let Some(u) = s else { return Ok(false) };
            let k = *kept.get(*u as usize).ok_or_else(|| format!("body vertex {u} is outside LOD{lod}"))?;
            Ok(match &pack.fine {
                Some(f) => f.stencil.rows[k as usize].iter().all(|&(c, _)| cage(c)),
                None => cage(k),
            })
        })
        .collect()
}

pub fn vertex_normals(positions: &[Point], triangles: &[[u32; 3]]) -> Vec<Point> {
    let mut n = vec![[0.0; 3]; positions.len()];
    for t in triangles {
        let (a, b, c) = (positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]);
        let (u, v) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
        let f = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        for &k in t {
            for d in 0..3 {
                n[k as usize][d] += f[d];
            }
        }
    }
    n.into_iter().map(|p| {
        let l = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt().max(1e-30);
        p.map(|x| x / l)
    }).collect()
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(u: Point, v: Point) -> Point {
    [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
}

fn dot(u: Point, v: Point) -> f64 {
    u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
}

fn normalized(u: Point) -> Point {
    let l = dot(u, u).sqrt().max(1e-30);
    u.map(|x| x / l)
}

fn ray_hits(origin: Point, dir: Point, t: &[Point; 3]) -> Option<f64> {
    let (e1, e2) = (sub(t[1], t[0]), sub(t[2], t[0]));
    let p = cross(dir, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-14 {
        return None;
    }
    let s = sub(origin, t[0]);
    let u = dot(s, p) / det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(dir, q) / det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(dot(e2, q) / det)
}


pub fn fit(opts: &Options) -> Result<Value, String> {
    only(opts, "wearable fit", &[FIT_KEYS])?;
    let root = need(opts, "store")?;
    let store = gf_io::store::Store::open(Path::new(root))?;
    let spec = ch::read_spec(Path::new(need(opts, "spec")?))?;
    let source = opts.get("macro").map(|p| ch::read_macro_definition(Path::new(p))).transpose()?;
    let (dense, _, _, pin) = ch::evaluate_traced(&store, &spec, source.as_ref())?;
    let shaped = crate::body::spec_body(&store, &spec, source.as_ref(), pin.as_ref())?;
    let pack = &shaped.pack;
    let game_ref = spec.base.game_base.clone().ok_or("the spec names no game base")?;
    let (_, base) = store.load_game(&game_ref)?;
    let mut head: Vec<Point> = base.kept.iter().map(|&d| dense[d as usize]).collect();
    gf_core::body::scale_about(&mut head, pack.pivot(), shaped.controls.head_scale);
    let seam = gf_core::body::head_seam(pack, &head, &base.triangles, &base.kept)?;
    let st = gf_core::body::stitch_lod(pack, 0, &seam, &shaped.controls)?;
    let mut surface: Vec<Point> = head.clone();
    surface.extend_from_slice(&st.positions);
    let h = head.len() as u32;
    let surface_triangles: Vec<[u32; 3]> = base.triangles.iter().copied().chain(st.triangles.iter().map(|t| t.map(|v| v + h))).collect();
    let surf = Surface::new(surface.clone(), &surface_triangles);
    let push = match opts.get("push").map(String::as_str).unwrap_or("on") {
        "on" => true,
        "off" => false,
        other => return Err(format!("--push takes on or off, got {other}")),
    };
    let frame = BaseFrame::new(pack, &shaped.controls, &shaped.manifest, &dense);
    let world = |p: Point| shaped.similarity.apply(p);
    let out = Path::new(need(opts, "out")?);
    let mut obj = String::from("o body\n");
    for p in head.iter().chain(&st.positions) {
        let w = world(*p);
        obj.push_str(&format!("v {} {} {}\n", w[0], w[1], w[2]));
    }
    for t in &base.triangles {
        obj.push_str(&format!("f {} {} {}\n", t[0] + 1, t[1] + 1, t[2] + 1));
    }
    let hide = match opts.get("hide").map(String::as_str).unwrap_or("on") {
        "on" => true,
        "off" => false,
        other => return Err(format!("--hide takes on or off, got {other}")),
    };
    let mut garments = String::new();
    let mut garment_triangles: Vec<[Point; 3]> = Vec::new();
    let mut covered: HashSet<u32> = HashSet::new();
    let mut offset = head.len() + st.positions.len();
    let mut rows = Vec::new();
    for file in need(opts, "mhclo")?.split(',') {
        let path = Path::new(file);
        let text = std::fs::read_to_string(path).map_err(|e| format!("{file}: {e}"))?;
        let binding = Binding::parse(&text).map_err(|e| format!("{file}: {e}"))?;
        let mut positions = frame.evaluate(&binding).map_err(|e| format!("{file}: {e}"))?;
        let mesh_path = path.parent().unwrap_or(Path::new(".")).join(binding.keys.get("obj_file").ok_or_else(|| format!("{file}: no obj_file line"))?);
        let faces = gf_io::body::read_obj(&mesh_path, None)?.faces;
        let pushed = push.then(|| push_out(&mut positions, &triangles_of(&faces), &surf, CLEARANCE_M));
        let mut depths = Vec::new();
        let mut deepest = (0.0, [0.0; 3]);
        let garment_normals = vertex_normals(&positions, &triangles_of(&faces));
        let mut facing_out = 0usize;
        for (i, p) in positions.iter().enumerate() {
            let Some(j) = surf.nearest(*p, 0.1) else { continue };
            let signed = surf.signed(*p, j);
            if signed < -INSIDE_TOLERANCE_M {
                let n = surf.normals[j];
                if garment_normals[i][0] * n[0] + garment_normals[i][1] * n[1] + garment_normals[i][2] * n[2] > 0.0 {
                    facing_out += 1;
                }
                depths.push(-signed * 1000.0);
                if -signed > deepest.0 {
                    deepest = (-signed, world(*p));
                }
            }
        }
        depths.sort_by(f64::total_cmp);
        let name = binding.keys.get("name").cloned().unwrap_or_else(|| file.to_string());
        garments.push_str(&format!("o {name}\n"));
        for p in &positions {
            let w = world(*p);
            garments.push_str(&format!("v {} {} {}\n", w[0], w[1], w[2]));
        }
        for f in &faces {
            let ids: Vec<String> = f.iter().map(|c| (c.0 as usize + offset + 1).to_string()).collect();
            garments.push_str(&format!("f {}\n", ids.join(" ")));
        }
        covered.extend(&binding.hidden);
        for f in &faces {
            for k in 1..f.len().saturating_sub(1) {
                garment_triangles.push([positions[f[0].0 as usize], positions[f[k].0 as usize], positions[f[k + 1].0 as usize]]);
            }
        }
        offset += positions.len();
        rows.push(json!({"name": name, "file": file, "vertices": positions.len(), "faces": faces.len(), "hides_base_vertices": binding.hidden.len(), "pushed": pushed,
                         "inside_body": {"vertices": depths.len(), "facing_out": facing_out, "share": depths.len() as f64 / positions.len() as f64,
                                         "max_mm": depths.last().copied().unwrap_or(0.0), "p95_mm": depths.get((depths.len() as f64 * 0.95) as usize).copied().unwrap_or(0.0),
                                         "deepest_world": deepest.1}}));
    }
    let hidden = if hide { hidden_body(pack, 0, &st, &covered)? } else { vec![false; st.positions.len()] };
    let mut dropped = 0;
    let mut uncovered: Vec<[u32; 3]> = Vec::new();
    for t in &st.triangles {
        if t.iter().all(|&v| hidden[v as usize]) {
            dropped += 1;
            let [a, b, c] = t.map(|v| st.positions[v as usize]);
            let centre = [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0];
            let n = normalized(cross(sub(b, a), sub(c, a)));
            if !garment_triangles.iter().any(|g| ray_hits(centre, n, g).is_some_and(|d| (-COVER_BEHIND_M..=COVER_REACH_M).contains(&d))) {
                uncovered.push(*t);
            }
            continue;
        }
        obj.push_str(&format!("f {} {} {}\n", t[0] + h + 1, t[1] + h + 1, t[2] + h + 1));
    }
    obj.push_str(&garments);
    if !uncovered.is_empty() {
        obj.push_str("o uncovered\n");
        for t in &uncovered {
            obj.push_str(&format!("f {} {} {}\n", t[0] + h + 1, t[1] + h + 1, t[2] + h + 1));
        }
    }
    std::fs::write(out, obj).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(json!({"ok": true, "command": "wearable fit", "out": out.display().to_string(),
              "body": {"vertices": st.positions.len(), "head_vertices": head.len(), "triangles": st.triangles.len(), "hidden_triangles": dropped, "hide": hide,
                       "hidden_uncovered": {"triangles": uncovered.len(), "object": "uncovered", "reach_mm": COVER_REACH_M * 1000.0,
                                            "meaning": "a left-out body triangle whose outward normal line from its centroid meets no garment triangle within the reach (or just behind it): a hole where the body is hidden and nothing covers it"},
                       "hide_meaning": "a body triangle is left out when all three corners are covered: every cage vertex a corner is refined from is in some garment's hidden list"},
              "garments": rows, "frame": "world: metres, z up, the character's root similarity applied",
              "inside_meaning": format!("garment vertices more than {} mm on the inner side of the nearest vertex of the stitched LOD0 character (head and body), along that vertex's normal", INSIDE_TOLERANCE_M * 1000.0)}))
}

fn pack_unit(manifest: &Value) -> f64 {
    manifest["source"]["unit_m"].as_f64().unwrap_or(0.1)
}

fn shaped_base(opts: &Options, base: &mut [Point]) -> Result<Value, String> {
    let (Some(def), Some(root), Some(sliders)) = (opts.get("macro"), opts.get("targets"), opts.get("sliders")) else {
        if opts.contains_key("macro") || opts.contains_key("sliders") {
            return Err("--macro, --targets and --sliders go together (the base is shaped by MakeHuman's macro targets)".into());
        }
        return Ok(Value::Null);
    };
    let source = ch::read_macro_definition(Path::new(def))?;
    let root = Path::new(root);
    let files = gf_io::macro_target_files(root)?;
    let sliders = number_map(&json_arg(sliders, "sliders")?, "sliders")?;
    let weights = macros::mpfb_keys(&macros::target_weights(&source.definition, &sliders, Some(&|n: &str| files.contains(n)), Mode::Mpfb)?, macros::KEY_FLOOR);
    let mut applied = 0;
    let count = base.len();
    for (id, w) in &weights {
        let path = [".target.gz", ".target"].iter().map(|ext| root.join(format!("{id}{ext}"))).find(|p| p.is_file()).ok_or_else(|| format!("no target file for {id} under {}", root.display()))?;
        for (v, d) in gf_io::body::read_target(&path)? {
            let p = base.get_mut(v as usize).ok_or_else(|| format!("{id} moves vertex {v}, the base has {count}"))?;
            for axis in 0..3 {
                p[axis] += w * d[axis];
            }
        }
        applied += 1;
    }
    Ok(json!({"sliders": sliders, "targets": applied}))
}

fn distances(a: &[Point], b: &[Point], shift: Point) -> Value {
    let mut d: Vec<f64> = a.iter().zip(b).map(|(p, q)| ((p[0] + shift[0] - q[0]).powi(2) + (p[1] + shift[1] - q[1]).powi(2) + (p[2] + shift[2] - q[2]).powi(2)).sqrt()).collect();
    d.sort_by(f64::total_cmp);
    let n = d.len();
    json!({"max": d[n - 1], "p99": d[((n - 1) as f64 * 0.99).round() as usize], "mean": d.iter().sum::<f64>() / n as f64})
}

pub fn check(opts: &Options) -> Result<Value, String> {
    only(opts, "wearable check", &[CHECK_KEYS])?;
    let mut base = gf_io::body::read_obj(Path::new(need(opts, "base")?), None)?.positions;
    let shaped = shaped_base(opts, &mut base)?;
    let mut rows = Vec::new();
    for file in need(opts, "mhclo")?.split(',') {
        let path = Path::new(file);
        let text = std::fs::read_to_string(path).map_err(|e| format!("{file}: {e}"))?;
        let binding = Binding::parse(&text).map_err(|e| format!("{file}: {e}"))?;
        let positions = binding.evaluate(&base).map_err(|e| format!("{file}: {e}"))?;
        let obj = binding.keys.get("obj_file").ok_or_else(|| format!("{file}: no obj_file line"))?;
        let garment = gf_io::body::read_obj(&path.parent().unwrap_or(Path::new(".")).join(obj), None)?.positions;
        let mut row = json!({"file": file, "vertices": positions.len(), "hidden": binding.hidden.len(), "scales": binding.axis_scales(&base)});
        if garment.len() != positions.len() {
            row["error"] = json!(format!("the binding has {} vertices, {obj} has {}", positions.len(), garment.len()));
        } else {
            let n = positions.len() as f64;
            let mut shift = [0.0; 3];
            for (p, q) in positions.iter().zip(&garment) {
                for axis in 0..3 {
                    shift[axis] += (q[axis] - p[axis]) / n;
                }
            }
            row["as_bound"] = distances(&positions, &garment, [0.0; 3]);
            row["mean_shift"] = json!(shift);
            row["after_shift"] = distances(&positions, &garment, shift);
        }
        rows.push(row);
    }
    Ok(json!({"ok": true, "command": "wearable check", "base_vertices": base.len(), "shaped": shaped, "files": rows,
              "units": "the base's (MakeHuman: decimetres)",
              "meaning": "each binding evaluated on the base (shaped by the macro targets when given) against its own .obj: as bound, and after removing the mean translation (MakeHuman exports some garments standing on the floor)"}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_hold_near_the_body_and_free_far_from_it() {
        assert_eq!(pin_weight(-0.01), 1.0);
        assert_eq!(pin_weight(PIN_NEAR_M), 1.0);
        assert_eq!(pin_weight(PIN_FAR_M), 0.0);
        assert_eq!(pin_weight(0.2), 0.0);
        let mid = pin_weight((PIN_NEAR_M + PIN_FAR_M) / 2.0);
        assert!((mid - 0.5).abs() < 1e-6, "{mid}");
        let mut last = 1.0;
        for k in 0..=50 {
            let w = pin_weight(PIN_NEAR_M + (PIN_FAR_M - PIN_NEAR_M) * k as f64 / 50.0);
            assert!(w <= last + 1e-7, "falls monotonically");
            last = w;
        }
    }

    fn dome() -> Surface {
        let h = |x: f64| 0.03 - 3.0 * x * x;
        let n = 41;
        let points: Vec<Point> = (0..n * n).map(|i| {
            let (x, y) = (-0.1 + 0.005 * (i % n) as f64, -0.1 + 0.005 * (i / n) as f64);
            [x, y, h(x)]
        }).collect();
        let mut tris = Vec::new();
        for r in 0..n - 1 {
            for c in 0..n - 1 {
                let a = (r * n + c) as u32;
                tris.push([a, a + 1, a + n as u32]);
                tris.push([a + 1, a + n as u32 + 1, a + n as u32]);
            }
        }
        Surface::new(points, &tris)
    }

    fn lowest_sample(garment: &[Point], surface: &Surface) -> f64 {
        let [a, b, c] = [garment[0], garment[1], garment[2]];
        let mid = |p: Point, q: Point| [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0, (p[2] + q[2]) / 2.0];
        [mid(a, b), mid(b, c), mid(c, a), [(a[0] + b[0] + c[0]) / 3.0, (a[1] + b[1] + c[1]) / 3.0, (a[2] + b[2] + c[2]) / 3.0]]
            .into_iter()
            .map(|p| surface.signed(p, surface.nearest(p, 0.1).unwrap()))
            .fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn a_coarse_cap_triangle_sagging_under_the_head_is_lifted_clear_but_a_deep_chord_is_left() {
        let surface = dome();
        let h = |x: f64| 0.03 - 3.0 * x * x;
        let cap = |w: f64| vec![[-w, -0.01, h(w) + 0.003], [w, -0.01, h(w) + 0.003], [0.0, 0.02, h(0.0) + 0.003]];
        let mut shallow = cap(0.05);
        let clear0 = clearances(&shallow, &surface);
        assert!(clear0.iter().all(|c| c.is_some_and(|s| s > 0.0)), "every corner starts outside: {clear0:?}");
        assert!(lowest_sample(&shallow, &surface) < -0.003, "the chord dips under the dome before the lift");
        let report = keep_outside(&mut shallow, &[0, 1, 2], &[[0, 1, 2]], &surface, &clear0);
        assert!(report["lifted"].as_u64().unwrap() > 0);
        assert!(lowest_sample(&shallow, &surface) > -0.0005, "lifted clear: {}", lowest_sample(&shallow, &surface));
        let mut deep = cap(0.09);
        let before = deep.clone();
        let clear0 = clearances(&deep, &surface);
        keep_outside(&mut deep, &[0, 1, 2], &[[0, 1, 2]], &surface, &clear0);
        assert_eq!(deep, before, "a chord deeper than the lift cap is a decimation limit, not something a lift should hide");
        let mut rooted = cap(0.05);
        rooted[0][2] = h(0.05) - 0.002;
        let before = rooted.clone();
        let clear0 = clearances(&rooted, &surface);
        keep_outside(&mut rooted, &[0, 1, 2], &[[0, 1, 2]], &surface, &clear0);
        assert_eq!(rooted, before, "a card rooted in the skin at full detail (a beard) is not lifted");
    }

    #[test]
    fn a_hanging_strip_becomes_one_chain_from_its_pinned_top() {
        let mut garment = Vec::new();
        for row in 0..21 {
            for col in 0..3 {
                garment.push([col as f64 * 0.01, 0.0, -(row as f64) * 0.01]);
            }
        }
        let mut tris = Vec::new();
        for row in 0..20u32 {
            for col in 0..2u32 {
                let a = row * 3 + col;
                tris.push([a, a + 1, a + 3]);
                tris.push([a + 1, a + 4, a + 3]);
            }
        }
        let top: Vec<bool> = (0..garment.len()).map(|v| v < 3).collect();
        let c = build_chains(&garment, &tris, &top);
        assert_eq!(c.chains.len(), 1);
        assert_eq!(c.chains[0].joints.len(), CHAIN_SEGMENTS + 1);
        let z: Vec<f64> = c.chains[0].joints.iter().map(|j| j[2]).collect();
        assert!(z.windows(2).all(|w| w[1] < w[0]), "joints run down the strip: {z:?}");
        assert!(c.along[..3].iter().all(Option::is_none), "pinned vertices are not on the chain");
        let (chain, t) = c.along[60].unwrap();
        assert_eq!(chain, 0);
        assert!((t - CHAIN_SEGMENTS as f64).abs() < 1e-9, "the bottom row is at the chain's tip");
        assert!(build_chains(&garment, &tris, &vec![true; garment.len()]).chains.is_empty(), "nothing free, no chains");
        let stub: Vec<bool> = (0..garment.len()).map(|v| v < 60).collect();
        let s = build_chains(&garment, &tris, &stub);
        assert!(s.chains.is_empty() && s.along.iter().all(Option::is_none), "a 1 cm flap is too short to swing");
    }

    #[test]
    fn only_the_anchor_bone_roots_a_chain_not_what_the_hair_rests_on() {
        let garment: Vec<Point> = (0..40).map(|v| [0.0, 0.0, -(v as f64) * 0.01]).collect();
        let pin: Vec<f32> = (0..40).map(|v| if !(4..30).contains(&v) { 1.0 } else { 0.0 }).collect();
        let dominant: Vec<usize> = (0..40).map(|v| if v < 10 { 7 } else { 3 }).collect();
        let (seed, anchor) = spring_seeds(&garment, &pin, &dominant);
        assert_eq!(anchor, Some(7));
        assert!(seed[..4].iter().all(|&s| s), "the scalp roots the chain");
        assert!(seed[4..].iter().all(|&s| !s), "the hair lying on the back is free, not a root");
        let (none, no_anchor) = spring_seeds(&garment, &[0.0; 40], &dominant);
        assert!(none.iter().all(|&s| !s) && no_anchor.is_none(), "nothing pinned, nothing rooted");
    }

    #[test]
    fn strands_hanging_side_by_side_become_one_chain_not_interleaved_ones() {
        let mut garment = Vec::new();
        for row in 0..401 {
            for col in 0..3 {
                garment.push([col as f64 * 0.01, 0.0, -(row as f64) * 0.001]);
            }
        }
        let mut tris = Vec::new();
        for row in 0..400u32 {
            for col in 0..2u32 {
                let a = row * 3 + col;
                tris.push([a, a + 1, a + 3]);
                tris.push([a + 1, a + 4, a + 3]);
            }
        }
        let top: Vec<bool> = (0..garment.len()).map(|v| v < 3).collect();
        let c = build_chains(&garment, &tris, &top);
        assert_eq!(c.chains.len(), 1, "one tail, one chain");
        assert!(c.along[3..].iter().all(|a| a.is_some_and(|(chain, _)| chain == 0)));
    }

    #[test]
    fn a_detached_card_joins_the_chain_it_hangs_beside() {
        let mut garment = Vec::new();
        for strip in 0..2 {
            for row in 0..21 {
                for col in 0..3 {
                    garment.push([col as f64 * 0.01 + strip as f64 * 0.005, 0.002 * strip as f64, -(row as f64) * 0.01]);
                }
            }
        }
        let mut tris = Vec::new();
        for strip in 0..2u32 {
            for row in 0..20u32 {
                for col in 0..2u32 {
                    let a = strip * 63 + row * 3 + col;
                    tris.push([a, a + 1, a + 3]);
                    tris.push([a + 1, a + 4, a + 3]);
                }
            }
        }
        let top: Vec<bool> = (0..garment.len()).map(|v| v < 3).collect();
        let c = build_chains(&garment, &tris, &top);
        assert_eq!(c.chains.len(), 1);
        let t: Vec<f64> = (63..126).step_by(3).map(|v| c.along[v].expect("the detached card is on the chain").1).collect();
        assert!(t.windows(2).all(|w| w[1] >= w[0]) && t[t.len() - 1] > t[0] + 2.0, "it bends further down the chain the lower it hangs: {t:?}");
    }

    #[test]
    fn a_bone_frame_round_trips_points_and_a_segment_distance_is_exact() {
        let bf = BoneFrame { origin: [0.1, 0.2, 1.0], axes: [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]], length: 0.25 };
        let p = [0.3, -0.1, 1.4];
        let q = bf.from_local(bf.to_local(p));
        assert!((0..3).all(|k| (p[k] - q[k]).abs() < 1e-12));
        assert!((segment_distance([1.0, 1.0, 0.0], [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]) - 1.0).abs() < 1e-12);
        assert!((segment_distance([3.0, 0.0, 0.0], [0.0, 0.0, 0.0], [2.0, 0.0, 0.0]) - 1.0).abs() < 1e-12, "past the tail: to the end point");
    }
}
