use serde_json::{Value, json};

use crate::i18n::{tr, trf};
use crate::probe::{describe, point_of};
use crate::session::Head;
use crate::viewer::V3;

fn round(x: f64, d: usize) -> f64 {
    format!("{x:.d$}").parse().unwrap_or(x)
}

pub fn valid(p: &Value) -> bool {
    p["n"].is_u64() && p["tri"].is_u64() && p["bary"].as_array().is_some_and(|b| b.len() == 3 && b.iter().all(|x| x.as_f64().is_some_and(f64::is_finite)))
}

pub fn read(state: &Value) -> (Vec<Value>, u64) {
    let pins: Vec<Value> = state["pins"].as_array().into_iter().flatten().filter(|p| valid(p)).cloned().collect();
    let next = state["next"].as_u64().unwrap_or_else(|| pins.iter().filter_map(|p| p["n"].as_u64()).map(|n| n + 1).max().unwrap_or(1));
    (pins, next)
}

fn tri_bary(p: &Value) -> (usize, [f64; 3]) {
    let b = &p["bary"];
    (p["tri"].as_u64().unwrap_or(0) as usize, [0, 1, 2].map(|k| b[k].as_f64().unwrap_or(0.0)))
}

pub fn normal_of(head: &Head, tri: usize) -> Option<V3> {
    let t = head.lods.first()?.indices.get(tri * 3..tri * 3 + 3)?;
    let q = |v: u32| [head.positions[v as usize * 3] as f64, head.positions[v as usize * 3 + 1] as f64, head.positions[v as usize * 3 + 2] as f64];
    let (a, b, c) = (q(t[0]), q(t[1]), q(t[2]));
    let n = crate::viewer::cross(crate::viewer::sub(b, a), crate::viewer::sub(c, a));
    let l = crate::viewer::dot(n, n).sqrt().max(1e-30);
    Some(n.map(|x| x / l * head.wind as f64))
}

impl crate::world::World {
    pub fn pins_state(&self) -> Value {
        json!({"pins": self.pins, "next": self.pins_next})
    }

    pub fn pins_add(&mut self) -> Result<String, String> {
        let Some(probe) = self.probe.clone().filter(|p| p.lod0.is_some()) else { return Err(tr("pins.probe_first").to_string()) };
        let g = self.grid.clone().ok_or_else(|| tr("studio.grid_loading").to_string())?;
        let (tri, bary, err) = probe.lod0.expect("filtered above");
        let tag = self.char_tag();
        let s = self.session.as_ref().ok_or_else(|| tr("studio.load_first").to_string())?;
        let d = describe(&g, &s.head, 0, tri, bary, None, &tag).ok_or_else(|| tr("designer.no_surface").to_string())?;
        let n = self.pins_next;
        let mut pin = json!({"n": n, "tri": tri, "bary": bary.map(|x| round(x, 6)), "dense": d.dense, "game": d.game, "note": ""});
        if probe.lod > 0 {
            pin["from_lod"] = json!(probe.lod);
            pin["lod0_error_mm"] = json!(round(err, 4));
        }
        self.pins.push(pin);
        self.pins_next = n + 1;
        let mut msg = trf("pins.added", &[("n", &n), ("address", &d.address)]);
        if probe.lod > 0 {
            msg += &trf("pins.added_lod", &[("level", &probe.lod), ("mm", &format!("{err:.3}"))]);
        }
        Ok(msg)
    }

    pub fn pins_delete(&mut self, n: u64) {
        self.pins.retain(|p| p["n"].as_u64() != Some(n));
    }

    pub fn pins_note(&mut self, n: u64, text: &str) {
        if let Some(p) = self.pins.iter_mut().find(|p| p["n"].as_u64() == Some(n)) {
            p["note"] = json!(text);
        }
    }

    pub fn pin_describe(&self, p: &Value) -> Option<crate::probe::Probe> {
        let (g, s) = (self.grid.as_ref()?, self.session.as_ref()?);
        let (tri, bary) = tri_bary(p);
        describe(g, &s.head, 0, tri, bary, None, &self.char_tag())
    }

    pub fn pins_export(&self) -> Result<Value, String> {
        let g = self.grid.as_ref().ok_or_else(|| tr("studio.grid_loading").to_string())?;
        let d = &g.definition;
        let lost: Vec<String> = self.pins.iter().filter(|p| self.pin_describe(p).is_none()).map(|p| p["n"].to_string()).collect();
        if !lost.is_empty() {
            return Err(trf("pins.export_unplaced", &[("list", &lost.join(", "))]));
        }
        let pins = self
            .pins
            .iter()
            .filter_map(|p| {
                let q = self.pin_describe(p)?;
                let mut o = json!({"n": p["n"], "address": q.address, "cell": q.cell, "region": q.region, "game": q.game, "dense": q.dense,
                                   "lod0_triangle": p["tri"], "bary": p["bary"], "position_mm": q.point.map(|x| round(x * 1000.0, 2)), "note": p["note"].as_str().unwrap_or("")});
                if p["from_lod"].is_u64() {
                    o["placed_on_lod"] = p["from_lod"].clone();
                    o["lod0_error_mm"] = p["lod0_error_mm"].clone();
                }
                Some(o)
            })
            .collect::<Vec<_>>();
        Ok(json!({"format": "gazeforge.pins/1", "character": self.char_tag(),
                  "chart": {"column_deg": d["column_deg"], "row_mm": d["row_mm"], "letters": d["letters"], "nose_tip_dense": d["nose_tip_dense"], "top_mm": d["top_mm"]},
                  "pins": pins}))
    }

