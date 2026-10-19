use serde::Serialize;

use crate::decimate::Decimated;
use crate::Uv;
use crate::placement::{self as pl, FitSettings, PlacementStats};
use crate::quadric;
use crate::uv_chart::{self, FlipGuard, Surface, UvStats};

#[derive(Clone, Serialize)]
pub struct F2Settings {
    pub fit: FitSettings,
    pub regular: f64,
    pub max_turn_deg: f64,
    pub turn_margin_deg: f64,
    pub confident_spread_deg: f64,
    pub confident_above_deg: f64,
    pub angle_margin_deg: f64,
    pub repair_rounds: usize,
    pub repair_max_step: f64,
    pub repair_max_step_confident: f64,
    pub uv_ring: usize,
}

#[derive(Default, Serialize)]
pub struct F2Stats {
    pub placement: PlacementStats,
    pub singular: usize,
    pub clamped_initial: usize,
    pub clamped_final: usize,
    pub confident_triangles: usize,
    pub folded_at_subset: usize,
    pub over_turn_before_repair: usize,
    pub orientation_moves: usize,
    pub angle_moves: usize,
    pub unresolved_orientation: usize,
    pub unresolved_angle: usize,
    pub over_turn_left: Vec<(u32, f64)>,
    pub uv: UvStats,
    pub uv_flip_guard: FlipGuard,
}

struct Context<'a> {
    mesh: &'a Decimated,
    boundary: Vec<bool>,
    vtris: Vec<Vec<u32>>,
    neighbours: Vec<Vec<u32>>,
    references: Vec<[f64; 3]>,
    vertex_normals: Vec<[f64; 3]>,
    subset: Vec<[f64; 3]>,
    subset_cos: Vec<f64>,
    confident: Vec<bool>,
}

impl Context<'_> {
    fn deviation(&self, game: &[[f64; 3]], t: u32) -> f64 {
        pl::deviation_deg(self.mesh.triangles[t as usize].map(|i| game[i as usize]), self.references[t as usize])
    }

    fn cosine(&self, game: &[[f64; 3]], t: u32) -> Option<f64> {
        let n = pl::triangle_normal(self.mesh.triangles[t as usize].map(|i| game[i as usize]));
        let l = quadric::length(n);
        (l >= 1e-20).then(|| quadric::dot(n, self.references[t as usize]) / l)
    }

    fn upright(&self, game: &[[f64; 3]], t: u32, cos_turn: f64) -> bool {
        if !self.confident[t as usize] {
            return true;
        }
        self.cosine(game, t).is_some_and(|c| c >= cos_turn || c >= self.subset_cos[t as usize] - 1e-12)
    }
}

struct StarState {
    deviation: f64,
    min: f64,
    max: f64,
    area: f64,
}

fn star_state(ctx: &Context, game: &[[f64; 3]], star: &[u32]) -> Vec<StarState> {
    star.iter()
        .map(|&s| {
            let p = ctx.mesh.triangles[s as usize].map(|i| game[i as usize]);
            let corners = quadric::corner_angles(p);
            StarState {
                deviation: ctx.deviation(game, s),
                min: corners.iter().cloned().fold(180.0, f64::min),
                max: corners.iter().cloned().fold(0.0, f64::max),
                area: quadric::length(pl::triangle_normal(p)),
            }
        })
        .collect()
}

fn limit_move(p: [f64; 3], subset: [f64; 3], max_move: f64) -> [f64; 3] {
    let d = quadric::sub(p, subset);
    let l = quadric::length(d);
    if l > max_move { [0, 1, 2].map(|k| subset[k] + d[k] * max_move / l) } else { p }
}

fn tangential_laplacian(ctx: &Context, game: &[[f64; 3]], g: usize) -> [f64; 3] {
    let ring = &ctx.neighbours[g];
    let centroid = ring.iter().fold([0.0; 3], |a, &w| [a[0] + game[w as usize][0], a[1] + game[w as usize][1], a[2] + game[w as usize][2]]);
    let centroid = centroid.map(|v| v / ring.len().max(1) as f64);
    let n = ctx.vertex_normals[g];
    let d = quadric::sub(centroid, game[g]);
    quadric::sub(d, n.map(|c| c * quadric::dot(d, n)))
}

