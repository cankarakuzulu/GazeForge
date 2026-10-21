use gf_geom::eyeball::EyeballShape;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IrisLook {
    pub inner: [f32; 3],
    pub outer: [f32; 3],
    pub limbal: [f32; 3],
    pub limbal_width: f32,
    pub collarette: f32,
    pub fibres: f32,
    pub crypts: f32,
    pub sector: Option<Sector>,
    pub seed: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sector {
    pub colour: [f32; 3],
    pub start_deg: f32,
    pub width_deg: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScleraLook {
    pub tint: [f32; 3],
    pub veins: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorneaLook {
    pub ior: f32,
    pub roughness: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EyeModel {
    pub id: String,
    pub name: String,
    pub family: String,
    pub shape: EyeballShape,
    pub iris: IrisLook,
    pub sclera: ScleraLook,
    pub cornea: CorneaLook,
}

fn rgb(hex: u32) -> [f32; 3] {
    [(hex >> 16) as f32 / 255.0, ((hex >> 8) & 255) as f32 / 255.0, (hex & 255) as f32 / 255.0]
}

const SCLERA: [f32; 3] = [0.93, 0.91, 0.88];

struct Row {
    id: &'static str,
    name: &'static str,
    family: &'static str,
    inner: u32,
    outer: u32,
    limbal: u32,
    collarette: f32,
    fibres: f32,
    crypts: f32,
    sector: Option<(u32, f32, f32)>,
    pupil: f64,
}

const ROWS: [Row; 25] = [
    Row { id: "brown-dark", name: "Dark brown", family: "brown", inner: 0x2a160b, outer: 0x3b2112, limbal: 0x120904, collarette: 0.35, fibres: 0.25, crypts: 0.2, sector: None, pupil: 0.3 },
    Row { id: "brown-medium", name: "Medium brown", family: "brown", inner: 0x4a2a12, outer: 0x5e3819, limbal: 0x1c0f06, collarette: 0.35, fibres: 0.35, crypts: 0.3, sector: None, pupil: 0.3 },
    Row { id: "brown-light", name: "Light brown", family: "brown", inner: 0x6b4220, outer: 0x8a5a2c, limbal: 0x2a170a, collarette: 0.38, fibres: 0.45, crypts: 0.35, sector: None, pupil: 0.3 },
    Row { id: "brown-chestnut", name: "Chestnut", family: "brown", inner: 0x5a2c14, outer: 0x7a3d1b, limbal: 0x220e05, collarette: 0.36, fibres: 0.4, crypts: 0.3, sector: None, pupil: 0.3 },
    Row { id: "brown-black", name: "Near black", family: "brown", inner: 0x150b06, outer: 0x1f120a, limbal: 0x070302, collarette: 0.33, fibres: 0.15, crypts: 0.1, sector: None, pupil: 0.3 },
    Row { id: "hazel-green", name: "Green hazel", family: "hazel", inner: 0x6a4a1e, outer: 0x5f6b33, limbal: 0x23260f, collarette: 0.42, fibres: 0.55, crypts: 0.4, sector: None, pupil: 0.3 },
    Row { id: "hazel-amber", name: "Amber hazel", family: "hazel", inner: 0x7a4f1a, outer: 0x6e6a32, limbal: 0x2a2310, collarette: 0.44, fibres: 0.5, crypts: 0.4, sector: None, pupil: 0.3 },
    Row { id: "hazel-light", name: "Light hazel", family: "hazel", inner: 0x8c6a2c, outer: 0x7d8248, limbal: 0x2f3016, collarette: 0.45, fibres: 0.55, crypts: 0.35, sector: None, pupil: 0.3 },
    Row { id: "amber", name: "Amber", family: "amber", inner: 0xa66a1c, outer: 0xb88327, limbal: 0x3d2408, collarette: 0.4, fibres: 0.45, crypts: 0.25, sector: None, pupil: 0.3 },
    Row { id: "amber-golden", name: "Golden", family: "amber", inner: 0xb88a2e, outer: 0xc9a043, limbal: 0x45300c, collarette: 0.4, fibres: 0.5, crypts: 0.25, sector: None, pupil: 0.3 },
    Row { id: "green-emerald", name: "Emerald", family: "green", inner: 0x4d5a22, outer: 0x2f7a46, limbal: 0x0f2615, collarette: 0.4, fibres: 0.6, crypts: 0.45, sector: None, pupil: 0.3 },
    Row { id: "green-olive", name: "Olive", family: "green", inner: 0x5c5a26, outer: 0x5c6e30, limbal: 0x1e2410, collarette: 0.4, fibres: 0.55, crypts: 0.4, sector: None, pupil: 0.3 },
    Row { id: "green-jade", name: "Jade", family: "green", inner: 0x5e6a3a, outer: 0x4e8a6a, limbal: 0x15291f, collarette: 0.42, fibres: 0.6, crypts: 0.4, sector: None, pupil: 0.3 },
    Row { id: "grey-steel", name: "Steel grey", family: "grey", inner: 0x6c6a5e, outer: 0x7f8a90, limbal: 0x262a2e, collarette: 0.4, fibres: 0.65, crypts: 0.35, sector: None, pupil: 0.3 },
    Row { id: "grey-silver", name: "Silver", family: "grey", inner: 0x8a8678, outer: 0xa3aab0, limbal: 0x33373b, collarette: 0.42, fibres: 0.7, crypts: 0.3, sector: None, pupil: 0.3 },
    Row { id: "grey-blue", name: "Grey blue", family: "grey", inner: 0x6f6e62, outer: 0x6f8496, limbal: 0x1f2a36, collarette: 0.4, fibres: 0.65, crypts: 0.35, sector: None, pupil: 0.3 },
    Row { id: "blue-deep", name: "Deep blue", family: "blue", inner: 0x4f5a5e, outer: 0x2d4f86, limbal: 0x0e1a33, collarette: 0.4, fibres: 0.7, crypts: 0.4, sector: None, pupil: 0.3 },
    Row { id: "blue-sky", name: "Sky blue", family: "blue", inner: 0x6c7a72, outer: 0x5a8ac0, limbal: 0x18304f, collarette: 0.42, fibres: 0.75, crypts: 0.35, sector: None, pupil: 0.3 },
    Row { id: "blue-ice", name: "Ice blue", family: "blue", inner: 0x8a948a, outer: 0x9cc0dc, limbal: 0x2c4660, collarette: 0.44, fibres: 0.8, crypts: 0.3, sector: None, pupil: 0.3 },
    Row { id: "blue-navy", name: "Navy", family: "blue", inner: 0x3c4446, outer: 0x1f3460, limbal: 0x080f20, collarette: 0.38, fibres: 0.6, crypts: 0.4, sector: None, pupil: 0.3 },
    Row { id: "central-heterochromia", name: "Central heterochromia", family: "special", inner: 0x9a5e1c, outer: 0x4f7a52, limbal: 0x162a1a, collarette: 0.55, fibres: 0.55, crypts: 0.35, sector: None, pupil: 0.3 },
    Row { id: "sectoral-heterochromia", name: "Sectoral heterochromia", family: "special", inner: 0x4f5a5e, outer: 0x2d4f86, limbal: 0x0e1a33, collarette: 0.4, fibres: 0.7, crypts: 0.4, sector: Some((0x6a4220, 200.0, 70.0)), pupil: 0.3 },
    Row { id: "pale", name: "Pale", family: "special", inner: 0xb0a8a0, outer: 0xc8c8d0, limbal: 0x5a5e66, collarette: 0.45, fibres: 0.8, crypts: 0.2, sector: None, pupil: 0.3 },
    Row { id: "brown-medium-dilated", name: "Medium brown, dilated", family: "brown", inner: 0x4a2a12, outer: 0x5e3819, limbal: 0x1c0f06, collarette: 0.35, fibres: 0.35, crypts: 0.3, sector: None, pupil: 0.55 },
    Row { id: "blue-sky-constricted", name: "Sky blue, constricted", family: "blue", inner: 0x6c7a72, outer: 0x5a8ac0, limbal: 0x18304f, collarette: 0.42, fibres: 0.75, crypts: 0.35, sector: None, pupil: 0.18 },
];

pub const DEFAULT_MODEL: &str = "brown-medium";

pub fn models() -> Vec<EyeModel> {
    ROWS.iter()
        .enumerate()
        .map(|(k, r)| EyeModel {
            id: r.id.into(),
            name: r.name.into(),
            family: r.family.into(),
            shape: EyeballShape { pupil: r.pupil, ..EyeballShape::default() },
            iris: IrisLook {
                inner: rgb(r.inner),
                outer: rgb(r.outer),
                limbal: rgb(r.limbal),
                limbal_width: 0.12,
                collarette: r.collarette,
                fibres: r.fibres,
                crypts: r.crypts,
                sector: r.sector.map(|(c, s, w)| Sector { colour: rgb(c), start_deg: s, width_deg: w }),
                seed: 7919 * (k as u32 + 1),
            },
            sclera: ScleraLook { tint: SCLERA, veins: 0.25 },
            cornea: CorneaLook { ior: 1.376, roughness: 0.03 },
        })
        .collect()
}

pub fn model(id: &str) -> Option<EyeModel> {
    models().into_iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_and_family_has_its_ui_text() {
        let en: serde_json::Value = serde_json::from_str(include_str!("../../../web/locales/en.json")).unwrap();
        for m in models() {
            for key in [format!("eyes.model.{}", m.id), format!("eyes.family.{}", m.family)] {
                assert!(en[&key].as_str().is_some_and(|s| !s.is_empty()), "web/locales/en.json has no {key}");
            }
        }
    }

    #[test]
    fn the_inventory_is_unique_valid_and_has_the_default() {
        let all = models();
        let mut ids: Vec<&str> = all.iter().map(|m| m.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), all.len(), "ids are unique");
        assert!(model(DEFAULT_MODEL).is_some());
        for m in &all {
            m.shape.layout().unwrap_or_else(|e| panic!("{}: {e}", m.id));
            assert!(m.iris.inner.iter().chain(&m.iris.outer).chain(&m.iris.limbal).all(|c| (0.0..=1.0).contains(c)));
        }
        assert!(all.iter().map(|m| m.family.as_str()).collect::<std::collections::BTreeSet<_>>().len() >= 6);
    }
}
