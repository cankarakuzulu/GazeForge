use std::path::Path;

use gf_core::eye_inventory::EyeModel;
use gf_geom::eyeball::EyeballShape;
use gf_io::store::Store;
use serde_json::{Value, json};

use crate::Ctx;
use crate::commands::{obj, shape, text};
use crate::error::ApiError;

pub const DEFAULT_RADIUS_MM: f64 = 12.0;

pub fn generate_input() -> Value {
    obj(
        json!({"store": {"type": "string", "minLength": 1},
               "catalog": {"type": "string", "minLength": 1, "description": "the catalog whose eye rig places the eyes"},
               "rig": {"type": "string", "minLength": 1, "description": "an eye rig reference; default: the store's rig for the catalog"},
               "radius_mm": {"type": "number", "exclusiveMinimum": 0, "description": "eyeball radius, default 12"},
               "helper_radius": {"type": "boolean", "description": "use the radius of MakeHuman's helper eyeball instead of radius_mm"},
               "model": {"type": "string", "minLength": 1, "description": "an eye model of the inventory (eyes.inventory); its proportions are used unless shape is given, and the file records the model"},
               "shape": {"type": "object", "description": "eyeball shape: cornea_curvature (cornea radius of curvature over the eyeball radius, 0.65), limbus (limbus diameter over the radius, 0.96), iris_depth (iris plane behind the cornea apex, over the radius, 0.24), pupil (pupil radius over the limbus radius, 0.3), segments (32), rings (16)"},
               "gaze": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "gaze direction in the dense frame, default [0, -1, 0] (the base head looks along -y)"},
               "out": {"type": "string", "minLength": 1, "description": "the eyes file to write"}}),
        &["store", "catalog", "out"],
    )
}

pub fn generate_output() -> Value {
    shape(json!({"out": {"type": "string"}, "rig": {"type": "string"}, "eyes": {"type": "array"}, "triangles": {"type": "integer"}}))
}

fn unit(v: [f64; 3]) -> Result<[f64; 3], ApiError> {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if !(l.is_finite() && l > 1e-9) {
        return Err(ApiError::invalid("gaze must be a finite, non-zero direction"));
    }
    Ok(v.map(|x| x / l))
}

pub const IRIS_SIZE: [usize; 2] = [1024, 256];
pub const SCLERA_SIZE: [usize; 2] = [512, 256];

pub struct Texture {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

pub struct PartLook {
    pub part: &'static str,
    pub colour: [f32; 4],
    pub roughness: f32,
    pub transmission: Option<f32>,
    pub ior: Option<f32>,
    pub blend: bool,
    pub texture: Option<Texture>,
}

pub fn model_textures(m: &EyeModel) -> Result<(Texture, Texture), String> {
    let l = m.shape.layout()?;
    let limbus_v = ((l.limbus_radius / l.radius).asin() / std::f64::consts::PI) as f32;
    let [iw, ih] = IRIS_SIZE;
    let [sw, sh] = SCLERA_SIZE;
    Ok((
        Texture { width: iw, height: ih, rgba: gf_core::eye_texture::iris_rgba(&m.iris, iw, ih) },
        Texture { width: sw, height: sh, rgba: gf_core::eye_texture::sclera_rgba(&m.sclera, limbus_v, sw, sh, m.iris.seed) },
    ))
}

pub fn look(m: &EyeModel) -> Result<Vec<PartLook>, String> {
    let (iris, sclera) = model_textures(m)?;
    let plain = |part: &'static str, colour: [f32; 4], texture: Option<Texture>, roughness: f32| PartLook {
        part,
        colour,
        roughness,
        transmission: None,
        ior: None,
        blend: false,
        texture,
    };
    Ok(vec![
        plain("sclera", [1.0; 4], Some(sclera), 0.35),
        PartLook { transmission: Some(1.0), ior: Some(m.cornea.ior), blend: true, ..plain("cornea", [1.0, 1.0, 1.0, 0.08], None, m.cornea.roughness) },
        plain("iris", [1.0; 4], Some(iris), 0.55),
        plain("pupil", [0.01, 0.01, 0.01, 1.0], None, 0.9),
    ])
}

