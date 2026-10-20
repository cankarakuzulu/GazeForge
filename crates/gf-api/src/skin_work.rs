use std::collections::BTreeMap;

use gf_core::skin_authored::{self as sa, Grid, Images};
use gf_core::skin_layers::{self as sl, SkinLayer, SkinSpec, Texel};
use gf_core::skin_map::{NONE, TexelMap};
use gf_core::skin_regions::Regions;
use gf_core::skin_texture::{self as st, SkinLook, SkinTextures, to_linear, to_srgb};
use gf_core::{game_mesh, skin_map, skin_regions};
use gf_geom::Uv;
use gf_geom::decimate::Decimated;
use gf_io::store::Store;

use crate::skin::{GRID_MM, parse_landmarks, stamp_images};

const PAD: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dirty {
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Dirty {
    pub fn full(size: usize) -> Dirty {
        Dirty { x0: 0, y0: 0, x1: size, y1: size }
    }

    pub fn union(self, o: Dirty) -> Dirty {
        Dirty { x0: self.x0.min(o.x0), y0: self.y0.min(o.y0), x1: self.x1.max(o.x1), y1: self.y1.max(o.y1) }
    }
}

struct LayerCache {
    key: String,
    authored: String,
    procedural: Vec<f32>,
    values: Vec<f32>,
    bytes: Vec<u8>,
}

pub struct SkinWork {
    pub size: usize,
    pub map: TexelMap,
    pub positions: Vec<[f64; 3]>,
    pub normals: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
    uvs: Vec<[Uv; 3]>,
    regions: Regions,
    landmarks: BTreeMap<usize, [f64; 3]>,
    texels: Vec<Option<Texel>>,
    grid: Grid,
    look: Option<SkinLook>,
    base: SkinTextures,
    base_linear: Vec<[f32; 3]>,
    layers: Vec<LayerCache>,
    images: Images,
    pads: Vec<(u32, u32)>,
    pub albedo: Vec<u8>,
    pub roughness: Vec<u8>,
}

fn layer_key(l: &SkinLayer) -> String {
    format!("{}/{}/{}/{}", l.kind, l.style, l.seed, l.is_procedural())
}

fn authored_key(l: &SkinLayer) -> String {
    let v = serde_json::json!({"stamps": l.stamps, "strokes": l.strokes});
    gf_core::hash::sha256_hex(&serde_json::to_vec(&v).unwrap_or_default())
}

impl SkinWork {
    pub fn new(store: &Store, skin: &SkinSpec, size: usize, dense: &[[f64; 3]], mesh: &Decimated) -> Result<SkinWork, String> {
        let raw = store.get_ref(&skin.landmarks).map_err(|e| format!("skin.landmarks: {e}"))?;
        let landmarks = parse_landmarks(&raw, dense).map_err(|e| format!("skin.landmarks: {e}"))?;
        let positions: Vec<[f64; 3]> = mesh.kept.iter().map(|&k| dense[k as usize]).collect();
        let normals = game_mesh::game_normals(&positions, &mesh.triangles);
        let regions = skin_regions::build(&positions, &normals, &mesh.triangles, &landmarks)?;
        let map = skin_map::build(&mesh.triangle_uvs, size, PAD);
        let texels = {
            let surface = sl::Surface { map: &map, positions: &positions, normals: &normals, triangles: &mesh.triangles, uvs: &mesh.triangle_uvs, regions: &regions, landmarks: &landmarks };
            sl::texels(&surface)
        };
        let grid = Grid::new(&texels, GRID_MM);
        let pads = (0..size * size).filter(|&k| map.triangle[k] == NONE && map.source[k] != NONE).map(|k| (k as u32, map.source[k])).collect();
        let mut work = SkinWork {
            size,
            positions,
            normals,
            triangles: mesh.triangles.clone(),
            uvs: mesh.triangle_uvs.clone(),
            regions,
            landmarks,
            texels,
            grid,
            look: None,
            base: SkinTextures { size, albedo: Vec::new(), normal: Vec::new(), orm: Vec::new() },
            base_linear: Vec::new(),
            layers: Vec::new(),
            images: Images::new(),
            pads,
            albedo: vec![0; size * size * 4],
            roughness: vec![0; size * size],
            map,
        };
        work.sync(store, skin)?;
        Ok(work)
    }

    fn surface(&self) -> sl::Surface<'_> {
        sl::Surface { map: &self.map, positions: &self.positions, normals: &self.normals, triangles: &self.triangles, uvs: &self.uvs, regions: &self.regions, landmarks: &self.landmarks }
    }

    pub fn sync(&mut self, store: &Store, skin: &SkinSpec) -> Result<Dirty, String> {
        if self.look.as_ref() != Some(&skin.look) {
            let base = st::generate(&self.map, &self.positions, &self.normals, &self.triangles, &self.uvs, &self.regions, &skin.look);
            self.base_linear = base.albedo.chunks_exact(4).map(|p| [0, 1, 2].map(|j| to_linear(p[j] as f32 / 255.0))).collect();
            self.base = base;
            self.look = Some(skin.look.clone());
        }
        for (key, image) in stamp_images(store, skin)? {
            self.images.entry(key).or_insert(image);
        }
        let mut layers = Vec::with_capacity(skin.layers.len());
        let mut old: Vec<Option<LayerCache>> = std::mem::take(&mut self.layers).into_iter().map(Some).collect();
        for layer in &skin.layers {
            let key = layer_key(layer);
            let authored = authored_key(layer);
            if let Some(slot) = old.iter_mut().find(|c| c.as_ref().is_some_and(|c| c.key == key && c.authored == authored)) {
                layers.push(slot.take().expect("found above"));
                continue;
            }
            let procedural = match old.iter_mut().find(|c| c.as_ref().is_some_and(|c| c.key == key)) {
                Some(slot) => slot.take().map(|c| c.procedural).unwrap_or_default(),
                None => sl::procedural(&self.surface(), &self.texels, layer)?,
            };
            let mut values = procedural.clone();
            sa::apply(&mut values, &self.texels, &self.grid, layer, &self.images, None)?;
            let bytes = sl::bytes(&self.map, &values);
            layers.push(LayerCache { key, authored, procedural, values, bytes });
        }
        self.layers = layers;
        self.composite(skin, None);
        Ok(Dirty::full(self.size))
    }

    pub fn repaint(&mut self, skin: &SkinSpec, index: usize, region: ([f32; 3], [f32; 3])) -> Result<Option<Dirty>, String> {
        let layer = skin.layers.get(index).ok_or_else(|| format!("no layer {index}"))?;
        let cache = self.layers.get_mut(index).ok_or_else(|| format!("layer {index} is not synced"))?;
        if cache.key != layer_key(layer) {
            return Err(format!("layer {index} changed since the last sync"));
        }
        cache.authored = authored_key(layer);
        let touched = sa::region_texels(&self.grid, &self.texels, region);
        for &k in &touched {
            cache.values[k as usize] = cache.procedural[k as usize];
        }
        sa::apply(&mut cache.values, &self.texels, &self.grid, layer, &self.images, Some(region))?;
        if touched.is_empty() {
            return Ok(None);
        }
        let mut set = vec![false; self.size * self.size];
        for &k in &touched {
            set[k as usize] = true;
            cache.bytes[k as usize] = (cache.values[k as usize].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
        for &(pad, src) in &self.pads {
            if set[src as usize] {
                cache.bytes[pad as usize] = cache.bytes[src as usize];
                set[pad as usize] = true;
            }
        }
        let dirty = self.composite(skin, Some(&set));
        Ok(dirty)
    }

    pub fn composite(&mut self, skin: &SkinSpec, only: Option<&[bool]>) -> Option<Dirty> {
        let responses: Vec<Option<sl::Response>> = skin.layers.iter().map(|l| l.response()).collect();
        let n = self.size * self.size;
        let mut dirty: Option<Dirty> = None;
        for k in 0..n {
            if only.is_some_and(|s| !s[k]) {
                continue;
            }
            let src = self.map.source[k];
            if src == NONE {
                self.albedo[k * 4..k * 4 + 4].copy_from_slice(&self.base.albedo[k * 4..k * 4 + 4]);
                self.roughness[k] = self.base.orm[k * 3 + 1];
                continue;
            }
            let mut c = self.base_linear[k];
            let mut r = self.base.orm[k * 3 + 1] as f32 / 255.0;
            for ((layer, response), cache) in skin.layers.iter().zip(&responses).zip(&self.layers) {
                let Some(resp) = response else { continue };
                let a = sl::coverage(cache.bytes[k], layer.amount as f32, resp.spread) * resp.opacity;
                if a <= 0.0 {
                    continue;
                }
                let v = layer.variant as f32;
                let (colour, rough) = match resp.variant {
                    Some((_, vc, vr)) => ([0, 1, 2].map(|j| resp.colour[j] + (vc[j] - resp.colour[j]) * v), resp.roughness + (vr - resp.roughness) * v),
                    None => (resp.colour, resp.roughness),
                };
                c = [0, 1, 2].map(|j| c[j] + (colour[j] - c[j]) * a);
                r += (rough - r) * a;
            }
            for (j, v) in c.iter().enumerate() {
                self.albedo[k * 4 + j] = (to_srgb(*v) * 255.0 + 0.5) as u8;
            }
            self.albedo[k * 4 + 3] = 255;
            self.roughness[k] = (r.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            if only.is_some() {
                let (x, y) = (k % self.size, k / self.size);
                let d = Dirty { x0: x, y0: y, x1: x + 1, y1: y + 1 };
                dirty = Some(dirty.map_or(d, |o| o.union(d)));
            }
        }
        if only.is_none() { Some(Dirty::full(self.size)) } else { dirty }
    }

    pub fn mask(&self, index: usize) -> Option<&[u8]> {
        self.layers.get(index).map(|c| c.bytes.as_slice())
    }

    pub fn neutral_point(&self, tri: [u32; 3], bary: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
        let p = tri.map(|i| self.positions.get(i as usize).copied());
        let n = tri.map(|i| self.normals.get(i as usize).copied());
        let (Some(a), Some(b), Some(c)) = (p[0], p[1], p[2]) else { return None };
        let (Some(na), Some(nb), Some(nc)) = (n[0], n[1], n[2]) else { return None };
        let mm = [0, 1, 2].map(|k| (bary[0] * a[k] + bary[1] * b[k] + bary[2] * c[k]) * 1000.0);
        let nn = [0, 1, 2].map(|k| bary[0] * na[k] + bary[1] * nb[k] + bary[2] * nc[k]);
        let l = (nn[0] * nn[0] + nn[1] * nn[1] + nn[2] * nn[2]).sqrt().max(1e-12);
        Some((mm, nn.map(|x| x / l)))
    }

    pub fn load_stamp(&mut self, store: &Store, image: &str, channel: &str) -> Result<(), String> {
        let key = (image.to_string(), channel.to_string());
        if self.images.contains_key(&key) {
            return Ok(());
        }
        let bytes = store.get_ref(image)?;
        let (w, h, rgba) = gf_io::png::decode_rgba(&bytes)?;
        self.images.insert(key, sa::Grey::from_rgba(w, h, &rgba, channel));
        Ok(())
    }
}
