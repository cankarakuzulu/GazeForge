use std::path::{Path, PathBuf};
use std::process::Command;

use ndarray::{Array1, Array2, arr2};
use ndarray_npy::write_npy;
use serde_json::Value;

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("gazeforge-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
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
fn the_continuous_placement_exports_and_refuses_what_it_does_not_read() {
    let root = fixture("f5");
    let (success, value) = export(&root, "good.npy", &["--placement", "f5", "--tip-vertex", "0"]);
    assert!(success, "{value}");
    assert_eq!(value["placement_version"], Value::String("f5/1".into()));
    assert_eq!(value["settings"]["membrane"], serde_json::json!(0.1));
    assert!(value["placement_stats"]["footprint_points"].as_u64().unwrap() > 0, "{value}");
    let (success, value) = export(&root, "good.npy", &["--placement", "f5/1", "--f5-membrane", "0.2", "--tip-vertex", "0"]);
    assert!(success, "{value}");
    assert_eq!(value["settings"]["membrane"], serde_json::json!(0.2));
    refused(export(&root, "good.npy", &["--placement", "f5", "--f3-rounds", "2", "--tip-vertex", "0"]), "--f3-rounds");
    refused(export(&root, "good.npy", &["--placement", "f3", "--f5-membrane", "0.2", "--tip-vertex", "0"]), "--f5-membrane");
    refused(export(&root, "good.npy", &["--placement", "f5", "--f5-normals", "both", "--tip-vertex", "0"]), "--f5-normals");
    refused(export(&root, "good.npy", &["--placement", "f5", "--f5-eye-front", "5", "--tip-vertex", "0"]), "--f5-eye-front");
    refused(export(&root, "good.npy", &["--placement", "f5/2", "--tip-vertex", "0"]), "f5/2");
}

#[test]
fn the_sliver_placement_exports_and_refuses_what_it_does_not_read() {
    let root = fixture("f6");
    let (success, value) = export(&root, "good.npy", &["--placement", "f6", "--tip-vertex", "0"]);
    assert!(success, "{value}");
    assert_eq!(value["placement_version"], Value::String("f6/1".into()));
    assert_eq!(value["settings"]["membrane"], serde_json::json!(0.05));
    assert_eq!(value["settings"]["smooth"], serde_json::json!(0.05));
    assert_eq!(value["settings"]["sliver_deg"], serde_json::json!(6.0));
    assert!(value["placement_stats"]["slivers"]["below_after"].as_u64().is_some(), "{value}");
    let (success, value) = export(&root, "good.npy", &["--placement", "f6/1", "--f5-membrane", "0.2", "--f6-sliver-deg", "8", "--tip-vertex", "0"]);
    assert!(success, "{value}");
    assert_eq!(value["settings"]["membrane"], serde_json::json!(0.2));
    assert_eq!(value["settings"]["sliver_deg"], serde_json::json!(8.0));
    let (_, f5) = export(&root, "good.npy", &["--placement", "f5", "--tip-vertex", "0"]);
    assert!(f5["placement_stats"].get("slivers").is_none(), "{f5}");
    refused(export(&root, "good.npy", &["--placement", "f5", "--f6-sliver-deg", "6", "--tip-vertex", "0"]), "--f6-sliver-deg");
    refused(export(&root, "good.npy", &["--placement", "f6", "--f3-rounds", "2", "--tip-vertex", "0"]), "--f3-rounds");
    refused(export(&root, "good.npy", &["--placement", "f6", "--f6-sliver-relax", "0", "--tip-vertex", "0"]), "--f6-sliver-relax");
    refused(export(&root, "good.npy", &["--placement", "f6", "--f6-sliver-eye", "8,4", "--tip-vertex", "0"]), "--f6-sliver-eye");
    refused(export(&root, "good.npy", &["--placement", "f6/2", "--tip-vertex", "0"]), "f6/2");
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

#[test]
fn the_path_gate_fires_on_a_planted_jump_and_refuses_a_bad_plant() {
    let root = fixture("path");
    write_npy(root.join("delta.npy"), &Array2::<f32>::zeros((4, 3))).unwrap();
    let (dense, base, start, delta) = (root.join("dense"), root.join("base"), root.join("good.npy"), root.join("delta.npy"));
    let run = |extra: &[&str]| {
        let mut args = vec![
            "gates",
            "path",
            "--dense",
            dense.to_str().unwrap(),
            "--base",
            base.to_str().unwrap(),
            "--start",
            start.to_str().unwrap(),
            "--delta",
            delta.to_str().unwrap(),
            "--steps",
            "4",
            "--placement",
            "subset",
            "--pop-limit",
            "0",
        ];
        args.extend_from_slice(extra);
        gazeforge(&args)
    };
    let (success, value) = run(&[]);
    assert!(success, "{value}");
    let (success, value) = run(&["--plant", "2,1,1.0"]);
    assert!(!success, "{value}");
    assert_eq!(value["gates"]["path"]["pops"], 2);
    assert_eq!(value["plant"]["game_vertex"], 1);
    refused(run(&["--plant", "2,9,1.0"]), "--plant");
    refused(run(&["--plant", "2,1"]), "--plant");
}

fn store_fixture(name: &str) -> (PathBuf, String, String, String) {
    let root = fixture(name);
    let targets = root.join("targets");
    std::fs::create_dir_all(targets.join("nose")).unwrap();
    std::fs::write(
        targets.join("catalog.json"),
        r#"{"meta": {}, "not_on_head": {}, "targets": {"nose/a": {"name": "a", "category": "nose", "file": "nose/a.npz", "max_mm": 2.0}}}"#,
    )
    .unwrap();
    let mut npz = ndarray_npy::NpzWriter::new(std::fs::File::create(targets.join("nose/a.npz")).unwrap());
    npz.add_array("indices", &Array1::from(vec![1u32, 2])).unwrap();
    npz.add_array("deltas", &arr2(&[[0.002f32, 0.0, 0.0], [0.0, 0.0, -0.002]])).unwrap();
    npz.finish().unwrap();
    let face = arr2(&[[0.0001f32, 0.0, 0.0], [0.011, 0.0, 0.0], [0.01, 0.0002, 0.009], [0.0, 0.0, 0.01]]);
    write_npy(root.join("face.npy"), &face).unwrap();
    let s = root.join("store");
    let (ok, pack) = gazeforge(&["store", "import-pack", "--store", s.to_str().unwrap(), "--dense", root.join("dense").to_str().unwrap()]);
    assert!(ok, "{pack}");
    let pack = pack["pack"].as_str().unwrap().to_string();
    let (ok, cat) = gazeforge(&["store", "import-catalog", "--store", s.to_str().unwrap(), "--targets", targets.to_str().unwrap(), "--pack", &pack]);
    assert!(ok, "{cat}");
    let (ok, game) = gazeforge(&["store", "import-game", "--store", s.to_str().unwrap(), "--base", root.join("base").to_str().unwrap(), "--pack", &pack]);
    assert!(ok, "{game}");
    (root, pack, cat["catalog"].as_str().unwrap().to_string(), game["game_base"].as_str().unwrap().to_string())
}

#[test]
fn store_list_dates_items_and_lists_an_old_eye_rig_as_unusable_instead_of_failing() {
    let (root, pack, catalog, _) = store_fixture("storelist");
    let s = root.join("store");
    std::fs::create_dir_all(s.join("eyerigs")).unwrap();
    let old = gf_core::hash::canonical_bytes(&serde_json::json!({"format": "gazeforge.eyerig/1", "catalog": catalog}));
    std::fs::write(s.join("eyerigs").join(format!("{}.json", gf_core::hash::sha256_hex(&old))), &old).unwrap();
    let (ok, listed) = gazeforge(&["call", "store.list", "--input", &format!(r#"{{"store": {:?}}}"#, s.to_str().unwrap())]);
    assert!(ok, "{listed}");
    let r = &listed["result"];
    assert_eq!(r["packs"][0]["pack"], pack.as_str());
    assert!(r["packs"][0]["imported"].as_str().is_some_and(|t| t.len() == 24 && t.ends_with('Z')), "{r}");
    assert_eq!(r["eyerigs"][0]["usable"], Value::Bool(false));
    assert_eq!(r["eyerigs"][0]["catalog"], catalog.as_str());
}

#[test]
fn a_spec_round_trips_its_face_through_the_store() {
    let (root, pack, catalog, game) = store_fixture("spec");
    let s = root.join("store");
    let spec = root.join("c.gfchar.json");
    let (s, spec_s, face) = (s.to_str().unwrap(), spec.to_str().unwrap(), root.join("face.npy"));
    let base = ["character", "create", "--store", s, "--pack", &pack, "--catalog", &catalog, "--game", &game, "--out", spec_s];
    let mut args = base.to_vec();
    args.extend_from_slice(&["--weights", r#"{"nose/a": 0.5}"#, "--residual-from", face.to_str().unwrap()]);
    let (ok, created) = gazeforge(&args);
    assert!(ok, "{created}");
    let text = std::fs::read_to_string(&spec).unwrap();
    assert!(text.contains(r#""placement":"f5/1""#), "{text}");
    let (ok, validated) = gazeforge(&["character", "validate", "--store", s, "--spec", spec_s]);
    assert!(ok && validated["revision"] == created["revision"] && validated["canonical"] == Value::Bool(true), "{validated}");
    let out = root.join("dense.npy");
    let (ok, evaluated) = gazeforge(&["character", "evaluate", "--store", s, "--spec", spec_s, "--out", out.to_str().unwrap(), "--precision", "f64"]);
    assert!(ok, "{evaluated}");
    let got: Array2<f64> = ndarray_npy::read_npy(&out).unwrap();
    let want: Array2<f32> = ndarray_npy::read_npy(&face).unwrap();
    let worst = got.iter().zip(want.iter()).map(|(a, b)| (a - *b as f64).abs()).fold(0.0, f64::max);
    assert!(worst <= 2.5e-6 + 1e-9, "{worst}");
}

#[test]
fn bad_specs_and_a_corrupt_store_are_refused() {
    let (root, pack, catalog, _) = store_fixture("badspec");
    let s = root.join("store");
    let spec = root.join("c.gfchar.json");
    let (s, spec_s) = (s.to_str().unwrap(), spec.to_str().unwrap());
    let create = |extra: &[&str]| {
        let mut args = vec!["character", "create", "--store", s, "--pack", &pack, "--catalog", &catalog, "--out", spec_s];
        args.extend_from_slice(extra);
        gazeforge(&args)
    };
    refused(create(&["--weights", r#"{"nose/a": 1.5}"#]), "outside [0, 1]");
    refused(create(&["--weights", r#"{"nose/zz": 0.5}"#]), "not a target of the catalog");
    refused(create(&["--placement", "f3/9"]), "not a released version");
    refused(create(&["--sliders", r#"{"age": 0.9}"#]), "needs --macro");
    refused(create(&["--quantum-mm", "0.01"]), "needs --residual-from");
    refused(create(&["--wieghts", "{}"]), "unknown option");
    assert!(create(&["--weights", r#"{"nose/a": 0.25}"#]).0);
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(&spec, text.replace(r#""format""#, r#""colour":"red","format""#)).unwrap();
    refused(gazeforge(&["character", "validate", "--store", s, "--spec", spec_s]), "colour");
    std::fs::write(&spec, text.replace("0.25", "7.0")).unwrap();
    refused(gazeforge(&["character", "validate", "--store", s, "--spec", spec_s]), "outside [0, 1]");
    std::fs::write(&spec, &text).unwrap();
    let hex = catalog.trim_start_matches("sha256:");
    let manifest = root.join("store").join("catalogs").join(format!("{hex}.json"));
    let original = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, original.replace("\"CC0\"", "\"XX0\"")).unwrap();
    refused(gazeforge(&["character", "validate", "--store", s, "--spec", spec_s]), "corrupt");
}

#[test]
fn export_obj_without_a_placement_is_the_explicit_f5() {
    let root = fixture("unnamed-obj");
    let (success, value) = export(&root, "good.npy", &["--tip-vertex", "0"]);
    assert!(success, "{value}");
    assert_eq!(value["placement_version"], Value::String("f5/1".into()));
    let unnamed = std::fs::read(root.join("out.obj")).unwrap();
    let (success, value) = export(&root, "good.npy", &["--placement", "f5/1", "--tip-vertex", "0"]);
    assert!(success, "{value}");
    assert_eq!(std::fs::read(root.join("out.obj")).unwrap(), unnamed);
}

#[test]
fn an_unnamed_placement_is_f5_for_the_spec_the_glb_and_the_designer() {
    let (root, pack, catalog, game) = store_fixture("unnamed-spec");
    let store = root.join("store");
    let spec = root.join("d.gfchar.json");
    let api_spec = root.join("api.gfchar.json");
    let glb = root.join("d.glb");
    let (s, spec_s) = (store.to_str().unwrap(), spec.to_str().unwrap());
    let (ok, created) = gazeforge(&["character", "create", "--store", s, "--pack", &pack, "--catalog", &catalog, "--game", &game, "--out", spec_s]);
    assert!(ok, "{created}");
    assert!(std::fs::read_to_string(&spec).unwrap().contains(r#""placement":"f5/1""#));
    let input = serde_json::json!({"store": s, "pack": pack, "catalog": catalog, "game": game, "out": api_spec.to_str().unwrap()});
    let (ok, created) = gazeforge(&["call", "character.create", "--input", &input.to_string()]);
    assert!(ok, "{created}");
    assert!(std::fs::read_to_string(&api_spec).unwrap().contains(r#""placement":"f5/1""#));
    let (ok, exported) = gazeforge(&["export-glb", "--store", s, "--spec", spec_s, "--out", glb.to_str().unwrap()]);
    assert!(ok, "{exported}");
    assert_eq!(exported["placement_version"], Value::String("f5/1".into()));
    let input = serde_json::json!({"store": s, "pack": pack, "catalog": catalog, "game": game});
    let (ok, settled) = gazeforge(&["call", "designer.settle", "--input", &input.to_string()]);
    assert!(ok, "{settled}");
    assert_eq!(settled["result"]["placement"], Value::String("f5/1".into()));
}
