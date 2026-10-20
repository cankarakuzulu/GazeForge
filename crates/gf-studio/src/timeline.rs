use std::sync::mpsc::{Receiver, channel};
use std::time::Instant;

use eframe::egui;
use serde_json::{Value, json};

use crate::face_rig::{Rig, SHAPES};
use crate::i18n::{tr, trf};
use crate::world::World;

pub const SPEEDS: [f64; 4] = [0.25, 0.5, 1.0, 2.0];

pub struct Clip {
    pub name: Option<String>,
    pub names: Vec<String>,
    pub weights: Vec<Vec<f64>>,
    pub times: Vec<f64>,
    pub fps: f64,
    pub head: Option<Vec<[f64; 4]>>,
    pub experimental: bool,
    pub raw: Value,
}

pub fn parse_clip(c: &Value) -> Result<Clip, String> {
    if c["format"] != "gazeforge.clip/1" {
        return Err(tr("timeline.bad_format").into());
    }
    let (Some(names), Some(weights), Some(times)) = (c["names"].as_array(), c["weights"].as_array(), c["times"].as_array()) else { return Err(tr("timeline.bad_empty").into()) };
    if times.is_empty() {
        return Err(tr("timeline.bad_empty").into());
    }
    let names: Vec<String> = names.iter().map(|n| n.as_str().unwrap_or_default().to_string()).collect();
    let weights: Vec<Vec<f64>> = weights.iter().map(|r| r.as_array().into_iter().flatten().map(|x| x.as_f64().unwrap_or(0.0)).collect()).collect();
    if weights.len() != times.len() || weights.iter().any(|r| r.len() != names.len()) {
        return Err(tr("timeline.bad_shape").into());
    }
    let head: Option<Vec<[f64; 4]>> = c["head"]["rotation_xyzw"].as_array().map(|a| a.iter().map(|q| std::array::from_fn(|k| q[k].as_f64().unwrap_or(0.0))).collect());
    if head.as_ref().is_some_and(|h| h.len() != times.len()) {
        return Err(tr("timeline.bad_head").into());
    }
    Ok(Clip {
        name: c["name"].as_str().map(str::to_string),
        names,
        weights,
        times: times.iter().map(|t| t.as_f64().unwrap_or(0.0)).collect(),
        fps: c["fps"].as_f64().filter(|f| *f > 0.0).unwrap_or(30.0),
        head,
        experimental: c["experimental"] == true,
        raw: c.clone(),
    })
}

pub fn qmat(q: [f64; 4]) -> [f64; 9] {
    let [x, y, z, w] = q;
    [
        1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w),
        2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w),
        2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y),
    ]
}

pub fn to_designer(q: [f64; 4]) -> [f64; 4] {
    [q[0], -q[2], q[1], q[3]]
}

pub fn angles(q: [f64; 4]) -> [f64; 3] {
    let m = qmat(q);
    let n = [m[2], m[5], m[8]];
    let yaw = n[0].atan2(n[2]);
    let pitch = n[1].clamp(-1.0, 1.0).asin();
    let u = [m[1], m[4], m[7]];
    let (cy, sy) = ((-yaw).cos(), (-yaw).sin());
    let u1 = [cy * u[0] + sy * u[2], u[1], -sy * u[0] + cy * u[2]];
    let (cp, sp) = (pitch.cos(), pitch.sin());
    let u2 = [u1[0], cp * u1[1] - sp * u1[2], sp * u1[1] + cp * u1[2]];
    let d = 180.0 / std::f64::consts::PI;
    [yaw * d, pitch * d, u2[0].atan2(u2[1]) * d]
}

pub struct Timeline {
    pub clip: Option<Clip>,
    pub label: String,
    pub map: Vec<Option<usize>>,
    pub t: f64,
    pub playing: bool,
    pub looping: bool,
    pub speed: f64,
    pub apply_head: bool,
    pub last: Option<Instant>,
    pub rig: Option<Rig>,
    pub status: String,
    pub rig_note: String,
    pub readout: String,
    pub job: Option<Receiver<Result<Rig, String>>>,
    pub capture: Option<Receiver<Result<Value, String>>>,
    pub file: String,
    pub source: String,
    pub glb: String,
    pub out: String,
}

