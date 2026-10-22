use std::path::Path;

use gf_core::game_mesh::Point;
use gf_io::character as ch;
use serde_json::{Value, json};

use super::groom::{Rng, add, cross, dot, scale, sub, unit};
use super::strands::Strand;
use super::{Options, need, only};

pub const KEYS: &[&str] = &["store", "spec", "macro", "groom", "base", "name", "cards", "atlas"];
pub const COLUMNS: usize = 8;
const CARD_COLUMNS: usize = COLUMNS - 1;
const CAP_MIN: f64 = 0.25;
const CAP_FULL: f64 = 0.75;
const CAP_LIFT_M: f64 = 0.0015;
const CAP_VERTICES: usize = 700;
const CAP_OPAQUE_TO: f64 = 0.6;
const STRANDS_PER_COLUMN: usize = 70;
const WIDTH_MIN_M: f64 = 0.006;
const WIDTH_MAX_M: f64 = 0.04;
pub const FAR_SETS: [(usize, usize, usize, f64); 2] = [(4, 4, 1, 0.08), (5, 8, 2, 0.12)];
const UV_MARGIN: f64 = 0.004;
const CARDS_DEFAULT: usize = 600;
const ATLAS_DEFAULT: usize = 1024;

pub struct Cards {
    pub positions: Vec<Point>,
    pub uvs: Vec<[f64; 2]>,
    pub quads: Vec<[u32; 4]>,
    pub members: Vec<usize>,
    pub widths_m: Vec<f64>,
    pub cut: usize,
    pub cap: Vec<[u32; 3]>,
}

fn dist2(a: Point, b: Point) -> f64 {
    let d = sub(a, b);
    dot(d, d)
}

fn shape(s: &Strand) -> [Point; 3] {
    let n = s.points.len();
    if n == 0 { [[0.0; 3]; 3] } else { [s.points[0], s.points[n / 2], s.points[n - 1]] }
}

fn apart(a: &[Point; 3], b: &[Point; 3]) -> f64 {
    dist2(a[0], b[0]) + dist2(a[1], b[1]) + dist2(a[2], b[2])
}

