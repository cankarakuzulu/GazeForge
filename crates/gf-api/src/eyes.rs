use std::path::Path;

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

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
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
    let mut eyeball: EyeballShape = match input.get("shape") {
        Some(s) => serde_json::from_value(s.clone()).map_err(|e| ApiError::invalid(format!("shape: {e}")))?,
        None => EyeballShape::default(),
    };
    let gaze = match input.get("gaze").and_then(Value::as_array) {
        Some(g) => unit([0, 1, 2].map(|k| g[k].as_f64().unwrap_or(f64::NAN)))?,
        None => [0.0, -1.0, 0.0],
    };
    let up = unit({
        let w = [0.0, 0.0, 1.0];
        let d = w[0] * gaze[0] + w[1] * gaze[1] + w[2] * gaze[2];
        [w[0] - d * gaze[0], w[1] - d * gaze[1], w[2] - d * gaze[2]]
    })
    .map_err(|_| ApiError::invalid("gaze must not point straight up or down"))?;
    let side = cross(up, gaze);
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
        let mesh = eyeball.mesh().map_err(ApiError::invalid)?;
        let rotate = |p: [f64; 3]| [0, 1, 2].map(|k| side[k] * p[0] + up[k] * p[1] + gaze[k] * p[2]);
        let vertices: Vec<[f64; 3]> = mesh.positions.iter().map(|&p| {
            let r = rotate(p);
            [0, 1, 2].map(|k| centre[k] + r[k])
        }).collect();
        triangles += mesh.triangles.len();
        report.push(json!({"side": name, "centre_mm": centre.map(|x| (x * 1e5).round() / 100.0), "radius_mm": eyeball.radius_mm, "helper_radius_mm": helper_radius * 1000.0}));
        eyes.push(json!({"name": name, "centre": centre, "radius": eyeball.radius_mm / 1000.0, "gaze": gaze, "vertices": vertices, "triangles": mesh.triangles,
                         "normals": mesh.normals.iter().map(|&n| rotate(n)).collect::<Vec<_>>(), "uvs": mesh.uvs, "parts": mesh.parts, "shape": eyeball}));
    }
    let doc = json!({"eyes": eyes, "generator": {"command": "eyes.generate", "rig": rig_ref, "placement": "the centres of MakeHuman's helper eyeballs on the dense base"}});
    let bytes = serde_json::to_vec(&doc).map_err(|e| ApiError::internal(e.to_string()))?;
    gf_geom::lids::eyes_from_json(std::str::from_utf8(&bytes).map_err(|e| ApiError::internal(e.to_string()))?).map_err(|e| ApiError::internal(format!("the generated eyes do not load: {e}")))?;
    let out = text(input, "out");
    if let Some(dir) = Path::new(out).parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| ApiError::invalid(format!("{}: {e}", dir.display())))?;
    }
    std::fs::write(out, &bytes).map_err(|e| ApiError::invalid(format!("{out}: {e}")))?;
    Ok(json!({"out": out, "rig": rig_ref, "eyes": report, "triangles": triangles}))
}
