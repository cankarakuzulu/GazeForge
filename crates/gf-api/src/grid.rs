use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use gf_geom::face_chart::{self as fc, Cell, CellQuery, FaceChart};
use serde_json::{Map, Value, json};

use crate::Ctx;
use crate::commands::{obj, shape};
use crate::designer::{self, HEAD_CATEGORIES, Loaded};
use crate::error::ApiError;

pub struct Cells {
    pub chart: FaceChart,
    pub kept: Vec<u32>,
    pub game_uv: Vec<[f64; 2]>,
    pub triangles: Vec<[u32; 3]>,
    dense_uv: Vec<[f64; 2]>,
}

impl Cells {
    pub fn new(dense_positions: &[[f64; 3]], kept: &[u32], triangles: &[[u32; 3]]) -> Result<Cells, String> {
        Cells::with_tip(dense_positions, kept, triangles, fc::NOSE_TIP)
    }

    pub fn with_tip(dense_positions: &[[f64; 3]], kept: &[u32], triangles: &[[u32; 3]], nose_tip: usize) -> Result<Cells, String> {
        let chart = FaceChart::from_base(dense_positions, nose_tip)?;
        if let Some(bad) = kept.iter().find(|&&d| d as usize >= dense_positions.len()) {
            return Err(format!("the game base keeps dense vertex {bad}, past the {} base vertices", dense_positions.len()));
        }
        if let Some(bad) = triangles.iter().flatten().find(|&&g| g as usize >= kept.len()) {
            return Err(format!("a game triangle names vertex {bad}, past the {} game vertices", kept.len()));
        }
        let dense_uv = chart.coords_all(dense_positions);
        let game_uv = kept.iter().map(|&d| dense_uv[d as usize]).collect();
        Ok(Cells { chart, kept: kept.to_vec(), game_uv, triangles: triangles.to_vec(), dense_uv })
    }

    pub fn dense_count(&self) -> usize {
        self.dense_uv.len()
    }

    pub fn dense_uv(&self, d: usize) -> Option<[f64; 2]> {
        self.dense_uv.get(d).copied()
    }

    pub fn game_cell(&self, g: usize) -> Option<Cell> {
        self.game_uv.get(g).map(|&uv| fc::cell_at(uv))
    }

    pub fn dense_cell(&self, d: usize) -> Option<Cell> {
        self.dense_uv(d).map(fc::cell_at)
    }

    pub fn point_uv(&self, t: usize, bary: [f64; 3]) -> Option<[f64; 2]> {
        let tri = self.triangles.get(t)?;
        Some(fc::interpolate(tri.map(|g| self.game_uv[g as usize]), bary))
    }

    pub fn point_cell(&self, t: usize, bary: [f64; 3]) -> Option<Cell> {
        let tri = self.triangles.get(t)?;
        Some(fc::point_cell(tri.map(|g| self.game_uv[g as usize]), bary))
    }

    pub fn triangle_cell(&self, t: usize) -> Option<Cell> {
        self.point_cell(t, [1.0 / 3.0; 3])
    }

    pub fn annotate(&self, value: &mut Value) -> usize {
        let mut n = 0;
        self.walk(value, &mut n);
        n
    }

