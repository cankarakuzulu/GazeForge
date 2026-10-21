use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use gf_geom::Uv;
use serde::Serialize;
use serde_json::{Value, json};

pub type Point = [f64; 3];

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn mul(a: Point, s: f64) -> Point {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: Point, b: Point) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Point, b: Point) -> Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn norm(a: Point) -> f64 {
    dot(a, a).sqrt()
}

fn unit(a: Point) -> Point {
    let n = norm(a);
    if n > 0.0 { mul(a, 1.0 / n) } else { [0.0; 3] }
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    add(a, mul(sub(b, a), t))
}

fn angle_deg(a: Point, b: Point) -> f64 {
    let (a, b) = (unit(a), unit(b));
    if norm(a) == 0.0 || norm(b) == 0.0 {
        return 180.0;
    }
    dot(a, b).clamp(-1.0, 1.0).acos().to_degrees()
}

fn face_normal(p: &[Point], t: [u32; 3]) -> Point {
    cross(sub(p[t[1] as usize], p[t[0] as usize]), sub(p[t[2] as usize], p[t[0] as usize]))
}

pub fn centroid(points: &[Point]) -> Point {
    let mut c = [0.0; 3];
    for p in points {
        c = add(c, *p);
    }
    mul(c, 1.0 / points.len().max(1) as f64)
}

#[derive(Clone)]
pub struct Mesh {
    pub positions: Vec<Point>,
    pub triangles: Vec<[u32; 3]>,
    pub uvs: Vec<[Uv; 3]>,
}

pub fn patch_corners(quads: &[[u32; 4]], per_face: usize) -> Result<Vec<[u32; 4]>, String> {
    let side = (per_face as f64).sqrt().round() as usize;
    if side * side != per_face || per_face == 0 || !quads.len().is_multiple_of(per_face) {
        return Err(format!("{} quads do not split into patches of {per_face}", quads.len()));
    }
    let mut out = Vec::with_capacity(quads.len() / per_face);
    for (f, block) in quads.chunks(per_face).enumerate() {
        let mut halves: HashSet<(u32, u32)> = HashSet::with_capacity(per_face * 4);
        let mut incidence: HashMap<u32, u32> = HashMap::new();
        for q in block {
            for k in 0..4 {
                halves.insert((q[k], q[(k + 1) % 4]));
                *incidence.entry(q[k]).or_default() += 1;
            }
        }
        let mut next: HashMap<u32, u32> = HashMap::new();
        for &(a, b) in &halves {
            if !halves.contains(&(b, a)) && next.insert(a, b).is_some() {
                return Err(format!("coarse face {f}: its patch boundary is not a simple loop"));
            }
        }
        let start = *next.keys().min().ok_or_else(|| format!("coarse face {f}: no patch boundary"))?;
        let mut walk = vec![start];
        loop {
            let n = next[walk.last().unwrap()];
            if n == start {
                break;
            }
            walk.push(n);
            if walk.len() > next.len() {
                return Err(format!("coarse face {f}: its patch boundary does not close"));
            }
        }
        if walk.len() != 4 * side || walk.len() != next.len() {
            return Err(format!("coarse face {f}: patch boundary of {} edges, expected {}", walk.len(), 4 * side));
        }
        let corners: Vec<u32> = walk.iter().copied().filter(|v| incidence[v] == 1).collect();
        if corners.len() != 4 {
            return Err(format!("coarse face {f}: {} patch corners, expected 4", corners.len()));
        }
        out.push([corners[0], corners[1], corners[2], corners[3]]);
    }
    Ok(out)
}

struct Embedder<'a> {
    cage: &'a [[usize; 4]],
    body: &'a [[u32; 4]],
    cage_half: HashMap<(usize, usize), (usize, usize)>,
    body_half: HashMap<(u32, u32), (usize, usize)>,
    face_stamp: Vec<u32>,
    face_map: Vec<(usize, usize)>,
    vertex_stamp: Vec<u32>,
    vertex_map: Vec<u32>,
    stamp: u32,
}

impl Embedder<'_> {
    fn run(&mut self, seed: usize, rot: usize) -> bool {
        self.stamp += 1;
        let s = self.stamp;
        let mut queue = VecDeque::new();
        self.face_stamp[0] = s;
        self.face_map[0] = (seed, rot);
        queue.push_back(0usize);
        let mut visited = 1usize;
        while let Some(cf) = queue.pop_front() {
            let (bf, rot) = self.face_map[cf];
            let c = self.cage[cf];
            let b = self.body[bf];
            for k in 0..4 {
                let u = c[k];
                let target = b[(k + rot) % 4];
                if self.vertex_stamp[u] == s {
                    if self.vertex_map[u] != target {
                        return false;
                    }
                } else {
                    self.vertex_stamp[u] = s;
                    self.vertex_map[u] = target;
                }
            }
            for k in 0..4 {
                let (u, v) = (c[k], c[(k + 1) % 4]);
                let Some(&(g, j)) = self.cage_half.get(&(v, u)) else { continue };
                let (bu, bv) = (b[(k + rot) % 4], b[(k + 1 + rot) % 4]);
                let Some(&(gb, jb)) = self.body_half.get(&(bv, bu)) else { return false };
                let r = (jb + 4 - j) % 4;
                if self.face_stamp[g] == s {
                    if self.face_map[g] != (gb, r) {
                        return false;
                    }
                } else {
                    self.face_stamp[g] = s;
                    self.face_map[g] = (gb, r);
                    visited += 1;
                    queue.push_back(g);
                }
            }
        }
        if visited != self.cage.len() {
            return false;
        }
        let mut images: Vec<u32> = self.vertex_map.clone();
        images.sort_unstable();
        images.dedup();
        images.len() == self.vertex_map.len()
    }
}

pub struct Embedding {
    pub vertices: HashMap<u32, u32>,
    pub faces: Vec<usize>,
    pub seeds_tried: usize,
}

pub fn embed_quads(cage: &[[u32; 4]], body: &[[u32; 4]]) -> Result<Embedding, String> {
    if cage.is_empty() {
        return Err("the cage has no faces".into());
    }
    let mut compact: HashMap<u32, usize> = HashMap::new();
    let mut ids: Vec<u32> = Vec::new();
    let cage_c: Vec<[usize; 4]> = cage
        .iter()
        .map(|q| {
            q.map(|v| {
                *compact.entry(v).or_insert_with(|| {
                    ids.push(v);
                    ids.len() - 1
                })
            })
        })
        .collect();
    let mut cage_half = HashMap::new();
    for (f, q) in cage_c.iter().enumerate() {
        for k in 0..4 {
            if cage_half.insert((q[k], q[(k + 1) % 4]), (f, k)).is_some() {
                return Err("the cage repeats a directed edge (inconsistent orientation)".into());
            }
        }
    }
    let mut body_half = HashMap::new();
    for (f, q) in body.iter().enumerate() {
        for k in 0..4 {
            if body_half.insert((q[k], q[(k + 1) % 4]), (f, k)).is_some() {
                return Err("the body repeats a directed edge (inconsistent orientation)".into());
            }
        }
    }
    let mut e = Embedder {
        cage: &cage_c,
        body,
        cage_half,
        body_half,
        face_stamp: vec![0; cage_c.len()],
        face_map: vec![(0, 0); cage_c.len()],
        vertex_stamp: vec![0; ids.len()],
        vertex_map: vec![0; ids.len()],
        stamp: 0,
    };
    let mut found: Vec<(Vec<u32>, Vec<usize>)> = Vec::new();
    for seed in 0..body.len() {
        for rot in 0..4 {
            if e.run(seed, rot) {
                found.push((e.vertex_map.clone(), e.face_map.iter().map(|f| f.0).collect()));
            }
        }
    }
    let seeds_tried = body.len() * 4;
    match found.len() {
        0 => Err("the cage does not embed in the body mesh by topology".into()),
        1 => {
            let (vm, faces) = found.remove(0);
            Ok(Embedding { vertices: ids.iter().zip(vm).map(|(&c, b)| (c, b)).collect(), faces, seeds_tried })
        }
        n => Err(format!("the cage embeds in the body mesh {n} ways; the correspondence is ambiguous")),
    }
}

