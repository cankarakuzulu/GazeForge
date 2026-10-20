use std::collections::HashMap;
use std::process::ExitCode;

use gf_api::tasks::{self, Options, data, mesh, rig, topology};
use serde_json::{Value, json};

mod front;
mod http;
mod mcp;

const USAGE: &str = "gazeforge <command> [--key value ...]
  build-base  --dense DIR --out DIR [--target 20000] [--shapes all|none|a,b] [--extra A.npy,B.npy] [--min-angle 4] [--max-angle 170]
              [--max-turn 45] [--boundary-weight 100] [--improve-below 20] [--flip-flat 15]
              [--weights W.npy (per dense vertex, > 0, multiplies the collapse cost)] [--lods 8000,3000 (nested, into OUT/lod_N)]
  export-obj  --dense DIR --base DIR --positions base|shape:NAME|FILE.npy --out FILE.obj [--placement f5|f6|subset|optimal|fit|f2|f3|f4|NAME/VERSION]
              [--tip-vertex 185894] [--quads on|off (clean triangle pairs written as quads; off by default)]
              optimal, fit, f2: [--max-move 3]
              fit, f2: [--fit-iterations 4] [--fit-smooth 0.1] [--fit-min-angle 5] [--fit-guard-steps 8] [--fit-repair-below 5] [--fit-trim 0]
              f2 only: [--f2-regular 1e-2] [--f2-max-turn 60] [--f2-turn-margin 2] [--f2-confident-spread 30] [--f2-confident-above 25]
              [--f2-angle-margin 0.5] [--f2-repair-rounds 8] [--f2-repair-max-step 0.4] [--f2-repair-max-step-confident 0.5]
              [--f2-uv-ring 2] [--f2-uvs surface|subset]
              f3 only: [--f3-regular 1e-3] [--f3-max-move 3] [--f3-smooth 0.1] [--f3-rounds 4] [--f3-relax-steps 24] [--f3-relax-step 0.5]
              [--f3-relax-angle 1,8] [--f3-relax-eye 3,8] [--f3-backoff-turn 25,55] [--f3-backoff-step 0.5] [--f3-uv-ring 2] [--eyes EYES.json]
              [--f3-eye-front 75] [--f3-eye-clearance 0.05] [--f3-eye-reach 2] [--f3-eye-depth 0.3,0.6] [--f3-eye-lift radial|gaze]
              [--f3-eye-rounds 1] [--f3-eye-samples 6]
              f4 = f3 + an angle pass: every f3 option, and [--f4-target 10] [--f4-passes 12] [--f4-step 0.25] [--f4-gain 0.5] [--f4-fold 60]
              [--f4-backoff dense|subset] [--f4-plane game|dense]
              f5 only: [--f5-tangential 0.01] [--f5-smooth 0.1] [--f5-membrane 0.1] [--f5-max-move 3]
              [--f5-uv-ring 2] [--f5-normals subset|dense] [--eyes EYES.json]
              [--f5-eye-front 75] [--f5-eye-clearance 0.05] [--f5-eye-depth 0.3,0.6]
              f6 = f5 + a continuous sliver nudge: every f5 option (f6 defaults --f5-membrane 0.05 --f5-smooth 0.05),
              and [--f6-sliver-deg 6] [--f6-sliver-rounds 32]
              [--f6-sliver-relax 0.5] [--f6-sliver-seam 0.25] [--f6-sliver-eye 4,8]
  collide-lids --dense DIR --positions base|shape:NAME|FILE.npy --eyes EYES.json --out FILE.npy [--clearance 0]
              [--front-deg 75] [--smooth 4]
  store import-pack --store DIR --dense DIR [--macro-base JSON|@FILE] [--pin-top Z_METRES]
  store import-catalog --store DIR --targets DIR --pack sha256:...
  store import-game --store DIR --base DIR --pack sha256:...
  store import-eyerig --store DIR --catalog sha256:... --makehuman MAKEHUMAN_DATA_DIR [--macro MACRO_JSON]
  character create --store DIR --pack sha256:... --catalog sha256:... --out FILE [--game sha256:...] [--placement NAME/VERSION]
              [--name NAME] [--weights JSON|@FILE] [--macro MACRO_JSON [--sliders JSON|@FILE] [--macro-mode gf|mpfb] [--head-scale 1]]
              [--residual-from FACE.npy [--quantum-mm 0.005]]
  character validate --store DIR --spec FILE [--macro MACRO_JSON]
  character evaluate --store DIR --spec FILE --out DENSE.npy [--macro MACRO_JSON] [--precision f32|f64]
  macro weights --macro MACRO_JSON --sliders JSON|@FILE [--targets TARGETS_ROOT] [--mode gf|mpfb] [--floor 0.01|none]
  macro labels --macro MACRO_JSON --slider NAME --values VALUES.npy --out TABLE.npy [--mode gf|mpfb]
  export-glb  --store DIR --spec FILE --out FILE.glb [--macro MACRO_JSON] [--morphs ID,ID|@FILE.json] [--sparse auto|always|never]
              [--lod-screen 0.25,0.1] [the options of the spec's placement, e.g. --eyes EYES.json]
  gates shell --dense DIR --before base|shape:NAME|FILE.npy --after base|shape:NAME|FILE.npy [--fold-tolerance 10]
              [--inner-tolerance 15] [--rough-mm 0.02] [--rough-rel 0.05] [--rough-tolerance 200]
  gates game  --dense DIR --base DIR --face base|shape:NAME|FILE.npy --obj GAME.obj [--gates triq,uv,census,seams]
              [--tip-vertex 185894] [--seam-limit-mm 0.05]
  gates path  --dense DIR --base DIR --start base|shape:NAME|FILE.npy --delta FILE.npy [--steps 20] [--pop-limit 10]
              [--placement f3 and its options] [--plant STEP,GAME_VERTEX,MM (a planted forward jump, to prove the gate)]
  A gate that fails prints its report with \"ok\": false and exits non-zero.
  topology improve --store DIR --game sha256:... [--catalog sha256:...] [--shape-targets all|head|none] [--samples 32] [--seed 1]
              [--dense DIR (adds its manifest shapes)] [--below 30] [--min-gain 0.5] [--max-fold 20] [--max-spread 40] [--rounds 32] [--levels all|finest]
              [--out DIR (also writes the new base as a directory, levels in DIR/lod_N)]
              edge flips over the same vertices, scored over the shape range; imports the result as a new game base
  topology inspect --store DIR --game sha256:...
JSON|@FILE: inline JSON, or @ and a path to a JSON file. MACRO_JSON is read from the user's MakeHuman/MPFB install.
An option the command does not read is refused; numbers must be finite and not negative; input positions must be finite.
The command registry (the same commands over every front door):
  commands                              list every command with its input schema
  call    COMMAND --input JSON|@FILE    run one command
  serve   [--port N]                    local HTTP API on 127.0.0.1: GET /v1/commands, POST /v1/<command>
  mcp                                   MCP server over stdio
Every command prints one JSON object on stdout; a non-zero exit code means it failed.";


fn options(args: &[String]) -> Result<Options, String> {
    let mut out = HashMap::new();
    let mut iter = args.iter();
    while let Some(key) = iter.next() {
        let name = key.strip_prefix("--").ok_or_else(|| format!("expected --key, got {key}"))?;
        let value = iter.next().ok_or_else(|| format!("--{name} needs a value"))?;
        if out.insert(name.to_string(), value.clone()).is_some() {
            return Err(format!("--{name} is given more than once"));
        }
    }
    Ok(out)
}

fn run(args: &[String]) -> Result<Value, String> {
    match args.first().map(String::as_str) {
        Some("build-base") => options(&args[1..]).and_then(|o| tasks::build_base(&o)),
        Some("export-obj") => options(&args[1..]).and_then(|o| tasks::export_obj(&o)),
        Some("collide-lids") => options(&args[1..]).and_then(|o| tasks::collide_lids(&o)),
        Some("export-glb") => options(&args[1..]).and_then(|o| mesh::export_glb(&o)),
        Some("gates") => match args.get(1).map(String::as_str) {
            Some("shell") => options(&args[2..]).and_then(|o| mesh::gates_shell(&o)),
            Some("game") => options(&args[2..]).and_then(|o| mesh::gates_game(&o)),
            Some("path") => options(&args[2..]).and_then(|o| mesh::gates_path(&o)),
            Some("facing") => options(&args[2..]).and_then(|o| mesh::gates_facing(&o)),
            Some("lids") => options(&args[2..]).and_then(|o| mesh::gates_lids(&o)),
            Some("rig") => options(&args[2..]).and_then(|o| rig::gates_rig(&o)),
            _ => Err(USAGE.to_string()),
        },
        Some("topology") => match args.get(1).map(String::as_str) {
            Some("improve") => options(&args[2..]).and_then(|o| topology::improve(&o)),
            Some("inspect") => options(&args[2..]).and_then(|o| topology::inspect(&o)),
            _ => Err(USAGE.to_string()),
        },
        Some(group @ ("store" | "character" | "macro")) => match args.get(1) {
            Some(verb) => options(&args[2..]).and_then(|o| data::run(group, verb, &o)),
            None => Err(USAGE.to_string()),
        },
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = front::dispatch(&args) {
        return code;
    }
    match run(&args) {
        Ok(value) => {
            println!("{value}");
            if value.get("ok") == Some(&Value::Bool(false)) { ExitCode::FAILURE } else { ExitCode::SUCCESS }
        }
        Err(error) => {
            println!("{}", json!({"ok": false, "error": error}));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gf_api::tasks::*;
    use gf_io::dense;

    fn opts(pairs: &[&str]) -> Result<Options, String> {
        options(&pairs.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn export_error(pairs: &[&str]) -> String {
        let mut args = vec!["export-obj".to_string()];
        args.extend(pairs.iter().map(|s| s.to_string()));
        run(&args).unwrap_err()
    }

    #[test]
    fn unknown_flags_are_refused() {
        let e = export_error(&["--placement", "fit", "--max-mvoe", "2"]);
        assert!(e.contains("unknown option") && e.contains("--max-mvoe"), "{e}");
        let e = run(&["build-base".into(), "--dense".into(), "d".into(), "--targte".into(), "1".into()]).unwrap_err();
        assert!(e.contains("--targte"), "{e}");
    }

    #[test]
    fn flags_the_placement_does_not_read_are_refused() {
        assert!(export_error(&["--placement", "fit", "--f2-regular", "0.1"]).contains("--f2-regular"));
        assert!(export_error(&["--placement", "subset", "--max-move", "2"]).contains("--max-move"));
        assert!(export_error(&["--placement", "optimal", "--fit-iterations", "2"]).contains("--fit-iterations"));
    }

    #[test]
    fn known_flags_pass_the_key_check() {
        let o = opts(&["--placement", "f2", "--max-move", "3", "--fit-trim", "0", "--f2-uvs", "subset", "--tip-vertex", "7"]).unwrap();
        assert!(only(&o, "export-obj", &[EXPORT_KEYS, FIT_KEYS, F2_KEYS]).is_ok());
    }

    #[test]
    fn repeated_flags_are_refused() {
        assert!(opts(&["--max-move", "2", "--max-move", "3"]).unwrap_err().contains("more than once"));
    }

    #[test]
    fn negative_magnitudes_are_refused() {
        let e = export_error(&["--placement", "fit", "--max-move", "-1"]);
        assert!(e.contains("--max-move must not be negative"), "{e}");
        assert!(export_error(&["--placement", "f2", "--f2-uv-ring", "-2"]).contains("must not be negative"));
        let o = opts(&["--target", "-20000"]).unwrap();
        assert!(count(&o, "target", 20000).unwrap_err().contains("must not be negative"));
    }

    #[test]
    fn non_finite_and_fractional_numbers_are_refused() {
        for bad in ["NaN", "nan", "inf", "-inf", "infinity"] {
            let o = opts(&["--max-move", bad]).unwrap();
            assert!(number(&o, "max-move", 3.0).is_err(), "{bad} accepted");
        }
        let o = opts(&["--fit-iterations", "2.5"]).unwrap();
        assert!(count(&o, "fit-iterations", 4).unwrap_err().contains("whole number"));
    }

    #[test]
    fn valid_numbers_and_defaults_are_unchanged() {
        let o = opts(&["--max-move", "2.5", "--fit-iterations", "6", "--f2-regular", "1e-2"]).unwrap();
        assert_eq!(number(&o, "max-move", 3.0).unwrap(), 2.5);
        assert_eq!(count(&o, "fit-iterations", 4).unwrap(), 6);
        assert_eq!(number(&o, "f2-regular", 0.0).unwrap(), 0.01);
        assert_eq!(number(&o, "fit-smooth", 0.1).unwrap(), 0.1);
        assert_eq!(count(&o, "tip-vertex", dense::NOSE_TIP_VERTEX).unwrap(), 185_894);
    }

    #[test]
    fn the_designer_places_with_the_cli_default_settings() {
        let none = Options::new();
        let cli = serde_json::to_value(f3_settings(&none).unwrap()).unwrap();
        assert_eq!(cli, serde_json::to_value(gf_api::designer::f3_defaults()).unwrap());
        let f4 = serde_json::to_value(f3_placement(&none, "f4").unwrap().settings.f4).unwrap();
        assert_eq!(f4, serde_json::to_value(Some(gf_api::designer::f4_defaults())).unwrap());
        let f5 = serde_json::to_value(f5_placement(&none).unwrap().settings).unwrap();
        assert_eq!(f5, serde_json::to_value(gf_api::designer::f5_defaults()).unwrap());
    }
}
