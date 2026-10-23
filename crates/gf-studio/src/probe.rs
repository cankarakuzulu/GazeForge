use serde_json::{Value, json};

use crate::face_grid::FaceGrid;
use crate::i18n::{tr, trf};
use crate::session::Head;
use crate::viewer::{Bounds, V3};
use gf_geom::face_chart as fc;

#[derive(Clone, Debug)]
pub struct Probe {
    pub address: String,
    pub cell: String,
    pub vcell: String,
    pub region: String,
    pub game: usize,
    pub dense: u32,
    pub lod: usize,
    pub triangle: usize,
    pub bary: [f64; 3],
    pub point: V3,
    pub lod0: Option<(usize, [f64; 3], f64)>,
}

fn at(p: &[f32], v: u32) -> V3 {
    let i = v as usize * 3;
    [p[i] as f64, p[i + 1] as f64, p[i + 2] as f64]
}

pub fn char_tag(name: &str, revision: Option<&str>) -> String {
    let name = if name.is_empty() { "character" } else { name };
    match revision.filter(|r| !r.is_empty()) {
        Some(r) => format!("{name} {}", r.replacen("sha256:", "", 1).chars().take(12).collect::<String>()),
        None => name.to_string(),
    }
}

pub fn point_of(head: &Head, lod: usize, tri: usize, bary: [f64; 3]) -> Option<V3> {
    let idx = &head.lods.get(lod)?.indices;
    let t = idx.get(tri * 3..tri * 3 + 3)?;
    let mut out = [0.0; 3];
    for k in 0..3 {
        let q = at(&head.positions, t[k]);
        for i in 0..3 {
            out[i] += bary[k] * q[i];
        }
    }
    Some(out)
}

pub fn describe(g: &FaceGrid, head: &Head, lod: usize, tri: usize, bary: [f64; 3], point: Option<V3>, tag: &str) -> Option<Probe> {
    let idx = &head.lods.get(lod)?.indices;
    let t = idx.get(tri * 3..tri * 3 + 3)?;
    let corners = [t[0], t[1], t[2]];
    let p = match point {
        Some(p) => p,
        None => point_of(head, lod, tri, bary)?,
    };
    let gap = corners.map(|c| {
        let q = at(&head.positions, c);
        ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt()
    });
    let min = gap.iter().cloned().fold(f64::INFINITY, f64::min);
    let game = corners[gap.iter().position(|&d| d == min).unwrap_or(0)] as usize;
    let uv = |v: u32| [g.uv[v as usize * 2] as f64, g.uv[v as usize * 2 + 1] as f64];
    let cell = g.cell_name(fc::pack(fc::point_cell(corners.map(uv), bary)));
    let vcell = g.cell_name(*g.cells.get(game)?);
    let region = g.regions.get(game).and_then(|&r| g.region_names.get(r as usize)).cloned().unwrap_or_else(|| "?".into());
    let dense = *g.dense.get(game)?;
    let place = if vcell == cell { cell.clone() } else { format!("{cell} (v in {vcell})") };
    let mm = p.map(|x| format!("{:.1}", x * 1000.0)).join(",");
    let address = format!("GF@ {place} | region {region} | game v{game} = dense {dense} | LOD{lod} t{tri} b({:.3},{:.3}) | {mm} mm | {tag}", bary[1], bary[2]);
    Some(Probe { address, cell, vcell, region, game, dense, lod, triangle: tri, bary, point: p, lod0: None })
}

