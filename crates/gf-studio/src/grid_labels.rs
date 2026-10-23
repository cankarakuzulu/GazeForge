use std::collections::HashSet;

use crate::face_grid::FaceGrid;
use crate::viewer::{Matrices, V3, dot, mul, norm, sub};

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub x: f64,
    pub y: f64,
    pub text: String,
    pub edge: bool,
}

#[derive(Clone, Copy)]
pub struct Shot<'a> {
    pub m: &'a Matrices,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub dist: f64,
}

pub struct Surface<'a> {
    pub pos: &'a [f32],
    pub idx: &'a [u32],
}

#[derive(Clone, Copy, Debug)]
pub struct Boxed {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl Boxed {
    fn hits(&self, q: &Boxed) -> bool {
        self.x0 < q.x1 && self.x1 > q.x0 && self.y0 < q.y1 && self.y1 > q.y0
    }
}

pub struct Axis {
    pub vertex: Vec<bool>,
    pub front: i64,
}

fn at(p: &[f32], i: usize) -> V3 {
    [p[i * 3] as f64, p[i * 3 + 1] as f64, p[i * 3 + 2] as f64]
}

pub fn project(s: &Shot, p: V3) -> Option<(f64, f64)> {
    let m = s.m.mvp.map(|x| x as f64);
    let x = m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12];
    let y = m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13];
    let w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
    if w <= 1e-9 {
        return None;
    }
    let (nx, ny) = (x / w, y / w);
    if nx.abs() > 1.0 || ny.abs() > 1.0 {
        return None;
    }
    Some((s.x + (nx + 1.0) / 2.0 * s.w, s.y + (1.0 - ny) / 2.0 * s.h))
}

pub fn facing(m: &Matrices, p: V3, n: V3) -> f64 {
    let v = if m.ortho { mul(m.f, -1.0) } else { norm(sub(m.eye, p)) };
    dot(n, v)
}

pub fn axis(g: &FaceGrid, lod0: &[u32], n: usize) -> Axis {
    let ch = &g.uv;
    let cols = g.letters.len().max(1) as i64;
    let front = g.letters.iter().position(|l| *l == g.front_column).unwrap_or(12) as i64;
    let cell_key = |v: usize| {
        let k = (ch[2 * v] as f64 / g.column_deg + 0.5).floor() as i64;
        let col = ((k + front) % cols + cols) % cols;
        let row = ((ch[2 * v + 1] as f64).max(0.0) / g.row_mm).floor() as i64 + 1;
        col * 4096 + row
    };
    let mut cells = HashSet::new();
    for t in lod0.chunks_exact(3) {
        let raw = [ch[2 * t[0] as usize] as f64, ch[2 * t[1] as usize] as f64, ch[2 * t[2] as usize] as f64];
        let us = raw.map(|u| u - 360.0 * ((u - raw[0]) / 360.0).round());
        let spread = |a: [f64; 3]| a.iter().cloned().fold(f64::MIN, f64::max) - a.iter().cloned().fold(f64::MAX, f64::min);
        if spread(us) >= g.axis_span_deg || spread(raw) >= 180.0 {
            for &v in t {
                cells.insert(cell_key(v as usize));
            }
        }
    }
    Axis { vertex: (0..n).map(|v| cells.contains(&cell_key(v))).collect(), front }
}

pub struct ZBuf {
    w: usize,
    h: usize,
    s: f64,
    buf: Vec<f64>,
    tol: f64,
}

