use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use gf_geom::lids::Eye;

use crate::fit::{self, Point, Similarity, add, dot, norm, scale, sub};
use crate::head::HeadOnly;

pub const FORMAT: &str = "gazeforge.eyerig/1";
pub const FOLLOW_HELPER: &str = "makehuman-helper/1";
pub const FOLLOW_NONE: &str = "none";
pub const OFFSET_MM_RANGE: f64 = 15.0;
pub const ROTATION_DEG_RANGE: f64 = 45.0;
pub const SCALE_RANGE: [f64; 2] = [0.5, 1.5];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sides<T> {
    pub left: T,
    pub right: T,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EyeRigManifest {
    pub format: String,
    pub catalog: String,
    pub source: String,
    pub axes: String,
    pub helpers: Sides<Vec<Point>>,
    pub targets: BTreeMap<String, Sides<Option<Vec<Point>>>>,
    pub missing: Vec<String>,
}

fn one() -> f64 {
    1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EyeAdjust {
    #[serde(default)]
    pub offset_mm: [f64; 3],
    #[serde(default)]
    pub rotation_deg: [f64; 3],
    #[serde(default = "one")]
    pub scale: f64,
}

impl Default for EyeAdjust {
    fn default() -> EyeAdjust {
        EyeAdjust { offset_mm: [0.0; 3], rotation_deg: [0.0; 3], scale: 1.0 }
    }
}

fn round_to(x: f64, step: f64) -> f64 {
    let r = (x / step).round() * step;
    if r == 0.0 { 0.0 } else { r }
}

impl EyeAdjust {
    pub fn canonical(&self) -> EyeAdjust {
        EyeAdjust { offset_mm: self.offset_mm.map(|x| round_to(x, 1e-4)), rotation_deg: self.rotation_deg.map(|x| round_to(x, 1e-4)), scale: round_to(self.scale, 1e-6) }
    }

    pub fn is_identity(&self) -> bool {
        self.offset_mm == [0.0; 3] && self.rotation_deg == [0.0; 3] && self.scale == 1.0
    }

    fn problems(&self, side: &str) -> Vec<String> {
        let mut out = Vec::new();
        if self.offset_mm.iter().any(|x| !x.is_finite() || x.abs() > OFFSET_MM_RANGE) {
            out.push(format!("eyeballs.{side}.offset_mm {:?} must be finite and within +-{OFFSET_MM_RANGE} mm", self.offset_mm));
        }
        if self.rotation_deg.iter().any(|x| !x.is_finite() || x.abs() > ROTATION_DEG_RANGE) {
            out.push(format!("eyeballs.{side}.rotation_deg {:?} must be finite and within +-{ROTATION_DEG_RANGE} degrees", self.rotation_deg));
        }
        if !(self.scale.is_finite() && (SCALE_RANGE[0]..=SCALE_RANGE[1]).contains(&self.scale)) {
            out.push(format!("eyeballs.{side}.scale {} is outside [{}, {}]", self.scale, SCALE_RANGE[0], SCALE_RANGE[1]));
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EyeballSpec {
    pub follow: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rig: Option<String>,
    #[serde(default)]
    pub left: EyeAdjust,
    #[serde(default)]
    pub right: EyeAdjust,
}

impl EyeballSpec {
    pub fn canonical(&self) -> EyeballSpec {
        EyeballSpec { left: self.left.canonical(), right: self.right.canonical(), ..self.clone() }
    }

    pub fn follows(&self) -> bool {
        self.follow == FOLLOW_HELPER
    }

    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        match self.follow.as_str() {
            FOLLOW_HELPER => match &self.rig {
                None => out.push(format!("eyeballs.follow {FOLLOW_HELPER} needs eyeballs.rig, the eye rig of the catalog")),
                Some(r) => {
                    if let Err(e) = crate::hash::parse_reference(r) {
                        out.push(format!("eyeballs.rig: {e}"));
                    }
                }
            },
            FOLLOW_NONE => {}
            other => out.push(format!("eyeballs.follow {other:?} is not one of {FOLLOW_HELPER}, {FOLLOW_NONE}")),
        }
        out.extend(self.left.problems("left"));
        out.extend(self.right.problems("right"));
        out
    }
}

pub fn helper_sum(rig: &EyeRigManifest, terms: &[(&str, f64)]) -> Sides<Vec<Point>> {
    let n = |s: &Vec<Point>| vec![[0.0; 3]; s.len()];
    let mut out = Sides { left: n(&rig.helpers.left), right: n(&rig.helpers.right) };
    for &(id, w) in terms {
        let Some(t) = rig.targets.get(id) else { continue };
        for (acc, d) in [(&mut out.left, &t.left), (&mut out.right, &t.right)] {
            if let Some(d) = d {
                for (a, v) in acc.iter_mut().zip(d) {
                    *a = add(*a, scale(*v, w));
                }
            }
        }
    }
    out
}

pub fn centred_helper(helper: &[Point], eye_centre: Point) -> Vec<Point> {
    let mut mean = [0.0; 3];
    for p in helper {
        mean = add(mean, *p);
    }
    let shift = sub(eye_centre, scale(mean, 1.0 / helper.len().max(1) as f64));
    helper.iter().map(|p| add(*p, shift)).collect()
}

pub struct EyeFrame {
    pub centre: Point,
    pub out: Point,
    pub up: Point,
    pub forward: Point,
}

fn unit(v: Point) -> Point {
    scale(v, 1.0 / norm(v).max(1e-300))
}

fn cross(a: Point, b: Point) -> Point {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub fn eye_frame(centre: Point, gaze: Point) -> EyeFrame {
    let forward = unit(gaze);
    let world_up = [0.0, 0.0, 1.0];
    let up = unit(sub(world_up, scale(forward, dot(world_up, forward))));
    let mut out = cross(up, forward);
    if out[0] * centre[0] < 0.0 {
        out = scale(out, -1.0);
    }
    EyeFrame { centre, out, up, forward }
}

fn axis_angle(axis: Point, deg: f64) -> [[f64; 3]; 3] {
    let a = unit(axis);
    let (s, c) = deg.to_radians().sin_cos();
    let t = 1.0 - c;
    [
        [t * a[0] * a[0] + c, t * a[0] * a[1] - s * a[2], t * a[0] * a[2] + s * a[1]],
        [t * a[0] * a[1] + s * a[2], t * a[1] * a[1] + c, t * a[1] * a[2] - s * a[0]],
        [t * a[0] * a[2] - s * a[1], t * a[1] * a[2] + s * a[0], t * a[2] * a[2] + c],
    ]
}

fn rotation_only(r: [[f64; 3]; 3]) -> Similarity {
    Similarity { scale: 1.0, rotation: r, translation: [0.0; 3] }
}

pub fn adjustment(frame: &EyeFrame, adjust: &EyeAdjust) -> Similarity {
    if adjust.is_identity() {
        return Similarity::identity();
    }
    let [pitch, yaw, roll] = adjust.rotation_deg;
    let rotation = rotation_only(axis_angle(cross(frame.forward, frame.up), pitch))
        .then(&rotation_only(axis_angle(cross(frame.forward, frame.out), yaw)))
        .then(&rotation_only(axis_angle(frame.forward, roll)));
    let about = Similarity { scale: adjust.scale, rotation: rotation.rotation, translation: [0.0; 3] };
    let offset = [0, 1, 2].map(|k| (adjust.offset_mm[0] * frame.out[k] + adjust.offset_mm[1] * frame.up[k] + adjust.offset_mm[2] * frame.forward[k]) / 1000.0);
    let c = frame.centre;
    Similarity { translation: sub(add(c, offset), about.apply(c)), ..about }
}

pub fn eye_transform(helper_base: &[Point], helper_delta: &[Point], frame: &EyeFrame, adjust: &EyeAdjust) -> Result<Option<Similarity>, String> {
    let moved = helper_delta.iter().any(|d| *d != [0.0; 3]);
    if !moved && adjust.is_identity() {
        return Ok(None);
    }
    let follow = if moved {
        let target: Vec<Point> = helper_base.iter().zip(helper_delta).map(|(p, d)| add(*p, *d)).collect();
        fit::similarity(helper_base, &target, &vec![1.0; helper_base.len()], true)?
    } else {
        Similarity::identity()
    };
    Ok(Some(adjustment(frame, adjust).then(&follow)))
}

pub struct MacroTrace {
    pub changes: Vec<(String, f64)>,
    pub coef: [f64; 7],
    pub head_scale: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct PlacedEye {
    pub side: &'static str,
    pub centre: Point,
    pub radius: f64,
    pub moved: bool,
    pub scale: f64,
    pub rotation: [[f64; 3]; 3],
    pub translation: Point,
}

pub fn place_eyes(
    spec: &EyeballSpec,
    rig: Option<&EyeRigManifest>,
    weights: &BTreeMap<String, f64>,
    pin: Option<(&MacroTrace, &HeadOnly)>,
    eyes: &[Eye],
) -> Result<(Vec<Eye>, Vec<PlacedEye>), String> {
    let sides: Vec<usize> = eyes.iter().map(|e| usize::from(e.centre[0] < 0.0)).collect();
    if !eyes.is_empty() && (sides.len() != 2 || sides[0] == sides[1]) {
        return Err(format!("eyeballs need one eye on each side of the midline; the eyes file has {} eyes", eyes.len()));
    }
    let rig = if spec.follows() { Some(rig.ok_or("eyeballs follow the sockets but no eye rig was given")?) } else { None };
    let terms: Vec<(&str, f64)> = weights.iter().map(|(k, &w)| (k.as_str(), w)).collect();
    let changes: Vec<(&str, f64)> = pin.map(|(t, _)| t.changes.iter().map(|(k, w)| (k.as_str(), *w)).collect()).unwrap_or_default();
    let mut placed = Vec::with_capacity(eyes.len());
    let mut report = Vec::with_capacity(eyes.len());
    for (eye, &side) in eyes.iter().zip(&sides) {
        let adjust = if side == 0 { &spec.left } else { &spec.right };
        let frame = eye_frame(eye.centre, eye.gaze());
        let transform = match rig {
            Some(rig) => {
                let pick = |s: Sides<Vec<Point>>| if side == 0 { s.left } else { s.right };
                let base = centred_helper(if side == 0 { &rig.helpers.left } else { &rig.helpers.right }, eye.centre);
                let mut delta = pick(helper_sum(rig, &terms));
                if let Some((t, head)) = pin {
                    let pinned = head.pin_points(&base, &pick(helper_sum(rig, &changes)), &t.coef, t.head_scale);
                    for (d, p) in delta.iter_mut().zip(pinned) {
                        *d = add(*d, p);
                    }
                }
                eye_transform(&base, &delta, &frame, adjust)?
            }
            None => (!adjust.is_identity()).then(|| adjustment(&frame, adjust)),
        };
        let t = transform.clone().unwrap_or_else(Similarity::identity);
        let moved = transform.map_or_else(|| eye.clone(), |t| eye.transformed(t.scale, t.rotation, t.translation));
        report.push(PlacedEye { side: if side == 0 { "left" } else { "right" }, centre: moved.centre, radius: moved.radius, moved: t != Similarity::identity(),
                                scale: t.scale, rotation: t.rotation, translation: t.translation });
        placed.push(moved);
    }
    Ok((placed, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(c: Point) -> Vec<Point> {
        (0..12).map(|i| {
            let a = i as f64 * 0.5;
            add(c, [0.012 * a.cos(), 0.004 * (i % 3) as f64, 0.012 * a.sin()])
        })
        .collect()
    }

    #[test]
    fn no_motion_and_no_adjustment_leave_the_eye_alone() {
        let c = [0.03, -0.12, 1.49];
        let f = eye_frame(c, [0.0, -1.0, 0.0]);
        assert!(eye_transform(&ring(c), &vec![[0.0; 3]; 12], &f, &EyeAdjust::default()).unwrap().is_none());
    }

    #[test]
    fn a_helper_translation_moves_the_eye_by_the_same_amount() {
        let c = [0.03, -0.12, 1.49];
        let f = eye_frame(c, [0.0, -1.0, 0.0]);
        let t = eye_transform(&ring(c), &vec![[0.0, 0.0, 0.006]; 12], &f, &EyeAdjust::default()).unwrap().unwrap();
        let moved = t.apply(c);
        assert!(norm(sub(moved, add(c, [0.0, 0.0, 0.006]))) < 1e-12, "{moved:?}");
    }

    #[test]
    fn the_frame_points_out_from_the_midline_on_both_sides() {
        for (x, sign) in [(0.03, 1.0), (-0.03, -1.0)] {
            let f = eye_frame([x, -0.12, 1.49], [0.0, -1.0, 0.0]);
            assert!(f.out[0] * sign > 0.99, "{:?}", f.out);
            assert!(f.up[2] > 0.99 && f.forward[1] < -0.99);
        }
    }

    #[test]
    fn adjustments_are_relative_to_the_eye_and_mirror_between_sides() {
        let adj = EyeAdjust { offset_mm: [2.0, 1.0, -3.0], rotation_deg: [10.0, 15.0, 0.0], scale: 1.1 };
        let mut moved = Vec::new();
        for x in [0.03, -0.03] {
            let c = [x, -0.12, 1.49];
            let f = eye_frame(c, [0.0, -1.0, 0.0]);
            let t = adjustment(&f, &adj);
            let centre = t.apply(c);
            assert!(norm(sub(centre, add(c, [x.signum() * 0.002, 0.003, 0.001]))) < 1e-12, "{centre:?}");
            let tip = sub(t.apply(add(c, [0.0, -0.012, 0.0])), centre);
            assert!((norm(tip) - 0.0132).abs() < 1e-12);
            assert!(tip[2] > 0.0, "pitch +10 turns the gaze up: {tip:?}");
            assert!(tip[0] * x > 0.0, "yaw +15 turns the gaze outward: {tip:?}");
            moved.push(tip);
        }
        assert!((moved[0][0] + moved[1][0]).abs() < 1e-12 && (moved[0][1] - moved[1][1]).abs() < 1e-12 && (moved[0][2] - moved[1][2]).abs() < 1e-12);
    }

    #[test]
    fn adjustments_ride_on_the_followed_socket() {
        let c = [0.03, -0.12, 1.49];
        let f = eye_frame(c, [0.0, -1.0, 0.0]);
        let adj = EyeAdjust { offset_mm: [0.0, 2.0, 0.0], ..EyeAdjust::default() };
        let t = eye_transform(&ring(c), &vec![[0.0, 0.0, 0.006]; 12], &f, &adj).unwrap().unwrap();
        assert!(norm(sub(t.apply(c), add(c, [0.0, 0.0, 0.008]))) < 1e-12);
    }

    #[test]
    fn out_of_range_adjustments_and_a_missing_rig_are_refused() {
        let spec = EyeballSpec { follow: FOLLOW_HELPER.into(), rig: None, left: EyeAdjust { scale: 2.0, ..EyeAdjust::default() }, right: EyeAdjust::default() };
        let p = spec.problems();
        assert_eq!(p.len(), 2, "{p:?}");
        let ok = EyeballSpec { follow: FOLLOW_NONE.into(), rig: None, left: EyeAdjust::default(), right: EyeAdjust::default() };
        assert!(ok.problems().is_empty());
    }
}
