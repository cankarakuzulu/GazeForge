"""Structural gates for the workspace layering.

Usage: dep_gate.py [edges|clippy|wasm|front-doors|all] [--root DIR] [--cargo PATH] [--target-dir DIR]

edges        every dependency edge between workspace crates (normal, dev and build) must be on ALLOWED.
clippy       cargo clippy on the pure crates (PURE) with -D warnings; their clippy.toml bans fs, thread,
             process, net and clock calls.
wasm         the pure maths crate builds for wasm32-unknown-unknown with no features.
front-doors  copy the workspace without its front doors (FRONT_DOORS) into a temporary directory and
             build the domain, format and api crates there.

Prints one JSON object; exit code 0 only when every selected gate passes.

Limits (what these gates cannot see):
- edges sees only workspace crates; a third-party crate that itself does IO or spawns threads is invisible.
- clippy sees calls written in the pure crates' own source; IO or threads inside a dependency (for example
  rayon behind the native-only feature) are not flagged, and a path missing from clippy.toml is not banned.
- wasm proves compilation only; a thread or clock panic at run time under wasm is not caught here.
- front-doors builds; it runs no tests in the reduced workspace.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

ALLOWED = {
    "gf-geom": [],
    "gf-core": ["gf-geom"],
    "gf-io": ["gf-core", "gf-geom"],
    "gf-api": ["gf-io", "gf-core", "gf-geom"],
    "gf-cli": ["gf-api", "gf-io", "gf-core", "gf-geom"],
    "gf-http": ["gf-api", "gf-io", "gf-core", "gf-geom"],
    "gf-mcp": ["gf-api", "gf-io", "gf-core", "gf-geom"],
    "gf-wasm": ["gf-geom"],
    "gf-studio": ["gf-api", "gf-io", "gf-core", "gf-geom"],
}
PURE = ["gf-geom", "gf-core"]
WASM = ["gf-geom", "gf-wasm"]
FRONT_DOORS = ["gf-cli", "gf-http", "gf-mcp", "gf-studio"]
KEEP_BUILT = ["gf-core", "gf-io", "gf-api"]


def run(cmd, cwd, env=None):
    proc = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace")
    return proc.returncode, proc.stdout, proc.stderr


def errors_of(text, limit=12):
    lines = [l for l in text.splitlines() if l.startswith("error") or l.lstrip().startswith("-->")]
    return lines[:limit]


def metadata(args):
    code, out, err = run([args["cargo"], "metadata", "--format-version", "1", "--no-deps"], args["root"])
    if code != 0:
        raise RuntimeError("cargo metadata failed: " + " | ".join(err.strip().splitlines()[-5:]))
    return json.loads(out)


def gate_edges(args):
    meta = metadata(args)
    names = {p["name"] for p in meta["packages"]}
    edges, bad, unknown = [], [], sorted(n for n in names if n not in ALLOWED)
    for package in meta["packages"]:
        for dep in package["dependencies"]:
            if dep["name"] not in names:
                continue
            kind = dep.get("kind") or "normal"
            edge = {"from": package["name"], "to": dep["name"], "kind": kind}
            edges.append(edge)
            if dep["name"] not in ALLOWED.get(package["name"], []):
                bad.append(edge)
    return {"pass": not bad and not unknown, "crates": sorted(names), "edges": edges, "refused": bad,
            "not_on_allow_list": unknown}


def gate_clippy(args):
    cmd = [args["cargo"], "clippy"] + sum((["-p", p] for p in PURE), []) + ["--all-features", "--all-targets", "--", "-D", "warnings"]
    toolchain = os.path.dirname(os.path.abspath(args["cargo"])) if os.path.dirname(args["cargo"]) else ""
    path = os.pathsep.join(p for p in (toolchain, os.environ.get("PATH", "")) if p)
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(args["target_dir"], "clippy"), PATH=path)
    code, _, err = run(cmd, args["root"], env)
    return {"pass": code == 0, "command": " ".join(cmd[1:]), "errors": errors_of(err)}


def gate_wasm(args):
    cmd = [args["cargo"], "build"] + sum((["-p", p] for p in WASM), []) + ["--target", "wasm32-unknown-unknown"]
    env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(args["target_dir"], "wasm"))
    code, _, err = run(cmd, args["root"], env)
    return {"pass": code == 0, "command": " ".join(cmd[1:]), "errors": errors_of(err)}


def gate_front_doors(args):
    meta = metadata(args)
    members = [p for p in meta["packages"] if p["name"] not in FRONT_DOORS]
    root = os.path.abspath(args["root"])
    with tempfile.TemporaryDirectory(prefix="gf_front_doors_") as tmp:
        rel = []
        for package in members:
            src = os.path.dirname(package["manifest_path"])
            r = os.path.relpath(src, root).replace(os.sep, "/")
            rel.append(r)
            shutil.copytree(src, os.path.join(tmp, r), ignore=shutil.ignore_patterns("target*"))
        with open(os.path.join(root, "Cargo.toml"), encoding="utf-8") as handle:
            lines = handle.read().splitlines()
        out, skipping = [], False
        for line in lines:
            if line.strip().startswith("members"):
                out.append("members = [" + ", ".join(json.dumps(r) for r in rel) + "]")
                skipping = "]" not in line
                continue
            if skipping:
                skipping = "]" not in line
                continue
            out.append(line)
        with open(os.path.join(tmp, "Cargo.toml"), "w", encoding="utf-8") as handle:
            handle.write("\n".join(out) + "\n")
        if os.path.exists(os.path.join(root, "Cargo.lock")):
            shutil.copy(os.path.join(root, "Cargo.lock"), os.path.join(tmp, "Cargo.lock"))
        cmd = [args["cargo"], "build", "--offline"] + sum((["-p", p] for p in KEEP_BUILT), [])
        env = dict(os.environ, CARGO_TARGET_DIR=os.path.join(args["target_dir"], "front_doors"))
        code, _, err = run(cmd, tmp, env)
    return {"pass": code == 0, "removed": [p for p in FRONT_DOORS if any(m["name"] == p for m in meta["packages"])],
            "kept": [p["name"] for p in members], "command": " ".join(cmd[1:]), "errors": errors_of(err)}


GATES = {"edges": gate_edges, "clippy": gate_clippy, "wasm": gate_wasm, "front-doors": gate_front_doors}


def main(argv):
    args = {"root": os.getcwd(), "cargo": os.environ.get("CARGO", "cargo"), "target_dir": os.path.join(os.getcwd(), "target-gates")}
    which = "all"
    rest = argv[1:]
    if rest and not rest[0].startswith("--"):
        which, rest = rest[0], rest[1:]
    while rest:
        key = rest.pop(0)
        if key not in ("--root", "--cargo", "--target-dir") or not rest:
            print(json.dumps({"ok": False, "error": f"unknown or incomplete option {key}"}))
            return 2
        args[key[2:].replace("-", "_")] = rest.pop(0)
    names = list(GATES) if which == "all" else [which]
    if any(n not in GATES for n in names):
        print(json.dumps({"ok": False, "error": f"unknown gate {which}; gates: {', '.join(GATES)} or all"}))
        return 2
    report = {}
    for name in names:
        try:
            report[name] = GATES[name](args)
        except (RuntimeError, OSError, ValueError) as error:
            report[name] = {"pass": False, "error": str(error)}
    ok = all(r["pass"] for r in report.values())
    print(json.dumps({"ok": ok, "gates": report}))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
