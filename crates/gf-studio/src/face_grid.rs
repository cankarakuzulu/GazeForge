use std::sync::Arc;

use serde_json::{Value, json};

use crate::session::Session;

pub struct FaceGrid {
    pub definition: Value,
    pub letters: Vec<String>,
    pub column_deg: f64,
    pub row_mm: f64,
    pub centre_mm: [f64; 2],
    pub front: [f64; 2],
    pub top_mm: f64,
    pub front_column: String,
    pub axis_span_deg: f64,
    pub uv: Arc<Vec<f32>>,
    pub cells: Vec<u32>,
    pub dense: Vec<u32>,
    pub regions: Vec<u8>,
    pub region_names: Vec<String>,
    pub anchors: Vec<(String, usize)>,
}

pub struct Refs {
    pub store: String,
    pub pack: String,
    pub catalog: String,
    pub game: String,
    pub n: usize,
}

impl Refs {
    pub fn of(s: &Session) -> Refs {
        Refs { store: s.store.clone(), pack: s.pack.clone(), catalog: s.catalog.clone(), game: s.game.clone(), n: s.head.n }
    }

    pub fn key(&self) -> String {
        format!("{}|{}|{}|{}", self.store, self.pack, self.catalog, self.game)
    }
}

fn bytes(r: &Value, field: &str) -> Result<Vec<u8>, String> {
    gf_api::designer::from_base64(r[field].as_str().ok_or_else(|| format!("grid.chart returned no {field}"))?)
}

fn words(b: &[u8]) -> Vec<u32> {
    b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub fn load(s: &Refs) -> Result<FaceGrid, String> {
    let env = gf_api::call(&gf_api::Ctx::from_env(), "grid.chart", &json!({"store": s.store, "pack": s.pack, "catalog": s.catalog, "game": s.game}));
    if env["ok"] != true {
        return Err(env["error"]["message"].as_str().unwrap_or("grid.chart failed").to_string());
    }
    let r = &env["result"];
    let d = &r["definition"];
    let num = |v: &Value| v.as_f64().unwrap_or(0.0);
    let pair = |v: &Value| [num(&v[0]), num(&v[1])];
    let uv: Vec<f32> = bytes(r, "chart_f32_b64")?.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
    if uv.len() != s.n * 2 {
        return Err(format!("grid.chart returned {} chart coordinates for {} game vertices", uv.len() / 2, s.n));
    }
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    Ok(FaceGrid {
        definition: d.clone(),
        letters: match &d["letters"] {
            Value::String(s) => s.chars().map(String::from).collect(),
            v => v.as_array().into_iter().flatten().map(text).collect(),
        },
        column_deg: num(&d["column_deg"]),
        row_mm: num(&d["row_mm"]),
        centre_mm: pair(&d["centre_mm"]),
        front: pair(&d["front"]),
        top_mm: num(&d["top_mm"]),
        front_column: text(&d["front_column"]),
        axis_span_deg: d["axis_span_deg"].as_f64().unwrap_or(45.0),
        uv: Arc::new(uv),
        cells: words(&bytes(r, "cells_u32_b64")?),
        dense: words(&bytes(r, "dense_u32_b64")?),
        regions: bytes(r, "regions_u8_b64")?,
        region_names: r["region_names"].as_array().into_iter().flatten().map(text).collect(),
        anchors: r["anchors"].as_array().into_iter().flatten().filter_map(|a| Some((a[0].as_str()?.to_string(), a[1].as_u64()? as usize))).collect(),
    })
}

impl crate::world::World {
    pub fn grid_sync(&mut self) {
        let Some(s) = self.session.as_mut() else { return };
        let refs = Refs::of(s);
        if self.grid_key != refs.key() {
            self.grid_key = refs.key();
            self.grid = None;
            self.grid_axis = None;
            self.grid_error = None;
            self.grid_job = None;
            s.head.chart = None;
        }
        if let Some(g) = &self.grid
            && s.head.chart.is_none()
        {
            s.head.chart = Some(g.uv.clone());
        }
        if let Some(rx) = &self.grid_job
            && let Ok((key, out)) = rx.try_recv()
        {
            self.grid_job = None;
            if key == self.grid_key {
                match out {
                    Ok(g) => {
                        s.head.chart = Some(g.uv.clone());
                        self.grid_axis = Some(Arc::new(crate::grid_labels::axis(&g, &s.head.lods[0].indices, s.head.n)));
                        self.grid = Some(Arc::new(g));
                    }
                    Err(e) => self.grid_error = Some(e),
                }
            }
        }
        self.probe_finish();
        let Some(s) = self.session.as_mut() else { return };
        if !self.state.opts.face_grid && !self.grid_wanted {
            return;
        }
        if self.grid.is_none() && self.grid_error.is_none() && self.grid_job.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send((refs.key(), load(&refs)));
            });
            self.grid_job = Some(rx);
        }
        if let (Some(g), Some(e)) = (&self.grid, s.head.eyes.as_mut()) {
            let at = (s.head.revision, e.key);
            if self.state.opts.eyes_grid == "grid" && e.chart_of != Some(at) {
                e.chart = g.eye_chart(&e.positions);
                e.chart_of = Some(at);
            }
        }
    }
}

impl FaceGrid {
    pub fn eye_chart(&self, positions: &[f32]) -> Vec<f32> {
        let (c, f, top) = ([self.centre_mm[0] / 1000.0, self.centre_mm[1] / 1000.0], self.front, self.top_mm / 1000.0);
        let mut out = Vec::with_capacity(positions.len() / 3 * 2);
        for p in positions.chunks_exact(3) {
            let (rx, ry) = (p[0] as f64 - c[0], p[1] as f64 - c[1]);
            let mut u = (f[0] * ry - f[1] * rx).atan2(f[0] * rx + f[1] * ry).to_degrees();
            if u <= -180.0 {
                u += 360.0;
            }
            out.push(u as f32);
            out.push(((top - p[2] as f64) * 1000.0) as f32);
        }
        out
    }

    pub fn cell_name(&self, packed: u32) -> String {
        let col = (packed >> 24) as usize;
        format!("{}{}.{}", self.letters.get(col).map_or("?", String::as_str), (packed >> 4) & 0xfffff, packed & 15)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> FaceGrid {
        FaceGrid {
            definition: Value::Null,
            letters: "ABCDEFGHIJKLMNOPQRSTUVWX".chars().map(String::from).collect(),
            column_deg: 15.0,
            row_mm: 10.0,
            centre_mm: [0.0, 0.0],
            front: [0.0, -1.0],
            top_mm: 100.0,
            front_column: "M".into(),
            axis_span_deg: 45.0,
            uv: Arc::new(Vec::new()),
            cells: Vec::new(),
            dense: Vec::new(),
            regions: Vec::new(),
            region_names: Vec::new(),
            anchors: Vec::new(),
        }
    }

    #[test]
    fn eye_chart_measures_angle_from_the_front_and_depth_below_the_top() {
        let g = grid();
        let uv = g.eye_chart(&[0.0, -0.05, 0.08, 0.05, 0.0, 0.07, 0.0, 0.05, 0.1]);
        assert!((uv[0] - 0.0).abs() < 1e-4 && (uv[1] - 20.0).abs() < 1e-3, "{uv:?}");
        assert!((uv[2].abs() - 90.0).abs() < 1e-3 && (uv[3] - 30.0).abs() < 1e-3, "{uv:?}");
        assert!((uv[4] - 180.0).abs() < 1e-3 && uv[5].abs() < 1e-3, "{uv:?}");
        assert_eq!(g.cell_name((12 << 24) | (7 << 4) | 5), "M7.5");
    }
}
