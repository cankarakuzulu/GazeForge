use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;

use crate::godot;
use crate::input::{self, ShapeInput};
use crate::keymap::{self, Keymap};
use crate::render::{self, Renderer};
use crate::script::write_png;
use crate::viewer::{self, Layout, MODES};
use crate::world::World;

pub struct UiState {
    pub keymap: Keymap,
    pub input: Box<dyn ShapeInput>,
    pub save_path: String,
    pub store: String,
    pub spec: String,
    pub macro_file: String,
    pub eyes: String,
    pub status: String,
    texture: Option<egui::TextureId>,
    texture_generation: u64,
    drag: Option<(usize, bool, f32)>,
    pub render_ms: f64,
    pub frame_ms: f64,
    pub show_help: bool,
    pub godot_config: Option<PathBuf>,
}

pub struct UiArgs {
    pub keymap: Option<PathBuf>,
    pub input: Option<String>,
    pub store: Option<String>,
    pub spec: Option<String>,
    pub macro_file: Option<String>,
    pub eyes: Option<String>,
    pub godot_config: Option<PathBuf>,
}

impl UiState {
    pub fn new(a: UiArgs) -> Result<UiState, String> {
        let name = a.input.clone().unwrap_or_else(|| "sliders".into());
        let input = input::by_name(&name).ok_or_else(|| format!("unknown input {name}; inputs: {}", input::INPUTS.join(", ")))?;
        Ok(UiState {
            keymap: Keymap::load(a.keymap.as_deref()),
            input,
            save_path: String::new(),
            store: a.store.unwrap_or_default(),
            spec: a.spec.unwrap_or_default(),
            macro_file: a.macro_file.unwrap_or_default(),
            eyes: a.eyes.unwrap_or_default(),
            status: "open a store".into(),
            texture: None,
            texture_generation: 0,
            drag: None,
            render_ms: 0.0,
            frame_ms: 0.0,
            show_help: false,
            godot_config: a.godot_config,
        })
    }
}

pub(crate) fn system_font() -> Option<Vec<u8>> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(win) = std::env::var_os("WINDIR").or_else(|| std::env::var_os("SystemRoot")) {
        let fonts = PathBuf::from(win).join("Fonts");
        candidates.push(fonts.join("segoeui.ttf"));
        candidates.push(fonts.join("arial.ttf"));
    }
    for p in ["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc", "/Library/Fonts/Arial.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"] {
        candidates.push(PathBuf::from(p));
    }
    candidates.into_iter().find_map(|p| std::fs::read(p).ok())
}

pub fn setup_context(ctx: &egui::Context) -> Result<(), String> {
    let font = system_font().ok_or("no system font found for the interface")?;
    let mut fonts = egui::FontDefinitions::empty();
    fonts.font_data.insert("ui".into(), std::sync::Arc::new(egui::FontData::from_owned(font)));
    fonts.families.insert(egui::FontFamily::Proportional, vec!["ui".into()]);
    fonts.families.insert(egui::FontFamily::Monospace, vec!["ui".into()]);
    ctx.set_fonts(fonts);
    ctx.set_theme(egui::ThemePreference::Dark);
    Ok(())
}

pub struct Gfx<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub egui: &'a egui::mutex::RwLock<egui_wgpu::Renderer>,
}

pub struct Studio<'a> {
    pub s: &'a mut UiState,
    pub world: &'a mut World,
    pub renderer: &'a mut Renderer,
    pub godot: &'a mut Option<Arc<godot::Host>>,
    pub gfx: Gfx<'a>,
    pub spin: bool,
    pub keep_painting: bool,
}

