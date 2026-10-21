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

#[derive(Clone)]
pub struct MorphData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
}

#[derive(Clone)]
pub struct SkinData {
    pub joints: Vec<[u8; 4]>,
    pub weights: Vec<[f32; 4]>,
    pub joints_1: Vec<[u8; 4]>,
    pub weights_1: Vec<[f32; 4]>,
}

#[derive(Clone)]
pub struct Lod {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub morphs: Vec<MorphData>,
    pub skin: Option<SkinData>,
    pub material: Option<usize>,
    pub custom: Vec<(String, Vec<f32>)>,
}

pub struct Attachment {
    pub name: String,
    pub joint: usize,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

pub struct Channel {
    pub joint: usize,
    pub rotation: [[f32; 4]; 2],
    pub translation: [[f32; 3]; 2],
}

pub struct Animation {
    pub name: String,
    pub channels: Vec<Channel>,
    pub morph: Option<usize>,
}

pub struct Rig {
    pub names: Vec<String>,
    pub parents: Vec<Option<usize>>,
    pub global: Vec<[f32; 3]>,
    pub attachments: Vec<Attachment>,
    pub animations: Vec<Animation>,
}

impl Rig {
    pub fn local(&self) -> Vec<[f32; 3]> {
        (0..self.names.len())
            .map(|j| match self.parents[j] {
                Some(p) => [0, 1, 2].map(|k| self.global[j][k] - self.global[p][k]),
                None => self.global[j],
            })
            .collect()
    }

    pub fn engine_global(&self) -> Vec<[f32; 3]> {
        let local = self.local();
        let mut out = local.clone();
        for j in 0..self.names.len() {
            if let Some(p) = self.parents[j] {
                out[j] = [0, 1, 2].map(|k| out[p][k] + local[j][k]);
            }
        }
        out
    }
}

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const UNSIGNED_BYTE: u32 = 5121;
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

    fn raw(&mut self, bytes: &[u8], target: Option<u32>, component: u32, count: usize, kind: &str) -> usize {
        let view = self.view(bytes, target);
        self.accessors.push(json!({"bufferView": view, "componentType": component, "count": count, "type": kind}));
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
    glb_rigged(lods, morph_names, mode, extras, generator, None)
}

pub fn glb_rigged(lods: &[Lod], morph_names: &[String], mode: Sparse, extras: Value, generator: &str, rig: Option<&Rig>) -> Result<(Vec<u8>, Value), String> {
    glb_full(lods, &[], morph_names, mode, extras, generator, rig)
}

pub struct Material {
    pub name: String,
    pub base_color: [f32; 4],
    pub base_texture: Option<Vec<u8>>,
    pub normal_texture: Option<Vec<u8>>,
    pub orm_texture: Option<Vec<u8>>,
    pub metallic: f32,
    pub roughness: f32,
    pub transmission: Option<f32>,
    pub ior: Option<f32>,
    pub blend: bool,
    pub alpha_cutoff: Option<f32>,
    pub double_sided: bool,
    pub extension: Option<MaterialExtension>,
}

pub struct MaterialExtension {
    pub name: &'static str,
    /// Every object in `value` with an integer `index` names a slot of `textures`; the writer turns it into the glTF texture index.
    pub value: Value,
    pub textures: Vec<Vec<u8>>,
}

pub struct PartMesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub parts: Vec<(Vec<u32>, usize)>,
    pub joint: Option<usize>,
    pub translation: [f32; 3],
}

pub fn glb_full(
    lods: &[Lod],
    extra: &[Lod],
    morph_names: &[String],
    mode: Sparse,
    extras: Value,
    generator: &str,
    rig: Option<&Rig>,
) -> Result<(Vec<u8>, Value), String> {
    glb_with_parts(lods, extra, morph_names, mode, extras, generator, rig, &[], &[])
}

fn material_json(b: &mut Builder, m: &Material, textures: &mut Vec<Value>, images: &mut Vec<Value>, used: &mut Vec<&'static str>) -> Result<Value, String> {
    let mut pbr = json!({"baseColorFactor": m.base_color, "metallicFactor": m.metallic, "roughnessFactor": m.roughness});
    let mut texture = |png: &Vec<u8>, suffix: &str| -> usize {
        let view = b.view(png, None);
        images.push(json!({"bufferView": view, "mimeType": "image/png", "name": format!("{}{suffix}", m.name)}));
        textures.push(json!({"source": images.len() - 1, "sampler": 0}));
        textures.len() - 1
    };
    if let Some(png) = &m.base_texture {
        pbr["baseColorTexture"] = json!({"index": texture(png, "")});
    }
    let normal = m.normal_texture.as_ref().map(|png| texture(png, "_normal"));
    let orm = m.orm_texture.as_ref().map(|png| texture(png, "_orm"));
    if let Some(i) = orm {
        pbr["metallicRoughnessTexture"] = json!({"index": i});
    }
    let mut out = json!({"name": m.name, "pbrMetallicRoughness": pbr});
    if let Some(i) = normal {
        out["normalTexture"] = json!({"index": i});
    }
    if let Some(i) = orm {
        out["occlusionTexture"] = json!({"index": i});
    }
    if m.double_sided {
        out["doubleSided"] = json!(true);
    }
    if let Some(cutoff) = m.alpha_cutoff {
        out["alphaMode"] = json!("MASK");
        out["alphaCutoff"] = json!(cutoff);
    } else if m.blend {
        out["alphaMode"] = json!("BLEND");
    }
    let mut ext = serde_json::Map::new();
    if let Some(t) = m.transmission {
        ext.insert("KHR_materials_transmission".into(), json!({"transmissionFactor": t}));
        used.push("KHR_materials_transmission");
    }
    if let Some(i) = m.ior {
        ext.insert("KHR_materials_ior".into(), json!({"ior": i}));
        used.push("KHR_materials_ior");
    }
    if let Some(e) = &m.extension {
        let slots: Vec<usize> = e.textures.iter().enumerate().map(|(k, png)| texture(png, &format!("_{}{k}", e.name.to_lowercase()))).collect();
        let mut value = e.value.clone();
        relink(&mut value, &slots)?;
        ext.insert(e.name.into(), value);
        used.push(e.name);
    }
    if !ext.is_empty() {
        out["extensions"] = Value::Object(ext);
    }
    Ok(out)
}

