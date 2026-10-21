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
        (None, Some((mime, _))) => (None, format!("the glb's colour texture is {mime}, which is not read yet: give a PNG with diffuse")),
        (None, None) => (None, "the glb has no colour texture".to_string()),
    };
    Ok((obj, texture, json!({"glb": path.display().to_string(), "vertices": scene.positions.len(), "triangles": scene.triangles.len(), "texture": note,
                             "frame": "glTF y up converted to z up; node transforms applied"})))
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
    let normals = vertex_normals(garment, triangles);
    let mut push = vec![0.0f64; garment.len()];
    let mut dir = vec![[0.0f64; 3]; garment.len()];
    let mut inner = 0usize;
    for (i, p) in garment.iter().enumerate() {
        let Some(j) = surface.nearest(*p, PUSH_REACH_M) else { continue };
        let s = surface.signed(*p, j);
        if s >= clearance {
            continue;
        }
        let n = surface.normals[j];
        if normals[i][0] * n[0] + normals[i][1] * n[1] + normals[i][2] * n[2] <= 0.0 {
            inner += 1;
            continue;
        }
        push[i] = clearance - s;
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
    json!({"moved": moved, "max_mm": max * 1000.0, "inner_faces_left": inner, "clearance_mm": clearance * 1000.0})
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
