use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CagePoint {
    Corner(u32),
    Edge([u32; 2], f64),
    Face([u32; 4], f64, f64),
}

impl CagePoint {
    pub fn terms(&self) -> Vec<(u32, f64)> {
        match *self {
            CagePoint::Corner(c) => vec![(c, 1.0)],
            CagePoint::Edge([a, b], t) => vec![(a, 1.0 - t), (b, t)],
            CagePoint::Face([c0, c1, c2, c3], s, t) => vec![(c0, (1.0 - s) * (1.0 - t)), (c1, s * (1.0 - t)), (c2, s * t), (c3, (1.0 - s) * t)],
        }
    }
}

fn distances(adjacent: &HashMap<u32, Vec<u32>>, from: &[u32]) -> HashMap<u32, u32> {
    let mut d: HashMap<u32, u32> = from.iter().map(|&v| (v, 0)).collect();
    let mut queue: VecDeque<u32> = from.iter().copied().collect();
    while let Some(v) = queue.pop_front() {
        let dv = d[&v];
        for &u in adjacent.get(&v).into_iter().flatten() {
            if let std::collections::hash_map::Entry::Vacant(e) = d.entry(u) {
                e.insert(dv + 1);
                queue.push_back(u);
            }
        }
    }
    d
}

pub fn cage_points(quads: &[[u32; 4]], vertices: usize, per_face: usize) -> Result<Vec<CagePoint>, String> {
    let side = (per_face as f64).sqrt().round() as usize;
    if side * side != per_face || per_face == 0 || !quads.len().is_multiple_of(per_face) {
        return Err(format!("{} quads do not split into patches of {per_face}", quads.len()));
    }
    let n = side as f64;
    let mut out: Vec<Option<CagePoint>> = vec![None; vertices];
    for (f, block) in quads.chunks(per_face).enumerate() {
        let mut halves: HashSet<(u32, u32)> = HashSet::with_capacity(per_face * 4);
        let mut incidence: HashMap<u32, u32> = HashMap::new();
        let mut adjacent: HashMap<u32, Vec<u32>> = HashMap::new();
        for q in block {
            for k in 0..4 {
                let (a, b) = (q[k], q[(k + 1) % 4]);
                if (a as usize) >= vertices || (b as usize) >= vertices {
                    return Err(format!("coarse face {f}: vertex outside the {vertices} vertices"));
                }
                halves.insert((a, b));
                *incidence.entry(a).or_default() += 1;
                if !halves.contains(&(b, a)) {
                    adjacent.entry(a).or_default().push(b);
                    adjacent.entry(b).or_default().push(a);
                }
            }
        }
        let mut next: HashMap<u32, u32> = HashMap::new();
        for &(a, b) in &halves {
            if !halves.contains(&(b, a)) && next.insert(a, b).is_some() {
                return Err(format!("coarse face {f}: its patch boundary is not a simple loop"));
            }
        }
        let start = *next.keys().filter(|v| incidence[*v] == 1).min().ok_or_else(|| format!("coarse face {f}: no patch corner"))?;
        let mut walk = vec![start];
        loop {
            let v = next[walk.last().unwrap()];
            if v == start {
                break;
            }
            walk.push(v);
            if walk.len() > next.len() {
                return Err(format!("coarse face {f}: its patch boundary does not close"));
            }
        }
        if walk.len() != 4 * side || walk.len() != next.len() || (0..4).any(|k| incidence[&walk[k * side]] != 1) || walk.iter().filter(|v| incidence[*v] == 1).count() != 4 {
            return Err(format!("coarse face {f}: the patch boundary is not four sides of {side}"));
        }
        let c = [walk[0], walk[side], walk[2 * side], walk[3 * side]];
        let along = |k: usize| -> Vec<u32> { (0..=side).map(|i| walk[(k * side + i) % (4 * side)]).collect() };
        let d0 = distances(&adjacent, &along(0));
        let d1 = distances(&adjacent, &along(1));
        let d2 = distances(&adjacent, &along(2));
        let d3 = distances(&adjacent, &along(3));
        for (pos, &v) in walk.iter().enumerate() {
            let p = if pos % side == 0 {
                CagePoint::Corner(v)
            } else {
                let k = pos / side;
                CagePoint::Edge([c[k], c[(k + 1) % 4]], (pos % side) as f64 / n)
            };
            out[v as usize].get_or_insert(p);
        }
        let on_walk: HashSet<u32> = walk.iter().copied().collect();
        for &v in adjacent.keys() {
            if on_walk.contains(&v) {
                continue;
            }
            let (u, w, u2, w2) = (d3[&v], d0[&v], d1[&v], d2[&v]);
            if (u + u2) as usize != side || (w + w2) as usize != side {
                return Err(format!("coarse face {f}: vertex {v} is not on a regular {side}x{side} grid"));
            }
            if out[v as usize].replace(CagePoint::Face(c, u as f64 / n, w as f64 / n)).is_some() {
                return Err(format!("vertex {v} is inside two coarse faces"));
            }
        }
    }
    out.into_iter().enumerate().map(|(i, p)| p.ok_or_else(|| format!("vertex {i} is in no coarse face"))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(side: usize, faces_x: usize) -> (Vec<[u32; 4]>, usize, Vec<[f64; 2]>) {
        let w = faces_x * side + 1;
        let h = side + 1;
        let id = |x: usize, y: usize| (y * w + x) as u32;
        let mut quads = Vec::new();
        for f in 0..faces_x {
            for y in 0..side {
                for x in 0..side {
                    let x0 = f * side + x;
                    quads.push([id(x0, y), id(x0 + 1, y), id(x0 + 1, y + 1), id(x0, y + 1)]);
                }
            }
        }
        let uv = (0..w * h).map(|i| [(i % w) as f64, (i / w) as f64]).collect();
        (quads, w * h, uv)
    }

    #[test]
    fn bilinear_terms_reproduce_a_bilinear_field_on_two_patches() {
        let (quads, n, uv) = grid(4, 2);
        let pts = cage_points(&quads, n, 16).unwrap();
        let field = |p: [f64; 2]| 0.3 + 0.2 * p[0] - 0.1 * p[1] + 0.05 * p[0] * p[1];
        let mut corners = 0;
        for (v, p) in pts.iter().enumerate() {
            let terms = p.terms();
            assert!((terms.iter().map(|t| t.1).sum::<f64>() - 1.0).abs() < 1e-12);
            assert!(terms.iter().all(|t| t.1 >= -1e-12));
            let got: f64 = terms.iter().map(|&(c, k)| k * field(uv[c as usize])).sum();
            assert!((got - field(uv[v])).abs() < 1e-12, "vertex {v} at {:?}: {got} vs {}", uv[v], field(uv[v]));
            corners += matches!(p, CagePoint::Corner(_)) as usize;
        }
        assert_eq!(corners, 6);
    }

    #[test]
    fn a_patch_that_is_not_a_grid_is_refused() {
        let (mut quads, n, _) = grid(4, 1);
        quads[5].swap(1, 3);
        assert!(cage_points(&quads, n, 16).is_err());
    }
}
