use std::collections::BTreeMap;

use crate::game_mesh::Point;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    pub a: u32,
    pub b: u32,
    pub reference: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bound {
    pub vertices: [u32; 3],
    pub weights: [f64; 3],
    pub offset: [f64; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Binding {
    pub keys: BTreeMap<String, String>,
    pub scales: [Option<Scale>; 3],
    pub bound: Vec<Bound>,
    pub hidden: Vec<u32>,
}

impl Binding {
    pub fn parse(text: &str) -> Result<Binding, String> {
        let mut out = Binding::default();
        let mut section = "";
        for (n, line) in text.lines().enumerate() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            let Some(&head) = parts.first() else { continue };
            if head.starts_with('#') {
                continue;
            }
            let bad = |what: &str| format!("line {}: {what}: {line:?}", n + 1);
            match head {
                "x_scale" | "y_scale" | "z_scale" => {
                    let [_, a, b, d] = parts.as_slice() else { return Err(bad("a scale line is NAME A B DISTANCE")) };
                    let axis = (head.as_bytes()[0] - b'x') as usize;
                    let scale = Scale { a: a.parse().map_err(|_| bad("scale vertex"))?, b: b.parse().map_err(|_| bad("scale vertex"))?, reference: d.parse().map_err(|_| bad("scale distance"))? };
                    if scale.reference <= 0.0 {
                        return Err(bad("a scale distance must be > 0"));
                    }
                    out.scales[axis] = Some(scale);
                }
                "verts" => section = "verts",
                "delete_verts" => section = "delete",
                _ if section == "verts" && parts.len() == 9 && head.bytes().all(|c| c.is_ascii_digit()) => {
                    let mut f = [0.0; 6];
                    for (k, p) in parts[3..].iter().enumerate() {
                        f[k] = p.parse().map_err(|_| bad("a binding weight or offset"))?;
                    }
                    let mut v = [0u32; 3];
                    for (k, p) in parts[..3].iter().enumerate() {
                        v[k] = p.parse().map_err(|_| bad("a binding vertex"))?;
                    }
                    out.bound.push(Bound { vertices: v, weights: [f[0], f[1], f[2]], offset: [f[3], f[4], f[5]] });
                }
                _ if section == "verts" && parts.len() == 1 && head.bytes().all(|c| c.is_ascii_digit()) => {
                    let v: u32 = head.parse().map_err(|_| bad("a pinned vertex"))?;
                    out.bound.push(Bound { vertices: [v; 3], weights: [1.0, 0.0, 0.0], offset: [0.0; 3] });
                }
                _ if section == "delete" && parts.iter().all(|p| *p == "-" || p.bytes().all(|c| c.is_ascii_digit())) => {
                    let mut k = 0;
                    while k < parts.len() {
                        let from: u32 = parts[k].parse().map_err(|_| bad("a hidden vertex or range"))?;
                        if parts.get(k + 1) == Some(&"-") {
                            let to: u32 = parts.get(k + 2).ok_or_else(|| bad("a range needs its end"))?.parse().map_err(|_| bad("a range end"))?;
                            if to < from {
                                return Err(bad("a range runs backwards"));
                            }
                            out.hidden.extend(from..=to);
                            k += 3;
                        } else {
                            out.hidden.push(from);
                            k += 1;
                        }
                    }
                }
                _ if head.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') && !head.bytes().all(|c| c.is_ascii_digit()) => {
                    out.keys.entry(head.to_string()).or_insert_with(|| parts[1..].join(" "));
                }
                _ => return Err(bad("not a binding, scale, hidden-vertex or keyword line")),
            }
        }
        if out.bound.is_empty() {
            return Err("the file binds no vertices (no lines after `verts`)".into());
        }
        Ok(out)
    }

    pub fn highest_vertex(&self) -> u32 {
        let bound = self.bound.iter().flat_map(|b| b.vertices);
        let scales = self.scales.iter().flatten().flat_map(|s| [s.a, s.b]);
        bound.chain(scales).chain(self.hidden.iter().copied()).max().unwrap_or(0)
    }

    pub fn axis_scales(&self, base: &[Point]) -> [f64; 3] {
        let mut s = [1.0; 3];
        for (axis, scale) in self.scales.iter().enumerate() {
            if let Some(sc) = scale {
                s[axis] = (base[sc.a as usize][axis] - base[sc.b as usize][axis]).abs() / sc.reference;
            }
        }
        s
    }

    pub fn evaluate(&self, base: &[Point]) -> Result<Vec<Point>, String> {
        let top = self.highest_vertex() as usize;
        if top >= base.len() {
            return Err(format!("the binding names base vertex {top}, the base has {}", base.len()));
        }
        let s = self.axis_scales(base);
        Ok(self
            .bound
            .iter()
            .map(|b| {
                let mut p = [0.0; 3];
                for k in 0..3 {
                    let q = base[b.vertices[k] as usize];
                    for (axis, x) in p.iter_mut().enumerate() {
                        *x += b.weights[k] * q[axis];
                    }
                }
                for (axis, x) in p.iter_mut().enumerate() {
                    *x += b.offset[axis] * s[axis];
                }
                p
            })
            .collect())
    }
}

