use std::path::Path;
use std::sync::mpsc::Receiver;

use gf_api::skin_work::{Dirty, SkinWork};
use gf_core::skin_authored::{Mark, Stamp, Stroke};
use gf_core::skin_layers::{SkinLayer, SkinSpec};
use gf_io::store::Store;
use serde_json::{Value, json};

use crate::i18n::{tr, trf};
use crate::render;
use crate::viewer::Rect;
use crate::world::World;

pub const PREVIEW_SIZE: usize = 1024;
pub const HISTORY_LIMIT: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    None,
    Brush,
    Stamp,
}

#[derive(Clone, Debug)]
pub struct Brush {
    pub radius_mm: f64,
    pub strength: f64,
    pub hardness: f64,
    pub erase: bool,
}

pub struct SkinUi {
    pub spec: Option<SkinSpec>,
    pub work: Option<SkinWork>,
    job: Option<Receiver<Result<SkinWork, String>>>,
    pending: bool,
    pub status: String,
    pub selected: usize,
    pub tool: Tool,
    pub brush: Brush,
    painting: Option<usize>,
    last_screen: Option<(f64, f64)>,
    pub stamp_path: String,
    pub stamp_ref: Option<String>,
    pub stamp_size_mm: f64,
    pub stamp_angle_deg: f64,
    pub stamp_strength: f64,
    pub landmarks_path: String,
    pub look_id: String,
    pub done: Vec<Option<SkinSpec>>,
    pub undone: Vec<Option<SkinSpec>>,
    pub revision: u64,
    pub dirty: Option<Dirty>,
    pub hover: Option<[f64; 3]>,
    pub last_error: Option<String>,
}

impl Default for SkinUi {
    fn default() -> SkinUi {
        SkinUi {
            spec: None,
            work: None,
            job: None,
            pending: false,
            status: tr("skin.none").to_string(),
            selected: 0,
            tool: Tool::None,
            brush: Brush { radius_mm: 4.0, strength: 1.0, hardness: 0.5, erase: false },
            painting: None,
            last_screen: None,
            stamp_path: String::new(),
            stamp_ref: None,
            stamp_size_mm: 30.0,
            stamp_angle_deg: 0.0,
            stamp_strength: 1.0,
            landmarks_path: String::new(),
            look_id: gf_core::skin_texture::DEFAULT_LOOK.to_string(),
            done: Vec::new(),
            undone: Vec::new(),
            revision: 0,
            dirty: None,
            hover: None,
            last_error: None,
        }
    }
}

impl SkinUi {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn ready(&self) -> bool {
        self.work.is_some() && self.job.is_none() && self.spec.is_some()
    }

    fn mark(&mut self, d: Option<Dirty>) {
        if let Some(d) = d {
            self.dirty = Some(self.dirty.map_or(d, |o| o.union(d)));
            self.revision += 1;
        }
    }
}

fn unique_name(spec: &SkinSpec, want: &str) -> String {
    if !spec.layers.iter().any(|l| l.name == want) {
        return want.to_string();
    }
    (2..).map(|k| format!("{want}_{k}")).find(|n| !spec.layers.iter().any(|l| &l.name == n)).unwrap_or_else(|| want.to_string())
}

pub fn new_layer(spec: &SkinSpec, kind: &str, style: &str) -> SkinLayer {
    let base = if kind == "paint" { "paint".to_string() } else { style.replace('-', "_") };
    SkinLayer {
        name: unique_name(spec, &base),
        kind: kind.into(),
        style: style.into(),
        live: None,
        amount: if kind == "grime" { 0.5 } else { 1.0 },
        variant: 0.0,
        seed: 1 + spec.layers.len() as u32,
        colour: None,
        procedural: None,
        marks: Vec::new(),
    }
}

impl World {
    fn skin_store_input(&self) -> Option<(String, String)> {
        self.session.as_ref().map(|s| (s.store.clone(), s.game.clone()))
    }