pub fn oriented_loops<F: AsRef<[u32]>>(faces: &[F]) -> Result<Vec<Vec<u32>>, String> {
    let mut halves: HashSet<(u32, u32)> = HashSet::new();
    for f in faces {
        let f = f.as_ref();
        for k in 0..f.len() {
            halves.insert((f[k], f[(k + 1) % f.len()]));
        }
    }
    let mut next: BTreeMap<u32, u32> = BTreeMap::new();
    for &(a, b) in &halves {
        if !halves.contains(&(b, a)) && next.insert(a, b).is_some() {
            return Err(format!("vertex {a} has two outgoing boundary edges (a pinched boundary)"));
        }
    }
    let mut seen: HashSet<u32> = HashSet::new();
    let mut loops = Vec::new();
    for &start in next.keys() {
        if seen.contains(&start) {
            continue;
        }
        let mut walk = vec![start];
        seen.insert(start);
        let mut v = start;
        loop {
            let n = *next.get(&v).ok_or_else(|| format!("boundary at vertex {v} does not close"))?;
            if n == start {
                break;
            }
            if !seen.insert(n) {
                return Err(format!("boundary through vertex {n} does not close into a simple loop"));
            }
            walk.push(n);
            v = n;
        }
        loops.push(walk);
    }
    Ok(loops)
}

fn basis(t: f64) -> [f64; 4] {
    [
        (1.0 - t).powi(3) / 6.0,
        (3.0 * t.powi(3) - 6.0 * t * t + 4.0) / 6.0,
        (-3.0 * t.powi(3) + 3.0 * t * t + 3.0 * t + 1.0) / 6.0,
        t.powi(3) / 6.0,
    ]
}

pub fn bspline_controls(samples: &[Point], per_span: usize) -> Result<(Vec<Point>, f64), String> {
    let n = samples.len() / per_span.max(1);
    if per_span == 0 || n * per_span != samples.len() || n < 4 {
        return Err(format!("{} samples do not make spans of {per_span}", samples.len()));
    }
    let mut a = vec![vec![0.0f64; n]; n];
    let mut rhs = vec![[0.0f64; 3]; n];
    for i in 0..n {
        for s in 0..per_span {
            let b = basis(s as f64 / per_span as f64);
            let cols = [(i + n - 1) % n, i, (i + 1) % n, (i + 2) % n];
            let y = samples[i * per_span + s];
            for p in 0..4 {
                for q in 0..4 {
                    a[cols[p]][cols[q]] += b[p] * b[q];
                }
                for k in 0..3 {
                    rhs[cols[p]][k] += b[p] * y[k];
                }
            }
        }
    }
    for col in 0..n {
        let pivot = (col..n).max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs())).unwrap();
        if a[pivot][col].abs() < 1e-14 {
            return Err("the spline system is singular".into());
        }
        a.swap(col, pivot);
        rhs.swap(col, pivot);
        let pivot_row = a[col].clone();
        let pivot_rhs = rhs[col];
        for row in 0..n {
            if row != col {
                let f = a[row][col] / pivot_row[col];
                if f != 0.0 {
                    for (x, y) in a[row][col..].iter_mut().zip(&pivot_row[col..]) {
                        *x -= f * y;
                    }
                    for (x, y) in rhs[row].iter_mut().zip(pivot_rhs) {
                        *x -= f * y;
                    }
                }
            }
        }
    }
    let controls: Vec<Point> = (0..n).map(|i| mul(rhs[i], 1.0 / a[i][i])).collect();
    let mut worst = 0.0f64;
    for i in 0..n {
        for s in 0..per_span {
            let b = basis(s as f64 / per_span as f64);
            let mut p = [0.0; 3];
            for (w, j) in b.iter().zip([(i + n - 1) % n, i, (i + 1) % n, (i + 2) % n]) {
                p = add(p, mul(controls[j], *w));
            }
            worst = worst.max(norm(sub(p, samples[i * per_span + s])));
        }
    }
    Ok((controls, worst))
}

pub fn cyclic_delta(from: f64, to: f64, period: f64) -> f64 {
    let d = (to - from).rem_euclid(period);
    if d > 0.5 * period { d - period } else { d }
}

pub fn loop_point(points: &[Point], param: f64, per_span: f64) -> Point {
    let n = points.len();
    let x = (param * per_span).rem_euclid(n as f64);
    let i = x.floor() as usize % n;
    lerp(points[i], points[(i + 1) % n], x - x.floor())
}

pub fn arc_params(ring: &[Point], seam: &[Point], per_span: f64) -> Vec<f64> {
    let cumulative = |pts: &[Point]| -> Vec<f64> {
        let mut c = vec![0.0];
        for i in 0..pts.len() {
            let d = norm(sub(pts[(i + 1) % pts.len()], pts[i]));
            c.push(c[i] + d);
        }
        c
    };
    let rc = cumulative(ring);
    let sc = cumulative(seam);
    let (rl, sl) = (rc[ring.len()], sc[seam.len()]);
    rc[..ring.len()]
        .iter()
        .map(|&a| {
            let target = a / rl * sl;
            let k = sc.partition_point(|&x| x <= target).clamp(1, seam.len()) - 1;
            let span = (sc[k + 1] - sc[k]).max(1e-15);
            (k as f64 + (target - sc[k]) / span) / per_span
        })
        .collect()
}

pub fn fit_params(ring: &[Point], seam: &[Point], per_span: f64) -> (Vec<f64>, f64, f64) {
    let n = ring.len();
    let mut ra = vec![0.0];
    for i in 0..n {
        ra.push(ra[i] + norm(sub(ring[(i + 1) % n], ring[i])));
    }
    let total = ra[n].max(1e-15);
    let fractions: Vec<f64> = ra[..n].iter().map(|a| a / total).collect();
    let mut sa = vec![0.0];
    for i in 0..seam.len() {
        sa.push(sa[i] + norm(sub(seam[(i + 1) % seam.len()], seam[i])));
    }
    let stotal = sa[seam.len()].max(1e-15);
    let param_of = |f: f64| -> f64 {
        let target = f.rem_euclid(1.0) * stotal;
        let k = sa.partition_point(|&x| x <= target).clamp(1, seam.len()) - 1;
        (k as f64 + (target - sa[k]) / (sa[k + 1] - sa[k]).max(1e-15)) / per_span
    };
    let cost = |delta: f64| -> f64 { fractions.iter().zip(ring).map(|(f, p)| dot(sub(*p, loop_point(seam, param_of(f + delta), per_span)), sub(*p, loop_point(seam, param_of(f + delta), per_span)))).sum() };
    let steps = 400;
    let mut best = (f64::INFINITY, 0.0);
    for s in 0..steps {
        let delta = s as f64 / steps as f64 - 0.5;
        let c = cost(delta);
        if c < best.0 {
            best = (c, delta);
        }
    }
    let mut step = 1.0 / steps as f64;
    while step > 1e-6 {
        for d in [best.1 - step, best.1 + step] {
            let c = cost(d);
            if c < best.0 {
                best = (c, d);
            }
        }
        step *= 0.5;
    }
    let period = seam.len() as f64 / per_span;
    let coarse: Vec<f64> = fractions.iter().map(|f| param_of(f + best.1)).collect();
    let samples = seam.len();
    let mut refined: Vec<f64> = coarse
        .iter()
        .zip(ring)
        .map(|(&p, &q)| {
            let centre = p * per_span;
            let window = (0.75 * per_span).ceil() as i64;
            let mut found = (f64::INFINITY, p);
            for k in -window..=window {
                let i = (centre.floor() as i64 + k).rem_euclid(samples as i64) as usize;
                let (d, t) = closest_on_segment(q, seam[i], seam[(i + 1) % samples]);
                if d < found.0 {
                    found = (d, ((i as f64 + t) / per_span).rem_euclid(period));
                }
            }
            found.1
        })
        .collect();
    for j in 1..n {
        if cyclic_delta(refined[j - 1], refined[j], period) <= 1e-6 {
            refined[j] = (refined[j - 1] + 1e-3).rem_euclid(period);
        }
    }
    (refined, best.1, (best.0 / n as f64).sqrt())
}