impl Default for Timeline {
    fn default() -> Timeline {
        Timeline {
            clip: None,
            label: String::new(),
            map: Vec::new(),
            t: 0.0,
            playing: false,
            looping: true,
            speed: 1.0,
            apply_head: true,
            last: None,
            rig: None,
            status: tr("timeline.no_clip").into(),
            rig_note: tr("timeline.deltas_none").into(),
            readout: String::new(),
            job: None,
            capture: None,
            file: String::new(),
            source: String::new(),
            glb: String::new(),
            out: String::new(),
        }
    }
}

impl Timeline {
    fn frame_at(&self, t: f64) -> (usize, usize, f64) {
        let c = self.clip.as_ref().expect("a clip");
        let n = c.times.len();
        let f = (t * c.fps).clamp(0.0, (n - 1) as f64);
        let i = (f.floor() as usize).min(n - 1);
        (i, (i + 1).min(n - 1), f - i as f64)
    }

    fn weights_at(&self, t: f64) -> Vec<f64> {
        let (i, j, a) = self.frame_at(t);
        let w = &self.clip.as_ref().expect("a clip").weights;
        w[i].iter().enumerate().map(|(k, x)| x + a * (w[j][k] - x)).collect()
    }

    fn head_at(&self, t: f64) -> Option<[f64; 4]> {
        let h = self.clip.as_ref()?.head.as_ref()?;
        let (i, j, a) = self.frame_at(t);
        let q0 = h[i];
        let mut q1 = h[j];
        if q0[0] * q1[0] + q0[1] * q1[1] + q0[2] * q1[2] + q0[3] * q1[3] < 0.0 {
            q1 = q1.map(|x| -x);
        }
        let q: [f64; 4] = std::array::from_fn(|k| q0[k] + a * (q1[k] - q0[k]));
        let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        Some(q.map(|x| x / n))
    }

    pub fn pivot(&self) -> [f64; 3] {
        self.rig.as_ref().and_then(|r| r.joints.iter().find(|j| j.name == "head")).map_or([0.0; 3], |j| j.position)
    }

    pub fn evaluate(&self, t: f64) -> (Vec<f32>, Option<[f64; 4]>, Vec<f64>) {
        let r = self.rig.as_ref().expect("deltas");
        let mut p = r.rest.clone();
        let w = self.weights_at(t);
        for (k, &v) in w.iter().enumerate() {
            let Some(s) = self.map.get(k).copied().flatten() else { continue };
            if v == 0.0 {
                continue;
            }
            for e in r.offsets[s] as usize..r.offsets[s + 1] as usize {
                let g = r.index[e] as usize * 3;
                for c in 0..3 {
                    p[g + c] = (p[g + c] as f64 + v * r.delta[3 * e + c] as f64) as f32;
                }
            }
        }
        let q = self.head_at(t);
        if let Some(q) = q.filter(|_| self.apply_head) {
            let (m, c) = (qmat(to_designer(q)), self.pivot());
            for g in (0..p.len()).step_by(3) {
                let (x, y, z) = (p[g] as f64 - c[0], p[g + 1] as f64 - c[1], p[g + 2] as f64 - c[2]);
                p[g] = (m[0] * x + m[1] * y + m[2] * z + c[0]) as f32;
                p[g + 1] = (m[3] * x + m[4] * y + m[5] * z + c[1]) as f32;
                p[g + 2] = (m[6] * x + m[7] * y + m[8] * z + c[2]) as f32;
            }
        }
        (p, q, w)
    }

    pub fn duration(&self) -> f64 {
        self.clip.as_ref().map_or(0.0, |c| (c.times.len() - 1) as f64 / c.fps)
    }
}

