use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use gf_core::skin_layers::{self as sl, SkinSpec};
use gf_core::skin_texture::{self as st, SkinLook};
use gf_core::{game_mesh, skin_map, skin_regions};
use gf_geom::decimate::Decimated;
use gf_io::gltf;
use gf_io::store::Store;
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{obj, shape, text};
use crate::error::ApiError;

pub const DEFAULT_SIZE: usize = 2048;
const PAD: usize = 8;

pub fn models_input() -> Value {
    obj(json!({}), &[])
}

pub fn models_output() -> Value {
    shape(json!({"default": {"type": "string"}, "looks": {"type": "array"}}))
}

pub fn models(_: &Ctx, _: &Value) -> Result<Value, ApiError> {
    Ok(json!({"default": st::DEFAULT_LOOK, "looks": st::looks()}))
}

pub fn generate_input() -> Value {
    obj(
        json!({"store": {"type": "string", "minLength": 1},
               "game": {"type": "string", "minLength": 1, "description": "the game base whose UVs the textures follow"},
               "look": {"type": "string", "minLength": 1, "description": "a preset of skin.models; default medium-warm"},
               "skin": {"type": "object", "description": "fields that override the preset's (melanin, haemoglobin, undertone, redness, lip_tint, under_eye, freckles, freckle_size_mm, moles, pores, age, stubble, hair, oil, seed)"},
               "landmarks": {"type": "string", "minLength": 1, "description": "the base's landmark table (base_landmarks.json: MediaPipe landmark -> dense vertex)"},
               "size": {"type": "integer", "minimum": 256, "maximum": 8192, "description": "texture edge in pixels, default 2048"},
               "regions": {"type": "boolean", "description": "also write <id>.regions.png: each texel coloured by its skin regions, for inspection"},
               "out": {"type": "string", "minLength": 1, "description": "folder; writes <id>.albedo.png, <id>.normal.png and <id>.orm.png"}}),
        &["store", "game", "landmarks", "out"],
    )
}

pub fn generate_output() -> Value {
    shape(json!({"look": {"type": "object"}, "albedo": {"type": "string"}, "normal": {"type": "string"}, "orm": {"type": "string"},
                 "size": {"type": "integer"}, "covered_fraction": {"type": "number"}, "seconds": {"type": "number"}}))
}

pub fn resolve_look(input: &Value) -> Result<SkinLook, ApiError> {
    let id = input.get("look").and_then(Value::as_str).unwrap_or(st::DEFAULT_LOOK);
    let base = st::look(id).ok_or_else(|| ApiError::invalid(format!("skin look {id:?} is not a preset of skin.models")))?;
    let Some(over) = input.get("skin") else { return Ok(base) };
    let mut merged = serde_json::to_value(&base).map_err(|e| ApiError::internal(e.to_string()))?;
    let over = over.as_object().ok_or_else(|| ApiError::invalid("skin must be an object"))?;
    for (k, v) in over {
        if k == "id" || merged.get(k).is_none() {
            return Err(ApiError::invalid(format!("skin has no field {k:?}")));
        }
        merged[k] = v.clone();
    }
    merged["id"] = json!(format!("{id}+custom"));
    serde_json::from_value(merged).map_err(|e| ApiError::invalid(format!("skin: {e}")))
}

pub fn read_landmarks(path: &Path, dense: &[[f64; 3]]) -> Result<BTreeMap<usize, [f64; 3]>, ApiError> {
    let raw = std::fs::read(path).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
    parse_landmarks(&raw, dense).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))
}

pub fn parse_landmarks(raw: &[u8], dense: &[[f64; 3]]) -> Result<BTreeMap<usize, [f64; 3]>, String> {
    let v: Value = serde_json::from_slice(raw).map_err(|e| e.to_string())?;
    let table = v.get("vertices").and_then(Value::as_object).ok_or("no vertices table")?;
    let mut out = BTreeMap::new();
    for (k, idx) in table {
        let lm: usize = k.parse().map_err(|_| format!("landmark key {k:?} is not a number"))?;
        let i = idx.as_u64().ok_or_else(|| format!("landmark {k} has no vertex index"))? as usize;
        let p = dense.get(i).ok_or_else(|| format!("landmark {k} names vertex {i}, beyond the dense base"))?;
        out.insert(lm, *p);
    }
    Ok(out)
}

