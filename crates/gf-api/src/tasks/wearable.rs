use std::collections::HashMap;
use std::path::Path;

use gf_core::game_mesh::Point;
use gf_core::macros::{self, Mode};
use gf_core::wearable::Binding;
use gf_io::character as ch;
use serde_json::{Value, json};

use super::data::{json_arg, number_map};
use super::{Options, need, only};

pub const CHECK_KEYS: &[&str] = &["base", "mhclo", "macro", "targets", "sliders"];
pub const FIT_KEYS: &[&str] = &["store", "spec", "macro", "mhclo", "out"];
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
    let (_, normals) = gf_core::body::two_sided_normals(&head, &base.triangles, &st);
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
    let h = head.len() as u32;
    for t in &st.triangles {
        obj.push_str(&format!("f {} {} {}\n", t[0] + h + 1, t[1] + h + 1, t[2] + h + 1));
    }
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
            let j = nearest(&st.positions, *p);
            let (q, n) = (st.positions[j], normals[j]);
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
        obj.push_str(&format!("o {name}\n"));
        for p in &positions {
            let w = world(*p);
            obj.push_str(&format!("v {} {} {}\n", w[0], w[1], w[2]));
        }
        for f in &faces {
            let ids: Vec<String> = f.iter().map(|c| (c.0 as usize + offset + 1).to_string()).collect();
            obj.push_str(&format!("f {}\n", ids.join(" ")));
        }
        offset += positions.len();
        rows.push(json!({"name": name, "file": file, "vertices": positions.len(), "faces": faces.len(), "hides_base_vertices": binding.hidden.len(),
                         "inside_body": {"vertices": depths.len(), "share": depths.len() as f64 / positions.len() as f64,
                                         "max_mm": depths.last().copied().unwrap_or(0.0), "p95_mm": depths.get((depths.len() as f64 * 0.95) as usize).copied().unwrap_or(0.0),
                                         "deepest_world": deepest.1}}));
    }
    std::fs::write(out, obj).map_err(|e| format!("{}: {e}", out.display()))?;
    Ok(json!({"ok": true, "command": "wearable fit", "out": out.display().to_string(), "body": {"vertices": st.positions.len(), "head_vertices": head.len()},
              "garments": rows, "frame": "world: metres, z up, the character's root similarity applied",
              "inside_meaning": format!("garment vertices more than {} mm on the inner side of the nearest stitched LOD0 body vertex's normal (a garment vertex near the head is measured against the body all the same)", INSIDE_TOLERANCE_M * 1000.0)}))
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
