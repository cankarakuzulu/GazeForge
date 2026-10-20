use std::path::PathBuf;
use std::time::Instant;

use eframe::egui;
use eframe::wgpu;
use serde_json::json;

use crate::godot;
use crate::render::Renderer;
use crate::studio::{self, Gfx, Studio, UiArgs, UiState};
use crate::world::World;

pub struct Args {
    pub store: Option<String>,
    pub spec: Option<String>,
    pub macro_file: Option<String>,
    pub eyes: Option<String>,
    pub keymap: Option<PathBuf>,
    pub bench: Option<(f64, bool)>,
    pub present: Option<String>,
    pub input: Option<String>,
    pub godot_config: Option<PathBuf>,
}

struct Bench {
    seconds: f64,
    idle: bool,
    started: Option<Instant>,
    intervals: Vec<f64>,
    render_ms: Vec<f64>,
    ui_ms: Vec<f64>,
    surface: serde_json::Value,
}

pub struct StudioApp {
    renderer: Renderer,
    world: World,
    ui: UiState,
    last: Instant,
    bench: Option<Bench>,
    godot: Option<std::sync::Arc<godot::Host>>,
}

pub fn run(args: Args) -> Result<(), String> {
    let mut options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1440.0, 900.0]).with_title("GazeForge Studio"), ..Default::default() };
    options.wgpu_options.surface.present_mode = match args.present.as_deref() {
        None | Some("vsync") => wgpu::PresentMode::AutoVsync,
        Some("immediate") => wgpu::PresentMode::AutoNoVsync,
        Some(other) => return Err(format!("--present is vsync or immediate, not {other}")),
    };
    let started = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        eframe::run_native("GazeForge Studio", options, Box::new(move |cc| StudioApp::new(cc, args).map(|a| Box::new(a) as Box<dyn eframe::App>).map_err(|e| e.into())))
    }));
    match started {
        Ok(r) => r.map_err(|e| e.to_string()),
        Err(p) => {
            let what = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            let backend = std::env::var("WGPU_BACKEND").map_or_else(|_| "the default backend".to_string(), |b| format!("WGPU_BACKEND={b}"));
            Err(format!("the window's GPU setup failed with {backend}: {}; GAZEFORGE_LOG=warn prints the driver's reason", what.lines().collect::<Vec<_>>().join(" ")))
        }
    }
}

impl StudioApp {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Result<StudioApp, String> {
        studio::setup_context(&cc.egui_ctx)?;
        let rs = cc.wgpu_render_state.as_ref().ok_or("the window has no wgpu render state")?;
        let renderer = Renderer::new(&rs.device);
        let ui = UiState::new(UiArgs {
            keymap: args.keymap.clone(),
            input: args.input.clone(),
            store: args.store.clone(),
            spec: args.spec.clone(),
            macro_file: args.macro_file.clone(),
            eyes: args.eyes.clone(),
            godot_config: args.godot_config.clone(),
        })?;
        let world = World::default();
        let ctx = cc.egui_ctx.clone();
        world.set_wake(Box::new(move || ctx.request_repaint()));
        let mut app = StudioApp {
            renderer,
            world,
            ui,
            last: Instant::now(),
            godot: None,
            bench: args.bench.map(|(seconds, idle)| {
                let info = rs.adapter.get_info();
                let surface = json!({"adapter": info.name, "backend": format!("{:?}", info.backend), "target_format": format!("{:?}", rs.target_format)});
                Bench { seconds, idle, started: None, intervals: Vec::new(), render_ms: Vec::new(), ui_ms: Vec::new(), surface }
            }),
        };
        if !app.ui.store.is_empty() {
            app.studio(rs, false, false).load();
        }
        Ok(app)
    }

    fn studio<'a>(&'a mut self, rs: &'a eframe::egui_wgpu::RenderState, spin: bool, keep_painting: bool) -> Studio<'a> {
        Studio {
            s: &mut self.ui,
            world: &mut self.world,
            renderer: &mut self.renderer,
            godot: &mut self.godot,
            gfx: Gfx { device: &rs.device, queue: &rs.queue, egui: &rs.renderer },
            spin,
            keep_painting,
        }
    }
}

