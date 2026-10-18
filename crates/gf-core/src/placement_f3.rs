use serde::Serialize;

use crate::decimate::Decimated;
use crate::dense::Uv;
use crate::lids::{self, ClearSettings, ClearStats, Eye};
use crate::placement::{self as pl, Sparse};
use crate::quadric::{dot, length, sub};
use crate::uv_chart::Surface;

#[derive(Clone, Serialize)]
pub struct F3Settings {
    pub regular: f64,
    pub max_move: f64,
    pub smooth: f64,
    pub rounds: usize,
    pub relax_steps: usize,
    pub relax_step: f64,
    pub relax_angle_deg: [f64; 2],
    pub relax_eye_mm: [f64; 2],
    pub backoff_turn_deg: [f64; 2],
    pub backoff_step: f64,
    pub uv_ring: usize,
    pub clear: ClearSettings,
}

#[derive(Default, Serialize)]
pub struct F3Stats {
    pub vertices: usize,
    pub boundary: usize,
    pub seam: usize,
    pub singular: usize,
    pub clamped: usize,
    pub fit_mm: Vec<f64>,
    pub relaxed_vertices: Vec<usize>,
    pub relax_largest_mm: f64,
    pub seam_offset_max_mm: f64,
    pub median_move_mm: f64,
    pub max_move_mm: f64,
    pub clear: Option<ClearStats>,
    pub uv_keys_derived: usize,
    pub uv_seed_misses: usize,
    pub uv_flipped_before: usize,
    pub uv_reverted_vertices: usize,
    pub uv_flipped_after: usize,
}

type Hits = Vec<(u32, [f64; 3], f64)>;

fn solve(points: &[[f64; 3]], hits: &Hits, mesh: &Decimated, fixed: &[bool], fixed_at: &[[f64; 3]], subset: &[[f64; 3]], edges: &[(u32, u32)], smooth: f64) -> Vec<[f64; 3]> {
    let count = mesh.kept.len();
    let mut a = Sparse::new(mesh, count);
    let mut rhs = vec![[0.0f64; 3]; count];
    for (i, &(t, w, _)) in hits.iter().enumerate() {
        if t == u32::MAX {
            continue;
        }
        let tri = mesh.triangles[t as usize];
        let mut target = points[i];
        for s in 0..3 {
            if fixed[tri[s] as usize] {
                for k in 0..3 {
                    target[k] -= w[s] * fixed_at[tri[s] as usize][k];
                }
            }
        }
        for r in 0..3 {
            if fixed[tri[r] as usize] {
                continue;
            }
            for k in 0..3 {
                rhs[tri[r] as usize][k] += w[r] * target[k];
            }
            for s in 0..3 {
                if !fixed[tri[s] as usize] {
                    a.add(tri[r], tri[s], w[r] * w[s]);
                }
            }
        }
    }
    for &(p, q) in edges {
        let e = sub(subset[p as usize], subset[q as usize]);
        let (fp, fq) = (fixed[p as usize], fixed[q as usize]);
        if !fp {
            a.add(p, p, smooth);
            for k in 0..3 {
                rhs[p as usize][k] += smooth * (e[k] + if fq { fixed_at[q as usize][k] } else { 0.0 });
            }
            if !fq {
                a.add(p, q, -smooth);
            }
        }
        if !fq {
            a.add(q, q, smooth);
            for k in 0..3 {
                rhs[q as usize][k] += smooth * (-e[k] + if fp { fixed_at[p as usize][k] } else { 0.0 });
            }
            if !fp {
                a.add(q, p, -smooth);
            }
        }
    }
    let mean_diagonal = (0..count).map(|r| a.vals[r][0]).sum::<f64>() / count as f64;
    let epsilon = 1e-6 * mean_diagonal;
    for g in 0..count {
        if fixed[g] {
            a.vals[g][0] = 1.0;
            rhs[g] = fixed_at[g];
        } else {
            a.add(g as u32, g as u32, epsilon);
            for k in 0..3 {
                rhs[g][k] += epsilon * subset[g][k];
            }
        }
    }
    let mut out = subset.to_vec();
    for g in 0..count {
        if fixed[g] {
            out[g] = fixed_at[g];
        }
    }
    for k in 0..3 {
        let b: Vec<f64> = rhs.iter().map(|r| r[k]).collect();
        let mut x: Vec<f64> = out.iter().map(|p| p[k]).collect();
        pl::conjugate_gradient(&a, &b, &mut x);
        for g in 0..count {
            out[g][k] = x[g];
        }
    }
    out
}

