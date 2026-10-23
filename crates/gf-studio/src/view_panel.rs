use eframe::egui;

use crate::i18n::{tr, trf};
use crate::viewer::{MAX_LIGHTS, light_preset};

const PRESETS: [&str; 5] = ["studio", "flat", "rim", "top", "key"];

pub fn preset_of(o: &crate::viewer::Opts) -> Option<&'static str> {
    PRESETS.iter().copied().find(|p| {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-6 * (1.0 + b.abs());
        light_preset(p).is_some_and(|(a, g, l)| {
            near(a, o.ambient)
                && near(g, o.ground)
                && o.lights.iter().zip(l.iter()).all(|(x, y)| x.on && near(x.az, y.az) && near(x.el, y.el) && near(x.i, y.i) && near(x.k, y.k))
                && o.lights.iter().skip(l.len()).all(|x| !x.on)
        })
    })
}

fn slider(ui: &mut egui::Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, step: f64) -> bool {
    ui.add(egui::Slider::new(v, range).step_by(step).text(label)).changed()
}

fn mm_slider(ui: &mut egui::Ui, label: &str, v: &mut f64, range: std::ops::RangeInclusive<f64>, step: f64) -> bool {
    ui.add(egui::Slider::new(v, range).step_by(step).custom_formatter(|x, _| format!("{:.1} mm", x * 1000.0)).text(label)).changed()
}

#[derive(Clone, PartialEq)]
enum Item {
    Head,
    Eyes,
    Extra(String),
    Overlay(String),
}

impl crate::world::World {
    fn items(&self) -> Vec<(Item, String, bool)> {
        let mut out = vec![(Item::Head, tr("mesh.head").to_string(), !self.state.head_hidden)];
        if self.session.as_ref().is_some_and(|s| s.head.eyes.as_ref().is_some_and(|e| !e.indices.is_empty())) {
            out.push((Item::Eyes, tr("mesh.eyes").to_string(), self.state.opts.eyes));
        }
        out.extend(self.extras.iter().map(|e| (Item::Extra(e.id.clone()), e.id.clone(), e.visible)));
        out.extend(self.overlays.iter().map(|o| (Item::Overlay(o.id.clone()), trf("designer.overlay_item", &[("id", &o.id)]), !self.hidden_overlays.contains(&o.id))));
        out
    }

    fn set_visible(&mut self, item: &Item, on: bool) {
        match item {
            Item::Head => self.state.head_hidden = !on,
            Item::Eyes => self.state.opts.eyes = on,
            Item::Extra(id) => {
                for e in self.extras.iter_mut().filter(|e| e.id == *id) {
                    e.visible = on;
                }
            }
            Item::Overlay(id) => {
                if on {
                    self.hidden_overlays.remove(id);
                } else {
                    self.hidden_overlays.insert(id.clone());
                }
            }
        }
    }

    pub fn isolate(&mut self, keep: &str) {
        for (item, _, _) in self.items() {
            let id = match &item {
                Item::Head => "head",
                Item::Eyes => "eyes",
                Item::Extra(id) | Item::Overlay(id) => id.as_str(),
            };
            let on = id == keep;
            self.set_visible(&item, on);
        }
    }

    pub fn show_all(&mut self) {
        for (item, _, _) in self.items() {
            self.set_visible(&item, true);
        }
        self.state.hidden_regions = 0;
    }

    pub fn set_visible_id(&mut self, id: &str, on: bool) -> bool {
        let Some((item, _, _)) = self.items().into_iter().find(|(item, _, _)| match item {
            Item::Head => id == "head",
            Item::Eyes => id == "eyes",
            Item::Extra(x) | Item::Overlay(x) => x == id,
        }) else {
            return false;
        };
        self.set_visible(&item, on);
        true
    }
}

