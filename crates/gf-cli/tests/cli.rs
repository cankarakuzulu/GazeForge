use std::path::{Path, PathBuf};
use std::process::Command;

use ndarray::{Array1, Array2, arr2};
use ndarray_npy::write_npy;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("gazeforge-cli-{name}-{}", std::process::id()));
    let dense = root.join("dense");
    let base = root.join("base");
    std::fs::create_dir_all(&dense).unwrap();
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(dense.join("manifest.json"), r#"{"vertices": 4, "faces": 2, "shapes": []}"#).unwrap();
    let positions = arr2(&[[0.0f32, 0.0, 0.0], [0.01, 0.0, 0.0], [0.01, 0.0, 0.01], [0.0, 0.0, 0.01]]);
    write_npy(dense.join("positions.npy"), &positions).unwrap();
    write_npy(dense.join("face_sizes.npy"), &Array1::from(vec![3u32, 3])).unwrap();
    write_npy(dense.join("corner_vertices.npy"), &Array1::from(vec![0u32, 1, 2, 0, 2, 3])).unwrap();
    let uvs = arr2(&[[0.0f32, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0], [1.0, 1.0], [0.0, 1.0]]);
    write_npy(dense.join("corner_uvs.npy"), &uvs).unwrap();
    write_npy(base.join("kept.npy"), &Array1::from(vec![0u32, 1, 2, 3])).unwrap();
    write_npy(base.join("owner.npy"), &Array1::from(vec![0u32, 1, 2, 3])).unwrap();
    write_npy(base.join("triangles.npy"), &arr2(&[[0u32, 1, 2], [0, 2, 3]])).unwrap();
    let tuv = Array2::from_shape_vec((2, 6), vec![0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0]).unwrap();
    write_npy(base.join("triangle_uvs.npy"), &tuv).unwrap();
    write_npy(root.join("good.npy"), &positions).unwrap();
    let mut nan = positions.clone();
    nan[[2, 1]] = f32::NAN;
    write_npy(root.join("nan.npy"), &nan).unwrap();
    let mut inf = positions.clone();
    inf[[3, 0]] = f32::INFINITY;
    write_npy(root.join("inf.npy"), &inf).unwrap();
    root
}

fn gazeforge(args: &[&str]) -> (bool, Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_gazeforge")).args(args).output().unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "expected one JSON line, got {stdout}");
    (output.status.success(), serde_json::from_str(stdout.trim()).unwrap())
}