    fn walk(&self, value: &mut Value, n: &mut usize) {
        match value {
            Value::Array(items) => items.iter_mut().for_each(|v| self.walk(v, n)),
            Value::Object(map) => {
                let mut adds: Vec<(String, Value)> = Vec::new();
                let name = |c: Option<Cell>| c.map_or(Value::Null, |c| json!(fc::cell_name(c)));
                let index = |v: &Value| v.as_u64().map(|x| x as usize);
                for (k, v) in map.iter() {
                    match k.as_str() {
                        "tri" | "triangle" if v.is_u64() => adds.push(("cell".into(), name(index(v).and_then(|t| self.triangle_cell(t))))),
                        "vertex" | "game_vertex" if v.is_u64() => adds.push(("cell".into(), name(index(v).and_then(|g| self.game_cell(g))))),
                        "dense_vertex" if v.is_u64() => adds.push(("cell".into(), name(index(v).and_then(|d| self.dense_cell(d))))),
                        "worst_visible" | "worst_visible_raw" if v.is_array() => {
                            let cells: Vec<Value> = v.as_array().unwrap().iter().map(|row| name(row.get(0).and_then(index).and_then(|t| self.triangle_cell(t)))).collect();
                            adds.push((format!("{k}_cells"), Value::Array(cells)));
                        }
                        "lost_vertices" | "added_vertices" if v.is_array() => {
                            let cells: Vec<Value> = v.as_array().unwrap().iter().map(|g| name(index(g).and_then(|g| self.game_cell(g)))).collect();
                            adds.push((format!("{k}_cells"), Value::Array(cells)));
                        }
                        _ => {}
                    }
                }
                for v in map.values_mut() {
                    self.walk(v, n);
                }
                for (k, v) in adds {
                    if !map.contains_key(&k) {
                        *n += match &v {
                            Value::Array(a) => a.len(),
                            _ => 1,
                        };
                        map.insert(k, v);
                    }
                }
            }
            _ => {}
        }
    }
}

pub(crate) struct Grid {
    key: String,
    cells: Cells,
    game_of: Vec<u32>,
    base_game: Vec<[f64; 3]>,
    regions: Vec<u8>,
    names: Vec<String>,
    skipped: usize,
    targets: usize,
    anchors: Vec<(String, u32)>,
    sanity: fc::Sanity,
}

impl Grid {
    fn region(&self, g: Option<usize>) -> Value {
        g.and_then(|g| self.regions.get(g)).map_or(Value::Null, |&r| json!(self.names[r as usize]))
    }

    fn nearest_corner(&self, t: usize, bary: [f64; 3]) -> Option<usize> {
        let tri = self.cells.triangles.get(t)?;
        let p = [0, 1, 2].map(|k| (0..3).map(|i| bary[i] * self.base_game[tri[i] as usize][k]).sum::<f64>());
        let d = |g: u32| (0..3).map(|k| (self.base_game[g as usize][k] - p[k]).powi(2)).sum::<f64>();
        let mut best = 0;
        for i in 1..3 {
            if d(tri[i]) < d(tri[best]) {
                best = i;
            }
        }
        Some(tri[best] as usize)
    }

    fn game(&self, d: usize) -> Option<usize> {
        self.game_of.get(d).filter(|&&g| g != u32::MAX).map(|&g| g as usize)
    }

    fn located(&self, kind: &str, index: usize, dense: Option<usize>, game: Option<usize>, uv: Option<[f64; 2]>, cell: Option<Cell>) -> Value {
        let Some(uv) = uv else {
            return json!({"kind": kind, "index": index, "error": format!("{kind} {index} is out of range")});
        };
        let cell = cell.unwrap_or_else(|| fc::cell_at(uv));
        let at = game.map(|g| self.base_game[g].map(|x| round(x * 1000.0, 2)));
        json!({"kind": kind, "index": index, "dense": dense, "game": game, "cell": fc::cell_name(cell), "label": fc::cell_label(cell),
               "region": self.region(game), "chart": [round(uv[0], 3), round(uv[1], 3)], "base_mm": at})
    }
}

fn round(x: f64, digits: i32) -> f64 {
    let s = 10f64.powi(digits);
    (x * s).round() / s
}

static GRID: OnceLock<Mutex<Option<Arc<Grid>>>> = OnceLock::new();

fn target_order(loaded: &Loaded, input: &Value) -> Vec<String> {
    if input.get("targets").is_some_and(Value::is_array) {
        return designer::pack_ids(loaded, input);
    }
    let mut ids: Vec<(&str, &String)> = loaded
        .catalog
        .targets
        .iter()
        .filter(|(_, e)| HEAD_CATEGORIES.contains(&e.category.as_str()))
        .map(|(id, e)| (e.category.as_str(), id))
        .collect();
    ids.sort();
    ids.into_iter().map(|(_, id)| id.clone()).collect()
}