impl Binding {
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (k, v) in &self.keys {
            out.push_str(&format!("{k} {v}\n"));
        }
        for (axis, scale) in self.scales.iter().enumerate() {
            if let Some(sc) = scale {
                out.push_str(&format!("{}_scale {} {} {}\n", ["x", "y", "z"][axis], sc.a, sc.b, sc.reference));
            }
        }
        out.push_str("verts 0\n");
        for b in &self.bound {
            out.push_str(&format!("{} {} {} {} {} {} {} {} {}\n", b.vertices[0], b.vertices[1], b.vertices[2], b.weights[0], b.weights[1], b.weights[2], b.offset[0], b.offset[1], b.offset[2]));
        }
        if !self.hidden.is_empty() {
            out.push_str("delete_verts\n");
            for chunk in self.hidden.chunks(16) {
                out.push_str(&chunk.iter().map(u32::to_string).collect::<Vec<_>>().join(" "));
                out.push('\n');
            }
        }
        out
    }
}

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub fn closest_on_triangle(p: Point, a: Point, b: Point, c: Point) -> [f64; 3] {
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return [0.0, 1.0, 0.0];
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [1.0 - v, v, 0.0];
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return [0.0, 0.0, 1.0];
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return [1.0 - w, 0.0, w];
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return [0.0, 1.0 - w, w];
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    [1.0 - v - w, v, w]
}