fn seam_vertices(mesh: &Decimated) -> Vec<bool> {
    let mut keys: Vec<Vec<(u32, u32)>> = vec![Vec::new(); mesh.kept.len()];
    for (tri, uvs) in mesh.triangles.iter().zip(&mesh.triangle_uvs) {
        for c in 0..3 {
            let key = (uvs[c][0].to_bits(), uvs[c][1].to_bits());
            let list = &mut keys[tri[c] as usize];
            if !list.contains(&key) {
                list.push(key);
            }
        }
    }
    keys.iter().map(|k| k.len() > 1).collect()
}

fn seam_offsets(points: &[[f64; 3]], hits: &Hits, mesh: &Decimated, game: &[[f64; 3]], seam: &[bool], normals: &[[f64; 3]]) -> Vec<f64> {
    let count = game.len();
    let mut num = vec![0.0f64; count];
    let mut den = vec![0.0f64; count];
    for (i, &(t, w, _)) in hits.iter().enumerate() {
        if t == u32::MAX {
            continue;
        }
        let tri = mesh.triangles[t as usize];
        let at = [0, 1, 2].map(|k| w[0] * game[tri[0] as usize][k] + w[1] * game[tri[1] as usize][k] + w[2] * game[tri[2] as usize][k]);
        let residual = sub(at, points[i]);
        for s in 0..3 {
            let g = tri[s] as usize;
            if seam[g] {
                num[g] += w[s] * dot(residual, normals[g]);
                den[g] += w[s] * w[s];
            }
        }
    }
    (0..count).map(|g| if seam[g] && den[g] > 1e-12 { -num[g] / den[g] } else { 0.0 }).collect()
}

fn smoothstep(x: f64, range: [f64; 2]) -> f64 {
    let t = ((x - range[0]) / (range[1] - range[0]).max(1e-12)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

struct Fit<'a> {
    mesh: &'a Decimated,
    fixed: Vec<bool>,
    seam: Vec<bool>,
    normals: Vec<[f64; 3]>,
    edges: Vec<(u32, u32)>,
    vtris: Vec<Vec<u32>>,
    neighbours: Vec<Vec<u32>>,
    subset_normals: Vec<Option<[f64; 3]>>,
}

impl Fit<'_> {
    fn once(&self, points: &[[f64; 3]], hits: &Hits, subset: &[[f64; 3]], smooth: f64) -> (Vec<[f64; 3]>, f64) {
        let mut at = subset.to_vec();
        let first = solve(points, hits, self.mesh, &self.fixed, &at, subset, &self.edges, smooth);
        let h = seam_offsets(points, hits, self.mesh, &first, &self.seam, &self.normals);
        let mut largest = 0.0f64;
        for g in 0..at.len() {
            if self.seam[g] {
                at[g] = [0, 1, 2].map(|k| subset[g][k] + h[g] * self.normals[g][k]);
                largest = largest.max(h[g].abs());
            }
        }
        (solve(points, hits, self.mesh, &self.fixed, &at, subset, &self.edges, smooth), largest)
    }

    fn sliver_step(&self, game: &[[f64; 3]], g: usize, settings: &F3Settings) -> Option<[f64; 3]> {
        let worst = self.vtris[g]
            .iter()
            .map(|&t| pl::min_angle(self.mesh.triangles[t as usize].map(|i| game[i as usize])))
            .fold(180.0f64, f64::min);
        let a = 1.0 - smoothstep(worst, settings.relax_angle_deg);
        if a <= 0.0 {
            return None;
        }
        let ring = &self.neighbours[g];
        let c = ring.iter().fold([0.0; 3], |s, &w| [s[0] + game[w as usize][0], s[1] + game[w as usize][1], s[2] + game[w as usize][2]]);
        let c = c.map(|x| x / ring.len() as f64);
        let n = self.normals[g];
        let d = sub(c, game[g]);
        let d = sub(d, n.map(|x| x * dot(d, n)));
        Some(d.map(|x| x * settings.relax_step * a))
    }

    fn back_off(&self, game: &[[f64; 3]], g: usize, settings: &F3Settings) -> f64 {
        self.vtris[g]
            .iter()
            .filter_map(|&t| {
                let m = self.subset_normals[t as usize]?;
                let n = pl::triangle_normal(self.mesh.triangles[t as usize].map(|i| game[i as usize]));
                let l = length(n);
                let turn = if l < 1e-20 { 180.0 } else { (dot(n, m) / l).clamp(-1.0, 1.0).acos().to_degrees() };
                Some(smoothstep(turn, settings.backoff_turn_deg))
            })
            .fold(0.0, f64::max)
    }

    fn relax(&self, game: &mut [[f64; 3]], subset: &[[f64; 3]], eyes: &[Eye], settings: &F3Settings, stats: &mut F3Stats) {
        let away: Vec<f64> = subset
            .iter()
            .map(|&p| eyes.iter().map(|e| smoothstep(length(sub(p, e.centre)) - e.radius, settings.relax_eye_mm)).fold(1.0, f64::min))
            .collect();
        for _ in 0..settings.relax_steps {
            let moves: Vec<Option<[f64; 3]>> = (0..game.len())
                .map(|g| {
                    if self.fixed[g] || self.neighbours[g].is_empty() {
                        return None;
                    }
                    let sliver = self.sliver_step(game, g, settings).map(|d| d.map(|x| x * away[g]));
                    let b = self.back_off(game, g, settings);
                    if sliver.is_none() && b <= 0.0 {
                        return None;
                    }
                    let d = sliver.unwrap_or([0.0; 3]);
                    let back = sub(subset[g], game[g]);
                    Some([0, 1, 2].map(|k| d[k] + back[k] * settings.backoff_step * b))
                })
                .collect();
            let mut moved = 0;
            for (g, m) in moves.iter().enumerate() {
                if let Some(m) = m {
                    for k in 0..3 {
                        game[g][k] += m[k];
                    }
                    stats.relax_largest_mm = stats.relax_largest_mm.max(length(*m) * 1000.0);
                    moved += 1;
                }
            }
            stats.relaxed_vertices.push(moved);
        }
    }
}