pub const GRID_MM: f32 = 4.0;

pub fn stamp_images(store: &Store, skin: &SkinSpec) -> Result<gf_core::skin_authored::Images, String> {
    let mut images = gf_core::skin_authored::Images::new();
    for stamp in skin.layers.iter().flat_map(|l| &l.marks).filter_map(|m| match m {
        gf_core::skin_authored::Mark::Stamp(s) => Some(s),
        gf_core::skin_authored::Mark::Stroke(_) => None,
    }) {
        let key = (stamp.image.clone(), stamp.channel.clone());
        if images.contains_key(&key) {
            continue;
        }
        let bytes = store.get_ref(&stamp.image).map_err(|e| format!("stamp image {}: {e}", stamp.image))?;
        let (w, h, rgba) = gf_io::png::decode_rgba(&bytes).map_err(|e| format!("stamp image {}: {e}", stamp.image))?;
        images.insert(key, gf_core::skin_authored::Grey::from_rgba(w, h, &rgba, &stamp.channel));
    }
    Ok(images)
}

pub fn head_material(store: &Store, skin: &SkinSpec, dense: &[[f64; 3]], mesh: &Decimated) -> Result<(gltf::Material, Value), String> {
    let started = Instant::now();
    let raw = store.get_ref(&skin.landmarks).map_err(|e| format!("skin.landmarks: {e}"))?;
    let landmarks = parse_landmarks(&raw, dense).map_err(|e| format!("skin.landmarks: {e}"))?;
    let positions: Vec<[f64; 3]> = mesh.kept.iter().map(|&k| dense[k as usize]).collect();
    let normals = game_mesh::game_normals(&positions, &mesh.triangles);
    let regions = skin_regions::build(&positions, &normals, &mesh.triangles, &landmarks)?;
    let map = skin_map::build(&mesh.triangle_uvs, skin.size, PAD);
    let mut tex = st::generate(&map, &positions, &normals, &mesh.triangles, &mesh.triangle_uvs, &regions, &skin.look);
    let surface = sl::Surface { map: &map, positions: &positions, normals: &normals, triangles: &mesh.triangles, uvs: &mesh.triangle_uvs, regions: &regions, landmarks: &landmarks };
    let texels = sl::texels(&surface);
    let grid = gf_core::skin_authored::Grid::new(&texels, GRID_MM);
    let images = stamp_images(store, skin)?;
    let mut live = Vec::new();
    let mut rows = Vec::new();
    for layer in &skin.layers {
        let response = layer.response().ok_or_else(|| format!("skin layer {}: no response for {} {}", layer.name, layer.kind, layer.style))?;
        let mask = sl::mask(&surface, &texels, &grid, layer, &images)?;
        let reached = mask.iter().filter(|&&m| m > 0).count();
        if layer.is_live() {
            live.push(mask);
        } else {
            sl::bake(&mut tex, &mask, &response, layer.amount as f32, layer.variant as f32);
        }
        rows.push(json!({"name": layer.name, "kind": layer.kind, "style": layer.style, "live": layer.is_live(), "amount": layer.amount,
                         "variant": layer.variant, "mask_texels": reached}));
    }
    let png = |rgba: &[u8]| gf_io::png::rgba(skin.size, skin.size, rgba);
    let rgba = |rgb: &[u8]| -> Vec<u8> { rgb.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect() };
    let masks = sl::pack(&live).iter().map(|m| png(m)).collect::<Result<Vec<_>, _>>()?;
    let material = gltf::Material {
        name: "head_skin".into(),
        base_color: [1.0; 4],
        base_texture: Some(png(&tex.albedo)?),
        normal_texture: Some(png(&rgba(&tex.normal))?),
        orm_texture: Some(png(&rgba(&tex.orm))?),
        metallic: 0.0,
        roughness: 1.0,
        transmission: None,
        ior: None,
        blend: false,
        alpha_cutoff: None,
        double_sided: false,
        extension: Some(gltf::MaterialExtension { name: sl::EXTENSION, value: sl::extension(skin), textures: masks }),
    };
    let report = json!({"look": skin.look.id, "size": skin.size, "layers": rows, "mask_textures": live.len().div_ceil(4),
                        "covered_fraction": map.covered() as f64 / (skin.size * skin.size) as f64, "seconds": started.elapsed().as_secs_f64()});
    Ok((material, report))
}

