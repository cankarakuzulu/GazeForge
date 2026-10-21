pub const PROFILE: &[(&str, Option<&str>)] = &[
    ("Root", None),
    ("Hips", Some("Root")),
    ("Spine", Some("Hips")),
    ("Chest", Some("Spine")),
    ("UpperChest", Some("Chest")),
    ("Neck", Some("UpperChest")),
    ("Head", Some("Neck")),
    ("LeftEye", Some("Head")),
    ("RightEye", Some("Head")),
    ("Jaw", Some("Head")),
    ("LeftShoulder", Some("UpperChest")),
    ("LeftUpperArm", Some("LeftShoulder")),
    ("LeftLowerArm", Some("LeftUpperArm")),
    ("LeftHand", Some("LeftLowerArm")),
    ("LeftThumbMetacarpal", Some("LeftHand")),
    ("LeftThumbProximal", Some("LeftThumbMetacarpal")),
    ("LeftThumbDistal", Some("LeftThumbProximal")),
    ("LeftIndexProximal", Some("LeftHand")),
    ("LeftIndexIntermediate", Some("LeftIndexProximal")),
    ("LeftIndexDistal", Some("LeftIndexIntermediate")),
    ("LeftMiddleProximal", Some("LeftHand")),
    ("LeftMiddleIntermediate", Some("LeftMiddleProximal")),
    ("LeftMiddleDistal", Some("LeftMiddleIntermediate")),
    ("LeftRingProximal", Some("LeftHand")),
    ("LeftRingIntermediate", Some("LeftRingProximal")),
    ("LeftRingDistal", Some("LeftRingIntermediate")),
    ("LeftLittleProximal", Some("LeftHand")),
    ("LeftLittleIntermediate", Some("LeftLittleProximal")),
    ("LeftLittleDistal", Some("LeftLittleIntermediate")),
    ("RightShoulder", Some("UpperChest")),
    ("RightUpperArm", Some("RightShoulder")),
    ("RightLowerArm", Some("RightUpperArm")),
    ("RightHand", Some("RightLowerArm")),
    ("RightThumbMetacarpal", Some("RightHand")),
    ("RightThumbProximal", Some("RightThumbMetacarpal")),
    ("RightThumbDistal", Some("RightThumbProximal")),
    ("RightIndexProximal", Some("RightHand")),
    ("RightIndexIntermediate", Some("RightIndexProximal")),
    ("RightIndexDistal", Some("RightIndexIntermediate")),
    ("RightMiddleProximal", Some("RightHand")),
    ("RightMiddleIntermediate", Some("RightMiddleProximal")),
    ("RightMiddleDistal", Some("RightMiddleIntermediate")),
    ("RightRingProximal", Some("RightHand")),
    ("RightRingIntermediate", Some("RightRingProximal")),
    ("RightRingDistal", Some("RightRingIntermediate")),
    ("RightLittleProximal", Some("RightHand")),
    ("RightLittleIntermediate", Some("RightLittleProximal")),
    ("RightLittleDistal", Some("RightLittleIntermediate")),
    ("LeftUpperLeg", Some("Hips")),
    ("LeftLowerLeg", Some("LeftUpperLeg")),
    ("LeftFoot", Some("LeftLowerLeg")),
    ("LeftToes", Some("LeftFoot")),
    ("RightUpperLeg", Some("Hips")),
    ("RightLowerLeg", Some("RightUpperLeg")),
    ("RightFoot", Some("RightLowerLeg")),
    ("RightToes", Some("RightFoot")),
];

pub const REQUIRED: &[&str] = &[
    "Hips", "Spine", "Head", "LeftShoulder", "LeftUpperArm", "LeftLowerArm", "LeftHand", "RightShoulder", "RightUpperArm", "RightLowerArm", "RightHand",
    "LeftUpperLeg", "LeftLowerLeg", "LeftFoot", "RightUpperLeg", "RightLowerLeg", "RightFoot",
];

const CENTRE: &[&str] = &["Root", "Hips", "Spine", "Chest", "UpperChest", "Neck", "Head", "Jaw"];

