use std::sync::OnceLock;

use serde_json::{Value, json};

use crate::Uv;
use crate::decimate::Decimated;
use crate::placement::{self as pl, Csr};
use crate::placement_f5::{self, Footprint};
use crate::uv_chart::SurfaceTopology;

pub(crate) struct F5Parts {
    pub(crate) foot: Footprint,
    pub(crate) rows: Csr,
    pub(crate) pattern: Vec<Vec<u32>>,
    pub(crate) seam_rows: Csr,
}

pub struct Prepared {
    dense_vertices: usize,
    dense_triangles: usize,
    game_vertices: usize,
    game_triangles: usize,
    pub(crate) boundary: Vec<bool>,
    pub(crate) seam: Vec<bool>,
    pub(crate) fixed: Vec<bool>,
    pub(crate) vtris: Vec<Vec<u32>>,
    pub(crate) rings: Vec<Vec<u32>>,
    pub(crate) edges: Vec<(u32, u32)>,
    pub(crate) edge_rows: Csr,
    pub(crate) neighbours: Vec<Vec<u32>>,
    pub(crate) corners: Csr,
    pub(crate) owned: Csr,
    pub(crate) surface: SurfaceTopology,
    pub(crate) patches: Csr,
    f3_pattern: OnceLock<Vec<Vec<u32>>>,
    f5: OnceLock<F5Parts>,
}

impl Prepared {
    pub fn new(dense_vertices: usize, dense_triangles: &[[u32; 3]], dense_uvs: &[[Uv; 3]], mesh: &Decimated, uv_ring: usize) -> Prepared {
        let count = mesh.kept.len();
        let boundary = pl::boundary_vertices(mesh);
        let seam: Vec<bool> = crate::placement_f3::seam_vertices(mesh).iter().zip(&boundary).map(|(&s, &b)| s && !b).collect();
        let fixed: Vec<bool> = (0..count).map(|g| boundary[g] || seam[g]).collect();
        let vtris = pl::vertex_triangles(mesh, count);
        let rings = pl::ring_triangles(mesh, &vtris);
        let edges = pl::unique_edges(mesh);
        let edge_rows = Csr::build(count, |push| {
            for (e, &(p, q)) in edges.iter().enumerate() {
                push(p as usize, (e * 2) as u32);
                push(q as usize, (e * 2 + 1) as u32);
            }
        });
        let surface = SurfaceTopology::new(dense_vertices, dense_triangles, dense_uvs, mesh, uv_ring);
        let mut key: Vec<Option<Uv>> = vec![None; count];
        for (tri, uvs) in mesh.triangles.iter().zip(&mesh.triangle_uvs) {
            for c in 0..3 {
                key[tri[c] as usize].get_or_insert(uvs[c]);
            }
        }
        let patches = Csr::from_rows(crate::map_range(count, |g| match key[g] {
            Some(uv) if !fixed[g] => surface.patch(dense_triangles, dense_uvs, &mesh.owner, g as u32, uv),
            _ => Vec::new(),
        }));
        Prepared {
            dense_vertices,
            dense_triangles: dense_triangles.len(),
            game_vertices: count,
            game_triangles: mesh.triangles.len(),
            boundary,
            seam,
            fixed,
            vtris,
            rings,
            edges,
            edge_rows,
            neighbours: crate::placement_f2::neighbours(mesh),
            corners: pl::corner_rows(dense_vertices, dense_triangles),
            owned: pl::owned_rows(dense_triangles, mesh),
            surface,
            patches,
            f3_pattern: OnceLock::new(),
            f5: OnceLock::new(),
        }
    }

    pub fn fits(&self, dense_vertices: usize, dense_triangles: usize, mesh: &Decimated, uv_ring: usize) -> bool {
        self.dense_vertices == dense_vertices
            && self.dense_triangles == dense_triangles
            && self.game_vertices == mesh.kept.len()
            && self.game_triangles == mesh.triangles.len()
            && self.surface.ring() == uv_ring
    }

    pub(crate) fn f3_pattern(&self, mesh: &Decimated) -> &[Vec<u32>] {
        self.f3_pattern.get_or_init(|| pl::sparse_pattern(mesh, self.game_vertices))
    }

    pub(crate) fn f5_parts(&self, dense_triangles: &[[u32; 3]], dense_uvs: &[[Uv; 3]], mesh: &Decimated) -> &F5Parts {
        self.f5.get_or_init(|| {
            let foot = placement_f5::footprint(self.dense_vertices, dense_triangles, dense_uvs, mesh, &self.rings);
            let fixed = &self.fixed;
            let rows = Csr::build(self.game_vertices, |push| {
                for (i, &t) in foot.triangle.iter().enumerate() {
                    if t == u32::MAX {
                        continue;
                    }
                    let w = foot.weights[i];
                    let tri = mesh.triangles[t as usize];
                    for r in 0..3 {
                        if !fixed[tri[r] as usize] && w[r] != 0.0 {
                            push(tri[r] as usize, (i * 3 + r) as u32);
                        }
                    }
                }
            });
            let seam = &self.seam;
            let seam_rows = Csr::build(self.game_vertices, |push| {
                for (i, &t) in foot.triangle.iter().enumerate() {
                    if t == u32::MAX {
                        continue;
                    }
                    let tri = mesh.triangles[t as usize];
                    for s in 0..3 {
                        if seam[tri[s] as usize] {
                            push(tri[s] as usize, (i * 3 + s) as u32);
                        }
                    }
                }
            });
            F5Parts { foot, rows, pattern: placement_f5::block_pattern(&self.edges, self.game_vertices), seam_rows }
        })
    }