pub fn loop_centre_radius(points: &[Point]) -> (Point, f64) {
    let n = points.len();
    let lengths: Vec<f64> = (0..n).map(|i| 0.5 * (norm(sub(points[(i + 1) % n], points[i])) + norm(sub(points[i], points[(i + n - 1) % n])))).collect();
    let total: f64 = lengths.iter().sum::<f64>().max(1e-15);
    let mut c = [0.0; 3];
    for (p, l) in points.iter().zip(&lengths) {
        c = add(c, mul(*p, *l));
    }
    let c = mul(c, 1.0 / total);
    let r2: f64 = points.iter().zip(&lengths).map(|(p, l)| l * ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2))).sum::<f64>() / total;
    (c, r2.sqrt())
}

pub fn signed_area_xy(points: &[Point]) -> f64 {
    let mut a = 0.0;
    for i in 0..points.len() {
        let (p, q) = (points[i], points[(i + 1) % points.len()]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    0.5 * a
}

pub fn closest_on_segment(p: Point, a: Point, b: Point) -> (f64, f64) {
    let ab = sub(b, a);
    let l = dot(ab, ab);
    let t = if l > 0.0 { (dot(sub(p, a), ab) / l).clamp(0.0, 1.0) } else { 0.0 };
    (norm(sub(p, lerp(a, b, t))), t)
}

pub struct Field {
    pub ring: Vec<Point>,
    pub displacement: Vec<Point>,
    pub width: f64,
}

impl Field {
    pub fn closest(&self, p: Point) -> (f64, usize, f64) {
        let n = self.ring.len();
        let mut best = (f64::INFINITY, 0, 0.0);
        for i in 0..n {
            let (d, t) = closest_on_segment(p, self.ring[i], self.ring[(i + 1) % n]);
            if d < best.0 {
                best = (d, i, t);
            }
        }
        best
    }

    pub fn weight(&self, distance: f64) -> f64 {
        if self.width <= 0.0 {
            return 0.0;
        }
        let t = (distance / self.width).min(1.0);
        (1.0 - t) * (1.0 - t) * (1.0 + 2.0 * t)
    }

    pub fn eval(&self, p: Point) -> (Point, f64) {
        let (d, i, t) = self.closest(p);
        let w = self.weight(d);
        if w == 0.0 {
            return ([0.0; 3], 0.0);
        }
        let n = self.ring.len();
        (mul(lerp(self.displacement[i], self.displacement[(i + 1) % n], t), w), w)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RingVertex {
    pub vertex: u32,
    pub param: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct SeamPoint {
    pub vertex: u32,
    pub param: f64,
    pub position: Point,
    pub base: Point,
    pub down: Point,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Node {
    Body(u32),
    Seam(usize),
}

pub struct Stitched {
    pub positions: Vec<Point>,
    pub triangles: Vec<[u32; 3]>,
    pub uvs: Vec<[Uv; 3]>,
    pub seam_copies: Vec<(u32, u32)>,
    pub source: Vec<Option<u32>>,
    pub before: Vec<Option<Point>>,
    pub band: Vec<f64>,
    pub fan: Vec<bool>,
}

fn snap(seam: &[SeamPoint], param: f64, period: f64) -> usize {
    let n = seam.len();
    let i = seam.partition_point(|s| s.param < param);
    let candidates = [(i + n - 1) % n, i % n];
    *candidates
        .iter()
        .min_by(|&&a, &&b| cyclic_delta(param, seam[a].param, period).abs().total_cmp(&cyclic_delta(param, seam[b].param, period).abs()))
        .unwrap()
}

fn chain(from: usize, to: usize, n: usize) -> Result<Vec<usize>, String> {
    let forward = (to + n - from) % n;
    let backward = (n - forward) % n;
    let (step, count) = if forward <= backward { (1, forward) } else { (n - 1, backward) };
    if count * 4 > n {
        return Err(format!("a body ring edge spans {count} of {n} seam points; the ring and the seam do not correspond"));
    }
    Ok((1..count).map(|k| (from + k * step) % n).collect())
}

const TANGENT_SPREAD: usize = 30;

pub fn stitch(body: &Mesh, ring: &[RingVertex], period: f64, seam: &[SeamPoint], field: &Field, smooth: usize, tangent: f64) -> Result<Stitched, String> {
    if seam.len() < 3 || ring.len() < 3 {
        return Err("the seam and the ring need at least three points".into());
    }
    if seam.windows(2).any(|w| w[1].param <= w[0].param) {
        return Err("seam points must be sorted by strictly increasing parameter".into());
    }
    let in_ring: HashMap<u32, usize> = ring.iter().enumerate().map(|(i, r)| (r.vertex, i)).collect();
    let snaps: Vec<usize> = ring.iter().map(|r| snap(seam, r.param, period)).collect();
    let halves: HashSet<(u32, u32)> = body.triangles.iter().flat_map(|t| (0..3).map(move |k| (t[k], t[(k + 1) % 3]))).collect();
    let mut polygons: Vec<(Vec<(Node, Uv)>, bool)> = Vec::with_capacity(body.triangles.len());
    for (t, tuv) in body.triangles.iter().zip(&body.uvs) {
        let mut poly: Vec<(Node, Uv)> = Vec::with_capacity(4);
        let mut chained = false;
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let node = match in_ring.get(&a) {
                Some(&r) => Node::Seam(snaps[r]),
                None => Node::Body(a),
            };
            poly.push((node, tuv[k]));
            if let (Some(&ra), Some(&rb)) = (in_ring.get(&a), in_ring.get(&b))
                && !halves.contains(&(b, a))
            {
                let between = chain(snaps[ra], snaps[rb], seam.len())?;
                let span = cyclic_delta(ring[ra].param, ring[rb].param, period);
                for m in between {
                    let f = if span.abs() > 1e-12 { (cyclic_delta(ring[ra].param, seam[m].param, period) / span).clamp(0.0, 1.0) } else { 0.0 };
                    let (ua, ub) = (tuv[k], tuv[(k + 1) % 3]);
                    let uv = [ua[0] + (ub[0] - ua[0]) * f as f32, ua[1] + (ub[1] - ua[1]) * f as f32];
                    poly.push((Node::Seam(m), uv));
                }
                chained = true;
            }
        }
        let mut cleaned: Vec<(Node, Uv)> = Vec::with_capacity(poly.len());
        for (i, entry) in poly.iter().enumerate() {
            if entry.0 != poly[(i + 1) % poly.len()].0 {
                cleaned.push(*entry);
            }
        }
        if cleaned.len() >= 3 {
            polygons.push((cleaned, chained));
        }
    }
    let mut index: HashMap<Node, u32> = HashMap::new();
    let mut nodes: Vec<Node> = Vec::new();
    let mut used_body: Vec<u32> = polygons.iter().flat_map(|(p, _)| p.iter().filter_map(|(n, _)| if let Node::Body(b) = n { Some(*b) } else { None })).collect();
    used_body.sort_unstable();
    used_body.dedup();
    for b in used_body {
        index.insert(Node::Body(b), nodes.len() as u32);
        nodes.push(Node::Body(b));
    }
    let mut used_seam: Vec<usize> = polygons.iter().flat_map(|(p, _)| p.iter().filter_map(|(n, _)| if let Node::Seam(m) = n { Some(*m) } else { None })).collect();
    used_seam.sort_unstable();
    used_seam.dedup();
    for m in used_seam {
        index.insert(Node::Seam(m), nodes.len() as u32);
        nodes.push(Node::Seam(m));
    }
    let mut triangles = Vec::new();
    let mut uvs = Vec::new();
    let mut fan = Vec::new();
    for (poly, chained) in &polygons {
        let n = poly.len();
        let pivot = poly.iter().position(|(node, _)| matches!(node, Node::Body(_))).unwrap_or(0);
        for i in 1..n - 1 {
            let c = [pivot, (pivot + i) % n, (pivot + i + 1) % n];
            let tri = c.map(|j| index[&poly[j].0]);
            if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
                continue;
            }
            triangles.push(tri);
            uvs.push(c.map(|j| poly[j].1));
            fan.push(*chained || c.iter().any(|&j| matches!(poly[j].0, Node::Seam(_))));
        }
    }
    let mut positions = Vec::with_capacity(nodes.len());
    let mut before = Vec::with_capacity(nodes.len());
    let mut band = Vec::with_capacity(nodes.len());
    let mut source = Vec::with_capacity(nodes.len());
    let mut seam_copies = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        match node {
            Node::Body(b) => {
                let p = body.positions[*b as usize];
                let (d, w) = field.eval(p);
                positions.push(add(p, d));
                before.push(Some(p));
                band.push(w);
                source.push(Some(*b));
            }
            Node::Seam(m) => {
                positions.push(seam[*m].position);
                before.push(None);
                band.push(1.0);
                source.push(None);
                seam_copies.push((i as u32, seam[*m].vertex));
            }
        }
    }
    let mut st = Stitched { positions, triangles, uvs, seam_copies, source, before, band, fan };
    if tangent > 0.0 {
        let copy_of: HashMap<u32, usize> = nodes.iter().enumerate().filter_map(|(i, n)| if let Node::Seam(m) = n { Some((i as u32, *m)) } else { None }).collect();
        let mut touching: HashMap<u32, Vec<usize>> = HashMap::new();
        for t in &st.triangles {
            for k in 0..3 {
                if let Some(&m) = copy_of.get(&t[k]) {
                    for j in 1..3 {
                        let v = t[(k + j) % 3];
                        if !copy_of.contains_key(&v) {
                            touching.entry(v).or_default().push(m);
                        }
                    }
                }
            }
        }
        let mut correction: Vec<Option<Point>> = vec![None; st.positions.len()];
        for (v, list) in &touching {
            let anchor = centroid(&list.iter().map(|&m| seam[m].position).collect::<Vec<_>>());
            let down = unit(list.iter().fold([0.0; 3], |acc, &m| add(acc, seam[m].down)));
            if norm(down) == 0.0 {
                continue;
            }
            let p = st.positions[*v as usize];
            let h = norm(sub(p, anchor));
            correction[*v as usize] = Some(mul(sub(add(anchor, mul(down, h)), p), tangent));
        }
        let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); st.positions.len()];
        for t in &st.triangles {
            for k in 0..3 {
                neighbours[t[k] as usize].push(t[(k + 1) % 3]);
                neighbours[t[(k + 1) % 3] as usize].push(t[k]);
            }
        }
        let mut spread: Vec<Point> = correction.iter().map(|c| c.unwrap_or([0.0; 3])).collect();
        for _ in 0..TANGENT_SPREAD {
            let previous = spread.clone();
            for v in 0..spread.len() {
                if correction[v].is_some() || copy_of.contains_key(&(v as u32)) || st.band[v] <= 0.0 || neighbours[v].is_empty() {
                    continue;
                }
                let avg = mul(neighbours[v].iter().fold([0.0; 3], |acc, &u| add(acc, previous[u as usize])), 1.0 / neighbours[v].len() as f64);
                spread[v] = mul(avg, st.band[v]);
            }
        }
        for (p, c) in st.positions.iter_mut().zip(&spread) {
            *p = add(*p, *c);
        }
    }
    improve_fans(&mut st, 20);
    if smooth > 0 {
        let Stitched { positions, triangles, before, band, .. } = &mut st;
        let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); positions.len()];
        for t in triangles.iter() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                neighbours[a as usize].push(b);
                neighbours[b as usize].push(a);
            }
        }
        for list in &mut neighbours {
            list.sort_unstable();
            list.dedup();
        }
        for _ in 0..smooth {
            let previous = positions.clone();
            for v in 0..positions.len() {
                if before[v].is_none() || band[v] <= 0.0 || neighbours[v].is_empty() {
                    continue;
                }
                let avg = centroid(&neighbours[v].iter().map(|&u| previous[u as usize]).collect::<Vec<_>>());
                positions[v] = lerp(previous[v], avg, 0.5 * band[v]);
            }
        }
    }
    Ok(st)
}

