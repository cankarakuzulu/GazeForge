use serde::Serialize;

use crate::decimate::Decimated;
use crate::Uv;
use crate::lids::{self, ClearSettings, ClearStats, Eye};
use crate::placement::{self as pl, Buckets};
use crate::prepared::Prepared;
use crate::quadric::{dot, length, sub};
use crate::uv_chart::{self, Surface};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub f4: Option<F4Settings>,
}

#[derive(Clone, Serialize)]
pub struct F4Settings {
    pub target_deg: f64,
    pub passes: usize,
    pub step: f64,
    pub gain: f64,
    pub fold_deg: f64,
    pub backoff_dense: bool,
    pub game_plane: bool,
}

#[derive(Default, Serialize)]
pub struct F4Stats {
    pub moved_per_pass: Vec<usize>,
    pub below_target_before: usize,
    pub below_target_after: usize,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub f4: Option<F4Stats>,
}

type Hits = Vec<(u32, [f64; 3], f64)>;

struct Matrix<'a> {
    cols: &'a [Vec<u32>],
    vals: Vec<Vec<f64>>,
    epsilon: f64,
}

fn hit_rows(hits: &Hits, mesh: &Decimated, count: usize) -> Buckets {
    Buckets::build(count, hits.len(), |i, push| {
        let t = hits[i].0;
        if t == u32::MAX {
            return;
        }
        let tri = mesh.triangles[t as usize];
        for r in 0..3 {
            push(tri[r] as usize, (i * 3 + r) as u32);
        }
    })
}

fn matrix<'a>(hits: &Hits, rows: &Buckets, mesh: &Decimated, prepared: &Prepared, pattern: &'a [Vec<u32>], smooth: f64) -> Matrix<'a> {
    let fixed = &prepared.fixed;
    let count = fixed.len();
    let mut vals: Vec<Vec<f64>> = crate::map_range(count, |g| {
        let cols = &pattern[g];
        let mut v = vec![0.0f64; cols.len()];
        if fixed[g] {
            return v;
        }
        let at = |c: u32| cols.iter().position(|&x| x == c).unwrap();
        for e in rows.row(g) {
            let (i, r) = ((e / 3) as usize, (e % 3) as usize);
            let (t, w, _) = hits[i];
            let tri = mesh.triangles[t as usize];
            for s in 0..3 {
                if !fixed[tri[s] as usize] {
                    v[at(tri[s])] += w[r] * w[s];
                }
            }
        }
        for &e in prepared.edge_rows.row(g) {
            let (p, q) = prepared.edges[(e / 2) as usize];
            let other = if e % 2 == 0 { q } else { p };
            v[0] += smooth;
            if !fixed[other as usize] {
                v[at(other)] += -smooth;
            }
        }
        v
    });
    let mean_diagonal = (0..count).map(|r| vals[r][0]).sum::<f64>() / count as f64;
    let epsilon = 1e-6 * mean_diagonal;
    for g in 0..count {
        if fixed[g] {
            vals[g][0] = 1.0;
        } else {
            vals[g][0] += epsilon;
        }
    }
    Matrix { cols: pattern, vals, epsilon }
}

fn rhs(points: &[[f64; 3]], hits: &Hits, rows: &Buckets, mesh: &Decimated, prepared: &Prepared, fixed_at: &[[f64; 3]], subset: &[[f64; 3]], smooth: f64, epsilon: f64) -> Vec<[f64; 3]> {
    let fixed = &prepared.fixed;
    crate::map_range(fixed.len(), |g| {
        if fixed[g] {
            return fixed_at[g];
        }
        let mut b = [0.0f64; 3];
        for e in rows.row(g) {
            let (i, r) = ((e / 3) as usize, (e % 3) as usize);
            let (t, w, _) = hits[i];
            let tri = mesh.triangles[t as usize];
            let mut target = points[i];
            for s in 0..3 {
                if fixed[tri[s] as usize] {
                    for k in 0..3 {
                        target[k] -= w[s] * fixed_at[tri[s] as usize][k];
                    }
                }
            }
            for k in 0..3 {
                b[k] += w[r] * target[k];
            }
        }
        for &e in prepared.edge_rows.row(g) {
            let (p, q) = prepared.edges[(e / 2) as usize];
            let d = sub(subset[p as usize], subset[q as usize]);
            if e % 2 == 0 {
                let fq = fixed[q as usize];
                for k in 0..3 {
                    b[k] += smooth * (d[k] + if fq { fixed_at[q as usize][k] } else { 0.0 });
                }
            } else {
                let fp = fixed[p as usize];
                for k in 0..3 {
                    b[k] += smooth * (-d[k] + if fp { fixed_at[p as usize][k] } else { 0.0 });
                }
            }
        }
        for k in 0..3 {
            b[k] += epsilon * subset[g][k];
        }
        b
    })
}

