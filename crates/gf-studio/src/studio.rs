use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;

use crate::browse::Pick;
use crate::cube::{self, Cube, Element};
use crate::godot;
use crate::i18n::{tr, tr_or, trf};
use crate::icons;
use crate::input::{self, ShapeInput};
use crate::keymap::{self, Keymap};
use crate::render::{self, Renderer};
use crate::script::write_png;
use crate::viewer::{self, Layout, MODES};
use crate::world::World;

const RIGHT_GROUP_WIDTH: f32 = 5.0 * 28.0 + 4.0 * 2.0;
const CLIP_SLIDER_WIDTH: f32 = 230.0;
const SIDE_FOOTER_PX: f32 = 300.0;
const SIDE_PANELS_MIN_PX: f32 = 160.0;
const DEFAULT_EYE: &str = "brown-medium";

pub struct UiState {
    pub keymap: Keymap,
    pub input: Box<dyn ShapeInput>,
    pub save_path: String,
    pub store: String,
    pub spec: String,
    pub macro_file: String,
    pub eyes: String,
    pub game: String,
    pub status: String,
    texture: Option<egui::TextureId>,
    texture_generation: u64,
    drag: Option<(usize, bool, f32)>,
    skin_rect: Option<viewer::Rect>,
    pub skin_colour_layer: Option<usize>,
    pub render_ms: f64,
    pub frame_ms: f64,
    pub show_help: bool,
    pub godot_config: Option<PathBuf>,
    pub flip_theme: bool,
    pub cubes: Vec<CubeSpot>,
    pub cube_hover: Option<(usize, Element)>,
    pub cube_drag: Option<usize>,
    pub wizards: crate::wizard::Wizards,
    pub project: crate::project::ProjectUi,
    pub load_ask: Option<LoadAsk>,
    pub eye_note_hidden: bool,
    pub browse: crate::browse::Browse,
}

#[derive(Clone)]
pub enum LoadAsk {
    Fields,
    Spec(String, String),
    Game(String),
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
            game: String::new(),
            status: tr("studio.status.open").into(),
            texture: None,
            texture_generation: 0,
            drag: None,
            skin_rect: None,
            skin_colour_layer: None,
            render_ms: 0.0,
            frame_ms: 0.0,
            show_help: false,
            godot_config: a.godot_config,
            flip_theme: false,
            cubes: Vec::new(),
            cube_hover: None,
            cube_drag: None,
            wizards: Default::default(),
            project: Default::default(),
            load_ask: None,
            eye_note_hidden: false,
            browse: Default::default(),
        })
    }
}