fn relink(value: &mut Value, slots: &[usize]) -> Result<(), String> {
    match value {
        Value::Object(map) => {
            if let Some(index) = map.get_mut("index").filter(|i| i.is_u64()) {
                let slot = index.as_u64().unwrap_or(0) as usize;
                *index = json!(*slots.get(slot).ok_or_else(|| format!("an extension names texture slot {slot}, it has {}", slots.len()))?);
            }
            map.values_mut().filter(|v| v.is_object() || v.is_array()).try_for_each(|v| relink(v, slots))
        }
        Value::Array(items) => items.iter_mut().try_for_each(|v| relink(v, slots)),
        _ => Ok(()),
    }
}

pub struct Root {
    pub name: String,
    pub translation: [f64; 3],
    pub rotation: [f64; 4],
    pub scale: f64,
}

#[allow(clippy::too_many_arguments)]
pub fn glb_with_parts(
    lods: &[Lod],
    extra: &[Lod],
    morph_names: &[String],
    mode: Sparse,
    extras: Value,
    generator: &str,
    rig: Option<&Rig>,
    part_meshes: &[PartMesh],
    materials: &[Material],
) -> Result<(Vec<u8>, Value), String> {
    glb_with_root(lods, extra, morph_names, mode, extras, generator, rig, part_meshes, materials, None)
}