fn mean_gap_mm(hits: &Hits) -> f64 {
    let valid: Vec<f64> = hits.iter().filter(|h| h.0 != u32::MAX).map(|h| h.2.sqrt()).collect();
    valid.iter().sum::<f64>() / valid.len().max(1) as f64 * 1000.0
}

fn uv_sign(t: [Uv; 3]) -> f64 {
    ((t[1][0] as f64 - t[0][0] as f64) * (t[2][1] as f64 - t[0][1] as f64) - (t[2][0] as f64 - t[0][0] as f64) * (t[1][1] as f64 - t[0][1] as f64)).signum()
}

pub fn place(
    positions: &[[f64; 3]],
    dense_triangles: &[[u32; 3]],
    dense_uvs: &[[Uv; 3]],
    mesh: &Decimated,
    eyes: &[Eye],
    settings: &F3Settings,
) -> (Vec<[f64; 3]>, Vec<[Uv; 3]>, F3Stats) {
    let count = mesh.kept.len();
    let boundary = pl::boundary_vertices(mesh);
    let seam: Vec<bool> = seam_vertices(mesh).iter().zip(&boundary).map(|(&s, &b)| s && !b).collect();
    let fixed: Vec<bool> = (0..count).map(|g| boundary[g] || seam[g]).collect();
    let mut stats = F3Stats {
        vertices: count,
        boundary: boundary.iter().filter(|&&b| b).count(),
        seam: seam.iter().filter(|&&s| s).count(),
        ..Default::default()
    };
    let points = positions;
    let dense_normals = pl::angle_weighted_normals(points, dense_triangles);
    let normals: Vec<[f64; 3]> = mesh.kept.iter().map(|&d| dense_normals[d as usize]).collect();
    let dense_clear = (!eyes.is_empty()).then(|| lids::dense_clearance(points, &dense_normals, eyes, settings.clear.front_deg));
    drop(dense_normals);
    let vtris = pl::vertex_triangles(mesh, count);
    let rings = pl::ring_triangles(mesh, &vtris);
    let subset: Vec<[f64; 3]> = mesh.kept.iter().map(|&d| points[d as usize]).collect();
    let subset_normals = mesh
        .triangles
        .iter()
        .map(|tri| {
            let n = pl::triangle_normal(tri.map(|i| subset[i as usize]));
            let l = length(n);
            (l >= 1e-20).then(|| n.map(|x| x / l))
        })
        .collect();
    let fit = Fit {
        mesh,
        fixed: fixed.clone(),
        seam: seam.clone(),
        normals,
        edges: pl::unique_edges(mesh),
        neighbours: crate::placement_f2::neighbours(mesh),
        vtris,
        subset_normals,
    };

    let (centre, local, sums) = pl::cluster_quadrics(points, dense_triangles, mesh);
    let mut game: Vec<[f64; 3]> = (0..count)
        .map(|g| {
            if boundary[g] {
                return subset[g];
            }
            let anchor = local[mesh.kept[g] as usize];
            let q = &sums[g];
            let Some(x) = pl::solve(q, anchor, settings.regular * (q[0] + q[4] + q[7]) / 3.0) else {
                stats.singular += 1;
                return subset[g];
            };
            let mut d = sub(x, anchor);
            if seam[g] {
                let n = fit.normals[g];
                d = n.map(|c| c * dot(d, n));
            }
            let l = length(d);
            if l > settings.max_move {
                d = d.map(|c| c * settings.max_move / l);
            }
            [0, 1, 2].map(|k| anchor[k] + d[k] + centre[k])
        })
        .collect();
    let mut largest = 0.0;
    for _ in 0..settings.rounds {
        let hits = pl::project(points, &mesh.owner, &game, mesh, &rings, 1);
        stats.fit_mm.push(mean_gap_mm(&hits));
        (game, largest) = fit.once(points, &hits, &subset, settings.smooth);
    }
    stats.seam_offset_max_mm = largest * 1000.0;
    stats.clamped = pl::clamp_moves(&mut game, &subset, settings.max_move);
    let surface = Surface::new(points, dense_triangles, dense_uvs, mesh, settings.uv_ring);
    let mut key: Vec<Option<Uv>> = vec![None; count];
    for (tri, uvs) in mesh.triangles.iter().zip(&mesh.triangle_uvs) {
        for c in 0..3 {
            key[tri[c] as usize].get_or_insert(uvs[c]);
        }
    }
    fit.relax(&mut game, &subset, eyes, settings, &mut stats);
    if let Some(dense_clear) = &dense_clear {
        let keep: Vec<bool> = dense_clear.iter().map(|c| c.is_finite()).collect();
        let grid = lids::PointGrid::new(points, &keep, 0.001);
        let reach = settings.clear.reach;
        let floor = |s: [f64; 3]| grid.envelope(points, dense_clear, s, reach);
        stats.clear = Some(lids::clear_game(&mut game, mesh, &boundary, eyes, floor, &settings.clear));
    }
    let mut moves: Vec<f64> = game.iter().zip(&subset).map(|(a, b)| length(sub(*a, *b))).collect();
    moves.sort_by(f64::total_cmp);
    stats.median_move_mm = moves[moves.len() / 2] * 1000.0;
    stats.max_move_mm = moves[moves.len() - 1] * 1000.0;

    let derived: Vec<Option<Uv>> = (0..count)
        .map(|g| {
            if fixed[g] {
                return None;
            }
            surface.closest_on_patch(g as u32, game[g], key[g]?).map(|hit| surface.uv(hit))
        })
        .collect();
    stats.uv_keys_derived = derived.iter().filter(|d| d.is_some()).count();
    stats.uv_seed_misses = (0..count).filter(|&g| !fixed[g] && derived[g].is_none()).count();
    let mut use_derived: Vec<bool> = derived.iter().map(|d| d.is_some()).collect();
    let build = |use_derived: &[bool]| -> Vec<[Uv; 3]> {
        mesh.triangles
            .iter()
            .zip(&mesh.triangle_uvs)
            .map(|(tri, uvs)| [0, 1, 2].map(|c| if use_derived[tri[c] as usize] { derived[tri[c] as usize].unwrap() } else { uvs[c] }))
            .collect()
    };
    let mut uvs = build(&use_derived);
    let flipped = |uvs: &[[Uv; 3]]| -> Vec<usize> {
        (0..uvs.len()).filter(|&t| uv_sign(mesh.triangle_uvs[t]) != 0.0 && uv_sign(uvs[t]) != uv_sign(mesh.triangle_uvs[t])).collect()
    };
    let mut bad = flipped(&uvs);
    stats.uv_flipped_before = bad.len();
    for _ in 0..64 {
        if bad.is_empty() {
            break;
        }
        for &t in &bad {
            for &g in &mesh.triangles[t] {
                if use_derived[g as usize] {
                    use_derived[g as usize] = false;
                    stats.uv_reverted_vertices += 1;
                }
            }
        }
        uvs = build(&use_derived);
        bad = flipped(&uvs);
    }
    stats.uv_flipped_after = bad.len();
    (game, uvs, stats)
}
