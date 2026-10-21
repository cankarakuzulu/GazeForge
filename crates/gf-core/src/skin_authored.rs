use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::hash;
use crate::skin_layers::{SkinLayer, Texel};
use crate::skin_texture::smoothstep;

pub const MODES: [&str; 2] = ["add", "erase"];
pub const CHANNELS: [&str; 2] = ["alpha", "luminance"];
pub const RADIUS_MM: [f64; 2] = [0.2, 120.0];
pub const STAMP_MM: [f64; 2] = [1.0, 400.0];
pub const MAX_POINTS: usize = 200_000;

fn add_mode() -> String {
    "add".into()
}

fn one() -> f64 {
    1.0
}

fn half() -> f64 {
    0.5
}

fn alpha() -> String {
    "alpha".into()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stroke {
    #[serde(default = "add_mode")]
    pub mode: String,
    pub radius_mm: f64,
    #[serde(default = "one")]
    pub strength: f64,
    #[serde(default = "half")]
    pub hardness: f64,
    pub points: Vec<[f64; 3]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub image: String,
    #[serde(default = "alpha")]
    pub channel: String,
    pub at: [f64; 3],
    pub normal: [f64; 3],
    #[serde(default)]
    pub angle_deg: f64,
    pub size_mm: f64,
    #[serde(default = "one")]
    pub strength: f64,
    #[serde(default = "add_mode")]
    pub mode: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Mark {
    Stroke(Stroke),
    Stamp(Stamp),
}

pub struct Grey {
    pub width: usize,
    pub height: usize,
    pub values: Vec<f32>,
}

impl Grey {
    pub fn from_rgba(width: usize, height: usize, rgba: &[u8], channel: &str) -> Grey {
        let values = rgba
            .chunks_exact(4)
            .map(|p| match channel {
                "luminance" => (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0,
                _ => p[3] as f32 / 255.0,
            })
            .collect();
        Grey { width, height, values }
    }

    fn sample(&self, u: f32, v: f32) -> f32 {
        let x = (u * self.width as f32 - 0.5).clamp(0.0, (self.width - 1) as f32);
        let y = (v * self.height as f32 - 0.5).clamp(0.0, (self.height - 1) as f32);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let at = |x: usize, y: usize| self.values[y * self.width + x];
        let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * fx;
        let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * fx;
        top + (bottom - top) * fy
    }
}

pub type Images = BTreeMap<(String, String), Grey>;

fn finite3(p: &[f64; 3]) -> bool {
    p.iter().all(|x| x.is_finite())
}

pub fn problems(layer: &SkinLayer, at: &str) -> Vec<String> {
    let mut out = Vec::new();
    let unit = |x: f64| x.is_finite() && (0.0..=1.0).contains(&x);
    for (i, m) in layer.marks.iter().enumerate() {
        let at = format!("{at}.marks[{i}]");
        match m {
            Mark::Stroke(s) => {
                if !MODES.contains(&s.mode.as_str()) {
                    out.push(format!("{at}: mode {:?} is not add or erase", s.mode));
                }
                if !(s.radius_mm.is_finite() && (RADIUS_MM[0]..=RADIUS_MM[1]).contains(&s.radius_mm)) {
                    out.push(format!("{at}: radius_mm {} is outside [{}, {}]", s.radius_mm, RADIUS_MM[0], RADIUS_MM[1]));
                }
                if !unit(s.strength) || !unit(s.hardness) {
                    out.push(format!("{at}: strength and hardness must be in [0, 1]"));
                }
                if s.points.is_empty() || s.points.len() > MAX_POINTS || !s.points.iter().all(finite3) {
                    out.push(format!("{at}: points must be 1 to {MAX_POINTS} finite [x, y, z] in millimetres"));
                }
            }
            Mark::Stamp(s) => {
                if let Err(e) = hash::parse_reference(&s.image) {
                    out.push(format!("{at}.image: {e}"));
                }
                if !CHANNELS.contains(&s.channel.as_str()) || !MODES.contains(&s.mode.as_str()) {
                    out.push(format!("{at}: channel must be alpha or luminance and mode add or erase"));
                }
                if !(s.size_mm.is_finite() && (STAMP_MM[0]..=STAMP_MM[1]).contains(&s.size_mm)) {
                    out.push(format!("{at}: size_mm {} is outside [{}, {}]", s.size_mm, STAMP_MM[0], STAMP_MM[1]));
                }
                let n2: f64 = s.normal.iter().map(|x| x * x).sum();
                if !finite3(&s.at) || !finite3(&s.normal) || n2 < 1e-12 || !s.angle_deg.is_finite() || !unit(s.strength) {
                    out.push(format!("{at}: at, normal (not zero), angle_deg and strength (in [0, 1]) must be finite"));
                }
            }
        }
    }
    out
}

pub fn canonical(layer: &mut SkinLayer) {
    let r = |x: f64, q: f64| {
        let v = (x / q).round() * q;
        if v == 0.0 { 0.0 } else { v }
    };
    for m in &mut layer.marks {
        match m {
            Mark::Stroke(s) => {
                s.radius_mm = r(s.radius_mm, 1e-3);
                s.strength = r(s.strength, 1e-4);
                s.hardness = r(s.hardness, 1e-4);
                s.points.iter_mut().for_each(|p| *p = p.map(|x| r(x, 1e-3)));
            }
            Mark::Stamp(s) => {
                s.at = s.at.map(|x| r(x, 1e-3));
                s.normal = s.normal.map(|x| r(x, 1e-6));
                s.angle_deg = r(s.angle_deg, 1e-3);
                s.size_mm = r(s.size_mm, 1e-3);
                s.strength = r(s.strength, 1e-4);
            }
        }
    }
}

pub fn strokes(layer: &SkinLayer) -> usize {
    layer.marks.iter().filter(|m| matches!(m, Mark::Stroke(_))).count()
}

pub fn stamps(layer: &SkinLayer) -> usize {
    layer.marks.iter().filter(|m| matches!(m, Mark::Stamp(_))).count()
}

pub struct Grid {
    cell: f32,
    cells: HashMap<(i32, i32, i32), Vec<u32>>,
}

impl Grid {
    pub fn new(texels: &[Option<Texel>], cell: f32) -> Grid {
        let mut cells: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
        for (k, t) in texels.iter().enumerate() {
            if let Some(t) = t {
                cells.entry(key(t.mm, cell)).or_default().push(k as u32);
            }
        }
        Grid { cell, cells }
    }

    pub fn within(&self, lo: [f32; 3], hi: [f32; 3]) -> Vec<u32> {
        let (a, b) = (key(lo, self.cell), key(hi, self.cell));
        let mut out = Vec::new();
        for x in a.0..=b.0 {
            for y in a.1..=b.1 {
                for z in a.2..=b.2 {
                    if let Some(v) = self.cells.get(&(x, y, z)) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        out
    }
}

pub fn region_texels(grid: &Grid, texels: &[Option<Texel>], region: ([f32; 3], [f32; 3])) -> Vec<u32> {
    grid.within(region.0, region.1)
        .into_iter()
        .filter(|&k| texels[k as usize].as_ref().is_some_and(|t| (0..3).all(|j| t.mm[j] >= region.0[j] && t.mm[j] <= region.1[j])))
        .collect()
}

fn key(p: [f32; 3], cell: f32) -> (i32, i32, i32) {
    ((p[0] / cell).floor() as i32, (p[1] / cell).floor() as i32, (p[2] / cell).floor() as i32)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-12);
    a.map(|x| x / l)
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn f32x3(p: [f64; 3]) -> [f32; 3] {
    p.map(|x| x as f32)
}

pub fn dabs(stroke: &Stroke) -> Vec<[f32; 3]> {
    let spacing = (stroke.radius_mm as f32 * 0.25).max(0.05);
    let pts: Vec<[f32; 3]> = stroke.points.iter().map(|p| f32x3(*p)).collect();
    let mut out = vec![pts[0]];
    for w in pts.windows(2) {
        let d = sub(w[1], w[0]);
        let len = dot(d, d).sqrt();
        let steps = (len / spacing).ceil().max(1.0) as usize;
        for k in 1..=steps {
            let t = k as f32 / steps as f32;
            out.push([w[0][0] + d[0] * t, w[0][1] + d[1] * t, w[0][2] + d[2] * t]);
        }
    }
    out
}

fn nearest_normal(grid: &Grid, texels: &[Option<Texel>], p: [f32; 3], r: f32) -> Option<[f32; 3]> {
    let reach = r.max(grid.cell);
    grid.within(p.map(|x| x - reach), p.map(|x| x + reach))
        .into_iter()
        .filter_map(|k| texels[k as usize].as_ref())
        .min_by(|a, b| dot(sub(a.mm, p), sub(a.mm, p)).total_cmp(&dot(sub(b.mm, p), sub(b.mm, p))))
        .map(|t| t.n)
}

struct StampFrame {
    c: [f32; 3],
    n: [f32; 3],
    side: [f32; 3],
    up: [f32; 3],
    half: f32,
    aspect: f32,
}

fn stamp_frame(s: &Stamp, img: &Grey) -> StampFrame {
    let half = s.size_mm as f32 * 0.5;
    let aspect = img.height as f32 / img.width.max(1) as f32;
    let n = norm(f32x3(s.normal));
    let up = if n[2].abs() > 0.95 { [0.0, 1.0, 0.0] } else { [0.0, 0.0, 1.0] };
    let side0 = norm(cross(up, n));
    let up0 = cross(n, side0);
    let (sa, ca) = (s.angle_deg as f32).to_radians().sin_cos();
    StampFrame {
        c: f32x3(s.at),
        n,
        side: [0, 1, 2].map(|k| ca * side0[k] + sa * up0[k]),
        up: [0, 1, 2].map(|k| -sa * side0[k] + ca * up0[k]),
        half,
        aspect,
    }
}

pub fn stamp_box(s: &Stamp, img: &Grey) -> ([f32; 3], [f32; 3]) {
    let f = stamp_frame(s, img);
    let reach = (f.half * f.half * (2.0 + f.aspect * f.aspect)).sqrt() + 0.01;
    (f.c.map(|x| x - reach), f.c.map(|x| x + reach))
}

pub fn mark_box(m: &Mark, images: &Images) -> Result<([f32; 3], [f32; 3]), String> {
    match m {
        Mark::Stroke(s) => {
            let r = s.radius_mm as f32 + 0.01;
            let d = dabs(s);
            let lo = [0, 1, 2].map(|k| d.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min) - r);
            let hi = [0, 1, 2].map(|k| d.iter().map(|p| p[k]).fold(f32::NEG_INFINITY, f32::max) + r);
            Ok((lo, hi))
        }
        Mark::Stamp(s) => images.get(&(s.image.clone(), s.channel.clone())).map(|img| stamp_box(s, img)).ok_or_else(|| format!("image {} ({}) was not loaded", s.image, s.channel)),
    }
}

fn overlaps(a: &([f32; 3], [f32; 3]), b: &([f32; 3], [f32; 3])) -> bool {
    (0..3).all(|k| a.0[k] <= b.1[k] && b.0[k] <= a.1[k])
}

fn combine(values: &mut [f32], k: usize, v: f32, mode: &str) {
    if mode == "erase" {
        values[k] *= 1.0 - v;
    } else {
        values[k] = values[k].max(v);
    }
}

pub fn apply(values: &mut [f32], texels: &[Option<Texel>], grid: &Grid, layer: &SkinLayer, images: &Images, region: Option<([f32; 3], [f32; 3])>) -> Result<(), String> {
    let inside = |p: [f32; 3]| region.is_none_or(|(lo, hi)| (0..3).all(|k| p[k] >= lo[k] && p[k] <= hi[k]));
    for (i, m) in layer.marks.iter().enumerate() {
        let mut touched: HashMap<usize, f32> = HashMap::new();
        let mode = match m {
            Mark::Stamp(s) => {
                let img = images.get(&(s.image.clone(), s.channel.clone())).ok_or_else(|| format!("mark {i}: image {} ({}) was not loaded", s.image, s.channel))?;
                let b = stamp_box(s, img);
                if region.is_some_and(|r| !overlaps(&b, &r)) {
                    continue;
                }
                let f = stamp_frame(s, img);
                for k in grid.within(b.0, b.1) {
                    let k = k as usize;
                    let Some(t) = texels[k].as_ref() else { continue };
                    if !inside(t.mm) || dot(t.n, f.n) < 0.2 {
                        continue;
                    }
                    let d = sub(t.mm, f.c);
                    if dot(d, f.n).abs() > f.half {
                        continue;
                    }
                    let u = dot(d, f.side) / (2.0 * f.half) + 0.5;
                    let v = 0.5 - dot(d, f.up) / (2.0 * f.half * f.aspect);
                    if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                        continue;
                    }
                    touched.insert(k, img.sample(u, v) * s.strength as f32);
                }
                &s.mode
            }
            Mark::Stroke(s) => {
                let r = s.radius_mm as f32;
                let inner = r * s.hardness as f32 * 0.999;
                for p in dabs(s) {
                    let b = (p.map(|x| x - r), p.map(|x| x + r));
                    if region.is_some_and(|reg| !overlaps(&b, &reg)) {
                        continue;
                    }
                    let Some(dn) = nearest_normal(grid, texels, p, r * 0.25) else { continue };
                    for k in grid.within(b.0, b.1) {
                        let k = k as usize;
                        let Some(t) = texels[k].as_ref() else { continue };
                        if !inside(t.mm) || dot(t.n, dn) < 0.3 {
                            continue;
                        }
                        let d = dot(sub(t.mm, p), sub(t.mm, p)).sqrt();
                        if d > r {
                            continue;
                        }
                        let v = (1.0 - smoothstep(inner, r, d)) * s.strength as f32;
                        let e = touched.entry(k).or_insert(0.0);
                        *e = e.max(v);
                    }
                }
                &s.mode
            }
        };
        let mut keys: Vec<usize> = touched.keys().copied().collect();
        keys.sort_unstable();
        for k in keys {
            combine(values, k, touched[&k], mode);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn texel(mm: [f32; 3], n: [f32; 3]) -> Texel {
    Texel::probe(mm, n)
}

pub fn presets() -> Vec<(&'static str, Vec<SkinLayer>)> {
    let layer = |name: &str, kind: &str, style: &str, amount: f64, seed: u32, colour: Option<[f64; 3]>| SkinLayer {
        name: name.into(),
        kind: kind.into(),
        style: style.into(),
        live: None,
        amount,
        variant: 0.0,
        seed,
        colour,
        procedural: None,
        marks: Vec::new(),
    };
    vec![
        ("field-grime", vec![layer("dust", "grime", "dust", 0.3, 11, None), layer("dirt", "grime", "dirt", 0.0, 12, None)]),
        ("battle-worn", vec![layer("dirt", "grime", "dirt", 0.35, 21, None), layer("blood", "blood", "drips", 0.0, 22, None), layer("splatter", "blood", "splatter", 0.0, 23, None)]),
        ("mud-run", vec![layer("mud", "grime", "mud", 0.0, 31, None), layer("sand", "grime", "sand", 0.0, 32, None)]),
        ("war-paint", vec![
            layer("band", "paint", "eye-band", 1.0, 41, None),
            layer("stripes", "paint", "cheek-stripes", 1.0, 42, Some([0.6, 0.55, 0.45])),
            layer("chin_paint", "paint", "chin", 0.0, 43, Some([0.35, 0.05, 0.03])),
        ]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane() -> Vec<Option<Texel>> {
        let mut out = Vec::new();
        for y in 0..60 {
            for x in 0..60 {
                out.push(Some(texel([x as f32 * 0.5, 0.0, y as f32 * 0.5], [0.0, -1.0, 0.0])));
            }
        }
        out.push(None);
        out.push(Some(texel([10.0, 0.4, 10.0], [0.0, 1.0, 0.0])));
        out
    }

    fn layer(strokes: Vec<Stroke>, stamps: Vec<Stamp>) -> SkinLayer {
        let mut l = presets()[0].1[0].clone();
        l.marks = strokes.into_iter().map(Mark::Stroke).chain(stamps.into_iter().map(Mark::Stamp)).collect();
        l
    }

    fn stroke(mode: &str, radius: f64, points: Vec<[f64; 3]>) -> Stroke {
        Stroke { mode: mode.into(), radius_mm: radius, strength: 1.0, hardness: 1.0, points }
    }

    #[test]
    fn a_stroke_paints_inside_its_radius_on_the_facing_side_only_and_erase_takes_it_back() {
        let t = plane();
        let grid = Grid::new(&t, 4.0);
        let mut v = vec![0.0; t.len()];
        apply(&mut v, &t, &grid, &layer(vec![stroke("add", 2.0, vec![[10.0, 0.0, 10.0], [20.0, 0.0, 10.0]])], vec![]), &Images::new(), None).unwrap();
        let at = |x: usize, y: usize| v[y * 60 + x];
        assert_eq!(at(20, 20), 1.0);
        assert_eq!(at(30, 20), 1.0);
        assert_eq!(at(20, 25), 0.0);
        assert_eq!(v[t.len() - 1], 0.0, "the back-facing texel 0.4 mm away must not be painted");
        let mut e = v.clone();
        apply(&mut e, &t, &grid, &layer(vec![stroke("add", 2.0, vec![[10.0, 0.0, 10.0], [20.0, 0.0, 10.0]]), stroke("erase", 2.0, vec![[10.0, 0.0, 10.0], [20.0, 0.0, 10.0]])], vec![]), &Images::new(), None).unwrap();
        assert!(e.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn applying_a_region_gives_exactly_what_a_full_apply_gives_inside_it() {
        let t = plane();
        let grid = Grid::new(&t, 4.0);
        let l = layer(
            vec![
                Stroke { mode: "add".into(), radius_mm: 3.0, strength: 0.8, hardness: 0.3, points: vec![[2.0, 0.0, 2.0], [25.0, 0.0, 20.0]] },
                Stroke { mode: "erase".into(), radius_mm: 1.5, strength: 0.6, hardness: 0.0, points: vec![[12.0, 0.0, 4.0], [14.0, 0.0, 20.0]] },
            ],
            vec![],
        );
        let base: Vec<f32> = (0..t.len()).map(|k| (k % 7) as f32 / 10.0).collect();
        let mut full = base.clone();
        apply(&mut full, &t, &grid, &l, &Images::new(), None).unwrap();
        let region = ([9.0, -1.0, 6.0], [16.0, 1.0, 14.0]);
        let mut part = base.clone();
        apply(&mut part, &t, &grid, &l, &Images::new(), Some(region)).unwrap();
        let mut inside = 0;
        for (k, tx) in t.iter().enumerate() {
            let Some(tx) = tx else { continue };
            if (0..3).all(|j| tx.mm[j] >= region.0[j] && tx.mm[j] <= region.1[j]) {
                assert_eq!(part[k], full[k], "texel {k}");
                inside += 1;
            } else {
                assert_eq!(part[k], base[k], "texel {k} outside the region changed");
            }
        }
        assert!(inside > 100);
    }

    #[test]
    fn region_texels_are_exactly_the_texels_inside_the_box_not_the_whole_grid_cells() {
        let t = plane();
        let grid = Grid::new(&t, 4.0);
        let region = ([9.2, -1.0, 6.1], [10.9, 1.0, 7.9]);
        let got: std::collections::BTreeSet<u32> = region_texels(&grid, &t, region).into_iter().collect();
        let want: std::collections::BTreeSet<u32> = t
            .iter()
            .enumerate()
            .filter(|(_, x)| x.as_ref().is_some_and(|x| (0..3).all(|j| x.mm[j] >= region.0[j] && x.mm[j] <= region.1[j])))
            .map(|(k, _)| k as u32)
            .collect();
        assert_eq!(got, want);
        assert!(grid.within(region.0, region.1).len() > want.len(), "the cells hold more than the box, which is what made the brush paint bars");
    }

    #[test]
    fn a_stamp_lands_upright_on_its_surface() {
        let t = plane();
        let grid = Grid::new(&t, 4.0);
        let mut img = Images::new();
        let (w, h) = (4usize, 2usize);
        let mut rgba = vec![0u8; w * h * 4];
        for x in 0..w {
            rgba[x * 4 + 3] = 255;
        }
        img.insert(("sha256:x".into(), "alpha".into()), Grey::from_rgba(w, h, &rgba, "alpha"));
        let s = Stamp { image: "sha256:x".into(), channel: "alpha".into(), at: [15.0, 0.0, 15.0], normal: [0.0, -1.0, 0.0], angle_deg: 0.0, size_mm: 8.0, strength: 1.0, mode: "add".into() };
        let mut v = vec![0.0; t.len()];
        apply(&mut v, &t, &grid, &layer(vec![], vec![s]), &img, None).unwrap();
        let at = |x: usize, y: usize| v[y * 60 + x];
        assert!(at(30, 33) > 0.9, "the image top row lands above the centre (z up): {}", at(30, 33));
        assert!(at(30, 27) < 0.1, "and its bottom row below: {}", at(30, 27));
        assert_eq!(at(30, 36), 0.0, "4 mm tall: nothing 3 mm above the centre");
        assert_eq!(at(30, 45), 0.0);
        assert_eq!(at(45, 30), 0.0);
    }

    #[test]
    fn presets_are_valid_layers_with_unique_names() {
        for (name, layers) in presets() {
            let mut seen = std::collections::BTreeSet::new();
            for l in &layers {
                assert!(l.response().is_some(), "{name}/{}", l.name);
                assert!(seen.insert(l.name.clone()), "{name}: {} twice", l.name);
                assert!(problems(l, name).is_empty());
            }
        }
    }

    fn star(img: &mut Images, w: usize, h: usize) {
        img.insert(("sha256:x".into(), "alpha".into()), Grey { width: w, height: h, values: vec![1.0; w * h] });
    }

    fn stamp(at: [f64; 3], size: f64, angle: f64, mode: &str) -> Stamp {
        Stamp { image: "sha256:x".into(), channel: "alpha".into(), at, normal: [0.0, -1.0, 0.0], angle_deg: angle, size_mm: size, strength: 1.0, mode: mode.into() }
    }

    #[test]
    fn marks_apply_in_authoring_order() {
        let t = plane();
        let grid = Grid::new(&t, 4.0);
        let mut img = Images::new();
        star(&mut img, 4, 4);
        let erase = Stroke { mode: "erase".into(), radius_mm: 6.0, strength: 1.0, hardness: 1.0, points: vec![[15.0, 0.0, 15.0]] };
        let run = |marks: Vec<Mark>| {
            let mut l = presets()[0].1[0].clone();
            l.marks = marks;
            let mut v = vec![0.0; t.len()];
            apply(&mut v, &t, &grid, &l, &img, None).unwrap();
            v[30 * 60 + 30]
        };
        assert_eq!(run(vec![Mark::Stroke(erase.clone()), Mark::Stamp(stamp([15.0, 0.0, 15.0], 4.0, 0.0, "add"))]), 1.0, "a stamp placed after an erase shows");
        assert_eq!(run(vec![Mark::Stamp(stamp([15.0, 0.0, 15.0], 4.0, 0.0, "add")), Mark::Stroke(erase)]), 0.0, "an erase after a stamp removes it");
        let paint = Stroke { mode: "add".into(), radius_mm: 6.0, strength: 1.0, hardness: 1.0, points: vec![[15.0, 0.0, 15.0]] };
        assert_eq!(run(vec![Mark::Stroke(paint), Mark::Stamp(stamp([15.0, 0.0, 15.0], 4.0, 0.0, "erase"))]), 0.0, "an erase stamp removes brush paint");
    }

    #[test]
    fn a_tall_rotated_stamp_repainted_through_its_own_box_equals_a_full_apply() {
        let t = plane();
        let grid = Grid::new(&t, 4.0);
        let mut img = Images::new();
        star(&mut img, 2, 16);
        let s = stamp([15.0, 0.0, 15.0], 3.0, 37.0, "add");
        let l = layer(vec![], vec![s.clone()]);
        let mut full = vec![0.0; t.len()];
        apply(&mut full, &t, &grid, &l, &img, None).unwrap();
        let b = mark_box(&Mark::Stamp(s), &img).unwrap();
        let mut part = vec![0.0; t.len()];
        apply(&mut part, &t, &grid, &l, &img, Some(b)).unwrap();
        assert_eq!(part, full);
        let covered = full.iter().filter(|&&v| v > 0.0).count();
        let reach_axis = |k: usize| t.iter().zip(&full).filter(|(_, v)| **v > 0.0).map(|(x, _)| (x.as_ref().unwrap().mm[k] - 15.0).abs()).fold(0.0f32, f32::max);
        assert!(covered > 30 && reach_axis(2) > 1.5 * 3.0 * 0.75, "the stamp must reach past the old 0.75 x size box: {covered} texels, z reach {}", reach_axis(2));
    }
}
