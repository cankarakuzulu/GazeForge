use std::fmt;

use gf_geom::Uv;
use gf_geom::decimate::Decimated;
use gf_geom::lids::{self, Eye};
use gf_geom::placement::{self as pl, FitSettings};
use gf_geom::placement_f2::{self, F2Settings};
use gf_geom::placement_f3::{self, F3Settings, F3Stats};
use gf_geom::placement_f5::{self, F5Settings, F5Stats};
pub use gf_geom::prepared::Prepared;
use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlacementVersion {
    pub name: &'static str,
    pub version: u32,
}

pub const RELEASED: &[PlacementVersion] = &[
    PlacementVersion { name: "subset", version: 1 },
    PlacementVersion { name: "optimal", version: 1 },
    PlacementVersion { name: "fit", version: 1 },
    PlacementVersion { name: "f2", version: 1 },
    PlacementVersion { name: "f3", version: 1 },
    PlacementVersion { name: "f4", version: 1 },
    PlacementVersion { name: "f5", version: 1 },
];

pub const DEFAULT: PlacementVersion = PlacementVersion { name: "f3", version: 1 };

impl fmt::Display for PlacementVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.name, self.version)
    }
}

impl PlacementVersion {
    pub fn parse(text: &str) -> Result<PlacementVersion, String> {
        let names: Vec<String> = RELEASED.iter().map(|v| v.to_string()).collect();
        let refuse = || format!("placement {text} is not a released version; released: {}", names.join(", "));
        match text.split_once('/') {
            Some((name, version)) => {
                let version: u32 = version.parse().map_err(|_| refuse())?;
                RELEASED.iter().find(|v| v.name == name && v.version == version).copied().ok_or_else(refuse)
            }
            None => RELEASED.iter().filter(|v| v.name == text).max_by_key(|v| v.version).copied().ok_or_else(refuse),
        }
    }
}

pub struct DenseMesh<'a> {
    pub triangles: &'a [[u32; 3]],
    pub triangle_uvs: &'a [[Uv; 3]],
}

pub struct Placed {
    pub game: Vec<[f64; 3]>,
    pub triangle_uvs: Option<Vec<[Uv; 3]>>,
    pub report: Map<String, Value>,
}

pub trait Placement {
    fn version(&self) -> PlacementVersion;
    fn place(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated) -> Placed;
    fn place_prepared(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated, _prepared: &Prepared) -> Placed {
        self.place(positions, dense, base)
    }
}

pub struct Subset;

pub struct Optimal {
    pub max_move: f64,
}

pub struct Fit {
    pub settings: FitSettings,
}

pub struct F2 {
    pub settings: F2Settings,
    pub surface_uvs: bool,
}

pub struct F3 {
    pub settings: F3Settings,
    pub eyes: Vec<Eye>,
}

pub struct F5 {
    pub settings: F5Settings,
    pub eyes: Vec<Eye>,
}