impl Studio<'_> {
    pub fn load(&mut self) {
        let opt = |s: &String| if s.trim().is_empty() { None } else { Some(s.trim().to_string()) };
        let (spec, mac, eyes) = (opt(&self.s.spec), opt(&self.s.macro_file), opt(&self.s.eyes));
        self.world.request_load(self.s.store.trim(), spec.as_deref(), mac.as_deref(), eyes.as_deref());
        self.s.status = "loading".into();
    }

    fn save(&mut self) {
        let path = if self.s.save_path.trim().is_empty() {
            let name = self.world.session.as_ref().map_or("character".to_string(), |s| s.character.name.clone());
            format!("{name}.gfchar.json")
        } else {
            self.s.save_path.trim().to_string()
        };
        self.s.save_path = path.clone();
        self.s.status = match self.world.save(&path) {
            Ok(()) => format!("saving {path}"),
            Err(e) => e,
        };
    }

    pub fn act(&mut self, id: &str) {
        match id {
            "screenshot" | "screenshot_layout" => self.s.status = self.screenshot(id == "screenshot_layout"),
            "help" => self.s.show_help = !self.s.show_help,
            "save_spec" => self.save(),
            _ => {
                self.s.status = match self.world.action(id, false) {
                    Ok(m) => m,
                    Err(e) => e,
                }
            }
        }
    }

    fn screenshot(&mut self, layout: bool) -> String {
        let Some(c) = self.renderer.composite.as_ref() else { return "nothing rendered yet".into() };
        let (x, y, w, h) = if layout {
            (0, 0, c.w, c.h)
        } else {
            let a = self.world.state.active_rect(c.w, c.h);
            (a.x, a.y, a.w, a.h)
        };
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let path = PathBuf::from(format!("gazeforge-studio-{stamp}.png"));
        match self.renderer.read_rgba(self.gfx.device, self.gfx.queue, x, y, w, h).and_then(|px| write_png(&path, w, h, &px)) {
            Ok(()) => format!("saved {}", path.display()),
            Err(e) => format!("screenshot failed: {e}"),
        }
    }

    fn set_godot(&mut self, slot: usize, on: bool) {
        if on {
            let host = match &*self.godot {
                Some(h) if !matches!(h.status().0, godot::Status::Failed | godot::Status::Stopped) => h.clone(),
                _ => {
                    let mut cfg = godot::Config::load(self.s.godot_config.as_deref());
                    if cfg.log.is_none() {
                        cfg.log = Some(std::env::temp_dir().join("gazeforge-studio-godot.log"));
                    }
                    if cfg.background.is_none() {
                        cfg.background = Some(render::background(self.world.state.opts.dark));
                    }
                    let h = godot::Host::start(cfg);
                    *self.godot = Some(h.clone());
                    h
                }
            };
            self.renderer.sources[slot] = render::Source::External(Box::new(godot::GodotSlot { host, slot }));
        } else {
            self.renderer.sources[slot] = render::Source::Native;
            if (0..4).all(|s| self.renderer.sources[s].name() != "godot") {
                *self.godot = None;
            }
        }
    }

    pub fn frame(&mut self, ui: &mut egui::Ui, dt: f64) {
        let ctx = ui.ctx().clone();
        if !ctx.input(|i| i.pointer.any_down()) {
            self.world.release_drag();
        }
        if !ctx.egui_wants_keyboard_input() {
            for a in self.s.keymap.actions(&ctx) {
                self.act(&a);
            }
        }
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::left("side").default_size(280.0).show(ui, |ui| self.side(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui, dt));
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for l in [Layout::Single, Layout::Side, Layout::Quad] {
                if ui.selectable_label(self.world.state.layout == l, l.name()).clicked() {
                    self.world.state.layout = l;
                    self.world.state.active = self.world.state.active.min(l.slots().len() - 1);
                }
            }
            ui.separator();
            let slot = self.world.state.active.min(self.world.slot_count() - 1);
            let mut mode = self.world.state.vp[slot].mode;
            egui::ComboBox::from_id_salt("mode").selected_text(MODES[mode].1).show_ui(ui, |ui| {
                for (i, (_, label)) in MODES.iter().enumerate() {
                    ui.selectable_value(&mut mode, i, *label);
                }
            });
            if mode != self.world.state.vp[slot].mode {
                self.world.set_mode(slot, mode);
            }
            let mut godot_on = self.renderer.sources[slot].name() == "godot";
            let was = godot_on;
            egui::ComboBox::from_id_salt("renderer").selected_text(if godot_on { "Godot renderer" } else { "GazeForge renderer" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut godot_on, false, "GazeForge renderer");
                ui.selectable_value(&mut godot_on, true, "Godot renderer");
            });
            if godot_on != was {
                self.set_godot(slot, godot_on);
            }
            let lods = self.world.lods();
            let mut lod = self.world.state.vp[slot].lod.min(lods - 1);
            let label = |i: usize, w: &World| w.session.as_ref().map_or(format!("LOD{i}"), |s| s.head.lods[i].label.clone());
            egui::ComboBox::from_id_salt("lod").selected_text(label(lod, self.world)).show_ui(ui, |ui| {
                for i in 0..lods {
                    ui.selectable_value(&mut lod, i, label(i, self.world));
                }
            });
            self.world.state.vp[slot].lod = lod;
            ui.separator();
            for (id, text) in [("view_front", "front"), ("view_left", "side"), ("view_top", "top"), ("view_three_quarter", "3/4"), ("ortho", "ortho/persp"), ("frame", "frame")] {
                if ui.button(text).clicked() {
                    self.act(id);
                }
            }
            ui.separator();
            let o = &mut self.world.state.opts;
            ui.checkbox(&mut o.wire, "wire");
            ui.checkbox(&mut o.xray, "x-ray");
            ui.checkbox(&mut o.points, "points");
            ui.checkbox(&mut o.eyes, "eyes");
            ui.checkbox(&mut o.back_hi, "back faces");
            ui.checkbox(&mut o.seams, "seams");
            ui.checkbox(&mut o.clip[0].on, "clip");
            if o.clip[0].on {
                ui.add(egui::Slider::new(&mut o.clip[0].offset, -0.15..=0.15).text("offset m"));
            }
            ui.checkbox(&mut o.dark, "dark viewport");
            let mut light = ui.ctx().theme() == egui::Theme::Light;
            if ui.checkbox(&mut light, "light interface").changed() {
                ui.ctx().set_theme(if light { egui::ThemePreference::Light } else { egui::ThemePreference::Dark });
            }
        });
    }

    fn side(&mut self, ui: &mut egui::Ui) {
        ui.heading("Store");
        ui.label("store");
        ui.text_edit_singleline(&mut self.s.store);
        ui.label("spec (optional)");
        ui.text_edit_singleline(&mut self.s.spec);
        ui.label("macro definition (optional)");
        ui.text_edit_singleline(&mut self.s.macro_file);
        ui.label("eyes (optional)");
        ui.text_edit_singleline(&mut self.s.eyes);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.world.loading.is_none(), egui::Button::new("load")).clicked() {
                self.load();
            }
            if let Some(b) = self.world.busy_text() {
                ui.spinner();
                ui.label(b);
            }
        });
        ui.label(&self.s.status);
        if let Some(l) = self.world.log.last() {
            ui.label(l);
        }
        ui.separator();
        if let Some(s) = &self.world.session {
            ui.label(format!("{} game vertices, {} head targets", s.head.n, s.targets));
            if let Some(st) = &s.settled {
                ui.label(format!("settled {} (worker {:.0} ms)", st.placement, st.ms));
            }
        }
        if self.world.shape.is_some() {
            ui.horizontal(|ui| {
                let h = self.world.shape.as_ref().map_or((0, 0), |s| (s.history.done.len(), s.history.undone.len()));
                if ui.add_enabled(h.0 > 0, egui::Button::new("undo")).on_hover_text(self.s.keymap.describe("undo")).clicked() {
                    self.s.status = self.world.undo(false).unwrap_or_default();
                }
                if ui.add_enabled(h.1 > 0, egui::Button::new("redo")).on_hover_text(self.s.keymap.describe("redo")).clicked() {
                    self.s.status = self.world.undo(true).unwrap_or_default();
                }
                ui.add(egui::TextEdit::singleline(&mut self.s.save_path).hint_text("save as .gfchar.json").desired_width(120.0));
                if ui.button("save").on_hover_text(self.s.keymap.describe("save_spec")).clicked() {
                    self.save();
                }
            });
            ui.separator();
            ui.label(format!("shape input: {}", self.s.input.name()));
            let events = match self.world.shape.as_ref() {
                Some(shape) => self.s.input.show(ui, shape),
                None => Vec::new(),
            };
            for e in events {
                self.world.shape_event(e);
            }
        }
        ui.separator();
        ui.checkbox(&mut self.world.hud.on, "timings overlay");
        ui.label(format!("viewport render {:.2} ms, frame {:.1} ms", self.s.render_ms, self.s.frame_ms));
        ui.label(format!("keys: {}", self.s.keymap.source));
        for p in &self.s.keymap.problems {
            ui.colored_label(egui::Color32::LIGHT_RED, p);
        }
        if ui.button(if self.s.show_help { "hide keys" } else { "show keys" }).clicked() {
            self.s.show_help = !self.s.show_help;
        }
        if self.s.show_help {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (id, what, _) in keymap::ACTIONS {
                    ui.label(format!("{}: {what}", self.s.keymap.describe(id)));
                }
                ui.label("left drag: orbit; right, middle or Shift drag: pan; wheel: zoom; Alt+wheel: clip plane 1; double-click: pivot");
            });
        }
    }

    fn input(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect, ppp: f32, w: u32, h: u32) {
        let rects = viewer::rects(self.world.state.layout, w, h);
        let at = |p: egui::Pos2| {
            let (x, y) = (((p.x - rect.min.x) * ppp) as u32, ((p.y - rect.min.y) * ppp) as u32);
            rects.iter().copied().find(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).unwrap_or(rects[0])
        };
        let (shift, alt, secondary, middle) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt, i.pointer.button_down(egui::PointerButton::Secondary), i.pointer.button_down(egui::PointerButton::Middle)));
        if resp.drag_started()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let r = at(p);
            self.world.state.active = r.slot;
            self.s.drag = Some((r.cam, secondary || middle || shift, r.h as f32 / ppp));
            self.world.state.pivot_flash = 0.4;
        }
        if resp.dragged()
            && let Some((cam, pan, hh)) = self.s.drag
        {
            let d = resp.drag_delta();
            if pan {
                self.world.state.pan(cam, d.x as f64, d.y as f64, hh as f64, false);
            } else {
                self.world.state.orbit(cam, d.x as f64, d.y as f64, false);
            }
            self.world.state.pivot_flash = 0.4;
        }
        if resp.drag_stopped() {
            self.s.drag = None;
        }
        if resp.hovered() {
            let dy = ui.input(|i| i.smooth_scroll_delta.y) as f64;
            if dy != 0.0 {
                if alt {
                    let p = &mut self.world.state.opts.clip[0];
                    p.on = true;
                    p.offset = (p.offset + dy.signum() * 0.001).clamp(-0.15, 0.15);
                } else if let Some(p) = resp.hover_pos() {
                    let r = at(p);
                    self.world.state.zoom(r.cam, -dy * 2.0, false);
                }
            }
        }
        if resp.double_clicked()
            && let (Some(p), Some(s)) = (resp.interact_pointer_pos(), &self.world.session)
        {
            let r = at(p);
            let (px, py) = ((p.x - rect.min.x) * ppp - r.x as f32, (p.y - rect.min.y) * ppp - r.y as f32);
            match render::pick(&self.world.state, &s.head, &r, px as f64, py as f64) {
                Some((_, hit)) => {
                    self.world.state.cams[r.cam].set(|g| g.target = hit, false);
                    self.world.state.pivot_flash = 0.8;
                    self.s.status = format!("pivot at {:.1}, {:.1}, {:.1} mm", hit[0] * 1000.0, hit[1] * 1000.0, hit[2] * 1000.0);
                }
                None => self.s.status = "no surface under the cursor".into(),
            }
        }
    }

    fn viewport(&mut self, ui: &mut egui::Ui, dt: f64) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        let (w, h) = (((rect.width() * ppp).round() as u32).max(1), ((rect.height() * ppp).round() as u32).max(1));
        self.input(ui, &resp, rect, ppp, w, h);
        if self.spin {
            let a = self.world.state.active_rect(w, h);
            self.world.state.orbit(a.cam, 2.0, 0.0, true);
        }
        let moving = self.world.state.step(dt);
        self.world.tick();
        self.renderer.sync_world(self.gfx.device, self.gfx.queue, self.world);
        self.renderer.dpr = ppp as f64;
        let head = self.world.session.as_ref().map(|s| &s.head);
        let f = self.renderer.render(self.gfx.device, self.gfx.queue, &self.world.state, head, w, h);
        self.world.frame_done(Instant::now());
        self.s.render_ms = f.encode_ms;
        let comp = self.renderer.composite.as_ref().expect("rendered above");
        if self.s.texture.is_none() || self.s.texture_generation != comp.generation {
            let mut r = self.gfx.egui.write();
            match self.s.texture {
                None => self.s.texture = Some(r.register_native_texture(self.gfx.device, &comp.srgb_view, wgpu::FilterMode::Nearest)),
                Some(id) => r.update_egui_texture_from_wgpu_texture(self.gfx.device, &comp.srgb_view, wgpu::FilterMode::Nearest, id),
            }
            self.s.texture_generation = comp.generation;
        }
        let painter = ui.painter_at(rect);
        if let Some(id) = self.s.texture {
            painter.image(id, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        let rects = viewer::rects(self.world.state.layout, w, h);
        let text_col = if self.world.state.opts.dark { egui::Color32::from_gray(200) } else { egui::Color32::from_gray(40) };
        for r in &rects {
            let min = rect.min + egui::vec2(r.x as f32 / ppp, r.y as f32 / ppp);
            let cam = &self.world.state.cams[r.cam];
            let st = self.world.state.vp[r.slot];
            let lod = if self.world.lods() > 1 { format!(" · LOD{}", st.lod) } else { String::new() };
            let source = &self.renderer.sources[r.slot];
            let src = match source {
                render::Source::Native => String::new(),
                render::Source::External(e) => format!(" · {}", e.status().unwrap_or_else(|| e.name().to_string())),
            };
            painter.text(min + egui::vec2(8.0, 6.0), egui::Align2::LEFT_TOP, format!("{} · {} · {}{lod}{src}", cam.name, if cam.ortho { "ortho" } else { "persp" }, MODES[st.mode].1), egui::FontId::proportional(13.0), text_col);
            if rects.len() > 1 && r.slot == self.world.state.active {
                let rr = egui::Rect::from_min_size(min, egui::vec2(r.w as f32 / ppp, r.h as f32 / ppp)).shrink(1.0);
                painter.rect_stroke(rr, 0.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(90, 150, 240)), egui::StrokeKind::Inside);
            }
        }
        if self.world.hud.on {
            let mut lines = self.world.hud.lines();
            if let Some(b) = self.world.busy_text() {
                lines.push(b);
            }
            let pos = rect.right_top() + egui::vec2(-8.0, 6.0);
            let font = egui::FontId::monospace(12.0);
            let galley = painter.layout_no_wrap(lines.join("\n"), font, text_col);
            let r = egui::Rect::from_min_size(pos - egui::vec2(galley.size().x, 0.0), galley.size()).expand(4.0);
            painter.rect_filled(r, 3.0, if self.world.state.opts.dark { egui::Color32::from_black_alpha(140) } else { egui::Color32::from_white_alpha(170) });
            painter.galley(r.min + egui::vec2(4.0, 4.0), galley, text_col);
        }
        if moving || self.keep_painting || self.world.busy_text().is_some() {
            ui.ctx().request_repaint();
        } else if self.godot.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(15));
        }
    }
}