fn placed(kind: &str, typed: &str) -> PathBuf {
    let p = PathBuf::from(typed);
    let p = if p.is_absolute() { p } else { gf_api::project::data_dir(kind).join(p) };
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    p
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
    style(ctx);
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
        let (spec, mac, eyes, game) = (opt(&self.s.spec), opt(&self.s.macro_file), opt(&self.s.eyes), opt(&self.s.game));
        self.world.request_load(self.s.store.trim(), spec.as_deref(), mac.as_deref(), eyes.as_deref(), game.as_deref());
        self.s.status = tr("studio.status.loading").into();
    }

    pub fn ask_load(&mut self, a: LoadAsk) {
        let store = match &a {
            LoadAsk::Spec(s, _) if !s.trim().is_empty() => s,
            _ => &self.s.store,
        };
        if store.trim().is_empty() {
            self.s.status = tr("studio.store_needed").into();
            return;
        }
        if self.world.loading.is_none() && self.world.project_dirty() {
            self.s.load_ask = Some(a);
        } else {
            self.load_now(a);
        }
    }

    fn load_now(&mut self, a: LoadAsk) {
        match a {
            LoadAsk::Fields => self.world.project_detach(),
            LoadAsk::Spec(store, spec) => {
                if !store.is_empty() {
                    self.s.store = store;
                }
                self.s.spec = spec;
            }
            LoadAsk::Game(g) => {
                self.s.game = g;
                self.world.topo.reset();
            }
        }
        self.load();
    }

    fn load_ask_window(&mut self, ctx: &egui::Context) {
        let Some(a) = self.s.load_ask.clone() else { return };
        let mut discard = None;
        let r = egui::Modal::new(egui::Id::new("load_ask")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.heading(tr("designer.unsaved"));
            ui.label(tr("studio.load_discards"));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tr("designer.discard_changes")).clicked() {
                    discard = Some(true);
                }
                if ui.button(tr("ui.p-dlg-cancel")).clicked() {
                    discard = Some(false);
                }
            });
        });
        if discard.is_none() && r.should_close() {
            discard = Some(false);
        }
        if let Some(d) = discard {
            self.s.load_ask = None;
            if d {
                self.load_now(a);
            }
        }
    }

    fn save(&mut self) {
        let typed = if self.s.save_path.trim().is_empty() {
            let name = self.world.session.as_ref().map_or("character".to_string(), |s| s.character.name.clone());
            format!("{name}.gfchar.json")
        } else {
            self.s.save_path.trim().to_string()
        };
        let path = placed("characters", &typed).display().to_string();
        self.s.save_path = path.clone();
        self.s.status = self.world.save(&path).err().unwrap_or_default();
    }

    pub fn act(&mut self, id: &str) {
        match id {
            "screenshot" | "screenshot_layout" => self.s.status = self.screenshot(id == "screenshot_layout"),
            "help" => self.s.show_help = !self.s.show_help,
            "tasks" => self.s.wizards.open = !self.s.wizards.open,
            _ if id.starts_with("topology_") => {
                if let Some(m) = self.topology_act(id) {
                    self.s.status = m;
                }
            }
            "interface_theme" => self.s.flip_theme = true,
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
        let Some(c) = self.renderer.composite.as_ref() else { return tr("studio.nothing_rendered").into() };
        let (x, y, w, h) = if layout {
            (0, 0, c.w, c.h)
        } else {
            let a = self.world.state.active_rect(c.w, c.h);
            (a.x, a.y, a.w, a.h)
        };
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let path = placed("screenshots", &format!("gazeforge-studio-{stamp}.png"));
        match self.renderer.read_rgba(self.gfx.device, self.gfx.queue, x, y, w, h).and_then(|px| write_png(&path, w, h, &px)) {
            Ok(()) => trf("studio.saved", &[("path", &path.display())]),
            Err(e) => trf("studio.screenshot_failed", &[("error", &e)]),
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
        if std::mem::take(&mut self.s.flip_theme) {
            let light = ctx.theme() != egui::Theme::Light;
            ctx.set_theme(if light { egui::ThemePreference::Light } else { egui::ThemePreference::Dark });
            self.s.status = if light { tr("designer.theme_light") } else { tr("designer.theme_dark") }.into();
        }
        let show_bad = self.world.state.opts.bad;
        self.world.with_topo(|t, w| t.sync(w, show_bad));
        if let Some(m) = self.world.topo.poll() {
            self.s.status = m;
        }
        if self.world.topo.running() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::Panel::left("side").default_size(300.0).show(ui, |ui| self.side(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui, dt));
        self.keys_window(&ctx);
        let slot = self.world.project.slot();
        self.s.wizards.show(&ctx, self.world, &slot);
        if let Some((store, spec)) = self.s.wizards.open_spec.take() {
            self.ask_load(LoadAsk::Spec(store, spec));
        }
        self.load_ask_window(&ctx);
    }

    fn keymap_source(&self) -> String {
        if self.s.keymap.source == "defaults" { tr("studio.keys_defaults").to_string() } else { self.s.keymap.source.clone() }
    }

    pub fn undo_hint(&mut self, what: &str) {
        let keys = self.s.keymap.describe("undo");
        self.s.status = if keys.is_empty() { trf("studio.undo_hint_plain", &[("what", &what)]) } else { trf("studio.undo_hint", &[("what", &what), ("keys", &keys)]) };
    }

    pub fn tip(&self, what: &str, id: &str) -> String {
        let keys = self.s.keymap.describe(id);
        if keys.is_empty() { what.to_string() } else { format!("{what} ({keys})") }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(3.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (l, icon, what) in [(Layout::Single, "square", tr("studio.layout.single")), (Layout::Side, "columns-2", tr("studio.layout.side")), (Layout::Quad, "layout-grid", tr("studio.layout.quad"))] {
                let tip = trf("studio.layout.tip", &[("what", &what), ("keys", &self.s.keymap.describe("layout"))]);
                if icons::button(ui, icon, self.world.state.layout == l, &tip).clicked() {
                    self.world.state.layout = l;
                    self.world.state.active = self.world.state.active.min(l.slots().len() - 1);
                }
            }
            group_gap(ui);
            ui.spacing_mut().item_spacing.x = 6.0;
            let slot = self.world.state.active.min(self.world.slot_count() - 1);
            let mut mode = self.world.state.vp[slot].mode;
            let mode_tip = trf("studio.mode_tip", &[("next", &self.s.keymap.describe("mode_next")), ("prev", &self.s.keymap.describe("mode_prev"))]);
            egui::ComboBox::from_id_salt("mode").width(170.0).selected_text(tr(&format!("mode.{}", MODES[mode]))).show_ui(ui, |ui| {
                for (i, id) in MODES.iter().enumerate() {
                    ui.selectable_value(&mut mode, i, tr(&format!("mode.{id}")));
                }
            }).response.on_hover_text(mode_tip);
            if mode != self.world.state.vp[slot].mode {
                self.world.set_mode(slot, mode);
            }
            let lods = self.world.lods();
            let mut lod = self.world.state.vp[slot].lod.min(lods - 1);
            let label = |i: usize, w: &World| w.session.as_ref().map_or(format!("LOD{i}"), |s| s.head.lods[i].label.clone());
            let lod_tip = trf("studio.lod_tip", &[("coarser", &self.s.keymap.describe("lod_coarser")), ("finer", &self.s.keymap.describe("lod_finer"))]);
            egui::ComboBox::from_id_salt("lod").width(90.0).selected_text(label(lod, self.world)).show_ui(ui, |ui| {
                for i in 0..lods {
                    ui.selectable_value(&mut lod, i, label(i, self.world));
                }
            }).response.on_hover_text(lod_tip);
            self.world.state.vp[slot].lod = lod;
            let mut godot_on = self.renderer.sources[slot].name() == "godot";
            let was = godot_on;
            egui::ComboBox::from_id_salt("renderer").width(130.0).selected_text(if godot_on { tr("studio.renderer.godot") } else { tr("studio.renderer.native") }).show_ui(ui, |ui| {
                ui.selectable_value(&mut godot_on, false, tr("studio.renderer.native"));
                ui.selectable_value(&mut godot_on, true, tr("studio.renderer.godot"));
            }).response.on_hover_text(tr("studio.renderer.tip"));
            if godot_on != was {
                self.set_godot(slot, godot_on);
            }
            group_gap(ui);
            ui.spacing_mut().item_spacing.x = 2.0;
            let ortho = self.world.state.cams[self.world.state.active_cam_index()].ortho;
            let view_buttons = [
                ("frame", "focus", false, tr("action.frame")),
                ("ortho", "box", ortho, tr("action.ortho")),
                ("view_three_quarter", "rotate-3d", false, tr("action.view_three_quarter")),
                ("view_home", "house", false, tr("action.view_home")),
            ];
            for (id, icon, on, what) in view_buttons {
                if icons::button(ui, icon, on, &self.tip(what, id)).clicked() {
                    self.act(id);
                }
            }
            group_gap(ui);
            let o = &self.world.state.opts;
            let toggles = [
                ("wire", "vector-square", o.wire, tr("action.wire")),
                ("xray", "scan-eye", o.xray, tr("action.xray")),
                ("points", "chart-scatter", o.points, tr("action.points")),
                ("eyes", "eye", o.eyes, tr("action.eyes")),
                ("back_faces", "flip-horizontal-2", o.back_hi, tr("action.back_faces")),
                ("seams", "scissors-line-dashed", o.seams, tr("action.seams")),
                ("light_gizmo", "lightbulb", o.light_gizmo, tr("action.light_gizmo")),
                ("clip", "slice", o.clip[0].on, tr("action.clip")),
                ("bad_triangles", "triangle-alert", o.bad, tr("action.bad_triangles")),
            ];
            for (id, icon, on, what) in toggles {
                if icons::button(ui, icon, on, &self.tip(what, id)).clicked() {
                    self.act(id);
                }
            }
            let o = &mut self.world.state.opts;
            if o.clip[0].on {
                ui.allocate_ui(egui::vec2(CLIP_SLIDER_WIDTH, 26.0), |ui| {
                    ui.spacing_mut().slider_width = CLIP_SLIDER_WIDTH - 110.0;
                    ui.add(egui::Slider::new(&mut o.clip[0].offset, -0.15..=0.15).text(tr("studio.clip_offset")).fixed_decimals(3));
                });
            }
            group_gap(ui);
            for (id, icon, what) in [("screenshot", "camera", tr("action.screenshot")), ("screenshot_layout", "images", tr("action.screenshot_layout"))] {
                if icons::button(ui, icon, false, &self.tip(what, id)).clicked() {
                    self.act(id);
                }
            }
            if ui.available_size_before_wrap().x < RIGHT_GROUP_WIDTH {
                ui.end_row();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.toolbar_right(ui));
        });
        ui.add_space(3.0);
    }

    fn toolbar_right(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = 2.0;
        let light = ui.ctx().theme() == egui::Theme::Light;
        let dark = self.world.state.opts.dark;
        let buttons = [
            ("help", "keyboard", self.s.show_help, self.tip(tr("studio.keys_title"), "help")),
            ("tasks", "list-checks", self.s.wizards.open, tr("ui.tasks-btn.title").to_string()),
            ("interface_theme", if light { "sun" } else { "moon" }, false,
             self.tip(if light { tr("studio.tip.light_ui") } else { tr("studio.tip.dark_ui") }, "interface_theme")),
            ("theme", "sun-moon", !dark, self.tip(if dark { tr("studio.tip.dark_view") } else { tr("studio.tip.light_view") }, "theme")),
            ("hud", "gauge", self.world.hud.on, self.tip(tr("action.hud"), "hud")),
        ];
        for (id, icon, on, tip) in buttons {
            if icons::button(ui, icon, on, &tip).clicked() {
                self.act(id);
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(b) = self.world.busy_text() {
                ui.spinner();
                ui.label(egui::RichText::new(b).color(ui.visuals().warn_fg_color));
                ui.separator();
            }
            let stale = self.world.session.is_some() && (self.s.status == tr("studio.status.open") || self.s.status == tr("studio.status.loading"));
            if !stale {
                ui.label(&self.s.status);
            }
            if let Some(l) = self.world.log.last()
                && *l != self.s.status
            {
                if !stale {
                    ui.separator();
                }
                ui.label(egui::RichText::new(l).weak());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(trf("studio.timing", &[("viewport", &format!("{:.2}", self.s.render_ms)), ("frame", &format!("{:.1}", self.s.frame_ms))])).weak().monospace());
            });
        });
    }

    fn side(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        let reserve = if self.world.shape.is_some() { SIDE_FOOTER_PX } else { 0.0 };
        egui::ScrollArea::vertical()
            .id_salt("side_panels")
            .max_height((ui.available_height() - reserve).max(SIDE_PANELS_MIN_PX))
            .auto_shrink([false, true])
            .show(ui, |ui| self.side_panels(ui));
        self.side_footer(ui);
    }

    fn side_panels(&mut self, ui: &mut egui::Ui) {
        let loaded = self.world.session.is_some();
        egui::CollapsingHeader::new(egui::RichText::new(tr("store.title")).strong()).id_salt("store").default_open(!loaded).show(ui, |ui| {
            egui::Grid::new("store_fields").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                let json = || Pick::Open(".json".into());
                for (label, value, hint, pick) in [
                    ("store", &mut self.s.store, tr("studio.field.store_hint"), Some(Pick::Folder)),
                    ("spec", &mut self.s.spec, tr("studio.field.spec_hint"), Some(Pick::Open(".gfchar.json".into()))),
                    ("macro", &mut self.s.macro_file, tr("studio.field.macro_hint"), Some(json())),
                    ("eyes", &mut self.s.eyes, tr("studio.field.eyes_hint"), Some(json())),
                    ("game", &mut self.s.game, tr("studio.field.game_hint"), None),
                ] {
                    ui.label(tr(&format!("studio.field.{label}")));
                    match pick {
                        Some(pick) => {
                            crate::browse::field(ui, &mut self.s.browse, label, value, hint, pick);
                        }
                        None => {
                            ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(f32::INFINITY));
                        }
                    }
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                let load = ui.add_enabled(self.world.loading.is_none(), egui::Button::new(tr("studio.load"))).on_hover_text(tr("studio.load_tip"));
                if load.clicked() {
                    self.ask_load(LoadAsk::Fields);
                }
            });
        });
        self.project_panel(ui);
        self.body_panel(ui);
        self.stitch_panel(ui);
        self.face_rig_panel(ui);
        self.skin_panel(ui);
        self.timeline_panel(ui);
        self.topology_panel(ui);
        egui::CollapsingHeader::new(tr("studio.diagnostics")).id_salt("diag").default_open(false).show(ui, |ui| {
            ui.checkbox(&mut self.world.hud.on, tr("action.hud"));
            ui.label(trf("studio.shape_input", &[("name", &tr_or(&format!("studio.input.{}", self.s.input.name()), self.s.input.name()))]));
            ui.label(trf("studio.keys_source", &[("source", &self.keymap_source())]));
            for p in &self.s.keymap.problems {
                ui.colored_label(egui::Color32::LIGHT_RED, p);
            }
        });
    }

    fn side_footer(&mut self, ui: &mut egui::Ui) {
        if let Some(s) = &self.world.session {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&s.character.name).strong());
                ui.label(egui::RichText::new(trf("studio.counts", &[("vertices", &s.head.n), ("targets", &s.targets)])).weak());
            });
            if let Some(st) = &s.settled {
                ui.label(egui::RichText::new(trf("studio.settled", &[("ms", &format!("{:.0}", st.ms))])).weak())
                    .on_hover_text(trf("designer.status.placement", &[("placement", &st.placement)]));
            }
        }
        if self.world.shape.is_some() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let h = (self.world.journal.can_undo() as usize, self.world.journal.can_redo() as usize);
                let undo_tip = self.tip(tr("action.undo"), "undo");
                let redo_tip = self.tip(tr("action.redo"), "redo");
                if ui.add_enabled_ui(h.0 > 0, |ui| icons::button(ui, "undo-2", false, &undo_tip)).inner.clicked() {
                    self.s.status = self.world.undo(false).unwrap_or_default();
                }
                if ui.add_enabled_ui(h.1 > 0, |ui| icons::button(ui, "redo-2", false, &redo_tip)).inner.clicked() {
                    self.s.status = self.world.undo(true).unwrap_or_default();
                }
                ui.add_space(6.0);
                if let Some(p) = self.s.browse.take("save") {
                    self.s.save_path = p;
                }
                ui.add(egui::TextEdit::singleline(&mut self.s.save_path).hint_text(tr("studio.save_hint")).desired_width(ui.available_width() - 64.0))
                    .on_hover_text(trf("studio.save_where", &[("dir", &gf_api::project::data_dir("characters").display())]));
                let near = if self.s.save_path.trim().is_empty() { gf_api::project::data_dir("characters").display().to_string() } else { self.s.save_path.clone() };
                self.s.browse.button(ui, "save", Pick::Save(".gfchar.json".into()), &near);
                let save_tip = self.tip(tr("action.save_spec"), "save_spec");
                if icons::button(ui, "save", false, &save_tip).clicked() {
                    self.save();
                }
            });
            ui.separator();
            let events = match self.world.shape.as_ref() {
                Some(shape) => self.s.input.show(ui, shape),
                None => Vec::new(),
            };
            for e in events {
                let reset = matches!(e, crate::shape::ShapeEvent::Reset) && self.world.shape.as_ref().is_some_and(|s| s.live != s.start);
                self.world.shape_event(e);
                if reset {
                    self.undo_hint(tr("designer.undo.reset"));
                }
            }
        }
    }

    fn keys_window(&mut self, ctx: &egui::Context) {
        let mut open = self.s.show_help;
        egui::Window::new(tr("studio.keys_title")).id(egui::Id::new("keys_window")).open(&mut open).collapsible(false).resizable(true).default_width(460.0).default_height(560.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("keys").num_columns(2).striped(true).spacing([16.0, 3.0]).show(ui, |ui| {
                    for (id, _) in keymap::ACTIONS {
                        let keys = self.s.keymap.describe(id);
                        ui.label(egui::RichText::new(if keys.is_empty() { "-".into() } else { keys }).monospace());
                        ui.label(tr(&format!("action.{id}")));
                        ui.end_row();
                    }
                });
                ui.separator();
                ui.label(tr("studio.help.mouse"));
                ui.label(tr("studio.help.cube"));
                ui.label(trf("studio.help.keymap", &[("source", &self.keymap_source())]));
            });
        });
        self.s.show_help = open;
    }

    fn input(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: egui::Rect, ppp: f32, w: u32, h: u32) {
        let rects = viewer::rects(self.world.state.layout, w, h);
        let at = |p: egui::Pos2| {
            let (x, y) = (((p.x - rect.min.x) * ppp) as u32, ((p.y - rect.min.y) * ppp) as u32);
            rects.iter().copied().find(|r| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).unwrap_or(rects[0])
        };
        let (shift, alt, secondary, middle) = ui.input(|i| (i.modifiers.shift, i.modifiers.alt, i.pointer.button_down(egui::PointerButton::Secondary), i.pointer.button_down(egui::PointerButton::Middle)));
        let local = |p: egui::Pos2, r: &viewer::Rect| (((p.x - rect.min.x) * ppp - r.x as f32) as f64, ((p.y - rect.min.y) * ppp - r.y as f32) as f64);
        let tool = self.world.skin.tool;
        self.world.skin.hover = match (tool, resp.hover_pos()) {
            (crate::skin::Tool::Brush, Some(p)) => {
                let r = at(p);
                let (px, py) = local(p, &r);
                self.world.skin_pick(&r, px, py).map(|h| h.2)
            }
            _ => None,
        };
        let skin_button = !(secondary || middle || shift);
        if tool == crate::skin::Tool::Brush && skin_button {
            if (resp.drag_started() || (resp.clicked() && !resp.double_clicked()))
                && let Some(p) = resp.interact_pointer_pos()
            {
                let r = at(p);
                let (px, py) = local(p, &r);
                if self.world.skin_paint_begin(&r, px, py) {
                    self.s.skin_rect = Some(r);
                }
            }
            if let (true, Some(r), Some(p)) = (resp.dragged(), self.s.skin_rect, resp.interact_pointer_pos()) {
                let (px, py) = local(p, &r);
                self.world.skin_paint_to(&r, px, py);
            }
        }
        let primary_down = ui.input(|i| i.pointer.primary_down());
        if (self.s.skin_rect.is_some() || self.world.skin_painting()) && (resp.drag_stopped() || resp.clicked() || !primary_down || !self.world.skin_painting()) {
            self.world.skin_paint_end();
            self.s.skin_rect = None;
        }
        if tool == crate::skin::Tool::Stamp
            && skin_button
            && resp.clicked()
            && !resp.double_clicked()
            && let Some(p) = resp.interact_pointer_pos()
        {
            let r = at(p);
            let (px, py) = local(p, &r);
            self.world.skin_place_stamp(&r, px, py);
        }
        let skin_owns_drag = self.s.skin_rect.is_some() || self.world.skin_painting();
        if resp.drag_started()
            && !skin_owns_drag
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
            && tool == crate::skin::Tool::None
            && let (Some(p), Some(s)) = (resp.interact_pointer_pos(), &self.world.session)
        {
            let r = at(p);
            let (px, py) = ((p.x - rect.min.x) * ppp - r.x as f32, (p.y - rect.min.y) * ppp - r.y as f32);
            match render::pick(&self.world.state, &s.head, &r, px as f64, py as f64) {
                Some((_, hit)) => {
                    self.world.state.cams[r.cam].set(|g| g.target = hit, false);
                    self.world.state.pivot_flash = 0.8;
                    self.s.status = trf("designer.pivot", &[("mesh", &tr("mesh.head")), ("point", &format!("{:.1}, {:.1}, {:.1}", hit[0] * 1000.0, hit[1] * 1000.0, hit[2] * 1000.0))]);
                }
                None => self.s.status = tr("designer.no_surface").into(),
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
        self.cube_input(ui, rect, ppp, w, h);
        let moving = self.world.state.step(dt);
        self.world.journal.pointer_down = ui.input(|i| i.pointer.any_down());
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
                None => self.s.texture = Some(r.register_native_texture(self.gfx.device, &comp.view, wgpu::FilterMode::Nearest)),
                Some(id) => r.update_egui_texture_from_wgpu_texture(self.gfx.device, &comp.view, wgpu::FilterMode::Nearest, id),
            }
            self.s.texture_generation = comp.generation;
        }
        let painter = ui.painter_at(rect);
        if let Some(id) = self.s.texture {
            painter.image(id, rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
        }
        let rects = viewer::rects(self.world.state.layout, w, h);
        let dark = self.world.state.opts.dark;
        let text_col = if dark { egui::Color32::from_gray(200) } else { egui::Color32::from_gray(40) };
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
            let active = rects.len() > 1 && r.slot == self.world.state.active;
            let caption = format!("{} · {} · {}{lod}{src}", tr_or(&format!("view.{}", cam.name), &cam.name), if cam.ortho { tr("view.ortho") } else { tr("ui.persp") }, tr(&format!("mode.{}", MODES[st.mode])));
            let galley = painter.layout_no_wrap(caption, egui::FontId::proportional(13.0), if active { egui::Color32::from_rgb(120, 170, 250) } else { text_col });
            let at = min + egui::vec2(8.0, 6.0);
            painter.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(6.0, 2.0)), 4.0, if dark { egui::Color32::from_black_alpha(110) } else { egui::Color32::from_white_alpha(150) });
            painter.galley(at, galley, text_col);
            if active {
                let rr = egui::Rect::from_min_size(min, egui::vec2(r.w as f32 / ppp, r.h as f32 / ppp)).shrink(1.0);
                painter.rect_stroke(rr, 0.0, egui::Stroke::new(1.5, egui::Color32::from_rgb(90, 150, 240)), egui::StrokeKind::Inside);
            }
        }
        self.face_rig_paint(&painter, rect, ppp, w, h);
        self.skin_overlay(&painter, rect, ppp, w, h);
        self.cube_paint(ui, &painter);
        self.eye_note(ui.ctx(), rect);
        let busy = self.world.busy_text();
        if self.world.hud.on || busy.is_some() {
            let mut lines = if self.world.hud.on { self.world.hud.lines() } else { Vec::new() };
            if let Some(b) = busy {
                lines.push(b);
            }
            let font = egui::FontId::monospace(12.0);
            let galley = painter.layout_no_wrap(lines.join("\n"), font, text_col);
            let pos = rect.right_bottom() + egui::vec2(-8.0, -8.0);
            let r = egui::Rect::from_min_size(pos - galley.size(), galley.size()).expand(4.0);
            painter.rect_filled(r, 3.0, if dark { egui::Color32::from_black_alpha(140) } else { egui::Color32::from_white_alpha(170) });
            painter.galley(r.min + egui::vec2(4.0, 4.0), galley, text_col);
        }
        if moving || self.keep_painting || self.world.busy_text().is_some() || self.s.cube_drag.is_some() {
            ui.ctx().request_repaint();
        } else if self.godot.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(15));
        }
    }

    fn eye_note(&mut self, ctx: &egui::Context, rect: egui::Rect) {
        use gf_core::eyerig::{self as er, EyeAdjust, EyeballSpec};
        if self.s.eye_note_hidden || self.world.session.as_ref().is_none_or(|s| s.eyes_file.is_some()) {
            return;
        }
        let Some(shape) = self.world.shape.as_ref() else { return };
        let Some(rig) = shape.eyerig.clone() else { return };
        let live = shape.live.eyeballs.clone();
        if live.as_ref().is_some_and(|e| e.model.is_some()) {
            return;
        }
        let (mut add, mut hide) = (false, false);
        egui::Area::new(egui::Id::new("eye_note")).order(egui::Order::Foreground).pivot(egui::Align2::CENTER_BOTTOM).fixed_pos(rect.center_bottom() - egui::vec2(0.0, 40.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(tr("eyes.note"));
                    add = ui.button(tr("eyes.add")).on_hover_text(tr("eyes.add.title")).clicked();
                    hide = icons::button(ui, "x", false, tr("eyes.note_hide.title")).clicked();
                });
            });
        });
        if hide {
            self.s.eye_note_hidden = true;
        }
        if add {
            let (left, right) = live.map_or((EyeAdjust::default(), EyeAdjust::default()), |e| (e.left, e.right));
            let model = er::EyeModelChoice { id: DEFAULT_EYE.into(), radius_mm: gf_api::eyes::DEFAULT_RADIUS_MM };
            let spec = EyeballSpec { follow: er::FOLLOW_HELPER.into(), rig: Some(rig), model: Some(model), left, right };
            self.world.shape_event(crate::shape::ShapeEvent::Preview(vec![crate::shape::Edit::Eyes(Some(spec))]));
            self.world.shape_event(crate::shape::ShapeEvent::Commit(trf("eyes.undo_model", &[("name", &tr_or(&format!("eyes.model.{DEFAULT_EYE}"), DEFAULT_EYE))])));
        }
    }

    fn cube_spots(&self, rect: egui::Rect, ppp: f32, w: u32, h: u32) -> Vec<CubeSpot> {
        viewer::rects(self.world.state.layout, w, h)
            .iter()
            .map(|r| {
                let vr = egui::Rect::from_min_size(rect.min + egui::vec2(r.x as f32 / ppp, r.y as f32 / ppp), egui::vec2(r.w as f32 / ppp, r.h as f32 / ppp));
                let radius = if vr.width().min(vr.height()) >= 360.0 { 28.0 } else { 20.0 };
                let half = radius * 1.75;
                let centre = egui::pos2(vr.max.x - 10.0 - half, vr.min.y + 8.0 + half);
                let below = centre.y + half + 4.0;
                let home = egui::Rect::from_min_size(egui::pos2(centre.x - 24.0, below), egui::vec2(22.0, 20.0));
                let three = egui::Rect::from_min_size(egui::pos2(centre.x + 2.0, below), egui::vec2(22.0, 20.0));
                CubeSpot { slot: r.slot, cam: r.cam, centre, radius, home, three }
            })
            .collect()
    }

    pub fn cube_of(&self, spot: &CubeSpot) -> Cube {
        Cube::new(spot.centre, spot.radius, &viewer::Camera::basis(&self.world.state.cams[spot.cam].cur))
    }

    fn snap(&mut self, slot: usize, e: Element) {
        self.world.state.active = slot;
        if e.is_face() {
            self.act(&format!("view_{}", e.name()));
        } else {
            let (yaw, pitch) = cube::yaw_pitch(e.dir());
            self.world.state.view_dir(&e.name(), yaw, pitch, false);
            self.s.status = trf("studio.view_status", &[("view", &e.name())]);
        }
        self.world.state.pivot_flash = 0.4;
    }

    fn cube_input(&mut self, ui: &egui::Ui, rect: egui::Rect, ppp: f32, w: u32, h: u32) {
        let spots = self.cube_spots(rect, ppp, w, h);
        self.s.cube_hover = None;
        for spot in &spots {
            let cube = self.cube_of(spot);
            let id = ui.id().with(("view-cube", spot.slot));
            let resp = ui.interact(cube.bounds(), id, egui::Sense::click_and_drag());
            if let Some(p) = resp.hover_pos() {
                self.s.cube_hover = cube.pick(p).map(|e| (spot.slot, e));
            }
            if resp.drag_started() {
                self.s.cube_drag = Some(spot.slot);
                self.world.state.active = spot.slot;
            }
            if resp.dragged() && self.s.cube_drag == Some(spot.slot) {
                let d = resp.drag_delta();
                self.world.state.orbit(spot.cam, d.x as f64, d.y as f64, false);
                self.world.state.pivot_flash = 0.4;
            }
            if resp.drag_stopped() {
                self.s.cube_drag = None;
            }
            if resp.clicked()
                && let Some(e) = resp.interact_pointer_pos().and_then(|p| cube.pick(p))
            {
                self.snap(spot.slot, e);
            }
            for (r, which, tip_id) in [(spot.home, "view_home", "view_home"), (spot.three, "view_three_quarter", "view_three_quarter")] {
                let b = ui.interact(r, id.with(which), egui::Sense::click());
                let tip = if which == "view_home" { self.tip(tr("action.view_home"), tip_id) } else { self.tip(tr("action.view_three_quarter"), tip_id) };
                if b.clicked() {
                    self.world.state.active = spot.slot;
                    self.act(which);
                }
                b.on_hover_text(tip);
            }
            resp.on_hover_text(tr("studio.cube_tip"));
        }
        self.s.cubes = spots;
    }

    fn cube_paint(&self, ui: &egui::Ui, painter: &egui::Painter) {
        let dark = self.world.state.opts.dark;
        let pointer = ui.input(|i| i.pointer.hover_pos());
        for spot in &self.s.cubes {
            let cube = self.cube_of(spot);
            let hover = self.s.cube_hover.filter(|h| h.0 == spot.slot).map(|h| h.1);
            let backdrop = egui::Rect::from_center_size(spot.centre, egui::vec2(spot.radius, spot.radius) * 3.3);
            painter.circle_filled(backdrop.center(), backdrop.width() / 2.0, if dark { egui::Color32::from_black_alpha(70) } else { egui::Color32::from_white_alpha(90) });
            cube.paint(ui, painter, hover, dark);
            for (r, icon) in [(spot.home, "house"), (spot.three, "rotate-3d")] {
                let hot = pointer.is_some_and(|p| r.contains(p));
                painter.rect_filled(r, 4.0, if hot { egui::Color32::from_rgb(70, 130, 230) } else if dark { egui::Color32::from_black_alpha(120) } else { egui::Color32::from_white_alpha(170) });
                let ink = if hot || dark { egui::Color32::from_gray(225) } else { egui::Color32::from_gray(50) };
                icons::paint(painter, egui::Rect::from_center_size(r.center(), egui::vec2(14.0, 14.0)), icon, ink);
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct CubeSpot {
    pub slot: usize,
    pub cam: usize,
    pub centre: egui::Pos2,
    pub radius: f32,
    pub home: egui::Rect,
    pub three: egui::Rect,
}

fn group_gap(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let r = ui.available_rect_before_wrap();
    let x = r.min.x;
    ui.painter().vline(x, (r.center().y - 9.0)..=(r.center().y + 9.0), ui.visuals().widgets.noninteractive.bg_stroke);
    ui.add_space(6.0);
}

fn style(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = egui::vec2(6.0, 4.0);
        s.spacing.button_padding = egui::vec2(6.0, 3.0);
        s.spacing.interact_size.y = 22.0;
    });
}