pub fn layers_input() -> Value {
    obj(json!({}), &[])
}

pub fn layers_output() -> Value {
    shape(json!({"extension": {"type": "string"}, "kinds": {"type": "array"}}))
}

pub fn layers(_: &Ctx, _: &Value) -> Result<Value, ApiError> {
    let kinds: Vec<Value> = sl::STYLES
        .iter()
        .map(|(kind, styles)| {
            let rows: Vec<Value> = styles
                .iter()
                .filter_map(|style| {
                    let probe = sl::SkinLayer { name: "x".into(), kind: kind.to_string(), style: style.to_string(), live: None, amount: 0.0, variant: 0.0, seed: 0, colour: None, procedural: None, marks: Vec::new() };
                    let r = probe.response()?;
                    Some(json!({"style": style, "colour": r.colour, "opacity": r.opacity, "roughness": r.roughness, "spread": r.spread,
                                "variant": r.variant.map(|(name, colour, roughness)| json!({"name": name, "colour": colour, "roughness": roughness}))}))
                })
                .collect();
            json!({"kind": kind, "styles": rows})
        })
        .collect();
    let presets: Vec<Value> = gf_core::skin_authored::presets().into_iter().map(|(name, layers)| json!({"name": name, "layers": layers})).collect();
    Ok(json!({"extension": sl::EXTENSION, "kinds": kinds, "presets": presets, "max_live": sl::MAX_LIVE, "default_size": sl::DEFAULT_SIZE,
              "layer": "{name: [a-z][a-z0-9_]*, kind, style, live: true (default) | false, amount: 0..1 (live: the default; baked: how much is painted in), variant: 0..1 (the second state), seed, colour: optional linear rgb, procedural: true (default) | false, marks: [{stroke: {...}} | {stamp: {...}}, ...] in the order they apply}",
              "stroke": "{mode: add (default) | erase, radius_mm, strength: 0..1 (default 1), hardness: 0..1 (default 0.5), points: [[x, y, z] millimetres on the neutral base]}",
              "stamp": "{image: a reference from skin.stamp, channel: alpha (default) | luminance, at: [x, y, z] mm on the neutral base, normal: [x, y, z], angle_deg, size_mm (the image's width), strength, mode: add | erase}",
              "mask_order": "the procedural pattern, then the marks in their order; add takes the larger value, erase multiplies by one minus its value"}))
}

pub fn landmarks_input() -> Value {
    obj(
        json!({"store": {"type": "string", "minLength": 1},
               "game": {"type": "string", "minLength": 1, "description": "the game base the table is checked against (its pack's dense base)"},
               "landmarks": {"type": "string", "minLength": 1, "description": "path of the table, e.g. base_landmarks.json from a photos run"}}),
        &["store", "game", "landmarks"],
    )
}

pub fn landmarks_output() -> Value {
    shape(json!({"landmarks": {"type": "string"}, "count": {"type": "integer"}}))
}

pub fn landmarks(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::not_found)?;
    let (manifest, mesh) = store.load_game(text(input, "game")).map_err(ApiError::not_found)?;
    let (_, dense) = store.load_pack(&manifest.pack).map_err(ApiError::not_found)?;
    let path = Path::new(text(input, "landmarks"));
    let table = read_landmarks(path, &dense.positions)?;
    let positions: Vec<[f64; 3]> = mesh.kept.iter().map(|&k| dense.positions[k as usize]).collect();
    let normals = game_mesh::game_normals(&positions, &mesh.triangles);
    skin_regions::build(&positions, &normals, &mesh.triangles, &table).map_err(ApiError::invalid)?;
    if let Some(missing) = sl::LANDMARKS.iter().find(|i| !table.contains_key(i)) {
        return Err(ApiError::invalid(format!("{}: landmark {missing} is missing; the skin layers need {:?}", path.display(), sl::LANDMARKS)));
    }
    let hex = store.put_file(path).map_err(ApiError::internal)?;
    Ok(json!({"landmarks": gf_core::hash::reference(&hex), "count": table.len()}))
}