pub fn zbuf(s: &Shot, meshes: &[Surface], cut: &dyn Fn(V3) -> bool) -> ZBuf {
    let step = 2.0;
    let (w, h) = (((s.w / step).ceil() as usize).max(1), ((s.h / step).ceil() as usize).max(1));
    let mut buf = vec![f64::INFINITY; w * h];
    let m = s.m.mvp.map(|x| x as f64);
    let f = s.m.f;
    for mesh in meshes {
        let n = mesh.pos.len() / 3;
        let mut sx = vec![0.0; n];
        let mut sy = vec![0.0; n];
        let mut sz = vec![f64::NAN; n];
        for i in 0..n {
            let p = at(mesh.pos, i);
            let ww = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
            if ww <= 1e-9 {
                continue;
            }
            sx[i] = ((m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12]) / ww + 1.0) * 0.5 * w as f64;
            sy[i] = (1.0 - (m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13]) / ww) * 0.5 * h as f64;
            sz[i] = dot(f, p);
        }
        for t in mesh.idx.chunks_exact(3) {
            let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
            if a >= n || b >= n || c >= n || sz[a].is_nan() || sz[b].is_nan() || sz[c].is_nan() {
                continue;
            }
            let (pa, pb, pc) = (at(mesh.pos, a), at(mesh.pos, b), at(mesh.pos, c));
            if cut([0, 1, 2].map(|k| (pa[k] + pb[k] + pc[k]) / 3.0)) {
                continue;
            }
            let x0 = sx[a].min(sx[b]).min(sx[c]).floor().max(0.0) as i64;
            let x1 = (sx[a].max(sx[b]).max(sx[c]).ceil() as i64).min(w as i64 - 1);
            let y0 = sy[a].min(sy[b]).min(sy[c]).floor().max(0.0) as i64;
            let y1 = (sy[a].max(sy[b]).max(sy[c]).ceil() as i64).min(h as i64 - 1);
            if x0 > x1 || y0 > y1 {
                continue;
            }
            let area = (sx[b] - sx[a]) * (sy[c] - sy[a]) - (sx[c] - sx[a]) * (sy[b] - sy[a]);
            if area.abs() < 1e-12 {
                continue;
            }
            let ia = 1.0 / area;
            for py in y0..=y1 {
                let qy = py as f64 + 0.5;
                for px in x0..=x1 {
                    let qx = px as f64 + 0.5;
                    let w0 = ((sx[b] - qx) * (sy[c] - qy) - (sx[c] - qx) * (sy[b] - qy)) * ia;
                    let w1 = ((sx[c] - qx) * (sy[a] - qy) - (sx[a] - qx) * (sy[c] - qy)) * ia;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < -1e-6 || w1 < -1e-6 || w2 < -1e-6 {
                        continue;
                    }
                    let d = w0 * sz[a] + w1 * sz[b] + w2 * sz[c];
                    let k = py as usize * w + px as usize;
                    if d < buf[k] {
                        buf[k] = d;
                    }
                }
            }
        }
    }
    let per_px = if s.m.ortho { 2.0 * s.m.half_h / s.h } else { 2.0 * s.m.tanh * s.dist / s.h };
    ZBuf { w, h, s: step, buf, tol: (3.0 * step * per_px).max(0.002) }
}

fn visible(z: &ZBuf, s: &Shot, p: V3) -> bool {
    let Some((x, y)) = project(s, p) else { return false };
    let (px, py) = (((x - s.x) / z.s).floor(), ((y - s.y) / z.s).floor());
    if px < 0.0 || py < 0.0 || px as usize >= z.w || py as usize >= z.h {
        return false;
    }
    dot(s.m.f, p) <= z.buf[py as usize * z.w + px as usize] + z.tol
}

#[derive(Default)]
struct Acc(Vec<(i64, Vec<f64>)>);

impl Acc {
    fn push(&mut self, k: i64, v: f64) {
        match self.0.iter_mut().find(|e| e.0 == k) {
            Some(e) => e.1.push(v),
            None => self.0.push((k, vec![v])),
        }
    }
}

fn median(a: &[f64]) -> f64 {
    let mut s = a.to_vec();
    s.sort_by(f64::total_cmp);
    s[s.len() >> 1]
}

#[derive(Clone)]
struct Item {
    k: i64,
    text: String,
    at: f64,
    n: usize,
    w: f64,
}

fn pick(m: &Acc, gap: &dyn Fn(&str) -> f64, lo: f64, hi: f64, name: &dyn Fn(i64) -> String) -> Vec<Item> {
    let mut items: Vec<Item> = m
        .0
        .iter()
        .filter(|(_, a)| a.len() >= 3)
        .map(|(k, a)| Item { k: *k, text: name(*k), at: median(a), n: a.len(), w: 0.0 })
        .filter(|it| it.at > lo && it.at < hi)
        .collect();
    items.sort_by(|a, b| b.n.cmp(&a.n));
    let mut kept: Vec<Item> = Vec::new();
    for mut it in items {
        let w = gap(&it.text);
        if !kept.iter().any(|q| (q.at - it.at).abs() < (q.w + w) / 2.0 + 2.0) {
            it.w = w;
            kept.push(it);
        }
    }
    kept
}