fn export(root: &Path, positions: &str, extra: &[&str]) -> (bool, Value) {
    let dense = root.join("dense");
    let base = root.join("base");
    let positions = root.join(positions);
    let out = root.join("out.obj");
    let mut args = vec![
        "export-obj",
        "--dense",
        dense.to_str().unwrap(),
        "--base",
        base.to_str().unwrap(),
        "--positions",
        positions.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    gazeforge(&args)
}

fn refused(result: (bool, Value), needle: &str) {
    let (success, value) = result;
    assert!(!success, "exit code was success: {value}");
    assert_eq!(value["ok"], Value::Bool(false));
    let error = value["error"].as_str().unwrap();
    assert!(error.contains(needle), "error {error:?} lacks {needle:?}");
}

#[test]
fn the_fixture_exports_when_nothing_is_wrong() {
    let root = fixture("good");
    let (success, value) = export(&root, "good.npy", &["--placement", "subset", "--tip-vertex", "0"]);
    assert!(success, "{value}");
    assert_eq!(value["ok"], Value::Bool(true));
}

#[test]
fn nan_positions_are_refused_with_a_json_error() {
    let root = fixture("nan");
    refused(export(&root, "nan.npy", &["--placement", "subset", "--tip-vertex", "0"]), "NaN or infinite");
    refused(export(&root, "nan.npy", &["--placement", "f2", "--tip-vertex", "0"]), "first: vertex 2");
}

#[test]
fn infinite_positions_are_refused_with_a_json_error() {
    let root = fixture("inf");
    refused(export(&root, "inf.npy", &["--placement", "fit", "--tip-vertex", "0"]), "first: vertex 3");
}

#[test]
fn unknown_flags_are_refused_with_a_json_error() {
    let root = fixture("unknown");
    refused(export(&root, "good.npy", &["--placement", "fit", "--f2-initial-guard", "0"]), "--f2-initial-guard");
    refused(gazeforge(&["build-base", "--dense", "x", "--out", "y", "--shape", "all"]), "--shape");
}

#[test]
fn negative_magnitudes_are_refused_with_a_json_error() {
    let root = fixture("negative");
    refused(export(&root, "good.npy", &["--placement", "fit", "--max-move", "-3", "--tip-vertex", "0"]), "must not be negative");
    refused(gazeforge(&["build-base", "--dense", "x", "--out", "y", "--target", "-5"]), "must not be negative");
}

#[test]
fn non_finite_option_values_are_refused_with_a_json_error() {
    let root = fixture("nanopt");
    refused(export(&root, "good.npy", &["--placement", "optimal", "--max-move", "NaN", "--tip-vertex", "0"]), "must be finite");
}

#[test]
fn the_versioned_placement_exports_the_fixture() {
    let root = fixture("f3");
    let (success, value) = export(&root, "good.npy", &["--placement", "f3", "--tip-vertex", "0"]);
    assert!(success, "{value}");
    assert_eq!(value["placement"], Value::String("f3".into()));
    assert_eq!(value["placement_stats"]["uv_flipped_after"], Value::from(0));
}

#[test]
fn the_versioned_placement_refuses_options_it_does_not_read() {
    let root = fixture("f3opts");
    refused(export(&root, "good.npy", &["--placement", "f3", "--max-move", "2", "--tip-vertex", "0"]), "--max-move");
    refused(export(&root, "good.npy", &["--placement", "f3", "--f3-backoff-turn", "55,25", "--tip-vertex", "0"]), "two increasing");
    refused(export(&root, "good.npy", &["--placement", "fit", "--eyes", "e.json", "--tip-vertex", "0"]), "--eyes");
    refused(export(&root, "good.npy", &["--placement", "f3", "--eyes", "missing.json", "--tip-vertex", "0"]), "missing.json");
}

#[test]
fn the_angle_pass_version_exports_and_refuses_what_it_does_not_read() {
    let root = fixture("f4");
    let (success, value) = export(&root, "good.npy", &["--placement", "f4", "--tip-vertex", "0"]);
    assert!(success, "{value}");
    assert_eq!(value["placement"], Value::String("f4".into()));
    assert!(value["placement_stats"]["f4"].is_object(), "{value}");
    assert!(value["settings"]["f4"].is_object(), "{value}");
    let (success, value) = export(&root, "good.npy", &["--placement", "f3", "--tip-vertex", "0"]);
    assert!(success, "{value}");
    assert!(value["settings"].get("f4").is_none() && value["placement_stats"].get("f4").is_none(), "{value}");
    refused(export(&root, "good.npy", &["--placement", "f3", "--f4-target", "10", "--tip-vertex", "0"]), "--f4-target");
    refused(export(&root, "good.npy", &["--placement", "f4", "--f4-backoff", "both", "--tip-vertex", "0"]), "--f4-backoff");
    refused(export(&root, "good.npy", &["--placement", "f4", "--f4-plane", "eye", "--tip-vertex", "0"]), "--f4-plane");
    refused(export(&root, "good.npy", &["--placement", "f4", "--f4-passes", "1.5", "--tip-vertex", "0"]), "f4-passes");
}

#[test]
fn bad_layout_options_are_refused() {
    let root = fixture("layout");
    let dense = root.join("dense");
    let out = root.join("built");
    let (d, o) = (dense.to_str().unwrap(), out.to_str().unwrap());
    refused(gazeforge(&["build-base", "--dense", d, "--out", o, "--target", "3", "--lods", "3"]), "strictly decreasing");
    refused(gazeforge(&["build-base", "--dense", d, "--out", o, "--target", "3", "--lods", "2,2"]), "strictly decreasing");
    write_npy(root.join("short.npy"), &Array1::from(vec![1.0f32, 1.0])).unwrap();
    refused(gazeforge(&["build-base", "--dense", d, "--out", o, "--weights", root.join("short.npy").to_str().unwrap()]), "has 2 values");
    write_npy(root.join("zero.npy"), &Array1::from(vec![1.0f32, 0.0, 1.0, 1.0])).unwrap();
    refused(gazeforge(&["build-base", "--dense", d, "--out", o, "--weights", root.join("zero.npy").to_str().unwrap()]), "not finite and positive");
}

#[test]
fn the_lid_step_refuses_bad_input() {
    let root = fixture("lids");
    let dense = root.join("dense");
    let good = root.join("good.npy");
    let out = root.join("collided.npy");
    let (d, g, o) = (dense.to_str().unwrap(), good.to_str().unwrap(), out.to_str().unwrap());
    refused(gazeforge(&["collide-lids", "--dense", d, "--positions", g, "--out", o, "--eyes", "missing.json"]), "missing.json");
    refused(gazeforge(&["collide-lids", "--dense", d, "--positions", g, "--out", o, "--eyes", "e.json", "--front", "70"]), "--front");
    let eyes = root.join("eyes.json");
    std::fs::write(&eyes, r#"{"eyes": [{"centre": [0, 0, 0], "radius": -1, "gaze": [0, 0, 1], "vertices": [[0, 0, 1]], "triangles": [[0, 0, 0]]}]}"#).unwrap();
    refused(gazeforge(&["collide-lids", "--dense", d, "--positions", g, "--out", o, "--eyes", eyes.to_str().unwrap()]), "radius");
}

#[test]
fn a_tip_vertex_outside_the_dense_base_is_refused() {
    let root = fixture("tip");
    refused(export(&root, "good.npy", &["--placement", "subset"]), "nose tip vertex 185894");
}