pub fn look_input() -> Value {
    obj(json!({"model": {"type": "string", "minLength": 1, "description": "an eye model id, as eyes.inventory lists it"},
               "textures": {"type": "boolean", "description": "include each textured part's RGBA pixels (default true)"}}), &["model"])
}

pub fn look_output() -> Value {
    shape(json!({"model": {"type": "string"},
                 "parts": {"type": "array", "description": "in the eye mesh's part order (sclera, cornea, iris, pupil): colour (linear RGBA), roughness, transmission, ior, blend, and texture {width, height, rgba_u8_b64} or null"}}))
}

pub fn look_command(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let id = text(input, "model");
    let m = gf_core::eye_inventory::model(id).ok_or_else(|| ApiError::not_found(format!("eye model {id:?} is not in the inventory")).hint("eyes.inventory lists the models"))?;
    let pixels = input.get("textures").and_then(Value::as_bool).unwrap_or(true);
    let parts: Vec<Value> = look(&m)
        .map_err(ApiError::invalid)?
        .into_iter()
        .map(|p| {
            let texture = p.texture.map(|t| {
                let mut v = json!({"width": t.width, "height": t.height});
                if pixels {
                    v["rgba_u8_b64"] = json!(crate::designer::base64(&t.rgba));
                }
                v
            });
            json!({"part": p.part, "colour": p.colour, "roughness": p.roughness, "transmission": p.transmission, "ior": p.ior, "blend": p.blend, "texture": texture})
        })
        .collect();
    Ok(json!({"model": m.id, "parts": parts}))
}

pub fn textures_input() -> Value {
    obj(
        json!({"models": {"type": "array", "items": {"type": "string"}, "description": "model ids; default: every model of the inventory"},
               "out": {"type": "string", "minLength": 1, "description": "folder; each model writes <id>.iris.png and <id>.sclera.png"}}),
        &["out"],
    )
}

pub fn textures_output() -> Value {
    shape(json!({"out": {"type": "string"}, "written": {"type": "array"}, "layout": {"type": "object"}}))
}

pub fn textures(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let out = Path::new(text(input, "out"));
    std::fs::create_dir_all(out).map_err(|e| ApiError::invalid(format!("{}: {e}", out.display())))?;
    let wanted: Option<Vec<String>> = input.get("models").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect());
    let all = gf_core::eye_inventory::models();
    if let Some(w) = &wanted
        && let Some(missing) = w.iter().find(|id| !all.iter().any(|m| &m.id == *id))
    {
        return Err(ApiError::invalid(format!("eye model {missing:?} is not in the inventory")));
    }
    let mut written = Vec::new();
    for m in all.iter().filter(|m| wanted.as_ref().is_none_or(|w| w.contains(&m.id))) {
        let (iris, sclera) = model_textures(m).map_err(ApiError::invalid)?;
        for (kind, t) in [("iris", iris), ("sclera", sclera)] {
            let path = out.join(format!("{}.{kind}.png", m.id));
            std::fs::write(&path, gf_io::png::rgba(t.width, t.height, &t.rgba).map_err(ApiError::internal)?).map_err(|e| ApiError::invalid(format!("{}: {e}", path.display())))?;
            written.push(path.display().to_string());
        }
    }
    Ok(json!({"out": out.display().to_string(), "written": written,
              "layout": {"iris": "u = angle around the gaze (0..1), v = from the pupil edge (0) to the limbus (1), matching the iris part's UVs",
                         "sclera": "u = angle around the gaze, v = polar angle from the front pole over pi, matching the sclera part's UVs"}}))
}

pub fn inventory_input() -> Value {
    obj(json!({"family": {"type": "string", "minLength": 1, "description": "only the models of one family"}}), &[])
}

pub fn inventory_output() -> Value {
    shape(json!({"default": {"type": "string"}, "families": {"type": "array"}, "models": {"type": "array"}}))
}

pub fn inventory(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let family = input.get("family").and_then(Value::as_str);
    let all = gf_core::eye_inventory::models();
    let mut families: Vec<&str> = Vec::new();
    for m in &all {
        if !families.contains(&m.family.as_str()) {
            families.push(&m.family);
        }
    }
    let models: Vec<&gf_core::eye_inventory::EyeModel> = all.iter().filter(|m| family.is_none_or(|f| m.family == f)).collect();
    Ok(json!({"default": gf_core::eye_inventory::DEFAULT_MODEL, "families": families, "models": models}))
}

