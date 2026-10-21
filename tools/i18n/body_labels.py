"""Generate the locale entries for the body sliders: body.slider.<id> for every regional pair or single target of a body
targets set (as body.sliders lists them), body.compound.<name> for the compound parameters, body.family.<family>.

usage: py -3 tools/i18n/body_labels.py SLIDERS_RESULT.json   (the result object of body.sliders)
Rewrites web/locales/en.json in place (sorted, indent 1), keeping every existing entry.
"""
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
WORDS = {"l": "left", "r": "right", "upperleg": "upper leg", "lowerleg": "lower leg", "upperarm": "upper arm", "lowerarm": "lower arm",
         "upperlegs": "upper legs", "lowerlegs": "lower legs", "circ": "circumference", "dist": "distance", "horiz": "horizontal",
         "vert": "vertical", "trans": "move", "napetowaist": "nape to waist", "waisttohip": "waist to hip", "frontchest": "front chest",
         "underbust": "underbust", "vshape": "V shape", "dorsi": "back (dorsi)", "valgus": "valgus", "measure": ""}
FAMILY = {"arms": "Arms", "legs": "Legs", "feet": "Feet", "hands": "Hands", "hip": "Hips", "stomach": "Stomach", "torso": "Torso", "pelvis": "Pelvis",
          "buttocks": "Buttocks", "neck": "Neck", "genitals": "Genitals"}
COMPOUND = {"leg_length": "Leg length", "arm_length": "Arm length", "shoulder_width": "Shoulder width", "torso_length": "Torso length",
            "hip_width": "Hip width", "waist": "Waist", "chest": "Chest", "athletic": "Athletic", "curvy": "Curvy", "belly": "Belly",
            "hand_size": "Hand size", "foot_size": "Foot size", "arm_muscle": "Arm muscle", "leg_muscle": "Leg muscle"}


LABELS = {"genitals/penis-testicles": "Testicle size", "genitals/penis-circ": "Penis girth"}


def label(slider_id):
    if slider_id in LABELS:
        return LABELS[slider_id]
    name = slider_id.split("/", 1)[1]
    words = [WORDS.get(w, w) for w in re.split(r"-", name)]
    text = " ".join(w for w in words if w).strip()
    return text[:1].upper() + text[1:]


r = json.load(open(sys.argv[1], encoding="utf-8"))
p = os.path.join(ROOT, "web", "locales", "en.json")
raw = open(p, encoding="utf-8").read()
d = json.loads(raw)
assert json.dumps(d, indent=1, ensure_ascii=False, sort_keys=True) + "\n" == raw, "en.json does not round-trip; refusing to rewrite it"
for pair in r["pairs"]:
    d["body.slider." + pair["id"]] = label(pair["id"])
for c in r["compounds"]:
    d["body.compound." + c["name"]] = COMPOUND.get(c["name"], c["name"].replace("_", " ").capitalize())
for fam in {pair["family"] for pair in r["pairs"]}:
    d["body.family." + fam] = FAMILY.get(fam, fam.capitalize())
open(p, "w", encoding="utf-8", newline="\n").write(json.dumps(d, indent=1, ensure_ascii=False, sort_keys=True) + "\n")
print(len(r["pairs"]), "sliders,", len(r["compounds"]), "compounds")