fn hinge(ctx: &Context, game: &[[f64; 3]], t: u32, g: usize) -> Option<[f64; 3]> {
    let tri = ctx.mesh.triangles[t as usize];
    let i = tri.iter().position(|&v| v as usize == g)?;
    let (a, b, c) = (game[g], game[tri[(i + 1) % 3] as usize], game[tri[(i + 2) % 3] as usize]);
    let edge = quadric::sub(c, b);
    let length = quadric::length(edge);
    if length < 1e-12 {
        return None;
    }
    let u = edge.map(|x| x / length);
    let r = ctx.references[t as usize];
    let r_perp = quadric::sub(r, u.map(|x| x * quadric::dot(u, r)));
    let lr = quadric::length(r_perp);
    if lr < 1e-6 {
        return None;
    }
    let r_perp = r_perp.map(|x| x / lr);
    let foot = [0, 1, 2].map(|k| b[k] + u[k] * quadric::dot(u, quadric::sub(a, b)));
    let height = quadric::length(quadric::sub(a, foot));
    let mut v = quadric::cross(r_perp, u);
    let target = |v: [f64; 3]| [0, 1, 2].map(|k| foot[k] + height * v[k]);
    if quadric::dot(pl::triangle_normal([target(v), b, c]), r_perp) < 0.0 {
        v = v.map(|x| -x);
    }
    Some(quadric::sub(target(v), a))
}

fn guard(ctx: &Context, game: &mut [[f64; 3]], cos_turn: f64, rounds: usize) -> usize {
    let star_upright = |game: &[[f64; 3]], g: usize| ctx.vtris[g].iter().all(|&t| ctx.upright(game, t, cos_turn));
    let mut backed = vec![false; game.len()];
    for _ in 0..rounds {
        let mut changed = 0;
        for g in 0..game.len() {
            if ctx.boundary[g] || star_upright(game, g) {
                continue;
            }
            backed[g] = true;
            changed += 1;
            let (from, to) = (ctx.subset[g], game[g]);
            game[g] = from;
            if !star_upright(game, g) {
                continue;
            }
            let (mut lo, mut hi) = (0.0f64, 1.0f64);
            for _ in 0..30 {
                let mid = 0.5 * (lo + hi);
                game[g] = [0, 1, 2].map(|k| from[k] + mid * (to[k] - from[k]));
                if star_upright(game, g) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            game[g] = [0, 1, 2].map(|k| from[k] + lo * (to[k] - from[k]));
        }
        if changed == 0 {
            break;
        }
    }
    backed.iter().filter(|&&b| b).count()
}

struct SettleRule<'a> {
    thresholds: &'a [f64],
    floor: f64,
    max_move: f64,
    rounds: usize,
    hinge: bool,
    origin: &'a [[f64; 3]],
    step_limits: &'a [f64],
}

struct Choice {
    full: bool,
    gain: f64,
    step: f64,
}

impl Choice {
    fn better(&self, other: &Choice) -> bool {
        match (self.full, other.full) {
            (true, false) => true,
            (false, true) => false,
            (true, true) => self.step < other.step,
            (false, false) => self.gain > other.gain + 1e-12,
        }
    }
}

fn within(ctx: &Context, game: &[[f64; 3]], star: &[u32], before: &[StarState], thresholds: &[f64], floor: f64) -> bool {
    let after = star_state(ctx, game, star);
    star.iter().zip(before.iter().zip(&after)).all(|(&s, (b, a))| {
        a.area >= 1e-20
            && a.deviation <= b.deviation.max(thresholds[s as usize]) + 1e-9
            && a.min >= b.min.min(floor) - 1e-9
            && a.max <= b.max.max(170.0) + 1e-9
    })
}

