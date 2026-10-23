use eframe::egui;

use crate::browse::{self, Pick};
use crate::face_rig::{GROUPS, MANUAL, Mode, PRESETS, SHAPES, group_of, preset_weights};
use crate::i18n::{tr, trf};
use crate::studio::Studio;
use crate::viewer;

impl Studio<'_> {
    pub fn face_rig_panel(&mut self, ui: &mut egui::Ui) {
        let loaded = self.world.session.is_some();
        egui::CollapsingHeader::new(tr("panel.rig")).id_salt("face_rig").default_open(false).show(ui, |ui| {
            if !loaded {
                ui.label(egui::RichText::new(tr("studio.load_first")).weak());
                return;
            }
            let busy = self.world.face.job.is_some();
            ui.horizontal(|ui| {
                ui.label(tr("rig.shapes_dir")).on_hover_text(tr("rig.shapes_dir_tip"));
                let mut dir = self.world.face_morphs.clone().unwrap_or_default();
                let hint = std::env::var("GAZEFORGE_FACE_MORPHS").unwrap_or_else(|_| tr("rig.shapes_dir_hint").to_string());
                if browse::field(ui, &mut self.s.browse, "rig.shapes", &mut dir, &hint, Pick::Folder) {
                    self.world.face_morphs = Some(dir.trim().to_string()).filter(|d| !d.is_empty());
                }
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy, egui::Button::new(tr("rig.load"))).clicked() {
                    self.world.face_load();
                }
                if busy {
                    ui.spinner();
                }
            });
            ui.label(egui::RichText::new(&self.world.face.status).weak());
            if self.world.face.rig.is_none() {
                return;
            }
            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label(tr("rig.mode"));
                let mut mode = self.world.face.mode;
                egui::ComboBox::from_id_salt("rig_mode").selected_text(tr(&format!("rig.mode.{}", mode.name()))).show_ui(ui, |ui| {
                    for m in [Mode::Blend, Mode::Joints] {
                        ui.selectable_value(&mut mode, m, tr(&format!("rig.mode.{}", m.name())));
                    }
                }).response.on_hover_text(tr("rig.mode_title"));
                if mode != self.world.face.mode {
                    self.world.face.mode = mode;
                    changed = true;
                }
                ui.checkbox(&mut self.world.face.overlay, tr("rig.overlay_title"));
            });
            ui.horizontal_wrapped(|ui| {
                for (name, _) in PRESETS {
                    if ui.small_button(tr(&format!("rig.preset.{name}"))).clicked()
                        && let Some(w) = preset_weights(name)
                    {
                        self.world.face.weights = w;
                        changed = true;
                    }
                }
            });
            for (group, _) in GROUPS {
                egui::CollapsingHeader::new(tr(&format!("rig.group.{group}"))).id_salt(("rig_group", group)).show(ui, |ui| {
                    for (i, shape) in SHAPES.iter().enumerate().filter(|(_, s)| group_of(s) == group) {
                        let fit = self.world.face.rig.as_ref().and_then(|r| r.fit.get(i).cloned()).unwrap_or_default();
                        let poor = fit["poorly_expressed"] == true;
                        let label = if poor { format!("{shape} *") } else { shape.to_string() };
                        let tip = trf("rig.fit_title", &[("p95", &format!("{:.2}", fit["p95_mm"].as_f64().unwrap_or(0.0))), ("max", &format!("{:.2}", fit["max_mm"].as_f64().unwrap_or(0.0))),
                                                          ("pct", &format!("{:.1}", 100.0 * fit["explained"].as_f64().unwrap_or(0.0)))]);
                        let w = &mut self.world.face.weights[i];
                        if ui.add(egui::Slider::new(w, 0.0..=1.0).text(label).fixed_decimals(2)).on_hover_text(tip).changed() {
                            changed = true;
                        }
                    }
                });
            }
            egui::CollapsingHeader::new(tr("rig.joint_pose")).id_salt("rig_joint").show(ui, |ui| {
                let joints: Vec<(usize, String)> = self.world.face.rig.as_ref().map(|r| r.joints.iter().enumerate().filter(|(_, j)| j.posed).map(|(k, j)| (k, j.name.clone())).collect()).unwrap_or_default();
                let mut joint = self.world.face.joint;
                let shown = joints.iter().find(|(k, _)| *k == joint).map_or(String::new(), |j| j.1.clone());
                egui::ComboBox::from_id_salt("rig_joint_pick").selected_text(shown).show_ui(ui, |ui| {
                    for (k, name) in &joints {
                        ui.selectable_value(&mut joint, *k, name);
                    }
                });
                self.world.face.joint = joint;
                let mut m = self.world.face.manual.get(&joint).copied().unwrap_or([0.0; 6]);
                let before = m;
                for (c, (key, lim)) in MANUAL.iter().enumerate() {
                    ui.add(egui::Slider::new(&mut m[c], -lim..=*lim).text(tr(&format!("rig.manual.{key}"))).step_by(if c < 3 { 0.5 } else { 0.1 }));
                }
                if m != before {
                    self.world.face.manual.insert(joint, m);
                    changed = true;
                }
                if ui.button(tr("rig.reset_joint")).clicked() && self.world.face.manual.remove(&joint).is_some() {
                    changed = true;
                }
            });
            if changed {
                self.world.face_update();
            }
        });
    }

    pub fn face_rig_paint(&self, painter: &egui::Painter, rect: egui::Rect, ppp: f32, w: u32, h: u32) {
        let f = &self.world.face;
        let (Some(rig), true) = (&f.rig, f.overlay && !f.overlay_off) else { return };
        let (points, lines) = rig.joint_overlay(f.mode, &f.weights, &f.manual);
        let colour = egui::Color32::from_rgb(255, 140, 38);
        for r in viewer::rects(self.world.state.layout, w, h) {
            let m = self.world.state.cams[r.cam].matrices(r.w as f64 / r.h.max(1) as f64).mvp;
            let to = |p: [f64; 3]| {
                let c = |row: usize| m[row] as f64 * p[0] + m[4 + row] as f64 * p[1] + m[8 + row] as f64 * p[2] + m[12 + row] as f64;
                let wc = c(3);
                if wc.abs() < 1e-9 {
                    return None;
                }
                let (x, y) = (c(0) / wc, c(1) / wc);
                Some(rect.min + egui::vec2((r.x as f64 + (x + 1.0) / 2.0 * r.w as f64) as f32 / ppp, (r.y as f64 + (1.0 - y) / 2.0 * r.h as f64) as f32 / ppp))
            };
            let clip = egui::Rect::from_min_size(rect.min + egui::vec2(r.x as f32 / ppp, r.y as f32 / ppp), egui::vec2(r.w as f32 / ppp, r.h as f32 / ppp));
            let p = painter.with_clip_rect(clip);
            for [a, b] in &lines {
                if let (Some(a), Some(b)) = (to(*a), to(*b)) {
                    p.line_segment([a, b], egui::Stroke::new(1.5, colour));
                }
            }
            for q in &points {
                if let Some(q) = to(*q) {
                    p.circle_filled(q, 2.5, colour);
                }
            }
        }
    }
}
