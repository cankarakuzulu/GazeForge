use serde::{Deserialize, Serialize};

pub const PARTS: [&str; 4] = ["sclera", "cornea", "iris", "pupil"];
const PUPIL_OVERLAP: f64 = 1.03;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct EyeballShape {
    pub radius_mm: f64,
    pub cornea_curvature: f64,
    pub limbus: f64,
    pub iris_depth: f64,
    pub pupil: f64,
    pub segments: usize,
    pub rings: usize,
}

impl Default for EyeballShape {
    fn default() -> EyeballShape {
        EyeballShape { radius_mm: 12.0, cornea_curvature: 0.65, limbus: 0.96, iris_depth: 0.24, pupil: 0.3, segments: 32, rings: 16 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub name: String,
    pub first_triangle: usize,
    pub triangles: usize,
}

#[derive(Clone, Debug, Default)]
pub struct EyeballMesh {
    pub positions: Vec<[f64; 3]>,
    pub normals: Vec<[f64; 3]>,
    pub uvs: Vec<[f64; 2]>,
    pub triangles: Vec<[u32; 3]>,
    pub parts: Vec<Part>,
}

pub struct Layout {
    pub radius: f64,
    pub limbus_radius: f64,
    pub limbus_z: f64,
    pub cornea_radius: f64,
    pub cornea_centre_z: f64,
    pub apex_z: f64,
    pub iris_z: f64,
    pub pupil_radius: f64,
}

impl EyeballShape {
    pub fn layout(&self) -> Result<Layout, String> {
        let finite = [self.radius_mm, self.cornea_curvature, self.limbus, self.iris_depth, self.pupil].iter().all(|x| x.is_finite());
        if !finite || self.radius_mm <= 0.0 {
            return Err("an eyeball needs finite parameters and a positive radius".into());
        }
        if !(0.2..=1.6).contains(&self.limbus) {
            return Err(format!("limbus {} (limbus diameter over the eyeball radius) is outside [0.2, 1.6]", self.limbus));
        }
        if !(0.05..=0.9).contains(&self.pupil) {
            return Err(format!("pupil {} (pupil radius over the limbus radius) is outside [0.05, 0.9]", self.pupil));
        }
        if self.segments < 8 || self.rings < 4 || self.segments > 512 || self.rings > 256 {
            return Err(format!("segments {} and rings {} must be in [8, 512] and [4, 256]", self.segments, self.rings));
        }
        let radius = self.radius_mm / 1000.0;
        let a = self.limbus * radius / 2.0;
        let rc = self.cornea_curvature * radius;
        if rc <= a {
            return Err(format!("cornea_curvature {} gives a cornea sphere no wider than the limbus", self.cornea_curvature));
        }
        let limbus_z = (radius * radius - a * a).sqrt();
        let cornea_centre_z = limbus_z - (rc * rc - a * a).sqrt();
        let apex_z = cornea_centre_z + rc;
        let iris_z = apex_z - self.iris_depth * radius;
        if !(iris_z > 0.0 && iris_z < limbus_z) {
            return Err(format!("iris_depth {} puts the iris outside the eye (it must sit behind the limbus plane, in front of the centre)", self.iris_depth));
        }
        Ok(Layout { radius, limbus_radius: a, limbus_z, cornea_radius: rc, cornea_centre_z, apex_z, iris_z, pupil_radius: self.pupil * a })
    }

    pub fn mesh(&self) -> Result<EyeballMesh, String> {
        let l = self.layout()?;
        let (n, rings) = (self.segments, self.rings);
        let tau = std::f64::consts::TAU;
        let mut m = EyeballMesh::default();
        let part = |m: &mut EyeballMesh, name: &str, f: &mut dyn FnMut(&mut EyeballMesh)| {
            let first = m.triangles.len();
            f(m);
            m.parts.push(Part { name: name.into(), first_triangle: first, triangles: m.triangles.len() - first });
        };
        let grid = |m: &mut EyeballMesh, rows: usize, point: &dyn Fn(f64, f64) -> ([f64; 3], [f64; 3], [f64; 2]), outward: bool| {
            let base = m.positions.len() as u32;
            for i in 0..=rows {
                for j in 0..=n {
                    let (p, nrm, uv) = point(i as f64 / rows as f64, j as f64 / n as f64);
                    m.positions.push(p);
                    m.normals.push(nrm);
                    m.uvs.push(uv);
                }
            }
            let at = |i: usize, j: usize| base + (i * (n + 1) + j) as u32;
            for i in 0..rows {
                for j in 0..n {
                    let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
                    if outward {
                        m.triangles.push([a, b, c]);
                        m.triangles.push([a, c, d]);
                    } else {
                        m.triangles.push([a, c, b]);
                        m.triangles.push([a, d, c]);
                    }
                }
            }
        };
        let limbus_theta = (l.limbus_radius / l.radius).asin();
        part(&mut m, PARTS[0], &mut |m| {
            grid(m, rings, &|s, t| {
                let theta = limbus_theta + s * (std::f64::consts::PI - limbus_theta);
                let phi = t * tau;
                let d = [theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos()];
                (d.map(|x| x * l.radius), d, [t, theta / std::f64::consts::PI])
            }, true)
        });
        let cornea_phi = (l.limbus_radius / l.cornea_radius).asin();
        part(&mut m, PARTS[1], &mut |m| {
            grid(m, (rings / 2).max(4), &|s, t| {
                let theta = s * cornea_phi;
                let phi = t * tau;
                let d = [theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos()];
                ([d[0] * l.cornea_radius, d[1] * l.cornea_radius, l.cornea_centre_z + d[2] * l.cornea_radius], d, [0.5 + 0.5 * d[0] / cornea_phi.sin(), 0.5 + 0.5 * d[1] / cornea_phi.sin()])
            }, true);
            let base = m.positions.len() as u32;
            m.positions.push([0.0, 0.0, l.limbus_z]);
            m.normals.push([0.0, 0.0, -1.0]);
            m.uvs.push([0.5, 0.5]);
            for j in 0..n {
                let phi = j as f64 / n as f64 * tau;
                m.positions.push([l.limbus_radius * phi.cos(), l.limbus_radius * phi.sin(), l.limbus_z]);
                m.normals.push([0.0, 0.0, -1.0]);
                m.uvs.push([0.5 + 0.5 * phi.cos(), 0.5 + 0.5 * phi.sin()]);
            }
            for j in 0..n {
                let (a, b) = (base + 1 + j as u32, base + 1 + ((j + 1) % n) as u32);
                m.triangles.push([base, b, a]);
            }
        });
        part(&mut m, PARTS[2], &mut |m| {
            grid(m, (rings / 4).max(2), &|s, t| {
                let r = l.pupil_radius + s * (l.limbus_radius - l.pupil_radius);
                let phi = t * tau;
                ([r * phi.cos(), r * phi.sin(), l.iris_z], [0.0, 0.0, 1.0], [t, s])
            }, true)
        });
        part(&mut m, PARTS[3], &mut |m| {
            let base = m.positions.len() as u32;
            let z = l.iris_z - 0.02 * l.pupil_radius;
            m.positions.push([0.0, 0.0, z]);
            m.normals.push([0.0, 0.0, 1.0]);
            m.uvs.push([0.5, 0.5]);
            for j in 0..n {
                let phi = j as f64 / n as f64 * tau;
                m.positions.push([PUPIL_OVERLAP * l.pupil_radius * phi.cos(), PUPIL_OVERLAP * l.pupil_radius * phi.sin(), z]);
                m.normals.push([0.0, 0.0, 1.0]);
                m.uvs.push([0.5 + 0.5 * phi.cos(), 0.5 + 0.5 * phi.sin()]);
            }
            for j in 0..n {
                let (a, b) = (base + 1 + j as u32, base + 1 + ((j + 1) % n) as u32);
                m.triangles.push([base, a, b]);
            }
        });
        Ok(m)
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (l.is_finite() && l > 1e-9).then(|| v.map(|x| x / l))
}

impl EyeballMesh {
    pub fn placed(&self, centre: [f64; 3], gaze: [f64; 3]) -> Result<EyeballMesh, String> {
        let gaze = unit(gaze).ok_or("gaze must be a finite, non-zero direction")?;
        let d = gaze[2];
        let up = unit([-d * gaze[0], -d * gaze[1], 1.0 - d * gaze[2]]).ok_or("gaze must not point straight up or down")?;
        let side = cross(up, gaze);
        let rotate = |p: [f64; 3]| [0, 1, 2].map(|k| side[k] * p[0] + up[k] * p[1] + gaze[k] * p[2]);
        Ok(EyeballMesh {
            positions: self.positions.iter().map(|&p| {
                let r = rotate(p);
                [0, 1, 2].map(|k| centre[k] + r[k])
            }).collect(),
            normals: self.normals.iter().map(|&n| rotate(n)).collect(),
            uvs: self.uvs.clone(),
            triangles: self.triangles.clone(),
            parts: self.parts.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn len(v: [f64; 3]) -> f64 {
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    }

    #[test]
    fn the_default_eye_has_human_proportions() {
        let l = EyeballShape::default().layout().unwrap();
        assert!((l.radius - 0.012).abs() < 1e-15);
        assert!((l.limbus_radius * 2.0 * 1000.0 - 11.52).abs() < 1e-9, "limbus diameter 11.52 mm");
        let bulge = (l.apex_z - l.radius) * 1000.0;
        assert!(bulge > 0.5 && bulge < 2.0, "the cornea stands {bulge} mm proud of the sclera");
        assert!(l.iris_z < l.limbus_z && l.iris_z > 0.009, "iris {} m behind the limbus plane {}", l.iris_z, l.limbus_z);
    }

    #[test]
    fn the_mesh_is_watertight_where_parts_meet_and_faces_outward() {
        let s = EyeballShape::default();
        let m = s.mesh().unwrap();
        let l = s.layout().unwrap();
        assert_eq!(m.parts.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), PARTS);
        assert_eq!(m.parts.iter().map(|p| p.triangles).sum::<usize>(), m.triangles.len());
        let sclera_first = m.positions.iter().take(s.segments + 1).collect::<Vec<_>>();
        for p in &sclera_first {
            assert!((p[2] - l.limbus_z).abs() < 1e-12 && ((p[0] * p[0] + p[1] * p[1]).sqrt() - l.limbus_radius).abs() < 1e-12);
        }
        let mut chamber = 0;
        for (k, part) in m.parts.iter().enumerate() {
            for t in &m.triangles[part.first_triangle..part.first_triangle + part.triangles] {
                let [a, b, c] = t.map(|i| m.positions[i as usize]);
                let n = [(b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]), (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]), (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])];
                if len(n) < 1e-20 {
                    continue;
                }
                let mid = [0, 1, 2].map(|i| (a[i] + b[i] + c[i]) / 3.0);
                let flat_at_limbus = (mid[2] - l.limbus_z).abs() < 1e-12;
                let want = match (k, flat_at_limbus) {
                    (0, _) => mid,
                    (1, true) => {
                        chamber += 1;
                        [0.0, 0.0, -1.0]
                    }
                    (1, false) => [mid[0], mid[1], mid[2] - l.cornea_centre_z],
                    _ => [0.0, 0.0, 1.0],
                };
                assert!(n[0] * want[0] + n[1] * want[1] + n[2] * want[2] > 0.0, "a {} triangle faces the wrong way", PARTS[k]);
            }
        }
        assert!(chamber > 0, "the cornea closes the anterior chamber with a disc at the limbus plane");
        let rim = |z: f64| m.positions.iter().filter(|p| (p[2] - z).abs() < 1e-12 && ((p[0] * p[0] + p[1] * p[1]).sqrt() - l.limbus_radius).abs() < 1e-12).count();
        assert!(rim(l.limbus_z) >= 3 * s.segments, "sclera rim, cornea rim and chamber rim meet at the limbus");
        let apex = m.positions.iter().map(|p| p[2]).fold(f64::MIN, f64::max);
        assert!((apex - l.apex_z).abs() < 1e-12);
    }

    #[test]
    fn a_placed_eye_looks_along_its_gaze_from_its_centre() {
        let s = EyeballShape::default();
        let l = s.layout().unwrap();
        let m = s.mesh().unwrap().placed([0.03, -0.1, 1.5], [0.0, -1.0, 0.0]).unwrap();
        let apex = m.positions.iter().copied().fold([0.0, f64::MAX, 0.0], |a, p| if p[1] < a[1] { p } else { a });
        assert!((apex[0] - 0.03).abs() < 1e-12 && (apex[1] - (-0.1 - l.apex_z)).abs() < 1e-12 && (apex[2] - 1.5).abs() < 1e-12, "{apex:?}");
        assert!(m.normals.iter().all(|n| (len(*n) - 1.0).abs() < 1e-9));
    }

    #[test]
    fn impossible_shapes_are_refused() {
        assert!(EyeballShape { cornea_curvature: 0.3, ..Default::default() }.layout().is_err());
        assert!(EyeballShape { iris_depth: 2.0, ..Default::default() }.layout().is_err());
        assert!(EyeballShape { radius_mm: -1.0, ..Default::default() }.layout().is_err());
        assert!(EyeballShape { pupil: 0.95, ..Default::default() }.layout().is_err());
    }
}