pub fn limit_positions(quads: &[[u32; 4]], positions: &[Point]) -> Vec<Point> {
    let n = positions.len();
    let mut valence = vec![0usize; n];
    let mut edge_sum = vec![[0.0; 3]; n];
    let mut diag_sum = vec![[0.0; 3]; n];
    let mut boundary = vec![false; n];
    let halves: HashSet<(u32, u32)> = quads.iter().flat_map(|q| (0..4).map(move |k| (q[k], q[(k + 1) % 4]))).collect();
    for &(a, b) in &halves {
        if !halves.contains(&(b, a)) {
            boundary[a as usize] = true;
            boundary[b as usize] = true;
        }
    }
    for q in quads {
        for k in 0..4 {
            let v = q[k] as usize;
            valence[v] += 1;
            edge_sum[v] = add(edge_sum[v], add(positions[q[(k + 1) % 4] as usize], positions[q[(k + 3) % 4] as usize]));
            diag_sum[v] = add(diag_sum[v], positions[q[(k + 2) % 4] as usize]);
        }
    }
    (0..n)
        .map(|v| {
            let m = valence[v] as f64;
            if valence[v] < 3 || boundary[v] {
                return positions[v];
            }
            mul(add(add(mul(positions[v], m * m), mul(edge_sum[v], 2.0)), diag_sum[v]), 1.0 / (m * (m + 5.0)))
        })
        .collect()
}

fn min_angle(p: &[Point], t: [u32; 3]) -> f64 {
    (0..3)
        .map(|k| angle_deg(sub(p[t[(k + 1) % 3] as usize], p[t[k] as usize]), sub(p[t[(k + 2) % 3] as usize], p[t[k] as usize])))
        .fold(180.0, f64::min)
}