fn settle<E: Fn(&[[f64; 3]], u32) -> f64>(ctx: &Context, game: &mut [[f64; 3]], rule: &SettleRule, excess: E) -> (usize, usize) {
    let mut moves = 0;
    for _ in 0..rule.rounds {
        let mut bad: Vec<(u32, f64)> = (0..ctx.mesh.triangles.len() as u32).map(|t| (t, excess(game, t))).filter(|&(_, e)| e > 0.0).collect();
        bad.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut changed = 0;
        for &(t, _) in &bad {
            if excess(game, t) <= 0.0 {
                continue;
            }
            let start_excess = excess(game, t);
            let mut best: Option<(Choice, usize, [f64; 3])> = None;
            for &g in &ctx.mesh.triangles[t as usize] {
                let g = g as usize;
                if ctx.boundary[g] {
                    continue;
                }
                let x = game[g];
                let star = &ctx.vtris[g];
                let before = star_state(ctx, game, star);
                let mut directions = vec![tangential_laplacian(ctx, game, g), quadric::sub(ctx.subset[g], x)];
                if rule.hinge {
                    directions.extend(hinge(ctx, game, t, g));
                }
                for d in directions {
                    if quadric::length(d) < 1e-12 {
                        continue;
                    }
                    let at = |f: f64| limit_move([0, 1, 2].map(|k| x[k] + f * d[k]), ctx.subset[g], rule.max_move);
                    let allowed = |game: &mut [[f64; 3]], f: f64| {
                        let candidate = at(f);
                        game[g] = candidate;
                        let fine = quadric::length(quadric::sub(candidate, rule.origin[g])) <= rule.step_limits[t as usize]
                            && within(ctx, game, star, &before, rule.thresholds, rule.floor);
                        game[g] = x;
                        fine
                    };
                    let mut reach = 1.0f64;
                    if !allowed(game, 1.0) {
                        let (mut lo, mut hi) = (0.0f64, 1.0f64);
                        for _ in 0..30 {
                            let mid = 0.5 * (lo + hi);
                            if allowed(game, mid) {
                                lo = mid;
                            } else {
                                hi = mid;
                            }
                        }
                        reach = lo;
                    }
                    game[g] = at(reach);
                    let reached = excess(game, t);
                    game[g] = x;
                    let f = if reached <= 0.0 {
                        let (mut lo, mut hi) = (0.0f64, reach);
                        for _ in 0..30 {
                            let mid = 0.5 * (lo + hi);
                            game[g] = at(mid);
                            if excess(game, t) <= 0.0 {
                                hi = mid;
                            } else {
                                lo = mid;
                            }
                        }
                        game[g] = x;
                        hi
                    } else {
                        reach
                    };
                    if !allowed(game, f) {
                        continue;
                    }
                    let candidate = at(f);
                    game[g] = candidate;
                    let left = excess(game, t);
                    game[g] = x;
                    let gain = start_excess - left;
                    if gain <= 1e-6 {
                        continue;
                    }
                    let key = Choice { full: left <= 0.0, gain, step: quadric::length(quadric::sub(candidate, x)) };
                    if best.as_ref().is_none_or(|b| key.better(&b.0)) {
                        best = Some((key, g, candidate));
                    }
                }
            }
            if let Some((_, g, candidate)) = best {
                game[g] = candidate;
                moves += 1;
                changed += 1;
            }
        }
        if changed == 0 {
            break;
        }
    }
    let unresolved = (0..ctx.mesh.triangles.len() as u32).filter(|&t| excess(game, t) > 0.0).count();
    (moves, unresolved)
}

pub(crate) fn neighbours(mesh: &Decimated) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = vec![Vec::new(); mesh.kept.len()];
    for tri in &mesh.triangles {
        for e in 0..3 {
            let (a, b) = (tri[e], tri[(e + 1) % 3]);
            if !out[a as usize].contains(&b) {
                out[a as usize].push(b);
                out[b as usize].push(a);
            }
        }
    }
    out
}

