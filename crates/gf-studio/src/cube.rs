use eframe::egui;

use crate::viewer::{Basis, V3, add, dot, mul, norm};

pub const FACES: [(&str, V3, V3, V3); 6] = [
    ("front", [0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ("back", [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
    ("top", [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ("bottom", [0.0, 0.0, -1.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]),
    ("left", [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    ("right", [-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
];
pub const BORDER: f64 = 0.6;
const LABELS: [&str; 6] = ["FRONT", "BACK", "TOP", "BOTTOM", "LEFT", "RIGHT"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Element(pub u8);

fn face_of(n: V3) -> usize {
    FACES.iter().position(|f| dot(f.1, n) > 0.5).expect("an axis direction")
}

impl Element {
    pub fn faces(self) -> Vec<usize> {
        (0..6).filter(|i| self.0 & (1 << i) != 0).collect()
    }

    pub fn name(self) -> String {
        self.faces().iter().map(|&i| FACES[i].0).collect::<Vec<_>>().join("-")
    }

    pub fn parse(name: &str) -> Option<Element> {
        let mut bits = 0u8;
        for part in name.split('-') {
            bits |= 1 << FACES.iter().position(|f| f.0 == part)?;
        }
        let e = Element(bits);
        let fs = e.faces();
        let opposite = fs.iter().any(|&a| fs.iter().any(|&b| dot(FACES[a].1, FACES[b].1) < -0.5));
        (!fs.is_empty() && fs.len() <= 3 && !opposite).then_some(e)
    }

    pub fn dir(self) -> V3 {
        norm(self.faces().iter().fold([0.0; 3], |a, &i| add(a, FACES[i].1)))
    }

    pub fn is_face(self) -> bool {
        self.0.count_ones() == 1
    }

    #[cfg(test)]
    pub fn all() -> Vec<Element> {
        let mut out = Vec::new();
        for bits in 1u8..64 {
            if let Some(e) = Element::parse(&Element(bits).name())
                && e.0 == bits
            {
                out.push(e);
            }
        }
        out
    }
}

pub fn yaw_pitch(dir: V3) -> (f64, f64) {
    let pitch = dir[2].clamp(-1.0, 1.0).asin();
    let flat = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt();
    let yaw = if flat < 1e-9 { 0.0 } else { dir[0].atan2(-dir[1]) };
    (yaw, pitch)
}

pub struct Cube {
    pub centre: egui::Pos2,
    pub r: f32,
    pub s: V3,
    pub u: V3,
    pub back: V3,
}

impl Cube {
    pub fn new(centre: egui::Pos2, r: f32, b: &Basis) -> Cube {
        Cube { centre, r, s: b.s, u: b.u, back: b.back }
    }

    pub fn screen(&self, p: V3) -> egui::Pos2 {
        self.centre + egui::vec2(dot(p, self.s) as f32, -dot(p, self.u) as f32) * self.r
    }

    fn local(&self, face: usize, x: f64, y: f64) -> V3 {
        let (_, n, a, b) = FACES[face];
        add(n, add(mul(a, x), mul(b, y)))
    }

    pub fn facing(&self, face: usize) -> f64 {
        dot(FACES[face].1, self.back)
    }

    pub fn visible(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..6).filter(|&f| self.facing(f) > 0.02).collect();
        v.sort_by(|a, b| self.facing(*a).total_cmp(&self.facing(*b)));
        v
    }

    pub fn pick(&self, q: egui::Pos2) -> Option<Element> {
        for f in self.visible() {
            let (_, _, a, b) = FACES[f];
            let o = self.screen(self.local(f, 0.0, 0.0));
            let (sa, sb) = (self.screen(a) - self.centre, self.screen(b) - self.centre);
            let d = q - o;
            let det = sa.x * sb.y - sa.y * sb.x;
            if det.abs() < 1e-6 {
                continue;
            }
            let x = ((d.x * sb.y - d.y * sb.x) / det) as f64;
            let y = ((sa.x * d.y - sa.y * d.x) / det) as f64;
            if x.abs() > 1.0 || y.abs() > 1.0 {
                continue;
            }
            let mut bits = 1u8 << f;
            if x.abs() > BORDER {
                bits |= 1 << face_of(mul(a, x.signum()));
            }
            if y.abs() > BORDER {
                bits |= 1 << face_of(mul(b, y.signum()));
            }
            return Some(Element(bits));
        }
        None
    }

    fn region(e: Element, face: usize) -> Option<[f64; 4]> {
        let (_, _, a, b) = FACES[face];
        let mut want = e.0 & !(1 << face);
        if e.0 & (1 << face) == 0 {
            return None;
        }
        let span = |axis: V3, want: &mut u8| {
            let (p, m) = (face_of(axis), face_of(mul(axis, -1.0)));
            if *want & (1 << p) != 0 {
                *want &= !(1 << p);
                [BORDER, 1.0]
            } else if *want & (1 << m) != 0 {
                *want &= !(1 << m);
                [-1.0, -BORDER]
            } else {
                [-BORDER, BORDER]
            }
        };
        let xs = span(a, &mut want);
        let ys = span(b, &mut want);
        (want == 0).then_some([xs[0], xs[1], ys[0], ys[1]])
    }

    pub fn point_of(&self, e: Element) -> Option<egui::Pos2> {
        let mut best: Option<(f64, egui::Pos2)> = None;
        for f in self.visible() {
            if let Some([x0, x1, y0, y1]) = Self::region(e, f) {
                let p = self.screen(self.local(f, (x0 + x1) / 2.0, (y0 + y1) / 2.0));
                let facing = self.facing(f);
                if facing > 0.15 && best.is_none_or(|b| facing > b.0) {
                    best = Some((facing, p));
                }
            }
        }
        best.map(|b| b.1)
    }

    pub fn bounds(&self) -> egui::Rect {
        egui::Rect::from_center_size(self.centre, egui::vec2(self.r, self.r) * 2.0 * 1.75)
    }

    fn quad(&self, f: usize, r: [f64; 4]) -> Vec<egui::Pos2> {
        [(r[0], r[2]), (r[1], r[2]), (r[1], r[3]), (r[0], r[3])].iter().map(|&(x, y)| self.screen(self.local(f, x, y))).collect()
    }

    pub fn paint(&self, ui: &egui::Ui, painter: &egui::Painter, hover: Option<Element>, dark: bool) {
        let (fill, edge, ink, hi) = if dark {
            (egui::Color32::from_rgb(58, 62, 70), egui::Color32::from_rgb(120, 128, 140), egui::Color32::from_rgb(225, 228, 235), egui::Color32::from_rgb(70, 130, 230))
        } else {
            (egui::Color32::from_rgb(236, 238, 242), egui::Color32::from_rgb(120, 126, 136), egui::Color32::from_rgb(40, 44, 52), egui::Color32::from_rgb(90, 150, 240))
        };
        for f in self.visible() {
            let t = self.facing(f) as f32;
            let shade = egui::Color32::from_rgb(
                (fill.r() as f32 * (0.78 + 0.22 * t)) as u8,
                (fill.g() as f32 * (0.78 + 0.22 * t)) as u8,
                (fill.b() as f32 * (0.78 + 0.22 * t)) as u8,
            );
            painter.add(egui::Shape::convex_polygon(self.quad(f, [-1.0, 1.0, -1.0, 1.0]), shade, egui::Stroke::new(1.0, edge)));
            if let Some(e) = hover
                && let Some(r) = Self::region(e, f)
            {
                painter.add(egui::Shape::convex_polygon(self.quad(f, r), hi.gamma_multiply(0.85), egui::Stroke::NONE));
            }
            if t > 0.2 {
                self.label(ui, painter, f, ink.gamma_multiply(((t - 0.2) / 0.4).min(1.0)));
            }
        }
    }

    fn label(&self, ui: &egui::Ui, painter: &egui::Painter, f: usize, color: egui::Color32) {
        let ctx = ui.ctx();
        let galley = painter.layout_no_wrap(LABELS[f].to_string(), egui::FontId::proportional(32.0), color);
        let size = galley.size();
        let k = (1.5 / size.x as f64).min(0.55 / size.y as f64);
        let mut mesh = egui::Mesh::default();
        let font = ctx.fonts(|fs| fs.font_image_size());
        let mut tess = egui::epaint::Tessellator::new(ctx.pixels_per_point(), Default::default(), font, Vec::new());
        tess.tessellate_text(&egui::epaint::TextShape::new(egui::Pos2::ZERO, galley, color), &mut mesh);
        let (_, n, a, b) = FACES[f];
        for v in &mut mesh.vertices {
            let (x, y) = ((v.pos.x - size.x / 2.0) as f64 * k, -(v.pos.y - size.y / 2.0) as f64 * k);
            v.pos = self.screen(add(n, add(mul(a, x), mul(b, y))));
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewer::{CamState, Camera};

    fn cube(yaw: f64, pitch: f64) -> Cube {
        let b = Camera::basis(&CamState { yaw, pitch, dist: 1.0, target: [0.0; 3], fov: 30.0 });
        Cube::new(egui::pos2(100.0, 100.0), 30.0, &b)
    }

    #[test]
    fn there_are_26_elements_and_each_snaps_to_its_own_direction() {
        let all = Element::all();
        assert_eq!(all.len(), 26);
        assert_eq!(all.iter().filter(|e| e.is_face()).count(), 6);
        for e in all {
            let (yaw, pitch) = yaw_pitch(e.dir());
            let back = Camera::basis(&CamState { yaw, pitch, dist: 1.0, target: [0.0; 3], fov: 30.0 }).back;
            assert!(dot(back, e.dir()) > 1.0 - 1e-9, "{}", e.name());
            assert_eq!(Element::parse(&e.name()), Some(e));
        }
        assert!(Element::parse("front-back").is_none());
    }

    #[test]
    fn face_directions_match_the_named_views() {
        for (name, yaw, pitch) in crate::viewer::NAMED_VIEWS.iter().take(6) {
            let (y, p) = yaw_pitch(Element::parse(name).unwrap().dir());
            assert!((y - yaw).sin().abs() < 1e-9 && (p - pitch).abs() < 1e-9, "{name}: {y} {p}");
        }
    }

    #[test]
    fn every_element_visible_from_a_corner_view_picks_back_to_itself() {
        let c = cube(0.7, 0.5);
        let mut hit = 0;
        for e in Element::all() {
            if let Some(p) = c.point_of(e) {
                assert_eq!(c.pick(p), Some(e), "{}", e.name());
                hit += 1;
            }
        }
        assert!(hit >= 7, "{hit}");
        let front = cube(0.0, 0.0);
        assert_eq!(front.pick(front.centre).map(|e| e.name()), Some("front".into()));
        let top_strip = front.centre + egui::vec2(0.0, -0.8 * front.r);
        assert_eq!(front.pick(top_strip).map(|e| e.name()), Some("front-top".into()));
        let left_side = front.centre + egui::vec2(0.8 * front.r, 0.0);
        assert_eq!(front.pick(left_side).map(|e| e.name()), Some("front-left".into()));
    }
}
