use std::f64::consts::PI;

pub const DEG: f64 = PI / 180.0;

pub const MODES: [&str; 12] = ["clay", "flat", "gouraud", "blinn", "aniso", "matcap", "normals", "hidden", "quality", "displacement", "curvature", "uv"];
pub const DISPLACEMENT: usize = 9;

pub fn mode_index(name: &str) -> Option<usize> {
    MODES.iter().position(|id| *id == name)
}

pub const NAMED_VIEWS: [(&str, f64, f64); 7] = [
    ("front", 0.0, 0.0),
    ("back", PI, 0.0),
    ("left", PI / 2.0, 0.0),
    ("right", -PI / 2.0, 0.0),
    ("top", 0.0, PI / 2.0),
    ("bottom", 0.0, -PI / 2.0),
    ("threequarter", 0.61, 0.14),
];

pub const HOME: [(&str, bool, f64, f64); 4] = [("perspective", false, 0.0, 0.08), ("front", true, 0.0, 0.0), ("side", true, PI / 2.0, 0.0), ("top", true, 0.0, PI / 2.0)];

pub type V3 = [f64; 3];

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn mul(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn norm(a: V3) -> V3 {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let l = if l == 0.0 { 1.0 } else { l };
    [a[0] / l, a[1] / l, a[2] / l]
}
fn wrap_angle(a: f64) -> f64 {
    a.sin().atan2(a.cos())
}

pub fn kelvin_rgb(k: f64) -> V3 {
    let t = k / 100.0;
    let (r, g, b) = if t <= 66.0 {
        (255.0, 99.4708025861 * t.ln() - 161.1195681661, if t <= 19.0 { 0.0 } else { 138.5177312231 * (t - 10.0).ln() - 305.0447927307 })
    } else {
        (329.698727446 * (t - 60.0).powf(-0.1332047592), 288.1221695283 * (t - 60.0).powf(-0.0755148492), 255.0)
    };
    [r, g, b].map(|x: f64| (x.clamp(0.0, 255.0) / 255.0).powf(2.2))
}

#[derive(Clone, Copy, Debug)]
pub struct CamState {
    pub yaw: f64,
    pub pitch: f64,
    pub dist: f64,
    pub target: V3,
    pub fov: f64,
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub goal: CamState,
    pub cur: CamState,
    pub ortho: bool,
    pub name: String,
}

pub struct Basis {
    pub back: V3,
    pub s: V3,
    pub u: V3,
    pub f: V3,
}

pub struct Matrices {
    pub mvp: [f32; 16],
    pub eye: V3,
    pub s: V3,
    pub u: V3,
    pub f: V3,
    pub half_h: f64,
    pub ortho: bool,
    pub tanh: f64,
    pub near: f64,
    pub far: f64,
}

impl Camera {
    pub fn new(name: &str, ortho: bool, yaw: f64, pitch: f64) -> Camera {
        let s = CamState { yaw, pitch, dist: 0.45, target: [0.0; 3], fov: 30.0 };
        let mut c = Camera { goal: s, cur: s, ortho, name: name.to_string() };
        c.clamp_goal();
        c.cur = c.goal;
        c
    }

    fn clamp_goal(&mut self) {
        let g = &mut self.goal;
        g.pitch = g.pitch.clamp(-PI / 2.0 + 1e-3, PI / 2.0 - 1e-3);
        g.dist = g.dist.clamp(0.02, 8.0);
        g.fov = g.fov.clamp(5.0, 100.0);
    }

    pub fn set(&mut self, f: impl FnOnce(&mut CamState), instant: bool) {
        f(&mut self.goal);
        self.clamp_goal();
        if instant {
            self.cur = self.goal;
        }
    }

    pub fn view(&mut self, yaw: f64, pitch: f64, instant: bool) {
        let cy = self.cur.yaw;
        self.set(|g| {
            g.yaw = cy + wrap_angle(yaw - cy);
            g.pitch = pitch;
        }, instant);
    }

    pub fn snap(&mut self) {
        self.cur = self.goal;
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let a = 1.0 - (-(dt.min(0.1)) * 18.0).exp();
        let mut moving = false;
        let mut ease = |x: f64, y: f64, eps: f64| {
            let d = y - x;
            if d.abs() > eps {
                moving = true;
                x + d * a
            } else {
                y
            }
        };
        let (c, g) = (&mut self.cur, &self.goal);
        c.yaw = ease(c.yaw, g.yaw, 1e-4);
        c.pitch = ease(c.pitch, g.pitch, 1e-4);
        c.fov = ease(c.fov, g.fov, 1e-3);
        c.dist = ease(c.dist.ln(), g.dist.ln(), 1e-4).exp();
        for k in 0..3 {
            c.target[k] = ease(c.target[k], g.target[k], 1e-6);
        }
        moving
    }

    pub fn basis(s: &CamState) -> Basis {
        let (cp, sp, cy, sy) = (s.pitch.cos(), s.pitch.sin(), s.yaw.cos(), s.yaw.sin());
        let back = [sy * cp, -cy * cp, sp];
        let side = [cy, sy, 0.0];
        let f = mul(back, -1.0);
        Basis { back, s: side, u: cross(side, f), f }
    }

    pub fn matrices(&self, aspect: f64) -> Matrices {
        let c = &self.cur;
        let b = Camera::basis(c);
        let tanh = (c.fov * DEG / 2.0).tan();
        let half_h = c.dist * tanh;
        let eye_dist = if self.ortho { c.dist.max(3.0) } else { c.dist };
        let eye = add(c.target, mul(b.back, eye_dist));
        let near = if self.ortho { 0.005 } else { (c.dist * 0.03).max(0.001) };
        let far = eye_dist + 4.0;
        let (s, u, f) = (b.s, b.u, b.f);
        let view = [s[0], u[0], -f[0], 0.0, s[1], u[1], -f[1], 0.0, s[2], u[2], -f[2], 0.0, -dot(s, eye), -dot(u, eye), dot(f, eye), 1.0];
        let proj = if self.ortho {
            let w = half_h * aspect;
            [1.0 / w, 0.0, 0.0, 0.0, 0.0, 1.0 / half_h, 0.0, 0.0, 0.0, 0.0, -2.0 / (far - near), 0.0, 0.0, 0.0, -(far + near) / (far - near), 1.0]
        } else {
            let t = 1.0 / tanh;
            [t / aspect, 0.0, 0.0, 0.0, 0.0, t, 0.0, 0.0, 0.0, 0.0, (far + near) / (near - far), -1.0, 0.0, 0.0, (2.0 * far * near) / (near - far), 0.0]
        };
        let mut gl = [0.0f64; 16];
        for col in 0..4 {
            for r in 0..4 {
                let mut x = 0.0;
                for k in 0..4 {
                    x += proj[k * 4 + r] * view[col * 4 + k];
                }
                gl[col * 4 + r] = x;
            }
        }
        let mut mvp = [0.0f32; 16];
        for col in 0..4 {
            mvp[col * 4] = gl[col * 4] as f32;
            mvp[col * 4 + 1] = gl[col * 4 + 1] as f32;
            mvp[col * 4 + 2] = (0.5 * gl[col * 4 + 2] + 0.5 * gl[col * 4 + 3]) as f32;
            mvp[col * 4 + 3] = gl[col * 4 + 3] as f32;
        }
        Matrices { mvp, eye, s, u, f, half_h, ortho: self.ortho, tanh, near, far }
    }

    pub fn ray(&self, nx: f64, ny: f64, aspect: f64) -> (V3, V3) {
        let m = self.matrices(aspect);
        if m.ortho {
            return (add(m.eye, add(mul(m.s, nx * m.half_h * aspect), mul(m.u, ny * m.half_h))), m.f);
        }
        (m.eye, norm(add(m.f, add(mul(m.s, nx * m.tanh * aspect), mul(m.u, ny * m.tanh)))))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub lo: V3,
    pub hi: V3,
    pub centre: V3,
    pub radius: f64,
}

impl Bounds {
    pub fn of(positions: &[f32]) -> Bounds {
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for p in positions.chunks_exact(3) {
            for k in 0..3 {
                let x = p[k] as f64;
                lo[k] = lo[k].min(x);
                hi[k] = hi[k].max(x);
            }
        }
        let centre = [0, 1, 2].map(|k| (lo[k] + hi[k]) / 2.0);
        let radius = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt() / 2.0;
        Bounds { lo, hi, centre, radius }
    }
}

pub fn frame_box(b: &Bounds, cam: &mut Camera, instant: bool) {
    let half = cam.goal.fov * DEG / 2.0;
    let dist = if cam.ortho { b.radius * 0.8 / half.tan() } else { b.radius * 0.8 / half.sin() };
    let centre = b.centre;
    cam.set(|g| {
        g.target = centre;
        g.dist = dist;
    }, instant);
}

#[derive(Clone, Debug)]
pub struct Light {
    pub on: bool,
    pub az: f64,
    pub el: f64,
    pub i: f64,
    pub k: f64,
}

#[derive(Clone, Debug)]
pub struct ClipPlane {
    pub on: bool,
    pub axis: String,
    pub az: f64,
    pub el: f64,
    pub offset: f64,
    pub flip: bool,
    pub slab: bool,
    pub thick: f64,
}

#[derive(Clone, Debug)]
pub struct ClipBox {
    pub on: bool,
    pub lo: V3,
    pub hi: V3,
}

#[derive(Clone, Debug)]
pub struct Heat {
    pub quality: f64,
    pub displacement: f64,
    pub curvature: f64,
}

#[derive(Clone, Debug)]
pub struct Opts {
    pub wire: bool,
    pub wire_w: f64,
    pub wire_fade: bool,
    pub xray: bool,
    pub points: bool,
    pub point_size: f64,
    pub eyes: bool,
    pub skin: bool,
    pub back_hi: bool,
    pub seams: bool,
    pub heat: Heat,
    pub aniso: f64,
    pub exposure: f64,
    pub ambient: f64,
    pub ground: f64,
    pub follow: bool,
    pub lights: Vec<Light>,
    pub caps: bool,
    pub plane_gizmo: bool,
    pub light_gizmo: bool,
    pub clip: Vec<ClipPlane>,
    pub clip_box: ClipBox,
    pub dark: bool,
    pub bad: bool,
}

pub const MAX_LIGHTS: usize = 4;

pub fn light_preset(name: &str) -> Option<(f64, f64, Vec<Light>)> {
    let l = |az: f64, el: f64, i: f64, k: f64| Light { on: true, az, el, i, k };
    Some(match name {
        "studio" => (0.16, 0.4, vec![l(-40.0, 30.0, 1.0, 5600.0), l(50.0, 10.0, 0.35, 6500.0), l(160.0, 35.0, 0.8, 7500.0)]),
        "flat" => (0.9, 1.0, vec![]),
        "rim" => (0.06, 0.4, vec![l(150.0, 20.0, 1.1, 7000.0), l(-150.0, 20.0, 1.1, 7000.0), l(0.0, 10.0, 0.15, 5000.0)]),
        "top" => (0.1, 0.3, vec![l(0.0, 85.0, 1.2, 5500.0)]),
        "key" => (0.08, 0.4, vec![l(-35.0, 25.0, 1.2, 5200.0)]),
        _ => return None,
    })
}

impl Default for Opts {
    fn default() -> Opts {
        let (ambient, ground, mut lights) = light_preset("studio").expect("studio preset");
        while lights.len() < MAX_LIGHTS {
            lights.push(Light { on: false, az: 0.0, el: 0.0, i: 0.5, k: 6500.0 });
        }
        Opts {
            wire: false,
            wire_w: 0.75,
            wire_fade: true,
            xray: false,
            points: false,
            point_size: 3.0,
            eyes: true,
            skin: true,
            back_hi: false,
            seams: false,
            heat: Heat { quality: 30.0, displacement: 3.0, curvature: 50.0 },
            aniso: 0.0,
            exposure: 0.0,
            ambient,
            ground,
            follow: true,
            lights,
            caps: true,
            plane_gizmo: true,
            light_gizmo: false,
            clip: ["x", "y", "z"]
                .iter()
                .map(|a| ClipPlane { on: false, axis: a.to_string(), az: 0.0, el: 0.0, offset: 0.0, flip: false, slab: false, thick: 0.01 })
                .collect(),
            clip_box: ClipBox { on: false, lo: [0.4, 0.0, 0.4], hi: [1.0, 0.5, 1.0] },
            dark: true,
            bad: false,
        }
    }
}

impl Opts {
    pub fn apply_preset(&mut self, name: &str) -> bool {
        let Some((ambient, ground, mut lights)) = light_preset(name) else { return false };
        while lights.len() < MAX_LIGHTS {
            lights.push(Light { on: false, az: 0.0, el: 0.0, i: 0.5, k: 6500.0 });
        }
        self.ambient = ambient;
        self.ground = ground;
        self.lights = lights;
        true
    }

    pub fn set(&mut self, path: &str, v: &serde_json::Value) -> Result<(), String> {
        let num = || v.as_f64().ok_or_else(|| format!("{path} needs a number"));
        let flag = || v.as_bool().ok_or_else(|| format!("{path} needs true or false"));
        let parts: Vec<&str> = path.split('.').collect();
        match parts.as_slice() {
            ["wire"] => self.wire = flag()?,
            ["wireW"] => self.wire_w = num()?,
            ["wireFade"] => self.wire_fade = flag()?,
            ["xray"] => self.xray = flag()?,
            ["points"] => self.points = flag()?,
            ["eyes"] => self.eyes = flag()?,
            ["skin"] => self.skin = flag()?,
            ["pointSize"] => self.point_size = num()?,
            ["backHi"] => self.back_hi = flag()?,
            ["seams"] => self.seams = flag()?,
            ["heat", "quality"] => self.heat.quality = num()?,
            ["heat", "displacement"] => self.heat.displacement = num()?,
            ["heat", "curvature"] => self.heat.curvature = num()?,
            ["aniso"] => self.aniso = num()?,
            ["exposure"] => self.exposure = num()?,
            ["ambient"] => self.ambient = num()?,
            ["ground"] => self.ground = num()?,
            ["follow"] => self.follow = flag()?,
            ["caps"] => self.caps = flag()?,
            ["planeGizmo"] => self.plane_gizmo = flag()?,
            ["lightGizmo"] => self.light_gizmo = flag()?,
            ["dark"] => self.dark = flag()?,
            ["badTriangles"] => self.bad = flag()?,
            ["preset"] => {
                if !self.apply_preset(v.as_str().unwrap_or("")) {
                    return Err(format!("unknown light preset {v}"));
                }
            }
            ["lights", i, field] => {
                let l = self.lights.get_mut(i.parse::<usize>().map_err(|_| format!("bad light index in {path}"))?).ok_or_else(|| format!("no light in {path}"))?;
                match *field {
                    "on" => l.on = flag()?,
                    "az" => l.az = num()?,
                    "el" => l.el = num()?,
                    "i" => l.i = num()?,
                    "k" => l.k = num()?,
                    _ => return Err(format!("unknown option {path}")),
                }
            }
            ["clip", i, field] => {
                let c = self.clip.get_mut(i.parse::<usize>().map_err(|_| format!("bad clip index in {path}"))?).ok_or_else(|| format!("no clip plane in {path}"))?;
                match *field {
                    "on" => c.on = flag()?,
                    "axis" => c.axis = v.as_str().filter(|a| ["x", "y", "z", "free"].contains(a)).ok_or_else(|| format!("{path} is x, y, z or free"))?.to_string(),
                    "az" => c.az = num()?,
                    "el" => c.el = num()?,
                    "offset" => c.offset = num()?,
                    "flip" => c.flip = flag()?,
                    "slab" => c.slab = flag()?,
                    "thick" => c.thick = num()?,
                    _ => return Err(format!("unknown option {path}")),
                }
            }
            ["box", "on"] => self.clip_box.on = flag()?,
            ["box", which @ ("lo" | "hi")] => {
                let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| format!("{path} needs [x, y, z]"))?;
                let p = [0, 1, 2].map(|k| a[k].as_f64().unwrap_or(0.0));
                if *which == "lo" {
                    self.clip_box.lo = p;
                } else {
                    self.clip_box.hi = p;
                }
            }
            _ => return Err(format!("unknown option {path}")),
        }
        Ok(())
    }

    pub fn toggle(&mut self, path: &str) -> Result<bool, String> {
        let now = match path {
            "wire" => self.wire,
            "xray" => self.xray,
            "points" => self.points,
            "eyes" => self.eyes,
            "skin" => self.skin,
            "backHi" => self.back_hi,
            "seams" => self.seams,
            "lightGizmo" => self.light_gizmo,
            "clip.0.on" => self.clip[0].on,
            "dark" => self.dark,
            "badTriangles" => self.bad,
            _ => return Err(format!("{path} cannot be toggled")),
        };
        self.set(path, &serde_json::Value::Bool(!now))?;
        Ok(!now)
    }
}

pub struct Plane {
    pub n: V3,
    pub d: f64,
}

pub fn planes(opts: &Opts, b: &Bounds) -> Vec<Plane> {
    let mut out = Vec::new();
    for c in &opts.clip {
        if !c.on {
            continue;
        }
        let mut n = match c.axis.as_str() {
            "x" => [1.0, 0.0, 0.0],
            "y" => [0.0, 1.0, 0.0],
            "z" => [0.0, 0.0, 1.0],
            _ => [(c.el * DEG).cos() * (c.az * DEG).sin(), -(c.el * DEG).cos() * (c.az * DEG).cos(), (c.el * DEG).sin()],
        };
        if c.flip {
            n = mul(n, -1.0);
        }
        let d = dot(n, b.centre) + c.offset;
        out.push(Plane { n, d });
        if c.slab {
            out.push(Plane { n: mul(n, -1.0), d: -(d - c.thick) });
        }
    }
    out.truncate(6);
    out
}

pub fn box_world(opts: &Opts, b: &Bounds) -> (V3, V3) {
    let ext = sub(b.hi, b.lo);
    let bx = &opts.clip_box;
    let lo = [0, 1, 2].map(|k| b.lo[k] + ext[k] * bx.lo[k].min(bx.hi[k]));
    let hi = [0, 1, 2].map(|k| b.lo[k] + ext[k] * bx.lo[k].max(bx.hi[k]));
    (lo, hi)
}

pub fn lights_world(opts: &Opts, m: &Matrices) -> (Vec<V3>, Vec<V3>) {
    let (mut dirs, mut cols) = (Vec::new(), Vec::new());
    for l in &opts.lights {
        if !l.on || dirs.len() >= MAX_LIGHTS {
            continue;
        }
        let (ce, se, sa, ca) = ((l.el * DEG).cos(), (l.el * DEG).sin(), (l.az * DEG).sin(), (l.az * DEG).cos());
        let d = if opts.follow { norm(add(add(mul(m.s, sa * ce), mul(m.u, se)), mul(m.f, -ca * ce))) } else { norm([sa * ce, -ca * ce, se]) };
        dirs.push(d);
        cols.push(mul(kelvin_rgb(l.k), l.i));
    }
    (dirs, cols)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Single,
    Side,
    Quad,
}

impl Layout {
    pub fn parse(s: &str) -> Option<Layout> {
        Some(match s {
            "single" => Layout::Single,
            "side" => Layout::Side,
            "quad" => Layout::Quad,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Layout::Single => "single",
            Layout::Side => "side",
            Layout::Quad => "quad",
        }
    }
    pub fn next(self) -> Layout {
        match self {
            Layout::Single => Layout::Side,
            Layout::Side => Layout::Quad,
            Layout::Quad => Layout::Single,
        }
    }
    pub fn slots(self) -> &'static [usize] {
        match self {
            Layout::Single => &[0],
            Layout::Side => &[0, 2],
            Layout::Quad => &[1, 2, 3, 0],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub slot: usize,
    pub cam: usize,
}

pub fn rects(layout: Layout, w: u32, h: u32) -> Vec<Rect> {
    let s = layout.slots();
    let r = |x: u32, y: u32, w: u32, h: u32, i: usize| Rect { x, y, w: w.max(1), h: h.max(1), slot: i, cam: s[i] };
    match s.len() {
        1 => vec![r(0, 0, w, h, 0)],
        2 => {
            let hw = w / 2;
            vec![r(0, 0, hw, h, 0), r(hw, 0, w - hw, h, 1)]
        }
        _ => {
            let (hw, hh) = (w / 2, h / 2);
            vec![r(0, 0, hw, hh, 0), r(hw, 0, w - hw, hh, 1), r(0, hh, hw, h - hh, 2), r(hw, hh, w - hw, h - hh, 3)]
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SlotState {
    pub mode: usize,
    pub lod: usize,
}

pub struct ViewerState {
    pub cams: Vec<Camera>,
    pub layout: Layout,
    pub active: usize,
    pub vp: [SlotState; 4],
    pub opts: Opts,
    pub bounds: Bounds,
    pub pivot_flash: f64,
    pub head_box: Option<(V3, V3)>,
}

impl Default for ViewerState {
    fn default() -> ViewerState {
        ViewerState {
            cams: HOME.iter().map(|&(name, ortho, yaw, pitch)| Camera::new(name, ortho, yaw, pitch)).collect(),
            layout: Layout::Single,
            active: 0,
            vp: [SlotState { mode: 0, lod: 0 }; 4],
            opts: Opts::default(),
            bounds: Bounds { lo: [-0.1; 3], hi: [0.1; 3], centre: [0.0; 3], radius: 0.15 },
            pivot_flash: 0.0,
            head_box: None,
        }
    }
}

impl ViewerState {
    pub fn active_rect(&self, w: u32, h: u32) -> Rect {
        let r = rects(self.layout, w, h);
        let i = self.active.min(r.len() - 1);
        r[i]
    }

    pub fn active_cam(&mut self) -> &mut Camera {
        let slots = self.layout.slots();
        let cam = slots[self.active.min(slots.len() - 1)];
        &mut self.cams[cam]
    }

    pub fn active_slot(&mut self) -> &mut SlotState {
        let slots = self.layout.slots().len();
        &mut self.vp[self.active.min(slots - 1)]
    }

    pub fn named_view(&mut self, name: &str, instant: bool) -> bool {
        let Some(&(_, yaw, pitch)) = NAMED_VIEWS.iter().find(|v| v.0 == name) else { return false };
        let cam = self.active_cam();
        cam.view(yaw, pitch, instant);
        cam.name = if name == "threequarter" { "three-quarter".into() } else { name.into() };
        true
    }

    pub fn active_cam_index(&self) -> usize {
        let slots = self.layout.slots();
        slots[self.active.min(slots.len() - 1)]
    }

    pub fn home_view(&mut self, instant: bool) {
        let i = self.active_cam_index();
        let (name, _, yaw, pitch) = HOME[i];
        let cam = &mut self.cams[i];
        cam.view(yaw, pitch, instant);
        cam.name = name.into();
    }

    pub fn view_dir(&mut self, name: &str, yaw: f64, pitch: f64, instant: bool) {
        let cam = self.active_cam();
        cam.view(yaw, pitch, instant);
        cam.name = name.into();
    }

    pub fn set_fov(&mut self, f: f64, instant: bool) {
        let cam = self.active_cam();
        let (old, nf) = (cam.goal.fov, f.clamp(10.0, 90.0));
        let dist = cam.goal.dist * (old * PI / 360.0).tan() / (nf * PI / 360.0).tan();
        cam.set(|g| {
            g.fov = nf;
            g.dist = dist;
        }, instant);
    }

    pub fn orbit(&mut self, cam: usize, dx: f64, dy: f64, instant: bool) {
        let c = &mut self.cams[cam];
        let (yaw, pitch) = (c.goal.yaw - dx * 0.008, c.goal.pitch + dy * 0.008);
        c.set(|g| {
            g.yaw = yaw;
            g.pitch = pitch;
        }, instant);
        if c.name != "perspective" {
            c.name = "free".into();
        }
    }

    pub fn pan(&mut self, cam: usize, dx: f64, dy: f64, h: f64, instant: bool) {
        let c = &mut self.cams[cam];
        let b = Camera::basis(&c.goal);
        let wpp = 2.0 * c.goal.dist * (c.goal.fov * PI / 360.0).tan() / h.max(1.0);
        let t = c.goal.target;
        let target = [0, 1, 2].map(|k| t[k] - b.s[k] * dx * wpp + b.u[k] * dy * wpp);
        c.set(|g| g.target = target, instant);
    }

    pub fn zoom(&mut self, cam: usize, wheel_delta_y: f64, instant: bool) {
        let c = &mut self.cams[cam];
        let dist = c.goal.dist * (wheel_delta_y * 0.0012).exp();
        c.set(|g| g.dist = dist, instant);
    }

    pub fn step(&mut self, dt: f64) -> bool {
        let mut moving = false;
        for &c in self.layout.slots() {
            moving |= self.cams[c].step(dt);
        }
        if self.pivot_flash > 0.0 {
            self.pivot_flash -= dt;
            moving = true;
        }
        moving
    }
}