fn grid(input: &Value) -> Result<(Arc<Grid>, bool), ApiError> {
    let (loaded, _) = designer::load(input)?;
    let ids = target_order(&loaded, input);
    let key = format!("{}|{}", loaded.key, ids.join(","));
    let slot = GRID.get_or_init(|| Mutex::new(None));
    let mut held = slot.lock().map_err(|_| ApiError::internal("the grid cache lock is poisoned"))?;
    if let Some(g) = held.as_ref().filter(|g| g.key == key) {
        return Ok((g.clone(), true));
    }
    let game = &loaded.game;
    let cells = Cells::new(&loaded.dense.positions, &game.kept, &game.triangles).map_err(ApiError::invalid)?;
    let mut game_of = vec![u32::MAX; loaded.dense.positions.len()];
    for (g, &d) in game.kept.iter().enumerate() {
        game_of[d as usize] = g as u32;
    }
    let base_game: Vec<[f64; 3]> = game.kept.iter().map(|&d| loaded.dense.positions[d as usize]).collect();
    let mut names: Vec<String> = vec!["rest of head".into()];
    let mut category_of = Vec::with_capacity(ids.len());
    for id in &ids {
        let entry = loaded.catalog.targets.get(id).ok_or_else(|| ApiError::not_found(format!("target {id} is not in the catalog")))?;
        let c = entry.category.as_str();
        if c == "head" {
            category_of.push(0);
            continue;
        }
        let at = names.iter().position(|n| n == c).unwrap_or_else(|| {
            names.push(c.to_string());
            names.len() - 1
        });
        category_of.push(at);
    }
    let rows = designer::pack_rows(&loaded, &ids)?;
    let (regions, skipped) = fc::designer_regions(game.kept.len(), &category_of, &rows.offsets, &rows.index, &rows.delta);
    let mut best: BTreeMap<(u32, u32), (f64, u32)> = BTreeMap::new();
    for (g, &uv) in cells.game_uv.iter().enumerate() {
        let c = fc::cell_at(uv);
        let k = c.col as f64 - fc::FRONT_COLUMN as f64;
        let mut du = (uv[0] - k * fc::COLUMN_DEG + 180.0).rem_euclid(360.0) - 180.0;
        du /= fc::COLUMN_DEG;
        let dv = (uv[1] - (c.row as f64 - 0.5) * fc::ROW_MM) / fc::ROW_MM;
        let d = du * du + dv * dv;
        let slot = best.entry((c.col, c.row)).or_insert((f64::INFINITY, 0));
        if d < slot.0 {
            *slot = (d, g as u32);
        }
    }
    let anchors = best.into_iter().map(|((col, row), (_, g))| (fc::cell_label(Cell { col, row, sub: 0 }), g)).collect();
    let sanity = fc::sanity(&cells.chart, &cells.game_uv, &base_game, &cells.triangles);
    let grid = Arc::new(Grid { key, cells, game_of, base_game, regions, names, skipped, targets: ids.len(), anchors, sanity });
    *held = Some(grid.clone());
    Ok((grid, false))
}

fn base_props() -> Value {
    json!({"store": {"type": "string", "minLength": 1, "description": "path of the store directory"},
           "pack": {"type": "string", "minLength": 1, "description": "dense pack reference"},
           "catalog": {"type": "string", "minLength": 1, "description": "target catalog reference (its head targets define the regions)"},
           "game": {"type": "string", "minLength": 1, "description": "game base reference"},
           "targets": {"type": "array", "items": {"type": "string"}, "description": "the region targets in order, as designer.pack takes them; default: the head targets sorted by category then id"}})
}