impl crate::studio::Studio<'_> {
    fn visibility_ui(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(tr("ui.visibility")).id_salt("view_visibility").default_open(false).show(ui, |ui| {
            let mut change: Option<(Item, bool)> = None;
            let mut isolate: Option<String> = None;
            for (item, label, on) in self.world.items() {
                ui.horizontal(|ui| {
                    let mut v = on;
                    if ui.checkbox(&mut v, &label).changed() {
                        change = Some((item.clone(), v));
                    }
                    if ui.small_button(tr("designer.isolate")).clicked() {
                        isolate = Some(match &item {
                            Item::Head => "head".to_string(),
                            Item::Eyes => "eyes".to_string(),
                            Item::Extra(id) | Item::Overlay(id) => id.clone(),
                        });
                    }
                });
            }
            if ui.button(tr("ui.show-all")).on_hover_text(tr("ui.show-all.title")).clicked() {
                self.world.show_all();
            }
            if let Some((item, on)) = change {
                self.world.set_visible(&item, on);
            }
            if let Some(id) = isolate {
                self.world.isolate(&id);
            }
        });
        egui::CollapsingHeader::new(tr("ui.regions")).id_salt("view_regions").default_open(false).show(ui, |ui| {
            self.world.grid_wanted = true;
            ui.label(egui::RichText::new(tr("ui.each_vertex_belongs_to_the_category_of_t")).weak());
            let Some(g) = self.world.grid.clone() else {
                ui.label(egui::RichText::new(tr("studio.grid_loading")).weak());
                return;
            };
            let mut counts = vec![0usize; g.region_names.len()];
            for &r in &g.regions {
                if let Some(c) = counts.get_mut(r as usize) {
                    *c += 1;
                }
            }
            for (i, name) in g.region_names.iter().enumerate().take(24) {
                let mut shown = (self.world.state.hidden_regions >> i) & 1 == 0;
                let label = format!("{}  ({})", crate::i18n::tr_or(&format!("category.{name}"), name), counts[i]);
                if ui.checkbox(&mut shown, label).changed() {
                    if shown {
                        self.world.state.hidden_regions &= !(1 << i);
                    } else {
                        self.world.state.hidden_regions |= 1 << i;
                    }
                }
            }
        });
    }

    pub fn view_panel(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new(tr("studio.view_section")).id_salt("view_section").default_open(false).show(ui, |ui| {
            ui.spacing_mut().slider_width = 90.0;
            self.visibility_ui(ui);
            let current_ref = self.world.ref_source;
            let mut ref_choice: Option<&'static str> = None;
            let st = &mut self.world.state;
            egui::CollapsingHeader::new(tr("ui.render")).id_salt("view_render").default_open(false).show(ui, |ui| {
                let o = &mut st.opts;
                slider(ui, tr("ui.wire_width"), &mut o.wire_w, 0.5..=4.0, 0.25);
                ui.checkbox(&mut o.wire_fade, tr("ui.fade_the_wire_where_triangles_are_only_a"));
                slider(ui, tr("ui.point_size"), &mut o.point_size, 1.0..=10.0, 1.0);
                ui.horizontal(|ui| {
                    ui.label(tr("ui.anisotropic_tangents"));
                    egui::ComboBox::from_id_salt("aniso").selected_text(tr(&format!("ui.option.{}", o.aniso as i64))).show_ui(ui, |ui| {
                        for v in [0.0, 1.0] {
                            ui.selectable_value(&mut o.aniso, v, tr(&format!("ui.option.{}", v as i64)));
                        }
                    });
                });
                slider(ui, tr("ui.quality_range"), &mut o.heat.quality, 5.0..=60.0, 1.0);
                slider(ui, tr("ui.displacement_range"), &mut o.heat.displacement, 0.1..=20.0, 0.1);
                slider(ui, tr("ui.curvature_range"), &mut o.heat.curvature, 5.0..=300.0, 1.0);
                ui.horizontal(|ui| {
                    ui.label(tr("ui.displacement_reference"));
                    egui::ComboBox::from_id_salt("ref_source").selected_text(tr(&format!("ui.option.{current_ref}"))).show_ui(ui, |ui| {
                        for s in ["start", "neutral", "snapshot"] {
                            if ui.selectable_label(current_ref == s, tr(&format!("ui.option.{s}"))).clicked() {
                                ref_choice = Some(s);
                            }
                        }
                    });
                });
                if ui.button(tr("ui.ref-now")).clicked() {
                    ref_choice = Some("snapshot");
                }
                ui.label(egui::RichText::new(tr("ui.quality_colours_the_smallest_angle_of_ea")).weak());
            });
            egui::CollapsingHeader::new(tr("ui.camera_active_viewport")).id_salt("view_camera").default_open(false).show(ui, |ui| {
                let mut fov = st.active_cam().goal.fov;
                if ui.add(egui::Slider::new(&mut fov, 10.0..=90.0).step_by(1.0).suffix("°").text(tr("ui.field_of_view"))).changed() {
                    st.set_fov(fov, false);
                }
            });
            egui::CollapsingHeader::new(tr("ui.lighting")).id_salt("view_lighting").default_open(false).show(ui, |ui| {
                let o = &mut st.opts;
                let current = preset_of(o);
                ui.horizontal(|ui| {
                    ui.label(tr("ui.preset"));
                    egui::ComboBox::from_id_salt("light_preset").selected_text(tr(&format!("preset.{}", current.unwrap_or("custom")))).show_ui(ui, |ui| {
                        for p in PRESETS {
                            if ui.selectable_label(current == Some(p), tr(&format!("preset.{p}"))).clicked() {
                                o.apply_preset(p);
                            }
                        }
                    });
                });
                ui.checkbox(&mut o.follow, tr("ui.lights_follow_the_camera"));
                ui.checkbox(&mut o.light_gizmo, tr("ui.light_gizmo"));
                slider(ui, tr("ui.exposure_ev"), &mut o.exposure, -3.0..=3.0, 0.05);
                slider(ui, tr("ui.ambient"), &mut o.ambient, 0.0..=1.5, 0.01);
                slider(ui, tr("ui.ground_share"), &mut o.ground, 0.0..=1.0, 0.01);
                for i in 0..MAX_LIGHTS.min(o.lights.len()) {
                    let l = &mut o.lights[i];
                    ui.checkbox(&mut l.on, trf("designer.light_n", &[("n", &(i + 1))]));
                    if l.on {
                        ui.indent(("light", i), |ui| {
                            slider(ui, tr("designer.azimuth"), &mut l.az, -180.0..=180.0, 1.0);
                            slider(ui, tr("designer.elevation"), &mut l.el, -90.0..=90.0, 1.0);
                            slider(ui, tr("designer.intensity"), &mut l.i, 0.0..=3.0, 0.01);
                            slider(ui, tr("designer.colour_k"), &mut l.k, 2000.0..=12000.0, 100.0);
                        });
                    }
                }
            });
            egui::CollapsingHeader::new(tr("ui.hide_box")).id_salt("view_box").default_open(false).show(ui, |ui| {
                let b = &mut st.opts.clip_box;
                ui.checkbox(&mut b.on, tr("ui.hide_everything_inside_the_box"));
                for (k, (from, to)) in [("ui.x_from", "ui.x_to"), ("ui.y_from", "ui.y_to"), ("ui.z_from", "ui.z_to")].into_iter().enumerate() {
                    slider(ui, tr(from), &mut b.lo[k], 0.0..=1.0, 0.01);
                    slider(ui, tr(to), &mut b.hi[k], 0.0..=1.0, 0.01);
                }
            });
            egui::CollapsingHeader::new(tr("ui.clipping")).id_salt("view_clipping").default_open(false).show(ui, |ui| {
                let o = &mut st.opts;
                ui.checkbox(&mut o.caps, tr("ui.fill_the_section_caps"));
                ui.checkbox(&mut o.plane_gizmo, tr("ui.plane_gizmo"));
                for (i, c) in o.clip.iter_mut().enumerate() {
                    ui.checkbox(&mut c.on, trf("designer.plane_n", &[("n", &(i + 1))]));
                    if !c.on {
                        continue;
                    }
                    ui.indent(("clip", i), |ui| {
                        ui.horizontal(|ui| {
                            ui.label(tr("designer.axis"));
                            egui::ComboBox::from_id_salt(("clip_axis", i)).selected_text(tr(&format!("designer.axis_{}", c.axis))).show_ui(ui, |ui| {
                                for a in ["x", "y", "z", "free"] {
                                    ui.selectable_value(&mut c.axis, a.to_string(), tr(&format!("designer.axis_{a}")));
                                }
                            });
                        });
                        mm_slider(ui, tr("designer.offset"), &mut c.offset, -0.15..=0.15, 0.0005);
                        if c.axis == "free" {
                            slider(ui, tr("designer.azimuth_free"), &mut c.az, -180.0..=180.0, 1.0);
                            slider(ui, tr("designer.elevation_free"), &mut c.el, -90.0..=90.0, 1.0);
                        }
                        ui.checkbox(&mut c.flip, tr("designer.keep_other_side"));
                        ui.checkbox(&mut c.slab, tr("designer.slab"));
                        if c.slab {
                            mm_slider(ui, tr("designer.slab"), &mut c.thick, 0.0005..=0.05, 0.0005);
                        }
                    });
                }
                ui.label(egui::RichText::new(tr("ui.inside_a_cut_back_faces_are_orange_and_t")).weak());
            });
            egui::CollapsingHeader::new(tr("designer.opt.grid")).id_salt("view_grid").default_open(false).show(ui, |ui| {
                let o = &mut st.opts;
                ui.checkbox(&mut o.face_grid, tr("action.face_grid"));
                ui.checkbox(&mut o.grid_labels, tr("studio.grid_labels"));
                ui.horizontal(|ui| {
                    ui.label(tr("ui.eyes"));
                    egui::ComboBox::from_id_salt("eyes_grid").selected_text(tr(&format!("ui.option.{}", o.eyes_grid))).show_ui(ui, |ui| {
                        for m in ["grid", "hide", "plain"] {
                            ui.selectable_value(&mut o.eyes_grid, m.to_string(), tr(&format!("ui.option.{m}")));
                        }
                    });
                })
                .response
                .on_hover_text(tr("ui.the_eye_spheres_while_the_face_grid_is_o.title"));
                ui.checkbox(&mut o.normal_grid, tr("action.mm_grid"));
            });
            if let Some(s) = ref_choice {
                self.world.set_reference(s);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_lights_read_as_the_studio_preset_and_each_preset_reads_as_itself() {
        let mut o = crate::viewer::Opts::default();
        assert_eq!(preset_of(&o), Some("studio"));
        for p in PRESETS {
            o.apply_preset(p);
            assert_eq!(preset_of(&o), Some(p));
        }
        o.apply_preset("studio");
        o.lights[1].i = (o.lights[1].i / 0.01).round() * 0.01;
        assert_eq!(preset_of(&o), Some("studio"), "a value snapped to a slider step still reads as the preset");
        o.lights[0].az += 1.0;
        assert_eq!(preset_of(&o), None);
    }
}
