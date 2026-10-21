use std::collections::BTreeMap;

use crate::game_mesh::Point;

#[derive(Clone, Debug, PartialEq)]
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
    fn refuses_a_base_too_small_and_an_empty_or_malformed_file() {
        let b = Binding::parse(SAMPLE).unwrap();
        assert!(b.evaluate(&base()[..3]).unwrap_err().contains("names base vertex 3"));
        assert!(Binding::parse("name x\nverts 0\n").unwrap_err().contains("binds no vertices"));
        assert!(Binding::parse("verts 0\n0 1 2 0.5 x 0.25 0 0 0\n").unwrap_err().contains("line 2"));
        assert!(Binding::parse("x_scale 0 1 0\nverts 0\n1\n").unwrap_err().contains("must be > 0"));
    }
}