pub fn place(
    positions: &[[f64; 3]],
    dense_triangles: &[[u32; 3]],
    dense_uvs: &[[Uv; 3]],
    mesh: &Decimated,
    settings: &F2Settings,
) -> (Vec<[f64; 3]>, Vec<[Uv; 3]>, F2Stats) {
    let count = mesh.kept.len();
    let triangles = mesh.triangles.len() as u32;
    let normals = pl::angle_weighted_normals(positions, dense_triangles);
    let vertex_normals: Vec<[f64; 3]> = mesh.kept.iter().map(|&d| normals[d as usize]).collect();
    let references = pl::triangle_references(mesh, &vertex_normals);
    let spread: Vec<f64> = mesh
        .triangles
        .iter()
        .zip(&references)
        .map(|(tri, r)| tri.iter().map(|&g| quadric::dot(vertex_normals[g as usize], *r).clamp(-1.0, 1.0).acos().to_degrees()).fold(0.0, f64::max))
        .collect();
    let confident: Vec<bool> = spread.iter().map(|&s| s <= settings.confident_spread_deg).collect();
    let hard = settings.max_turn_deg - settings.turn_margin_deg;
    let thresholds: Vec<f64> = confident.iter().map(|&c| if c { settings.confident_above_deg } else { hard }).collect();
    let step_limits: Vec<f64> = confident.iter().map(|&c| if c { settings.repair_max_step_confident } else { settings.repair_max_step }).collect();
    let vtris = pl::vertex_triangles(mesh, count);
    let rings = pl::ring_triangles(mesh, &vtris);
    let mut ctx = Context {
        mesh,
        boundary: pl::boundary_vertices(mesh),
        vtris,
        neighbours: neighbours(mesh),
        references,
        vertex_normals,
        subset: mesh.kept.iter().map(|&d| positions[d as usize]).collect(),
        subset_cos: Vec::new(),
        confident,
    };
    ctx.subset_cos = (0..triangles).map(|t| ctx.cosine(&ctx.subset, t).unwrap_or(-1.0)).collect();
    let ctx = ctx;
    let cos_turn = settings.max_turn_deg.to_radians().cos();
    let mut stats = F2Stats { placement: PlacementStats { vertices: count, ..Default::default() }, ..Default::default() };
    stats.folded_at_subset = ctx.subset_cos.iter().filter(|&&c| c < cos_turn).count();
    stats.confident_triangles = ctx.confident.iter().filter(|&&c| c).count();

    let (centre, local, sums) = pl::cluster_quadrics(positions, dense_triangles, mesh);
    let max_move = settings.fit.max_move;
    let mut game: Vec<[f64; 3]> = Vec::with_capacity(count);
    for (g, &d) in mesh.kept.iter().enumerate() {
        if ctx.boundary[g] {
            stats.placement.boundary_kept += 1;
            game.push(positions[d as usize]);
            continue;
        }
        let anchor = local[d as usize];
        let q = &sums[g];
        match pl::solve(q, anchor, settings.regular * (q[0] + q[4] + q[7]) / 3.0) {
            Some(x) => {
                if quadric::length(quadric::sub(x, anchor)) > max_move {
                    stats.clamped_initial += 1;
                }
                let x = limit_move(x, anchor, max_move);
                game.push([x[0] + centre[0], x[1] + centre[1], x[2] + centre[2]]);
            }
            None => {
                stats.singular += 1;
                stats.placement.fallbacks += 1;
                game.push(positions[d as usize]);
            }
        }
    }
    pl::fit_rounds(positions, mesh, &mut game, &ctx.subset, &ctx.boundary, &rings, &settings.fit, &mut stats.placement);
    stats.clamped_final = pl::clamp_moves(&mut game, &ctx.subset, max_move);
    stats.placement.backed_off = guard(&ctx, &mut game, cos_turn, 8);
    stats.over_turn_before_repair = (0..triangles).filter(|&t| ctx.deviation(&game, t) > settings.max_turn_deg).count();

    let origin = game.clone();
    let mut rule = SettleRule {
        thresholds: &thresholds,
        floor: settings.fit.min_angle_deg,
        max_move,
        rounds: settings.repair_rounds,
        hinge: true,
        origin: &origin,
        step_limits: &step_limits,
    };
    let (first, _) = settle(&ctx, &mut game, &rule, |at, t| ctx.deviation(at, t) - hard);
    let (second, unresolved) = settle(&ctx, &mut game, &rule, |at, t| ctx.deviation(at, t) - thresholds[t as usize]);
    stats.orientation_moves = first + second;
    stats.unresolved_orientation = unresolved;
    if settings.fit.repair_below_deg > 0.0 {
        let below = settings.fit.repair_below_deg + settings.angle_margin_deg;
        rule.hinge = false;
        let (moves, unresolved) =
            settle(&ctx, &mut game, &rule, |at, t| below - pl::min_angle(mesh.triangles[t as usize].map(|i| at[i as usize])));
        stats.angle_moves = moves;
        stats.unresolved_angle = unresolved;
    }
    let mut left: Vec<(u32, f64)> = (0..triangles).map(|t| (t, ctx.deviation(&game, t))).filter(|&(_, d)| d > settings.max_turn_deg).collect();
    left.sort_by(|a, b| b.1.total_cmp(&a.1));
    stats.over_turn_left = left;
    pl::finish_moves(&mut stats.placement, &game, &ctx.subset);

    let surface = Surface::new(positions, dense_triangles, dense_uvs, mesh, settings.uv_ring);
    let (derived, uv_stats) = surface.derive_uvs(mesh, &game);
    stats.uv = uv_stats;
    let (uvs, flips) = uv_chart::guard_flips(mesh, &derived);
    stats.uv_flip_guard = flips;
    (game, uvs, stats)
}
