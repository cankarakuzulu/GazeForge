use std::collections::HashSet;

use serde::Serialize;

use crate::decimate::Decimated;
use crate::dense::Uv;
use crate::placement::closest;

pub type Hit = (u32, [f64; 3], f64);

pub struct Surface<'a> {
    positions: &'a [[f64; 3]],
    triangles: &'a [[u32; 3]],
    uvs: &'a [[Uv; 3]],
    owner: &'a [u32],
    pub charts: usize,
    across: Vec<[u32; 3]>,
    seeds: Vec<Vec<(u32, u8)>>,
    neighbours: Vec<Vec<u32>>,
    ring: usize,
}

#[derive(Default, Serialize)]
pub struct UvStats {
    pub charts: usize,
    pub corners: usize,
    pub lookups: usize,
    pub seed_misses: usize,
    pub gap_median_mm: f64,
    pub gap_p99_mm: f64,
    pub gap_max_mm: f64,
    pub gap_over_005mm: usize,
    pub uv_change_median: f64,
    pub uv_change_p95: f64,
    pub uv_change_max: f64,
}

fn uv_close(a: Uv, b: Uv) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

fn find(parent: &mut [u32], mut x: u32) -> u32 {
    while parent[x as usize] != x {
        parent[x as usize] = parent[parent[x as usize] as usize];
        x = parent[x as usize];
    }
    x
}

fn continuity(triangles: &[[u32; 3]], uvs: &[[Uv; 3]]) -> (Vec<[u32; 3]>, usize) {
    let mut edges: Vec<(u64, u32, u8)> = Vec::with_capacity(triangles.len() * 3);
    for (t, tri) in triangles.iter().enumerate() {
        for e in 0..3u8 {
            let (a, b) = (tri[e as usize], tri[(e as usize + 1) % 3]);
            edges.push((((a.min(b) as u64) << 32) | a.max(b) as u64, t as u32, e));
        }
    }
    edges.sort_unstable();
    let mut parent: Vec<u32> = (0..triangles.len() as u32).collect();
    let mut across = vec![[u32::MAX; 3]; triangles.len()];
    let uv_at = |t: u32, v: u32| {
        let tri = triangles[t as usize];
        uvs[t as usize][tri.iter().position(|&x| x == v).unwrap()]
    };
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j].0 == edges[i].0 {
            j += 1;
        }
        let (a, b) = ((edges[i].0 >> 32) as u32, (edges[i].0 & 0xffff_ffff) as u32);
        if j - i == 2 {
            let ((s, es), (t, et)) = ((edges[i].1, edges[i].2), (edges[i + 1].1, edges[i + 1].2));
            if uv_close(uv_at(s, a), uv_at(t, a)) && uv_close(uv_at(s, b), uv_at(t, b)) {
                across[s as usize][es as usize] = t;
                across[t as usize][et as usize] = s;
                let (rs, rt) = (find(&mut parent, s), find(&mut parent, t));
                if rs != rt {
                    parent[rs.max(rt) as usize] = rs.min(rt);
                }
            }
        }
        i = j;
    }
    let roots = (0..triangles.len() as u32).filter(|&t| find(&mut parent, t) == t).count();
    (across, roots)
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * q) as usize]
}