#[allow(clippy::too_many_arguments)]
pub fn glb_with_root(
    lods: &[Lod],
    extra: &[Lod],
    morph_names: &[String],
    mode: Sparse,
    extras: Value,
    generator: &str,
    rig: Option<&Rig>,
    part_meshes: &[PartMesh],
    materials: &[Material],
    root: Option<&Root>,
) -> Result<(Vec<u8>, Value), String> {
    for p in part_meshes {
        let n = p.positions.len();
        if p.normals.len() != n || p.uvs.len() != n {
            return Err(format!("{}: attribute counts disagree", p.name));
        }
        if p.parts.iter().any(|(idx, mat)| *mat >= materials.len() || idx.len() % 3 != 0 || idx.iter().any(|&i| i as usize >= n)) {
            return Err(format!("{}: a part names a missing material or indexes past its vertices", p.name));
        }
        if p.joint.is_some_and(|j| rig.is_none_or(|r| j >= r.names.len())) {
            return Err(format!("{}: attached to a joint the rig does not have", p.name));
        }
    }
    let mut b = Builder::default();
    let mut meshes = Vec::new();
    let mut nodes = Vec::new();
    let mut report = Vec::new();
    let no_morphs: Vec<String> = Vec::new();
    for (lod, morph_names) in lods.iter().map(|l| (l, morph_names)).chain(extra.iter().map(|l| (l, &no_morphs[..]))) {
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
        if let Some(m) = lod.material {
            if m >= materials.len() {
                return Err(format!("{}: names material {m}, there are {}", lod.name, materials.len()));
            }
            primitive["material"] = json!(m);
        }
        if let Some(skin) = &lod.skin {
            if rig.is_none() || skin.joints.len() != n || skin.weights.len() != n {
                return Err(format!("{}: skin data without a rig or with the wrong vertex count", lod.name));
            }
            let joint_bytes: Vec<u8> = skin.joints.iter().flatten().copied().collect();
            let weight_bytes: Vec<u8> = skin.weights.iter().flat_map(|w| w.iter().flat_map(|x| x.to_le_bytes())).collect();
            primitive["attributes"]["JOINTS_0"] = json!(b.raw(&joint_bytes, Some(ARRAY_BUFFER), UNSIGNED_BYTE, n, "VEC4"));
            primitive["attributes"]["WEIGHTS_0"] = json!(b.raw(&weight_bytes, Some(ARRAY_BUFFER), FLOAT, n, "VEC4"));
            if !skin.joints_1.is_empty() {
                if skin.joints_1.len() != n || skin.weights_1.len() != n {
                    return Err(format!("{}: second skin set with the wrong vertex count", lod.name));
                }
                let joint_bytes: Vec<u8> = skin.joints_1.iter().flatten().copied().collect();
                let weight_bytes: Vec<u8> = skin.weights_1.iter().flat_map(|w| w.iter().flat_map(|x| x.to_le_bytes())).collect();
                primitive["attributes"]["JOINTS_1"] = json!(b.raw(&joint_bytes, Some(ARRAY_BUFFER), UNSIGNED_BYTE, n, "VEC4"));
                primitive["attributes"]["WEIGHTS_1"] = json!(b.raw(&weight_bytes, Some(ARRAY_BUFFER), FLOAT, n, "VEC4"));
            }
        }
        for (name, values) in &lod.custom {
            if !name.starts_with('_') || values.len() != n {
                return Err(format!("{}: custom attribute {name} must start with _ and have one value per vertex", lod.name));
            }
            let bytes: Vec<u8> = values.iter().flat_map(|x| x.to_le_bytes()).collect();
            primitive["attributes"][name] = json!(b.raw(&bytes, Some(ARRAY_BUFFER), FLOAT, n, "SCALAR"));
        }
        let mut mesh = json!({"name": lod.name});
        if !targets.is_empty() {
            primitive["targets"] = json!(targets);
            mesh["weights"] = json!(vec![0.0; targets.len()]);
            mesh["extras"] = json!({"targetNames": morph_names});
        }
        mesh["primitives"] = json!([primitive]);
        meshes.push(mesh);
        let mut node = json!({"name": lod.name, "mesh": meshes.len() - 1});
        if lod.skin.is_some() {
            node["skin"] = json!(0);
        }
        nodes.push(node);
        report.push(json!({"node": lod.name, "vertices": n, "triangles": lod.indices.len() / 3,
                           "index_type": if short { "u16" } else { "u32" }, "morphs": morph_report}));
    }
    let mut roots: Vec<usize> = (0..nodes.len()).collect();
    let mut skins = Vec::new();
    let mut animations: Vec<Value> = Vec::new();
    let part_first = nodes.len() + rig.map_or(0, |r| r.names.len() + r.attachments.len());
    if let Some(rig) = rig {
        let first = nodes.len();
        let local = rig.local();
        let engine = rig.engine_global();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); rig.names.len()];
        for (j, parent) in rig.parents.iter().enumerate() {
            match parent {
                Some(p) => children[*p].push(first + j),
                None => roots.push(first + j),
            }
        }
        let attachment_first = first + rig.names.len();
        for (a, attachment) in rig.attachments.iter().enumerate() {
            children.get_mut(attachment.joint).ok_or("an attachment names a joint the rig does not have")?.push(attachment_first + a);
        }
        for (k, p) in part_meshes.iter().enumerate() {
            if let Some(j) = p.joint {
                children[j].push(part_first + k);
            }
        }
        for j in 0..rig.names.len() {
            let mut node = json!({"name": rig.names[j], "translation": local[j]});
            if !children[j].is_empty() {
                node["children"] = json!(children[j]);
            }
            nodes.push(node);
        }
        for attachment in &rig.attachments {
            let short = attachment.positions.len() <= u16::MAX as usize;
            let position = b.floats(&attachment.positions, Some(ARRAY_BUFFER), true);
            let normal = b.floats(&attachment.normals, Some(ARRAY_BUFFER), false);
            let indices = b.indices(&attachment.indices, short);
            meshes.push(json!({"name": attachment.name, "primitives": [{"attributes": {"POSITION": position, "NORMAL": normal}, "indices": indices, "mode": 4}]}));
            nodes.push(json!({"name": attachment.name, "mesh": meshes.len() - 1}));
        }
        let matrices: Vec<u8> = engine
            .iter()
            .flat_map(|t| [1.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -t[0], -t[1], -t[2], 1.0])
            .flat_map(|x| x.to_le_bytes())
            .collect();
        let ibm = b.raw(&matrices, None, FLOAT, rig.names.len(), "MAT4");
        skins.push(json!({"name": "head_rig", "inverseBindMatrices": ibm, "joints": (first..first + rig.names.len()).collect::<Vec<_>>(),
                          "skeleton": first + rig.parents.iter().position(|p| p.is_none()).unwrap_or(0)}));
        if !rig.animations.is_empty() {
            let times: Vec<u8> = [0.0f32, 1.0].iter().flat_map(|x| x.to_le_bytes()).collect();
            let input = b.raw(&times, None, FLOAT, 2, "SCALAR");
            b.accessors[input]["min"] = json!([0.0]);
            b.accessors[input]["max"] = json!([1.0]);
            let mut data: Vec<u8> = Vec::new();
            let mut layout: Vec<(usize, &str)> = Vec::new();
            for animation in &rig.animations {
                for c in &animation.channels {
                    if c.joint >= rig.names.len() {
                        return Err(format!("animation {} names a joint the rig does not have", animation.name));
                    }
                    layout.push((data.len(), "VEC4"));
                    data.extend(c.rotation.iter().flatten().flat_map(|x| x.to_le_bytes()));
                    layout.push((data.len(), "VEC3"));
                    data.extend(c.translation.iter().flatten().flat_map(|x| x.to_le_bytes()));
                }
            }
            let view = b.view(&data, None);
            let mut outputs = Vec::with_capacity(layout.len());
            for &(offset, kind) in &layout {
                b.accessors.push(json!({"bufferView": view, "byteOffset": offset, "componentType": FLOAT, "count": 2, "type": kind}));
                outputs.push(b.accessors.len() - 1);
            }
            let mut next = outputs.into_iter();
            for animation in &rig.animations {
                let mut samplers = Vec::new();
                let mut channels = Vec::new();
                for c in &animation.channels {
                    for path in ["rotation", "translation"] {
                        let output = next.next().ok_or("animation outputs ran out")?;
                        channels.push(json!({"sampler": samplers.len(), "target": {"node": first + c.joint, "path": path}}));
                        samplers.push(json!({"input": input, "output": output, "interpolation": "LINEAR"}));
                    }
                }
                if let Some(k) = animation.morph {
                    let m = morph_names.len();
                    if k >= m {
                        return Err(format!("animation {} drives a morph target the meshes do not have", animation.name));
                    }
                    let mut keys = vec![0.0f32; 2 * m];
                    keys[m + k] = 1.0;
                    let bytes: Vec<u8> = keys.iter().flat_map(|x| x.to_le_bytes()).collect();
                    let output = b.raw(&bytes, None, FLOAT, 2 * m, "SCALAR");
                    for (node, lod) in lods.iter().enumerate() {
                        if !lod.morphs.is_empty() {
                            channels.push(json!({"sampler": samplers.len(), "target": {"node": node, "path": "weights"}}));
                            samplers.push(json!({"input": input, "output": output, "interpolation": "LINEAR"}));
                        }
                    }
                }
                animations.push(json!({"name": animation.name, "samplers": samplers, "channels": channels}));
            }
        }
    }
    for (k, p) in part_meshes.iter().enumerate() {
        let short = p.positions.len() <= u16::MAX as usize;
        let position = b.floats(&p.positions, Some(ARRAY_BUFFER), true);
        let normal = b.floats(&p.normals, Some(ARRAY_BUFFER), false);
        let uv = b.floats(&p.uvs, Some(ARRAY_BUFFER), false);
        let primitives: Vec<Value> = p
            .parts
            .iter()
            .map(|(idx, mat)| json!({"attributes": {"POSITION": position, "NORMAL": normal, "TEXCOORD_0": uv}, "indices": b.indices(idx, short), "material": mat, "mode": 4}))
            .collect();
        meshes.push(json!({"name": p.name, "primitives": primitives}));
        nodes.push(json!({"name": p.name, "mesh": meshes.len() - 1, "translation": p.translation}));
        if p.joint.is_none() {
            roots.push(part_first + k);
        }
        report.push(json!({"node": p.name, "vertices": p.positions.len(), "primitives": p.parts.len(), "joint": p.joint}));
    }
    if let Some(r) = root {
        nodes.push(json!({"name": r.name, "children": roots, "translation": r.translation, "rotation": r.rotation, "scale": [r.scale, r.scale, r.scale]}));
        roots = vec![nodes.len() - 1];
    }
    let (mut material_values, mut textures, mut images, mut used) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for m in materials {
        let v = material_json(&mut b, m, &mut textures, &mut images, &mut used)?;
        material_values.push(v);
    }
    used.sort();
    used.dedup();
    while b.bin.len() % 4 != 0 {
        b.bin.push(0);
    }
    let mut document = json!({
        "asset": {"version": "2.0", "generator": generator},
        "scene": 0,
        "scenes": [{"name": "GazeForge", "nodes": roots, "extras": {"gazeforge": extras.clone()}}],
        "nodes": nodes,
        "meshes": meshes,
        "accessors": b.accessors,
        "bufferViews": b.views,
        "buffers": [{"byteLength": b.bin.len()}],
        "extras": {"gazeforge": extras},
    });
    if !skins.is_empty() {
        document["skins"] = json!(skins);
    }
    if !animations.is_empty() {
        document["animations"] = json!(animations);
    }
    if !material_values.is_empty() {
        document["materials"] = json!(material_values);
    }
    if !images.is_empty() {
        document["images"] = json!(images);
        document["textures"] = json!(textures);
        document["samplers"] = json!([{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 33071}]);
    }
    if !used.is_empty() {
        document["extensionsUsed"] = json!(used);
    }
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