pub fn stamp_input() -> Value {
    obj(
        json!({"store": {"type": "string", "minLength": 1}, "image": {"type": "string", "minLength": 1, "description": "path of a PNG"}}),
        &["store", "image"],
    )
}

pub fn stamp_output() -> Value {
    shape(json!({"image": {"type": "string"}, "width": {"type": "integer"}, "height": {"type": "integer"}, "alpha_mean": {"type": "number"}, "luminance_mean": {"type": "number"}}))
}

pub fn stamp(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::not_found)?;
    let path = Path::new(text(input, "image"));
    let bytes = std::fs::read(path).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
    let (w, h, rgba) = gf_io::png::decode_rgba(&bytes).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
    let mean = |channel: &str| {
        let g = gf_core::skin_authored::Grey::from_rgba(w, h, &rgba, channel);
        g.values.iter().map(|&v| v as f64).sum::<f64>() / g.values.len().max(1) as f64
    };
    let hex = store.put_file(path).map_err(ApiError::internal)?;
    Ok(json!({"image": gf_core::hash::reference(&hex), "width": w, "height": h, "alpha_mean": mean("alpha"), "luminance_mean": mean("luminance")}))
}

pub fn project_input() -> Value {
    obj(
        crate::photo::with_tools(json!({"run": {"type": "string", "minLength": 1, "description": "a photos-path run: views.json, fit_photos.json (cameras) and fitted_dense.npy"},
               "store": {"type": "string", "minLength": 1},
               "game": {"type": "string", "minLength": 1},
               "landmarks": {"type": "string", "minLength": 1, "description": "the base's landmark table; default <run>/base_landmarks.json"},
               "look": {"type": "string", "minLength": 1, "description": "the preset whose detail (pores, freckles, stubble...) fills what no photo sees; its tone is fitted to the photos"},
               "skin": {"type": "object", "description": "overrides of the preset's fields, as in skin.generate"},
               "brows": {"enum": ["remove", "keep"], "description": "remove (default): the brows are not projected and the skin under them is filled; keep: they stay in the texture"},
               "size": {"type": "integer", "minimum": 256, "maximum": 8192},
               "name": {"type": "string", "minLength": 1, "description": "file prefix, default projected"},
               "out": {"type": "string", "minLength": 1, "description": "folder for the PNGs"}})),
        &["run", "store", "game", "out"],
    )
}

pub fn project_output() -> Value {
    shape(json!({"albedo": {"type": "string"}, "generated_albedo": {"type": "string"}, "confidence": {"type": "string"}, "look": {"type": "object"},
                 "photos": {"type": "object"}, "fill": {"type": "object"}, "seconds": {"type": "number"}}))
}

fn write_png(path: &Path, size: usize, rgba: &[u8]) -> Result<String, ApiError> {
    let png = gf_io::png::rgba(size, size, rgba).map_err(ApiError::internal)?;
    std::fs::write(path, png).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
    Ok(path.display().to_string())
}

