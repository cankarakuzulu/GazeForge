use std::path::Path;

use eframe::egui;
use eframe::egui_wgpu;
use eframe::wgpu;
use serde_json::{Value, json};

use crate::keymap;
use crate::render::read_texture;
use crate::script::{Offscreen, write_png};
use crate::studio::{self, Gfx, Studio, UiArgs, UiState};

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
                    let state = UiState::new(UiArgs { keymap: text("keymap").map(Into::into), input: None, store: None, spec: None, macro_file: None, eyes: None, godot_config: None })?;
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
            "ui_frames" => {
                let n = num("count").unwrap_or(1.0).max(1.0) as usize;
                self.ui_frames(n)?;
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
            other => Err(format!("unknown op {other}")),
        }
    }
}