fn monotone(list: &[Item], sign: i64, unwrap: &dyn Fn(i64) -> i64) -> (Vec<Item>, usize) {
    let mut sorted = list.to_vec();
    sorted.sort_by(|a, b| a.at.total_cmp(&b.at));
    let mut dp = vec![0usize; sorted.len()];
    let mut prev = vec![-1i64; sorted.len()];
    for i in 0..sorted.len() {
        dp[i] = sorted[i].n;
        for j in 0..i {
            if sign * unwrap(sorted[j].k) < sign * unwrap(sorted[i].k) && dp[j] + sorted[i].n > dp[i] {
                dp[i] = dp[j] + sorted[i].n;
                prev[i] = j as i64;
            }
        }
    }
    let Some(best) = dp.iter().copied().max() else { return (Vec::new(), 0) };
    let mut keep = Vec::new();
    let mut i = dp.iter().position(|&d| d == best).map_or(-1, |i| i as i64);
    while i >= 0 {
        keep.insert(0, sorted[i as usize].clone());
        i = prev[i as usize];
    }
    (keep, best)
}

pub struct Head<'a> {
    pub pos: &'a [f32],
    pub nrm: &'a [f32],
}

#[derive(Default)]
pub struct Ruler {
    pub minor: Vec<V3>,
    pub major: Vec<V3>,
    pub zero: Vec<V3>,
    pub back: Vec<V3>,
    pub labels: Vec<Label>,
    pub caption: Option<(f64, f64, String)>,
}

fn axis_of(v: V3) -> Option<(usize, f64)> {
    let mut k = 0;
    for i in 1..3 {
        if v[i].abs() > v[k].abs() {
            k = i;
        }
    }
    (v[k].abs() > 0.999).then_some((k, v[k].signum()))
}

pub fn ruler(m: &Matrices, target: V3, s: &Shot, bounds: &crate::viewer::Bounds, labels_on: bool) -> Ruler {
    let mut out = Ruler::default();
    let name = |k: usize| ["x", "y", "z"][k];
    if m.ortho {
        let (hh, hw) = (m.half_h, m.half_h * s.w / s.h.max(1.0));
        let px_per_mm = s.h / (2.0 * hh * 1000.0);
        let major = if px_per_mm * 10.0 >= 9.0 { 10 } else if px_per_mm * 50.0 >= 9.0 { 50 } else { 100 };
        let minor = if px_per_mm >= 6.0 { 1 } else if px_per_mm * 2.0 >= 6.0 { 2 } else if px_per_mm * 5.0 >= 6.0 && major == 10 { 5 } else { 0 };
        let every = |gap: f64| [10, 20, 50, 100, 200, 500].into_iter().find(|&st| st % major == 0 && st as f64 * px_per_mm >= gap).unwrap_or(1000);
        let (step_v, step_h) = (every(44.0), every(18.0));
        let (sa, ua) = (axis_of(m.s), axis_of(m.u));
        let mut lines = |axis: Option<(usize, f64)>, dir: V3, other: V3, half: f64, other_half: f64, vertical: bool| {
            let Some((k, sign)) = axis else { return };
            let (lo, hi) = (target[k] - half, target[k] + half);
            let step = if minor > 0 { minor } else { major };
            let mut mm = (lo * 1000.0 / step as f64).ceil() as i64 * step;
            while mm as f64 <= hi * 1000.0 {
                let t = (mm as f64 / 1000.0 - target[k]) * sign;
                let base = crate::viewer::add(target, mul(dir, t));
                let seg = [crate::viewer::add(base, mul(other, -other_half * 1.05)), crate::viewer::add(base, mul(other, other_half * 1.05))];
                let kind = if mm == 0 { 0 } else if mm % major == 0 { 1 } else { 2 };
                match kind {
                    0 => out.zero.extend(seg),
                    1 => out.major.extend(seg),
                    _ => out.minor.extend(seg),
                }
                if kind != 2 && labels_on && mm % if vertical { step_v } else { step_h } == 0 {
                    let f = (t / if vertical { hw } else { hh } + 1.0) / 2.0;
                    let (x, y) = if vertical { (s.x + f * s.w, s.y + s.h - 14.0) } else { (s.x + 4.0, s.y + (1.0 - f) * s.h) };
                    out.labels.push(Label { x, y, text: mm.to_string(), edge: vertical });
                }
                mm += step;
            }
        };
        lines(sa, m.s, m.u, hw, hh, true);
        lines(ua, m.u, m.s, hh, hw, false);
        out.caption = Some(match (sa, ua) {
            (Some((a, _)), Some((b, _))) => (s.x + s.w - 92.0, s.y + s.h - 30.0, format!("{} → · {} ↑ · mm", name(a), name(b))),
            _ => (s.x + s.w - 160.0, s.y + s.h - 30.0, crate::i18n::tr("viewer.mm_grid_hint").to_string()),
        });
        return out;
    }
    let f = m.f;
    let mut k = 0;
    for i in 1..3 {
        if f[i].abs() > f[k].abs() {
            k = i;
        }
    }
    let plane = if f[k] > 0.0 { bounds.hi[k] + 0.02 } else { bounds.lo[k] - 0.02 };
    let others: Vec<usize> = (0..3).filter(|&i| i != k).collect();
    let ext = bounds.radius * 1.3;
    for &a in &others {
        let o2 = *others.iter().find(|&&i| i != a).expect("two others");
        let mut mm = ((bounds.centre[a] - ext) * 1000.0 / 10.0).ceil() as i64 * 10;
        while mm as f64 <= (bounds.centre[a] + ext) * 1000.0 {
            let (mut p, mut q) = ([0.0; 3], [0.0; 3]);
            p[k] = plane;
            q[k] = plane;
            p[a] = mm as f64 / 1000.0;
            q[a] = p[a];
            p[o2] = bounds.centre[o2] - ext;
            q[o2] = bounds.centre[o2] + ext;
            out.back.extend([p, q]);
            mm += 10;
        }
    }
    out.caption = Some((s.x + s.w - 150.0, s.y + s.h - 30.0, crate::i18n::trf("viewer.back_plane", &[("axis", &name(k))])));
    out
}