pub fn build(strands: &[Strand], count: usize, centre: Point, widest: f64, blocked: &dyn Fn(Point) -> bool) -> Cards {
    let shapes: Vec<[Point; 3]> = strands.iter().map(shape).collect();
    let n = count.min(shapes.len());
    let mut seeds: Vec<usize> = Vec::with_capacity(n);
    if n > 0 {
        seeds.push(0);
        let mut nearest: Vec<f64> = shapes.iter().map(|s| apart(s, &shapes[0])).collect();
        while seeds.len() < n {
            let i = (0..shapes.len()).fold(0, |best, k| if nearest[k] > nearest[best] { k } else { best });
            seeds.push(i);
            for (k, s) in shapes.iter().enumerate() {
                nearest[k] = nearest[k].min(apart(s, &shapes[i]));
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (k, s) in shapes.iter().enumerate() {
        let c = (0..n).fold(0, |best, c| if apart(s, &shapes[seeds[c]]) < apart(s, &shapes[seeds[best]]) { c } else { best });
        groups[c].push(k);
    }
    let mut out = Cards { positions: Vec::new(), uvs: Vec::new(), quads: Vec::new(), members: Vec::new(), widths_m: Vec::new(), cut: 0, cap: Vec::new() };
    for group in &groups {
        let Some(rows) = group.iter().map(|&s| strands[s].points.len()).min().filter(|&r| r >= 2) else { continue };
        let line: Vec<Point> = (0..rows).map(|k| scale(group.iter().fold([0.0; 3], |a, &s| add(a, strands[s].points[k])), 1.0 / group.len() as f64)).collect();
        let widths: Vec<f64> = (0..rows)
            .map(|k| {
                let spread = (group.iter().map(|&s| dist2(strands[s].points[k], line[k])).sum::<f64>() / group.len() as f64).sqrt();
                (2.0 * spread).clamp(WIDTH_MIN_M, widest)
            })
            .collect();
        let mut previous: Option<Point> = None;
        let mut edges = Vec::with_capacity(rows);
        for k in 0..rows {
            let t = unit(sub(line[(k + 1).min(rows - 1)], line[k.saturating_sub(1)]));
            let c = cross(t, unit(sub(line[k], centre)));
            let mut side = if dot(c, c) > 1e-12 { unit(c) } else { previous.unwrap_or([1.0, 0.0, 0.0]) };
            if previous.is_some_and(|p| dot(p, side) < 0.0) {
                side = scale(side, -1.0);
            }
            previous = Some(side);
            let half = scale(side, widths[k] / 2.0);
            edges.push((sub(line[k], half), add(line[k], half)));
        }
        let used = (1..rows).find(|&k| blocked(line[k]) || blocked(edges[k].0) || blocked(edges[k].1)).unwrap_or(rows);
        if used < rows {
            out.cut += 1;
        }
        if used < 2 {
            continue;
        }
        let card = out.members.len();
        let column = card % CARD_COLUMNS;
        let (u0, u1) = (column as f64 / COLUMNS as f64 + UV_MARGIN, (column + 1) as f64 / COLUMNS as f64 - UV_MARGIN);
        let first = out.positions.len() as u32;
        for (k, (l, r)) in edges.iter().take(used).enumerate() {
            let v = 1.0 - k as f64 / (used - 1) as f64;
            out.positions.push(*l);
            out.positions.push(*r);
            out.uvs.push([u0, v]);
            out.uvs.push([u1, v]);
        }
        for k in 0..used as u32 - 1 {
            let (l, r) = (first + 2 * k, first + 2 * k + 1);
            out.quads.push([l, r, r + 2, l + 2]);
        }
        out.members.push(group.len());
        out.widths_m.push(widths[..used].iter().sum::<f64>() / used as f64);
    }
    out
}

pub fn halved(strands: &[Strand]) -> Vec<Strand> {
    strands
        .iter()
        .map(|s| {
            let n = s.points.len();
            let keep: Vec<usize> = (0..n).step_by(2).chain((n % 2 == 0 && n > 0).then_some(n - 1)).collect();
            Strand { points: keep.iter().map(|&k| s.points[k]).collect(), colours: keep.iter().filter_map(|&k| s.colours.get(k).copied()).collect() }
        })
        .collect()
}

pub fn append(a: &mut Cards, b: Cards) {
    let shift = a.positions.len() as u32;
    a.positions.extend(b.positions);
    a.uvs.extend(b.uvs);
    a.quads.extend(b.quads.into_iter().map(|q| q.map(|i| i + shift)));
    a.members.extend(b.members);
    a.widths_m.extend(b.widths_m);
    a.cut += b.cut;
    a.cap.extend(b.cap.into_iter().map(|t| t.map(|i| i + shift)));
}

pub fn scalp_cap(head: &[Point], triangles: &[[u32; 3]], uvs: &[[gf_geom::Uv; 3]], weight: &[f64], centre: Point) -> Cards {
    let normals = gf_core::game_mesh::game_normals(head, triangles);
    let picked: Vec<usize> = (0..triangles.len()).filter(|&i| triangles[i].iter().any(|&v| weight.get(v as usize).copied().unwrap_or(0.0) >= CAP_MIN)).collect();
    let chosen: Vec<[u32; 3]> = picked.iter().map(|&i| triangles[i]).collect();
    let mut index = std::collections::BTreeMap::new();
    for &v in chosen.iter().flatten() {
        let next = index.len() as u32;
        index.entry(v).or_insert(next);
    }
    let mut order: Vec<(u32, u32)> = index.iter().map(|(&v, &i)| (i, v)).collect();
    order.sort_unstable();
    let lifted: Vec<Point> = order.iter().map(|&(_, v)| add(head[v as usize], scale(normals[v as usize], CAP_LIFT_M))).collect();
    let weights: Vec<f64> = order.iter().map(|&(_, v)| weight[v as usize]).collect();
    let local: Vec<[u32; 3]> = chosen.iter().map(|t| t.map(|v| index[&v])).collect();
    let (kept, tris): (Vec<u32>, Vec<[u32; 3]>) = if lifted.len() > CAP_VERTICES {
        let settings = gf_geom::decimate::Settings {
            target_vertices: CAP_VERTICES,
            min_angle_deg: 4.0,
            max_angle_deg: 170.0,
            max_normal_turn_deg: 45.0,
            boundary_weight: 100.0,
            improve_below_deg: 20.0,
            flip_flat_deg: 15.0,
        };
        let head_uvs: Vec<[gf_geom::Uv; 3]> = picked.iter().map(|&i| uvs.get(i).copied().unwrap_or([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]])).collect();
        let d = gf_geom::decimate::decimate_levels(vec![lifted.clone()], &local, &head_uvs, &settings, None, &[], None).swap_remove(0);
        (d.kept, d.triangles)
    } else {
        ((0..lifted.len() as u32).collect(), local)
    };
    let column = CARD_COLUMNS;
    let (u0, u1) = (column as f64 / COLUMNS as f64 + UV_MARGIN, (column + 1) as f64 / COLUMNS as f64 - UV_MARGIN);
    let mut out = Cards { positions: Vec::new(), uvs: Vec::new(), quads: Vec::new(), members: Vec::new(), widths_m: Vec::new(), cut: 0, cap: tris };
    for &k in &kept {
        let p = lifted[k as usize];
        let turn = ((p[1] - centre[1]).atan2(p[0] - centre[0]) / std::f64::consts::TAU + 0.5) * 3.0;
        let wave = 1.0 - (2.0 * (turn - turn.floor()) - 1.0).abs();
        let w = ((weights[k as usize] - CAP_MIN) / (CAP_FULL - CAP_MIN)).clamp(0.0, 1.0);
        out.positions.push(p);
        out.uvs.push([u0 + (u1 - u0) * wave, 0.04 + 0.92 * w]);
    }
    out
}

pub fn atlas(size: usize, colour: [f64; 3], seed: u64) -> Vec<u8> {
    let mut rgb = vec![colour; size * size];
    let mut alpha = vec![0.0f64; size * size];
    let column = size as f64 / COLUMNS as f64;
    let mut rng = Rng(seed ^ 0x5EED_CA2D);
    for c in 0..CARD_COLUMNS {
        let (lo, hi) = ((c as f64 * column).ceil() as i64 + 1, ((c + 1) as f64 * column).floor() as i64 - 2);
        for _ in 0..STRANDS_PER_COLUMN {
            let x0 = (c as f64 + 0.08 + 0.84 * rng.next()) * column;
            let length = 0.8 + 0.2 * rng.next();
            let (amp, freq, phase) = (0.5 + 1.5 * rng.next(), 1.0 + 3.0 * rng.next(), std::f64::consts::TAU * rng.next());
            let half = 0.7 + 0.6 * rng.next();
            let bright = 0.75 + 0.5 * rng.next();
            let tone = colour.map(|x| (x * bright).clamp(0.0, 1.0));
            for y in 0..((length * size as f64) as usize).min(size) {
                let t = y as f64 / size as f64;
                let x = x0 + amp * (freq * std::f64::consts::TAU * t + phase).sin();
                let fade = ((length - t) / (0.2 * length)).clamp(0.0, 1.0);
                for xi in ((x - half - 1.0).floor() as i64).max(lo)..=((x + half + 1.0).ceil() as i64).min(hi) {
                    let a = (half + 0.5 - (xi as f64 + 0.5 - x).abs()).clamp(0.0, 1.0) * fade;
                    if a <= 0.0 {
                        continue;
                    }
                    let i = y * size + xi as usize;
                    rgb[i] = [0, 1, 2].map(|k| tone[k] * a + rgb[i][k] * (1.0 - a));
                    alpha[i] = a + alpha[i] * (1.0 - a);
                }
            }
        }
    }
    let (lo, hi) = ((CARD_COLUMNS as f64 * column).ceil() as usize + 1, ((COLUMNS as f64 * column).floor() as usize).saturating_sub(2).min(size - 1));
    let mut shade = vec![1.0f64; size];
    let mut edge = vec![0.0f64; size];
    for x in lo..=hi {
        shade[x] = 0.8 + 0.35 * rng.next();
        edge[x] = rng.next();
    }
    for _ in 0..3 {
        let prev = shade.clone();
        for x in lo + 1..hi {
            shade[x] = (prev[x - 1] + 2.0 * prev[x] + prev[x + 1]) / 4.0;
        }
    }
    for y in 0..size {
        let t = 1.0 - y as f64 / size as f64;
        for x in lo..=hi {
            let start = 0.04 + (CAP_OPAQUE_TO - 0.04) * edge[x];
            let a = ((t - 0.04) / (start - 0.04).max(1e-6)).clamp(0.0, 1.0);
            let i = y * size + x;
            rgb[i] = colour.map(|c| (c * shade[x]).clamp(0.0, 1.0));
            alpha[i] = if t >= start { 1.0 } else { a * a };
        }
    }
    rgb.iter().zip(&alpha).flat_map(|(c, &a)| [c[0], c[1], c[2], a].map(|x| (x * 255.0).round().clamp(0.0, 255.0) as u8)).collect()
}

pub fn obj_text(cards: &Cards, name: &str) -> String {
    let mut s = format!("o {}\n", name.replace(char::is_whitespace, "_"));
    for p in &cards.positions {
        s.push_str(&format!("v {:.6} {:.6} {:.6}\n", p[0], p[1], p[2]));
    }
    for t in &cards.uvs {
        s.push_str(&format!("vt {:.6} {:.6}\n", t[0], t[1]));
    }
    for t in &cards.cap {
        let [a, b, c] = t.map(|i| i + 1);
        s.push_str(&format!("f {a}/{a} {b}/{b} {c}/{c}\n"));
    }
    for q in &cards.quads {
        let [a, b, c, d] = q.map(|i| i + 1);
        s.push_str(&format!("f {a}/{a} {b}/{b} {c}/{c} {d}/{d}\n"));
    }
    s
}

fn srgb(c: f32) -> f64 {
    let c = c.clamp(0.0, 1.0) as f64;
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

pub fn run(opts: &Options) -> Result<Value, String> {
    only(opts, "groom cards", &[KEYS])?;
    let store = gf_io::store::Store::open(Path::new(need(opts, "store")?))?;
    let spec_path = need(opts, "spec")?;
    let spec = ch::read_spec(Path::new(spec_path))?;
    let source = opts.get("macro").map(|p| ch::read_macro_definition(Path::new(p))).transpose()?;
    let groom_value: Value = match opts.get("groom") {
        Some(p) => serde_json::from_str(&std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?).map_err(|e| format!("{p}: {e}"))?,
        None => spec.body.as_ref().and_then(|b| b.groom.clone()).ok_or("groom cards needs --groom GROOM.json or a character with a groom (body.groom)")?,
    };
    let g = super::groom::parse(&groom_value)?;
    let number = |key: &str, default: usize, lo: usize, hi: usize| -> Result<usize, String> {
        let v = match opts.get(key) {
            Some(t) => t.parse::<usize>().map_err(|_| format!("--{key} takes a whole number, got {t}"))?,
            None => default,
        };
        if (lo..=hi).contains(&v) { Ok(v) } else { Err(format!("--{key} must be from {lo} to {hi}, got {v}")) }
    };
    let count = number("cards", CARDS_DEFAULT, 1, 5000)?;
    let size = number("atlas", ATLAS_DEFAULT, 256, 4096)?;
    let name = opts.get("name").cloned().unwrap_or_else(|| format!("{} cards", g.name));
    let scalp = crate::body::scalp(&store, &spec, source.as_ref())?;
    let (strands, grown) = super::groom::grow(&g, &scalp.head, &scalp.triangles, &scalp.dense, &scalp.surface)?;
    let centre = scale(scalp.head.iter().fold([0.0; 3], |a, &p| add(a, p)), 1.0 / scalp.head.len().max(1) as f64);
    let face = super::groom::Face::on(&gf_core::base_landmarks::on(&scalp.dense)?)?;
    let blocked = |p: Point| face.keep_out(p).is_some();
    let mut cards = build(&strands, count, centre, WIDTH_MAX_M, &blocked);
    let (near_cards, near_vertices) = (cards.members.len(), cards.positions.len());
    let regions = gf_core::skin_regions::build(&scalp.head, &gf_core::game_mesh::game_normals(&scalp.head, &scalp.triangles), &scalp.triangles, &gf_core::base_landmarks::on(&scalp.dense)?)?;
    let scalp_index = gf_core::skin_regions::NAMES.iter().position(|n| *n == "scalp").ok_or("no scalp skin region")?;
    let weight: Vec<f64> = regions.weights.iter().map(|w| w[scalp_index] as f64).collect();
    let cap = scalp_cap(&scalp.head, &scalp.triangles, &scalp.triangle_uvs, &weight, centre);
    let (cap_vertices, cap_triangles) = (cap.positions.len(), cap.cap.len());
    append(&mut cards, cap);
    let mut levels = Vec::new();
    let mut far_rows = Vec::new();
    for (from, share, halvings, widest) in FAR_SETS {
        let mut thinned = halved(&strands);
        for _ in 1..halvings {
            thinned = halved(&thinned);
        }
        let far = build(&thinned, (count / share).max(1), centre, widest, &blocked);
        levels.push(gf_io::wearable_store::Level { from, first_face: cards.cap.len() + cards.quads.len() });
        far_rows.push(json!({"from_level": from, "cards": far.members.len(), "vertices": far.positions.len(), "rows": thinned.first().map_or(0, |s| s.points.len()),
                             "widest_mm": far.widths_m.iter().copied().fold(0.0, f64::max) * 1000.0}));
        append(&mut cards, far);
    }
    let png = gf_io::png::rgba(size, size, &atlas(size, g.colour.map(srgb), g.seed))?;
    let obj = obj_text(&cards, &name);
    let dir = store.root.join("cache").join("groom_cards").join(gf_core::hash::sha256_hex(&[obj.as_bytes(), &png].concat()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let file = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect::<String>();
    let (obj_path, png_path) = (dir.join(format!("{file}.obj")), dir.join(format!("{file}.png")));
    std::fs::write(&obj_path, &obj).map_err(|e| format!("{}: {e}", obj_path.display()))?;
    std::fs::write(&png_path, &png).map_err(|e| format!("{}: {e}", png_path.display()))?;
    let mut bind = Options::new();
    for key in ["store", "spec", "macro", "base"] {
        if let Some(v) = opts.get(key) {
            bind.insert(key.into(), v.clone());
        }
    }
    bind.insert("obj".into(), obj_path.to_string_lossy().into_owned());
    bind.insert("diffuse".into(), png_path.to_string_lossy().into_owned());
    bind.insert("name".into(), name.clone());
    bind.insert("frame".into(), "canonical".into());
    bind.insert("hide-under".into(), "off".into());
    bind.insert("double-sided".into(), "on".into());
    let bound = super::wearable::bind_with(&bind, levels)?;
    let members: Vec<usize> = cards.members[..near_cards].to_vec();
    Ok(json!({"ok": true, "command": "groom cards", "wearable": bound["wearable"], "name": name, "cards": members.len(), "vertices": near_vertices, "far": far_rows, "cap": {"vertices": cap_vertices, "triangles": cap_triangles, "lift_mm": CAP_LIFT_M * 1000.0}, "cut_at_face": cards.cut,
              "strands_per_card": {"min": members.iter().min(), "max": members.iter().max(), "mean": members.iter().sum::<usize>() as f64 / members.len().max(1) as f64},
              "card_width_mm": {"min": cards.widths_m[..near_cards].iter().copied().fold(f64::INFINITY, f64::min) * 1000.0, "max": cards.widths_m[..near_cards].iter().copied().fold(0.0, f64::max) * 1000.0},
              "atlas": {"size": size, "columns": COLUMNS, "file": png_path.to_string_lossy()}, "obj": obj_path.to_string_lossy(), "groom": grown, "bind": bound,
              "method": "the groom grown on this character's scalp; its strands grouped by shape (root, middle and tip together, around seeds spread by farthest-point sampling, so strands combed apart never share a card), one card per group: a strip along the group's mean strand, as wide as twice its spread (6 to 40 mm), lying across the hair (its side perpendicular to the strand and to the line from the head's centre); its UVs one of the atlas's first seven columns, root at the top; under the cards a cap over the scalp (the head's scalp skin region from 25 %, lifted 1.5 mm, decimated to 700 vertices) mapped into the eighth column, painted opaque and fading in streaks where the scalp weight falls, so no bare skin shows between cards and the hairline is soft; a card that would reach in front of the face (the zone the groom keeps its strands out of) ends at the row before, its texture fitted to what is left; the atlas paints strands in the groom's colour with tapered tips; from level 4 on, a quarter as many cards made the same way from every other strand point, up to 80 mm wide, and from level 5 an eighth as many from every fourth point, up to 120 mm, so the far levels keep their cover (all sets in one mesh, each after the one before: the wearable's levels); the cards bound to the character like any garment (wearable bind, canonical frame, hiding nothing, double-sided)"}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strand(root: Point, dir: Point) -> Strand {
        Strand { points: (0..5).map(|k| add(root, scale(dir, k as f64 * 0.02))).collect(), colours: vec![[0.1, 0.1, 0.1]; 5] }
    }

    #[test]
    fn cards_follow_their_strands_and_lie_across_them() {
        let mut strands = Vec::new();
        for i in 0..20 {
            let x = if i < 10 { -0.05 } else { 0.05 } + (i % 10) as f64 * 0.001;
            strands.push(strand([x, 0.0, 0.1], [0.0, 0.0, -1.0]));
        }
        let cards = build(&strands, 2, [0.0, 0.1, 0.0], WIDTH_MAX_M, &|_| false);
        assert_eq!(cards.members, vec![10, 10]);
        assert_eq!(cards.positions.len(), 2 * 2 * 5);
        assert_eq!(cards.quads.len(), 2 * 4);
        for card in 0..2 {
            let row = &cards.positions[card * 10..card * 10 + 2];
            let mid = scale(add(row[0], row[1]), 0.5);
            assert!((mid[2] - 0.1).abs() < 1e-12, "the root row sits at the roots: {mid:?}");
            let side = sub(row[1], row[0]);
            let out = sub(mid, [0.0, 0.1, 0.0]);
            assert!(side[2].abs() < 1e-12 && dot(side, out).abs() < 1e-12, "the card lies across the strands and the outward line: {side:?}");
            assert!(dot(side, side).sqrt() >= WIDTH_MIN_M - 1e-12);
        }
        assert_eq!(cards.uvs[0][1], 1.0);
        assert_eq!(cards.uvs[8][1], 0.0);
        assert!(cards.uvs[10][0] > cards.uvs[1][0], "the second card uses the next atlas column");
        let again = build(&strands, 2, [0.0, 0.1, 0.0], WIDTH_MAX_M, &|_| false);
        assert_eq!(again.positions, cards.positions);
        let cut = build(&strands, 2, [0.0, 0.1, 0.0], WIDTH_MAX_M, &|p| p[2] < 0.05);
        assert_eq!((cut.cut, cut.positions.len()), (2, 2 * 2 * 3), "each card ends before its first row below z 0.05");
        assert_eq!(cut.uvs[4][1], 0.0, "the shortened card still runs its texture root to tip");
        let half = halved(&strands);
        assert_eq!(half[0].points.len(), 3);
        assert_eq!(half[0].points[2], strands[0].points[4], "halving keeps the tip");
        let mut both = build(&strands, 2, [0.0, 0.1, 0.0], WIDTH_MAX_M, &|_| false);
        let far = build(&half, 1, [0.0, 0.1, 0.0], FAR_SETS[0].3, &|_| false);
        let (n, q) = (both.positions.len() as u32, both.quads.len());
        append(&mut both, far);
        assert_eq!(both.quads[q], [n, n + 1, n + 3, n + 2], "the far set's faces follow, indexed past the near set");
        assert!(both.widths_m[2] > both.widths_m[0], "one far card covers both groups: {:?}", both.widths_m);
    }

    #[test]
    fn the_cap_covers_the_scalp_lifted_and_fades_where_the_scalp_ends() {
        let n = 6;
        let head: Vec<Point> = (0..n * n).map(|k| [(k % n) as f64 * 0.01, (k / n) as f64 * 0.01, 0.0]).collect();
        let mut triangles = Vec::new();
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let v = (y * n + x) as u32;
                triangles.push([v, v + 1, v + n as u32 + 1]);
                triangles.push([v, v + n as u32 + 1, v + n as u32]);
            }
        }
        let weight: Vec<f64> = head.iter().map(|p| if p[0] > 0.025 { 1.0 } else { 0.0 }).collect();
        let uvs: Vec<[gf_geom::Uv; 3]> = triangles.iter().map(|t| t.map(|v| [head[v as usize][0] as f32, head[v as usize][1] as f32])).collect();
        let cap = scalp_cap(&head, &triangles, &uvs, &weight, [0.025, 0.025, -0.1]);
        assert!(!cap.cap.is_empty() && cap.cap.len() < triangles.len(), "only triangles touching the scalp: {}", cap.cap.len());
        assert!(cap.positions.iter().all(|p| (p[2].abs() - CAP_LIFT_M).abs() < 1e-12), "lifted off the skin");
        assert!(cap.positions.iter().all(|p| p[0] > 0.015), "nothing past one ring outside the scalp");
        for (p, uv) in cap.positions.iter().zip(&cap.uvs) {
            let want = if p[0] > 0.025 { 0.96 } else { 0.04 };
            assert!((uv[1] - want).abs() < 1e-12, "v follows the scalp weight: {p:?} {uv:?}");
            assert!(uv[0] > CARD_COLUMNS as f64 / COLUMNS as f64, "the cap uses the last column");
        }
    }

    #[test]
    fn strands_combed_apart_never_share_a_card() {
        let mut strands = Vec::new();
        for i in 0..10 {
            let x = i as f64 * 0.001;
            strands.push(strand([x, 0.0, 0.1], [-1.0, 0.0, -0.2]));
            strands.push(strand([x, 0.0, 0.1], [1.0, 0.0, -0.2]));
        }
        let cards = build(&strands, 2, [0.0, 0.1, 0.0], WIDTH_MAX_M, &|_| false);
        assert_eq!(cards.members, vec![10, 10]);
        for card in 0..2 {
            let tip = scale(add(cards.positions[card * 10 + 8], cards.positions[card * 10 + 9]), 0.5);
            assert!(tip[0].abs() > 0.05, "a card follows one side, not the average of both: {tip:?}");
        }
    }

    #[test]
    fn the_atlas_is_see_through_between_strands_and_stays_in_its_columns() {
        let size = 256;
        let a = atlas(size, [0.4, 0.3, 0.2], 7);
        assert_eq!(a.len(), size * size * 4);
        let alpha = |x: usize, y: usize| a[(y * size + x) * 4 + 3];
        let column = size / COLUMNS;
        for c in 0..COLUMNS {
            for y in 0..size {
                assert_eq!(alpha(c * column, y), 0, "column edges stay clear");
            }
        }
        let clear = (0..size * size).filter(|i| a[i * 4 + 3] < 250).count();
        assert!(clear > size * size / 10, "see-through: {clear}");
        let top_cover = (0..size).filter(|&x| alpha(x, 2) > 128).count();
        assert!(top_cover > size / 4, "the roots are covered: {top_cover}");
        let bottom_cover = (0..size).filter(|&x| alpha(x, size - 1) > 128).count();
        assert!(bottom_cover < top_cover, "the tips thin out: {bottom_cover} vs {top_cover}");
        assert_eq!(atlas(size, [0.4, 0.3, 0.2], 7), a);
    }
}
