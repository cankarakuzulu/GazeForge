use std::collections::HashMap;
use std::sync::OnceLock;

use eframe::egui;

use crate::icon_data::ICONS;

#[derive(Clone, Debug)]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    pub closed: bool,
    pub filled: bool,
}

pub type Icon = Vec<Stroke>;

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(i) = rest.find(name) {
        let before = rest[..i].chars().last();
        let after = &rest[i + name.len()..];
        if before.is_none_or(|c| c.is_whitespace()) && after.starts_with("=\"") {
            let v = &after[2..];
            return v.find('"').map(|e| &v[..e]);
        }
        rest = &rest[i + name.len()..];
    }
    None
}

fn num(tag: &str, name: &str) -> f32 {
    attr(tag, name).and_then(|v| v.trim().parse().ok()).unwrap_or(0.0)
}

fn numbers(text: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut Vec<f32>| {
        if let Ok(v) = cur.parse() {
            out.push(v);
        }
        cur.clear();
    };
    for c in text.chars() {
        match c {
            '0'..='9' => cur.push(c),
            '.' if cur.contains('.') && !cur.contains(['e', 'E']) => {
                flush(&mut cur, &mut out);
                cur.push(c);
            }
            '.' => cur.push(c),
            '-' | '+' if !cur.is_empty() && !cur.ends_with(['e', 'E']) => {
                flush(&mut cur, &mut out);
                cur.push(c);
            }
            '-' | '+' | 'e' | 'E' => cur.push(c),
            _ => flush(&mut cur, &mut out),
        }
    }
    flush(&mut cur, &mut out);
    out
}

struct Cursor<'a> {
    b: &'a [u8],
    i: usize,
}

impl Cursor<'_> {
    fn skip(&mut self) {
        while self.i < self.b.len() && (self.b[self.i].is_ascii_whitespace() || self.b[self.i] == b',') {
            self.i += 1;
        }
    }

    fn command(&mut self) -> Option<u8> {
        self.skip();
        let c = *self.b.get(self.i)?;
        if c.is_ascii_alphabetic() && c != b'e' && c != b'E' {
            self.i += 1;
            Some(c)
        } else {
            None
        }
    }

    fn has_number(&mut self) -> bool {
        self.skip();
        self.b.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'-' || *c == b'+' || *c == b'.')
    }

    fn number(&mut self) -> f32 {
        self.skip();
        let start = self.i;
        let mut seen_dot = false;
        let mut seen_e = false;
        while self.i < self.b.len() {
            let c = self.b[self.i];
            let at_start = self.i == start;
            let after_e = self.i > start && matches!(self.b[self.i - 1], b'e' | b'E');
            let ok = c.is_ascii_digit() || ((c == b'-' || c == b'+') && (at_start || after_e)) || (c == b'.' && !seen_dot && !seen_e) || ((c == b'e' || c == b'E') && !seen_e && !at_start);
            if !ok {
                break;
            }
            seen_dot |= c == b'.';
            seen_e |= c == b'e' || c == b'E';
            self.i += 1;
        }
        std::str::from_utf8(&self.b[start..self.i]).ok().and_then(|s| s.parse().ok()).unwrap_or(0.0)
    }

    fn flag(&mut self) -> bool {
        self.skip();
        let c = self.b.get(self.i).copied();
        self.i += 1;
        c == Some(b'1')
    }
}