fn closest_on_triangle(p: V3, a: V3, b: V3, c: V3) -> [f64; 3] {
    let sub3 = |x: V3, y: V3| [x[0] - y[0], x[1] - y[1], x[2] - y[2]];
    let dot3 = |x: V3, y: V3| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
    let (ab, ac, ap) = (sub3(b, a), sub3(c, a), sub3(p, a));
    let (d1, d2) = (dot3(ab, ap), dot3(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let bp = sub3(p, b);
    let (d3, d4) = (dot3(ab, bp), dot3(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return [0.0, 1.0, 0.0];
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [1.0 - v, v, 0.0];
    }
    let cp = sub3(p, c);
    let (d5, d6) = (dot3(ab, cp), dot3(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return [0.0, 0.0, 1.0];
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return [1.0 - w, 0.0, w];
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / (d4 - d3 + (d5 - d6));
        return [0.0, 1.0 - w, w];
    }
    let den = 1.0 / (va + vb + vc);
    let (v, w) = (vb * den, vc * den);
    [1.0 - v - w, v, w]
}

pub fn to_lod0(head: &Head, p: V3) -> Option<(usize, [f64; 3], f64)> {
    let idx = &head.lods.first()?.indices;
    let pos = &head.positions;
    let mut best: Option<(usize, [f64; 3])> = None;
    let mut bd2 = f64::INFINITY;
    for (t, c) in idx.chunks_exact(3).enumerate() {
        let (a, b, cc) = (at(pos, c[0]), at(pos, c[1]), at(pos, c[2]));
        let mut out2 = 0.0;
        for k in 0..3 {
            let (lo, hi) = (a[k].min(b[k]).min(cc[k]), a[k].max(b[k]).max(cc[k]));
            let d = if p[k] < lo { lo - p[k] } else if p[k] > hi { p[k] - hi } else { 0.0 };
            out2 += d * d;
        }
        if out2 >= bd2 {
            continue;
        }
        let w = closest_on_triangle(p, a, b, cc);
        let q = [0, 1, 2].map(|k| w[0] * a[k] + w[1] * b[k] + w[2] * cc[k]);
        let d2 = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2);
        if d2 < bd2 {
            bd2 = d2;
            best = Some((t, w));
        }
    }
    best.map(|(t, w)| (t, w, bd2.sqrt() * 1000.0))
}

fn lod_address(text: &str) -> Option<(usize, usize, f64, f64)> {
    let i = text.find("LOD")?;
    let rest = &text[i + 3..];
    let (lod, rest) = rest.split_once(" t")?;
    let (tri, rest) = rest.split_once(" b(")?;
    let (b1, rest) = rest.split_once(',')?;
    let (b2, _) = rest.split_once(')')?;
    Some((lod.parse().ok()?, tri.parse().ok()?, b1.parse().ok()?, b2.parse().ok()?))
}

pub struct GoTo {
    pub target: V3,
    pub radius: f64,
    pub what: String,
    pub warn: String,
}

impl crate::world::World {
    fn grid_call(&self, address: &str, accept: bool) -> Result<Value, String> {
        let s = self.session.as_ref().ok_or_else(|| tr("studio.load_first").to_string())?;
        let mut input = json!({"store": s.store, "pack": s.pack, "catalog": s.catalog, "game": s.game, "address": address});
        if accept {
            input["accept_inconsistent"] = json!(true);
        }
        let env = gf_api::call(&gf_api::Ctx::from_env(), "grid.resolve", &input);
        if env["ok"] != true {
            return Err(env["error"]["message"].as_str().unwrap_or("grid.resolve failed").to_string());
        }
        Ok(env["result"].clone())
    }

    pub fn occluder(&self, r: &crate::viewer::Rect, px: f64, py: f64, head_at: f64) -> Option<String> {
        let st = &self.state;
        let near = |pos: &[f32], idx: &[u32]| crate::render::ray_distance(st, r, px, py, pos, idx).is_some_and(|d| d < head_at - 1e-6);
        let eyes_shown = st.opts.eyes && !(st.opts.face_grid && st.opts.eyes_grid == "hide");
        if let Some(e) = self.session.as_ref().and_then(|s| s.head.eyes.as_ref()).filter(|e| eyes_shown && !e.indices.is_empty())
            && near(&e.positions, &e.indices)
        {
            return Some(tr("mesh.eyes").to_string());
        }
        self.extras.iter().filter(|e| e.visible).find(|e| near(&e.positions, &e.indices)).map(|e| self.extra_label(&e.id))
    }

    pub fn char_tag(&self) -> String {
        let Some(s) = &self.session else { return char_tag("", None) };
        char_tag(&s.character.name, s.settled.as_ref().map(|x| x.revision.as_str()))
    }

    pub fn probe_hit(&mut self, hit: &crate::render::Hit) -> Result<Option<Probe>, String> {
        self.grid_wanted = true;
        let Some(g) = self.grid.clone() else {
            if let Some(e) = &self.grid_error {
                return Err(trf("designer.err.grid", &[("error", e)]));
            }
            self.probe_waiting = Some(hit.clone());
            return Ok(None);
        };
        let tag = self.char_tag();
        let s = self.session.as_ref().ok_or_else(|| tr("studio.load_first").to_string())?;
        let mut p = describe(&g, &s.head, hit.lod, hit.index, hit.bary, Some(hit.point), &tag).ok_or_else(|| tr("designer.no_surface").to_string())?;
        p.lod0 = if hit.lod == 0 { Some((hit.index, hit.bary, 0.0)) } else { to_lod0(&s.head, hit.point) };
        self.flash = Some((p.point, std::time::Instant::now() + std::time::Duration::from_millis(900)));
        self.probe = Some(p.clone());
        Ok(Some(p))
    }

    pub fn probe_finish(&mut self) {
        if self.grid.is_some()
            && let Some(hit) = self.probe_waiting.take()
            && let Err(e) = self.probe_hit(&hit)
        {
            self.note(trf("designer.err.probe", &[("error", &e)]));
        }
    }

    pub fn go_to(&mut self, text: &str) -> Result<GoTo, String> {
        let t = text.trim();
        if t.is_empty() {
            return Err(tr("studio.goto_empty").to_string());
        }
        let mut warn = String::new();
        if t.starts_with("GF@") {
            let r = self.grid_call(t, true)?;
            let w: Vec<&str> = r["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            if !w.is_empty() {
                warn = trf("designer.frag.goto_warning", &[("warnings", &w.join("; "))]);
            }
        }
        let s = self.session.as_ref().ok_or_else(|| tr("studio.load_first").to_string())?;
        let head = &s.head;
        let lod_hit = lod_address(t).filter(|&(l, tri, _, _)| l < head.lods.len() && tri < head.lods[l].indices.len() / 3);
        let go = if let Some((l, tri, b1, b2)) = lod_hit {
            let target = point_of(head, l, tri, [1.0 - b1 - b2, b1, b2]).ok_or_else(|| tr("designer.no_surface").to_string())?;
            GoTo { target, radius: 0.03, what: format!("LOD{l} t{tri}"), warn }
        } else {
            let r = self.grid_call(t, false)?;
            let cell = r["cell"].as_str().unwrap_or_default().to_string();
            let Some(g) = r["nearest_game"].as_u64().or_else(|| r["game"].as_u64()) else {
                let note = r["note"].as_str().map_or_else(|| tr("designer.goto_no_vertex").to_string(), str::to_string);
                return Err(trf("designer.goto_none", &[("address", &t), ("note", &note), ("cell", &cell)]));
            };
            let target = at(&head.positions, g as u32);
            if r["kind"] == "vertex" {
                GoTo { target, radius: 0.03, what: format!("game v{g} = dense {} ({cell})", r["dense"]), warn }
            } else {
                let ext = r["game_vertices"].as_array().into_iter().flatten().filter_map(Value::as_u64).fold(0.0f64, |e, v| {
                    let q = at(&head.positions, v as u32);
                    e.max(((q[0] - target[0]).powi(2) + (q[1] - target[1]).powi(2) + (q[2] - target[2]).powi(2)).sqrt())
                });
                let what = trf("designer.goto_cell", &[("cell", &cell), ("count", &r["game_count"]), ("game", &g)]);
                GoTo { target, radius: (ext * 1.4).max(0.02), what, warn }
            }
        };
        let b = Bounds { lo: go.target, hi: go.target, centre: go.target, radius: go.radius };
        crate::viewer::frame_box(&b, self.state.active_cam(), false);
        self.flash = Some((go.target, std::time::Instant::now() + std::time::Duration::from_millis(2500)));
        Ok(go)
    }
}

impl crate::studio::Studio<'_> {
    pub fn probe_input(&mut self, resp: &eframe::egui::Response, alt: bool, rect: eframe::egui::Rect, ppp: f32, rects: &[crate::viewer::Rect]) {
        let at = |p: eframe::egui::Pos2| {
            let (x, y) = ((p.x - rect.min.x) * ppp, (p.y - rect.min.y) * ppp);
            let r = rects.iter().copied().find(|r| x >= r.x as f32 && x < (r.x + r.w) as f32 && y >= r.y as f32 && y < (r.y + r.h) as f32).unwrap_or(rects[0]);
            (r, (x - r.x as f32) as f64, (y - r.y as f32) as f64)
        };
        if resp.clicked()
            && !resp.double_clicked()
            && (alt || self.world.probe_mode)
            && let Some(p) = resp.interact_pointer_pos()
        {
            self.world.grid_wanted = true;
            let (r, px, py) = at(p);
            let hit = self.world.session.as_ref().and_then(|s| crate::render::pick_triangle(&self.world.state, &s.head, &r, px, py, None));
            let blocked = self.world.occluder(&r, px, py, hit.as_ref().map_or(f64::INFINITY, |h| h.distance));
            self.s.status = match (hit, blocked) {
                (None, Some(mesh)) => trf("designer.probe_not_head", &[("mesh", &mesh)]),
                (None, _) => tr("designer.no_surface").to_string(),
                (Some(_), Some(mesh)) => trf("designer.probe_not_head", &[("mesh", &mesh)]),
                (Some(h), None) => match self.world.probe_hit(&h) {
                    Ok(Some(_)) => String::new(),
                    Ok(None) => tr("studio.grid_loading").to_string(),
                    Err(e) => trf("designer.err.probe", &[("error", &e)]),
                },
            };
        }
        let want = self.world.state.opts.face_grid && self.world.grid.is_some();
        let Some(p) = resp.hover_pos().filter(|_| want) else {
            self.world.hover = None;
            return;
        };
        if self.s.hover_at.is_some_and(|t| t.elapsed().as_millis() < 50) {
            resp.ctx.request_repaint_after(std::time::Duration::from_millis(60));
            return;
        }
        self.s.hover_at = Some(std::time::Instant::now());
        let (r, px, py) = at(p);
        let tag = self.world.char_tag();
        self.world.hover = match (&self.world.session, &self.world.grid) {
            (Some(s), Some(g)) => crate::render::pick_triangle(&self.world.state, &s.head, &r, px, py, None)
                .filter(|h| self.world.occluder(&r, px, py, h.distance).is_none())
                .and_then(|h| describe(g, &s.head, h.lod, h.index, h.bary, Some(h.point), &tag))
                .map(|q| (q.point, r.slot, if q.vcell == q.cell { q.cell } else { format!("{} (v in {})", q.cell, q.vcell) })),
            _ => None,
        };
    }

    pub fn flash_paint(&mut self, ctx: &eframe::egui::Context, painter: &eframe::egui::Painter, rect: eframe::egui::Rect, ppp: f32, w: u32, h: u32) {
        use eframe::egui;
        let Some((p, until)) = self.world.flash else { return };
        let left = until.saturating_duration_since(std::time::Instant::now()).as_secs_f32();
        if left <= 0.0 {
            self.world.flash = None;
            return;
        }
        let st = &self.world.state;
        for r in crate::viewer::rects(st.layout, w, h) {
            let cam = &st.cams[r.cam];
            let m = cam.matrices(r.w as f64 / r.h.max(1) as f64);
            let shot = crate::grid_labels::Shot { m: &m, x: r.x as f64 / ppp as f64, y: r.y as f64 / ppp as f64, w: r.w as f64 / ppp as f64, h: r.h as f64 / ppp as f64, dist: cam.cur.dist };
            if let Some((x, y)) = crate::grid_labels::project(&shot, p) {
                let c = rect.min + egui::vec2(x as f32, y as f32);
                let a = (left.min(0.6) / 0.6 * 255.0) as u8;
                painter.circle_stroke(c, 9.0, egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(255, 200, 40, a)));
                painter.circle_filled(c, 2.5, egui::Color32::from_rgba_unmultiplied(255, 200, 40, a));
            }
        }
        ctx.request_repaint();
    }

    pub fn probe_card(&mut self, ctx: &eframe::egui::Context, rect: eframe::egui::Rect) {
        use eframe::egui;
        if !self.world.state.opts.face_grid && !self.world.probe_mode && self.world.probe.is_none() {
            return;
        }
        let mut go = false;
        let mut close = false;
        let mut pin = false;
        let shown = egui::Area::new(egui::Id::new("probe_card")).order(egui::Order::Foreground).pivot(egui::Align2::LEFT_BOTTOM).fixed_pos(rect.left_bottom() + egui::vec2(8.0, -8.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_max_width((rect.width() / 2.0 - 160.0).clamp(240.0, 620.0));
                if let Some(p) = &self.world.probe {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(egui::RichText::new(&p.address).monospace().size(11.5)).wrap().selectable(true));
                    });
                    if let Some((_, _, mm)) = p.lod0.filter(|_| p.lod > 0) {
                        ui.label(egui::RichText::new(trf("designer.pin_lod_note", &[("mm", &format!("{mm:.3}")), ("level", &p.lod)])).weak().size(11.0));
                    }
                    ui.horizontal(|ui| {
                        if ui.button(tr("ui.addr-copy")).on_hover_text(tr("ui.addr-copy.title")).clicked() {
                            ui.ctx().copy_text(p.address.clone());
                            self.s.status = tr("designer.copied").into();
                        }
                        if ui.button(tr("ui.addr-pin")).on_hover_text(tr("ui.addr-pin.title")).clicked() {
                            pin = true;
                        }
                        if crate::icons::button(ui, "x", false, tr("ui.addr-close.title")).clicked() {
                            close = true;
                        }
                    });
                    ui.separator();
                }
                ui.horizontal(|ui| {
                    let edit = ui.add(egui::TextEdit::singleline(&mut self.s.goto_text).hint_text(tr("ui.goto.placeholder")).desired_width(320.0).font(egui::TextStyle::Monospace));
                    go = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    go |= ui.button(tr("ui.goto-btn")).clicked();
                });
            });
        });
        self.s.overlay_next.push(shown.response.rect);
        if close {
            self.world.probe = None;
        }
        if pin {
            self.s.status = self.world.pins_add().unwrap_or_else(|e| e);
        }
        if go {
            self.world.grid_wanted = true;
            let text = self.s.goto_text.clone();
            self.s.status = match self.world.go_to(&text) {
                Ok(g) => trf("designer.go_to", &[("what", &g.what), ("warn", &g.warn)]),
                Err(e) => e,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lod_address_parses_out_of_a_whole_line() {
        let line = "GF@ M14.5 | region nose | game v123 = dense 456 | LOD2 t3301 b(0.250,0.125) | 1.0,2.0,3.0 mm | bare 0123456789ab";
        assert_eq!(lod_address(line), Some((2, 3301, 0.25, 0.125)));
        assert_eq!(lod_address("M12.3"), None);
        assert_eq!(char_tag("", Some("sha256:0123456789abcdef")), "character 0123456789ab");
        assert_eq!(char_tag("bare", None), "bare");
    }

    #[test]
    fn closest_point_on_a_triangle_lands_on_the_face_edge_or_corner() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let near = |got: [f64; 3], want: [f64; 3]| got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-12);
        assert!(near(closest_on_triangle([0.2, 0.2, 1.0], a, b, c), [0.6, 0.2, 0.2]));
        assert!(near(closest_on_triangle([-1.0, -1.0, 0.0], a, b, c), [1.0, 0.0, 0.0]));
        assert!(near(closest_on_triangle([0.5, -1.0, 0.0], a, b, c), [0.5, 0.5, 0.0]));
    }
}
