use serde::Serialize;

pub const NOSE_TIP: usize = 185894;
pub const COLUMN_DEG: f64 = 15.0;
pub const ROW_MM: f64 = 10.0;
pub const COLUMNS: u32 = 24;
pub const FRONT_COLUMN: u32 = 12;
pub const LETTERS: &str = "ABCDEFGHIJKLMNOPQRSTUVWX";
pub const ADDRESS_PREFIX: &str = "GF@";
pub const AXIS_SPAN_DEG: f64 = 45.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct FaceChart {
    pub centre: [f64; 2],
    pub front: [f64; 2],
    pub top: f64,
    pub nose_tip: usize,
    pub nose: [f64; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct Cell {
    pub col: u32,
    pub row: u32,
    pub sub: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellQuery {
    pub col: u32,
    pub row: u32,
    pub sub: Option<u32>,
}

impl FaceChart {
    pub fn from_base(positions: &[[f64; 3]], nose_tip: usize) -> Result<FaceChart, String> {
        let nose = *positions.get(nose_tip).ok_or_else(|| format!("the nose tip vertex {nose_tip} is past the {} base vertices", positions.len()))?;
        crate::check_finite(positions)?;
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        let mut top = f64::NEG_INFINITY;
        for p in positions {
            top = top.max(p[2]);
            if p[2] >= nose[2] {
                for k in 0..2 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
            }
        }
        let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
        let d = [nose[0] - centre[0], nose[1] - centre[1]];
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        if l < 1e-6 {
            return Err("the nose tip lies on the chart axis".into());
        }
        Ok(FaceChart { centre, front: [d[0] / l, d[1] / l], top, nose_tip, nose })
    }

    pub fn coords(&self, p: [f64; 3]) -> [f64; 2] {
        let r = [p[0] - self.centre[0], p[1] - self.centre[1]];
        let f = self.front;
        let u = (f[0] * r[1] - f[1] * r[0]).atan2(f[0] * r[0] + f[1] * r[1]).to_degrees();
        [if u <= -180.0 { u + 360.0 } else { u }, (self.top - p[2]) * 1000.0]
    }

    pub fn coords_all(&self, positions: &[[f64; 3]]) -> Vec<[f64; 2]> {
        positions.iter().map(|&p| self.coords(p)).collect()
    }

    pub fn definition(&self) -> serde_json::Value {
        serde_json::json!({
            "axis": [0.0, 0.0, 1.0],
            "centre_mm": [self.centre[0] * 1000.0, self.centre[1] * 1000.0],
            "front": self.front,
            "top_mm": self.top * 1000.0,
            "nose_tip_dense": self.nose_tip,
            "nose_tip_mm": self.nose.map(|x| x * 1000.0),
            "column_deg": COLUMN_DEG,
            "row_mm": ROW_MM,
            "columns": COLUMNS,
            "letters": LETTERS,
            "front_column": letter(FRONT_COLUMN).to_string(),
            "axis_span_deg": AXIS_SPAN_DEG,
            "axis_rule": "the seam u = +-180 runs through the middle of column A, so no cell boundary lies on it, and corner u values are unwrapped before a triangle is interpolated; a triangle whose unwrapped corner u values spread over axis_span_deg or more lies within a few millimetres of the axis (the crown, inside the mouth and throat) and has no meaningful angle: a point on it takes the cell of its largest-barycentric corner, and the cells it touches get no in-cell label",
            "rule": "computed once from the base dense positions and carried by vertex identity: a vertical (z) axis through centre, the midpoint of the x and y extent of the base vertices at or above the nose tip; u = the angle about that axis from the direction to the nose tip, positive towards +x on the face (the viewer's right in the front view), in (-180, 180]; v = millimetres below the highest base vertex",
            "cells": "column letter A..X, one per column_deg of u, M centred on the nose tip (u in [-7.5, 7.5) deg), A at the back; row number 1, 2, ... one per row_mm of v from the top of the head down; subcell 1..9 in numeric-keypad layout seen from outside the head: 7 8 9 the top third, 4 5 6 the middle, 1 2 3 the bottom, left to right with u",
        })
    }
}

pub fn letter(col: u32) -> char {
    LETTERS.as_bytes()[(col % COLUMNS) as usize] as char
}

pub fn cell_at(uv: [f64; 2]) -> Cell {
    let x = uv[0] / COLUMN_DEG + 0.5;
    let k = x.floor();
    let col = ((k as i64 + FRONT_COLUMN as i64).rem_euclid(COLUMNS as i64)) as u32;
    let y = uv[1].max(0.0) / ROW_MM;
    let row = y.floor();
    let rx = (((x - k) * 3.0).floor() as i64).clamp(0, 2) as u32;
    let ry = (((y - row) * 3.0).floor() as i64).clamp(0, 2) as u32;
    Cell { col, row: row as u32 + 1, sub: 7 - 3 * ry + rx }
}

pub fn cell_name(c: Cell) -> String {
    format!("{}{}.{}", letter(c.col), c.row, c.sub)
}

pub fn cell_label(c: Cell) -> String {
    format!("{}{}", letter(c.col), c.row)
}

pub fn pack(c: Cell) -> u32 {
    (c.col << 24) | (c.row.min(0xFFFFF) << 4) | c.sub
}

pub fn unpack(p: u32) -> Cell {
    Cell { col: p >> 24, row: (p >> 4) & 0xFFFFF, sub: p & 15 }
}

pub fn parse_cell(text: &str) -> Option<CellQuery> {
    let t = text.trim();
    let mut chars = t.chars();
    let first = chars.next()?.to_ascii_uppercase();
    let col = LETTERS.find(first)? as u32;
    let rest: &str = &t[first.len_utf8()..];
    let (row_text, sub_text) = match rest.split_once('.') {
        Some((r, s)) => (r, Some(s)),
        None => (rest, None),
    };
    if row_text.is_empty() || !row_text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let row: u32 = row_text.parse().ok().filter(|&r| r >= 1)?;
    let sub = match sub_text {
        None => None,
        Some(s) if s.len() == 1 && (b'1'..=b'9').contains(&s.as_bytes()[0]) => Some((s.as_bytes()[0] - b'0') as u32),
        Some(_) => return None,
    };
    Some(CellQuery { col, row, sub })
}

impl CellQuery {
    pub fn contains(&self, c: Cell) -> bool {
        c.col == self.col && c.row == self.row && self.sub.is_none_or(|s| s == c.sub)
    }

    pub fn name(&self) -> String {
        match self.sub {
            Some(s) => format!("{}{}.{s}", letter(self.col), self.row),
            None => format!("{}{}", letter(self.col), self.row),
        }
    }
}

pub fn unwrap3(us: [f64; 3]) -> [f64; 3] {
    us.map(|u| {
        let d = u - us[0];
        if d > 180.0 {
            u - 360.0
        } else if d < -180.0 {
            u + 360.0
        } else {
            u
        }
    })
}

fn wrap(u: f64) -> f64 {
    let w = (u + 180.0).rem_euclid(360.0) - 180.0;
    if w <= -180.0 { w + 360.0 } else { w }
}

pub fn interpolate(corners: [[f64; 2]; 3], bary: [f64; 3]) -> [f64; 2] {
    let u = unwrap3(corners.map(|c| c[0]));
    [wrap(u[0] * bary[0] + u[1] * bary[1] + u[2] * bary[2]), corners[0][1] * bary[0] + corners[1][1] * bary[1] + corners[2][1] * bary[2]]
}

pub fn u_spread(corners: [[f64; 2]; 3]) -> f64 {
    let u = unwrap3(corners.map(|c| c[0]));
    u.iter().copied().fold(f64::NEG_INFINITY, f64::max) - u.iter().copied().fold(f64::INFINITY, f64::min)
}

pub fn wraps_axis(corners: [[f64; 2]; 3]) -> bool {
    u_spread(corners) >= AXIS_SPAN_DEG
}

pub fn point_cell(corners: [[f64; 2]; 3], bary: [f64; 3]) -> Cell {
    if wraps_axis(corners) { cell_at(corners[nearest_corner(bary)]) } else { cell_at(interpolate(corners, bary)) }
}

pub fn triangle_coords(corners: [[f64; 2]; 3]) -> [f64; 2] {
    interpolate(corners, [1.0 / 3.0; 3])
}

pub fn chart_area(corners: [[f64; 2]; 3]) -> f64 {
    let u = unwrap3(corners.map(|c| c[0]));
    let v = corners.map(|c| c[1]);
    (u[1] - u[0]) * (v[2] - v[0]) - (u[2] - u[0]) * (v[1] - v[0])
}

pub fn nearest_corner(bary: [f64; 3]) -> usize {
    if bary[0] >= bary[1] && bary[0] >= bary[2] {
        0
    } else if bary[1] >= bary[2] {
        1
    } else {
        2
    }
}

pub fn designer_regions(vertices: usize, category_of_target: &[usize], offsets: &[u32], index: &[u32], delta: &[i16]) -> (Vec<u8>, usize) {
    let mut best = vec![0.0f32; vertices];
    let mut region = vec![0u8; vertices];
    let mut skipped = 0usize;
    for (t, &id) in category_of_target.iter().enumerate() {
        if id == 0 {
            continue;
        }
        let (lo, hi) = (offsets[t] as usize, offsets[t + 1] as usize);
        let mag: Vec<f32> = (lo..hi)
            .map(|e| {
                let [x, y, z] = [0, 1, 2].map(|k| delta[e * 3 + k] as f64);
                (x * x + y * y + z * z).sqrt() as f32
            })
            .collect();
        let top = mag.iter().fold(0.0f64, |m, &x| m.max(x as f64));
        if top <= 0.0 {
            continue;
        }
        let wide = mag.iter().filter(|&&m| m as f64 >= 0.5 * top).count();
        if wide as f64 > 0.2 * vertices as f64 {
            skipped += 1;
            continue;
        }
        for (e, &m) in (lo..hi).zip(&mag) {
            let g = index[e] as usize;
            let f = m as f64 / top;
            if f >= 0.35 && f > best[g] as f64 {
                best[g] = f as f32;
                region[g] = id as u8;
            }
        }
    }
    (region, skipped)
}

#[derive(Clone, Debug, Serialize)]
pub struct CellArea {
    pub cell: String,
    pub area_mm2: f64,
    pub triangles: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct Sanity {
    pub definition: &'static str,
    pub front_columns: String,
    pub front_rows: [u32; 2],
    pub front_cells: usize,
    pub full_front_cells: usize,
    pub largest: Option<CellArea>,
    pub smallest: Option<CellArea>,
    pub nominal_area_mm2_at_nose_radius: f64,
    pub triangles: usize,
    pub folded_triangles: usize,
    pub folded_by_cell: Vec<(String, usize)>,
    pub folded_rule: &'static str,
}

pub fn sanity(chart: &FaceChart, coords: &[[f64; 2]], positions: &[[f64; 3]], triangles: &[[u32; 3]]) -> Sanity {
    use std::collections::BTreeMap;
    let signs: Vec<f64> = triangles.iter().map(|t| chart_area(t.map(|i| coords[i as usize]))).collect();
    let majority = if signs.iter().filter(|&&a| a > 0.0).count() * 2 >= signs.len() { 1.0 } else { -1.0 };
    let mut folded: BTreeMap<Cell, usize> = BTreeMap::new();
    let mut areas: BTreeMap<Cell, (f64, usize, u32, bool)> = BTreeMap::new();
    let front = |c: Cell| (c.col as i64 - FRONT_COLUMN as i64).abs() <= 5;
    for (t, tri) in triangles.iter().enumerate() {
        let cc = tri.map(|i| coords[i as usize]);
        let cell = cell_at(triangle_coords(cc));
        let flipped = signs[t] * majority < 0.0;
        if flipped {
            *folded.entry(Cell { sub: 0, ..cell }).or_default() += 1;
        }
        let p = tri.map(|i| positions[i as usize]);
        let e1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
        let e2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
        let cr = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let area = 0.5 * (cr[0] * cr[0] + cr[1] * cr[1] + cr[2] * cr[2]).sqrt() * 1e6;
        let slot = areas.entry(Cell { sub: 0, ..cell }).or_insert((0.0, 0, 0, false));
        slot.0 += area;
        slot.1 += 1;
        slot.2 |= 1 << cell.sub;
        slot.3 |= flipped;
    }
    let face: Vec<(&Cell, &(f64, usize, u32, bool))> = areas.iter().filter(|(c, _)| front(**c)).collect();
    let rows = face.iter().fold([u32::MAX, 0], |r, (c, _)| [r[0].min(c.row), r[1].max(c.row)]);
    let full: Vec<CellArea> = face
        .iter()
        .filter(|(_, s)| s.2 == 0b11_1111_1110 && !s.3)
        .map(|(c, s)| CellArea { cell: cell_label(**c), area_mm2: (s.0 * 10.0).round() / 10.0, triangles: s.1 })
        .collect();
    let largest = full.iter().max_by(|a, b| a.area_mm2.total_cmp(&b.area_mm2)).cloned();
    let smallest = full.iter().min_by(|a, b| a.area_mm2.total_cmp(&b.area_mm2)).cloned();
    let radius = ((chart.nose[0] - chart.centre[0]).powi(2) + (chart.nose[1] - chart.centre[1]).powi(2)).sqrt() * 1000.0;
    let mut folded_by_cell: Vec<(String, usize)> = folded.iter().map(|(c, n)| (cell_label(*c), *n)).collect();
    folded_by_cell.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Sanity {
        definition: "face front = columns H..R (|u| < 82.5 deg); a full cell has all nine subcells and no folded triangle; areas are the base game triangles assigned by their chart centroid",
        front_columns: format!("{}..{}", letter(FRONT_COLUMN - 5), letter(FRONT_COLUMN + 5)),
        front_rows: if face.is_empty() { [0, 0] } else { rows },
        front_cells: face.len(),
        full_front_cells: full.len(),
        largest,
        smallest,
        nominal_area_mm2_at_nose_radius: ((radius * COLUMN_DEG.to_radians() * ROW_MM) * 10.0).round() / 10.0,
        triangles: triangles.len(),
        folded_triangles: folded.values().sum(),
        folded_by_cell,
        folded_rule: "a triangle is folded when its orientation in the chart (u right, v down) is opposite to the majority: the surface there faces away from the axis (behind the ears, under the chin and jaw, inside the nostrils and lips), so one cell name covers two layers of surface; the vertex index in an address tells them apart",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chart() -> FaceChart {
        FaceChart { centre: [0.0, 0.0], front: [0.0, -1.0], top: 0.2, nose_tip: 0, nose: [0.0, -0.1, 0.1] }
    }

    #[test]
    fn the_front_is_column_m_and_plus_x_is_to_the_right() {
        let c = chart();
        let at = |p| cell_at(c.coords(p));
        assert_eq!(cell_name(at([0.0, -0.1, 0.195])), "M1.5");
        assert!(c.coords([0.01, -0.1, 0.1])[0] > 0.0);
        assert_eq!(letter(at([0.1, 0.0, 0.1]).col), 'S');
        assert_eq!(letter(at([-0.1, 0.0, 0.1]).col), 'G');
        assert_eq!(letter(at([0.0, 0.1, 0.1]).col), 'A');
        assert_eq!(letter(at([-0.001, 0.1, 0.1]).col), 'A');
        assert_eq!(at([0.0, -0.1, 0.2 - 0.0255]).row, 3);
    }

    #[test]
    fn subcells_follow_the_numeric_keypad() {
        let s = |u: f64, v: f64| cell_at([u, v]).sub;
        assert_eq!([s(-7.0, 0.5), s(0.0, 0.5), s(7.0, 0.5)], [7, 8, 9]);
        assert_eq!([s(-7.0, 5.0), s(0.0, 5.0), s(7.0, 5.0)], [4, 5, 6]);
        assert_eq!([s(-7.0, 9.5), s(0.0, 9.5), s(7.0, 9.5)], [1, 2, 3]);
    }

    #[test]
    fn names_parse_back_and_pack_round_trips() {
        for p in [[0.0, 0.0], [179.9, 301.0], [-179.9, 55.5], [44.0, 123.4]] {
            let c = cell_at(p);
            let q = parse_cell(&cell_name(c)).unwrap();
            assert!(q.contains(c));
            assert_eq!(unpack(pack(c)), c);
        }
        assert_eq!(parse_cell("m12"), Some(CellQuery { col: 12, row: 12, sub: None }));
        assert_eq!(parse_cell("F12.3"), Some(CellQuery { col: 5, row: 12, sub: Some(3) }));
        assert!(parse_cell("Z3").is_none() && parse_cell("M0").is_none() && parse_cell("M3.0").is_none() && parse_cell("M3.12").is_none() && parse_cell("M").is_none());
    }

    #[test]
    fn interpolation_crosses_the_back_seam() {
        let c = interpolate([[179.0, 0.0], [-179.0, 0.0], [179.0, 2.0]], [0.25, 0.5, 0.25]);
        assert!((c[0] - 180.0).abs() < 1e-9 || (c[0] + 180.0).abs() < 1e-9, "{c:?}");
        assert!(chart_area([[179.0, 0.0], [-179.0, 0.0], [179.0, 2.0]]) > 0.0);
    }

    #[test]
    fn a_triangle_around_the_axis_takes_its_largest_corner_cell() {
        let around = [[0.0, 5.0], [120.0, 5.0], [-120.0, 5.0]];
        assert!(wraps_axis(around));
        assert_eq!(point_cell(around, [0.2, 0.7, 0.1]), cell_at(around[1]));
        assert_eq!(point_cell(around, [1.0 / 3.0; 3]), cell_at(around[0]));
        let seam = [[179.0, 50.0], [-179.0, 50.0], [178.0, 52.0]];
        assert!(!wraps_axis(seam));
        assert_eq!(point_cell(seam, [1.0 / 3.0; 3]).col, 0);
        let face = [[1.0, 50.0], [3.0, 50.0], [2.0, 52.0]];
        assert_eq!(point_cell(face, [0.2, 0.3, 0.5]), cell_at(interpolate(face, [0.2, 0.3, 0.5])));
    }

    #[test]
    fn the_chart_is_derived_from_the_base_only() {
        let base = vec![[0.0, -0.1, 0.1], [0.05, 0.0, 0.15], [-0.05, 0.02, 0.2], [0.3, 0.3, 0.0]];
        let c = FaceChart::from_base(&base, 0).unwrap();
        assert_eq!(c.top, 0.2);
        assert!(c.centre[0].abs() < 1e-12 && (c.centre[1] + 0.04).abs() < 1e-12, "{:?}", c.centre);
        assert_eq!(cell_at(c.coords(base[0])).col, FRONT_COLUMN);
        assert!(FaceChart::from_base(&base, 9).is_err());
    }

    #[test]
    fn regions_take_the_relatively_strongest_target_and_skip_wide_ones() {
        let offsets = [0, 2, 4, 14];
        let mut index = vec![0, 1, 1, 2];
        index.extend(0..10);
        let mut delta = vec![10, 0, 0, 5, 0, 0, 10, 0, 0, 10, 0, 0];
        delta.extend(std::iter::repeat_n(7, 30));
        let (r, skipped) = designer_regions(10, &[1, 2, 3], &offsets, &index, &delta);
        assert_eq!(&r[..3], &[1, 2, 2]);
        assert_eq!(skipped, 1);
    }
}