fn solve(a: &Matrix, b: &[[f64; 3]], subset: &[[f64; 3]], fixed: &[bool], fixed_at: &[[f64; 3]]) -> Vec<[f64; 3]> {
    let mut out = subset.to_vec();
    for g in 0..out.len() {
        if fixed[g] {
            out[g] = fixed_at[g];
        }
    }
    let axes: Vec<Vec<f64>> = crate::map_range(3, |k| {
        let column: Vec<f64> = b.iter().map(|r| r[k]).collect();
        let mut x: Vec<f64> = out.iter().map(|p| p[k]).collect();
        pl::conjugate_gradient_rows(a.cols, &a.vals, &column, &mut x);
        x
    });
    for k in 0..3 {
        for g in 0..out.len() {
            out[g][k] = axes[k][g];
        }
    }
    out
}

pub(crate) fn seam_vertices(mesh: &Decimated) -> Vec<bool> {
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

fn seam_offsets(points: &[[f64; 3]], hits: &Hits, rows: &Buckets, mesh: &Decimated, game: &[[f64; 3]], seam: &[bool], normals: &[[f64; 3]]) -> Vec<f64> {
    crate::map_range(game.len(), |g| {
        if !seam[g] {
            return 0.0;
        }
        let (mut num, mut den) = (0.0f64, 0.0f64);
        for e in rows.row(g) {
            let (i, s) = ((e / 3) as usize, (e % 3) as usize);
            let (t, w, _) = hits[i];
            let tri = mesh.triangles[t as usize];
            let at = [0, 1, 2].map(|k| w[0] * game[tri[0] as usize][k] + w[1] * game[tri[1] as usize][k] + w[2] * game[tri[2] as usize][k]);
            let residual = sub(at, points[i]);
            num += w[s] * dot(residual, normals[g]);
            den += w[s] * w[s];
        }
        if den > 1e-12 { -num / den } else { 0.0 }
    })
}

fn smoothstep(x: f64, range: [f64; 2]) -> f64 {
    let t = ((x - range[0]) / (range[1] - range[0]).max(1e-12)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

struct Fit<'a> {
    mesh: &'a Decimated,
    prepared: &'a Prepared,
    pattern: &'a [Vec<u32>],
    normals: Vec<[f64; 3]>,
    subset_normals: Vec<Option<[f64; 3]>>,
    dense_refs: Option<Vec<[f64; 3]>>,
}

fn small_angle_penalty(p: [[f64; 3]; 3], target: f64) -> f64 {
    crate::quadric::corner_angles(p).iter().map(|&a| (target - a).max(0.0).powi(2)).sum()
}

fn tangent_basis(n: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let helper = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    let t1 = crate::quadric::cross(n, helper);
    let l = length(t1).max(1e-30);
    let t1 = t1.map(|x| x / l);
    (t1, crate::quadric::cross(n, t1))
}

impl Fit<'_> {
    fn once(&self, points: &[[f64; 3]], hits: &Hits, subset: &[[f64; 3]], smooth: f64) -> (Vec<[f64; 3]>, f64) {
        let prepared = self.prepared;
        let rows = hit_rows(hits, self.mesh, prepared.fixed.len());
        let a = matrix(hits, &rows, self.mesh, prepared, self.pattern, smooth);
        let mut at = subset.to_vec();
        let first = solve(&a, &rhs(points, hits, &rows, self.mesh, prepared, &at, subset, smooth, a.epsilon), subset, &prepared.fixed, &at);
        let h = seam_offsets(points, hits, &rows, self.mesh, &first, &prepared.seam, &self.normals);
        let mut largest = 0.0f64;
        for g in 0..at.len() {
            if prepared.seam[g] {
                at[g] = [0, 1, 2].map(|k| subset[g][k] + h[g] * self.normals[g][k]);
                largest = largest.max(h[g].abs());
            }
        }
        (solve(&a, &rhs(points, hits, &rows, self.mesh, prepared, &at, subset, smooth, a.epsilon), subset, &prepared.fixed, &at), largest)
    }

    fn sliver_step(&self, game: &[[f64; 3]], g: usize, settings: &F3Settings) -> Option<[f64; 3]> {
        let worst = self.prepared.vtris[g]
            .iter()
            .map(|&t| pl::min_angle(self.mesh.triangles[t as usize].map(|i| game[i as usize])))
            .fold(180.0f64, f64::min);
        let a = 1.0 - smoothstep(worst, settings.relax_angle_deg);
        if a <= 0.0 {
            return None;
        }
        let ring = &self.prepared.neighbours[g];
        let c = ring.iter().fold([0.0; 3], |s, &w| [s[0] + game[w as usize][0], s[1] + game[w as usize][1], s[2] + game[w as usize][2]]);
        let c = c.map(|x| x / ring.len() as f64);
        let n = self.normals[g];
        let d = sub(c, game[g]);
        let d = sub(d, n.map(|x| x * dot(d, n)));
        Some(d.map(|x| x * settings.relax_step * a))
    }

    fn back_off(&self, game: &[[f64; 3]], g: usize, settings: &F3Settings) -> f64 {
        self.prepared.vtris[g]
            .iter()
            .filter_map(|&t| {
                let m = match &self.dense_refs {
                    Some(refs) => refs[t as usize],
                    None => self.subset_normals[t as usize]?,
                };
                let n = pl::triangle_normal(self.mesh.triangles[t as usize].map(|i| game[i as usize]));
                let l = length(n);
                let turn = if l < 1e-20 { 180.0 } else { (dot(n, m) / l).clamp(-1.0, 1.0).acos().to_degrees() };
                Some(smoothstep(turn, settings.backoff_turn_deg))
            })
            .fold(0.0, f64::max)
    }

    fn worst_at(&self, game: &[[f64; 3]], g: usize) -> f64 {
        self.prepared.vtris[g].iter().map(|&t| pl::min_angle(self.mesh.triangles[t as usize].map(|i| game[i as usize]))).fold(180.0f64, f64::min)
    }

    fn star_with(&self, game: &[[f64; 3]], g: usize, p: [f64; 3], t: u32) -> [[f64; 3]; 3] {
        self.mesh.triangles[t as usize].map(|i| if i as usize == g { p } else { game[i as usize] })
    }

    fn angle_pass(&self, game: &mut [[f64; 3]], away: &[f64], f4: &F4Settings, refs: &[[f64; 3]]) -> F4Stats {
        let mut stats = F4Stats::default();
        let below = |game: &[[f64; 3]]| (0..game.len()).filter(|&g| !self.prepared.fixed[g] && self.worst_at(game, g) < f4.target_deg).count();
        stats.below_target_before = below(game);
        for _ in 0..f4.passes {
            let mut moved = 0;
            for g in 0..game.len() {
                if self.prepared.fixed[g] || self.prepared.neighbours[g].is_empty() || away[g] <= 0.0 || self.worst_at(game, g) >= f4.target_deg {
                    continue;
                }
                let tris = &self.prepared.vtris[g];
                let penalty = |p: [f64; 3]| tris.iter().map(|&t| small_angle_penalty(self.star_with(game, g, p, t), f4.target_deg)).sum::<f64>();
                let fold = |p: [f64; 3]| tris.iter().map(|&t| pl::deviation_deg(self.star_with(game, g, p, t), refs[t as usize])).fold(0.0f64, f64::max);
                let ring = &self.prepared.neighbours[g];
                let edge = ring.iter().map(|&w| length(sub(game[w as usize], game[g]))).sum::<f64>() / ring.len() as f64;
                let plane = if f4.game_plane {
                    let n = tris.iter().fold([0.0; 3], |a, &t| {
                        let m = pl::triangle_normal(self.mesh.triangles[t as usize].map(|i| game[i as usize]));
                        [a[0] + m[0], a[1] + m[1], a[2] + m[2]]
                    });
                    let l = length(n);
                    if l > 1e-30 { n.map(|x| x / l) } else { self.normals[g] }
                } else {
                    self.normals[g]
                };
                let (t1, t2) = tangent_basis(plane);
                let p0 = game[g];
                let h = 1e-3 * edge;
                let at = |d: [f64; 3], s: f64| [0, 1, 2].map(|k| p0[k] + s * d[k]);
                let gx = (penalty(at(t1, h)) - penalty(at(t1, -h))) / (2.0 * h);
                let gy = (penalty(at(t2, h)) - penalty(at(t2, -h))) / (2.0 * h);
                let gl = (gx * gx + gy * gy).sqrt();
                if !(gl > 1e-30) {
                    continue;
                }
                let dir = [0, 1, 2].map(|k| -(gx * t1[k] + gy * t2[k]) / gl);
                let (f0, fold0) = (penalty(p0), fold(p0).max(f4.fold_deg));
                let newton = f4.gain * edge * edge * gl / (2.0 * (180.0 / std::f64::consts::PI).powi(2));
                let mut s = newton.min(f4.step * edge) * away[g];
                for _ in 0..2 {
                    let p = at(dir, s);
                    if penalty(p) < f0 && fold(p) <= fold0 {
                        game[g] = p;
                        moved += 1;
                        break;
                    }
                    s *= 0.5;
                }
            }
            stats.moved_per_pass.push(moved);
            if moved == 0 {
                break;
            }
        }
        stats.below_target_after = below(game);
        stats
    }

    fn away(&self, subset: &[[f64; 3]], eyes: &[Eye], settings: &F3Settings) -> Vec<f64> {
        subset
            .iter()
            .map(|&p| eyes.iter().map(|e| smoothstep(length(sub(p, e.centre)) - e.radius, settings.relax_eye_mm)).fold(1.0, f64::min))
            .collect()
    }

    fn relax(&self, game: &mut [[f64; 3]], subset: &[[f64; 3]], eyes: &[Eye], settings: &F3Settings, stats: &mut F3Stats) {
        let away = self.away(subset, eyes, settings);
        let prepared = self.prepared;
        let mut active: Vec<usize> = (0..game.len()).collect();
        let mut touched = vec![false; game.len()];
        for _ in 0..settings.relax_steps {
            let view: &[[f64; 3]] = game;
            let moves: Vec<Option<[f64; 3]>> = crate::map_each(&active, |g| {
                if prepared.fixed[g] || prepared.neighbours[g].is_empty() {
                    return None;
                }
                let sliver = self.sliver_step(view, g, settings).map(|d| d.map(|x| x * away[g]));
                let b = self.back_off(view, g, settings);
                if sliver.is_none() && b <= 0.0 {
                    return None;
                }
                let d = sliver.unwrap_or([0.0; 3]);
                let back = sub(subset[g], view[g]);
                Some([0, 1, 2].map(|k| d[k] + back[k] * settings.backoff_step * b))
            });
            let mut moved = 0;
            for (&g, m) in active.iter().zip(&moves) {
                if let Some(m) = m {
                    for k in 0..3 {
                        game[g][k] += m[k];
                    }
                    stats.relax_largest_mm = stats.relax_largest_mm.max(length(*m) * 1000.0);
                    moved += 1;
                    for &t in &prepared.vtris[g] {
                        for &w in &self.mesh.triangles[t as usize] {
                            touched[w as usize] = true;
                        }
                    }
                    for &w in &prepared.neighbours[g] {
                        touched[w as usize] = true;
                    }
                }
            }
            stats.relaxed_vertices.push(moved);
            active = (0..game.len()).filter(|&g| touched[g]).collect();
            touched.fill(false);
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
    let prepared = Prepared::new(positions.len(), dense_triangles, dense_uvs, mesh, settings.uv_ring);
    place_prepared(positions, dense_triangles, dense_uvs, mesh, eyes, settings, &prepared)
}

pub fn place_prepared(
    positions: &[[f64; 3]],
    dense_triangles: &[[u32; 3]],
    dense_uvs: &[[Uv; 3]],
    mesh: &Decimated,
    eyes: &[Eye],
    settings: &F3Settings,
    prepared: &Prepared,
) -> (Vec<[f64; 3]>, Vec<[Uv; 3]>, F3Stats) {
    let count = mesh.kept.len();
    let (boundary, seam, fixed) = (&prepared.boundary, &prepared.seam, &prepared.fixed);
    let mut stats = F3Stats {
        vertices: count,
        boundary: boundary.iter().filter(|&&b| b).count(),
        seam: seam.iter().filter(|&&s| s).count(),
        ..Default::default()
    };
    let points = positions;
    let (normals, dense_clear) = if eyes.is_empty() {
        (pl::angle_weighted_normals_at(points, dense_triangles, &prepared.corners, &mesh.kept), None)
    } else {
        let dense_normals = pl::angle_weighted_normals_with(points, dense_triangles, &prepared.corners);
        let normals: Vec<[f64; 3]> = mesh.kept.iter().map(|&d| dense_normals[d as usize]).collect();
        (normals, Some(lids::dense_clearance(points, &dense_normals, eyes, settings.clear.front_deg)))
    };
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
    let fit = Fit { mesh, prepared, pattern: prepared.f3_pattern(mesh), normals, subset_normals, dense_refs: None };
    let refs = pl::triangle_references(mesh, &fit.normals);
    let fit = Fit { dense_refs: settings.f4.as_ref().filter(|f| f.backoff_dense).map(|_| refs.clone()), ..fit };

    let (centre, local, sums) = pl::cluster_quadrics_with(points, dense_triangles, mesh, &prepared.owned);
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
    drop(local);
    let mut largest = 0.0;
    for _ in 0..settings.rounds {
        crate::cancel::point();
        let hits = pl::project_pruned(points, &mesh.owner, &game, mesh, &prepared.rings);
        stats.fit_mm.push(mean_gap_mm(&hits));
        (game, largest) = fit.once(points, &hits, &subset, settings.smooth);
    }
    stats.seam_offset_max_mm = largest * 1000.0;
    stats.clamped = pl::clamp_moves(&mut game, &subset, settings.max_move);
    crate::cancel::point();
    let surface = Surface::with_topology(points, dense_triangles, dense_uvs, mesh, &prepared.surface);
    fit.relax(&mut game, &subset, eyes, settings, &mut stats);
    if let Some(f4) = &settings.f4 {
        let away = fit.away(&subset, eyes, settings);
        stats.f4 = Some(fit.angle_pass(&mut game, &away, f4, &refs));
    }
    drop(refs);
    if let Some(dense_clear) = &dense_clear {
        let keep: Vec<bool> = dense_clear.iter().map(|c| c.is_finite()).collect();
        let grid = lids::PointGrid::new(points, &keep, 0.001);
        let reach = settings.clear.reach;
        let floor = |s: [f64; 3]| grid.envelope(points, dense_clear, s, reach);
        stats.clear = Some(lids::clear_game(&mut game, mesh, boundary, eyes, floor, &settings.clear));
    }
    let mut moves: Vec<f64> = game.iter().zip(&subset).map(|(a, b)| length(sub(*a, *b))).collect();
    moves.sort_by(f64::total_cmp);
    stats.median_move_mm = moves[moves.len() / 2] * 1000.0;
    stats.max_move_mm = moves[moves.len() - 1] * 1000.0;

    let spheres = uv_chart::triangle_spheres(points, dense_triangles);
    let derived: Vec<Option<Uv>> = crate::map_range(count, |g| {
        if fixed[g] {
            return None;
        }
        surface.closest_in(prepared.patches.row(g), game[g], &spheres).map(|hit| surface.uv(hit))
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn lattice(n: usize, spacing: f64) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
        let points = (0..n * n).map(|i| [(i % n) as f64 * spacing, (i / n) as f64 * spacing, 0.0]).collect();
        let mut tris = Vec::new();
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let a = (y * n + x) as u32;
                tris.push([a, a + 1, a + n as u32 + 1]);
                tris.push([a, a + n as u32 + 1, a + n as u32]);
            }
        }
        (points, tris)
    }

    fn settings(f4: Option<F4Settings>) -> F3Settings {
        F3Settings {
            regular: 1e-3,
            max_move: 0.5,
            smooth: 0.1,
            rounds: 0,
            relax_steps: 0,
            relax_step: 0.5,
            relax_angle_deg: [1.0, 8.0],
            relax_eye_mm: [0.003, 0.008],
            backoff_turn_deg: [25.0, 55.0],
            backoff_step: 0.5,
            uv_ring: 2,
            clear: ClearSettings { along_gaze: false, depth: [0.0003, 0.0006], reach: 0.002, clearance: 0.00005, front_deg: 75.0, rounds: 1, samples: 6 },
            f4,
        }
    }

    fn min_angle_of(game: &[[f64; 3]], mesh: &Decimated) -> f64 {
        mesh.triangles.iter().map(|t| pl::min_angle(t.map(|i| game[i as usize]))).fold(180.0, f64::min)
    }

    #[test]
    fn the_angle_pass_opens_a_planted_sliver_with_fit_and_relaxation_off() {
        let (fine, n, step) = (16usize, 6usize, 0.004);
        let size = (n - 1) * fine + 1;
        let (dense, dense_tris) = lattice(size, step / fine as f64);
        let uv_of = |p: [f64; 3]| [p[0] as f32 * 50.0, p[1] as f32 * 50.0];
        let dense_uvs: Vec<[Uv; 3]> = dense_tris.iter().map(|t| t.map(|i| uv_of(dense[i as usize]))).collect();
        let mut kept: Vec<u32> = (0..n * n).map(|i| ((i / n) * fine * size + (i % n) * fine) as u32).collect();
        kept[2 * n + 2] = (40 * size + 47) as u32;
        let (_, triangles) = lattice(n, step);
        let triangle_uvs: Vec<[Uv; 3]> = triangles.iter().map(|t| t.map(|g| uv_of(dense[kept[g as usize] as usize]))).collect();
        let owner: Vec<u32> = dense
            .iter()
            .map(|p| {
                (0..kept.len())
                    .min_by(|&a, &b| length(sub(*p, dense[kept[a] as usize])).total_cmp(&length(sub(*p, dense[kept[b] as usize]))))
                    .unwrap() as u32
            })
            .collect();
        let mesh = Decimated { kept, owner, triangles, triangle_uvs, stats: Default::default() };
        let (f3_game, _, _) = place(&dense, &dense_tris, &dense_uvs, &mesh, &[], &settings(None));
        let f4 = F4Settings { target_deg: 10.0, passes: 12, step: 0.25, gain: 0.5, fold_deg: 60.0, backoff_dense: true, game_plane: true };
        let (f4_game, uvs, stats) = place(&dense, &dense_tris, &dense_uvs, &mesh, &[], &settings(Some(f4)));
        let (before, after) = (min_angle_of(&f3_game, &mesh), min_angle_of(&f4_game, &mesh));
        assert!(before < 9.0, "the plant must leave a sliver under f3, got {before}");
        assert!(after > before + 1.0, "f4 {after} vs f3 {before}");
        assert!(stats.f4.unwrap().below_target_after < 4);
        assert_eq!(uvs.len(), mesh.triangles.len());
    }
}
