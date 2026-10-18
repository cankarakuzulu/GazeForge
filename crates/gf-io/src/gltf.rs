use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sparse {
    Auto,
    Always,
    Never,
}

impl Sparse {
    pub fn parse(text: &str) -> Result<Sparse, String> {
        match text {
            "auto" => Ok(Sparse::Auto),
            "always" => Ok(Sparse::Always),
            "never" => Ok(Sparse::Never),
            other => Err(format!("--sparse must be auto, always or never, got {other}")),
        }
    }
}

pub struct MorphData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

pub struct Lod {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub morphs: Vec<MorphData>,
}

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;

#[derive(Default)]
struct Builder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

fn bounds<const N: usize>(data: impl Iterator<Item = [f32; N]>) -> (Vec<f32>, Vec<f32>) {
    let mut lo = vec![f32::INFINITY; N];
    let mut hi = vec![f32::NEG_INFINITY; N];
    for v in data {
        for k in 0..N {
            lo[k] = lo[k].min(v[k]);
            hi[k] = hi[k].max(v[k]);
        }
    }
    (lo, hi)
}

impl Builder {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut view = json!({"buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len()});
        if let Some(t) = target {
            view["target"] = json!(t);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(view);
        self.views.len() - 1
    }

    fn floats<const N: usize>(&mut self, data: &[[f32; N]], target: Option<u32>, with_bounds: bool) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.iter().flat_map(|x| x.to_le_bytes())).collect();
        let view = self.view(&bytes, target);
        let kind = if N == 2 { "VEC2" } else { "VEC3" };
        let mut accessor = json!({"bufferView": view, "componentType": FLOAT, "count": data.len(), "type": kind});
        if with_bounds {
            let (lo, hi) = bounds(data.iter().copied());
            accessor["min"] = json!(lo);
            accessor["max"] = json!(hi);
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    fn indices(&mut self, indices: &[u32], short: bool) -> usize {
        let bytes: Vec<u8> = if short {
            indices.iter().flat_map(|&i| (i as u16).to_le_bytes()).collect()
        } else {
            indices.iter().flat_map(|&i| i.to_le_bytes()).collect()
        };
        let view = self.view(&bytes, Some(ELEMENT_ARRAY_BUFFER));
        let component = if short { UNSIGNED_SHORT } else { UNSIGNED_INT };
        self.accessors.push(json!({"bufferView": view, "componentType": component, "count": indices.len(), "type": "SCALAR"}));
        self.accessors.len() - 1
    }

    fn morph(&mut self, data: &[[f32; 3]], mode: Sparse, short: bool) -> (usize, &'static str, usize) {
        let entries: Vec<u32> = (0..data.len() as u32).filter(|&i| data[i as usize].iter().any(|x| *x != 0.0)).collect();
        let index_bytes = if short { 2 } else { 4 };
        let sparse = match mode {
            Sparse::Always => true,
            Sparse::Never => false,
            Sparse::Auto => entries.len() * (index_bytes + 12) < data.len() * 12,
        };
        if !sparse {
            let accessor = self.floats(data, Some(ARRAY_BUFFER), true);
            return (accessor, "dense", data.len());
        }
        let listed: Vec<u32> = if entries.is_empty() { vec![0] } else { entries };
        let index_bytes: Vec<u8> = if short {
            listed.iter().flat_map(|&i| (i as u16).to_le_bytes()).collect()
        } else {
            listed.iter().flat_map(|&i| i.to_le_bytes()).collect()
        };
        let values: Vec<[f32; 3]> = listed.iter().map(|&i| data[i as usize]).collect();
        let value_bytes: Vec<u8> = values.iter().flat_map(|v| v.iter().flat_map(|x| x.to_le_bytes())).collect();
        let index_view = self.view(&index_bytes, None);
        let value_view = self.view(&value_bytes, None);
        let (mut lo, mut hi) = bounds(values.iter().copied());
        if listed.len() < data.len() {
            for k in 0..3 {
                lo[k] = lo[k].min(0.0);
                hi[k] = hi[k].max(0.0);
            }
        }
        let component = if short { UNSIGNED_SHORT } else { UNSIGNED_INT };
        self.accessors.push(json!({
            "componentType": FLOAT, "count": data.len(), "type": "VEC3", "min": lo, "max": hi,
            "sparse": {"count": listed.len(), "indices": {"bufferView": index_view, "componentType": component},
                       "values": {"bufferView": value_view}}
        }));
        (self.accessors.len() - 1, "sparse", listed.len())
    }
}

pub fn glb(lods: &[Lod], morph_names: &[String], mode: Sparse, extras: Value, generator: &str) -> Result<(Vec<u8>, Value), String> {
    let mut b = Builder::default();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    let mut report = Vec::new();
    for lod in lods {
        let n = lod.positions.len();
        if lod.normals.len() != n || lod.uvs.len() != n || lod.morphs.len() != morph_names.len() {
            return Err(format!("{}: attribute or morph counts disagree", lod.name));
        }
        if lod.indices.iter().any(|&i| i as usize >= n) || lod.indices.len() % 3 != 0 {
            return Err(format!("{}: an index is out of range or the index count is not a multiple of 3", lod.name));
        }
        if lod.positions.iter().chain(&lod.normals).any(|v| v.iter().any(|x| !x.is_finite())) {
            return Err(format!("{}: a position or normal is not finite", lod.name));
        }
        let short = n <= u16::MAX as usize;
        let position = b.floats(&lod.positions, Some(ARRAY_BUFFER), true);
        let normal = b.floats(&lod.normals, Some(ARRAY_BUFFER), false);
        let uv = b.floats(&lod.uvs, Some(ARRAY_BUFFER), false);
        let indices = b.indices(&lod.indices, short);
        let mut targets = Vec::new();
        let mut morph_report = Vec::new();
        for (name, m) in morph_names.iter().zip(&lod.morphs) {
            if m.positions.len() != n || m.normals.len() != n {
                return Err(format!("{}: morph {name} has the wrong vertex count", lod.name));
            }
            let (p, p_kind, p_entries) = b.morph(&m.positions, mode, short);
            let (q, q_kind, q_entries) = b.morph(&m.normals, mode, short);
            targets.push(json!({"POSITION": p, "NORMAL": q}));
            morph_report.push(json!({"name": name, "position": p_kind, "position_entries": p_entries,
                                     "normal": q_kind, "normal_entries": q_entries}));
        }
        let mut primitive = json!({"attributes": {"POSITION": position, "NORMAL": normal, "TEXCOORD_0": uv}, "indices": indices, "mode": 4});
        let mut mesh = json!({"name": lod.name});
        if !targets.is_empty() {
            primitive["targets"] = json!(targets);
            mesh["weights"] = json!(vec![0.0; targets.len()]);
            mesh["extras"] = json!({"targetNames": morph_names});
        }
        mesh["primitives"] = json!([primitive]);
        meshes.push(mesh);
        nodes.push(json!({"name": lod.name, "mesh": meshes.len() - 1}));
        report.push(json!({"node": lod.name, "vertices": n, "triangles": lod.indices.len() / 3,
                           "index_type": if short { "u16" } else { "u32" }, "morphs": morph_report}));
    }
    while b.bin.len() % 4 != 0 {
        b.bin.push(0);
    }
    let document = json!({
        "asset": {"version": "2.0", "generator": generator},
        "scene": 0,
        "scenes": [{"name": "GazeForge", "nodes": (0..nodes.len()).collect::<Vec<_>>(), "extras": {"gazeforge": extras.clone()}}],
        "nodes": nodes,
        "meshes": meshes,
        "accessors": b.accessors,
        "bufferViews": b.views,
        "buffers": [{"byteLength": b.bin.len()}],
        "extras": {"gazeforge": extras},
    });
    let mut text = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
    while text.len() % 4 != 0 {
        text.push(b' ');
    }
    let total = 12 + 8 + text.len() + 8 + b.bin.len();
    let total32 = u32::try_from(total).map_err(|_| format!("the glb would be {total} bytes, over the 4 GiB limit"))?;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&0x4654_6C67u32.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&total32.to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes());
    out.extend_from_slice(&text);
    out.extend_from_slice(&(b.bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes());
    out.extend_from_slice(&b.bin);
    Ok((out, json!(report)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lod(morph: Vec<[f32; 3]>) -> Lod {
        Lod {
            name: "Head_LOD0".into(),
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
            indices: vec![0, 1, 2, 2, 1, 3],
            morphs: vec![MorphData { positions: morph, normals: vec![[0.0; 3]; 4] }],
        }
    }

    fn document(bytes: &[u8]) -> Value {
        let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        serde_json::from_slice(&bytes[20..20 + length]).unwrap()
    }

    #[test]
    fn the_container_is_aligned_and_its_length_is_right() {
        let (bytes, _) = glb(&[lod(vec![[0.0; 3]; 4])], &["m".into()], Sparse::Auto, json!({}), "test").unwrap();
        assert_eq!(&bytes[0..4], b"glTF");
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize, bytes.len());
        assert_eq!(bytes.len() % 4, 0);
        let doc = document(&bytes);
        assert_eq!(doc["meshes"][0]["extras"]["targetNames"][0], "m");
    }

    #[test]
    fn a_morph_on_one_vertex_is_sparse_and_bounds_include_zero() {
        let morph = vec![[0.0; 3], [0.0; 3], [0.0, 0.0, 0.5], [0.0; 3]];
        let (bytes, report) = glb(&[lod(morph)], &["m".into()], Sparse::Auto, json!({}), "test").unwrap();
        let doc = document(&bytes);
        let target = doc["meshes"][0]["primitives"][0]["targets"][0]["POSITION"].as_u64().unwrap() as usize;
        let accessor = &doc["accessors"][target];
        assert_eq!(accessor["sparse"]["count"], 1);
        assert_eq!(accessor["min"], json!([0.0, 0.0, 0.0]));
        assert_eq!(accessor["max"], json!([0.0, 0.0, 0.5]));
        assert_eq!(report[0]["morphs"][0]["position"], "sparse");
        let (_, dense) = glb(&[lod(vec![[0.1; 3]; 4])], &["m".into()], Sparse::Auto, json!({}), "test").unwrap();
        assert_eq!(dense[0]["morphs"][0]["position"], "dense");
    }

    #[test]
    fn bad_indices_are_refused() {
        let mut bad = lod(vec![[0.0; 3]; 4]);
        bad.indices[0] = 9;
        assert!(glb(&[bad], &["m".into()], Sparse::Auto, json!({}), "test").is_err());
    }
}
