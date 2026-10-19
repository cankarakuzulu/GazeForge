use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;
use eframe::wgpu;
use serde_json::json;

use crate::godot;
use crate::input::{self, ShapeInput};
use crate::keymap::{self, Keymap};
use crate::render::{self, Renderer};
use crate::script::write_png;
use crate::viewer::{self, Layout, MODES};
use crate::world::World;

pub struct Args {
    pub store: Option<String>,
    pub spec: Option<String>,
    pub macro_file: Option<String>,
    pub keymap: Option<PathBuf>,
    pub bench_seconds: Option<f64>,
    pub present: Option<String>,
    pub input: Option<String>,
    pub godot_config: Option<PathBuf>,
}

struct Bench {
    seconds: f64,
    started: Option<Instant>,
    intervals: Vec<f64>,
    render_ms: Vec<f64>,
    ui_ms: Vec<f64>,
    surface: serde_json::Value,
}

pub struct StudioApp {
    renderer: Renderer,
    world: World,
    keymap: Keymap,
    input: Box<dyn ShapeInput>,
    save_path: String,
    store: String,
    spec: String,
    macro_file: String,
    status: String,
    texture: Option<egui::TextureId>,
    texture_generation: u64,
    epoch: u64,
    last: Instant,
    drag: Option<(usize, bool, f32)>,
    render_ms: f64,
    frame_ms: f64,
    show_help: bool,
    bench: Option<Bench>,
    godot: Option<std::sync::Arc<godot::Host>>,
    godot_config: Option<PathBuf>,
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

pub fn run(args: Args) -> Result<(), String> {
    let mut options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_title("GazeForge Studio"), ..Default::default() };
    options.wgpu_options.surface.present_mode = match args.present.as_deref() {
        None | Some("vsync") => wgpu::PresentMode::AutoVsync,
        Some("immediate") => wgpu::PresentMode::AutoNoVsync,
        Some(other) => return Err(format!("--present is vsync or immediate, not {other}")),
    };
    eframe::run_native("GazeForge Studio", options, Box::new(move |cc| StudioApp::new(cc, args).map(|a| Box::new(a) as Box<dyn eframe::App>).map_err(|e| e.into())))
        .map_err(|e| e.to_string())
}

impl StudioApp {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Result<StudioApp, String> {
        let font = system_font().ok_or("no system font found for the interface")?;
        let mut fonts = egui::FontDefinitions::empty();
        fonts.font_data.insert("ui".into(), std::sync::Arc::new(egui::FontData::from_owned(font)));
        fonts.families.insert(egui::FontFamily::Proportional, vec!["ui".into()]);
        fonts.families.insert(egui::FontFamily::Monospace, vec!["ui".into()]);
        cc.egui_ctx.set_fonts(fonts);
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        let rs = cc.wgpu_render_state.as_ref().ok_or("the window has no wgpu render state")?;
        let renderer = Renderer::new(&rs.device);
        let keymap = Keymap::load(args.keymap.as_deref());
        let name = args.input.clone().unwrap_or_else(|| "sliders".into());
        let input = input::by_name(&name).ok_or_else(|| format!("unknown input {name}; inputs: {}", input::INPUTS.join(", ")))?;
        let world = World::default();
        let ctx = cc.egui_ctx.clone();
        world.set_wake(Box::new(move || ctx.request_repaint()));
        let mut app = StudioApp {
            renderer,
            world,
            keymap,
            input,
            save_path: String::new(),
            store: args.store.clone().unwrap_or_default(),
            spec: args.spec.clone().unwrap_or_default(),
            macro_file: args.macro_file.clone().unwrap_or_default(),
            status: "open a store".into(),
            texture: None,
            texture_generation: 0,
            epoch: 0,
            last: Instant::now(),
            drag: None,
            render_ms: 0.0,
            frame_ms: 0.0,
            show_help: false,
            godot: None,
            godot_config: args.godot_config.clone(),
            bench: args.bench_seconds.map(|seconds| {
                let info = rs.adapter.get_info();
                let surface = json!({"adapter": info.name, "backend": format!("{:?}", info.backend), "target_format": format!("{:?}", rs.target_format)});
                Bench { seconds, started: None, intervals: Vec::new(), render_ms: Vec::new(), ui_ms: Vec::new(), surface }
            }),
        };
        if !app.store.is_empty() {
            app.load();
        }
        Ok(app)
    }