fn arc(out: &mut Vec<[f32; 2]>, p0: [f32; 2], r: [f32; 2], rot_deg: f32, large: bool, sweep: bool, p1: [f32; 2]) {
    let (mut rx, mut ry) = (r[0].abs() as f64, r[1].abs() as f64);
    if rx == 0.0 || ry == 0.0 {
        out.push(p1);
        return;
    }
    let phi = (rot_deg as f64).to_radians();
    let (c, s) = (phi.cos(), phi.sin());
    let (dx, dy) = ((p0[0] - p1[0]) as f64 / 2.0, (p0[1] - p1[1]) as f64 / 2.0);
    let (x1, y1) = (c * dx + s * dy, -s * dx + c * dy);
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut k = (num / den).max(0.0).sqrt();
    if large == sweep {
        k = -k;
    }
    let (cx1, cy1) = (k * rx * y1 / ry, -k * ry * x1 / rx);
    let (cx, cy) = (c * cx1 - s * cy1 + (p0[0] + p1[0]) as f64 / 2.0, s * cx1 + c * cy1 + (p0[1] + p1[1]) as f64 / 2.0);
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| (ux * vy - uy * vx).atan2(ux * vx + uy * vy);
    let t0 = angle(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut dt = angle((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
    if !sweep && dt > 0.0 {
        dt -= std::f64::consts::TAU;
    } else if sweep && dt < 0.0 {
        dt += std::f64::consts::TAU;
    }
    let n = ((dt.abs() / (std::f64::consts::PI / 12.0)).ceil() as usize).max(2);
    for i in 1..=n {
        let t = t0 + dt * i as f64 / n as f64;
        let (x, y) = (rx * t.cos(), ry * t.sin());
        out.push([(c * x - s * y + cx) as f32, (s * x + c * y + cy) as f32]);
    }
}

fn cubic(out: &mut Vec<[f32; 2]>, p0: [f32; 2], c1: [f32; 2], c2: [f32; 2], p1: [f32; 2]) {
    let n = 10;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
        out.push([0, 1].map(|k| w[0] * p0[k] + w[1] * c1[k] + w[2] * c2[k] + w[3] * p1[k]));
    }
}

pub fn parse_path(d: &str, filled: bool) -> Vec<Stroke> {
    let mut cur = Cursor { b: d.as_bytes(), i: 0 };
    let mut out: Vec<Stroke> = Vec::new();
    let mut pts: Vec<[f32; 2]> = Vec::new();
    let (mut p, mut start, mut last_ctrl): ([f32; 2], [f32; 2], Option<[f32; 2]>) = ([0.0; 2], [0.0; 2], None);
    let mut cmd = b'M';
    let finish = |pts: &mut Vec<[f32; 2]>, out: &mut Vec<Stroke>, closed: bool| {
        if pts.len() > 1 {
            out.push(Stroke { points: std::mem::take(pts), closed, filled });
        } else {
            pts.clear();
        }
    };
    loop {
        if let Some(c) = cur.command() {
            cmd = c;
        } else if !cur.has_number() {
            break;
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { p } else { [0.0, 0.0] };
        let pt = |cur: &mut Cursor, base: [f32; 2]| {
            let x = cur.number();
            let y = cur.number();
            [x + base[0], y + base[1]]
        };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                finish(&mut pts, &mut out, false);
                p = pt(&mut cur, base);
                start = p;
                pts.push(p);
                cmd = if rel { b'l' } else { b'L' };
                last_ctrl = None;
            }
            b'L' => {
                p = pt(&mut cur, base);
                pts.push(p);
                last_ctrl = None;
            }
            b'H' => {
                p = [cur.number() + if rel { p[0] } else { 0.0 }, p[1]];
                pts.push(p);
                last_ctrl = None;
            }
            b'V' => {
                p = [p[0], cur.number() + if rel { p[1] } else { 0.0 }];
                pts.push(p);
                last_ctrl = None;
            }
            b'C' => {
                let c1 = pt(&mut cur, base);
                let c2 = pt(&mut cur, base);
                let e = pt(&mut cur, base);
                cubic(&mut pts, p, c1, c2, e);
                last_ctrl = Some(c2);
                p = e;
            }
            b'S' => {
                let c1 = last_ctrl.map_or(p, |c| [2.0 * p[0] - c[0], 2.0 * p[1] - c[1]]);
                let c2 = pt(&mut cur, base);
                let e = pt(&mut cur, base);
                cubic(&mut pts, p, c1, c2, e);
                last_ctrl = Some(c2);
                p = e;
            }
            b'Q' | b'T' => {
                let q = if cmd.eq_ignore_ascii_case(&b'Q') { pt(&mut cur, base) } else { last_ctrl.map_or(p, |c| [2.0 * p[0] - c[0], 2.0 * p[1] - c[1]]) };
                let e = pt(&mut cur, base);
                let c1 = [p[0] + 2.0 / 3.0 * (q[0] - p[0]), p[1] + 2.0 / 3.0 * (q[1] - p[1])];
                let c2 = [e[0] + 2.0 / 3.0 * (q[0] - e[0]), e[1] + 2.0 / 3.0 * (q[1] - e[1])];
                cubic(&mut pts, p, c1, c2, e);
                last_ctrl = Some(q);
                p = e;
            }
            b'A' => {
                let r = [cur.number(), cur.number()];
                let rot = cur.number();
                let large = cur.flag();
                let sweep = cur.flag();
                let e = pt(&mut cur, base);
                arc(&mut pts, p, r, rot, large, sweep, e);
                p = e;
                last_ctrl = None;
            }
            b'Z' => {
                p = start;
                finish(&mut pts, &mut out, true);
                pts.push(p);
                last_ctrl = None;
                if cur.has_number() {
                    break;
                }
            }
            _ => break,
        }
    }
    finish(&mut pts, &mut out, false);
    out
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32, filled: bool) -> Stroke {
    let n = 32;
    let points = (0..n).map(|i| {
        let t = i as f32 / n as f32 * std::f32::consts::TAU;
        [cx + rx * t.cos(), cy + ry * t.sin()]
    }).collect();
    Stroke { points, closed: true, filled }
}

pub fn parse(svg: &str) -> Icon {
    let mut out = Vec::new();
    let body = svg.find("<svg").map_or(svg, |i| &svg[i + 4..]);
    for piece in body.split('<').skip(1) {
        let tag = piece.split('>').next().unwrap_or("");
        let name = tag.split_whitespace().next().unwrap_or("");
        let filled = attr(tag, "fill").is_some_and(|f| f != "none");
        match name {
            "path" => out.extend(parse_path(attr(tag, "d").unwrap_or(""), filled)),
            "circle" => out.push(ellipse(num(tag, "cx"), num(tag, "cy"), num(tag, "r"), num(tag, "r"), filled)),
            "ellipse" => out.push(ellipse(num(tag, "cx"), num(tag, "cy"), num(tag, "rx"), num(tag, "ry"), filled)),
            "line" => out.push(Stroke { points: vec![[num(tag, "x1"), num(tag, "y1")], [num(tag, "x2"), num(tag, "y2")]], closed: false, filled: false }),
            "polyline" | "polygon" => {
                let v = numbers(attr(tag, "points").unwrap_or(""));
                let points = v.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
                out.push(Stroke { points, closed: name == "polygon", filled });
            }
            "rect" => {
                let (x, y, w, h) = (num(tag, "x"), num(tag, "y"), num(tag, "width"), num(tag, "height"));
                let rx = attr(tag, "rx").and_then(|v| v.parse().ok()).or_else(|| attr(tag, "ry").and_then(|v| v.parse().ok())).unwrap_or(0.0f32).min(w / 2.0).min(h / 2.0);
                let d = if rx > 0.0 {
                    format!("M{} {}H{}A{rx} {rx} 0 0 1 {} {}V{}A{rx} {rx} 0 0 1 {} {}H{}A{rx} {rx} 0 0 1 {} {}V{}A{rx} {rx} 0 0 1 {} {}Z",
                        x + rx, y, x + w - rx, x + w, y + rx, y + h - rx, x + w - rx, y + h, x + rx, x, y + h - rx, y + rx, x + rx, y)
                } else {
                    format!("M{x} {y}H{}V{}H{x}Z", x + w, y + h)
                };
                out.extend(parse_path(&d, filled));
            }
            _ => {}
        }
    }
    out
}

pub fn get(name: &str) -> Option<&'static Icon> {
    static TABLE: OnceLock<HashMap<&'static str, Icon>> = OnceLock::new();
    TABLE.get_or_init(|| ICONS.iter().map(|(n, svg)| (*n, parse(svg))).collect()).get(name)
}