pub struct ReadMesh {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub morph_names: Vec<String>,
    pub morphs: Vec<MorphData>,
}

fn accessor_bytes<'a>(doc: &Value, bin: &'a [u8], view: &Value, offset: u64, length: usize) -> Result<&'a [u8], String> {
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize + offset as usize;
    let _ = doc;
    bin.get(start..start + length).ok_or_else(|| "an accessor runs past the binary chunk".to_string())
}

fn read_vec3(doc: &Value, bin: &[u8], index: u64) -> Result<Vec<[f32; 3]>, String> {
    let a = &doc["accessors"][index as usize];
    if a["componentType"].as_u64() != Some(FLOAT as u64) || a["type"] != "VEC3" {
        return Err(format!("accessor {index} is not a float VEC3"));
    }
    let count = a["count"].as_u64().ok_or("an accessor has no count")? as usize;
    let floats = |bytes: &[u8]| -> Vec<[f32; 3]> {
        bytes.chunks_exact(12).map(|c| [0, 4, 8].map(|o| f32::from_le_bytes(c[o..o + 4].try_into().unwrap()))).collect()
    };
    let mut out = match a.get("bufferView").and_then(Value::as_u64) {
        Some(v) => {
            let view = &doc["bufferViews"][v as usize];
            if view.get("byteStride").and_then(Value::as_u64).is_some_and(|s| s != 12) {
                return Err(format!("accessor {index} is interleaved; only tightly packed data is read"));
            }
            floats(accessor_bytes(doc, bin, view, a["byteOffset"].as_u64().unwrap_or(0), count * 12)?)
        }
        None => vec![[0.0; 3]; count],
    };
    if let Some(sparse) = a.get("sparse") {
        let n = sparse["count"].as_u64().ok_or("a sparse accessor has no count")? as usize;
        let iv = &doc["bufferViews"][sparse["indices"]["bufferView"].as_u64().ok_or("sparse indices have no view")? as usize];
        let width = if sparse["indices"]["componentType"].as_u64() == Some(UNSIGNED_SHORT as u64) { 2 } else { 4 };
        let ib = accessor_bytes(doc, bin, iv, sparse["indices"]["byteOffset"].as_u64().unwrap_or(0), n * width)?;
        let vv = &doc["bufferViews"][sparse["values"]["bufferView"].as_u64().ok_or("sparse values have no view")? as usize];
        let values = floats(accessor_bytes(doc, bin, vv, sparse["values"]["byteOffset"].as_u64().unwrap_or(0), n * 12)?);
        for (k, value) in values.into_iter().enumerate() {
            let i = if width == 2 { u16::from_le_bytes([ib[2 * k], ib[2 * k + 1]]) as usize } else { u32::from_le_bytes(ib[4 * k..4 * k + 4].try_into().unwrap()) as usize };
            *out.get_mut(i).ok_or("a sparse index is out of range")? = value;
        }
    }
    Ok(out)
}