impl<'a> Surface<'a> {
    pub fn new(positions: &'a [[f64; 3]], triangles: &'a [[u32; 3]], uvs: &'a [[Uv; 3]], mesh: &'a Decimated, ring: usize) -> Surface<'a> {
        let (across, charts) = continuity(triangles, uvs);
        let games = mesh.kept.len();
        let mut game_of = vec![u32::MAX; positions.len()];
        for (g, &d) in mesh.kept.iter().enumerate() {
            game_of[d as usize] = g as u32;
        }
        let mut seeds: Vec<Vec<(u32, u8)>> = vec![Vec::new(); games];
        for (t, tri) in triangles.iter().enumerate() {
            for (c, &v) in tri.iter().enumerate() {
                let g = game_of[v as usize];
                if g != u32::MAX {
                    seeds[g as usize].push((t as u32, c as u8));
                }
            }
        }
        Surface { positions, triangles, uvs, owner: &mesh.owner, charts, across, seeds, neighbours: crate::placement_f2::neighbours(mesh), ring }
    }

    fn region(&self, g: u32) -> Vec<u32> {
        let mut set = vec![g];
        let mut frontier = 0;
        for _ in 0..self.ring {
            let end = set.len();
            for k in frontier..end {
                for &w in &self.neighbours[set[k] as usize] {
                    if !set.contains(&w) {
                        set.push(w);
                    }
                }
            }
            frontier = end;
        }
        set.sort_unstable();
        set
    }

    fn distance(&self, t: u32, p: [f64; 3]) -> ([f64; 3], f64) {
        let tri = self.triangles[t as usize].map(|i| self.positions[i as usize]);
        closest(p, tri[0], tri[1], tri[2])
    }

    pub fn closest_on_patch(&self, g: u32, p: [f64; 3], uv: Uv) -> Option<Hit> {
        let region = self.region(g);
        let inside = |t: u32| self.triangles[t as usize].iter().any(|&v| region.binary_search(&self.owner[v as usize]).is_ok());
        let mut seen: HashSet<u32> = HashSet::new();
        let mut queue: Vec<u32> = self.seeds[g as usize]
            .iter()
            .filter(|&&(t, c)| uv_close(self.uvs[t as usize][c as usize], uv))
            .map(|&(t, _)| t)
            .collect();
        seen.extend(queue.iter().copied());
        let mut best: Option<Hit> = None;
        while let Some(t) = queue.pop() {
            let (w3, d2) = self.distance(t, p);
            if best.is_none_or(|b| d2 < b.2) {
                best = Some((t, w3, d2));
            }
            for &s in &self.across[t as usize] {
                if s != u32::MAX && inside(s) && seen.insert(s) {
                    queue.push(s);
                }
            }
        }
        best
    }

    fn uv(&self, hit: Hit) -> Uv {
        let t = self.uvs[hit.0 as usize];
        [0, 1].map(|k| (hit.1[0] * t[0][k] as f64 + hit.1[1] * t[1][k] as f64 + hit.1[2] * t[2][k] as f64) as f32)
    }

    pub fn derive_uvs(&self, mesh: &Decimated, game: &[[f64; 3]]) -> (Vec<[Uv; 3]>, UvStats) {
        let mut wanted: Vec<Vec<Uv>> = vec![Vec::new(); game.len()];
        for (tri, uvs) in mesh.triangles.iter().zip(&mesh.triangle_uvs) {
            for i in 0..3 {
                let list = &mut wanted[tri[i] as usize];
                if !list.contains(&uvs[i]) {
                    list.push(uvs[i]);
                }
            }
        }
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
        let chunk = game.len().div_ceil(threads).max(1);
        let all: Vec<usize> = (0..game.len()).collect();
        let found: Vec<Vec<(Uv, Option<Hit>)>> = std::thread::scope(|scope| {
            let handles: Vec<_> = all
                .chunks(chunk)
                .map(|part| {
                    let wanted = &wanted;
                    scope.spawn(move || {
                        part.iter()
                            .map(|&g| wanted[g].iter().map(|&uv| (uv, self.closest_on_patch(g as u32, game[g], uv))).collect::<Vec<_>>())
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let mut stats = UvStats { charts: self.charts, ..Default::default() };
        let mut gaps: Vec<f64> = Vec::new();
        for list in &found {
            for (_, hit) in list {
                stats.lookups += 1;
                match hit {
                    Some(h) => gaps.push(h.2.sqrt() * 1000.0),
                    None => stats.seed_misses += 1,
                }
            }
        }
        gaps.sort_by(f64::total_cmp);
        stats.gap_median_mm = percentile(&gaps, 0.5);
        stats.gap_p99_mm = percentile(&gaps, 0.99);
        stats.gap_max_mm = gaps.last().copied().unwrap_or(0.0);
        stats.gap_over_005mm = gaps.iter().filter(|&&g| g > 0.05).count();
        let mut changes: Vec<f64> = Vec::new();
        let uvs: Vec<[Uv; 3]> = mesh
            .triangles
            .iter()
            .zip(&mesh.triangle_uvs)
            .map(|(tri, old)| {
                [0, 1, 2].map(|i| {
                    stats.corners += 1;
                    let hit = found[tri[i] as usize].iter().find(|(uv, _)| *uv == old[i]).and_then(|(_, h)| *h);
                    match hit {
                        Some(h) => {
                            let uv = self.uv(h);
                            changes.push(((uv[0] - old[i][0]) as f64).hypot((uv[1] - old[i][1]) as f64));
                            uv
                        }
                        None => old[i],
                    }
                })
            })
            .collect();
        changes.sort_by(f64::total_cmp);
        stats.uv_change_median = percentile(&changes, 0.5);
        stats.uv_change_p95 = percentile(&changes, 0.95);
        stats.uv_change_max = changes.last().copied().unwrap_or(0.0);
        (uvs, stats)
    }
}

fn uv_sign(t: [Uv; 3]) -> i8 {
    let area = (t[1][0] as f64 - t[0][0] as f64) * (t[2][1] as f64 - t[0][1] as f64) - (t[2][0] as f64 - t[0][0] as f64) * (t[1][1] as f64 - t[0][1] as f64);
    if area > 0.0 {
        1
    } else if area < 0.0 {
        -1
    } else {
        0
    }
}

#[derive(Default, Serialize)]
pub struct FlipGuard {
    pub flipped_before: usize,
    pub reverted_keys: usize,
    pub flipped_after: usize,
    pub rounds: usize,
}

pub fn guard_flips(mesh: &Decimated, derived: &[[Uv; 3]]) -> (Vec<[Uv; 3]>, FlipGuard) {
    let base = &mesh.triangle_uvs;
    let major = {
        let positive = base.iter().filter(|t| uv_sign(**t) > 0).count();
        if positive * 2 >= base.len() { 1 } else { -1 }
    };
    let key = |g: u32, uv: Uv| (g, uv[0].to_bits(), uv[1].to_bits());
    let mut reverted: HashSet<(u32, u32, u32)> = HashSet::new();
    let mut out = derived.to_vec();
    let mut report = FlipGuard::default();
    for round in 0..64 {
        let flipped: Vec<usize> = (0..out.len()).filter(|&t| uv_sign(out[t]) != major).collect();
        if round == 0 {
            report.flipped_before = flipped.len();
        }
        report.flipped_after = flipped.len();
        if flipped.is_empty() {
            break;
        }
        report.rounds = round + 1;
        for &t in &flipped {
            for i in 0..3 {
                reverted.insert(key(mesh.triangles[t][i], base[t][i]));
            }
        }
        for t in 0..out.len() {
            for i in 0..3 {
                if reverted.contains(&key(mesh.triangles[t][i], base[t][i])) {
                    out[t][i] = base[t][i];
                }
            }
        }
    }
    report.reverted_keys = reverted.len();
    (out, report)
}