fn improve_fans(st: &mut Stitched, max_passes: usize) -> usize {
    let mut flips = 0;
    for _ in 0..max_passes {
        let mut changed = false;
        let mut edges: HashMap<(u32, u32), usize> = HashMap::new();
        for (i, t) in st.triangles.iter().enumerate() {
            for k in 0..3 {
                edges.insert((t[k], t[(k + 1) % 3]), i);
            }
        }
        let copies: HashSet<u32> = st.seam_copies.iter().map(|c| c.0).collect();
        let mut touched = vec![false; st.triangles.len()];
        for i in 0..st.triangles.len() {
            if !st.fan[i] || touched[i] {
                continue;
            }
            for k in 0..3 {
                let t1 = st.triangles[i];
                let (a, b, c) = (t1[k], t1[(k + 1) % 3], t1[(k + 2) % 3]);
                if copies.contains(&a) && copies.contains(&b) {
                    continue;
                }
                let Some(&j) = edges.get(&(b, a)) else { continue };
                if j == i || touched[j] || !st.fan[j] {
                    continue;
                }
                let t2 = st.triangles[j];
                let kb = (0..3).find(|&m| t2[m] == b).unwrap();
                let d = t2[(kb + 2) % 3];
                if d == c || edges.contains_key(&(c, d)) || edges.contains_key(&(d, c)) {
                    continue;
                }
                let uv1 = st.uvs[i];
                let uv2 = st.uvs[j];
                let ka2 = (kb + 1) % 3;
                if uv1[k] != uv2[ka2] || uv1[(k + 1) % 3] != uv2[kb] {
                    continue;
                }
                let n1 = [a, d, c];
                let n2 = [d, b, c];
                let before = min_angle(&st.positions, t1).min(min_angle(&st.positions, t2));
                let after = min_angle(&st.positions, n1).min(min_angle(&st.positions, n2));
                let reference = add(unit(face_normal(&st.positions, t1)), unit(face_normal(&st.positions, t2)));
                if after <= before + 1e-9 || dot(unit(face_normal(&st.positions, n1)), unit(reference)) < 0.7 || dot(unit(face_normal(&st.positions, n2)), unit(reference)) < 0.7 {
                    continue;
                }
                let (ua, ub, uc, ud) = (uv1[k], uv1[(k + 1) % 3], uv1[(k + 2) % 3], uv2[(kb + 2) % 3]);
                st.triangles[i] = n1;
                st.uvs[i] = [ua, ud, uc];
                st.triangles[j] = n2;
                st.uvs[j] = [ud, ub, uc];
                touched[i] = true;
                touched[j] = true;
                flips += 1;
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
    flips
}

pub fn ring_field(ring_points: &[Point], targets: &[Point], width: f64) -> Field {
    Field { ring: ring_points.to_vec(), displacement: targets.iter().zip(ring_points).map(|(t, p)| sub(*t, *p)).collect(), width }
}

pub fn seam_delta(seam: &[SeamPoint], param: f64, period: f64) -> Point {
    let n = seam.len();
    let i = seam.partition_point(|s| s.param <= param);
    let (a, b) = ((i + n - 1) % n, i % n);
    let span = cyclic_delta(seam[a].param, seam[b].param, period);
    let t = if span.abs() > 1e-12 { (cyclic_delta(seam[a].param, param, period) / span).clamp(0.0, 1.0) } else { 0.0 };
    let da = sub(seam[a].position, seam[a].base);
    let db = sub(seam[b].position, seam[b].base);
    lerp(da, db, t)
}

#[derive(Clone, Copy, Serialize)]
pub struct Limits {
    pub gap_m: f64,
    pub dihedral_deg: f64,
    pub fold_deg: f64,
    pub penetration_m: f64,
    pub reach_m: f64,
    pub near_m: f64,
}

impl Limits {
    pub fn exact() -> Limits {
        Limits { gap_m: 1e-5, dihedral_deg: 40.0, fold_deg: 120.0, penetration_m: 0.0005, reach_m: 0.08, near_m: 0.025 }
    }

    pub fn any_body() -> Limits {
        Limits { gap_m: 1e-4, ..Limits::exact() }
    }
}

struct Grid {
    cell: f64,
    cells: HashMap<(i64, i64, i64), Vec<u32>>,
}

impl Grid {
    fn key(&self, p: Point) -> (i64, i64, i64) {
        ((p[0] / self.cell).floor() as i64, (p[1] / self.cell).floor() as i64, (p[2] / self.cell).floor() as i64)
    }

    fn build(positions: &[Point], triangles: &[[u32; 3]], keep: &[bool], cell: f64) -> Grid {
        let mut g = Grid { cell, cells: HashMap::new() };
        for (i, t) in triangles.iter().enumerate() {
            if !keep[i] {
                continue;
            }
            let pts = t.map(|v| positions[v as usize]);
            let lo = g.key([0, 1, 2].map(|k| pts.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min)));
            let hi = g.key([0, 1, 2].map(|k| pts.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max)));
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        g.cells.entry((x, y, z)).or_default().push(i as u32);
                    }
                }
            }
        }
        g
    }

    fn nearest(&self, p: Point, positions: &[Point], triangles: &[[u32; 3]]) -> Option<(f64, Point, bool, u32)> {
        let k = self.key(p);
        let mut best: Option<(f64, Point, bool, u32)> = None;
        let mut seen = HashSet::new();
        for x in -1..=1 {
            for y in -1..=1 {
                for z in -1..=1 {
                    if let Some(list) = self.cells.get(&(k.0 + x, k.1 + y, k.2 + z)) {
                        for &t in list {
                            if !seen.insert(t) {
                                continue;
                            }
                            let tri = triangles[t as usize].map(|v| positions[v as usize]);
                            let (q, interior) = closest_point_triangle(p, tri[0], tri[1], tri[2]);
                            let d = norm(sub(p, q));
                            if best.is_none_or(|b| d < b.0) {
                                best = Some((d, q, interior, t));
                            }
                        }
                    }
                }
            }
        }
        best
    }
}

pub fn closest_point_triangle(p: Point, a: Point, b: Point, c: Point) -> (Point, bool) {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return (a, false);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return (b, false);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (add(a, mul(ab, v)), false);
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return (c, false);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (add(a, mul(ac, w)), false);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (add(b, mul(sub(c, b), w)), false);
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    (add(a, add(mul(ab, v), mul(ac, w))), true)
}

fn stats(values: &mut [f64]) -> Value {
    if values.is_empty() {
        return json!({"count": 0});
    }
    values.sort_by(f64::total_cmp);
    let p = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    json!({"count": values.len(), "max": values[values.len() - 1], "p95": p(0.95), "median": p(0.5),
           "mean": values.iter().sum::<f64>() / values.len() as f64})
}

fn vertex_normals(positions: &[Point], triangles: &[[u32; 3]], n: usize) -> Vec<Point> {
    let mut out = vec![[0.0; 3]; n];
    for t in triangles {
        let f = face_normal(positions, *t);
        for &v in t {
            if (v as usize) < n {
                out[v as usize] = add(out[v as usize], f);
            }
        }
    }
    out
}