pub fn project(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let started = Instant::now();
    let run = Path::new(text(input, "run"));
    let mut look = resolve_look(input)?;
    let size = input.get("size").and_then(Value::as_u64).map(|s| s as usize).unwrap_or(DEFAULT_SIZE);
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::not_found)?;
    let (manifest, mesh) = store.load_game(text(input, "game")).map_err(ApiError::not_found)?;
    let (_, dense) = store.load_pack(&manifest.pack).map_err(ApiError::not_found)?;
    let fitted = gf_io::dense::read_points_any(&run.join("fitted_dense.npy")).map_err(ApiError::not_found)?;
    if fitted.len() != dense.positions.len() {
        return Err(ApiError::invalid(format!("fitted_dense.npy has {} vertices, the pack {}", fitted.len(), dense.positions.len())));
    }
    let landmarks_path = input.get("landmarks").and_then(Value::as_str).map(Path::new).map(Path::to_path_buf).unwrap_or_else(|| run.join("base_landmarks.json"));
    let landmarks = read_landmarks(&landmarks_path, &dense.positions)?;
    let neutral: Vec<[f64; 3]> = mesh.kept.iter().map(|&k| dense.positions[k as usize]).collect();
    let shaped: Vec<[f64; 3]> = mesh.kept.iter().map(|&k| fitted[k as usize]).collect();
    let neutral_n = game_mesh::game_normals(&neutral, &mesh.triangles);
    let shaped_n = game_mesh::game_normals(&shaped, &mesh.triangles);
    let regions = skin_regions::build(&neutral, &neutral_n, &mesh.triangles, &landmarks).map_err(ApiError::invalid)?;
    let map = skin_map::build(&mesh.triangle_uvs, size, PAD);
    let n = size * size;
    let dir = run.join("skin");
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::invalid(format!("{}: {e}", dir.display())))?;
    let io = |r: Result<(), String>| r.map_err(ApiError::internal);
    io(gf_io::write_table_i32(&dir.join("texel_tri.npy"), n, 1, map.triangle.iter().map(|&t| if t == skin_map::NONE { -1 } else { t as i32 }).collect()))?;
    io(gf_io::write_table_f32(&dir.join("texel_bary.npy"), n, 2, map.bary.iter().flatten().copied().collect()))?;
    io(gf_io::dense::write_points(&dir.join("positions.npy"), &shaped))?;
    io(gf_io::dense::write_points(&dir.join("normals.npy"), &shaped_n))?;
    io(gf_io::write_table_i32(&dir.join("triangles.npy"), mesh.triangles.len(), 3, mesh.triangles.iter().flatten().map(|&i| i as i32).collect()))?;
    let nr = skin_regions::NAMES.len();
    io(gf_io::write_table_f32(&dir.join("regions.npy"), neutral.len(), nr, regions.weights.iter().flatten().copied().collect()))?;
    let brows = input.get("brows").and_then(Value::as_str).unwrap_or("remove");
    let photos = crate::photo::worker(
        input,
        "skin_worker.py",
        "project",
        json!({"dir": dir.display().to_string(), "views": run.join("views.json").display().to_string(), "cameras": run.join("fit_photos.json").display().to_string(), "brows": brows}),
        run,
    )?;
    let (_, _, projected) = gf_io::read_table_f32(&dir.join("projected.npy")).map_err(ApiError::internal)?;
    let (_, _, confidence) = gf_io::read_table_f32(&dir.join("confidence.npy")).map_err(ApiError::internal)?;
    if projected.len() != n * 3 || confidence.len() != n {
        return Err(ApiError::internal("the worker's arrays do not match the texel count"));
    }
    let neutral_mm: Vec<[f32; 3]> = neutral.iter().map(|p| p.map(|x| (x * 1000.0) as f32)).collect();
    let mirror = skin_map::mirror(&map, &mesh.triangles, &neutral_mm);
    let px = |k: usize| [projected[k * 3], projected[k * 3 + 1], projected[k * 3 + 2]];
    let mut colour = vec![[0.0f32; 3]; n];
    let mut conf = vec![0.0f32; n];
    let mut mirrored = 0usize;
    for k in 0..n {
        if map.triangle[k] == skin_map::NONE {
            continue;
        }
        let (q, c) = (confidence[k], px(k));
        let m = mirror[k];
        let (qm, cm) = if m != skin_map::NONE { (confidence[m as usize], px(m as usize)) } else { (0.0, [0.0; 3]) };
        let wm = (qm * (1.0 - q)).max(0.0);
        let total = q + wm;
        if total > 1e-6 {
            colour[k] = [0, 1, 2].map(|j| (q * c[j] + wm * cm[j]) / total);
        }
        if wm > q {
            mirrored += 1;
        }
        conf[k] = total.min(1.0);
    }
    let region_at = |k: usize, r: usize| -> f32 {
        let t = mesh.triangles[map.triangle[k] as usize];
        let [b0, b1] = map.bary[k];
        b0 * regions.weights[t[0] as usize][r] + b1 * regions.weights[t[1] as usize][r] + (1.0 - b0 - b1) * regions.weights[t[2] as usize][r]
    };
    let (cheeks, forehead) = (skin_regions::Regions::index("cheeks").unwrap(), skin_regions::Regions::index("forehead").unwrap());
    let mut samples: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for k in 0..n {
        if map.triangle[k] != skin_map::NONE && conf[k] > 0.8 && (region_at(k, cheeks) > 0.4 || region_at(k, forehead) > 0.4) {
            for j in 0..3 {
                samples[j].push(colour[k][j]);
            }
        }
    }
    if samples[0].len() < 100 {
        return Err(ApiError::invalid(format!("only {} confident cheek and forehead texels; the photos do not show enough skin to fit a tone", samples[0].len())));
    }
    let median = |v: &mut Vec<f32>| {
        v.sort_by(f32::total_cmp);
        v[v.len() / 2]
    };
    let target = [median(&mut samples[0]), median(&mut samples[1]), median(&mut samples[2])];
    let (m, h, u, err) = st::fit_tone(target);
    look.melanin = m;
    look.haemoglobin = h;
    look.undertone = u;
    look.id = format!("{}+photos", look.id);
    let tex = st::generate(&map, &neutral, &neutral_n, &mesh.triangles, &mesh.triangle_uvs, &regions, &look);
    let proc_lin = |k: usize| [0, 1, 2].map(|j| st::to_linear(tex.albedo[k * 4 + j] as f32 / 255.0));
    let v = neutral.len();
    let mut gsum = vec![[0.0f64; 3]; v];
    let mut wsum = vec![0.0f64; v];
    for k in 0..n {
        if map.triangle[k] == skin_map::NONE || conf[k] < 0.6 {
            continue;
        }
        let p = proc_lin(k);
        let t = mesh.triangles[map.triangle[k] as usize];
        let [b0, b1] = map.bary[k];
        for (c, b) in t.iter().zip([b0, b1, 1.0 - b0 - b1]) {
            for j in 0..3 {
                gsum[*c as usize][j] += (b * (colour[k][j] / p[j].max(1e-4)).clamp(0.2, 5.0)) as f64;
            }
            wsum[*c as usize] += b as f64;
        }
    }
    let known: Vec<bool> = wsum.iter().map(|&w| w > 1e-6).collect();
    let n_known = known.iter().filter(|&&k| k).count().max(1) as f64;
    let mean_gain: [f64; 3] = [0, 1, 2].map(|j| (0..v).filter(|&i| known[i]).map(|i| gsum[i][j] / wsum[i]).sum::<f64>() / n_known);
    let mut gain: Vec<[f64; 3]> = (0..v).map(|i| if known[i] { gsum[i].map(|x| x / wsum[i]) } else { mean_gain }).collect();
    let nb = skin_regions::neighbours(v, &mesh.triangles);
    for round in 0..400 {
        let prev = gain.clone();
        for i in 0..v {
            if nb[i].is_empty() || (known[i] && round >= 20) {
                continue;
            }
            let mut s = [0.0; 3];
            for &j in &nb[i] {
                for c in 0..3 {
                    s[c] += prev[j as usize][c];
                }
            }
            gain[i] = s.map(|x| x / nb[i].len() as f64);
        }
    }
    let mut albedo = vec![0u8; n * 4];
    let mut generated = vec![0u8; n * 4];
    let mut conf_png = vec![0u8; n * 4];
    for k in 0..n {
        let s = map.source[k];
        if s == skin_map::NONE {
            albedo[k * 4..k * 4 + 4].copy_from_slice(&[128, 128, 128, 255]);
            generated[k * 4..k * 4 + 4].copy_from_slice(&[128, 128, 128, 255]);
            conf_png[k * 4..k * 4 + 4].copy_from_slice(&[0, 0, 0, 255]);
            continue;
        }
        let s = s as usize;
        let t = mesh.triangles[map.triangle[s] as usize];
        let [b0, b1] = map.bary[s];
        let b = [b0, b1, 1.0 - b0 - b1];
        let g = [0, 1, 2].map(|j| (0..3).map(|c| b[c] as f64 * gain[t[c] as usize][j]).sum::<f64>() as f32);
        let p = proc_lin(s);
        let q = conf[s];
        let c = [0, 1, 2].map(|j| q * colour[s][j] + (1.0 - q) * p[j] * g[j]);
        for j in 0..3 {
            albedo[k * 4 + j] = (st::to_srgb(c[j]) * 255.0 + 0.5) as u8;
            generated[k * 4 + j] = (st::to_srgb(p[j] * mean_gain[j] as f32) * 255.0 + 0.5) as u8;
        }
        albedo[k * 4 + 3] = 255;
        generated[k * 4 + 3] = 255;
        let qc = (q * 255.0 + 0.5) as u8;
        conf_png[k * 4..k * 4 + 4].copy_from_slice(&[qc, qc, qc, 255]);
    }
    let out = Path::new(text(input, "out"));
    std::fs::create_dir_all(out).map_err(|e| ApiError::invalid(format!("{}: {e}", out.display())))?;
    let name = input.get("name").and_then(Value::as_str).unwrap_or("projected");
    let rgba = |rgb: &[u8]| -> Vec<u8> { rgb.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect() };
    let albedo_path = write_png(&out.join(format!("{name}.albedo.png")), size, &albedo)?;
    let normal_path = write_png(&out.join(format!("{name}.normal.png")), size, &rgba(&tex.normal))?;
    let orm_path = write_png(&out.join(format!("{name}.orm.png")), size, &rgba(&tex.orm))?;
    let generated_path = write_png(&out.join(format!("{name}.generated.albedo.png")), size, &generated)?;
    let conf_path = write_png(&out.join(format!("{name}.confidence.png")), size, &conf_png)?;
    let covered = map.covered().max(1) as f64;
    let photo_part = (0..n).filter(|&k| map.triangle[k] != skin_map::NONE).map(|k| conf[k] as f64).sum::<f64>() / covered;
    Ok(json!({"albedo": albedo_path, "normal": normal_path, "orm": orm_path, "generated_albedo": generated_path, "confidence": conf_path,
              "look": look, "photos": photos,
              "fill": {"photo_share": photo_part, "mirrored_texels": mirrored, "tone_target_linear": target, "tone_fit_log_rms": err, "albedo_scale": "as photographed: the colour at the shading of the median forehead, cheek and nose texel",
                       "gain_vertices_known": known.iter().filter(|&&k| k).count(), "vertices": v},
              "seconds": started.elapsed().as_secs_f64()}))
}