fn read_indices(doc: &Value, bin: &[u8], index: u64) -> Result<Vec<u32>, String> {
    let a = &doc["accessors"][index as usize];
    let count = a["count"].as_u64().ok_or("an accessor has no count")? as usize;
    let view = &doc["bufferViews"][a["bufferView"].as_u64().ok_or("indices have no view")? as usize];
    let offset = a["byteOffset"].as_u64().unwrap_or(0);
    match a["componentType"].as_u64().map(|c| c as u32) {
        Some(UNSIGNED_SHORT) => Ok(accessor_bytes(doc, bin, view, offset, count * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as u32).collect()),
        Some(UNSIGNED_INT) => Ok(accessor_bytes(doc, bin, view, offset, count * 4)?.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()),
        _ => Err(format!("accessor {index}: indices must be u16 or u32")),
    }
}

pub fn chunks(bytes: &[u8]) -> Result<(Value, &[u8]), String> {
    let word = |at: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(bytes.get(at..at + 4).ok_or("the file is shorter than a glb header")?.try_into().unwrap()))
    };
    if word(0)? != 0x4654_6C67 || word(4)? != 2 {
        return Err("not a glTF 2.0 binary".into());
    }
    let json_len = word(12)? as usize;
    let doc: Value = serde_json::from_slice(bytes.get(20..20 + json_len).ok_or("the JSON chunk is truncated")?).map_err(|e| e.to_string())?;
    let bin_len = word(20 + json_len)? as usize;
    let bin = bytes.get(28 + json_len..28 + json_len + bin_len).ok_or("the binary chunk is truncated")?;
    Ok((doc, bin))
}

pub fn read_plain(doc: &Value, bin: &[u8], index: u64) -> Result<Vec<f32>, String> {
    let a = &doc["accessors"][index as usize];
    let width = match a["type"].as_str() {
        Some("SCALAR") => 1,
        Some("VEC2") => 2,
        Some("VEC3") => 3,
        Some("VEC4") => 4,
        Some("MAT4") => 16,
        _ => return Err(format!("accessor {index} has an unknown type")),
    };
    let count = a["count"].as_u64().ok_or("an accessor has no count")? as usize * width;
    let view = &doc["bufferViews"][a["bufferView"].as_u64().ok_or("the accessor has no view")? as usize];
    let offset = a["byteOffset"].as_u64().unwrap_or(0);
    match a["componentType"].as_u64().map(|c| c as u32) {
        Some(FLOAT) => Ok(accessor_bytes(doc, bin, view, offset, count * 4)?.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect()),
        Some(UNSIGNED_BYTE) => Ok(accessor_bytes(doc, bin, view, offset, count)?.iter().map(|&x| x as f32).collect()),
        Some(UNSIGNED_SHORT) => Ok(accessor_bytes(doc, bin, view, offset, count * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]) as f32).collect()),
        _ => Err(format!("accessor {index}: unsupported component type")),
    }
}

pub struct SceneMesh {
    pub positions: Vec<[f64; 3]>,
    pub uvs: Vec<[f64; 2]>,
    pub triangles: Vec<[u32; 3]>,
    pub base_color: Option<(String, Vec<u8>)>,
    pub base_color_uri: Option<String>,
}

type Mat4 = [[f64; 4]; 4];

