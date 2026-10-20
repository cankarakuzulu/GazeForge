use std::sync::mpsc::{Receiver, channel};

use serde_json::{Value, json};

use crate::i18n::{tr, trf};
use crate::world::World;

pub const CONTROLS: [(&str, f64, f64, f64, f64); 6] = [
    ("blend_mm", 0.0, 300.0, 5.0, 60.0),
    ("neck_offset_mm", -60.0, 60.0, 1.0, 0.0),
    ("head_scale", 0.8, 1.25, 0.01, 1.0),
    ("body_scale", 0.8, 1.25, 0.01, 1.0),
    ("smooth", 0.0, 20.0, 1.0, 0.0),
    ("tangent", 0.0, 1.0, 0.05, 0.5),
];
const BODY: [f32; 3] = [0.78, 0.74, 0.70];
const FAIL: [f32; 3] = [0.86, 0.22, 0.18];

pub struct StitchMesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
    pub lines: Vec<[f64; 3]>,
    pub head: Option<Vec<f32>>,
    pub pass: bool,
}

pub struct Stitch {
    pub store: String,
    pub body: String,
    pub values: [f64; 6],
    pub show: bool,
    pub seam: bool,
    pub auto: bool,
    pub status: String,
    pub pass: Option<bool>,
    pub last: Option<StitchMesh>,
    pub stale: bool,
    pub pending: bool,
    pub wanted: bool,
    pub job: Option<Receiver<Result<(Value, bool), String>>>,
}

impl Default for Stitch {
    fn default() -> Stitch {
        Stitch {
            store: String::new(),
            body: String::new(),
            values: CONTROLS.map(|c| c.4),
            show: true,
            seam: true,
            auto: false,
            status: tr("stitch.not_stitched").into(),
            pass: None,
            last: None,
            stale: false,
            pending: false,
            wanted: false,
            job: None,
        }
    }
}

fn floats(v: &Value, what: &str) -> Result<Vec<f32>, String> {
    let b = gf_api::designer::from_base64(v.as_str().ok_or_else(|| format!("character.stitch returned no {what}"))?)?;
    Ok(b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

fn mesh(r: &Value, pass: bool) -> Result<StitchMesh, String> {
    let m = &r["mesh"];
    let idx = gf_api::designer::from_base64(m["indices"].as_str().ok_or("character.stitch returned no indices")?)?;
    let lines = floats(&m["seam_lines"], "seam lines")?.chunks_exact(3).map(|c| [c[0] as f64, c[1] as f64, c[2] as f64]).collect();
    let scale = m["head_scale"].as_f64().unwrap_or(1.0);
    Ok(StitchMesh {
        positions: floats(&m["positions"], "positions")?,
        normals: floats(&m["normals"], "normals")?,
        indices: idx.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
        lines,
        head: if scale != 1.0 { Some(floats(&m["head_positions"], "head positions")?) } else { None },
        pass,
    })
}

pub fn report(r: &Value, pass: bool) -> String {
    let g = &r["gates"];
    let n = &g["normals"]["seam_dihedral_deg"];
    let mut lines = vec![
        format!("{} - {}", if pass { tr("stitch.pass") } else { tr("stitch.fail") },
                trf("stitch.body_line", &[("mode", &r["body"]["mode"].as_str().unwrap_or("")), ("vertices", &r["body"]["vertices"]), ("seams", &r["body"]["seam_points"])])),
        trf("stitch.gap_line", &[("mm", &format!("{:.4}", g["watertight"]["max_gap_m"].as_f64().unwrap_or(0.0) * 1000.0)), ("open", &g["topology"]["open_seam_edges"])]),
        trf("stitch.angle_line", &[("max", &format!("{:.1}", n["max"].as_f64().unwrap_or(0.0))), ("mean", &format!("{:.1}", n["mean"].as_f64().unwrap_or(0.0)))]),
        trf("stitch.band_line", &[("flips", &g["band"]["flips"]), ("folds", &g["band"]["folded_edges"]), ("body", &g["interpenetration"]["body_vertices_inside_head"]),
                                   ("head", &g["interpenetration"]["head_vertices_inside_body"])]),
    ];
    lines.extend(g["failed"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|f| format!("- {f}")));
    lines.join("\n")
}

impl crate::studio::Studio<'_> {
    pub fn stitch_panel(&mut self, ui: &mut eframe::egui::Ui) {
        use eframe::egui;
        let loaded = self.world.session.is_some();
        egui::CollapsingHeader::new(tr("panel.stitch")).id_salt("stitch").default_open(false).show(ui, |ui| {
            if !loaded {
                ui.label(egui::RichText::new(tr("studio.load_first")).weak());
                return;
            }
            let st = &mut self.world.stitch;
            egui::Grid::new("stitch_paths").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label(tr("stitch.store"));
                ui.add(egui::TextEdit::singleline(&mut st.store).hint_text(tr("stitch.store_placeholder")).desired_width(f32::INFINITY));
                ui.end_row();
                ui.label(tr("stitch.body_pack"));
                ui.add(egui::TextEdit::singleline(&mut st.body).hint_text(tr("stitch.body_placeholder")).desired_width(f32::INFINITY));
                ui.end_row();
            });
            let mut restitch = false;
            for (i, (id, lo, hi, step, _)) in CONTROLS.iter().enumerate() {
                let r = ui.add(egui::Slider::new(&mut st.values[i], *lo..=*hi).step_by(*step).text(tr(&format!("stitch.control.{id}"))));
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    restitch = true;
                }
            }
            let mut redraw = false;
            redraw |= ui.checkbox(&mut st.show, tr("stitch.show_body")).changed();
            redraw |= ui.checkbox(&mut st.seam, tr("stitch.seam_overlay")).changed();
            ui.checkbox(&mut st.auto, tr("stitch.auto"));
            let busy = st.job.is_some();
            ui.horizontal(|ui| {
                if ui.button(tr("stitch.go")).clicked() {
                    restitch = true;
                }
                if busy {
                    ui.spinner();
                }
            });
            let colour = match st.pass {
                Some(true) => ui.visuals().text_color(),
                Some(false) => ui.visuals().error_fg_color,
                None => ui.visuals().weak_text_color(),
            };
            ui.colored_label(colour, &st.status);
            if restitch {
                self.world.stitch_go();
            } else if redraw {
                self.world.stitch_draw();
            }
        });
    }
}