pub fn generate(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let started = Instant::now();
    let look = resolve_look(input)?;
    let size = input.get("size").and_then(Value::as_u64).map(|s| s as usize).unwrap_or(DEFAULT_SIZE);
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::not_found)?;
    let (manifest, mesh) = store.load_game(text(input, "game")).map_err(ApiError::not_found)?;
    let (_, dense) = store.load_pack(&manifest.pack).map_err(ApiError::not_found)?;
    let landmarks = read_landmarks(Path::new(text(input, "landmarks")), &dense.positions)?;
    let positions: Vec<[f64; 3]> = mesh.kept.iter().map(|&k| dense.positions[k as usize]).collect();
    let normals = game_mesh::game_normals(&positions, &mesh.triangles);
    let regions = skin_regions::build(&positions, &normals, &mesh.triangles, &landmarks).map_err(ApiError::invalid)?;
    let map = skin_map::build(&mesh.triangle_uvs, size, PAD);
    let tex = st::generate(&map, &positions, &normals, &mesh.triangles, &mesh.triangle_uvs, &regions, &look);
    let out = Path::new(text(input, "out"));
    std::fs::create_dir_all(out).map_err(|e| ApiError::invalid(format!("{}: {e}", out.display())))?;
    let rgba = |rgb: &[u8]| -> Vec<u8> { rgb.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect() };
    let mut written = BTreeMap::new();
    for (kind, pixels) in [("albedo", tex.albedo.clone()), ("normal", rgba(&tex.normal)), ("orm", rgba(&tex.orm))] {
        let path = out.join(format!("{}.{kind}.png", look.id));
        let png = gf_io::png::rgba(size, size, &pixels).map_err(ApiError::internal)?;
        std::fs::write(&path, png).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
        written.insert(kind, path.display().to_string());
    }
    if input.get("regions").and_then(Value::as_bool) == Some(true) {
        let path = out.join(format!("{}.regions.png", look.id));
        let png = gf_io::png::rgba(size, size, &st::region_map(&map, &mesh.triangles, &regions)).map_err(ApiError::internal)?;
        std::fs::write(&path, png).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
        written.insert("regions", path.display().to_string());
    }
    Ok(json!({"look": look, "albedo": written["albedo"], "normal": written["normal"], "orm": written["orm"], "regions": written.get("regions"), "size": size,
              "covered_fraction": map.covered() as f64 / (size * size) as f64, "seconds": started.elapsed().as_secs_f64()}))
}