fn report(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
    pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

impl Placement for Subset {
    fn version(&self) -> PlacementVersion {
        PlacementVersion { name: "subset", version: 1 }
    }
    fn place(&self, positions: &[[f64; 3]], _dense: &DenseMesh, base: &Decimated) -> Placed {
        let game = base.kept.iter().map(|&d| positions[d as usize]).collect();
        Placed { game, triangle_uvs: None, report: report(vec![("placement_stats", Value::Null)]) }
    }
}

impl Placement for Optimal {
    fn version(&self) -> PlacementVersion {
        PlacementVersion { name: "optimal", version: 1 }
    }
    fn place(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated) -> Placed {
        let (game, stats) = pl::optimal(positions, dense.triangles, base, self.max_move);
        Placed { game, triangle_uvs: None, report: report(vec![("placement_stats", json!(stats))]) }
    }
}

impl Placement for Fit {
    fn version(&self) -> PlacementVersion {
        PlacementVersion { name: "fit", version: 1 }
    }
    fn place(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated) -> Placed {
        let (game, stats) = pl::fit(positions, dense.triangles, base, &self.settings);
        Placed { game, triangle_uvs: None, report: report(vec![("placement_stats", json!(stats))]) }
    }
}

impl Placement for F2 {
    fn version(&self) -> PlacementVersion {
        PlacementVersion { name: "f2", version: 1 }
    }
    fn place(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated) -> Placed {
        let (game, uvs, stats) = placement_f2::place(positions, dense.triangles, dense.triangle_uvs, base, &self.settings);
        let triangle_uvs = if self.surface_uvs { uvs } else { base.triangle_uvs.clone() };
        let uv_mode = if self.surface_uvs { "surface" } else { "subset" };
        Placed {
            game,
            triangle_uvs: Some(triangle_uvs),
            report: report(vec![("uvs", json!(uv_mode)), ("settings", json!(self.settings)), ("placement_stats", json!(stats))]),
        }
    }
}

impl F3 {
    fn placed(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated, game: Vec<[f64; 3]>, triangle_uvs: Vec<[Uv; 3]>, stats: F3Stats) -> Placed {
        let lids = (!self.eyes.is_empty()).then(|| {
            let front = self.settings.clear.front_deg;
            let (dense_area, dense_depth) = lids::penetration_area(positions, dense.triangles, &self.eyes, front, 2);
            let (game_area, game_depth) = lids::penetration_area(&game, &base.triangles, &self.eyes, front, 6);
            json!({"dense_inside_mm2": dense_area * 1e6, "dense_deepest_mm": dense_depth * 1000.0,
                   "game_inside_mm2": game_area * 1e6, "game_deepest_mm": game_depth * 1000.0})
        });
        Placed {
            game,
            triangle_uvs: Some(triangle_uvs),
            report: report(vec![
                ("settings", json!(self.settings)),
                ("eyes", json!(self.eyes.len())),
                ("placement_stats", json!(stats)),
                ("lids", json!(lids)),
            ]),
        }
    }
}

impl Placement for F3 {
    fn version(&self) -> PlacementVersion {
        let name = if self.settings.f4.is_some() { "f4" } else { "f3" };
        PlacementVersion { name, version: 1 }
    }
    fn place(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated) -> Placed {
        let (game, triangle_uvs, stats) =
            placement_f3::place(positions, dense.triangles, dense.triangle_uvs, base, &self.eyes, &self.settings);
        self.placed(positions, dense, base, game, triangle_uvs, stats)
    }
    fn place_prepared(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated, prepared: &Prepared) -> Placed {
        if !prepared.fits(positions.len(), dense.triangles.len(), base, self.settings.uv_ring) {
            return self.place(positions, dense, base);
        }
        let (game, triangle_uvs, stats) =
            placement_f3::place_prepared(positions, dense.triangles, dense.triangle_uvs, base, &self.eyes, &self.settings, prepared);
        self.placed(positions, dense, base, game, triangle_uvs, stats)
    }
}

impl F5 {
    fn placed(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated, game: Vec<[f64; 3]>, triangle_uvs: Vec<[Uv; 3]>, stats: F5Stats) -> Placed {
        let lids = (!self.eyes.is_empty()).then(|| {
            let front = self.settings.eye_front_deg;
            let (dense_area, dense_depth) = lids::penetration_area(positions, dense.triangles, &self.eyes, front, 2);
            let (game_area, game_depth) = lids::penetration_area(&game, &base.triangles, &self.eyes, front, 6);
            json!({"dense_inside_mm2": dense_area * 1e6, "dense_deepest_mm": dense_depth * 1000.0,
                   "game_inside_mm2": game_area * 1e6, "game_deepest_mm": game_depth * 1000.0})
        });
        Placed {
            game,
            triangle_uvs: Some(triangle_uvs),
            report: report(vec![
                ("settings", json!(self.settings)),
                ("eyes", json!(self.eyes.len())),
                ("placement_stats", json!(stats)),
                ("lids", json!(lids)),
            ]),
        }
    }
}

impl Placement for F5 {
    fn version(&self) -> PlacementVersion {
        PlacementVersion { name: "f5", version: 1 }
    }
    fn place(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated) -> Placed {
        let (game, triangle_uvs, stats) =
            placement_f5::place(positions, dense.triangles, dense.triangle_uvs, base, &self.eyes, &self.settings);
        self.placed(positions, dense, base, game, triangle_uvs, stats)
    }
    fn place_prepared(&self, positions: &[[f64; 3]], dense: &DenseMesh, base: &Decimated, prepared: &Prepared) -> Placed {
        if !prepared.fits(positions.len(), dense.triangles.len(), base, self.settings.uv_ring) {
            return self.place(positions, dense, base);
        }
        let (game, triangle_uvs, stats) =
            placement_f5::place_prepared(positions, dense.triangles, dense.triangle_uvs, base, &self.eyes, &self.settings, prepared);
        self.placed(positions, dense, base, game, triangle_uvs, stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_versions_parse_and_print_the_same() {
        for v in RELEASED {
            assert_eq!(PlacementVersion::parse(&v.to_string()).unwrap(), *v);
        }
        assert_eq!(DEFAULT.to_string(), "f3/1");
        assert_eq!(PlacementVersion::parse("f3").unwrap(), DEFAULT);
        assert_eq!(PlacementVersion::parse("f5").unwrap(), PlacementVersion { name: "f5", version: 1 });
    }

    #[test]
    fn unreleased_versions_are_refused() {
        for bad in ["f3/2", "f5/2", "f6", "f3/x", "", "F3/1", "subset/0"] {
            assert!(PlacementVersion::parse(bad).is_err(), "{bad} accepted");
        }
    }

    #[test]
    fn the_released_list_is_append_only() {
        let names: Vec<String> = RELEASED.iter().map(|v| v.to_string()).collect();
        assert_eq!(names[..7], ["subset/1", "optimal/1", "fit/1", "f2/1", "f3/1", "f4/1", "f5/1"]);
    }
}
