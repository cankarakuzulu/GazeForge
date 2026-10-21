use std::collections::HashMap;

use gf_geom::Uv;

pub type Point = [f64; 3];
pub type Row = Vec<(u32, f64)>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stencil {
    pub rows: Vec<Row>,
    pub sources: usize,
}

fn merge(mut row: Row) -> Row {
    row.sort_by_key(|e| e.0);
    let mut out: Row = Vec::with_capacity(row.len());
    for (i, w) in row {
        match out.last_mut() {
            Some(last) if last.0 == i => last.1 += w,
            _ => out.push((i, w)),
        }
    }
    out.retain(|e| e.1 != 0.0);
    out
}

impl Stencil {
    pub fn identity(n: usize) -> Stencil {
        Stencil { rows: (0..n as u32).map(|i| vec![(i, 1.0)]).collect(), sources: n }
    }

    pub fn apply(&self, points: &[Point]) -> Vec<Point> {
        self.rows
            .iter()
            .map(|row| {
                let mut p = [0.0; 3];
                for &(i, w) in row {
                    let q = points[i as usize];
                    for k in 0..3 {
                        p[k] += w * q[k];
                    }
                }
                p
            })
            .collect()
    }

    pub fn then(&self, next: &Stencil) -> Stencil {
        let rows = next
            .rows
            .iter()
            .map(|row| merge(row.iter().flat_map(|&(j, w)| self.rows[j as usize].iter().map(move |&(i, v)| (i, w * v))).collect()))
            .collect();
        Stencil { rows, sources: self.sources }
    }
}

pub struct Level {
    pub stencil: Stencil,
    pub quads: Vec<[u32; 4]>,
    pub uvs: Vec<[Uv; 4]>,
}

fn mid(a: Uv, b: Uv) -> Uv {
    [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]
}

