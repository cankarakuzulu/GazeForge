use serde_json::{Value, json};

pub struct Channel {
    pub node: usize,
    pub path: &'static str,
    pub values: Vec<f32>,
}

pub struct Clip {
    pub name: String,
    pub times: Vec<f32>,
    pub channels: Vec<Channel>,
    pub extras: Value,
}

pub struct Rest {
    pub node: usize,
    pub path: String,
    pub rest: Vec<f32>,
    pub poses: Vec<(usize, Vec<f32>)>,
}

fn pad(buf: &mut Vec<u8>, byte: u8) {
    while !buf.len().is_multiple_of(4) {
        buf.push(byte);
    }
}

fn push_accessor(doc: &mut Value, bin: &mut Vec<u8>, values: &[f32], kind: &str, bounds: bool) -> usize {
    pad(bin, 0);
    let offset = bin.len();
    for v in values {
        bin.extend(v.to_le_bytes());
    }
    let views = doc["bufferViews"].as_array_mut().expect("bufferViews checked");
    views.push(json!({"buffer": 0, "byteOffset": offset, "byteLength": values.len() * 4}));
    let view = views.len() - 1;
    let width = match kind {
        "VEC4" => 4,
        "VEC3" => 3,
        _ => 1,
    };
    let mut accessor = json!({"bufferView": view, "componentType": 5126, "count": values.len() / width, "type": kind});
    if bounds {
        let lo = values.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        accessor["min"] = json!([lo]);
        accessor["max"] = json!([hi]);
    }
    let accessors = doc["accessors"].as_array_mut().expect("accessors checked");
    accessors.push(accessor);
    accessors.len() - 1
}

pub fn node_index(doc: &Value, name: &str) -> Option<usize> {
    doc["nodes"].as_array()?.iter().position(|n| n["name"].as_str() == Some(name))
}

pub fn morph_nodes(doc: &Value) -> Vec<(usize, Vec<String>)> {
    let mut out = Vec::new();
    for (i, node) in doc["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
        let Some(m) = node["mesh"].as_u64() else { continue };
        let mesh = &doc["meshes"][m as usize];
        let has_targets = mesh["primitives"][0]["targets"].as_array().is_some_and(|t| !t.is_empty());
        if !has_targets {
            continue;
        }
        let names = mesh["extras"]["targetNames"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        out.push((i, names));
    }
    out
}

pub fn shape_poses(doc: &Value, bin: &[u8], shapes: &[String]) -> Result<Vec<Rest>, String> {
    let mut rests: Vec<Rest> = Vec::new();
    for anim in doc["animations"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let Some(name) = anim["name"].as_str() else { continue };
        let Some(shape) = shapes.iter().position(|s| s == name) else { continue };
        for ch in anim["channels"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let path = ch["target"]["path"].as_str().unwrap_or("");
            if path != "rotation" && path != "translation" {
                continue;
            }
            let node = ch["target"]["node"].as_u64().ok_or("a channel has no node")? as usize;
            let sampler = &anim["samplers"][ch["sampler"].as_u64().ok_or("a channel has no sampler")? as usize];
            let values = gf_io::gltf::read_plain(doc, bin, sampler["output"].as_u64().ok_or("a sampler has no output")?)?;
            let width = if path == "rotation" { 4 } else { 3 };
            if values.len() < 2 * width {
                return Err(format!("animation {name}: a {path} channel has fewer than two keys"));
            }
            let (rest, pose) = (values[..width].to_vec(), values[values.len() - width..].to_vec());
            match rests.iter_mut().find(|r| r.node == node && r.path == path) {
                Some(r) => r.poses.push((shape, pose)),
                None => rests.push(Rest { node, path: path.to_string(), rest, poses: vec![(shape, pose)] }),
            }
        }
    }
    Ok(rests)
}

pub fn append(bytes: &[u8], clips: &[Clip]) -> Result<Vec<u8>, String> {
    let (doc, bin) = gf_io::gltf::chunks(bytes)?;
    let mut doc = doc;
    let mut bin = bin.to_vec();
    if doc["buffers"].as_array().is_none_or(|b| b.is_empty()) {
        return Err("the glb has no buffer".into());
    }
    for key in ["bufferViews", "accessors", "animations"] {
        if doc[key].is_null() {
            doc[key] = json!([]);
        }
    }
    for clip in clips {
        if doc["animations"].as_array().is_some_and(|a| a.iter().any(|x| x["name"].as_str() == Some(clip.name.as_str()))) {
            return Err(format!("the glb already has an animation named {}", clip.name));
        }
        let input = push_accessor(&mut doc, &mut bin, &clip.times, "SCALAR", true);
        let mut written: Vec<(usize, usize)> = Vec::new();
        let (mut samplers, mut channels) = (Vec::new(), Vec::new());
        for (k, ch) in clip.channels.iter().enumerate() {
            let same = written.iter().find(|(j, _)| clip.channels[*j].path == ch.path && clip.channels[*j].values == ch.values).map(|(_, a)| *a);
            let output = match same {
                Some(a) => a,
                None => {
                    let kind = match ch.path {
                        "rotation" => "VEC4",
                        "translation" => "VEC3",
                        _ => "SCALAR",
                    };
                    let a = push_accessor(&mut doc, &mut bin, &ch.values, kind, false);
                    written.push((k, a));
                    a
                }
            };
            channels.push(json!({"sampler": samplers.len(), "target": {"node": ch.node, "path": ch.path}}));
            samplers.push(json!({"input": input, "output": output, "interpolation": "LINEAR"}));
        }
        doc["animations"].as_array_mut().expect("animations set").push(json!({"name": clip.name, "samplers": samplers, "channels": channels, "extras": clip.extras}));
    }
    pad(&mut bin, 0);
    doc["buffers"][0]["byteLength"] = json!(bin.len());
    let mut text = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    pad(&mut text, b' ');
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend(0x4654_6C67u32.to_le_bytes());
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((text.len() as u32).to_le_bytes());
    out.extend(0x4E4F_534Au32.to_le_bytes());
    out.extend(&text);
    out.extend((bin.len() as u32).to_le_bytes());
    out.extend(0x004E_4942u32.to_le_bytes());
    out.extend(&bin);
    Ok(out)
}