pub fn generate(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let store = Store::open(Path::new(text(input, "store"))).map_err(ApiError::not_found)?;
    let catalog = text(input, "catalog");
    let rig_ref = match input.get("rig").and_then(Value::as_str) {
        Some(r) => r.to_string(),
        None => store.find_eyerig(catalog).map_err(ApiError::invalid)?.ok_or_else(|| ApiError::not_found(format!("the store has no eye rig for catalog {catalog}")).hint("store import-eyerig makes one"))?,
    };
    let rig = store.load_eyerig(&rig_ref).map_err(ApiError::not_found)?;
    if rig.catalog != catalog {
        return Err(ApiError::invalid(format!("eye rig {rig_ref} belongs to catalog {}, not {catalog}", rig.catalog)));
    }
    let model = match input.get("model").and_then(Value::as_str) {
        Some(id) => Some(gf_core::eye_inventory::model(id).ok_or_else(|| ApiError::invalid(format!("eye model {id:?} is not in the inventory")).hint("eyes.inventory lists the models"))?),
        None => None,
    };
    let mut eyeball: EyeballShape = match (input.get("shape"), &model) {
        (Some(s), _) => serde_json::from_value(s.clone()).map_err(|e| ApiError::invalid(format!("shape: {e}")))?,
        (None, Some(m)) => m.shape.clone(),
        (None, None) => EyeballShape::default(),
    };
    let gaze = match input.get("gaze").and_then(Value::as_array) {
        Some(g) => unit([0, 1, 2].map(|k| g[k].as_f64().unwrap_or(f64::NAN)))?,
        None => gf_io::character::MODEL_GAZE,
    };
    let mut eyes = Vec::new();
    let mut report = Vec::new();
    let mut triangles = 0;
    for (name, helper) in [("left", &rig.helpers.left), ("right", &rig.helpers.right)] {
        let n = helper.len().max(1) as f64;
        let centre = [0, 1, 2].map(|k| helper.iter().map(|p| p[k]).sum::<f64>() / n);
        let helper_radius = helper.iter().map(|p| (0..3).map(|k| (p[k] - centre[k]).powi(2)).sum::<f64>().sqrt()).sum::<f64>() / n;
        eyeball.radius_mm = if input.get("helper_radius").and_then(Value::as_bool).unwrap_or(false) {
            helper_radius * 1000.0
        } else {
            input.get("radius_mm").and_then(Value::as_f64).unwrap_or(DEFAULT_RADIUS_MM)
        };
        let mesh = eyeball.mesh().map_err(ApiError::invalid)?.placed(centre, gaze).map_err(ApiError::invalid)?;
        triangles += mesh.triangles.len();
        report.push(json!({"side": name, "centre_mm": centre.map(|x| (x * 1e5).round() / 100.0), "radius_mm": eyeball.radius_mm, "helper_radius_mm": helper_radius * 1000.0}));
        eyes.push(json!({"name": name, "centre": centre, "radius": eyeball.radius_mm / 1000.0, "gaze": gaze, "vertices": mesh.positions, "triangles": mesh.triangles,
                         "normals": mesh.normals, "uvs": mesh.uvs, "parts": mesh.parts, "shape": eyeball}));
    }
    let doc = json!({"eyes": eyes, "model": model, "generator": {"command": "eyes.generate", "rig": rig_ref, "placement": "the centres of MakeHuman's helper eyeballs on the dense base"}});
    let bytes = serde_json::to_vec(&doc).map_err(|e| ApiError::internal(e.to_string()))?;
    gf_geom::lids::eyes_from_json(std::str::from_utf8(&bytes).map_err(|e| ApiError::internal(e.to_string()))?).map_err(|e| ApiError::internal(format!("the generated eyes do not load: {e}")))?;
    let out = text(input, "out");
    if let Some(dir) = Path::new(out).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| ApiError::invalid(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(out, &bytes).map_err(|e| ApiError::invalid(format!("{out}: {e}")))?;
    Ok(json!({"out": out, "rig": rig_ref, "eyes": report, "triangles": triangles}))
}