pub fn gates(head_positions: &[Point], head_triangles: &[[u32; 3]], seam: &[SeamPoint], st: &Stitched, limits: &Limits) -> (Value, Vec<String>) {
    let h = head_positions.len() as u32;
    let mut failed = Vec::new();
    let mut gap = 0.0f64;
    let mut merged: Vec<u32> = (0..st.positions.len() as u32).map(|i| h + i).collect();
    for &(s, head) in &st.seam_copies {
        gap = gap.max(norm(sub(st.positions[s as usize], head_positions[head as usize])));
        merged[s as usize] = head;
    }
    if gap >= limits.gap_m {
        failed.push(format!("seam gap {:.3e} m >= {:.0e} m", gap, limits.gap_m));
    }
    let on_seam: HashSet<u32> = seam.iter().map(|s| s.vertex).collect();
    let body_tris: Vec<[u32; 3]> = st.triangles.iter().map(|t| t.map(|v| merged[v as usize])).collect();
    let mut directed: HashMap<(u32, u32), u32> = HashMap::new();
    let mut undirected: HashMap<(u32, u32), u32> = HashMap::new();
    for t in head_triangles.iter().chain(&body_tris) {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *directed.entry((a, b)).or_default() += 1;
            *undirected.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let touches = |e: &(u32, u32)| on_seam.contains(&e.0) || on_seam.contains(&e.1);
    let open_seam = undirected.iter().filter(|(e, c)| **c == 1 && touches(e)).count();
    let nonmanifold = undirected.iter().filter(|(e, c)| **c > 2 && touches(e)).count();
    let orientation = directed.iter().filter(|(e, c)| **c > 1 && touches(e)).count();
    let seam_edges_unmatched = (0..seam.len())
        .filter(|&m| {
            let (a, b) = (seam[m].vertex, seam[(m + 1) % seam.len()].vertex);
            undirected.get(&(a.min(b), a.max(b))).copied().unwrap_or(0) != 2
        })
        .count();
    let body_open_elsewhere = undirected.iter().filter(|(e, c)| **c == 1 && !touches(e)).count();
    if open_seam > 0 || nonmanifold > 0 || orientation > 0 || seam_edges_unmatched > 0 {
        failed.push(format!("seam topology: {open_seam} open, {nonmanifold} non-manifold, {orientation} orientation conflicts, {seam_edges_unmatched} seam edges not shared by exactly two triangles"));
    }
    let mut edge_tris: HashMap<(u32, u32), Vec<(bool, usize)>> = HashMap::new();
    for (i, t) in head_triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if on_seam.contains(&a) && on_seam.contains(&b) {
                edge_tris.entry((a.min(b), a.max(b))).or_default().push((true, i));
            }
        }
    }
    for (i, t) in body_tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if let Some(list) = edge_tris.get_mut(&(a.min(b), a.max(b))) {
                list.push((false, i));
            }
        }
    }
    let mut all_positions: Vec<Point> = head_positions.to_vec();
    all_positions.extend_from_slice(&st.positions);
    let mut dihedral = Vec::new();
    for m in 0..seam.len() {
        let (a, b) = (seam[m].vertex, seam[(m + 1) % seam.len()].vertex);
        if let Some(list) = edge_tris.get(&(a.min(b), a.max(b))) {
            let head = list.iter().find(|x| x.0);
            let body = list.iter().find(|x| !x.0);
            if let (Some(hh), Some(bb)) = (head, body) {
                let nh = face_normal(head_positions, head_triangles[hh.1]);
                let nb = face_normal(&all_positions, body_tris[bb.1]);
                dihedral.push(angle_deg(nh, nb));
            }
        }
    }
    let head_vn = vertex_normals(head_positions, head_triangles, head_positions.len());
    let body_vn = vertex_normals(&st.positions, &st.triangles, st.positions.len());
    let mut one_sided: Vec<f64> = st.seam_copies.iter().map(|&(s, hv)| angle_deg(head_vn[hv as usize], body_vn[s as usize])).collect();
    let dihedral_max = dihedral.iter().copied().fold(0.0f64, f64::max);
    if dihedral_max > limits.dihedral_deg || dihedral.len() != seam.len() {
        failed.push(format!("normal continuity: max seam dihedral {dihedral_max:.1} deg (limit {}), {} of {} seam edges measured", limits.dihedral_deg, dihedral.len(), seam.len()));
    }
    let in_band: Vec<bool> = st.triangles.iter().zip(&st.fan).map(|(t, f)| *f || t.iter().any(|&v| st.band[v as usize] > 0.0)).collect();
    let mut flips = 0usize;
    let mut degenerate = 0usize;
    let mut min_angle = 180.0f64;
    for (i, t) in st.triangles.iter().enumerate() {
        if !in_band[i] {
            continue;
        }
        let n_after = face_normal(&st.positions, *t);
        if norm(n_after) < 1e-12 {
            degenerate += 1;
        }
        if let (Some(a), Some(b), Some(c)) = (st.before[t[0] as usize], st.before[t[1] as usize], st.before[t[2] as usize]) {
            let n_before = cross(sub(b, a), sub(c, a));
            if dot(n_before, n_after) <= 0.0 {
                flips += 1;
            }
        }
        for k in 0..3 {
            let p = st.positions[t[k] as usize];
            let e1 = sub(st.positions[t[(k + 1) % 3] as usize], p);
            let e2 = sub(st.positions[t[(k + 2) % 3] as usize], p);
            min_angle = min_angle.min(angle_deg(e1, e2));
        }
    }
    let mut band_edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, t) in st.triangles.iter().enumerate() {
        if in_band[i] {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                band_edges.entry((a.min(b), a.max(b))).or_default().push(i);
            }
        }
    }
    let mut band_interior: Vec<f64> = band_edges
        .values()
        .filter(|l| l.len() == 2 && !st.fan[l[0]] && !st.fan[l[1]])
        .map(|l| angle_deg(face_normal(&st.positions, st.triangles[l[0]]), face_normal(&st.positions, st.triangles[l[1]])))
        .collect();
    let mut head_row_edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, t) in head_triangles.iter().enumerate() {
        if t.iter().any(|v| on_seam.contains(v)) {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                head_row_edges.entry((a.min(b), a.max(b))).or_default().push(i);
            }
        }
    }
    let mut head_row: Vec<f64> = head_row_edges
        .values()
        .filter(|l| l.len() == 2)
        .map(|l| angle_deg(face_normal(head_positions, head_triangles[l[0]]), face_normal(head_positions, head_triangles[l[1]])))
        .collect();
    let worst_seam = dihedral.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(m, d)| json!({"param": seam[m].param, "deg": d, "position": seam[m].position}));
    let folds = band_edges
        .values()
        .filter(|l| l.len() == 2 && angle_deg(face_normal(&st.positions, st.triangles[l[0]]), face_normal(&st.positions, st.triangles[l[1]])) > limits.fold_deg)
        .count();
    if flips > 0 || folds > 0 || degenerate > 0 {
        failed.push(format!("blend band: {flips} flipped, {folds} folded edges, {degenerate} degenerate triangles"));
    }
    let seam_points: Vec<Point> = seam.iter().map(|s| s.position).collect();
    let near_seam = |p: Point| -> bool {
        (0..seam_points.len()).any(|i| closest_on_segment(p, seam_points[i], seam_points[(i + 1) % seam_points.len()]).0 < limits.reach_m)
    };
    let head_keep: Vec<bool> = head_triangles.iter().map(|t| !t.iter().any(|v| on_seam.contains(v))).collect();
    let head_grid = Grid::build(head_positions, head_triangles, &head_keep, limits.near_m);
    let copies: HashSet<u32> = st.seam_copies.iter().map(|c| c.0).collect();
    let mut body_inside = 0usize;
    let mut body_depth = 0.0f64;
    for (v, p) in st.positions.iter().enumerate() {
        if copies.contains(&(v as u32)) || !near_seam(*p) {
            continue;
        }
        if let Some((d, q, interior, t)) = head_grid.nearest(*p, head_positions, head_triangles)
            && interior
            && d < limits.near_m
        {
            let s = dot(sub(*p, q), unit(face_normal(head_positions, head_triangles[t as usize])));
            if s < -limits.penetration_m {
                body_inside += 1;
                body_depth = body_depth.max(-s);
            }
        }
    }
    let body_keep: Vec<bool> = st.triangles.iter().map(|t| !t.iter().any(|v| copies.contains(v))).collect();
    let body_grid = Grid::build(&st.positions, &st.triangles, &body_keep, limits.near_m);
    let mut head_inside = 0usize;
    let mut head_depth = 0.0f64;
    for (v, p) in head_positions.iter().enumerate() {
        if on_seam.contains(&(v as u32)) || !near_seam(*p) {
            continue;
        }
        if let Some((d, q, interior, t)) = body_grid.nearest(*p, &st.positions, &st.triangles)
            && interior
            && d < limits.near_m
        {
            let s = dot(sub(*p, q), unit(face_normal(&st.positions, st.triangles[t as usize])));
            if s < -limits.penetration_m {
                head_inside += 1;
                head_depth = head_depth.max(-s);
            }
        }
    }
    if body_inside > 0 || head_inside > 0 {
        failed.push(format!("interpenetration near the neck: {body_inside} body vertices inside the head (deepest {:.2} mm), {head_inside} head vertices inside the body (deepest {:.2} mm)", body_depth * 1000.0, head_depth * 1000.0));
    }
    let report = json!({
        "pass": failed.is_empty(),
        "failed": failed,
        "limits": limits,
        "watertight": {"seam_points": seam.len(), "seam_copies": st.seam_copies.len(), "max_gap_m": gap},
        "topology": {"open_seam_edges": open_seam, "nonmanifold_seam_edges": nonmanifold, "orientation_conflicts": orientation,
                     "seam_edges_not_shared_by_two": seam_edges_unmatched, "body_open_edges_elsewhere": body_open_elsewhere},
        "normals": {"seam_dihedral_deg": stats(&mut dihedral), "one_sided_vertex_normal_deg": stats(&mut one_sided), "worst_seam_edge": worst_seam,
                    "baseline_body_band_dihedral_deg": stats(&mut band_interior), "baseline_head_seam_row_dihedral_deg": stats(&mut head_row),
                    "meaning": "angle between the face normals of the head and body triangles on each seam edge; the baselines are the same angle between neighbouring triangles inside the body band and inside the head's seam row"},
        "band": {"triangles": in_band.iter().filter(|b| **b).count(), "flips": flips, "folded_edges": folds, "degenerate": degenerate, "min_angle_deg": min_angle},
        "interpenetration": {"body_vertices_inside_head": body_inside, "body_depth_mm": body_depth * 1000.0,
                             "head_vertices_inside_body": head_inside, "head_depth_mm": head_depth * 1000.0,
                             "reach_mm": limits.reach_m * 1000.0, "near_mm": limits.near_m * 1000.0},
    });
    let failed_list = report["failed"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    (report, failed_list)
}