    pub fn pin_markers(&self) -> Vec<(V3, V3, String)> {
        let Some(s) = &self.session else { return Vec::new() };
        self.pins
            .iter()
            .filter_map(|p| {
                let (tri, bary) = tri_bary(p);
                Some((point_of(&s.head, 0, tri, bary)?, normal_of(&s.head, tri)?, p["n"].to_string()))
            })
            .collect()
    }
}

impl crate::studio::Studio<'_> {
    pub fn pins_panel(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui;
        egui::CollapsingHeader::new(tr("panel.pins")).id_salt("pins").default_open(false).show(ui, |ui| {
            if self.world.session.is_none() {
                ui.label(egui::RichText::new(tr("studio.load_first")).weak());
                return;
            }
            if !self.world.pins.is_empty() {
                self.world.grid_wanted = true;
            }
            ui.label(egui::RichText::new(tr("pins.note")).weak());
            ui.horizontal(|ui| {
                if ui.button(tr("pins.add")).clicked() {
                    self.s.status = self.world.pins_add().unwrap_or_else(|e| e);
                }
                let picked = self.s.browse.take("pins_export");
                crate::browse::Browse::button(&mut self.s.browse, ui, "pins_export", crate::browse::Pick::Save(".json".into()), "gazeforge-pins.json");
                if ui.button(tr("pins.export")).clicked() || picked.is_some() {
                    let path = picked.unwrap_or_else(|| crate::studio::placed("pins", "gazeforge-pins.json").display().to_string());
                    self.s.status = match self.world.pins_export().and_then(|doc| {
                        std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap_or_default()).map_err(|e| format!("{path}: {e}")).map(|_| doc["pins"].as_array().map_or(0, Vec::len))
                    }) {
                        Ok(n) => format!("{} ({path})", trf("pins.exported", &[("n", &n)])),
                        Err(e) => e,
                    };
                }
            });
            if self.world.pins.is_empty() {
                ui.label(egui::RichText::new(tr("pins.none")).weak());
            }
            let mut go = None;
            let mut delete = None;
            let mut notes = Vec::new();
            for p in self.world.pins.clone() {
                let n = p["n"].as_u64().unwrap_or(0);
                let d = self.world.pin_describe(&p);
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(n.to_string()).strong());
                    let text = d.as_ref().map_or_else(|| "…".to_string(), |d| format!("{} · {}", d.cell, d.region));
                    let l = ui.label(text);
                    if let Some(d) = &d {
                        l.on_hover_text(&d.address);
                    }
                    if ui.small_button(tr("pins.go")).clicked() {
                        go = Some(d.as_ref().map_or_else(|| format!("game v{}", p["game"]), |d| d.address.clone()));
                    }
                    if crate::icons::button(ui, "x", false, tr("pins.delete")).clicked() {
                        delete = Some(n);
                    }
                });
                let mut note = p["note"].as_str().unwrap_or_default().to_string();
                if ui.add(egui::TextEdit::singleline(&mut note).hint_text(tr("pins.note_placeholder")).desired_width(f32::INFINITY)).changed() {
                    notes.push((n, note));
                }
            }
            for (n, note) in notes {
                self.world.pins_note(n, &note);
            }
            if let Some(n) = delete {
                self.world.pins_delete(n);
            }
            if let Some(t) = go {
                self.s.status = match self.world.go_to(&t) {
                    Ok(g) => trf("designer.go_to", &[("what", &g.what), ("warn", &g.warn)]),
                    Err(e) => e,
                };
            }
        });
    }

    pub fn pins_paint(&mut self, painter: &eframe::egui::Painter, rect: eframe::egui::Rect, ppp: f32, w: u32, h: u32) {
        use eframe::egui;
        let marks = self.world.pin_markers();
        let Some(s) = &self.world.session else { return };
        if marks.is_empty() {
            return;
        }
        let st = &self.world.state;
        for r in crate::viewer::rects(st.layout, w, h) {
            let cam = &st.cams[r.cam];
            let m = cam.matrices(r.w as f64 / r.h.max(1) as f64);
            let shot = crate::grid_labels::Shot { m: &m, x: r.x as f64 / ppp as f64, y: r.y as f64 / ppp as f64, w: r.w as f64 / ppp as f64, h: r.h as f64 / ppp as f64, dist: cam.cur.dist };
            let clip = egui::Rect::from_min_size(rect.min + egui::vec2(shot.x as f32, shot.y as f32), egui::vec2(shot.w as f32, shot.h as f32));
            let p = painter.with_clip_rect(clip);
            for (pos, nrm, text) in &marks {
                let Some((x, y)) = crate::grid_labels::project(&shot, *pos) else { continue };
                if crate::grid_labels::facing(&m, *pos, *nrm) < 0.0 {
                    continue;
                }
                let (px, py) = ((x - shot.x) * ppp as f64, (y - shot.y) * ppp as f64);
                let eye_dist = crate::viewer::dot(crate::viewer::sub(*pos, m.eye), m.f);
                if let Some(hit) = crate::render::pick_triangle(st, &s.head, &r, px, py, None)
                    && dot_along(&m, hit.point) < eye_dist - 0.002
                {
                    continue;
                }
                let galley = p.layout_no_wrap(text.clone(), egui::FontId::proportional(11.0), egui::Color32::WHITE);
                let at = rect.min + egui::vec2(x as f32 + 6.0, y as f32 - 18.0);
                p.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(5.0, 2.0)), 9.0, egui::Color32::from_rgb(0xd8, 0x1b, 0x60));
                p.galley(at, galley, egui::Color32::WHITE);
                p.circle_filled(rect.min + egui::vec2(x as f32, y as f32), 2.5, egui::Color32::from_rgb(0xd8, 0x1b, 0x60));
            }
        }
    }
}

fn dot_along(m: &crate::viewer::Matrices, p: V3) -> f64 {
    crate::viewer::dot(crate::viewer::sub(p, m.eye), m.f)
}