impl crate::studio::Studio<'_> {
    pub fn ruler_paint(&mut self, painter: &eframe::egui::Painter, rect: eframe::egui::Rect, ppp: f32, w: u32, h: u32) {
        use eframe::egui;
        let st = &self.world.state;
        if !st.opts.normal_grid {
            return;
        }
        let dark = st.opts.dark;
        let (muted, ink, chip) = if dark {
            (egui::Color32::from_gray(150), egui::Color32::from_gray(225), egui::Color32::from_rgba_unmultiplied(40, 42, 48, 210))
        } else {
            (egui::Color32::from_gray(95), egui::Color32::from_gray(30), egui::Color32::from_rgba_unmultiplied(232, 232, 228, 220))
        };
        let overlays: Vec<Boxed> = self.s.overlay_rects.iter().map(|q| Boxed { x0: (q.min.x - rect.min.x) as f64, y0: (q.min.y - rect.min.y) as f64, x1: (q.max.x - rect.min.x) as f64, y1: (q.max.y - rect.min.y) as f64 }).collect();
        for r in crate::viewer::rects(st.layout, w, h) {
            let cam = &st.cams[r.cam];
            let m = cam.matrices(r.w as f64 / r.h.max(1) as f64);
            let shot = Shot { m: &m, x: r.x as f64 / ppp as f64, y: r.y as f64 / ppp as f64, w: r.w as f64 / ppp as f64, h: r.h as f64 / ppp as f64, dist: cam.cur.dist };
            let rl = ruler(&m, cam.cur.target, &shot, &st.bounds, st.opts.grid_labels);
            let mut obstacles = vec![Boxed { x0: shot.x, y0: shot.y, x1: shot.x + 280.0, y1: shot.y + 28.0 }];
            obstacles.extend(overlays.iter().copied());
            for c in self.s.cubes.iter().filter(|c| c.slot == r.slot) {
                let half = c.radius as f64 * 1.65;
                let (cx, cy) = ((c.centre.x - rect.min.x) as f64, (c.centre.y - rect.min.y) as f64);
                obstacles.push(Boxed { x0: cx - half, y0: cy - half, x1: cx + half, y1: (c.home.max.y - rect.min.y) as f64 + 4.0 });
            }
            let clip = egui::Rect::from_min_size(rect.min + egui::vec2(shot.x as f32, shot.y as f32), egui::vec2(shot.w as f32, shot.h as f32));
            let p = painter.with_clip_rect(clip);
            let chip_at = |x: f64, y: f64, text: &str, colour: egui::Color32, centred: bool| {
                let galley = p.layout_no_wrap(text.to_string(), egui::FontId::monospace(10.0), colour);
                let mut at = rect.min + egui::vec2(x as f32, y as f32);
                if centred {
                    at.y -= galley.size().y / 2.0;
                }
                p.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(3.0, 1.0)), 2.0, chip);
                p.galley(at, galley, colour);
            };
            for l in &rl.labels {
                let wd = l.text.chars().count() as f64 * 6.2 + 8.0;
                let b = Boxed { x0: l.x, x1: l.x + wd, y0: l.y - 7.0, y1: l.y + 7.0 };
                if !obstacles.iter().any(|q| b.hits(q)) {
                    chip_at(l.x, l.y, &l.text, muted, true);
                }
            }
            if let Some((x, y, text)) = &rl.caption {
                let wd = text.chars().count() as f64 * 6.2 + 8.0;
                let b = Boxed { x0: *x, x1: x + wd, y0: *y, y1: y + 14.0 };
                if !obstacles.iter().any(|q| b.hits(q)) {
                    chip_at(*x, *y, text, ink, false);
                }
            }
        }
    }

    pub fn grid_paint(&mut self, ctx: &eframe::egui::Context, painter: &eframe::egui::Painter, rect: eframe::egui::Rect, ppp: f32, w: u32, h: u32) {
        use eframe::egui;
        let st = &self.world.state;
        let (Some(g), Some(ax), Some(s)) = (&self.world.grid, &self.world.grid_axis, &self.world.session) else { return };
        if !st.opts.face_grid || !st.opts.grid_labels {
            return;
        }
        let rects = crate::viewer::rects(st.layout, w, h);
        let overlays: Vec<Boxed> = self
            .s
            .overlay_rects
            .iter()
            .map(|q| Boxed { x0: (q.min.x - rect.min.x).round() as f64, y0: (q.min.y - rect.min.y).round() as f64, x1: (q.max.x - rect.min.x).round() as f64, y1: (q.max.y - rect.min.y).round() as f64 })
            .collect();
        let key = format!(
            "{:?}|{:?}|{}|{:?}|{:?}|{}|{}|{:?}",
            rects.iter().map(|r| (r.x, r.y, r.w, r.h, r.cam)).collect::<Vec<_>>(),
            rects.iter().map(|r| { let c = &st.cams[r.cam].cur; (c.yaw, c.pitch, c.dist, c.target, c.fov, st.cams[r.cam].ortho, st.vp[r.slot].lod) }).collect::<Vec<_>>(),
            s.head.revision,
            st.opts.clip.iter().map(|c| (c.on, c.offset)).collect::<Vec<_>>(),
            (st.opts.clip_box.on, st.opts.eyes, st.opts.eyes_grid.as_str(), st.opts.normal_grid),
            self.world.extras.iter().filter(|e| e.visible).map(|e| e.key).sum::<u64>(),
            ppp,
            overlays.iter().map(|b| (b.x0, b.y0, b.x1, b.y1)).collect::<Vec<_>>()
        );
        if let Some((hp, slot, text)) = &self.world.hover
            && let Some(r) = rects.iter().find(|r| r.slot == *slot)
        {
            let cam = &st.cams[r.cam];
            let m = cam.matrices(r.w as f64 / r.h.max(1) as f64);
            let shot = Shot { m: &m, x: r.x as f64 / ppp as f64, y: r.y as f64 / ppp as f64, w: r.w as f64 / ppp as f64, h: r.h as f64 / ppp as f64, dist: cam.cur.dist };
            if let Some((x, y)) = project(&shot, *hp) {
                let galley = painter.layout_no_wrap(text.clone(), egui::FontId::monospace(11.0), egui::Color32::WHITE);
                let at = rect.min + egui::vec2(x as f32 + 12.0, y as f32 - 26.0);
                painter.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(5.0, 2.0)), 3.0, egui::Color32::from_rgb(0x0b, 0x5c, 0x7c));
                painter.galley(at, galley, egui::Color32::WHITE);
            }
        }
        let now = std::time::Instant::now();
        if self.s.grid_view.as_ref().is_none_or(|(k, _)| *k != key) {
            self.s.grid_view = Some((key.clone(), now));
        }
        let since = self.s.grid_view.as_ref().map_or(now, |v| v.1);
        if now.duration_since(since).as_millis() < 120 {
            ctx.request_repaint_after(std::time::Duration::from_millis(130));
            return;
        }
        if self.s.grid_labels.as_ref().is_none_or(|(k, _)| *k != key) {
            let planes = crate::viewer::planes(&st.opts, &st.bounds);
            let (blo, bhi) = crate::viewer::box_world(&st.opts, &st.bounds);
            let boxed = st.opts.clip_box.on;
            let cut = |p: V3| planes.iter().any(|q| dot(q.n, p) > q.d) || (boxed && (0..3).all(|k| p[k] > blo[k] && p[k] < bhi[k]));
            let mut all = Vec::new();
            for r in &rects {
                let lod = st.vp[r.slot].lod.min(s.head.lods.len().saturating_sub(1));
                let mut meshes = vec![Surface { pos: &s.head.positions, idx: &s.head.lods[lod].indices }];
                if let Some(e) = s.head.eyes.as_ref().filter(|_| st.opts.eyes && st.opts.eyes_grid != "hide") {
                    meshes.push(Surface { pos: &e.positions, idx: &e.indices });
                }
                for x in self.world.extras.iter().filter(|e| e.visible) {
                    meshes.push(Surface { pos: &x.positions, idx: &x.indices });
                }
                let cam = &st.cams[r.cam];
                let m = cam.matrices(r.w as f64 / r.h.max(1) as f64);
                let shot = Shot { m: &m, x: r.x as f64 / ppp as f64, y: r.y as f64 / ppp as f64, w: r.w as f64 / ppp as f64, h: r.h as f64 / ppp as f64, dist: cam.cur.dist };
                let z = zbuf(&shot, &meshes, &cut);
                let mut obstacles = vec![Boxed { x0: shot.x, y0: shot.y, x1: shot.x + 280.0, y1: shot.y + 28.0 }];
                obstacles.extend(overlays.iter().copied());
                for c in self.s.cubes.iter().filter(|c| c.slot == r.slot) {
                    let half = c.radius as f64 * 1.65;
                    let o = |p: egui::Pos2| ((p.x - rect.min.x) as f64, (p.y - rect.min.y) as f64);
                    let (cx, cy) = o(c.centre);
                    let (_, by) = o(c.home.max);
                    obstacles.push(Boxed { x0: cx - half, y0: cy - half, x1: cx + half, y1: by.max(cy + half) + 4.0 });
                }
                let ruler_box = st
                    .opts
                    .normal_grid
                    .then(|| ruler(&m, cam.cur.target, &shot, &st.bounds, true).caption)
                    .flatten()
                    .map(|(x, y, t)| Boxed { x0: x, x1: x + t.chars().count() as f64 * 6.2 + 8.0, y0: y, y1: y + 14.0 })
                    .filter(|b| !obstacles.iter().any(|q| b.hits(q)));
                let how = Placing { obstacles: &obstacles, normal_grid: st.opts.normal_grid, ruler: ruler_box };
                all.push(place(g, ax, &Head { pos: &s.head.positions, nrm: &s.head.normals }, &shot, &z, &cut, &how));
            }
            self.s.grid_labels = Some((key, all));
        }
        let Some((_, all)) = &self.s.grid_labels else { return };
        let dark = st.opts.dark;
        for (r, labels) in rects.iter().zip(all) {
            let clip = egui::Rect::from_min_size(rect.min + egui::vec2(r.x as f32 / ppp, r.y as f32 / ppp), egui::vec2(r.w as f32 / ppp, r.h as f32 / ppp));
            let p = painter.with_clip_rect(clip);
            for l in labels {
                let (ink, bg) = match (dark, l.edge) {
                    (true, true) => (egui::Color32::from_rgb(0x8f, 0xf5, 0xe6), egui::Color32::from_rgba_unmultiplied(10, 20, 24, 217)),
                    (true, false) => (egui::Color32::from_rgb(0xc8, 0xff, 0xf4), egui::Color32::from_rgba_unmultiplied(10, 20, 24, 179)),
                    (false, true) => (egui::Color32::from_rgb(0x0b, 0x5c, 0x7c), egui::Color32::from_rgba_unmultiplied(255, 255, 255, 217)),
                    (false, false) => (egui::Color32::from_rgb(0x04, 0x38, 0x4f), egui::Color32::from_rgba_unmultiplied(255, 255, 255, 184)),
                };
                let galley = p.layout_no_wrap(l.text.clone(), egui::FontId::monospace(10.0), ink);
                let centre = rect.min + egui::vec2(l.x as f32, l.y as f32);
                let at = centre - galley.size() / 2.0;
                p.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(3.0, 1.0)), 3.0, bg);
                p.galley(at, galley, ink);
            }
        }
    }
}

