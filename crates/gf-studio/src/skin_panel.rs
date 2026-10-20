use eframe::egui;

use gf_core::skin_layers::{STYLES, SkinSpec};
use gf_core::skin_texture as st;

use crate::i18n::{tr, trf};
use crate::skin::{Tool, new_layer};
use crate::studio::Studio;
use crate::viewer;

impl Studio<'_> {
    pub fn skin_panel(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(tr("panel.skin")).id_salt("skin").default_open(false).show(ui, |ui| {
            if self.world.session.is_none() {
                ui.label(egui::RichText::new(tr("studio.load_first")).weak());
                return;
            }
            if self.world.skin.spec.is_none() {
                self.skin_create_ui(ui);
                return;
            }
            ui.horizontal(|ui| {
                if self.world.skin.busy() {
                    ui.spinner();
                }
                ui.label(egui::RichText::new(&self.world.skin.status).weak());
            });
            ui.checkbox(&mut self.world.state.opts.skin, tr("skin.show"));
            self.skin_look_ui(ui);
            ui.separator();
            self.skin_layers_ui(ui);
            ui.separator();
            self.skin_tools_ui(ui);
            ui.separator();
            ui.horizontal(|ui| {
                let (can_undo, can_redo) = (!self.world.skin.done.is_empty(), !self.world.skin.undone.is_empty());
                if ui.add_enabled(can_undo, egui::Button::new(tr("skin.undo"))).clicked() {
                    self.world.skin_undo(false);
                }
                if ui.add_enabled(can_redo, egui::Button::new(tr("skin.redo"))).clicked() {
                    self.world.skin_undo(true);
                }
            });
            if let Some(spec) = &self.world.skin.spec {
                for p in spec.problems() {
                    ui.colored_label(egui::Color32::from_rgb(230, 120, 60), p);
                }
            }
        });
    }

    fn skin_create_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(tr("skin.intro")).weak());
        ui.horizontal(|ui| {
            ui.label(tr("skin.landmarks"));
            ui.add(egui::TextEdit::singleline(&mut self.world.skin.landmarks_path).hint_text(tr("skin.landmarks_hint")));
        });
        self.skin_look_combo(ui);
        if ui.button(tr("skin.create")).clicked() {
            self.s.status = match self.world.skin_create() {
                Ok(()) => tr("skin.created").into(),
                Err(e) => trf("skin.failed", &[("error", &e)]),
            };
        }
    }

    fn skin_look_combo(&mut self, ui: &mut egui::Ui) -> bool {
        let mut id = self.world.skin.look_id.clone();
        ui.horizontal(|ui| {
            ui.label(tr("skin.look"));
            egui::ComboBox::from_id_salt("skin_look").selected_text(id.clone()).show_ui(ui, |ui| {
                for l in st::looks() {
                    ui.selectable_value(&mut id, l.id.clone(), l.id);
                }
            });
        });
        let changed = id != self.world.skin.look_id;
        self.world.skin.look_id = id;
        changed
    }

    fn skin_look_ui(&mut self, ui: &mut egui::Ui) {
        if self.skin_look_combo(ui)
            && let Some(look) = st::look(&self.world.skin.look_id)
        {
            self.world.skin_edit(|s| s.look = look);
        }
        let size = self.world.skin.spec.as_ref().map_or(0, |s| s.size);
        ui.horizontal(|ui| {
            ui.label(tr("skin.export_size"));
            for s in [1024usize, 2048, 4096] {
                if ui.selectable_label(size == s, s.to_string()).clicked() && size != s {
                    self.world.skin_edit(|spec| spec.size = s);
                }
            }
        });
    }

    fn skin_layers_ui(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new(tr("skin.layers")).strong());
        let rows: Vec<(String, String, String)> = self.world.skin.spec.as_ref().map(|s| s.layers.iter().map(|l| (l.name.clone(), l.kind.clone(), l.style.clone())).collect()).unwrap_or_default();
        for (i, (name, kind, style)) in rows.iter().enumerate() {
            let text = trf("skin.layer_row", &[("name", name), ("kind", &tr(&format!("skin.kind.{kind}"))), ("style", &tr(&format!("skin.style.{style}")))]);
            if ui.selectable_label(self.world.skin.selected == i, text).clicked() {
                self.world.skin.selected = i;
            }
        }
        ui.horizontal_wrapped(|ui| {
            for (kind, styles) in STYLES {
                ui.menu_button(tr(&format!("skin.add.{kind}")), |ui| {
                    for style in styles {
                        if ui.button(tr(&format!("skin.style.{style}"))).clicked() {
                            let (kind, style) = (kind.to_string(), style.to_string());
                            self.world.skin_edit(|s| {
                                let l = new_layer(s, &kind, &style);
                                s.layers.push(l);
                            });
                            self.world.skin.selected = rows.len();
                            ui.close();
                        }
                    }
                });
            }
            ui.menu_button(tr("skin.add_preset"), |ui| {
                for (name, layers) in gf_core::skin_authored::presets() {
                    if ui.button(tr(&format!("skin.preset.{name}"))).clicked() {
                        self.world.skin_edit(|s| add_preset(s, layers));
                        ui.close();
                    }
                }
            });
        });
        self.skin_layer_editor(ui);
    }

    fn skin_layer_editor(&mut self, ui: &mut egui::Ui) {
        let i = self.world.skin.selected;
        let Some(layer) = self.world.skin.spec.as_ref().and_then(|s| s.layers.get(i)).cloned() else { return };
        let response = layer.response();
        let mut l = layer.clone();
        let mut live = l.is_live();
        let mut procedural = l.is_procedural();
        let mut started = false;
        ui.horizontal(|ui| {
            ui.checkbox(&mut live, tr("skin.live")).on_hover_text(tr("skin.live_title"));
            ui.checkbox(&mut procedural, tr("skin.procedural")).on_hover_text(tr("skin.procedural_title"));
        });
        if live != layer.is_live() {
            l.live = Some(live);
        }
        if procedural != layer.is_procedural() {
            l.procedural = Some(procedural);
        }
        let amount = ui.add(egui::Slider::new(&mut l.amount, 0.0..=1.0).text(tr("skin.amount")).fixed_decimals(2));
        started |= amount.drag_started();
        if let Some((state, _, _)) = response.and_then(|r| r.variant) {
            let v = ui.add(egui::Slider::new(&mut l.variant, 0.0..=1.0).text(tr(&format!("skin.variant.{state}"))).fixed_decimals(2));
            started |= v.drag_started();
        }
        ui.horizontal(|ui| {
            let mut c = l.colour.unwrap_or_else(|| response.map_or([0.5; 3], |r| r.colour.map(|x| x as f64))).map(|x| x as f32);
            ui.label(tr("skin.colour"));
            if egui::color_picker::color_edit_button_rgb(ui, &mut c).changed() {
                l.colour = Some(c.map(|x| x as f64));
            }
            if l.colour.is_some() && ui.small_button(tr("skin.colour_reset")).clicked() {
                l.colour = None;
            }
            ui.label(tr("skin.seed"));
            ui.add(egui::DragValue::new(&mut l.seed).range(0..=999_999));
        });
        ui.label(egui::RichText::new(trf("skin.authored", &[("strokes", &l.strokes.len().to_string()), ("stamps", &l.stamps.len().to_string())])).weak());
        let mut remove = false;
        let mut moved: Option<isize> = None;
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!l.strokes.is_empty(), egui::Button::new(tr("skin.clear_strokes"))).clicked() {
                l.strokes.clear();
            }
            if ui.add_enabled(!l.stamps.is_empty(), egui::Button::new(tr("skin.clear_stamps"))).clicked() {
                l.stamps.clear();
            }
            if ui.small_button(tr("skin.up")).clicked() {
                moved = Some(-1);
            }
            if ui.small_button(tr("skin.down")).clicked() {
                moved = Some(1);
            }
            if ui.small_button(tr("skin.remove")).clicked() {
                remove = true;
            }
        });
        if remove {
            self.world.skin_edit(|s| {
                s.layers.remove(i);
            });
            return;
        }
        if let Some(d) = moved {
            let j = i as isize + d;
            let n = self.world.skin.spec.as_ref().map_or(0, |s| s.layers.len()) as isize;
            if (0..n).contains(&j) {
                self.world.skin_edit(|s| s.layers.swap(i, j as usize));
                self.world.skin.selected = j as usize;
            }
            return;
        }
        if l == layer {
            return;
        }
        let only_values = l.strokes == layer.strokes && l.stamps == layer.stamps && l.kind == layer.kind && l.style == layer.style && l.seed == layer.seed && l.procedural == layer.procedural;
        if started || !only_values || amount.drag_stopped() {
            self.world.skin_checkpoint();
        }
        let dragging = amount.dragged();
        if dragging || only_values {
            self.world.skin_tweak(|s| s.layers[i] = l);
        } else {
            self.world.skin_edit(|s| s.layers[i] = l);
        }
    }

    fn skin_tools_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(tr("skin.tool"));
            let t = &mut self.world.skin.tool;
            ui.selectable_value(t, Tool::None, tr("skin.tool.none"));
            ui.selectable_value(t, Tool::Brush, tr("skin.tool.brush")).on_hover_text(tr("skin.tool.brush_title"));
            ui.selectable_value(t, Tool::Stamp, tr("skin.tool.stamp")).on_hover_text(tr("skin.tool.stamp_title"));
        });
        let b = &mut self.world.skin.brush;
        match self.world.skin.tool {
            Tool::Brush => {
                ui.add(egui::Slider::new(&mut b.radius_mm, 0.5..=40.0).text(tr("skin.brush.radius")).logarithmic(true));
                ui.add(egui::Slider::new(&mut b.strength, 0.0..=1.0).text(tr("skin.brush.strength")));
                ui.add(egui::Slider::new(&mut b.hardness, 0.0..=1.0).text(tr("skin.brush.hardness")));
                ui.checkbox(&mut b.erase, tr("skin.brush.erase"));
            }
            Tool::Stamp => {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.world.skin.stamp_path).hint_text(tr("skin.stamp.path_hint")));
                    if ui.button(tr("skin.stamp.load")).clicked() {
                        self.s.status = match self.world.skin_load_stamp() {
                            Ok(()) => tr("skin.stamp.loaded").into(),
                            Err(e) => trf("skin.failed", &[("error", &e)]),
                        };
                    }
                });
                ui.add(egui::Slider::new(&mut self.world.skin.stamp_size_mm, 2.0..=200.0).text(tr("skin.stamp.size")).logarithmic(true));
                ui.add(egui::Slider::new(&mut self.world.skin.stamp_angle_deg, -180.0..=180.0).text(tr("skin.stamp.angle")));
                ui.add(egui::Slider::new(&mut self.world.skin.stamp_strength, 0.0..=1.0).text(tr("skin.brush.strength")));
                ui.checkbox(&mut self.world.skin.brush.erase, tr("skin.brush.erase"));
            }
            Tool::None => {}
        }
    }

    pub fn skin_overlay(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f32, w: u32, h: u32) {
        let (Some(hit), Tool::Brush) = (self.world.skin.hover, self.world.skin.tool) else { return };
        let radius_m = self.world.skin.brush.radius_mm / 1000.0;
        for r in viewer::rects(self.world.state.layout, w, h) {
            let cam = &self.world.state.cams[r.cam];
            let m = cam.matrices(r.w as f64 / r.h.max(1) as f64);
            let mvp = m.mvp;
            let to = |p: [f64; 3]| {
                let c = |row: usize| mvp[row] as f64 * p[0] + mvp[4 + row] as f64 * p[1] + mvp[8 + row] as f64 * p[2] + mvp[12 + row] as f64;
                let wc = c(3);
                if wc.abs() < 1e-9 {
                    return None;
                }
                Some(egui::pos2(rect.min.x + (r.x as f64 + (c(0) / wc + 1.0) / 2.0 * r.w as f64) as f32 / ppp, rect.min.y + (r.y as f64 + (1.0 - c(1) / wc) / 2.0 * r.h as f64) as f32 / ppp))
            };
            let side = m.s;
            let edge = [hit[0] + side[0] * radius_m, hit[1] + side[1] * radius_m, hit[2] + side[2] * radius_m];
            if let (Some(c), Some(e)) = (to(hit), to(edge)) {
                let clip = egui::Rect::from_min_size(rect.min + egui::vec2(r.x as f32 / ppp, r.y as f32 / ppp), egui::vec2(r.w as f32 / ppp, r.h as f32 / ppp));
                let colour = if self.world.skin.brush.erase { egui::Color32::from_rgb(120, 200, 255) } else { egui::Color32::from_rgb(255, 160, 60) };
                painter.with_clip_rect(clip).circle_stroke(c, (e - c).length().max(2.0), egui::Stroke::new(1.5, colour));
            }
        }
    }
}

fn add_preset(spec: &mut SkinSpec, layers: Vec<gf_core::skin_layers::SkinLayer>) {
    for mut l in layers {
        let base = l.name.clone();
        let mut name = base.clone();
        let mut k = 2;
        while spec.layers.iter().any(|x| x.name == name) {
            name = format!("{base}_{k}");
            k += 1;
        }
        l.name = name;
        spec.layers.push(l);
    }
}