pub fn catmull_clark(quads: &[[u32; 4]], uvs: &[[Uv; 4]], vertices: usize) -> Result<Level, String> {
    if uvs.len() != quads.len() {
        return Err(format!("{} quads but {} corner uv sets", quads.len(), uvs.len()));
    }
    let mut edge_of: HashMap<(u32, u32), u32> = HashMap::new();
    let mut edge_ends: Vec<(u32, u32)> = Vec::new();
    let mut edge_faces: Vec<Vec<u32>> = Vec::new();
    let mut vertex_faces: Vec<Vec<u32>> = vec![Vec::new(); vertices];
    let mut vertex_edges: Vec<Vec<u32>> = vec![Vec::new(); vertices];
    for (f, q) in quads.iter().enumerate() {
        for k in 0..4 {
            let (a, b) = (q[k], q[(k + 1) % 4]);
            if a as usize >= vertices || b as usize >= vertices {
                return Err(format!("quad {f} names a vertex outside the {vertices} vertices"));
            }
            let key = (a.min(b), a.max(b));
            let e = *edge_of.entry(key).or_insert_with(|| {
                edge_ends.push(key);
                edge_faces.push(Vec::new());
                vertex_edges[key.0 as usize].push((edge_ends.len() - 1) as u32);
                vertex_edges[key.1 as usize].push((edge_ends.len() - 1) as u32);
                (edge_ends.len() - 1) as u32
            });
            edge_faces[e as usize].push(f as u32);
            vertex_faces[a as usize].push(f as u32);
        }
    }
    if let Some(e) = edge_faces.iter().position(|f| f.len() > 2) {
        return Err(format!("edge {:?} is shared by more than two quads", edge_ends[e]));
    }
    let (n, ne, nf) = (vertices as u32, edge_ends.len() as u32, quads.len() as u32);
    let face_row = |f: u32| -> Row { quads[f as usize].iter().map(|&v| (v, 0.25)).collect() };
    let mut rows: Vec<Row> = Vec::with_capacity((n + ne + nf) as usize);
    for v in 0..vertices {
        let boundary: Vec<u32> = vertex_edges[v].iter().copied().filter(|&e| edge_faces[e as usize].len() == 1).collect();
        let row = if !boundary.is_empty() {
            if boundary.len() != 2 {
                return Err(format!("vertex {v} has {} boundary edges; the boundary must be simple loops", boundary.len()));
            }
            let other = |e: u32| {
                let (a, b) = edge_ends[e as usize];
                if a as usize == v { b } else { a }
            };
            vec![(v as u32, 0.75), (other(boundary[0]), 0.125), (other(boundary[1]), 0.125)]
        } else if vertex_edges[v].is_empty() {
            vec![(v as u32, 1.0)]
        } else {
            let k = vertex_edges[v].len() as f64;
            let faces = &vertex_faces[v];
            let mut row: Row = Vec::new();
            for &f in faces {
                row.extend(face_row(f).into_iter().map(|(i, w)| (i, w / faces.len() as f64 / k)));
            }
            for &e in &vertex_edges[v] {
                let (a, b) = edge_ends[e as usize];
                row.push((a, 2.0 * 0.5 / k / k));
                row.push((b, 2.0 * 0.5 / k / k));
            }
            row.push((v as u32, (k - 3.0) / k));
            merge(row)
        };
        rows.push(row);
    }
    for e in 0..edge_ends.len() {
        let (a, b) = edge_ends[e];
        let faces = &edge_faces[e];
        let row = if faces.len() == 1 {
            vec![(a, 0.5), (b, 0.5)]
        } else {
            let mut row: Row = vec![(a, 0.25), (b, 0.25)];
            for &f in faces {
                row.extend(face_row(f).into_iter().map(|(i, w)| (i, w * 0.25)));
            }
            merge(row)
        };
        rows.push(row);
    }
    for f in 0..nf {
        rows.push(face_row(f));
    }
    let mut out_quads = Vec::with_capacity(quads.len() * 4);
    let mut out_uvs = Vec::with_capacity(quads.len() * 4);
    for (f, q) in quads.iter().enumerate() {
        let e = |k: usize| n + edge_of[&(q[k].min(q[(k + 1) % 4]), q[k].max(q[(k + 1) % 4]))];
        let centre = n + ne + f as u32;
        let u = &uvs[f];
        let c = [(u[0][0] + u[1][0] + u[2][0] + u[3][0]) / 4.0, (u[0][1] + u[1][1] + u[2][1] + u[3][1]) / 4.0];
        for k in 0..4 {
            let prev = (k + 3) % 4;
            out_quads.push([q[k], e(k), centre, e(prev)]);
            out_uvs.push([u[k], mid(u[k], u[(k + 1) % 4]), c, mid(u[prev], u[k])]);
        }
    }
    Ok(Level { stencil: Stencil { rows, sources: vertices }, quads: out_quads, uvs: out_uvs })
}

pub fn subdivide(quads: &[[u32; 4]], uvs: &[[Uv; 4]], vertices: usize, levels: usize) -> Result<Level, String> {
    let mut level = Level { stencil: Stencil::identity(vertices), quads: quads.to_vec(), uvs: uvs.to_vec() };
    for _ in 0..levels {
        let next = catmull_clark(&level.quads, &level.uvs, level.stencil.rows.len())?;
        level = Level { stencil: level.stencil.then(&next.stencil), quads: next.quads, uvs: next.uvs };
    }
    Ok(level)
}