pub struct Placing<'a> {
    pub obstacles: &'a [Boxed],
    pub normal_grid: bool,
    pub ruler: Option<Boxed>,
}

pub fn place(g: &FaceGrid, ax: &Axis, head: &Head, s: &Shot, z: &ZBuf, cut: &dyn Fn(V3) -> bool, how: &Placing) -> Vec<Label> {
    let mut out = Vec::new();
    let obstacles = how.obstacles;
    let blocked = |b: &Boxed| obstacles.iter().any(|q| b.hits(q));
    let on_ruler = |b: &Boxed| how.ruler.is_some_and(|q| b.hits(&q));
    let normal_grid = how.normal_grid;
    let letters = &g.letters;
    let cols = letters.len().max(1) as i64;
    let ch = &g.uv;
    let n = head.pos.len() / 3;
    let mut seen: Vec<(usize, f64, f64)> = Vec::new();
    let (mut y_top, mut y_bot) = (f64::INFINITY, f64::NEG_INFINITY);
    for v in 0..n {
        if ax.vertex[v] {
            continue;
        }
        let p = at(head.pos, v);
        if facing(s.m, p, at(head.nrm, v)) < 0.1 || cut(p) {
            continue;
        }
        let Some((x, y)) = project(s, p) else { continue };
        if !visible(z, s, p) {
            continue;
        }
        seen.push((v, x, y));
        y_top = y_top.min(y);
        y_bot = y_bot.max(y);
    }
    let mid = (y_top + y_bot) / 2.0;
    let (mut col_up, mut col_down, mut col_all, mut row_acc) = (Acc::default(), Acc::default(), Acc::default(), Acc::default());
    for &(v, sx, sy) in &seen {
        let x = ch[2 * v] as f64 / g.column_deg + 0.5;
        let k = x.floor();
        if (x - k - 0.5).abs() < 0.3 {
            let c = ((k as i64 + ax.front) % cols + cols) % cols;
            if sy < mid { &mut col_up } else { &mut col_down }.push(c, sx);
            col_all.push(c, sx);
        }
        let y = (ch[2 * v + 1] as f64).max(0.0) / g.row_mm;
        let row = y.floor();
        if (y - row - 0.5).abs() < 0.3 {
            row_acc.push(row as i64 + 1, sy);
        }
    }
    let wide = |t: &str| t.chars().count() as f64 * 7.0 + 8.0;
    let letter = |k: i64| letters.get(k as usize).cloned().unwrap_or_default();
    let (lo, hi) = (s.x + 8.0, s.x + s.w - 8.0);
    let mut cols_up = pick(&col_up, &wide, lo, hi, &letter);
    let mut cols_down = pick(&col_down, &wide, lo, hi, &letter);
    let named: HashSet<i64> = cols_up.iter().chain(&cols_down).map(|it| it.k).collect();
    let rest = Acc(col_all.0.into_iter().filter(|(k, _)| !named.contains(k)).collect());
    for it in pick(&rest, &wide, lo, hi, &letter) {
        if !cols_up.iter().any(|q| (q.at - it.at).abs() < (q.w + it.w) / 2.0 + 2.0) {
            cols_up.push(it);
        }
    }
    let reference = cols_up.iter().chain(&cols_down).fold(None::<&Item>, |a, b| match a {
        Some(a) if a.n >= b.n => Some(a),
        _ => Some(b),
    });
    let r0 = reference.map_or(0, |it| it.k);
    let unwrap = |k: i64| {
        let mut d = ((k - r0) % cols + cols) % cols;
        if d >= cols / 2 {
            d -= cols;
        }
        d
    };
    let total = |sg: i64| monotone(&cols_up, sg, &unwrap).1 + monotone(&cols_down, sg, &unwrap).1;
    let dir = if total(-1) > total(1) { -1 } else { 1 };
    let best = |list: &[Item]| {
        let (a, ta) = monotone(list, dir, &unwrap);
        let (b, tb) = monotone(list, -dir, &unwrap);
        if tb > ta { b } else { a }
    };
    cols_up = best(&cols_up);
    cols_down = best(&cols_down);
    let rows_kept = pick(&row_acc, &|_| 13.0, s.y + 8.0, s.y + s.h - 8.0, &|k| k.to_string());
    let mut put = |it: &Item, x: f64, y: f64, at_bottom: bool| {
        let w = it.text.chars().count() as f64 * 7.0 + 8.0;
        let at = |yy: f64| Boxed { x0: x - w / 2.0, x1: x + w / 2.0, y0: yy - 7.0, y1: yy + 7.0 };
        let y = if at_bottom && on_ruler(&at(y)) { y - 18.0 } else { y };
        let b = at(y);
        if blocked(&b) || on_ruler(&b) {
            return;
        }
        out.push(Label { x, y, text: it.text.clone(), edge: true });
    };
    let bottom = s.y + s.h - if normal_grid { 32.0 } else { 14.0 };
    let top = s.y + 12.0;
    for it in &cols_up {
        put(it, it.at, top, false);
    }
    for it in &cols_down {
        put(it, it.at, bottom, true);
    }
    let mut rows_all: Vec<Item> = row_acc
        .0
        .iter()
        .filter(|(_, a)| a.len() >= 3)
        .map(|(k, a)| Item { k: *k, text: k.to_string(), at: median(a), n: a.len(), w: 13.0 })
        .filter(|it| it.at > s.y + 8.0 && it.at < s.y + s.h - 8.0)
        .collect();
    rows_all.sort_by(|a, b| a.at.total_cmp(&b.at));
    let gaps: Vec<f64> = rows_all.windows(2).map(|w| w[1].at - w[0].at).collect();
    let roomy = gaps.is_empty() || median(&gaps) >= 30.0;
    let side = |list: Vec<Item>| {
        let mut kept: Vec<Item> = Vec::new();
        for it in list {
            if !kept.iter().any(|q| (q.at - it.at).abs() < 15.0) {
                kept.push(it);
            }
        }
        kept
    };
    let left_free = !normal_grid || !s.m.ortho;
    let right = side(if roomy || !left_free { rows_kept.clone() } else { rows_all.iter().filter(|it| it.k % 2 == 0).cloned().collect() });
    let left = if left_free { side(if roomy { rows_kept } else { rows_all.iter().filter(|it| it.k % 2 == 1).cloned().collect() }) } else { Vec::new() };
    for it in &right {
        put(it, s.x + s.w - 14.0, it.at, false);
    }
    for it in &left {
        put(it, s.x + 16.0, it.at, false);
    }
    let mut cand: Vec<(f64, f64, f64, String)> = Vec::new();
    for (label, v) in &g.anchors {
        let v = *v;
        if v >= n || ax.vertex[v] {
            continue;
        }
        let p = at(head.pos, v);
        let f = facing(s.m, p, at(head.nrm, v));
        if f < 0.45 || cut(p) {
            continue;
        }
        let px_per_mm = if s.m.ortho { s.h / (2.0 * s.m.half_h * 1000.0) } else { s.h / (2.0 * s.m.tanh * dot(sub(p, s.m.eye), s.m.f).max(1e-4) * 1000.0) };
        if g.row_mm * px_per_mm * f < 56.0 {
            continue;
        }
        if let Some((x, y)) = project(s, p)
            && visible(z, s, p)
        {
            cand.push((f, x, y, label.clone()));
        }
    }
    cand.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut placed: Vec<(f64, f64, f64)> = Vec::new();
    for (_, x, y, text) in cand {
        let w = text.chars().count() as f64 * 6.5 + 8.0;
        let b = Boxed { x0: x - w / 2.0, x1: x + w / 2.0, y0: y - 7.0, y1: y + 7.0 };
        if blocked(&b) || placed.iter().any(|q| (q.0 - x).abs() < (q.2 + w) / 2.0 + 2.0 && (q.1 - y).abs() < 15.0) {
            continue;
        }
        placed.push((x, y, w));
        out.push(Label { x, y, text, edge: false });
    }
    out
}