fn with(extra: Value) -> Value {
    let mut base = base_props();
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

const BASE: [&str; 4] = ["store", "pack", "catalog", "game"];

pub fn chart_input() -> Value {
    obj(base_props(), &BASE)
}

pub fn chart_output() -> Value {
    shape(json!({"definition": {"type": "object"}, "vertices": {"type": "integer"}, "chart_f32_b64": {"type": "string"}, "cells_u32_b64": {"type": "string"},
                 "regions_u8_b64": {"type": "string"}, "region_names": {"type": "array"}, "anchors": {"type": "array"}, "sanity": {"type": "object"}, "cached": {"type": "boolean"}}))
}

pub fn chart(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let (g, cached) = grid(input)?;
    let uv = designer::f32_base64(g.cells.game_uv.iter().flatten().copied());
    let packed: Vec<u8> = g.cells.game_uv.iter().flat_map(|&uv| fc::pack(fc::cell_at(uv)).to_le_bytes()).collect();
    Ok(json!({"definition": g.cells.chart.definition(), "vertices": g.cells.game_uv.len(), "triangles": g.cells.triangles.len(),
              "chart_f32_b64": uv, "cells_u32_b64": designer::base64(&packed), "cell_packing": "col << 24 | row << 4 | subcell, little-endian u32 per game vertex",
              "dense_u32_b64": designer::base64(&g.cells.kept.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>()),
              "regions_u8_b64": designer::base64(&g.regions), "region_names": g.names, "regions_skipped_wide_targets": g.skipped, "region_targets": g.targets,
              "anchors": g.anchors, "sanity": g.sanity, "cached": cached}))
}

pub fn locate_input() -> Value {
    let ints = |what: &str| json!({"type": "array", "items": {"type": "integer", "minimum": 0}, "description": what});
    obj(
        with(json!({"dense": ints("dense vertex indices"), "game_vertices": ints("game (LOD0) vertex indices"), "triangles": ints("game LOD0 triangle indices"),
                    "points": {"type": "array", "items": {"type": "object"}, "description": "surface points {triangle: game LOD0 triangle, bary: [b0, b1, b2]}"},
                    "report": {"type": ["object", "string"], "description": "a gate report (JSON object, or the path of a JSON file) whose game vertices and triangles get a cell"}})),
        &BASE,
    )
}

pub fn locate_output() -> Value {
    shape(json!({"locations": {"type": "array"}, "report": {"type": ["object", "null"]}, "annotated": {"type": "integer"}}))
}

pub fn locate(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let (g, cached) = grid(input)?;
    let ints = |k: &str| -> Vec<usize> { input.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(|x| x as usize).collect()).unwrap_or_default() };
    let mut out = Vec::new();
    for d in ints("dense") {
        out.push(g.located("dense", d, Some(d), g.game(d), g.cells.dense_uv(d), None));
    }
    for v in ints("game_vertices") {
        out.push(g.located("game", v, g.cells.kept.get(v).map(|&d| d as usize), Some(v).filter(|&v| v < g.cells.kept.len()), g.cells.game_uv.get(v).copied(), None));
    }
    for t in ints("triangles") {
        let uv = g.cells.point_uv(t, [1.0 / 3.0; 3]);
        out.push(g.located("triangle", t, None, None, uv, g.cells.triangle_cell(t)));
    }
    for (i, p) in input.get("points").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]).iter().enumerate() {
        let t = p.get("triangle").and_then(Value::as_u64).map(|x| x as usize);
        let b: Vec<f64> = p.get("bary").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        let (Some(t), [b0, b1, b2]) = (t, b.as_slice()) else {
            return Err(ApiError::invalid(format!("points[{i}] needs triangle and bary: [b0, b1, b2]")));
        };
        let bary = [*b0, *b1, *b2];
        let corner = g.nearest_corner(t, bary);
        let mut row = g.located("point", t, corner.map(|c| g.cells.kept[c] as usize), corner, g.cells.point_uv(t, bary), g.cells.point_cell(t, bary));
        if let (Some(c), Some(map)) = (corner, row.as_object_mut()) {
            map.insert("vertex_cell".into(), json!(fc::cell_name(fc::cell_at(g.cells.game_uv[c]))));
        }
        out.push(row);
    }
    let (report, annotated) = match input.get("report") {
        None => (Value::Null, 0),
        Some(Value::String(path)) => {
            let text = std::fs::read_to_string(Path::new(path)).map_err(|e| ApiError::not_found(format!("report {path}: {e}")))?;
            let mut v: Value = serde_json::from_str(&text).map_err(|e| ApiError::invalid(format!("report {path} is not JSON: {e}")))?;
            let n = g.cells.annotate(&mut v);
            (v, n)
        }
        Some(v) => {
            let mut v = v.clone();
            let n = g.cells.annotate(&mut v);
            (v, n)
        }
    };
    Ok(json!({"locations": out, "report": report, "annotated": annotated, "cached": cached,
              "rule": "a vertex's cell is its base chart coordinate's cell; a triangle's is its chart centroid's; a point's is the barycentric blend of its corners' chart coordinates (on a triangle around the chart axis: its largest-barycentric corner's cell), and its vertex is the corner nearest to it in space on the base, with that vertex's cell as vertex_cell; a report's tri/triangle, vertex/game_vertex and dense_vertex fields get a sibling cell, and its worst_visible, lost_vertices and added_vertices lists a sibling *_cells list"}))
}