fn sided(rows: &[(&'static str, &str)], left: &str, right: &str) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    for &(profile, bone) in rows {
        if CENTRE.contains(&profile) {
            out.push((profile, bone.to_string()));
            continue;
        }
        let tail = profile.strip_prefix("Left").unwrap_or(profile);
        for (side, token) in [("Left", left), ("Right", right)] {
            let name = PROFILE.iter().map(|p| p.0).find(|p| p.strip_prefix(side) == Some(tail)).unwrap_or(profile);
            out.push((name, bone.replace("{s}", token)));
        }
    }
    out
}

type Table = (Vec<(&'static str, &'static str)>, Option<[String; 15]>, &'static str, &'static str);

const FINGERS: [(&str, &str); 5] = [("Thumb", "thumb"), ("Index", "index"), ("Middle", "middle"), ("Ring", "ring"), ("Little", "pinky")];

pub fn godot(preset: &str) -> Option<Vec<(&'static str, String)>> {
    let finger_rows = |names: &[&'static str; 15], bones: &[String; 15]| -> Vec<(&'static str, String)> { names.iter().copied().zip(bones.iter().cloned()).collect() };
    let finger_names: [&'static str; 15] = [
        "LeftThumbMetacarpal", "LeftThumbProximal", "LeftThumbDistal", "LeftIndexProximal", "LeftIndexIntermediate", "LeftIndexDistal",
        "LeftMiddleProximal", "LeftMiddleIntermediate", "LeftMiddleDistal", "LeftRingProximal", "LeftRingIntermediate", "LeftRingDistal",
        "LeftLittleProximal", "LeftLittleIntermediate", "LeftLittleDistal",
    ];
    let digits = |f: &dyn Fn(usize, usize) -> String| -> [String; 15] { std::array::from_fn(|i| f(i / 3, i % 3)) };
    let (base, fingers, left, right): Table = match preset {
        "game_engine" | "game_engine_with_breast" => (
            vec![("Root", "Root"), ("Hips", "pelvis"), ("Spine", "spine_01"), ("Chest", "spine_02"), ("UpperChest", "spine_03"), ("Neck", "neck_01"), ("Head", "head"),
                 ("LeftShoulder", "clavicle_{s}"), ("LeftUpperArm", "upperarm_{s}"), ("LeftLowerArm", "lowerarm_{s}"), ("LeftHand", "hand_{s}"),
                 ("LeftUpperLeg", "thigh_{s}"), ("LeftLowerLeg", "calf_{s}"), ("LeftFoot", "foot_{s}"), ("LeftToes", "ball_{s}")],
            Some(digits(&|f, j| format!("{}_0{}_{{s}}", FINGERS[f].1, j + 1))),
            "l", "r",
        ),
        "mixamo" | "mixamo_unity" => {
            let mut rows = vec![("Hips", "mixamorig:Hips"), ("Spine", "mixamorig:Spine"), ("Chest", "mixamorig:Spine1"), ("UpperChest", "mixamorig:Spine2"), ("Neck", "mixamorig:Neck"),
                                ("Head", "mixamorig:Head"), ("LeftShoulder", "mixamorig:{s}Shoulder"), ("LeftUpperArm", "mixamorig:{s}Arm"), ("LeftLowerArm", "mixamorig:{s}ForeArm"),
                                ("LeftHand", "mixamorig:{s}Hand"), ("LeftUpperLeg", "mixamorig:{s}UpLeg"), ("LeftLowerLeg", "mixamorig:{s}Leg"), ("LeftFoot", "mixamorig:{s}Foot"),
                                ("LeftToes", "mixamorig:{s}ToeBase")];
            if preset == "mixamo_unity" {
                rows.extend([("Root", "mixamorig:Root"), ("LeftEye", "mixamorig:{s}Eye"), ("Jaw", "mixamorig:Jaw")]);
            }
            (rows, Some(digits(&|f, j| format!("mixamorig:{{s}}Hand{}{}", if f == 4 { "Pinky" } else { FINGERS[f].0 }, j + 1))), "Left", "Right")
        }
        "default" => (
            vec![("Hips", "root"), ("Spine", "spine05"), ("Chest", "spine03"), ("UpperChest", "spine01"), ("Neck", "neck01"), ("Head", "head"), ("Jaw", "jaw"),
                 ("LeftEye", "eye.{s}"), ("LeftShoulder", "clavicle.{s}"), ("LeftUpperArm", "upperarm01.{s}"), ("LeftLowerArm", "lowerarm01.{s}"), ("LeftHand", "wrist.{s}"),
                 ("LeftUpperLeg", "upperleg01.{s}"), ("LeftLowerLeg", "lowerleg01.{s}"), ("LeftFoot", "foot.{s}"), ("LeftToes", "toe1-1.{s}")],
            Some(digits(&|f, j| format!("finger{}-{}.{{s}}", f + 1, j + 1))),
            "L", "R",
        ),
        "cmu_mb" => (
            vec![("Hips", "Hips"), ("Spine", "LowerBack"), ("Chest", "Spine"), ("UpperChest", "Spine1"), ("Neck", "Neck"), ("Head", "Head"),
                 ("LeftShoulder", "{s}Shoulder"), ("LeftUpperArm", "{s}Arm"), ("LeftLowerArm", "{s}ForeArm"), ("LeftHand", "{s}Hand"),
                 ("LeftThumbProximal", "{t}Thumb"), ("LeftIndexProximal", "{s}HandFinger1"),
                 ("LeftUpperLeg", "{s}UpLeg"), ("LeftLowerLeg", "{s}Leg"), ("LeftFoot", "{s}Foot"), ("LeftToes", "{s}ToeBase")],
            None,
            "Left", "Right",
        ),
        _ => return None,
    };
    let mut rows: Vec<(&'static str, String)> = Vec::new();
    for (profile, bone) in sided(&base, left, right) {
        let fixed = if bone.contains("{t}") { bone.replace("{t}", if profile.starts_with("Left") { "L" } else { "R" }) } else { bone };
        rows.push((profile, fixed));
    }
    if let Some(f) = fingers {
        let names = finger_names;
        for (profile, bone) in finger_rows(&names, &f) {
            for (side, token) in [("Left", left), ("Right", right)] {
                let tail = &profile["Left".len()..];
                let name = PROFILE.iter().map(|p| p.0).find(|p| p.strip_prefix(side) == Some(tail)).unwrap_or(profile);
                rows.push((name, bone.replace("{s}", token)));
            }
        }
    }
    Some(rows)
}

pub fn unity(preset: &str) -> Option<Vec<(String, String)>> {
    let thumb = |p: &str| -> String {
        for side in ["Left", "Right"] {
            for (g, u) in [("ThumbMetacarpal", "ThumbProximal"), ("ThumbProximal", "ThumbIntermediate"), ("ThumbDistal", "ThumbDistal")] {
                if p == format!("{side}{g}") {
                    return format!("{side}{u}");
                }
            }
        }
        p.to_string()
    };
    godot(preset).map(|rows| rows.into_iter().filter(|(p, _)| *p != "Root").map(|(p, b)| (thumb(p), b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body_rig::RigDef;
    use crate::rig_presets;

    fn side_sign(rig: &RigDef, bone: &str) -> f64 {
        rig.bones.iter().find(|b| b.name == bone).map(|b| b.default_head[0].signum()).unwrap_or(0.0)
    }

    #[test]
    fn every_full_preset_maps_the_required_bones_as_a_hierarchy_on_the_right_sides() {
        let (ue, _) = rig_presets::preset("game_engine").unwrap();
        let ue = RigDef::parse(&ue).unwrap();
        let left = side_sign(&ue, "hand_l");
        assert!(left != 0.0);
        for name in rig_presets::names() {
            let Some(map) = godot(name) else {
                assert_eq!(name, "openpose", "only the joint set has no humanoid map");
                continue;
            };
            let (rig, _) = rig_presets::preset(name).unwrap();
            let rig = RigDef::parse(&rig).unwrap();
            let parent = |b: &str| rig.bones.iter().find(|x| x.name == b).and_then(|x| x.parent.clone());
            let bone_of = |p: &str| map.iter().find(|r| r.0 == p).map(|r| r.1.clone());
            for req in REQUIRED {
                assert!(bone_of(req).is_some(), "{name}: {req} is not mapped");
            }
            let mut seen = std::collections::HashSet::new();
            for (profile, bone) in &map {
                assert!(PROFILE.iter().any(|p| p.0 == *profile), "{name}: {profile} is not a profile bone");
                assert!(rig.bones.iter().any(|b| &b.name == bone), "{name}: {profile} -> {bone}, not a bone of the rig");
                assert!(seen.insert(bone.clone()), "{name}: {bone} mapped twice");
                let mut up = PROFILE.iter().find(|p| p.0 == *profile).and_then(|p| p.1);
                while let Some(u) = up {
                    if let Some(anc) = bone_of(u) {
                        let mut b = parent(bone);
                        let mut found = false;
                        while let Some(x) = b {
                            if x == anc {
                                found = true;
                                break;
                            }
                            b = parent(&x);
                        }
                        assert!(found, "{name}: {profile} ({bone}) is not under {u} ({anc})");
                        break;
                    }
                    up = PROFILE.iter().find(|p| p.0 == u).and_then(|p| p.1);
                }
                if profile.starts_with("Left") || profile.starts_with("Right") {
                    let want = if profile.starts_with("Left") { left } else { -left };
                    assert_eq!(side_sign(&rig, bone), want, "{name}: {profile} ({bone}) is on the other side");
                }
            }
        }
    }

    #[test]
    fn unity_names_shift_the_thumb_and_drop_the_root() {
        let u = unity("game_engine").unwrap();
        assert!(u.iter().any(|(p, b)| p == "LeftThumbProximal" && b == "thumb_01_l"));
        assert!(u.iter().any(|(p, b)| p == "LeftThumbIntermediate" && b == "thumb_02_l"));
        assert!(u.iter().all(|(p, _)| p != "Root" && !p.contains("Metacarpal")));
        assert!(unity("openpose").is_none());
    }
}