pub fn two_sided_normals(head_positions: &[Point], head_triangles: &[[u32; 3]], st: &Stitched) -> (Vec<(u32, Point)>, Vec<Point>) {
    let head_vn = vertex_normals(head_positions, head_triangles, head_positions.len());
    let mut body_vn = vertex_normals(&st.positions, &st.triangles, st.positions.len());
    let mut seam = Vec::new();
    for &(s, hv) in &st.seam_copies {
        let n = unit(add(head_vn[hv as usize], body_vn[s as usize]));
        seam.push((hv, n));
        body_vn[s as usize] = n;
    }
    (seam, body_vn.into_iter().map(unit).collect())
}

pub enum Plant {
    Gap,
    Crack,
    Crease,
    Flip,
    Penetrate,
}

impl Plant {
    pub fn parse(text: &str) -> Result<Plant, String> {
        Ok(match text {
            "gap" => Plant::Gap,
            "crack" => Plant::Crack,
            "crease" => Plant::Crease,
            "flip" => Plant::Flip,
            "penetrate" => Plant::Penetrate,
            other => return Err(format!("plant must be gap, crack, crease, flip or penetrate, got {other}")),
        })
    }
}

pub fn plant(st: &mut Stitched, kind: &Plant, seam_centroid: Point) -> Result<String, String> {
    match kind {
        Plant::Gap => {
            let &(s, _) = st.seam_copies.first().ok_or("no seam copies")?;
            st.positions[s as usize][0] += 2e-4;
            Ok(format!("moved seam copy {s} by 0.2 mm in x"))
        }
        Plant::Crack => {
            let i = st.fan.iter().position(|f| *f).ok_or("no fan triangle")?;
            st.triangles.remove(i);
            st.uvs.remove(i);
            st.fan.remove(i);
            Ok(format!("removed fan triangle {i}"))
        }
        Plant::Crease => {
            let mut moved = 0;
            let copies: HashSet<u32> = st.seam_copies.iter().map(|c| c.0).collect();
            let fan_vertices: HashSet<u32> = st.triangles.iter().zip(&st.fan).filter(|(_, f)| **f).flat_map(|(t, _)| t.iter().copied()).collect();
            for v in fan_vertices {
                if copies.contains(&v) {
                    continue;
                }
                let p = st.positions[v as usize];
                let radial = unit([p[0] - seam_centroid[0], p[1] - seam_centroid[1], 0.0]);
                st.positions[v as usize] = add(p, mul(radial, 0.03));
                moved += 1;
            }
            Ok(format!("pushed {moved} fan vertices 30 mm outward"))
        }
        Plant::Flip => {
            let i = st
                .triangles
                .iter()
                .enumerate()
                .position(|(i, t)| !st.fan[i] && t.iter().any(|&v| st.band[v as usize] > 0.5))
                .or_else(|| st.fan.iter().position(|f| *f))
                .ok_or("no band triangle")?;
            st.triangles[i].swap(1, 2);
            st.uvs[i].swap(1, 2);
            Ok(format!("reversed band triangle {i}"))
        }
        Plant::Penetrate => {
            let mut moved = 0;
            for v in 0..st.positions.len() {
                if st.before[v].is_some() && st.band[v] > 0.3 {
                    st.positions[v][2] += 0.025;
                    moved += 1;
                }
            }
            Ok(format!("raised {moved} band vertices 25 mm"))
        }
    }
}

pub struct BodyLod {
    pub kept: Vec<u32>,
    pub triangles: Vec<[u32; 3]>,
    pub uvs: Vec<[Uv; 3]>,
}

pub struct BodyPack {
    pub mode: String,
    pub mesh: Mesh,
    pub ring: Vec<RingVertex>,
    pub ring_targets: Vec<Point>,
    pub seam_loop: Vec<u32>,
    pub seam_base: Vec<Point>,
    pub per_span: usize,
    pub lods: Vec<BodyLod>,
    pub source: Vec<u32>,
}

impl BodyPack {
    pub fn period(&self) -> f64 {
        self.seam_loop.len() as f64 / self.per_span as f64
    }

    pub fn pivot(&self) -> Point {
        centroid(&self.seam_base)
    }

    pub fn lod_count(&self) -> usize {
        1 + self.lods.len()
    }

    pub fn lod_mesh(&self, lod: usize) -> Result<(Mesh, Vec<u32>), String> {
        if lod == 0 {
            return Ok((self.mesh.clone(), (0..self.mesh.positions.len() as u32).collect()));
        }
        let l = self.lods.get(lod - 1).ok_or_else(|| format!("the body has {} LODs, LOD{lod} asked", self.lod_count()))?;
        let positions = l.kept.iter().map(|&k| self.mesh.positions[k as usize]).collect();
        Ok((Mesh { positions, triangles: l.triangles.clone(), uvs: l.uvs.clone() }, l.kept.clone()))
    }
}

#[derive(Clone, Copy, Serialize)]
pub struct Controls {
    pub blend_m: f64,
    pub neck_offset_m: f64,
    pub head_scale: f64,
    pub body_scale: f64,
    pub smooth: usize,
    pub tangent: f64,
}

impl Controls {
    pub fn check(&self) -> Result<(), String> {
        let finite = [self.blend_m, self.neck_offset_m, self.head_scale, self.body_scale].iter().all(|x| x.is_finite());
        if !finite || self.blend_m < 0.0 || self.blend_m > 0.5 || self.neck_offset_m.abs() > 0.2 {
            return Err("blend must be 0..500 mm and the neck offset within +-200 mm, all finite".into());
        }
        if !(0.5..=2.0).contains(&self.head_scale) || !(0.5..=2.0).contains(&self.body_scale) || self.smooth > 50 || !(0.0..=1.0).contains(&self.tangent) {
            return Err("head and body scale must be 0.5..2, smoothing 0..50 iterations and tangent 0..1".into());
        }
        Ok(())
    }
}

pub fn scale_about(positions: &mut [Point], pivot: Point, s: f64) {
    if s != 1.0 {
        for p in positions.iter_mut() {
            *p = add(pivot, mul(sub(*p, pivot), s));
        }
    }
}