    pub fn skin_from_session(&mut self) {
        self.skin = SkinUi::default();
        let Some(s) = &self.session else { return };
        if s.character.skin.is_null() {
            return;
        }
        match serde_json::from_value::<SkinSpec>(s.character.skin.clone()) {
            Ok(spec) => {
                self.skin.look_id = spec.look.id.clone();
                self.skin.spec = Some(spec);
                self.skin_rebuild();
            }
            Err(e) => self.skin.status = trf("skin.bad_spec", &[("error", &e.to_string())]),
        }
    }

    pub fn skin_create(&mut self) -> Result<(), String> {
        let (store, game) = self.skin_store_input().ok_or_else(|| tr("studio.load_first").to_string())?;
        let look = gf_core::skin_texture::look(&self.skin.look_id).ok_or_else(|| trf("skin.no_look", &[("look", &self.skin.look_id)]))?;
        let env = gf_api::call(&gf_api::Ctx::from_env(), "skin.landmarks", &json!({"store": store, "game": game, "landmarks": self.skin.landmarks_path}));
        if env["ok"] != true {
            return Err(env["error"]["message"].as_str().unwrap_or("skin.landmarks failed").to_string());
        }
        let landmarks = env["result"]["landmarks"].as_str().unwrap_or_default().to_string();
        let before = self.skin.spec.take();
        self.skin_push_history(before);
        self.skin.spec = Some(SkinSpec { look, landmarks, size: gf_core::skin_layers::DEFAULT_SIZE, layers: Vec::new() });
        self.skin.work = None;
        self.skin_write_character();
        self.skin_rebuild();
        Ok(())
    }