impl World {
    pub fn timeline_deltas(&mut self) {
        let Some(s) = &self.session else { return };
        if self.timeline.job.is_some() {
            return;
        }
        let mut input = s.settle_input(&s.character, false);
        let prefix = self.face_prefix();
        input["morphs"] = json!(SHAPES.iter().map(|n| format!("{prefix}{n}")).collect::<Vec<_>>());
        input["rounds"] = json!(0);
        if let Some(dir) = self.face_morphs() {
            input["morph_dir"] = json!(dir);
        }
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let env = gf_api::call(&gf_api::Ctx::from_env(), "rig.face", &input);
            let _ = tx.send(if env["ok"] == true { Rig::from_json(&env["result"]) } else { Err(env["error"]["message"].as_str().unwrap_or("rig.face failed").to_string()) });
        });
        self.timeline.job = Some(rx);
        self.timeline.rig_note = tr("timeline.deltas_loading").into();
    }

    pub fn timeline_clip(&mut self, clip: Value, label: &str) -> bool {
        match parse_clip(&clip) {
            Ok(c) => {
                self.timeline.playing = false;
                self.timeline.map = c.names.iter().map(|n| SHAPES.iter().position(|s| s == n)).collect();
                let unmapped = self.timeline.map.iter().filter(|m| m.is_none()).count();
                self.timeline.status = trf("timeline.clip", &[("name", &c.name.clone().unwrap_or_else(|| tr("timeline.clip_default").into())), ("label", &label), ("frames", &c.times.len()), ("fps", &c.fps), ("channels", &c.names.len())])
                    + &if unmapped > 0 { trf("timeline.frag.unmapped", &[("n", &unmapped)]) } else { String::new() }
                    + if c.head.is_some() { tr("timeline.frag.head_rotation") } else { "" }
                    + if c.experimental { tr("timeline.frag.experimental") } else { "" };
                self.timeline.clip = Some(c);
                self.timeline.label = label.to_string();
                self.timeline.t = 0.0;
                self.timeline_show();
                true
            }
            Err(e) => {
                self.timeline.status = trf("timeline.error", &[("error", &e)]);
                false
            }
        }
    }

    pub fn timeline_show(&mut self) {
        let tl = &self.timeline;
        let Some(c) = &tl.clip else {
            let _ = self.set_pose("timeline", None);
            self.eye_pose = None;
            self.set_overlay("timeline-head", None);
            return;
        };
        let (i, _, _) = tl.frame_at(tl.t);
        let mut text = trf("timeline.readout", &[("t", &format!("{:.2}", tl.t)), ("length", &format!("{:.2}", tl.duration())), ("frame", &(i + 1)), ("frames", &c.times.len())]);
        if tl.rig.is_none() {
            self.timeline.readout = text + tr("timeline.frag.need_deltas");
            return;
        }
        let (p, q, w) = tl.evaluate(tl.t);
        let pivot = tl.pivot();
        let mut overlay = None;
        if let Some(q) = q {
            let (a, m) = (angles(q), qmat(to_designer(q)));
            text += &trf("timeline.frag.head", &[("yaw", &format!("{:.1}", a[0])), ("pitch", &format!("{:.1}", a[1])), ("roll", &format!("{:.1}", a[2]))]);
            overlay = Some((vec![pivot, [pivot[0] - 0.15 * m[1], pivot[1] - 0.15 * m[4], pivot[2] - 0.15 * m[7]]], [0.2, 0.75, 1.0]));
        }
        let mut top: Vec<(f64, &str)> = w.iter().zip(&c.names).map(|(v, n)| (*v, n.as_str())).collect();
        top.sort_by(|x, y| y.0.total_cmp(&x.0));
        let top: Vec<String> = top.into_iter().take(3).filter(|x| x.0 > 0.05).map(|(v, n)| format!("{n} {v:.2}")).collect();
        if !top.is_empty() {
            text += &format!(", {}", top.join(", "));
        }
        self.eye_pose = q.filter(|_| self.timeline.apply_head).map(|q| (qmat(to_designer(q)), pivot));
        self.timeline.readout = text;
        let _ = self.set_pose("timeline", Some(p));
        self.set_overlay("timeline-head", overlay);
    }

    pub fn timeline_poll(&mut self) {
        if let Some(rx) = &self.timeline.job
            && let Ok(out) = rx.try_recv()
        {
            self.timeline.job = None;
            match out {
                Ok(r) => {
                    self.timeline.rig_note = trf("timeline.deltas_loaded", &[("shapes", &SHAPES.len()), ("vertices", &r.n)]);
                    self.timeline.rig = Some(r);
                    self.timeline_show();
                }
                Err(e) => self.timeline.rig_note = trf("timeline.deltas_error", &[("error", &e)]),
            }
        }
        if let Some(rx) = &self.timeline.capture
            && let Ok(out) = rx.try_recv()
        {
            self.timeline.capture = None;
            match out {
                Ok(v) => {
                    let label = v[1].as_str().unwrap_or_default().to_string();
                    self.timeline_clip(v[0].clone(), &label);
                }
                Err(e) => self.timeline.status = trf("timeline.error", &[("error", &e)]),
            }
        }
        if self.timeline.playing {
            let now = Instant::now();
            let dt = self.timeline.last.map_or(0.0, |l| (now - l).as_secs_f64());
            self.timeline.last = Some(now);
            let dur = self.timeline.duration();
            self.timeline.t += dt * self.timeline.speed;
            if self.timeline.t > dur {
                if self.timeline.looping && dur > 0.0 {
                    self.timeline.t %= dur;
                } else {
                    self.timeline.t = dur;
                    self.timeline.playing = false;
                }
            }
            self.timeline_show();
        }
    }

    pub fn timeline_play(&mut self, on: bool) {
        self.timeline.playing = on && self.timeline.clip.is_some();
        self.timeline.last = None;
    }

    pub fn timeline_capture(&mut self, csv: bool) {
        let tl = &self.timeline;
        let (path, out) = (tl.source.trim().to_string(), tl.out.trim().to_string());
        if path.is_empty() || out.is_empty() {
            self.timeline.status = tr("timeline.need_paths").into();
            return;
        }
        let mut input = json!({"out": out, "return_clip": true});
        if tl.glb.trim().is_empty() {
            input["morphs"] = json!(SHAPES.to_vec());
        } else {
            input["glb"] = json!(tl.glb.trim());
        }
        let ctx = crate::wizard::ctx().unwrap_or_else(|_| gf_api::Ctx::from_env());
        let (tx, rx) = channel();
        let image = |p: &str| { let l = p.to_ascii_lowercase(); [".png", ".jpg", ".jpeg", ".bmp", ".webp"].iter().any(|e| l.ends_with(e)) || !std::path::Path::new(p).extension().is_some_and(|e| (2..=4).contains(&e.len())) };
        if csv {
            input["csv"] = json!(path);
            self.timeline.status = tr("timeline.importing").into();
            std::thread::spawn(move || {
                let env = gf_api::call(&ctx, "animation.from_csv", &input);
                let _ = tx.send(if env["ok"] == true { Ok(json!([env["result"]["clip"], tr("timeline.label_csv")])) } else { Err(env["error"]["message"].as_str().unwrap_or("animation.from_csv failed").to_string()) });
            });
        } else {
            if image(&path) {
                input["frames"] = json!(path);
                input["fps"] = json!(30);
            } else {
                input["video"] = json!(path);
            }
            if let Ok(l) = std::env::var("GAZEFORGE_FACE_LANDMARKER") {
                input["landmarker"] = json!(l);
            }
            self.timeline.status = trf("timeline.capturing", &[("state", &tr("job.state.queued"))]);
            std::thread::spawn(move || {
                let sub = gf_api::call(&ctx, "jobs.submit", &json!({"command": "animation.from_video", "input": input}));
                let Some(id) = sub["result"]["id"].as_str().map(str::to_string) else {
                    let _ = tx.send(Err(sub["error"]["message"].as_str().unwrap_or("jobs.submit failed").to_string()));
                    return;
                };
                loop {
                    let r = gf_api::call(&ctx, "jobs.status", &json!({"id": id, "wait_s": 2}))["result"].clone();
                    match r["state"].as_str() {
                        Some("done") => {
                            let _ = tx.send(Ok(json!([r["result"]["result"]["clip"], tr("timeline.label_capture")])));
                            return;
                        }
                        Some("failed") | Some("cancelled") | None => {
                            let _ = tx.send(Err(r["result"]["error"]["message"].as_str().or(r["error"]["message"].as_str()).unwrap_or("the capture failed").to_string()));
                            return;
                        }
                        _ => {}
                    }
                }
            });
        }
        self.timeline.capture = Some(rx);
    }
}