pub fn triangles(quads: &[[u32; 4]], uvs: &[[Uv; 4]]) -> (Vec<[u32; 3]>, Vec<[Uv; 3]>) {
    let mut t = Vec::with_capacity(quads.len() * 2);
    let mut u = Vec::with_capacity(quads.len() * 2);
    for (q, w) in quads.iter().zip(uvs) {
        for s in [[0, 1, 2], [0, 2, 3]] {
            t.push(s.map(|k| q[k]));
            u.push(s.map(|k| w[k]));
        }
    }
    (t, u)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize) -> (Vec<[u32; 4]>, Vec<[Uv; 4]>, Vec<Point>) {
        let w = n + 1;
        let id = |x: usize, y: usize| (y * w + x) as u32;
        let mut quads = Vec::new();
        let mut uvs = Vec::new();
        for y in 0..n {
            for x in 0..n {
                quads.push([id(x, y), id(x + 1, y), id(x + 1, y + 1), id(x, y + 1)]);
                let uv = |a: usize, b: usize| [a as f32 / n as f32, b as f32 / n as f32];
                uvs.push([uv(x, y), uv(x + 1, y), uv(x + 1, y + 1), uv(x, y + 1)]);
            }
        }
        let pts = (0..w * w).map(|i| [(i % w) as f64, (i / w) as f64, 0.0]).collect();
        (quads, uvs, pts)
    }

    #[test]
    fn rows_are_affine_and_a_flat_regular_grid_stays_on_its_lattice_away_from_the_boundary() {
        let (quads, uvs, pts) = grid(6);
        let l = subdivide(&quads, &uvs, pts.len(), 2).unwrap();
        assert!(l.stencil.rows.iter().all(|r| (r.iter().map(|e| e.1).sum::<f64>() - 1.0).abs() < 1e-12));
        let fine = l.stencil.apply(&pts);
        assert_eq!(fine.len(), 25 * 25);
        assert_eq!(l.quads.len(), 24 * 24);
        assert!(fine.iter().all(|p| p[2] == 0.0));
        let inner = |p: Point| p[0] > 1.5 && p[0] < 4.5 && p[1] > 1.5 && p[1] < 4.5;
        let mut checked = 0;
        for (q, u) in l.quads.iter().zip(&l.uvs) {
            for k in 0..4 {
                let p = fine[q[k] as usize];
                if inner(p) {
                    let g = [p[0] * 4.0, p[1] * 4.0];
                    assert!((g[0] - g[0].round()).abs() < 1e-9 && (g[1] - g[1].round()).abs() < 1e-9, "{p:?}");
                    assert!((u[k][0] as f64 - p[0] / 6.0).abs() < 1e-6 && (u[k][1] as f64 - p[1] / 6.0).abs() < 1e-6, "{p:?} {:?}", u[k]);
                    checked += 1;
                }
            }
        }
        assert!(checked > 100);
    }

    #[test]
    fn a_fine_boundary_vertex_depends_only_on_coarse_boundary_vertices() {
        let (quads, uvs, pts) = grid(3);
        let boundary_of = |quads: &[[u32; 4]]| -> std::collections::HashSet<u32> {
            let mut count: HashMap<(u32, u32), u32> = HashMap::new();
            for q in quads {
                for k in 0..4 {
                    *count.entry((q[k].min(q[(k + 1) % 4]), q[k].max(q[(k + 1) % 4]))).or_default() += 1;
                }
            }
            count.into_iter().filter(|e| e.1 == 1).flat_map(|e| [e.0.0, e.0.1]).collect()
        };
        let coarse = boundary_of(&quads);
        let l = subdivide(&quads, &uvs, pts.len(), 2).unwrap();
        let fine = boundary_of(&l.quads);
        assert_eq!(fine.len(), 4 * 3 * 4);
        for v in fine {
            assert!(l.stencil.rows[v as usize].iter().all(|e| coarse.contains(&e.0)), "fine boundary vertex {v}");
        }
    }

    #[test]
    fn a_shared_edge_of_three_quads_is_refused() {
        let quads = [[0, 1, 2, 3], [1, 0, 4, 5], [0, 1, 6, 7]];
        let uvs = [[[0.0, 0.0]; 4]; 3];
        assert!(catmull_clark(&quads, &uvs, 8).err().is_some_and(|e| e.contains("more than two")));
    }
}