pub fn bind_to_surface(garment: &[Point], base: &BTreeMap<u32, Point>, faces: &[[u32; 3]], scales: [Scale; 3]) -> Result<Binding, String> {
    let faces: Vec<[u32; 3]> = faces.iter().copied().filter(|f| f.iter().all(|v| base.contains_key(v))).collect();
    if faces.is_empty() {
        return Err("no base face has all its vertices on the character".into());
    }
    let s: Vec<f64> = scales.iter().enumerate().map(|(axis, sc)| (base[&sc.a][axis] - base[&sc.b][axis]).abs() / sc.reference).collect();
    if s.iter().any(|x| !(x.is_finite() && *x > 0.0)) {
        return Err(format!("a scale pair measures zero on the character: {s:?}"));
    }
    let cell = {
        let mut sum = 0.0;
        for f in &faces {
            sum += dot(sub(base[&f[1]], base[&f[0]]), sub(base[&f[1]], base[&f[0]])).sqrt();
        }
        (2.0 * sum / faces.len() as f64).max(1e-6)
    };
    let key = |p: Point| ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64, (p[2] / cell).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64, i64), Vec<u32>> = std::collections::HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        let c = [0, 1, 2].map(|k| (base[&f[0]][k] + base[&f[1]][k] + base[&f[2]][k]) / 3.0);
        grid.entry(key(c)).or_default().push(i as u32);
    }
    let mut bound = Vec::with_capacity(garment.len());
    for &p in garment {
        let c = key(p);
        let mut best: Option<(f64, usize, [f64; 3])> = None;
        let mut r = 0i64;
        loop {
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != r {
                            continue;
                        }
                        for &i in grid.get(&(c.0 + dx, c.1 + dy, c.2 + dz)).into_iter().flatten() {
                            let f = faces[i as usize];
                            let (a, b, cc) = (base[&f[0]], base[&f[1]], base[&f[2]]);
                            let w = closest_on_triangle(p, a, b, cc);
                            let q = [0, 1, 2].map(|k| w[0] * a[k] + w[1] * b[k] + w[2] * cc[k]);
                            let d = dot(sub(p, q), sub(p, q));
                            if best.is_none_or(|bb| d < bb.0) {
                                best = Some((d, i as usize, w));
                            }
                        }
                    }
                }
            }
            if let Some((d, _, _)) = best
                && d.sqrt() < (r as f64 - 1.0).max(0.0) * cell
            {
                break;
            }
            r += 1;
            if r > 4096 {
                break;
            }
        }
        let (_, i, w) = best.ok_or("a garment vertex found no face")?;
        let f = faces[i];
        let q = [0, 1, 2].map(|k| w[0] * base[&f[0]][k] + w[1] * base[&f[1]][k] + w[2] * base[&f[2]][k]);
        bound.push(Bound { vertices: f, weights: w, offset: [0, 1, 2].map(|k| (p[k] - q[k]) / s[k]) });
    }
    let [x, y, z] = scales;
    Ok(Binding { keys: BTreeMap::new(), scales: [Some(x), Some(y), Some(z)], bound, hidden: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# comment\nname tiny\nx_scale 0 1 2.0\ny_scale 0 2 1.0\nz_scale 0 3 1.0\nobj_file tiny.obj\nverts 0\n\
material tiny.mhmat\n0 1 2 0.5 0.25 0.25 0.1 0.2 0.3\n3\ndelete_verts\n1 2\n3\n";

    fn base() -> Vec<Point> {
        vec![[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 3.0]]
    }

    #[test]
    fn parses_keys_after_verts_pinned_lines_and_hidden_vertices() {
        let b = Binding::parse(SAMPLE).unwrap();
        assert_eq!(b.keys["name"], "tiny");
        assert_eq!(b.keys["material"], "tiny.mhmat");
        assert_eq!(b.bound.len(), 2);
        assert_eq!(b.bound[1], Bound { vertices: [3; 3], weights: [1.0, 0.0, 0.0], offset: [0.0; 3] });
        assert_eq!(b.hidden, vec![1, 2, 3]);
        assert_eq!(b.highest_vertex(), 3);
    }

    #[test]
    fn evaluates_barycentric_plus_offsets_scaled_per_axis() {
        let b = Binding::parse(SAMPLE).unwrap();
        let p = b.evaluate(&base()).unwrap();
        assert_eq!(b.axis_scales(&base()), [2.0, 2.0, 3.0]);
        let want = [0.5 * 0.0 + 0.25 * 4.0 + 0.1 * 2.0, 0.25 * 2.0 + 0.2 * 2.0, 0.3 * 3.0];
        for axis in 0..3 {
            assert!((p[0][axis] - want[axis]).abs() < 1e-12, "{p:?}");
        }
        assert_eq!(p[1], [0.0, 0.0, 3.0]);
    }

    #[test]
    fn reads_hidden_vertex_ranges_inclusive() {
        let b = Binding::parse("verts 0\n0\ndelete_verts\n4 - 7 9 12 - 12\n20\n").unwrap();
        assert_eq!(b.hidden, vec![4, 5, 6, 7, 9, 12, 20]);
        assert!(Binding::parse("verts 0\n0\ndelete_verts\n7 - 4\n").unwrap_err().contains("backwards"));
        assert!(Binding::parse("verts 0\n0\ndelete_verts\n7 -\n").unwrap_err().contains("needs its end"));
    }

    #[test]
    fn a_surface_binding_reproduces_the_garment_and_round_trips_as_text() {
        let base: BTreeMap<u32, Point> = [(0, [0.0, 0.0, 0.0]), (1, [1.0, 0.0, 0.0]), (2, [0.0, 1.0, 0.0]), (3, [1.0, 1.0, 0.0]), (4, [0.0, 0.0, 2.0])].into();
        let scales = [Scale { a: 0, b: 1, reference: 1.0 }, Scale { a: 0, b: 2, reference: 1.0 }, Scale { a: 0, b: 4, reference: 2.0 }];
        let garment = vec![[0.25, 0.25, 0.1], [0.9, 0.8, -0.2], [2.0, 2.0, 0.0], [0.5, 0.5, 0.0]];
        let b = bind_to_surface(&garment, &base, &[[0, 1, 2], [1, 3, 2]], scales).unwrap();
        let mut flat = vec![[0.0; 3]; 5];
        for (k, p) in &base {
            flat[*k as usize] = *p;
        }
        let back = b.evaluate(&flat).unwrap();
        for (p, q) in garment.iter().zip(&back) {
            assert!((0..3).all(|k| (p[k] - q[k]).abs() < 1e-12), "{p:?} {q:?}");
        }
        assert_eq!(b.bound[0].offset, [0.0, 0.0, 0.1]);
        let mut with_hidden = b.clone();
        with_hidden.hidden = (0..20).collect();
        with_hidden.keys.insert("name".into(), "probe".into());
        assert_eq!(Binding::parse(&with_hidden.to_text()).unwrap(), with_hidden);
        let mut scaled = flat.clone();
        for p in &mut scaled {
            p[0] *= 2.0;
        }
        assert!((b.evaluate(&scaled).unwrap()[0][0] - 0.5).abs() < 1e-12, "x offsets scale with the base");
        assert!(bind_to_surface(&garment, &base, &[[0, 1, 7]], scales).unwrap_err().contains("no base face"));
    }

    #[test]
    fn refuses_a_base_too_small_and_an_empty_or_malformed_file() {
        let b = Binding::parse(SAMPLE).unwrap();
        assert!(b.evaluate(&base()[..3]).unwrap_err().contains("names base vertex 3"));
        assert!(Binding::parse("name x\nverts 0\n").unwrap_err().contains("binds no vertices"));
        assert!(Binding::parse("verts 0\n0 1 2 0.5 x 0.25 0 0 0\n").unwrap_err().contains("line 2"));
        assert!(Binding::parse("x_scale 0 1 0\nverts 0\n1\n").unwrap_err().contains("must be > 0"));
    }
}