impl crate::studio::Studio<'_> {
    pub fn timeline_panel(&mut self, ui: &mut egui::Ui) {
        let loaded = self.world.session.is_some();
        egui::CollapsingHeader::new(tr("panel.timeline")).id_salt("timeline").default_open(false).show(ui, |ui| {
            if !loaded {
                ui.label(egui::RichText::new(tr("studio.load_first")).weak());
                return;
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(self.world.timeline.job.is_none(), egui::Button::new(tr("timeline.load_deltas"))).clicked() {
                    self.world.timeline_deltas();
                }
                ui.label(egui::RichText::new(&self.world.timeline.rig_note).weak());
            });
            let mut load = false;
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.world.timeline.file).hint_text(tr("timeline.file_title")).desired_width(200.0));
                load = ui.button(tr("timeline.load_clip")).clicked();
            });
            if load {
                let path = self.world.timeline.file.trim().to_string();
                match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| gf_api::exactjson::text(&t).map_err(|e| e.to_string())) {
                    Ok(v) => {
                        let name = std::path::Path::new(&path).file_name().map_or(path.clone(), |f| f.to_string_lossy().to_string());
                        self.world.timeline_clip(v, &name);
                    }
                    Err(e) => self.world.timeline.status = trf("timeline.error", &[("error", &e)]),
                }
            }
            egui::CollapsingHeader::new(tr("timeline.capture")).id_salt("timeline_capture").show(ui, |ui| {
                let tl = &mut self.world.timeline;
                ui.add(egui::TextEdit::singleline(&mut tl.source).hint_text(tr("timeline.src_placeholder")).desired_width(f32::INFINITY));
                ui.add(egui::TextEdit::singleline(&mut tl.glb).hint_text(tr("timeline.glb_placeholder")).desired_width(f32::INFINITY));
                ui.add(egui::TextEdit::singleline(&mut tl.out).hint_text(tr("timeline.out_placeholder")).desired_width(f32::INFINITY));
                let busy = tl.capture.is_some();
                ui.horizontal(|ui| {
                    if ui.add_enabled(!busy, egui::Button::new(tr("timeline.capture"))).on_hover_text(tr("timeline.capture_title")).clicked() {
                        self.world.timeline_capture(false);
                    }
                    if ui.add_enabled(!busy, egui::Button::new(tr("timeline.csv"))).on_hover_text(tr("timeline.csv_title")).clicked() {
                        self.world.timeline_capture(true);
                    }
                    if busy {
                        ui.spinner();
                    }
                });
            });
            let has_clip = self.world.timeline.clip.is_some();
            let mut show = false;
            ui.horizontal(|ui| {
                let playing = self.world.timeline.playing;
                if ui.add_enabled(has_clip, egui::Button::new(if playing { tr("timeline.pause") } else { tr("timeline.play") })).clicked() {
                    self.world.timeline_play(!playing);
                }
                let tl = &mut self.world.timeline;
                ui.checkbox(&mut tl.looping, tr("timeline.loop"));
                egui::ComboBox::from_id_salt("timeline_speed").width(60.0).selected_text(format!("{}x", tl.speed)).show_ui(ui, |ui| {
                    for s in SPEEDS {
                        ui.selectable_value(&mut tl.speed, s, format!("{s}x"));
                    }
                }).response.on_hover_text(tr("timeline.speed"));
                show |= ui.checkbox(&mut tl.apply_head, tr("timeline.head")).on_hover_text(tr("timeline.head_title")).changed();
            });
            if let Some(c) = &self.world.timeline.clip {
                let n = c.times.len();
                let fps = c.fps;
                let (mut frame, _, _) = self.world.timeline.frame_at(self.world.timeline.t);
                ui.spacing_mut().slider_width = ui.available_width();
                if ui.add(egui::Slider::new(&mut frame, 0..=n - 1).show_value(false)).changed() {
                    self.world.timeline_play(false);
                    self.world.timeline.t = frame as f64 / fps;
                    show = true;
                }
            }
            if show {
                self.world.timeline_show();
            }
            ui.label(egui::RichText::new(&self.world.timeline.readout).weak());
            ui.label(&self.world.timeline.status);
            if self.world.timeline.playing {
                ui.ctx().request_repaint();
            }
        });
    }
}