impl eframe::App for StudioApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64();
        self.last = now;
        self.ui.frame_ms = dt * 1000.0;
        self.world.hud.presented(now);
        let ctx = ui.ctx().clone();
        if let Some(t0) = self.bench.as_ref().filter(|b| !b.idle).and_then(|b| b.started.map(|t| (t, b.seconds))) {
            let (elapsed, half) = (t0.0.elapsed().as_secs_f64(), t0.1 / 2.0);
            if let (true, Some(id)) = (elapsed > half, self.world.shape.as_ref().and_then(|s| s.targets.first()).map(|t| t.id.clone())) {
                let v = 0.5 - 0.5 * ((elapsed - half) * 3.0).cos();
                self.world.shape_event(crate::shape::ShapeEvent::Preview(vec![crate::shape::Edit::Weight(id, v)]));
            }
        }
        let spin = self.bench.as_ref().is_some_and(|b| b.started.is_some() && !b.idle);
        let keep = self.bench.as_ref().is_some_and(|b| !b.idle);
        if let Some(rs) = frame.wgpu_render_state() {
            self.studio(rs, spin, keep).frame(ui, dt);
        }
        self.world.hud.frame_cpu.push(now.elapsed().as_secs_f64() * 1000.0);
        let loaded = self.world.session.is_some() && !(self.bench.as_ref().is_some_and(|b| b.idle) && self.world.busy_text().is_some());
        let mut finished = None;
        if let Some(b) = &mut self.bench
            && loaded
        {
            match b.started {
                None => {
                    b.started = Some(now);
                    if b.idle {
                        ctx.request_repaint_after(std::time::Duration::from_secs_f64(b.seconds));
                    }
                }
                Some(t0) => {
                    if b.idle {
                        ctx.request_repaint_after(std::time::Duration::from_secs_f64((b.seconds - t0.elapsed().as_secs_f64()).max(0.0)));
                    }
                    b.intervals.push(dt * 1000.0);
                    b.render_ms.push(self.ui.render_ms);
                    b.ui_ms.push(now.elapsed().as_secs_f64() * 1000.0);
                    b.surface["monitor_size"] = json!(ctx.input(|i| i.viewport().monitor_size.map(|s| [s.x, s.y])));
                    b.surface["inner_rect"] = json!(ctx.input(|i| i.viewport().inner_rect.map(|r| [r.min.x, r.min.y, r.width(), r.height()])));
                    b.surface["pixels_per_point"] = json!(ctx.pixels_per_point());
                    if t0.elapsed().as_secs_f64() >= b.seconds {
                        let (window, desktop) = os_visibility(frame);
                        b.surface["os_window_visible"] = json!(window);
                        b.surface["os_desktop_visible"] = json!(desktop);
                        let mut r = bench_report(b);
                        r["hud"] = self.world.hud.json();
                        finished = Some(r);
                    }
                }
            }
        }
        if let Some(report) = finished {
            println!("{report}");
            self.bench = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn bench_report(b: &Bench) -> serde_json::Value {
    let stat = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.total_cmp(b));
        let n = s.len().max(1);
        let at = |q: f64| s.get(((n - 1) as f64 * q).round() as usize).copied().unwrap_or(0.0);
        json!({"count": s.len(), "mean": s.iter().sum::<f64>() / n as f64, "p50": at(0.5), "p95": at(0.95), "max": s.last().copied().unwrap_or(0.0)})
    };
    let total: f64 = b.intervals.iter().sum();
    if b.idle {
        let woken = b.intervals.len().saturating_sub(1);
        return json!({"ok": true, "mode": "idle", "seconds": total / 1000.0, "frames_while_idle": woken, "idle_hz": woken as f64 / (total / 1000.0).max(1e-9), "surface": b.surface});
    }
    let hidden = b.surface["os_window_visible"] == json!(false);
    let paced = if hidden { "the OS reports the window hidden, so eframe paints it at most every 100 ms and does not vsync; present_hz is that throttle, not the display" } else { "display" };
    json!({"ok": true, "mode": "window", "seconds": total / 1000.0, "frames": b.intervals.len(), "present_hz": b.intervals.len() as f64 / (total / 1000.0).max(1e-9), "paced_by": paced,
           "frame_interval_ms": stat(&b.intervals), "viewport_encode_ms": stat(&b.render_ms), "ui_cpu_ms": stat(&b.ui_ms), "surface": b.surface})
}

#[cfg(windows)]
fn os_visibility(frame: &eframe::Frame) -> (Option<bool>, Option<bool>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    #[link(name = "user32")]
    unsafe extern "system" {
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn GetDesktopWindow() -> isize;
    }
    let hwnd = match frame.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
        _ => None,
    };
    // SAFETY: both calls only read window state; the handle belongs to this process's live window.
    unsafe { (hwnd.map(|h| IsWindowVisible(h) != 0), Some(IsWindowVisible(GetDesktopWindow()) != 0)) }
}

#[cfg(not(windows))]
fn os_visibility(_frame: &eframe::Frame) -> (Option<bool>, Option<bool>) {
    (None, None)
}