    pub fn skin_rebuild(&mut self) {
        let Some((store, game)) = self.skin_store_input() else { return };
        let Some(spec) = self.skin.spec.clone() else { return };
        if self.skin.job.is_some() {
            self.skin.pending = true;
            return;
        }
        let existing = self.skin.work.take();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let out = (|| -> Result<SkinWork, String> {
                let store = Store::open(Path::new(&store))?;
                match existing {
                    Some(mut work) => {
                        work.sync(&store, &spec)?;
                        Ok(work)
                    }
                    None => {
                        let (manifest, mesh) = store.load_game(&game)?;
                        let (_, dense) = store.load_pack(&manifest.pack)?;
                        SkinWork::new(&store, &spec, PREVIEW_SIZE, &dense.positions, &mesh)
                    }
                }
            })();
            let _ = tx.send(out);
        });
        self.skin.job = Some(rx);
        self.skin.pending = false;
        self.skin.status = tr("skin.updating").into();
    }

    pub fn skin_poll(&mut self) {
        let Some(rx) = &self.skin.job else { return };
        let Ok(out) = rx.try_recv() else { return };
        self.skin.job = None;
        match out {
            Ok(work) => {
                let head_n = self.session.as_ref().map_or(0, |s| s.head.n);
                if head_n != work.positions.len() {
                    self.skin.status = trf("skin.base_mismatch", &[("head", &head_n.to_string()), ("skin", &work.positions.len().to_string())]);
                    self.skin.work = None;
                    return;
                }
                let size = work.size;
                self.skin.work = Some(work);
                self.skin.mark(Some(Dirty::full(size)));
                self.skin.status = tr("skin.ready").into();
                self.skin.last_error = None;
            }
            Err(e) => {
                self.skin.status = trf("skin.failed", &[("error", &e)]);
                self.skin.last_error = Some(e);
            }
        }
        if self.skin.pending {
            self.skin_rebuild();
        }
    }

    pub fn skin_write_character(&mut self) {
        let value = self.skin.spec.as_ref().map_or(Value::Null, |s| serde_json::to_value(s.canonical()).unwrap_or(Value::Null));
        if let Some(s) = self.session.as_mut() {
            s.character.skin = value;
        }
    }

    pub fn skin_edit(&mut self, f: impl FnOnce(&mut SkinSpec)) {
        let Some(spec) = self.skin.spec.as_mut() else { return };
        let before = spec.clone();
        f(spec);
        if *spec == before {
            return;
        }
        let n = spec.layers.len();
        self.skin_push_history(Some(before));
        self.skin.selected = self.skin.selected.min(n.saturating_sub(1));
        self.skin_write_character();
        self.skin_rebuild();
    }

    fn skin_push_history(&mut self, spec: Option<SkinSpec>) {
        self.skin.done.push(spec);
        let over = self.skin.done.len().saturating_sub(HISTORY_LIMIT);
        self.skin.done.drain(..over);
        self.journal_push(crate::history::Domain::Skin);
    }

    pub fn skin_checkpoint(&mut self) {
        let Some(spec) = self.skin.spec.clone() else { return };
        self.skin_push_history(Some(spec));
    }

    pub fn skin_tweak(&mut self, f: impl FnOnce(&mut SkinSpec)) {
        let Some(spec) = self.skin.spec.as_mut() else { return };
        let before = spec.clone();
        f(spec);
        if *spec != before {
            self.skin_write_character();
            self.skin_rebuild();
        }
    }

    pub fn skin_step(&mut self, redo: bool) -> Option<String> {
        let current = self.skin.spec.clone();
        let (from, to) = if redo { (&mut self.skin.undone, &mut self.skin.done) } else { (&mut self.skin.done, &mut self.skin.undone) };
        let prev = from.pop()?;
        to.push(current);
        self.skin.painting = None;
        match prev {
            Some(spec) => {
                self.skin.selected = self.skin.selected.min(spec.layers.len().saturating_sub(1));
                self.skin.look_id = spec.look.id.clone();
                self.skin.spec = Some(spec);
                self.skin_write_character();
                self.skin_rebuild();
            }
            None => {
                self.skin.spec = None;
                self.skin.work = None;
                self.skin.dirty = None;
                self.skin.status = tr("skin.none").into();
                self.skin_write_character();
            }
        }
        Some(tr("history.skin").to_string())
    }

    pub fn skin_pick(&self, r: &Rect, px: f64, py: f64) -> Option<([f64; 3], [f64; 3], [f64; 3])> {
        let s = self.session.as_ref()?;
        let work = self.skin.work.as_ref()?;
        let hit = render::pick_triangle(&self.state, &s.head, r, px, py, Some(0))?;
        let (mm, n) = work.neutral_point(hit.triangle, hit.bary)?;
        Some((mm, n, hit.point))
    }

    pub fn skin_paint_begin(&mut self, r: &Rect, px: f64, py: f64) -> bool {
        if !self.skin.ready() || self.skin.tool != Tool::Brush {
            return false;
        }
        let index = self.skin.selected;
        if self.skin.spec.as_ref().is_none_or(|s| index >= s.layers.len()) {
            self.skin.status = tr("skin.pick_layer").into();
            return false;
        }
        let Some((mm, _, _)) = self.skin_pick(r, px, py) else { return false };
        let b = self.skin.brush.clone();
        let before = self.skin.spec.clone().expect("ready");
        let spec = self.skin.spec.as_mut().expect("ready");
        spec.layers[index].marks.push(Mark::Stroke(Stroke { mode: if b.erase { "erase" } else { "add" }.into(), radius_mm: b.radius_mm, strength: b.strength, hardness: b.hardness, points: vec![mm] }));
        gf_core::skin_authored::canonical(&mut spec.layers[index]);
        let Some(Mark::Stroke(s)) = spec.layers[index].marks.last() else { unreachable!("a stroke was pushed above") };
        let mm = s.points[0];
        if !self.skin_repaint_dab(index, mm, mm) {
            self.skin.spec = Some(before);
            return false;
        }
        self.skin_push_history(Some(before));
        self.skin.painting = Some(index);
        self.skin.last_screen = Some((px, py));
        true
    }

    pub fn skin_paint_to(&mut self, r: &Rect, px: f64, py: f64) {
        let Some((x0, y0)) = self.skin.last_screen else { return };
        let pixels = ((px - x0).powi(2) + (py - y0).powi(2)).sqrt();
        let steps = (pixels / 3.0).ceil().clamp(1.0, 256.0) as usize;
        for k in 1..=steps {
            let t = k as f64 / steps as f64;
            self.skin_paint_point(r, x0 + (px - x0) * t, y0 + (py - y0) * t);
        }
        self.skin.last_screen = Some((px, py));
    }

    fn skin_paint_point(&mut self, r: &Rect, px: f64, py: f64) {
        let Some(index) = self.skin.painting else { return };
        let Some((mm, _, _)) = self.skin_pick(r, px, py) else { return };
        let Some(layer) = self.skin.spec.as_mut().and_then(|s| s.layers.get_mut(index)) else { return };
        let Some(Mark::Stroke(stroke)) = layer.marks.last_mut() else { return };
        let last = *stroke.points.last().expect("a stroke has a point");
        let step = (stroke.radius_mm * 0.25).max(0.05);
        if (0..3).map(|k| (mm[k] - last[k]).powi(2)).sum::<f64>().sqrt() < step {
            return;
        }
        stroke.points.push(mm);
        gf_core::skin_authored::canonical(layer);
        let Some(Mark::Stroke(stroke)) = layer.marks.last() else { return };
        let mm = *stroke.points.last().expect("pushed above");
        if !self.skin_repaint_dab(index, last, mm)
            && let Some(Mark::Stroke(stroke)) = self.skin.spec.as_mut().and_then(|s| s.layers.get_mut(index)).and_then(|l| l.marks.last_mut())
        {
            stroke.points.pop();
        }
    }

    pub fn skin_paint_end(&mut self) {
        self.skin.last_screen = None;
        if self.skin.painting.take().is_some() {
            self.skin_write_character();
        }
    }

    pub fn skin_painting(&self) -> bool {
        self.skin.painting.is_some()
    }

    fn skin_repaint_dab(&mut self, index: usize, a: [f64; 3], b: [f64; 3]) -> bool {
        let r = self.skin.brush.radius_mm + 0.5;
        let lo = [0, 1, 2].map(|k| (a[k].min(b[k]) - r) as f32);
        let hi = [0, 1, 2].map(|k| (a[k].max(b[k]) + r) as f32);
        self.skin_repaint(index, (lo, hi))
    }

    fn skin_repaint(&mut self, index: usize, region: ([f32; 3], [f32; 3])) -> bool {
        let (Some(work), Some(spec)) = (self.skin.work.as_mut(), self.skin.spec.as_ref()) else { return false };
        match work.repaint(spec, index, region) {
            Ok(d) => {
                self.skin.mark(d);
                true
            }
            Err(e) => {
                self.skin.status = trf("skin.failed", &[("error", &e)]);
                false
            }
        }
    }

    pub fn skin_load_stamp(&mut self) -> Result<(), String> {
        let (store, _) = self.skin_store_input().ok_or_else(|| tr("studio.load_first").to_string())?;
        if !self.skin.ready() {
            return Err(tr("skin.updating").to_string());
        }
        let env = gf_api::call(&gf_api::Ctx::from_env(), "skin.stamp", &json!({"store": store, "image": self.skin.stamp_path}));
        if env["ok"] != true {
            return Err(env["error"]["message"].as_str().unwrap_or("skin.stamp failed").to_string());
        }
        let image = env["result"]["image"].as_str().unwrap_or_default().to_string();
        let opened = Store::open(Path::new(&store))?;
        if let Some(work) = self.skin.work.as_mut() {
            work.load_stamp(&opened, &image, "alpha")?;
        }
        self.skin.stamp_ref = Some(image);
        Ok(())
    }

    pub fn skin_place_stamp(&mut self, r: &Rect, px: f64, py: f64) -> bool {
        if !self.skin.ready() || self.skin.tool != Tool::Stamp {
            return false;
        }
        let Some(image) = self.skin.stamp_ref.clone() else {
            self.skin.status = tr("skin.stamp_first").into();
            return false;
        };
        let index = self.skin.selected;
        if self.skin.spec.as_ref().is_none_or(|s| index >= s.layers.len()) {
            self.skin.status = tr("skin.pick_layer").into();
            return false;
        }
        let Some((mm, n, _)) = self.skin_pick(r, px, py) else { return false };
        let Some((store, _)) = self.skin_store_input() else { return false };
        let loaded = Store::open(Path::new(&store)).and_then(|st| self.skin.work.as_mut().map_or(Err("no skin".into()), |w| w.load_stamp(&st, &image, "alpha")));
        if let Err(e) = loaded {
            self.skin.status = trf("skin.failed", &[("error", &e)]);
            return false;
        }
        let stamp = Stamp {
            image,
            channel: "alpha".into(),
            at: mm,
            normal: n,
            angle_deg: self.skin.stamp_angle_deg,
            size_mm: self.skin.stamp_size_mm,
            strength: self.skin.stamp_strength,
            mode: if self.skin.brush.erase { "erase" } else { "add" }.into(),
        };
        let before = self.skin.spec.clone().expect("ready");
        let spec = self.skin.spec.as_mut().expect("ready");
        spec.layers[index].marks.push(Mark::Stamp(stamp));
        gf_core::skin_authored::canonical(&mut spec.layers[index]);
        let Some(Mark::Stamp(placed)) = spec.layers[index].marks.last() else { unreachable!("a stamp was pushed above") };
        let Some(region) = self.skin.work.as_ref().and_then(|w| w.stamp_box(placed)) else {
            self.skin.spec = Some(before);
            return false;
        };
        if !self.skin_repaint(index, region) {
            self.skin.spec = Some(before);
            return false;
        }
        self.skin_push_history(Some(before));
        self.skin_write_character();
        true
    }

    pub fn skin_script(&mut self, step: &Value, width: u32, height: u32) -> Result<Value, String> {
        let text = |k: &str| step.get(k).and_then(Value::as_str).map(str::to_string);
        let num = |k: &str| step.get(k).and_then(Value::as_f64);
        let seconds = num("seconds").unwrap_or(600.0);
        let rect = crate::viewer::rects(self.state.layout, width, height).into_iter().find(|r| r.slot == self.state.active).ok_or("no viewport rect")?;
        let layer = |w: &World| step.get("layer").and_then(Value::as_u64).map(|x| x as usize).unwrap_or(w.skin.selected);
        match text("do").as_deref().unwrap_or("summary") {
            "create" => {
                self.skin.landmarks_path = text("landmarks").ok_or("skin create needs landmarks")?;
                if let Some(l) = text("look") {
                    self.skin.look_id = l;
                }
                self.skin_create()?;
            }
            "add" => {
                let (kind, style) = (text("kind").ok_or("skin add needs kind")?, text("style").ok_or("skin add needs style")?);
                self.skin_edit(|s| {
                    let l = new_layer(s, &kind, &style);
                    s.layers.push(l);
                });
                self.skin.selected = self.skin.spec.as_ref().map_or(0, |s| s.layers.len().saturating_sub(1));
            }
            "preset" => {
                let name = text("name").ok_or("skin preset needs name")?;
                let (_, layers) = gf_core::skin_authored::presets().into_iter().find(|(n, _)| *n == name).ok_or_else(|| format!("no skin preset {name}"))?;
                self.skin_edit(|s| s.layers.extend(layers));
            }
            "set" => {
                let i = layer(self);
                let fields = step.get("fields").and_then(Value::as_object).ok_or("skin set needs fields")?.clone();
                let current = self.skin.spec.as_ref().and_then(|s| s.layers.get(i)).ok_or_else(|| format!("no skin layer {i}"))?;
                let mut v = serde_json::to_value(current).map_err(|e| e.to_string())?;
                for (k, x) in fields {
                    v[k] = x;
                }
                let next: SkinLayer = serde_json::from_value(v).map_err(|e| format!("skin set: {e}"))?;
                self.skin_edit(|s| s.layers[i] = next);
            }
            "select" => self.skin.selected = layer(self),
            "tool" => {
                self.skin.tool = match text("tool").as_deref() {
                    Some("brush") => Tool::Brush,
                    Some("stamp") => Tool::Stamp,
                    _ => Tool::None,
                };
                let b = &mut self.skin.brush;
                b.radius_mm = num("radius_mm").unwrap_or(b.radius_mm);
                b.strength = num("strength").unwrap_or(b.strength);
                b.hardness = num("hardness").unwrap_or(b.hardness);
                b.erase = step.get("erase").and_then(Value::as_bool).unwrap_or(b.erase);
                self.skin.stamp_size_mm = num("size_mm").unwrap_or(self.skin.stamp_size_mm);
                self.skin.stamp_angle_deg = num("angle_deg").unwrap_or(self.skin.stamp_angle_deg);
            }
            "paint" => {
                self.wait(|w| !w.skin.busy(), seconds)?;
                self.skin.selected = layer(self);
                let pts: Vec<(f64, f64)> = step.get("points").and_then(Value::as_array).ok_or("skin paint needs points [[x, y], ...] in pixels")?.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect();
                let (first, rest) = pts.split_first().ok_or("skin paint needs at least one point")?;
                if !self.skin_paint_begin(&rect, first.0, first.1) {
                    return Err(format!("the first point {first:?} did not hit the head, or the brush or a layer is not ready: {}", self.skin.status));
                }
                for p in rest {
                    self.skin_paint_to(&rect, p.0, p.1);
                }
                self.skin_paint_end();
            }
            "stamp_image" => {
                self.wait(|w| !w.skin.busy(), seconds)?;
                self.skin.stamp_path = text("path").ok_or("skin stamp_image needs path")?;
                self.skin_load_stamp()?;
            }
            "stamp" => {
                self.wait(|w| !w.skin.busy(), seconds)?;
                self.skin.selected = layer(self);
                let (x, y) = (num("x").ok_or("skin stamp needs x")?, num("y").ok_or("skin stamp needs y")?);
                if !self.skin_place_stamp(&rect, x, y) {
                    return Err(format!("the stamp at ({x}, {y}) did not land: {}", self.skin.status));
                }
            }
            "undo" => {
                self.undo(false);
            }
            "redo" => {
                self.undo(true);
            }
            "dump_masks" => {
                let dir = std::path::PathBuf::from(text("dir").ok_or("skin dump_masks needs dir")?);
                std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
                let work = self.skin.work.as_ref().ok_or("no skin is loaded")?;
                let names: Vec<String> = self.skin.spec.as_ref().map(|s| s.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default();
                for (i, name) in names.iter().enumerate() {
                    let m = work.mask(i).ok_or_else(|| format!("no mask for layer {i}"))?;
                    let rgba: Vec<u8> = m.iter().flat_map(|&v| [v, v, v, 255]).collect();
                    let png = gf_io::png::rgba(work.size, work.size, &rgba)?;
                    std::fs::write(dir.join(format!("{name}.png")), png).map_err(|e| e.to_string())?;
                }
            }
            "summary" | "wait" => {}
            other => return Err(format!("skin do is create, add, preset, set, select, tool, paint, stamp_image, stamp, undo, redo, wait or summary, got {other}")),
        }
        self.wait(|w| !w.skin.busy(), seconds)?;
        self.tick();
        Ok(self.skin_summary())
    }

    pub fn skin_summary(&self) -> Value {
        let masks: Vec<Value> = self
            .skin
            .work
            .as_ref()
            .map(|w| {
                (0..self.skin.spec.as_ref().map_or(0, |s| s.layers.len()))
                    .filter_map(|i| w.mask(i).map(|m| json!({"layer": i, "sha256": gf_core::hash::sha256_hex(m), "texels": m.iter().filter(|&&v| v > 0).count()})))
                    .collect()
            })
            .unwrap_or_default();
        json!({"spec": self.skin.spec.as_ref().map(SkinSpec::canonical), "status": self.skin.status, "busy": self.skin.busy(), "size": self.skin.work.as_ref().map(|w| w.size),
               "masks": masks, "revision": self.skin.revision, "undo": self.skin.done.len(), "redo": self.skin.undone.len(), "error": self.skin.last_error})
    }
}
