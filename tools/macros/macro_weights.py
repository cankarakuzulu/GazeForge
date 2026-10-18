"""GazeForge macro weights: macro slider values -> MakeHuman macrodetails target weights.

Clean-room implementation, written from macro.json, the target file names and observed MPFB output only.

Limits (what this cannot see):
- It reproduces MPFB's shape-key VALUES; it does not model MPFB's choice of which keys to create beyond two rules (the
  target file must exist, the weight must reach KEY_FLOOR). A weight under the floor is real in this model and absent in
  MPFB's mesh.
- Sliders outside [0, 1] are rejected; macro.json's own part ranges reach -0.01 and 1.01 and were never probed there.
- Verified on six MPFB 2.0.18 Humans only; a different MPFB or macro.json may differ.
"""
import itertools
import json
import os
import sys

MACRO_SLIDERS = ("gender", "age", "muscle", "weight", "proportions", "height", "cupsize", "firmness")
RACES = ("african", "asian", "caucasian")
KEY_FLOOR = 0.01
LABEL_DIGITS = 4

FAMILIES = (
    ("racegenderage", ("race", "gender", "age"), "macrodetails/"),
    ("genderagemuscleweight", ("gender", "age", "muscle", "weight"), "macrodetails/universal-"),
    ("genderagemuscleweightproportions", ("gender", "age", "muscle", "weight", "proportions"), "macrodetails/proportions/"),
    ("genderagemuscleweightheight", ("gender", "age", "muscle", "weight", "height"), "macrodetails/height/"),
    ("genderagemuscleweightcupsizefirmness", ("gender", "age", "muscle", "weight", "cupsize", "firmness"), "breast/"),
)
NOT_MULTIPLIED = {"genderagemuscleweightcupsizefirmness": ("gender",)}


def load_macro(path):
    with open(path) as handle:
        return json.load(handle)


def part_labels(part, t, digits):
    labels = {}
    if part["low"]:
        labels[part["low"]] = 1.0 - t if digits is None else round(1.0 - t, digits)
    if part["high"]:
        labels[part["high"]] = t if digits is None else round(t, digits)
    return labels


def slider_labels(macro, name, value, mode="mpfb"):
    parts = macro["macrotargets"][name]["parts"]
    for part in parts:
        if part["lowest"] <= value <= part["highest"]:
            t = (value - part["lowest"]) / (part["highest"] - part["lowest"])
            return part_labels(part, t, LABEL_DIGITS if mode == "mpfb" else None)
    if mode == "mpfb":
        return {}
    part = min(parts, key=lambda p: min(abs(value - p["lowest"]), abs(value - p["highest"])))
    t = min(1.0, max(0.0, (value - part["lowest"]) / (part["highest"] - part["lowest"])))
    return part_labels(part, t, None)


def variable_labels(macro, sliders, mode="mpfb"):
    labels = {name: slider_labels(macro, name, sliders[name], mode) for name in MACRO_SLIDERS}
    labels["race"] = {race: sliders[race] for race in RACES}
    return labels


def target_weights(macro, sliders, exists=None, mode="mpfb"):
    for name in MACRO_SLIDERS + RACES:
        value = sliders[name]
        if not 0.0 <= value <= 1.0:
            raise ValueError(f"slider {name}={value} outside [0, 1]")
    if mode not in ("mpfb", "gf"):
        raise ValueError(f"mode {mode!r} is neither 'mpfb' nor 'gf'")
    labels = variable_labels(macro, sliders, mode)
    weights = {}
    for family, variables, prefix in FAMILIES:
        if list(macro["combinations"][family]) != list(variables):
            raise ValueError(f"macro.json combination {family} is {macro['combinations'][family]}, expected {variables}")
        skip = NOT_MULTIPLIED.get(family, ())
        for combo in itertools.product(*(sorted(labels[v].items()) for v in variables)):
            weight = 1.0
            for variable, (_, w) in zip(variables, combo):
                if variable not in skip:
                    weight *= w
            name = prefix + "-".join(label for label, _ in combo)
            if exists is not None and not exists(name):
                continue
            weights[name] = weights.get(name, 0.0) + weight
    return weights


def mpfb_keys(weights, floor=None):
    floor = KEY_FLOOR if floor is None else floor
    return {name: w for name, w in weights.items() if w > floor}


def files_in(targets_root):
    found = set()
    for folder, _, files in itertools.chain(os.walk(os.path.join(targets_root, "macrodetails")),
                                            os.walk(os.path.join(targets_root, "breast"))):
        for file in files:
            for ext in (".target.gz", ".target", ".npz"):
                if file.endswith(ext):
                    rel = os.path.relpath(os.path.join(folder, file[: -len(ext)]), targets_root).replace(os.sep, "/")
                    found.add(rel)
    return found


def main(argv):
    if len(argv) < 4:
        print(json.dumps({"ok": False, "error": {"code": "invalid_input",
                                                  "message": "usage: macro_weights.py MACRO_JSON TARGETS_ROOT SLIDERS_JSON [--mode mpfb|gf]"}}))
        return 2
    mode = argv[argv.index("--mode") + 1] if "--mode" in argv else "gf"
    try:
        macro = load_macro(argv[1])
        available = files_in(argv[2])
        sliders = json.loads(argv[3])
        weights = target_weights(macro, sliders, exists=available.__contains__, mode=mode)
    except (OSError, ValueError, KeyError) as error:
        print(json.dumps({"ok": False, "error": {"code": "invalid_input", "message": f"{type(error).__name__}: {error}"}}))
        return 2
    if not available:
        print(json.dumps({"ok": False, "error": {"code": "not_found", "message": f"no macro target files under {argv[2]}"}}))
        return 3
    if mode == "mpfb":
        weights = mpfb_keys(weights)
    print(json.dumps({"ok": True, "mode": mode, "sliders": sliders, "weights": dict(sorted(weights.items()))}))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
