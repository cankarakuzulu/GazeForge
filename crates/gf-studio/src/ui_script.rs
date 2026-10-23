use std::path::Path;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;
use serde_json::{Value, json};

use crate::cube::{Cube, Element};
use crate::keymap;
use crate::viewer::{CamState, Camera, HOME, NAMED_VIEWS, V3};
use crate::render::read_texture;
use crate::script::{Offscreen, write_png};
use crate::studio::{self, CubeSpot, Gfx, Studio, UiArgs, UiState};

pub struct OffUi {
    ctx: egui::Context,
    egui: egui::mutex::RwLock<egui_wgpu::Renderer>,
    pub state: UiState,
    time: f64,
    modifiers: egui::Modifiers,
    pointer: egui::Pos2,
}

const DT: f64 = 1.0 / 60.0;

impl Offscreen {
    fn ui_frame(&mut self, events: Vec<egui::Event>) -> Result<egui::FullOutput, String> {
        let (w, h) = (self.width, self.height);
        let Offscreen { gpu, renderer, world, godot, ui, .. } = self;
        let u = ui.as_mut().ok_or("no interface: run a ui step first")?;
        u.time += DT;
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32, h as f32))),
            time: Some(u.time),
            predicted_dt: DT as f32,
            modifiers: u.modifiers,
            events,
            focused: true,
            ..Default::default()
        };
        let mut st = Studio { s: &mut u.state, world, renderer, godot, gfx: Gfx { device: &gpu.device, queue: &gpu.queue, egui: &u.egui }, spin: false, keep_painting: false };
        let mut out = u.ctx.run_ui(raw, |ui| st.frame(ui, DT));
        let delta = std::mem::take(&mut out.textures_delta);
        let mut er = u.egui.write();
        for (id, d) in &delta.set {
            er.update_texture(&gpu.device, &gpu.queue, *id, d);
        }
        for id in &delta.free {
            er.free_texture(id);
        }
        Ok(out)
    }

    fn ui_paint(&mut self, out: egui::FullOutput) -> Result<Vec<u8>, String> {
        let (w, h) = (self.width, self.height);
        let u = self.ui.as_mut().ok_or("no interface")?;
        let (device, queue) = (&self.gpu.device, &self.gpu.queue);
        let ppp = out.pixels_per_point;
        let prims = u.ctx.tessellate(out.shapes, ppp);
        let mut er = u.egui.write();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let sd = egui_wgpu::ScreenDescriptor { size_in_pixels: [w, h], pixels_per_point: ppp };
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("ui") });
        let extra = er.update_buffers(device, queue, &mut enc, &prims, &sd);
        {
            let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            let mut pass = pass.forget_lifetime();
            er.render(&mut pass, &prims, &sd);
        }
        queue.submit(extra.into_iter().chain([enc.finish()]));
        drop(er);
        read_texture(device, queue, &texture, 0, 0, w, h)
    }

    fn ui_frames(&mut self, n: usize) -> Result<Option<egui::FullOutput>, String> {
        let mut last = None;
        for _ in 0..n {
            let ev = vec![egui::Event::PointerMoved(self.ui.as_ref().map_or(egui::Pos2::ZERO, |u| u.pointer))];
            last = Some(self.ui_frame(ev)?);
        }
        Ok(last)
    }

    fn pointer(&mut self, kind: &str, at: egui::Pos2) -> Result<(), String> {
        let modifiers = self.ui.as_ref().map_or(egui::Modifiers::NONE, |u| u.modifiers);
        if let Some(u) = self.ui.as_mut() {
            u.pointer = at;
        }
        let ev = match kind {
            "move" => egui::Event::PointerMoved(at),
            "down" | "up" => egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: kind == "down", modifiers },
            _ => return Err(format!("unknown pointer event {kind}")),
        };
        self.ui_frame(vec![ev])?;
        Ok(())
    }

    pub fn ui_click(&mut self, at: egui::Pos2) -> Result<(), String> {
        self.pointer("move", at)?;
        self.pointer("move", at)?;
        self.pointer("down", at)?;
        self.pointer("up", at)?;
        self.ui_frames(1)?;
        Ok(())
    }

    pub fn ui_step(&mut self, op: &str, step: &Value, out: &Path) -> Result<Value, String> {
        let num = |k: &str| step.get(k).and_then(Value::as_f64);
        let text = |k: &str| step.get(k).and_then(Value::as_str);
        match op {
            "ui" => {
                if self.ui.is_none() {
                    let ctx = egui::Context::default();
                    studio::setup_context(&ctx)?;
                    let state = UiState::new(UiArgs { keymap: text("keymap").map(Into::into), input: None, store: text("store").map(Into::into), spec: text("spec").map(Into::into), macro_file: text("macro").map(Into::into), eyes: text("eyes").map(Into::into), godot_config: None })?;
                    let options = egui_wgpu::RendererOptions { dithering: false, ..Default::default() };
                    let er = egui_wgpu::Renderer::new(&self.gpu.device, wgpu::TextureFormat::Rgba8Unorm, options);
                    self.ui = Some(OffUi { ctx, egui: egui::mutex::RwLock::new(er), state, time: 0.0, modifiers: egui::Modifiers::NONE, pointer: egui::pos2(-10.0, -10.0) });
                }
                let u = self.ui.as_mut().expect("made above");
                if let Some(t) = text("theme") {
                    u.ctx.set_theme(if t == "light" { egui::ThemePreference::Light } else { egui::ThemePreference::Dark });
                }
                self.ui_frames(2)?;
                Ok(json!({"interface": true, "size": [self.width, self.height]}))
            }
            "ui_grid_labels" => {
                let u = self.ui.as_ref().ok_or("ui_grid_labels needs the ui op first")?;
                let all = u.state.grid_labels.as_ref().map(|(_, a)| a.iter().map(|l| l.iter().map(|x| json!({"text": x.text, "x": x.x, "y": x.y, "edge": x.edge})).collect::<Vec<_>>()).collect::<Vec<_>>());
                Ok(json!({"viewports": all, "rested": u.state.grid_view.as_ref().map(|v| v.1.elapsed().as_millis() as u64),
                          "viewport": u.state.viewport_rect.map(|r| [r.min.x, r.min.y, r.width(), r.height()])}))
            }
            "ui_frames" => {
                let n = num("count").unwrap_or(1.0).max(1.0) as usize;
                let pause = std::time::Duration::from_secs_f64(num("ms").unwrap_or(0.0).max(0.0) / 1000.0 / n as f64);
                for _ in 0..n {
                    std::thread::sleep(pause);
                    self.ui_frames(1)?;
                }
                Ok(json!({"frames": n}))
            }
            "ui_screenshot" => {
                let file = text("file").ok_or("ui_screenshot needs file")?;
                let n = num("frames").unwrap_or(3.0).max(1.0) as usize;
                if n > 1 {
                    self.ui_frames(n - 1)?;
                }
                let full = self.ui_frames(1)?.ok_or("no frame")?;
                let px = self.ui_paint(full)?;
                let path = out.join(file);
                write_png(&path, self.width, self.height, &px)?;
                Ok(json!({"file": path.display().to_string(), "width": self.width, "height": self.height}))
            }
            "ui_click" => {
                let at = egui::pos2(num("x").ok_or("ui_click needs x")? as f32, num("y").ok_or("ui_click needs y")? as f32);
                self.ui_click(at)?;
                Ok(json!({"clicked": [at.x, at.y], "status": self.ui.as_ref().map(|u| u.state.status.clone())}))
            }
            "ui_hover" => {
                let at = egui::pos2(num("x").ok_or("ui_hover needs x")? as f32, num("y").ok_or("ui_hover needs y")? as f32);
                self.pointer("move", at)?;
                self.ui_frames(1)?;
                Ok(json!({"hovered": [at.x, at.y]}))
            }
            "ui_key" => {
                let b = keymap::parse(text("key").ok_or("ui_key needs key")?)?;
                let modifiers = egui::Modifiers { alt: b.alt, ctrl: b.ctrl, shift: b.shift, mac_cmd: false, command: b.ctrl };
                if let Some(u) = self.ui.as_mut() {
                    u.modifiers = modifiers;
                }
                let press = egui::Event::Key { key: b.key, physical_key: None, pressed: true, repeat: false, modifiers };
                let release = egui::Event::Key { key: b.key, physical_key: None, pressed: false, repeat: false, modifiers };
                self.ui_frame(vec![press])?;
                self.ui_frame(vec![release])?;
                if let Some(u) = self.ui.as_mut() {
                    u.modifiers = egui::Modifiers::NONE;
                }
                self.ui_frames(1)?;
                let st = &self.world.state;
                let a = st.active.min(st.layout.slots().len() - 1);
                let cam = &st.cams[st.layout.slots()[a]];
                Ok(json!({"key": text("key"), "status": self.ui.as_ref().map(|u| u.state.status.clone()), "camera": {"name": cam.name, "yaw": cam.goal.yaw, "pitch": cam.goal.pitch, "ortho": cam.ortho}}))
            }
            "wizard" => {
                let action = text("action").ok_or("wizard needs action (start, set, run, next, back, wait)")?;
                let world = &self.world;
                let u = self.ui.as_mut().ok_or("no interface: run a ui step first")?;
                let w = &mut u.state.wizards;
                match action {
                    "start" => {
                        if world.session.is_none() {
                            return Err("nothing is loaded".into());
                        }
                        w.open = true;
                        w.start(text("id").ok_or("wizard start needs id")?, crate::wizard::run_values(world), world.settle_input(), text("project").unwrap_or("untitled"))?;
                    }
                    "set" | "run" => {
                        w.load_form();
                        for (k, v) in step.get("input").and_then(Value::as_object).into_iter().flatten() {
                            w.set_field(k, v);
                        }
                        if action == "run" {
                            match w.run_step() {
                                Err(e) if step.get("expect_error").and_then(Value::as_bool) == Some(true) => w.show_error(e),
                                other => other?,
                            }
                        }
                    }
                    "next" => w.go(1),
                    "back" => w.go(-1),
                    "wait" => {
                        let t = std::time::Instant::now();
                        while w.running() {
                            if t.elapsed().as_secs_f64() > num("seconds").unwrap_or(600.0) {
                                return Err("wizard wait timed out".into());
                            }
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            w.poll()?;
                        }
                    }
                    other => return Err(format!("unknown wizard action {other}")),
                }
                let state = w.state();
                self.ui_frames(2)?;
                Ok(state)
            }
            "cube_click" | "cube_drag" => self.cube_step(op, step),
            other => Err(format!("unknown op {other}")),
        }
    }

    fn settle_cameras(&mut self) -> Result<usize, String> {
        for i in 0..240 {
            let still = self.world.state.cams.iter().all(|c| (c.cur.yaw - c.goal.yaw).abs() < 1e-6 && (c.cur.pitch - c.goal.pitch).abs() < 1e-6 && (c.cur.dist - c.goal.dist).abs() < 1e-9);
            if still {
                return Ok(i);
            }
            self.ui_frames(1)?;
        }
        Ok(240)
    }

    fn spot(&self, step: &Value) -> Result<CubeSpot, String> {
        let u = self.ui.as_ref().ok_or("no interface: run a ui step first")?;
        let st = &self.world.state;
        let slot = step.get("slot").and_then(Value::as_u64).map_or(st.active.min(st.layout.slots().len() - 1), |s| s as usize);
        u.state.cubes.iter().find(|c| c.slot == slot).cloned().ok_or_else(|| format!("no view cube on viewport {slot} in the {} layout", st.layout.name()))
    }

    fn cube_now(&self, spot: &CubeSpot) -> Cube {
        Cube::new(spot.centre, spot.radius, &Camera::basis(&self.world.state.cams[spot.cam].cur))
    }

    fn cube_step(&mut self, op: &str, step: &Value) -> Result<Value, String> {
        self.ui_frames(1)?;
        let waited = self.settle_cameras()?;
        let spot = self.spot(step)?;
        let cam_json = |c: &crate::viewer::Camera| json!({"name": c.name, "yaw": c.goal.yaw, "pitch": c.goal.pitch, "ortho": c.ortho, "dist": c.goal.dist});
        if op == "cube_drag" {
            let (dx, dy) = (step.get("dx").and_then(Value::as_f64).unwrap_or(0.0), step.get("dy").and_then(Value::as_f64).unwrap_or(0.0));
            let n = step.get("steps").and_then(Value::as_u64).unwrap_or(12).max(1) as usize;
            let before = self.world.state.cams[spot.cam].goal;
            let start = spot.centre;
            self.pointer("move", start)?;
            self.pointer("down", start)?;
            for k in 1..=n {
                self.pointer("move", start + egui::vec2((dx * k as f64 / n as f64) as f32, (dy * k as f64 / n as f64) as f32))?;
            }
            self.pointer("up", start + egui::vec2(dx as f32, dy as f32))?;
            self.ui_frames(1)?;
            let after = self.world.state.cams[spot.cam].goal;
            let (want_yaw, want_pitch) = (-dx * 0.008, dy * 0.008);
            let (got_yaw, got_pitch) = (after.yaw - before.yaw, after.pitch - before.pitch);
            let ok = (got_yaw - want_yaw).abs() <= 0.1 * want_yaw.abs().max(0.01) && (got_pitch - want_pitch).abs() <= 0.1 * want_pitch.abs().max(0.01);
            return Ok(json!({"slot": spot.slot, "drag_px": [dx, dy], "yaw_change": got_yaw, "pitch_change": got_pitch, "expected": [want_yaw, want_pitch], "ok": ok,
                             "active": self.world.state.active, "camera": cam_json(&self.world.state.cams[spot.cam])}));
        }
        let target = step.get("element").and_then(Value::as_str).ok_or("cube_click needs element")?.to_string();
        let (point, want): (egui::Pos2, V3) = match target.as_str() {
            "home" => {
                let (_, _, yaw, pitch) = HOME[spot.cam];
                (spot.home.center(), Camera::basis(&CamState { yaw, pitch, dist: 1.0, target: [0.0; 3], fov: 30.0 }).back)
            }
            "threequarter" => {
                let &(_, yaw, pitch) = NAMED_VIEWS.iter().find(|v| v.0 == "threequarter").expect("named view");
                (spot.three.center(), Camera::basis(&CamState { yaw, pitch, dist: 1.0, target: [0.0; 3], fov: 30.0 }).back)
            }
            name => {
                let e = Element::parse(name).ok_or_else(|| format!("{name} is not a cube face, edge or corner"))?;
                let p = self.cube_now(&spot).point_of(e).ok_or_else(|| format!("{name} is not visible on the cube from this view"))?;
                (p, e.dir())
            }
        };
        let before = cam_json(&self.world.state.cams[spot.cam]);
        self.ui_click(point)?;
        let angle = |back: V3| crate::viewer::dot(back, want).clamp(-1.0, 1.0).acos().to_degrees();
        let goal_err = angle(Camera::basis(&self.world.state.cams[spot.cam].goal).back);
        let settled = self.settle_cameras()?;
        let cur_err = angle(Camera::basis(&self.world.state.cams[spot.cam].cur).back);
        Ok(json!({"slot": spot.slot, "element": target, "clicked_at": [point.x, point.y], "waited_frames": waited, "before": before, "camera": cam_json(&self.world.state.cams[spot.cam]),
                  "goal_error_deg": goal_err, "settled_error_deg": cur_err, "settle_frames": settled, "active": self.world.state.active,
                  "status": self.ui.as_ref().map(|u| u.state.status.clone()), "ok": goal_err < 0.1 && cur_err < 0.1 && self.world.state.active == spot.slot}))
    }
}