    fn load(&mut self) {
        let opt = |s: &String| if s.trim().is_empty() { None } else { Some(s.trim().to_string()) };
        let (spec, mac) = (opt(&self.spec), opt(&self.macro_file));
        self.world.request_load(self.store.trim(), spec.as_deref(), mac.as_deref());
        self.status = "loading".into();
    }

    fn save(&mut self) {
        let path = if self.save_path.trim().is_empty() {
            let name = self.world.session.as_ref().map_or("character".to_string(), |s| s.character.name.clone());
            format!("{name}.gfchar.json")
        } else {
            self.save_path.trim().to_string()
        };
        self.save_path = path.clone();
        self.status = match self.world.save(&path) {
            Ok(()) => format!("saving {path}"),
            Err(e) => e,
        };
    }

    fn act(&mut self, id: &str, frame: &eframe::Frame) {
        match id {
            "screenshot" | "screenshot_layout" => self.status = self.screenshot(id == "screenshot_layout", frame),
            "help" => self.show_help = !self.show_help,
            "save_spec" => self.save(),
            _ => {
                self.status = match self.world.action(id, false) {
                    Ok(m) => m,
                    Err(e) => e,
                }
            }
        }
    }

    fn screenshot(&mut self, layout: bool, frame: &eframe::Frame) -> String {
        let Some(rs) = frame.wgpu_render_state() else { return "no render state".into() };
        let Some(c) = self.renderer.composite.as_ref() else { return "nothing rendered yet".into() };
        let (x, y, w, h) = if layout {
            (0, 0, c.w, c.h)
        } else {
            let a = self.world.state.active_rect(c.w, c.h);
            (a.x, a.y, a.w, a.h)
        };
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let path = PathBuf::from(format!("gazeforge-studio-{stamp}.png"));
        match self.renderer.read_rgba(&rs.device, &rs.queue, x, y, w, h).and_then(|px| write_png(&path, w, h, &px)) {
            Ok(()) => format!("saved {}", path.display()),
            Err(e) => format!("screenshot failed: {e}"),
        }
    }