pub fn paint(painter: &egui::Painter, rect: egui::Rect, name: &str, color: egui::Color32) {
    let Some(icon) = get(name) else {
        painter.rect_stroke(rect.shrink(2.0), 2.0, egui::Stroke::new(1.0, color), egui::StrokeKind::Inside);
        return;
    };
    let scale = rect.width().min(rect.height()) / 24.0;
    let origin = rect.center() - egui::vec2(12.0, 12.0) * scale;
    let width = (2.0 * scale).max(1.0);
    for s in icon {
        let pts: Vec<egui::Pos2> = s.points.iter().map(|p| origin + egui::vec2(p[0], p[1]) * scale).collect();
        if s.filled {
            painter.add(egui::Shape::convex_polygon(pts.clone(), color, egui::Stroke::NONE));
        }
        let stroke = egui::epaint::PathStroke::new(width, color);
        painter.add(egui::Shape::Path(egui::epaint::PathShape { points: pts, closed: s.closed, fill: egui::Color32::TRANSPARENT, stroke }));
    }
}

pub fn button(ui: &mut egui::Ui, name: &str, on: bool, tip: &str) -> egui::Response {
    let size = egui::vec2(28.0, 26.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let v = ui.visuals();
    let fill = if on {
        v.selection.bg_fill
    } else if resp.hovered() {
        v.widgets.hovered.weak_bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    let ink = if on { v.selection.stroke.color } else if resp.hovered() { v.widgets.hovered.fg_stroke.color } else { v.widgets.inactive.fg_stroke.color };
    ui.painter().rect_filled(rect, 4.0, fill);
    paint(ui.painter(), egui::Rect::from_center_size(rect.center(), egui::vec2(17.0, 17.0)), name, ink);
    resp.on_hover_text(tip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_icon_parses_to_strokes_inside_the_box() {
        for (name, svg) in ICONS {
            let icon = parse(svg);
            assert!(!icon.is_empty(), "{name} parsed to nothing");
            for s in &icon {
                assert!(s.points.len() >= 2, "{name} has a one-point stroke");
                for p in &s.points {
                    assert!(p[0] >= -0.5 && p[0] <= 24.5 && p[1] >= -0.5 && p[1] <= 24.5, "{name} point {p:?} outside 24x24");
                }
            }
        }
    }

    #[test]
    fn path_commands_and_compact_arc_flags() {
        let s = parse_path("M3 3h4v4H3z", false);
        assert_eq!(s.len(), 1);
        assert!(s[0].closed);
        assert_eq!(s[0].points, vec![[3.0, 3.0], [7.0, 3.0], [7.0, 7.0], [3.0, 7.0]]);
        let a = parse_path("M17.055 4.533a24 24 0 00-10.11 0", false);
        let last = *a[0].points.last().unwrap();
        assert!((last[0] - 6.945).abs() < 1e-3 && (last[1] - 4.533).abs() < 1e-3, "{last:?}");
        let mid = a[0].points[a[0].points.len() / 2];
        assert!(mid[1] < 4.533, "the arc bows upward: {mid:?}");
        let r = parse_path("m12.828 4.813 5.344 2.375", false);
        assert!((r[0].points[1][0] - 18.172).abs() < 1e-4 && (r[0].points[1][1] - 7.188).abs() < 1e-4);
    }

    #[test]
    fn the_page_sprite_carries_the_same_icons() {
        let html = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/index.html")).expect("web/index.html");
        for (name, svg) in ICONS {
            let body = svg.split_once('>').and_then(|_| svg.find("<svg")).map(|i| &svg[i..]).unwrap_or(svg);
            let inner = body.split_once('>').map(|x| x.1).unwrap_or("").rsplit_once("</svg>").map(|x| x.0).unwrap_or("");
            let flat: Vec<&str> = inner.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            let want = format!("<symbol id=\"i-{name}\" viewBox=\"0 0 24 24\">{}</symbol>", flat.join(" "));
            assert!(html.contains(&want), "web/index.html sprite differs for {name}; run tools/icons/build_icons.py");
        }
    }
}
