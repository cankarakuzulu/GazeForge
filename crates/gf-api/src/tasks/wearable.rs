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
pub const FIT_KEYS: &[&str] = &["store", "spec", "macro", "mhclo", "out", "hide"];
const INSIDE_TOLERANCE_M: f64 = 0.001;

struct BaseFrame {
    body: HashMap<u32, Point>,
    helpers: HashMap<u32, Point>,
    head: HashMap<u32, Point>,
    unit: f64,
}

impl BaseFrame {
    fn resolve(&self, v: u32) -> Option<Point> {
        self.head.get(&v).or_else(|| self.body.get(&v)).or_else(|| self.helpers.get(&v)).copied()
    }

    fn evaluate(&self, binding: &Binding) -> Result<Vec<Point>, String> {
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

fn hidden_body(pack: &gf_core::body::BodyPack, st: &gf_core::body::Stitched, covered: &HashSet<u32>) -> Result<Vec<bool>, String> {
    let (_, kept) = pack.lod_mesh(0)?;
    let cage = |c: u32| pack.source.get(c as usize).is_some_and(|o| covered.contains(o));
    st.source
        .iter()
        .map(|s| {
            let Some(u) = s else { return Ok(false) };
            let k = *kept.get(*u as usize).ok_or_else(|| format!("body vertex {u} is outside LOD0"))?;
            Ok(match &pack.fine {
                Some(f) => f.stencil.rows[k as usize].iter().all(|&(c, _)| cage(c)),
                None => cage(k),
            })
        })
        .collect()
}

fn vertex_normals(positions: &[Point], triangles: &[[u32; 3]]) -> Vec<Point> {
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

fn nearest(points: &[Point], p: Point) -> usize {
    let d = |q: &Point| (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2);
    (0..points.len()).min_by(|&a, &b| d(&points[a]).total_cmp(&d(&points[b]))).unwrap_or(0)
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
    let normals = vertex_normals(&surface, &surface_triangles);
    let control = pack.control(&shaped.controls);
    let control_joint = pack.control_joint(&shaped.controls);
    let mut dense_head = dense.clone();
    gf_core::body::scale_about(&mut dense_head, pack.pivot(), shaped.controls.head_scale);
    let unit = pack_unit(&shaped.manifest);
    let frame = BaseFrame {
        body: pack.source.iter().enumerate().map(|(i, &o)| (o, control(pack.mesh.positions[i]))).collect(),
        helpers: pack.helpers.source.iter().zip(&pack.helpers.positions).map(|(&o, &p)| (o, control_joint(p))).collect(),
        head: pack.cage_source.iter().enumerate().map(|(d, &o)| (o, dense_head[d])).collect(),
        unit,
    };
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
    let mut covered: HashSet<u32> = HashSet::new();
    let mut offset = head.len() + st.positions.len();
    let mut rows = Vec::new();
    for file in need(opts, "mhclo")?.split(',') {
        let path = Path::new(file);
        let text = std::fs::read_to_string(path).map_err(|e| format!("{file}: {e}"))?;
        let binding = Binding::parse(&text).map_err(|e| format!("{file}: {e}"))?;
        let positions = frame.evaluate(&binding).map_err(|e| format!("{file}: {e}"))?;
        let mesh_path = path.parent().unwrap_or(Path::new(".")).join(binding.keys.get("obj_file").ok_or_else(|| format!("{file}: no obj_file line"))?);
        let faces = gf_io::body::read_obj(&mesh_path, None)?.faces;
        let mut depths = Vec::new();
        let mut deepest = (0.0, [0.0; 3]);
        for p in &positions {
            let j = nearest(&surface, *p);
            let (q, n) = (surface[j], normals[j]);
            let signed = (p[0] - q[0]) * n[0] + (p[1] - q[1]) * n[1] + (p[2] - q[2]) * n[2];
            if signed < -INSIDE_TOLERANCE_M {
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
        offset += positions.len();
        rows.push(json!({"name": name, "file": file, "vertices": positions.len(), "faces": faces.len(), "hides_base_vertices": binding.hidden.len(),
                         "inside_body": {"vertices": depths.len(), "share": depths.len() as f64 / positions.len() as f64,
                                         "max_mm": depths.last().copied().unwrap_or(0.0), "p95_mm": depths.get((depths.len() as f64 * 0.95) as usize).copied().unwrap_or(0.0),
                                         "deepest_world": deepest.1}}));
    }
    let hidden = if hide { hidden_body(pack, &st, &covered)? } else { vec![false; st.positions.len()] };
    let mut dropped = 0;
    for t in &st.triangles {
        if t.iter().all(|&v| hidden[v as usize]) {
            dropped += 1;
            continue;
        }
        obj.push_str(&format!("f {} {} {}\n", t[0] + h + 1, t[1] + h + 1, t[2] + h + 1));
    }
    obj.push_str(&garments);
    std::fs::write(out, obj).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(json!({"ok": true, "command": "wearable fit", "out": out.display().to_string(),
              "body": {"vertices": st.positions.len(), "head_vertices": head.len(), "triangles": st.triangles.len(), "hidden_triangles": dropped, "hide": hide,
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