    fn set_godot(&mut self, slot: usize, on: bool) {
        if on {
            let host = match &self.godot {
                Some(h) if !matches!(h.status().0, godot::Status::Failed | godot::Status::Stopped) => h.clone(),
                _ => {
                    let mut cfg = godot::Config::load(self.godot_config.as_deref());
                    if cfg.log.is_none() {
                        cfg.log = Some(std::env::temp_dir().join("gazeforge-studio-godot.log"));
                    }
                    if cfg.background.is_none() {
                        cfg.background = Some(render::background(self.world.state.opts.dark));
                    }
                    let h = godot::Host::start(cfg);
                    self.godot = Some(h.clone());
                    h
                }
            };
            self.renderer.sources[slot] = render::Source::External(Box::new(godot::GodotSlot { host, slot }));
        } else {
            self.renderer.sources[slot] = render::Source::Native;
            if (0..4).all(|s| self.renderer.sources[s].name() != "godot") {
                self.godot = None;
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame) {
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
            egui::ComboBox::from_id_salt("lod").selected_text(label(lod, &self.world)).show_ui(ui, |ui| {
                for i in 0..lods {
                    ui.selectable_value(&mut lod, i, label(i, &self.world));
                }
            });
            self.world.state.vp[slot].lod = lod;
            ui.separator();
            for (id, text) in [("view_front", "front"), ("view_left", "side"), ("view_top", "top"), ("view_three_quarter", "3/4"), ("ortho", "ortho/persp"), ("frame", "frame")] {
                if ui.button(text).clicked() {
                    self.act(id, frame);
                }
            }
            ui.separator();
            let o = &mut self.world.state.opts;
            ui.checkbox(&mut o.wire, "wire");
            ui.checkbox(&mut o.xray, "x-ray");
            ui.checkbox(&mut o.points, "points");
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
        ui.text_edit_singleline(&mut self.store);
        ui.label("spec (optional)");
        ui.text_edit_singleline(&mut self.spec);
        ui.label("macro definition (optional)");
        ui.text_edit_singleline(&mut self.macro_file);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.world.loading.is_none(), egui::Button::new("load")).clicked() {
                self.load();
            }
            if let Some(b) = self.world.busy_text() {
                ui.spinner();
                ui.label(b);
            }
        });
        ui.label(&self.status);
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
                if ui.add_enabled(h.0 > 0, egui::Button::new("undo")).on_hover_text(self.keymap.describe("undo")).clicked() {
                    self.status = self.world.undo(false).unwrap_or_default();
                }
                if ui.add_enabled(h.1 > 0, egui::Button::new("redo")).on_hover_text(self.keymap.describe("redo")).clicked() {
                    self.status = self.world.undo(true).unwrap_or_default();
                }
                ui.add(egui::TextEdit::singleline(&mut self.save_path).hint_text("save as .gfchar.json").desired_width(120.0));
                if ui.button("save").on_hover_text(self.keymap.describe("save_spec")).clicked() {
                    self.save();
                }
            });
            ui.separator();
            ui.label(format!("shape input: {}", self.input.name()));
            let events = match self.world.shape.as_ref() {
                Some(shape) => self.input.show(ui, shape),
                None => Vec::new(),
            };
            for e in events {
                self.world.shape_event(e);
            }
        }
        ui.separator();
        ui.checkbox(&mut self.world.hud.on, "timings overlay");
        ui.label(format!("viewport render {:.2} ms, frame {:.1} ms", self.render_ms, self.frame_ms));
        ui.label(format!("keys: {}", self.keymap.source));
        for p in &self.keymap.problems {
            ui.colored_label(egui::Color32::LIGHT_RED, p);
        }
        if ui.button(if self.show_help { "hide keys" } else { "show keys" }).clicked() {
            self.show_help = !self.show_help;
        }
        if self.show_help {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (id, what, _) in keymap::ACTIONS {
                    ui.label(format!("{}: {what}", self.keymap.describe(id)));
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
            self.drag = Some((r.cam, secondary || middle || shift, r.h as f32 / ppp));
            self.world.state.pivot_flash = 0.4;
        }
        if resp.dragged()
            && let Some((cam, pan, hh)) = self.drag
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
            self.drag = None;
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
                    self.status = format!("pivot at {:.1}, {:.1}, {:.1} mm", hit[0] * 1000.0, hit[1] * 1000.0, hit[2] * 1000.0);
                }
                None => self.status = "no surface under the cursor".into(),
            }
        }
    }

    fn viewport(&mut self, ui: &mut egui::Ui, frame: &eframe::Frame, dt: f64) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        let ppp = ui.ctx().pixels_per_point();
        let (w, h) = (((rect.width() * ppp).round() as u32).max(1), ((rect.height() * ppp).round() as u32).max(1));
        self.input(ui, &resp, rect, ppp, w, h);
        if self.bench.as_ref().is_some_and(|b| b.started.is_some()) {
            let a = self.world.state.active_rect(w, h);
            self.world.state.orbit(a.cam, 2.0, 0.0, true);
        }
        let moving = self.world.state.step(dt);
        self.world.tick();
        let Some(rs) = frame.wgpu_render_state() else { return };
        if self.world.epoch != self.epoch {
            self.renderer.clear_head();
            self.epoch = self.world.epoch;
        }
        if let Some(s) = &self.world.session {
            self.renderer.sync_head(&rs.device, &rs.queue, &s.head, self.world.uvs_dirty);
            self.world.uvs_dirty = false;
        }
        self.renderer.dpr = ppp as f64;
        let head = self.world.session.as_ref().map(|s| &s.head);
        let f = self.renderer.render(&rs.device, &rs.queue, &self.world.state, head, w, h);
        self.world.frame_done(Instant::now());
        self.render_ms = f.encode_ms;
        let comp = self.renderer.composite.as_ref().expect("rendered above");
        if self.texture.is_none() || self.texture_generation != comp.generation {
            let mut r = rs.renderer.write();
            match self.texture {
                None => self.texture = Some(r.register_native_texture(&rs.device, &comp.srgb_view, wgpu::FilterMode::Nearest)),
                Some(id) => r.update_egui_texture_from_wgpu_texture(&rs.device, &comp.srgb_view, wgpu::FilterMode::Nearest, id),
            }
            self.texture_generation = comp.generation;
        }
        let painter = ui.painter_at(rect);
        if let Some(id) = self.texture {
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
        if moving || self.bench.is_some() || self.world.busy_text().is_some() {
            ui.ctx().request_repaint();
        } else if self.godot.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(15));
        }
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64();
        self.last = now;
        self.frame_ms = dt * 1000.0;
        self.world.hud.presented(now);
        let ctx = ui.ctx().clone();
        if let Some(t0) = self.bench.as_ref().and_then(|b| b.started.map(|t| (t, b.seconds))) {
            let (elapsed, half) = (t0.0.elapsed().as_secs_f64(), t0.1 / 2.0);
            if let (true, Some(id)) = (elapsed > half, self.world.shape.as_ref().and_then(|s| s.targets.first()).map(|t| t.id.clone())) {
                let v = 0.5 - 0.5 * ((elapsed - half) * 3.0).cos();
                self.world.shape_event(crate::shape::ShapeEvent::Preview(vec![crate::shape::Edit::Weight(id, v)]));
            }
        }
        let wants_text = ctx.egui_wants_keyboard_input();
        if !wants_text {
            for a in self.keymap.actions(&ctx) {
                self.act(&a, frame);
            }
        }
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui, frame));
        egui::Panel::left("side").default_size(280.0).show(ui, |ui| self.side(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui, frame, dt));
        self.world.hud.frame_cpu.push(now.elapsed().as_secs_f64() * 1000.0);
        let loaded = self.world.session.is_some();
        let mut finished = None;
        if let Some(b) = &mut self.bench
            && loaded
        {
            match b.started {
                None => b.started = Some(now),
                Some(t0) => {
                    b.intervals.push(dt * 1000.0);
                    b.render_ms.push(self.render_ms);
                    b.ui_ms.push(now.elapsed().as_secs_f64() * 1000.0);
                    b.surface["monitor_size"] = json!(ctx.input(|i| i.viewport().monitor_size.map(|s| [s.x, s.y])));
                    b.surface["inner_rect"] = json!(ctx.input(|i| i.viewport().inner_rect.map(|r| [r.min.x, r.min.y, r.width(), r.height()])));
                    b.surface["pixels_per_point"] = json!(ctx.pixels_per_point());
                    if t0.elapsed().as_secs_f64() >= b.seconds {
                        let mut r = bench_report(b);
                        r["hud"] = self.world.hud.json();
                        finished = Some(r);
                    }
                }
            }
        }
        if let Some(report) = finished {
            println!("{report}");
            self.bench = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn bench_report(b: &Bench) -> serde_json::Value {
    let stat = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.total_cmp(b));
        let n = s.len().max(1);
        let at = |q: f64| s.get(((n - 1) as f64 * q).round() as usize).copied().unwrap_or(0.0);
        json!({"count": s.len(), "mean": s.iter().sum::<f64>() / n as f64, "p50": at(0.5), "p95": at(0.95), "max": s.last().copied().unwrap_or(0.0)})
    };
    let total: f64 = b.intervals.iter().sum();
    json!({"ok": true, "mode": "window", "seconds": total / 1000.0, "frames": b.intervals.len(), "present_hz": b.intervals.len() as f64 / (total / 1000.0).max(1e-9),
           "frame_interval_ms": stat(&b.intervals), "viewport_encode_ms": stat(&b.render_ms), "ui_cpu_ms": stat(&b.ui_ms), "surface": b.surface})
}