fn mat_mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut m = [[0.0; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, x) in row.iter_mut().enumerate() {
            *x = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

fn node_matrix(node: &Value) -> Mat4 {
    if let Some(m) = node["matrix"].as_array().filter(|m| m.len() == 16) {
        let f = |k: usize| m[k].as_f64().unwrap_or(0.0);
        return std::array::from_fn(|i| std::array::from_fn(|j| f(j * 4 + i)));
    }
    let v = |key: &str, d: &[f64]| -> Vec<f64> { node[key].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_else(|| d.to_vec()) };
    let (t, r, sc) = (v("translation", &[0.0, 0.0, 0.0]), v("rotation", &[0.0, 0.0, 0.0, 1.0]), v("scale", &[1.0, 1.0, 1.0]));
    let (x, y, z, w) = (r[0], r[1], r[2], r[3]);
    let rot = [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
        [2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
        [2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
    ];
    let mut m = [[0.0; 4]; 4];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = rot[i][j] * sc[j];
        }
        m[i][3] = t[i];
    }
    m[3][3] = 1.0;
    m
}

pub fn read_glb_scene(bytes: &[u8]) -> Result<SceneMesh, String> {
    let (doc, bin) = chunks(bytes)?;
    let mut out = SceneMesh { positions: Vec::new(), uvs: Vec::new(), triangles: Vec::new(), base_color: None, base_color_uri: None };
    let scene = doc["scene"].as_u64().unwrap_or(0) as usize;
    let roots: Vec<u64> = match doc["scenes"][scene]["nodes"].as_array() {
        Some(n) => n.iter().filter_map(Value::as_u64).collect(),
        None => (0..doc["nodes"].as_array().map_or(0, Vec::len) as u64).collect(),
    };
    let identity: Mat4 = std::array::from_fn(|i| std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 }));
    let mut stack: Vec<(u64, Mat4)> = roots.into_iter().map(|n| (n, identity)).collect();
    while let Some((n, parent)) = stack.pop() {
        let node = &doc["nodes"][n as usize];
        let world = mat_mul(&parent, &node_matrix(node));
        for c in node["children"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
            stack.push((c, world));
        }
        let Some(mesh) = node["mesh"].as_u64() else { continue };
        for primitive in doc["meshes"][mesh as usize]["primitives"].as_array().into_iter().flatten() {
            if primitive["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            let attributes = &primitive["attributes"];
            let p = read_vec3(&doc, bin, attributes["POSITION"].as_u64().ok_or("a primitive has no POSITION")?)?;
            let uv: Vec<f32> = match attributes["TEXCOORD_0"].as_u64() {
                Some(i) => read_plain(&doc, bin, i)?,
                None => Vec::new(),
            };
            let offset = out.positions.len() as u32;
            for (k, q) in p.iter().enumerate() {
                let v = [q[0] as f64, q[1] as f64, q[2] as f64];
                let w: [f64; 3] = std::array::from_fn(|i| world[i][0] * v[0] + world[i][1] * v[1] + world[i][2] * v[2] + world[i][3]);
                out.positions.push([w[0], -w[2], w[1]]);
                out.uvs.push(if uv.len() >= 2 * (k + 1) { [uv[2 * k] as f64, 1.0 - uv[2 * k + 1] as f64] } else { [0.0, 0.0] });
            }
            let idx = match primitive["indices"].as_u64() {
                Some(i) => read_indices(&doc, bin, i)?,
                None => (0..p.len() as u32).collect(),
            };
            out.triangles.extend(idx.chunks_exact(3).map(|t| [t[0] + offset, t[1] + offset, t[2] + offset]));
            if out.base_color.is_none()
                && let Some(t) = primitive["material"].as_u64().and_then(|m| doc["materials"][m as usize]["pbrMetallicRoughness"]["baseColorTexture"]["index"].as_u64())
                && let Some(img) = doc["textures"][t as usize]["source"].as_u64()
            {
                let image = &doc["images"][img as usize];
                if let Some(view) = image["bufferView"].as_u64() {
                    let v = &doc["bufferViews"][view as usize];
                    let data = accessor_bytes(&doc, bin, v, 0, v["byteLength"].as_u64().unwrap_or(0) as usize)?;
                    out.base_color = Some((image["mimeType"].as_str().unwrap_or("").to_string(), data.to_vec()));
                } else if let Some(uri) = image["uri"].as_str()
                    && out.base_color_uri.is_none()
                {
                    out.base_color_uri = Some(if uri.starts_with("data:") { "a data URI".into() } else { uri.to_string() });
                }
            }
        }
    }
    if out.triangles.is_empty() {
        return Err("the glb has no triangle meshes in its scene".into());
    }
    Ok(out)
}

pub fn scene_obj_text(m: &SceneMesh) -> String {
    let mut s = String::new();
    for p in &m.positions {
        s.push_str(&format!("v {} {} {}\n", p[0], p[1], p[2]));
    }
    for t in &m.uvs {
        s.push_str(&format!("vt {} {}\n", t[0], t[1]));
    }
    for t in &m.triangles {
        s.push_str(&format!("f {}/{} {}/{} {}/{}\n", t[0] + 1, t[0] + 1, t[1] + 1, t[1] + 1, t[2] + 1, t[2] + 1));
    }
    s
}

pub fn with_extension(glb: &[u8], name: &str, value: Value) -> Result<Vec<u8>, String> {
    let (mut doc, bin) = chunks(glb)?;
    let bin = bin.to_vec();
    doc["extensions"][name] = value;
    let used = doc["extensionsUsed"].as_array().cloned().unwrap_or_default();
    if !used.iter().any(|u| u == name) {
        let mut used = used;
        used.push(json!(name));
        doc["extensionsUsed"] = json!(used);
    }
    let mut text = serde_json::to_vec(&doc).map_err(|e| e.to_string())?;
    while text.len() % 4 != 0 {
        text.push(b' ');
    }
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

pub fn read_glb(bytes: &[u8]) -> Result<(Value, Vec<ReadMesh>), String> {
    let word = |at: usize| -> Result<u32, String> {
        Ok(u32::from_le_bytes(bytes.get(at..at + 4).ok_or("the file is shorter than a glb header")?.try_into().unwrap()))
    };
    if word(0)? != 0x4654_6C67 || word(4)? != 2 {
        return Err("not a glTF 2.0 binary".into());
    }
    let json_len = word(12)? as usize;
    let doc: Value = serde_json::from_slice(bytes.get(20..20 + json_len).ok_or("the JSON chunk is truncated")?).map_err(|e| e.to_string())?;
    let bin_len = word(20 + json_len)? as usize;
    let bin = bytes.get(28 + json_len..28 + json_len + bin_len).ok_or("the binary chunk is truncated")?;
    let mut meshes = Vec::new();
    for mesh in doc["meshes"].as_array().ok_or("the glb has no meshes")? {
        let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::new());
        let mut morphs: Vec<MorphData> = Vec::new();
        let mut groups: Vec<(&Value, u32)> = Vec::new();
        for primitive in mesh["primitives"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let attributes = &primitive["attributes"];
            let offset = match groups.iter().find(|(a, _)| *a == attributes) {
                Some((_, o)) => *o,
                None => {
                    let offset = positions.len() as u32;
                    let attribute = |name: &str| attributes[name].as_u64();
                    let p = read_vec3(&doc, bin, attribute("POSITION").ok_or("a primitive has no POSITION")?)?;
                    let n = match attribute("NORMAL") {
                        Some(i) => read_vec3(&doc, bin, i)?,
                        None => Vec::new(),
                    };
                    if !n.is_empty() || !normals.is_empty() {
                        normals.resize(positions.len(), [0.0; 3]);
                        normals.extend(if n.is_empty() { vec![[0.0; 3]; p.len()] } else { n });
                    }
                    let targets = primitive["targets"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                    if morphs.len() < targets.len() {
                        morphs.resize_with(targets.len(), || MorphData { positions: vec![[0.0; 3]; positions.len()], normals: vec![[0.0; 3]; positions.len()] });
                    }
                    for (k, m) in morphs.iter_mut().enumerate() {
                        let (dp, dn) = match targets.get(k) {
                            Some(target) => {
                                let dp = read_vec3(&doc, bin, target["POSITION"].as_u64().ok_or("a morph target has no POSITION")?)?;
                                let dn = match target["NORMAL"].as_u64() {
                                    Some(i) => read_vec3(&doc, bin, i)?,
                                    None => vec![[0.0; 3]; dp.len()],
                                };
                                (dp, dn)
                            }
                            None => (vec![[0.0; 3]; p.len()], vec![[0.0; 3]; p.len()]),
                        };
                        m.positions.extend(dp);
                        m.normals.extend(dn);
                    }
                    positions.extend(p);
                    groups.push((attributes, offset));
                    offset
                }
            };
            indices.extend(read_indices(&doc, bin, primitive["indices"].as_u64().ok_or("a primitive has no indices")?)?.into_iter().map(|i| i + offset));
        }
        if positions.is_empty() {
            return Err(format!("mesh {} has no primitives", mesh["name"].as_str().unwrap_or("")));
        }
        let morph_names = mesh["extras"]["targetNames"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        meshes.push(ReadMesh { name: mesh["name"].as_str().unwrap_or("").to_string(), positions, normals, indices, morph_names, morphs });
    }
    Ok((doc, meshes))
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
            skin: None,
            material: None,
            custom: Vec::new(),
        }
    }

    #[test]
    fn custom_attributes_and_a_root_extension_survive_a_round_trip() {
        let mut l = lod(vec![[0.0; 3]; 4]);
        l.custom = vec![("_PIN".into(), vec![1.0, 0.5, 0.0, 0.25])];
        let (bytes, _) = glb(&[l], &["m".into()], Sparse::Never, json!({}), "test").unwrap();
        let bytes = with_extension(&bytes, "GAZEFORGE_physics", json!({"version": 1})).unwrap();
        let (doc, bin) = chunks(&bytes).unwrap();
        assert_eq!(doc["extensions"]["GAZEFORGE_physics"]["version"], 1);
        assert_eq!(doc["extensionsUsed"], json!(["GAZEFORGE_physics"]));
        let pin = doc["meshes"][0]["primitives"][0]["attributes"]["_PIN"].as_u64().unwrap();
        assert_eq!(read_plain(&doc, bin, pin).unwrap(), vec![1.0, 0.5, 0.0, 0.25]);
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize, bytes.len());
        assert_eq!(read_glb(&bytes).unwrap().1[0].positions.len(), 4);
        let again = with_extension(&bytes, "GAZEFORGE_physics", json!({"version": 2})).unwrap();
        assert_eq!(chunks(&again).unwrap().0["extensionsUsed"], json!(["GAZEFORGE_physics"]), "listed once");
        let mut bad = lod(vec![[0.0; 3]; 4]);
        bad.custom = vec![("PIN".into(), vec![0.0; 4])];
        assert!(glb(&[bad], &["m".into()], Sparse::Never, json!({}), "test").unwrap_err().contains("must start with _"));
    }

    #[test]
    fn a_rigged_glb_reads_back_and_its_bind_matrices_invert_the_joints() {
        let mut skinned = lod(vec![[0.0, 0.0, 0.5], [0.0; 3], [0.0; 3], [0.0; 3]]);
        skinned.skin = Some(SkinData { joints: vec![[0, 1, 0, 0]; 4], weights: vec![[0.25, 0.75, 0.0, 0.0]; 4], joints_1: Vec::new(), weights_1: Vec::new() });
        let rig = Rig {
            names: vec!["neck".into(), "head".into()],
            parents: vec![None, Some(0)],
            global: vec![[0.1, 1.3, 0.0], [0.1, 1.5, 0.01]],
            attachments: vec![Attachment { name: "Eye".into(), joint: 1, positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                                           normals: vec![[0.0, 0.0, 1.0]; 3], indices: vec![0, 1, 2] }],
            animations: Vec::new(),
        };
        let (bytes, _) = glb_rigged(&[skinned], &["m".into()], Sparse::Always, json!({}), "test", Some(&rig)).unwrap();
        let (doc, meshes) = read_glb(&bytes).unwrap();
        assert_eq!(meshes.len(), 2);
        assert_eq!(meshes[0].morphs[0].positions[0], [0.0, 0.0, 0.5]);
        assert_eq!(meshes[0].positions[3], [1.0, 1.0, 0.0]);
        assert_eq!(doc["nodes"][0]["skin"], 0);
        assert_eq!(doc["skins"][0]["joints"], json!([1, 2]));
        assert_eq!(doc["nodes"][2]["children"], json!([3]));
        assert_eq!(doc["scenes"][0]["nodes"], json!([0, 1]));
        let engine = rig.engine_global();
        assert_eq!([0, 1, 2].map(|k| engine[1][k] + -engine[1][k]), [0.0; 3]);
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

    fn material(extension: Option<MaterialExtension>) -> Material {
        Material {
            name: "head_skin".into(),
            base_color: [1.0; 4],
            base_texture: Some(vec![1, 2, 3]),
            normal_texture: Some(vec![4, 5]),
            orm_texture: None,
            metallic: 0.0,
            roughness: 1.0,
            transmission: None,
            ior: None,
            blend: false,
            alpha_cutoff: None,
            double_sided: false,
            extension,
        }
    }

    #[test]
    fn a_material_extension_gets_its_textures_relinked_and_the_lod_its_material() {
        let value = json!({"version": 1, "live": [{"mask": {"index": 1, "channel": 2}}, {"mask": {"index": 0, "channel": 0}}]});
        let ext = MaterialExtension { name: "GAZEFORGE_skin", value, textures: vec![vec![9; 5], vec![8; 6]] };
        let mut head = lod(vec![[0.0; 3]; 4]);
        head.material = Some(0);
        let (bytes, _) = glb_with_parts(&[head], &[], &["m".into()], Sparse::Auto, json!({}), "test", None, &[], &[material(Some(ext))]).unwrap();
        let doc = document(&bytes);
        assert_eq!(doc["meshes"][0]["primitives"][0]["material"], 0);
        assert_eq!(doc["extensionsUsed"], json!(["GAZEFORGE_skin"]));
        assert!(doc.get("extensionsRequired").is_none());
        let e = &doc["materials"][0]["extensions"]["GAZEFORGE_skin"];
        assert_eq!(e["live"][0]["mask"], json!({"index": 3, "channel": 2}));
        assert_eq!(e["live"][1]["mask"], json!({"index": 2, "channel": 0}));
        assert_eq!(doc["textures"].as_array().unwrap().len(), 4);
        let image = doc["textures"][3]["source"].as_u64().unwrap() as usize;
        let view = &doc["bufferViews"][doc["images"][image]["bufferView"].as_u64().unwrap() as usize];
        assert_eq!(view["byteLength"], 6);
        assert_eq!(doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"], 0);
        assert_eq!(doc["materials"][0]["normalTexture"]["index"], 1);
    }

    #[test]
    fn a_missing_material_or_texture_slot_is_refused() {
        let mut head = lod(vec![[0.0; 3]; 4]);
        head.material = Some(1);
        assert!(glb_with_parts(&[head], &[], &["m".into()], Sparse::Auto, json!({}), "test", None, &[], &[material(None)]).is_err());
        let ext = MaterialExtension { name: "GAZEFORGE_skin", value: json!({"live": [{"mask": {"index": 2}}]}), textures: vec![vec![1]] };
        assert!(glb_with_parts(&[lod(vec![[0.0; 3]; 4])], &[], &["m".into()], Sparse::Auto, json!({}), "test", None, &[], &[material(Some(ext))]).is_err());
    }

    fn repack(doc: &Value, bin: &[u8]) -> Vec<u8> {
        let mut json = serde_json::to_vec(doc).unwrap();
        json.resize(json.len().div_ceil(4) * 4, b' ');
        let mut bin = bin.to_vec();
        bin.resize(bin.len().div_ceil(4) * 4, 0);
        let mut out = Vec::new();
        out.extend(0x4654_6C67u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(0x4E4F_534Au32.to_le_bytes());
        out.extend(json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(0x004E_4942u32.to_le_bytes());
        out.extend(bin);
        out
    }

    #[test]
    fn a_colour_image_outside_the_glb_is_named_not_mistaken_for_none() {
        let mut head = lod(vec![[0.0; 3]; 4]);
        head.material = Some(0);
        let (bytes, _) = glb_with_parts(&[head], &[], &["m".into()], Sparse::Auto, json!({}), "test", None, &[], &[material(None)]).unwrap();
        let inside = read_glb_scene(&bytes).unwrap();
        assert!(inside.base_color.is_some() && inside.base_color_uri.is_none());
        let (mut doc, bin) = chunks(&bytes).unwrap();
        let base = doc["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"]["index"].as_u64().unwrap() as usize;
        let image = doc["textures"][base]["source"].as_u64().unwrap() as usize;
        doc["images"][image] = json!({"uri": "skin_color.jpg"});
        let outside = read_glb_scene(&repack(&doc, bin)).unwrap();
        assert!(outside.base_color.is_none());
        assert_eq!(outside.base_color_uri.as_deref(), Some("skin_color.jpg"));
        doc["images"][image] = json!({"uri": "data:image/png;base64,AAAA"});
        assert_eq!(read_glb_scene(&repack(&doc, bin)).unwrap().base_color_uri.as_deref(), Some("a data URI"));
    }

    #[test]
    fn bad_indices_are_refused() {
        let mut bad = lod(vec![[0.0; 3]; 4]);
        bad.indices[0] = 9;
        assert!(glb(&[bad], &["m".into()], Sparse::Auto, json!({}), "test").is_err());
    }
}