pub fn head_seam(pack: &BodyPack, positions: &[Point], triangles: &[[u32; 3]], dense_of: &[u32]) -> Result<Vec<SeamPoint>, String> {
    let index: HashMap<u32, usize> = pack.seam_loop.iter().enumerate().map(|(i, &d)| (d, i)).collect();
    let loops = oriented_loops(triangles)?;
    let neck = loops
        .into_iter()
        .filter(|l| l.iter().all(|&v| index.contains_key(&dense_of[v as usize])))
        .max_by_key(|l| l.len())
        .ok_or("the head has no boundary loop on the body pack's seam: the pack belongs to another dense base")?;
    let mut seam: Vec<SeamPoint> = neck
        .iter()
        .map(|&v| {
            let i = index[&dense_of[v as usize]];
            SeamPoint { vertex: v, param: i as f64 / pack.per_span as f64, position: positions[v as usize], base: pack.seam_base[i], down: [0.0; 3] }
        })
        .collect();
    let start = (0..seam.len()).min_by(|&a, &b| seam[a].param.total_cmp(&seam[b].param)).unwrap_or(0);
    seam.rotate_left(start);
    let ascending = seam.windows(2).all(|w| w[1].param > w[0].param);
    if !ascending {
        seam.reverse();
        let start = (0..seam.len()).min_by(|&a, &b| seam[a].param.total_cmp(&seam[b].param)).unwrap_or(0);
        seam.rotate_left(start);
        if !seam.windows(2).all(|w| w[1].param > w[0].param) {
            return Err("the head's neck loop does not follow the seam order".into());
        }
    }
    let at: HashMap<u32, usize> = seam.iter().enumerate().map(|(i, s)| (s.vertex, i)).collect();
    let n = seam.len();
    let mut down = vec![[0.0; 3]; n];
    for t in triangles {
        for k in 0..3 {
            let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
            if let (Some(&ia), Some(&ib)) = (at.get(&a), at.get(&b))
                && ((ia + 1) % n == ib || (ib + 1) % n == ia)
            {
                let (pa, pb, pc) = (positions[a as usize], positions[b as usize], positions[c as usize]);
                let e = sub(pb, pa);
                let w = sub(mul(add(pa, pb), 0.5), pc);
                let along = dot(w, e) / dot(e, e).max(1e-30);
                let d = unit(sub(w, mul(e, along)));
                down[ia] = add(down[ia], d);
                down[ib] = add(down[ib], d);
            }
        }
    }
    for (s, d) in seam.iter_mut().zip(down) {
        s.down = unit(d);
    }
    Ok(seam)
}

pub fn stitch_lod(pack: &BodyPack, lod: usize, seam: &[SeamPoint], controls: &Controls) -> Result<Stitched, String> {
    controls.check()?;
    let period = pack.period();
    let (mut mesh, kept) = pack.lod_mesh(lod)?;
    let local: HashMap<u32, u32> = kept.iter().enumerate().map(|(i, &k)| (k, i as u32)).collect();
    let ring0: Vec<Point> = pack.ring.iter().map(|r| pack.mesh.positions[r.vertex as usize]).collect();
    let c = centroid(&ring0);
    let control = |p: Point| -> Point {
        let mut q = add(c, mul(sub(p, c), controls.body_scale));
        q[2] += controls.neck_offset_m;
        q
    };
    let ring_points: Vec<Point> = ring0.iter().map(|p| control(*p)).collect();
    let targets: Vec<Point> = pack.ring.iter().zip(&pack.ring_targets).map(|(r, t)| add(*t, seam_delta(seam, r.param, period))).collect();
    let field = ring_field(&ring_points, &targets, controls.blend_m);
    for p in mesh.positions.iter_mut() {
        *p = control(*p);
    }
    let ring: Vec<RingVertex> = pack.ring.iter().filter_map(|r| local.get(&r.vertex).map(|&v| RingVertex { vertex: v, param: r.param })).collect();
    stitch(&mesh, &ring, period, seam, &field, controls.smooth, controls.tangent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(n: usize, z: f64) -> (Vec<Point>, Vec<[u32; 3]>) {
        let mut p = Vec::new();
        for i in 0..n {
            let a = i as f64 / n as f64 * std::f64::consts::TAU;
            p.push([a.cos() * 0.1, a.sin() * 0.1, z]);
        }
        (p, Vec::new())
    }

    #[test]
    fn spline_controls_reproduce_their_own_curve() {
        let (ring, _) = grid(12, 1.0);
        let mut samples = Vec::new();
        let n = ring.len();
        for i in 0..n {
            for s in 0..8 {
                let b = basis(s as f64 / 8.0);
                let mut p = [0.0; 3];
                for (w, j) in b.iter().zip([(i + n - 1) % n, i, (i + 1) % n, (i + 2) % n]) {
                    p = add(p, mul(ring[j], *w));
                }
                samples.push(p);
            }
        }
        let (controls, worst) = bspline_controls(&samples, 8).unwrap();
        assert!(worst < 1e-12);
        assert!(controls.iter().zip(&ring).all(|(a, b)| norm(sub(*a, *b)) < 1e-10));
    }

    #[test]
    fn a_tube_zips_onto_a_finer_seam_without_cracks() {
        let n = 8usize;
        let fine = 4usize;
        let mut head_p = Vec::new();
        for i in 0..n * fine {
            let a = i as f64 / (n * fine) as f64 * std::f64::consts::TAU;
            head_p.push([a.cos() * 0.1, a.sin() * 0.1, 1.0]);
        }
        for i in 0..n * fine {
            let a = i as f64 / (n * fine) as f64 * std::f64::consts::TAU;
            head_p.push([a.cos() * 0.1, a.sin() * 0.1, 1.05]);
        }
        let m = (n * fine) as u32;
        let mut head_t = Vec::new();
        for i in 0..m {
            let j = (i + 1) % m;
            head_t.push([i, j, m + j]);
            head_t.push([i, m + j, m + i]);
        }
        let mut body_p = Vec::new();
        for z in [1.0, 0.95] {
            for i in 0..n {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                body_p.push([a.cos() * 0.1, a.sin() * 0.1, z]);
            }
        }
        let k = n as u32;
        let mut body_t = Vec::new();
        for i in 0..k {
            let j = (i + 1) % k;
            body_t.push([j, i, k + i]);
            body_t.push([j, k + i, k + j]);
        }
        let body = Mesh { positions: body_p.clone(), uvs: vec![[[0.0; 2]; 3]; body_t.len()], triangles: body_t };
        let ring: Vec<RingVertex> = (0..k).map(|i| RingVertex { vertex: i, param: i as f64 }).collect();
        let seam: Vec<SeamPoint> = (0..m)
            .map(|i| SeamPoint { vertex: i, param: i as f64 / fine as f64, position: head_p[i as usize], base: head_p[i as usize], down: [0.0, 0.0, -1.0] })
            .collect();
        let field = ring_field(&body_p[..n], &body_p[..n], 0.03);
        let st = stitch(&body, &ring, n as f64, &seam, &field, 0, 1.0).unwrap();
        let (report, failed) = gates(&head_p, &head_t, &seam, &st, &Limits::exact());
        assert!(failed.is_empty(), "{report}");
        assert_eq!(report["topology"]["open_seam_edges"], 0);
        for kind in ["gap", "crack", "flip"] {
            let mut planted = stitch(&body, &ring, n as f64, &seam, &field, 0, 1.0).unwrap();
            plant(&mut planted, &Plant::parse(kind).unwrap(), [0.0, 0.0, 1.0]).unwrap();
            let (_, failed) = gates(&head_p, &head_t, &seam, &planted, &Limits::exact());
            assert!(!failed.is_empty(), "{kind} did not fire");
        }
    }
}