impl World {
    pub fn stitch_go(&mut self) {
        let Some(s) = &self.session else {
            self.stitch.status = tr("studio.load_first").into();
            return;
        };
        let store = if self.stitch.store.trim().is_empty() { s.store.clone() } else { self.stitch.store.trim().to_string() };
        if self.stitch.body.trim().is_empty() {
            self.stitch.status = tr("stitch.need_body").into();
            self.stitch.pass = None;
            return;
        }
        if self.stitch.job.is_some() {
            self.stitch.pending = true;
            return;
        }
        let character = s.settle_input(&s.character, false);
        let mut input = json!({"store": store, "character": character, "body": self.stitch.body.trim(), "return_mesh": true});
        if let Some(m) = character.get("macro") {
            input["macro"] = m.clone();
        }
        for (i, c) in CONTROLS.iter().enumerate() {
            input[c.0] = json!(self.stitch.values[i]);
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let env = gf_api::call(&gf_api::Ctx::from_env(), "character.stitch", &input);
            let out = if env["ok"] == true {
                Ok((env["result"].clone(), true))
            } else if env["error"]["code"] == "gate_failed" && env["error"]["details"]["mesh"].is_object() {
                Ok((env["error"]["details"].clone(), false))
            } else {
                Err(env["error"]["message"].as_str().unwrap_or("character.stitch failed").to_string())
            };
            let _ = tx.send(out);
        });
        self.stitch.job = Some(rx);
        self.stitch.wanted = true;
        self.stitch.status = tr("stitch.stitching").into();
        self.stitch.pass = None;
    }

    pub fn stitch_poll(&mut self) {
        let Some(rx) = &self.stitch.job else { return };
        let Ok(out) = rx.try_recv() else { return };
        self.stitch.job = None;
        match out.and_then(|(r, pass)| mesh(&r, pass).map(|m| (r, m))) {
            Ok((r, m)) => {
                self.stitch.status = report(&r, m.pass);
                self.stitch.pass = Some(m.pass);
                self.stitch.last = Some(m);
                self.stitch.stale = false;
            }
            Err(e) => {
                self.stitch.last = None;
                self.stitch.status = trf("stitch.failed", &[("error", &e)]);
                self.stitch.pass = Some(false);
            }
        }
        self.stitch_draw();
        if std::mem::take(&mut self.stitch.pending) {
            self.stitch_go();
        }
    }

    pub fn stitch_draw(&mut self) {
        let visible = self.stitch.show && !self.stitch.stale;
        let (body, seam, head) = match &self.stitch.last {
            Some(m) => (
                visible.then(|| (m.positions.clone(), m.normals.clone(), m.indices.clone(), if m.pass { BODY } else { FAIL })),
                (self.stitch.seam && !self.stitch.stale).then(|| (m.lines.clone(), if m.pass { [1.0, 0.35, 0.2] } else { [1.0, 0.0, 0.0] })),
                if visible { m.head.clone() } else { None },
            ),
            None => (None, None, None),
        };
        self.set_extra("stitch-body", body);
        self.set_overlay("stitch-seam", seam);
        let _ = self.set_pose("stitch-head", head);
    }

    pub fn stitch_changed(&mut self, slider: bool) {
        if self.stitch.last.is_none() {
            return;
        }
        if slider {
            if !self.stitch.stale {
                self.stitch.stale = true;
                self.stitch.status = if self.stitch.auto { tr("stitch.changed_auto") } else { tr("stitch.changed") }.into();
                self.stitch_draw();
            }
        } else if self.stitch.auto && self.stitch.show {
            self.stitch_go();
        } else if !self.stitch.stale {
            self.stitch.stale = true;
            self.stitch.status = tr("stitch.changed").into();
            self.stitch_draw();
        }
    }
}