pub fn resolve_input() -> Value {
    obj(
        with(json!({"address": {"type": "string", "minLength": 1, "description": "a cell (M12), a subcell (M12.3), \"dense N\", \"game vN\" or a whole GF@ address line"},
                    "accept_inconsistent": {"type": "boolean", "description": "resolve a GF@ line whose parts disagree (by its dense index) and list the disagreements under warnings, instead of refusing it; default false"}})),
        &["store", "pack", "catalog", "game", "address"],
    )
}

pub fn resolve_output() -> Value {
    shape(json!({"kind": {"type": "string"}, "cell": {"type": "string"}, "game_vertices": {"type": "array"}, "dense_count": {"type": "integer"}, "centroid_base_mm": {"type": "array"},
                 "consistent": {"type": "boolean"}, "warnings": {"type": "array"}, "checked": {"type": "array"}}))
}

fn number_after(text: &str, marker: &str) -> Option<usize> {
    let at = text.find(marker)? + marker.len();
    let digits: String = text[at..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Address {
    pub dense: Option<usize>,
    pub game: Option<usize>,
    pub cell: Option<CellQuery>,
    pub vertex_cell: Option<CellQuery>,
    pub surface: Option<(usize, usize, [f64; 3])>,
}

fn surface_part(body: &str) -> Option<(usize, usize, [f64; 3])> {
    let lod = number_after(body, "LOD")?;
    let at = body.find("LOD")?;
    let rest = &body[at..];
    let t = number_after(rest, " t")?;
    let open = rest.find(" b(")? + 3;
    let close = open + rest[open..].find(')')?;
    let parts: Vec<f64> = rest[open..close].split(',').map(|x| x.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
    let [b1, b2] = parts.as_slice() else { return None };
    Some((lod, t, [1.0 - b1 - b2, *b1, *b2]))
}

pub fn parse_address(text: &str) -> Address {
    let body = text.trim().strip_prefix(fc::ADDRESS_PREFIX).unwrap_or(text.trim());
    let cell = body.split('|').map(|f| f.split('(').next().unwrap_or("").trim()).find_map(fc::parse_cell);
    let vertex_cell = body.find("(v in ").and_then(|at| {
        let rest = &body[at + 6..];
        rest.split(')').next().and_then(fc::parse_cell)
    });
    Address { dense: number_after(body, "dense "), game: number_after(body, "game v"), cell, vertex_cell, surface: surface_part(body) }
}

pub fn unreadable_fields(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    let line = trimmed.strip_prefix(fc::ADDRESS_PREFIX);
    let mut out = Vec::new();
    let mut fields: Vec<&str> = line.unwrap_or(trimmed).split('|').map(str::trim).collect();
    if let Some(mm) = fields.iter().position(|f| f.ends_with(" mm")) {
        fields.truncate(mm + 1);
    }
    for prefix in ["game v", "dense", "region ", "LOD"] {
        let n = fields.iter().filter(|f| f.starts_with(prefix)).count();
        if n > 1 {
            out.push(format!("{n} fields start with {:?}; a line states each part once", prefix.trim()));
        }
    }
    for (i, field) in fields.into_iter().enumerate() {
        if i == 0 && line.is_some() {
            let (cell, vin) = match field.split_once('(') {
                Some((c, v)) => (c.trim(), Some(v.trim())),
                None => (field, None),
            };
            if !cell.is_empty() && fc::parse_cell(cell).is_none() {
                out.push(format!("the cell field {cell:?} is not a cell"));
            }
            if let Some(v) = vin
                && v.strip_prefix("v in ").and_then(|r| r.strip_suffix(')')).and_then(fc::parse_cell).is_none()
            {
                out.push(format!("the vertex cell field \"({v}\" is not \"(v in <cell>)\""));
            }
        }
        let numbers: &[&str] = if field.starts_with("game v") {
            if field.contains("dense") { &["game v", "dense "] } else { &["game v"] }
        } else if field.starts_with("dense") {
            &["dense "]
        } else {
            &[]
        };
        for marker in numbers {
            if number_after(field, marker).is_none() {
                out.push(format!("{field:?} has no readable number after {:?}", marker.trim()));
            }
        }
        if field.starts_with("LOD") && surface_part(field).is_none() {
            out.push(format!("the surface field {field:?} is not \"LODn tT b(b1,b2)\""));
        }
    }
    out
}

pub fn resolve(_: &Ctx, input: &Value) -> Result<Value, ApiError> {
    let (g, cached) = grid(input)?;
    let address = input.get("address").and_then(Value::as_str).unwrap_or("");
    let accept = input.get("accept_inconsistent").and_then(Value::as_bool).unwrap_or(false);
    let a = parse_address(address);
    let mut warnings: Vec<String> = unreadable_fields(address);
    let mut checked: Vec<&str> = vec!["every stated field reads"];
    if let Some(d) = a.dense
        && d >= g.cells.dense_count()
    {
        return Err(ApiError::not_found(format!("dense vertex {d} is past the {} dense vertices", g.cells.dense_count())));
    }
    if let Some(gv) = a.game
        && gv >= g.cells.kept.len()
    {
        return Err(ApiError::not_found(format!("game vertex {gv} is past the {} game vertices", g.cells.kept.len())));
    }
    let vertex = match (a.dense, a.game) {
        (Some(d), game) => {
            let kept = g.game(d);
            if let Some(gv) = game {
                checked.push("game = kept index of dense");
                match kept {
                    Some(k) if k == gv => {}
                    Some(k) => warnings.push(format!("the line says game v{gv} = dense {d}, but dense {d} is game v{k} (game v{gv} is dense {})", g.cells.kept[gv])),
                    None => warnings.push(format!("the line says game v{gv} = dense {d}, but the game base does not keep dense {d} (game v{gv} is dense {})", g.cells.kept[gv])),
                }
            }
            kept
        }
        (None, Some(gv)) => Some(gv),
        (None, None) => None,
    };
    let named = a.dense.is_some() || a.game.is_some();
    if named {
        let vcell = match (a.dense, vertex) {
            (_, Some(gv)) => fc::cell_at(g.cells.game_uv[gv]),
            (Some(d), None) => g.cells.dense_cell(d).unwrap(),
            (None, None) => unreachable!(),
        };
        if let Some(stated) = a.vertex_cell.or(a.cell) {
            checked.push("cell = the vertex's cell (or the stated v in cell)");
            if !stated.contains(vcell) {
                let which = if a.vertex_cell.is_some() { "its (v in) cell" } else { "its cell" };
                warnings.push(format!("the line gives {which} as {}, but the vertex lies in {}", stated.name(), fc::cell_name(vcell)));
            }
        }
        if let Some((lod, t, bary)) = a.surface {
            if lod == 0 {
                match g.cells.triangles.get(t) {
                    None => warnings.push(format!("the line names LOD0 triangle {t}, past the {} game triangles", g.cells.triangles.len())),
                    Some(tri) => {
                        if let (Some(stated), Some(hit)) = (a.cell, g.cells.point_cell(t, bary)) {
                            checked.push("cell = the cell of the LOD0 surface point");
                            if !stated.contains(hit) {
                                warnings.push(format!("the line gives the point's cell as {}, but LOD0 t{t} at those barycentrics lies in {}", stated.name(), fc::cell_name(hit)));
                            }
                        }
                        if let Some(gv) = vertex {
                            checked.push("the vertex is a corner of the LOD0 triangle");
                            if !tri.contains(&(gv as u32)) {
                                warnings.push(format!("game v{gv} is not a corner of LOD0 t{t}"));
                            }
                        }
                    }
                }
            } else {
                checked.push("the point of a LOD1+ line is not checked (the server holds the finest level only)");
            }
        }
        refuse_unless_accepted(&warnings, accept)?;
        let consistent = warnings.is_empty();
        if let Some(gv) = vertex {
            let d = g.cells.kept[gv] as usize;
            return Ok(json!({"kind": "vertex", "query": address, "game": gv, "dense": d, "kept": true, "cell": fc::cell_name(fc::cell_at(g.cells.game_uv[gv])), "region": g.region(Some(gv)),
                             "game_vertices": [gv], "game_count": 1, "dense_count": 1, "centroid_base_mm": g.base_game[gv].map(|x| round(x * 1000.0, 2)), "nearest_game": gv,
                             "consistent": consistent, "warnings": warnings, "checked": checked, "cached": cached}));
        }
        let d = a.dense.unwrap();
        return Ok(json!({"kind": "dense", "query": address, "dense": d, "game": null, "kept": false, "cell": fc::cell_name(vcell), "region": null, "game_vertices": [], "game_count": 0,
                         "dense_count": 1, "centroid_base_mm": null, "nearest_game": null, "note": format!("the game base does not keep dense vertex {d}"),
                         "consistent": consistent, "warnings": warnings, "checked": checked, "cached": cached}));
    }
    let Some(q) = a.cell else {
        return Err(ApiError::invalid(format!("{address:?} names no cell, dense vertex or game vertex"))
            .hint("give a cell like M12 or M12.3, \"dense 485123\", \"game v10234\" or a GF@ address line"));
    };
    refuse_unless_accepted(&warnings, accept)?;
    let consistent = warnings.is_empty();
    let members: Vec<usize> = (0..g.cells.game_uv.len()).filter(|&v| q.contains(fc::cell_at(g.cells.game_uv[v]))).collect();
    let dense_count = (0..g.cells.dense_count()).filter(|&d| q.contains(g.cells.dense_cell(d).unwrap())).count();
    let centroid = if members.is_empty() {
        None
    } else {
        let s = members.iter().fold([0.0; 3], |a, &v| [0, 1, 2].map(|k| a[k] + g.base_game[v][k]));
        Some(s.map(|x| x / members.len() as f64))
    };
    let nearest = centroid.and_then(|c| {
        members.iter().copied().min_by(|&a, &b| {
            let d = |v: usize| (0..3).map(|k| (g.base_game[v][k] - c[k]).powi(2)).sum::<f64>();
            d(a).total_cmp(&d(b))
        })
    });
    let mut regions: Map<String, Value> = Map::new();
    for &v in &members {
        let r = g.names[g.regions[v] as usize].clone();
        let n = regions.get(&r).and_then(Value::as_u64).unwrap_or(0) + 1;
        regions.insert(r, json!(n));
    }
    Ok(json!({"kind": if q.sub.is_some() { "subcell" } else { "cell" }, "query": address, "cell": q.name(), "game_vertices": members, "game_count": members.len(),
              "dense_count": dense_count, "centroid_base_mm": centroid.map(|c| c.map(|x| round(x * 1000.0, 2))), "nearest_game": nearest, "regions": regions,
              "consistent": consistent, "warnings": warnings, "checked": checked, "cached": cached}))
}

fn refuse_unless_accepted(warnings: &[String], accept: bool) -> Result<(), ApiError> {
    if warnings.is_empty() || accept {
        return Ok(());
    }
    Err(ApiError::invalid(format!("the address disagrees with itself: {}", warnings.join("; ")))
        .hint("pass accept_inconsistent: true to resolve it by its dense index (or game index when it has no dense index) with the disagreements listed under warnings"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_parse_their_parts() {
        let a = parse_address("GF@ M12.3 | region nose | game v10234 = dense 485123 | LOD0 t33012 b(0.21,0.35) | 12.3,45.6,-7.8 mm | x");
        assert_eq!((a.dense, a.game), (Some(485123), Some(10234)));
        assert_eq!(a.cell.unwrap().name(), "M12.3");
        assert!(a.vertex_cell.is_none());
        assert_eq!(a.surface.map(|s| (s.0, s.1)), Some((0, 33012)));
        assert!((a.surface.unwrap().2[0] - 0.44).abs() < 1e-9);
        let b = parse_address("GF@ M14.5 (v in M14.6) | region nose | game v1 = dense 2 | LOD2 t9 b(0.000,1.000) | 0,0,0 mm | y");
        assert_eq!((b.cell.unwrap().name(), b.vertex_cell.unwrap().name()), ("M14.5".to_string(), "M14.6".to_string()));
        assert_eq!(b.surface.map(|s| (s.0, s.1)), Some((2, 9)));
        assert_eq!(parse_address("dense 7").dense, Some(7));
        assert_eq!(parse_address(" f9 ").cell.unwrap().name(), "F9");
        assert_eq!(parse_address("nothing here"), Address::default());
    }

    #[test]
    fn stated_fields_that_do_not_read_are_named() {
        for good in [
            "GF@ M12.3 | region nose | game v10234 = dense 485123 | LOD0 t33012 b(0.21,0.35) | 12.3,45.6,-7.8 mm | x",
            "GF@ M14.5 (v in M14.6) | region nose | game v1 = dense 2 | LOD2 t9 b(0.000,1.000) | 0,0,0 mm | y",
            "GF@ | game v1060 = dense 446284",
            "GF@ M14.5 | region nose | game v1060 = dense 446284 | LOD0 t13322 b(1.000,0.000) | -1.7,-151.1,1473.9 mm | dense LOD study region x",
            "game v1060 = dense 446284 | LOD0 t5 b(0.300,0.300)",
            "dense 7",
            "game v3",
            "M12",
            " f9 ",
        ] {
            assert_eq!(unreadable_fields(good), Vec::<String>::new(), "{good}");
        }
        for (bad, n) in [
            ("GF@ Q | region nose | game v1060 = dense 446284 | LOD0 t5 b(0.300,0.300)", 1),
            ("GF@ Z99.5 | game v1060 = dense 446284", 1),
            ("GF@ M14.10 | game v1060 = dense 446284", 1),
            ("GF@ M14.0 | game v1060 = dense 446284", 1),
            ("GF@ M14.5 (v in Q) | game v1060 = dense 446284", 1),
            ("GF@ M14.5 (v M14.6) | game v1060 = dense 446284", 1),
            ("game v1060 = dense 99999999999999999999999", 1),
            ("dense 99999999999999999999999", 1),
            ("game vx = dense y", 2),
            ("GF@ M14.5 | game v1060 = dense 446284 | LOD0 t5 b(0.3)", 1),
            ("GF@ Q (v in Q) | game v1 = dense x | LOD1 t", 4),
            ("GF@ M14.5 | region nose | game v1060 = dense 446284 | game v5 = dense 7", 1),
            ("GF@ M14.5 | region nose | region chin | game v1060", 1),
        ] {
            assert_eq!(unreadable_fields(bad).len(), n, "{bad}: {:?}", unreadable_fields(bad));
        }
    }

    #[test]
    fn reports_get_cells_beside_their_vertices_and_triangles() {
        let dense = vec![[0.0, -0.1, 0.1], [0.05, -0.08, 0.1], [-0.05, -0.08, 0.1], [0.0, 0.1, 0.12]];
        let cells = Cells::with_tip(&dense, &[0, 1, 2], &[[0, 1, 2]], 0).unwrap();
        let mut report = json!({"triq": {"outside_box_examples": [{"tri": 0, "min_deg": 1.0}]}, "seams": {"worst": [{"vertex": 1}], "lost_vertices": [2, 9]},
                                "census": {"worst_visible": [[0, 70.0]]}, "path": "x"});
        let n = cells.annotate(&mut report);
        assert_eq!(n, 5);
        assert_eq!(report["triq"]["outside_box_examples"][0]["cell"], json!(fc::cell_name(cells.triangle_cell(0).unwrap())));
        assert_eq!(report["seams"]["worst"][0]["cell"], json!(fc::cell_name(cells.game_cell(1).unwrap())));
        assert_eq!(report["seams"]["lost_vertices_cells"][1], Value::Null);
        assert_eq!(report["census"]["worst_visible_cells"].as_array().unwrap().len(), 1);
    }
}