    pub fn summary(&self) -> Value {
        json!({"patch_entries": self.patches.len(), "corner_entries": self.corners.len(), "owned_entries": self.owned.len(),
               "f5_ready": self.f5.get().is_some(), "f3_ready": self.f3_pattern.get().is_some()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lids::ClearSettings;
    use crate::placement_f3::F3Settings;
    use crate::placement_f5::F5Settings;
    use crate::quadric::{length, sub};
    use crate::uv_chart::{self, Surface};

    fn grid(m: usize) -> Vec<[u32; 3]> {
        let mut tris = Vec::new();
        for y in 0..m - 1 {
            for x in 0..m - 1 {
                let a = (y * m + x) as u32;
                tris.push([a, a + 1, a + m as u32 + 1]);
                tris.push([a, a + m as u32 + 1, a + m as u32]);
            }
        }
        tris
    }

    fn fixture(wave: f64) -> (Vec<[f64; 3]>, Vec<[u32; 3]>, Vec<[Uv; 3]>, Decimated) {
        let (fine, n, step) = (8usize, 7usize, 0.004);
        let size = (n - 1) * fine + 1;
        let spacing = step / fine as f64;
        let flat: Vec<[f64; 3]> = (0..size * size).map(|i| [(i % size) as f64 * spacing, (i / size) as f64 * spacing, 0.0]).collect();
        let dense: Vec<[f64; 3]> = flat.iter().map(|p| [p[0], p[1], wave * (p[0] * 260.0).sin() * (p[1] * 190.0).cos()]).collect();
        let dense_tris = grid(size);
        let uv_of = |p: [f64; 3]| [p[0] as f32 * 50.0, p[1] as f32 * 50.0];
        let dense_uvs: Vec<[Uv; 3]> = dense_tris.iter().map(|t| t.map(|i| uv_of(flat[i as usize]))).collect();
        let mut kept: Vec<u32> = (0..n * n).map(|i| ((i / n) * fine * size + (i % n) * fine) as u32).collect();
        kept[2 * n + 2] = (2 * fine * size + 2 * fine + 3) as u32;
        let triangles = grid(n);
        let triangle_uvs: Vec<[Uv; 3]> = triangles.iter().map(|t| t.map(|g| uv_of(flat[kept[g as usize] as usize]))).collect();
        let owner: Vec<u32> = flat
            .iter()
            .map(|p| (0..kept.len()).min_by(|&a, &b| length(sub(*p, flat[kept[a] as usize])).total_cmp(&length(sub(*p, flat[kept[b] as usize])))).unwrap() as u32)
            .collect();
        (dense, dense_tris, dense_uvs, Decimated { kept, owner, triangles, triangle_uvs, stats: Default::default() })
    }

    fn f3_settings() -> F3Settings {
        F3Settings {
            regular: 1e-3,
            max_move: 0.003,
            smooth: 0.1,
            rounds: 3,
            relax_steps: 6,
            relax_step: 0.5,
            relax_angle_deg: [1.0, 30.0],
            relax_eye_mm: [0.003, 0.008],
            backoff_turn_deg: [25.0, 55.0],
            backoff_step: 0.5,
            uv_ring: 2,
            clear: ClearSettings { along_gaze: false, depth: [0.0003, 0.0006], reach: 0.002, clearance: 0.00005, front_deg: 75.0, rounds: 1, samples: 6 },
            f4: None,
        }
    }

    fn f5_settings() -> F5Settings {
        F5Settings {
            tangential: 0.01,
            smooth: 0.1,
            membrane: 0.1,
            max_move: 0.003,
            uv_ring: 2,
            subset_normals: true,
            eye_clearance: 0.00005,
            eye_depth: [0.0003, 0.0006],
            eye_front_deg: 75.0,
        }
    }

    fn bits(game: &[[f64; 3]], uvs: &[[Uv; 3]]) -> (Vec<[u64; 3]>, Vec<[[u32; 2]; 3]>) {
        (game.iter().map(|p| p.map(f64::to_bits)).collect(), uvs.iter().map(|t| t.map(|uv| uv.map(f32::to_bits))).collect())
    }

    #[test]
    fn the_row_parallel_normals_and_quadrics_equal_the_sequential_ones() {
        let (dense, tris, _, mesh) = fixture(0.002);
        let corners = pl::corner_rows(dense.len(), &tris);
        let all: Vec<[u64; 3]> = pl::angle_weighted_normals(&dense, &tris).iter().map(|n| n.map(f64::to_bits)).collect();
        let rows: Vec<[u64; 3]> = pl::angle_weighted_normals_with(&dense, &tris, &corners).iter().map(|n| n.map(f64::to_bits)).collect();
        assert_eq!(all, rows);
        let at: Vec<[u64; 3]> = pl::angle_weighted_normals_at(&dense, &tris, &corners, &mesh.kept).iter().map(|n| n.map(f64::to_bits)).collect();
        assert_eq!(at, mesh.kept.iter().map(|&d| all[d as usize]).collect::<Vec<_>>());
        let (c0, l0, q0) = pl::cluster_quadrics(&dense, &tris, &mesh);
        let (c1, l1, q1) = pl::cluster_quadrics_with(&dense, &tris, &mesh, &pl::owned_rows(&tris, &mesh));
        assert_eq!(c0.map(f64::to_bits), c1.map(f64::to_bits));
        assert_eq!(l0.iter().map(|p| p.map(f64::to_bits)).collect::<Vec<_>>(), l1.iter().map(|p| p.map(f64::to_bits)).collect::<Vec<_>>());
        assert_eq!(q0.iter().map(|q| q.map(f64::to_bits)).collect::<Vec<_>>(), q1.iter().map(|q| q.map(f64::to_bits)).collect::<Vec<_>>());
    }

    #[test]
    fn the_pruned_searches_return_what_the_full_searches_return() {
        let (dense, tris, uvs, mesh) = fixture(0.002);
        let prepared = Prepared::new(dense.len(), &tris, &uvs, &mesh, 2);
        let game: Vec<[f64; 3]> =
            mesh.kept.iter().enumerate().map(|(g, &d)| [0, 1, 2].map(|k| dense[d as usize][k] + 3e-4 * ((g * 7 + k * 3) % 5) as f64 - 6e-4)).collect();
        let full = pl::project(&dense, &mesh.owner, &game, &mesh, &prepared.rings, 1);
        let pruned = pl::project_pruned(&dense, &mesh.owner, &game, &mesh, &prepared.rings);
        let key = |h: &[(u32, [f64; 3], f64)]| h.iter().map(|x| (x.0, x.1.map(f64::to_bits), x.2.to_bits())).collect::<Vec<_>>();
        assert_eq!(key(&full), key(&pruned));
        let surface = Surface::with_topology(&dense, &tris, &uvs, &mesh, &prepared.surface);
        let spheres = uv_chart::triangle_spheres(&dense, &tris);
        let open = vec![[0.0f32, 0.0, 0.0, f32::INFINITY]; tris.len()];
        let mut searched = 0;
        for g in 0..game.len() {
            let patch = prepared.patches.row(g);
            let a = surface.closest_in(patch, game[g], &open).map(|h| (h.0, h.1.map(f64::to_bits), h.2.to_bits()));
            let b = surface.closest_in(patch, game[g], &spheres).map(|h| (h.0, h.1.map(f64::to_bits), h.2.to_bits()));
            assert_eq!(a, b, "game vertex {g}");
            searched += usize::from(!patch.is_empty());
        }
        assert!(searched > 10, "only {searched} patches searched");
    }

    #[test]
    fn a_reused_cache_places_like_a_fresh_one() {
        let (first, tris, uvs, mesh) = fixture(0.002);
        let (second, _, _, _) = fixture(0.0013);
        let prepared = Prepared::new(first.len(), &tris, &uvs, &mesh, 2);
        assert!(prepared.fits(first.len(), tris.len(), &mesh, 2) && !prepared.fits(first.len(), tris.len(), &mesh, 1));
        let f3 = f3_settings();
        let _ = crate::placement_f3::place_prepared(&first, &tris, &uvs, &mesh, &[], &f3, &prepared);
        let (g_reused, u_reused, _) = crate::placement_f3::place_prepared(&second, &tris, &uvs, &mesh, &[], &f3, &prepared);
        let (g_fresh, u_fresh, _) = crate::placement_f3::place(&second, &tris, &uvs, &mesh, &[], &f3);
        assert_eq!(bits(&g_reused, &u_reused), bits(&g_fresh, &u_fresh));
        let f5 = f5_settings();
        let _ = crate::placement_f5::place_prepared(&first, &tris, &uvs, &mesh, &[], &f5, &prepared);
        let (g_reused, u_reused, _) = crate::placement_f5::place_prepared(&second, &tris, &uvs, &mesh, &[], &f5, &prepared);
        let (g_fresh, u_fresh, _) = crate::placement_f5::place(&second, &tris, &uvs, &mesh, &[], &f5);
        assert_eq!(bits(&g_reused, &u_reused), bits(&g_fresh, &u_fresh));
        assert!(g_fresh.iter().zip(&first).any(|(a, b)| a != b));
    }
}
