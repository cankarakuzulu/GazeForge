"""Reference-photo gate: head pose and image quality of a front and a profile photo; one JSON object on stdout.

usage: photo_gate.py check <args.json>
args: front and/or profile (image paths), profile_side (right|left, default right), landmarker (.task model),
      thresholds (optional overrides of the calibrated limits in photo_gate_model.json), allow (optional list of check names
      recorded as allowed), run (optional directory that receives gate.json).

Pose convention (degrees, head relative to the camera): yaw + turns the nose to the subject's left, pitch + lifts the nose,
roll + tips the top of the head to the subject's left; the rotation is Rz(yaw) Rx(-pitch) Ry(roll) in a frame with x to the
subject's left, -y forward, z up. A profile's yaw is its deviation from a true 90 degree side view.

Estimators and their measured limits (photo_gate_model.json holds the calibration and its error table):
- front yaw: MediaPipe's facial transformation matrix, referred to the base mesh rendered straight on.
- front roll: PnP of 30 stable landmarks against the base mesh's own landmark positions (70 mm full-frame lens assumed).
- front pitch: the mean of the transformation matrix's pitch and that PnP's pitch. Each moved by up to about 5 degrees with
  face shape alone, on different faces; the mean halves the worst case. Pitch is the least certain front axis.
- profile pitch: a trimmed ICP similarity fit of the silhouette's front edge (forehead to chin) to the base's straight profile.
- profile yaw: the landmarker finds no face in a true side view; when it finds one, the view is turned toward the camera and
  its reading is used. Below that detector's floor, and for views turned away from the camera, yaw is not measured.
- profile roll is not observable in a side view and is reported as unmeasured, never as zero.
"""
import json
import math
import os
import sys

import cv2
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import photo_worker as pw

MODEL_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "photo_gate_model.json")
A_FRONT = np.array([[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]])
R0_CV = np.array([[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]])
FOCAL_RATIO = 70.0 / 36.0
STABLE = pw.STABLE
FACE_OVAL = [10, 338, 297, 332, 284, 251, 389, 356, 454, 323, 361, 288, 397, 365, 379, 378, 400, 377, 152, 148, 176, 149, 150, 136,
             172, 58, 132, 93, 234, 127, 162, 21, 54, 103, 67, 109]


def model():
    with open(MODEL_PATH, encoding="utf-8") as fh:
        return json.load(fh)


def thresholds(overrides):
    t = dict(model()["thresholds"])
    for k, v in (overrides or {}).items():
        if k not in t:
            pw.fail(f"unknown threshold {k}", "invalid_input", known=sorted(t))
        t[k] = float(v)
    return t


def landmarker(path, faces=4, confidence=0.5):
    from mediapipe.tasks import python as mp_python
    from mediapipe.tasks.python import vision
    if not os.path.isfile(path):
        pw.fail(f"the face landmarker model {path} does not exist", "not_found")
    with open(path, "rb") as fh:
        base = mp_python.BaseOptions(model_asset_buffer=fh.read())
    opts = vision.FaceLandmarkerOptions(base_options=base, num_faces=faces, min_face_detection_confidence=confidence,
                                        min_face_presence_confidence=confidence, output_face_blendshapes=True,
                                        output_facial_transformation_matrixes=True)
    return vision.FaceLandmarker.create_from_options(opts)


def faces_in(det, bgr):
    import mediapipe as mp
    rgb = np.ascontiguousarray(cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB))
    res = det.detect(mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb))
    h, w = bgr.shape[:2]
    out = []
    for i, marks in enumerate(res.face_landmarks):
        pts = np.array([[p.x * w, p.y * h, p.z * w] for p in marks])
        shapes = {c.category_name: float(c.score) for c in res.face_blendshapes[i]} if res.face_blendshapes else {}
        matrix = np.array(res.facial_transformation_matrixes[i]) if res.facial_transformation_matrixes else None
        out.append({"points": pts, "blendshapes": shapes, "matrix": matrix})
    out.sort(key=lambda f: -float(np.ptp(f["points"][:, 1])))
    return out


def orthonormal(m):
    u, _, vt = np.linalg.svd(m)
    r = u @ vt
    if np.linalg.det(r) < 0:
        u[:, -1] *= -1
        r = u @ vt
    return r


def angles(r):
    b = math.asin(max(-1.0, min(1.0, r[2, 1])))
    return {"yaw": math.degrees(math.atan2(-r[0, 1], r[1, 1])), "pitch": -math.degrees(b),
            "roll": math.degrees(math.atan2(-r[2, 0], r[2, 2]))}


def pnp_pose(pts, ref, size):
    w, h = size
    obj = np.array(ref["stable_3d"], float)
    f = FOCAL_RATIO * max(w, h)
    k = np.array([[f, 0.0, w / 2], [0.0, f, h / 2], [0.0, 0.0, 1.0]])
    rvec0, _ = cv2.Rodrigues(R0_CV)
    t0 = -R0_CV @ np.array(ref["camera_guess"], float)
    ok, rvec, tvec = cv2.solvePnP(obj, pts[STABLE, :2].astype(np.float64), k, None, rvec0, t0.reshape(3, 1),
                                  useExtrinsicGuess=True, flags=cv2.SOLVEPNP_ITERATIVE)
    if not ok:
        return None, None
    r, _ = cv2.Rodrigues(rvec)
    proj, _ = cv2.projectPoints(obj, rvec, tvec, k, None)
    rms = float(np.sqrt(((proj.reshape(-1, 2) - pts[STABLE, :2]) ** 2).sum(1).mean()))
    a = angles(R0_CV.T @ r)
    return {ax: a[ax] - ref["pnp_offset"][ax] for ax in a}, rms


def front_pose(face, ref, size):
    pts = face["points"]
    span = float(np.linalg.norm(pts[263, :2] - pts[33, :2]))
    a = angles(A_FRONT.T @ orthonormal(face["matrix"][:3, :3]) @ np.array(ref["reference"]).T @ A_FRONT)
    b, rms = pnp_pose(pts, ref, size)
    conf = float(min(1.0, span / 60.0))
    out = {"yaw": {"deg": a["yaw"], "confidence": conf, "estimator": "landmark transform"}}
    if b is None:
        out["pitch"] = {"deg": None, "confidence": 0.0, "estimator": "mean of landmark transform and PnP on 30 base landmarks"}
        out["roll"] = {"deg": None, "confidence": 0.0, "estimator": "PnP on 30 base landmarks"}
        return out
    fit = conf * math.exp(-rms / (0.05 * span))
    out["pitch"] = {"deg": 0.5 * (a["pitch"] + b["pitch"]), "confidence": fit, "reprojection_px": rms,
                    "estimator": "mean of landmark transform and PnP on 30 base landmarks", "transform": a["pitch"], "pnp": b["pitch"]}
    out["roll"] = {"deg": b["roll"], "confidence": fit, "estimator": "PnP on 30 base landmarks", "reprojection_px": rms}
    return out


def similarity(src, dst):
    ms, md = src.mean(0), dst.mean(0)
    a, b = src - ms, dst - md
    u, sv, vt = np.linalg.svd(b.T @ a / len(a))
    d = np.diag([1.0, np.sign(np.linalg.det(u @ vt))])
    r = u @ d @ vt
    s = (sv * np.diag(d)).sum() / max((a ** 2).sum() / len(a), 1e-12)
    return r, s, md - s * r @ ms


def curve_segment(prof):
    keys = prof["keys"] or {}
    n, c = keys.get("nasion"), keys.get("chin")
    if not n or not c or c[1] - n[1] < 10:
        return None, None
    h = c[1] - n[1]
    right = prof["right"]
    rows = range(max(int(n[1] - 0.6 * h), 0), min(int(c[1] + 0.1 * h), len(right)))
    pts = np.array([[right[v], -float(v)] for v in rows if not np.isnan(right[v])])
    return (pts if len(pts) >= 20 else None), {"nasion": [n[0], -n[1]], "height": h}


def curve_pitch(prof, ref):
    p, meta = curve_segment(prof)
    if p is None:
        return None
    q = np.array(ref["profile_curve"], float)
    s0 = meta["height"] / ref["profile_curve_height"]
    best = None
    for start in (-20.0, -10.0, 0.0, 10.0, 20.0):
        a = math.radians(start)
        r = np.array([[math.cos(a), -math.sin(a)], [math.sin(a), math.cos(a)]])
        s = s0
        t = np.array(meta["nasion"]) - s * r @ np.array(ref["profile_curve_nasion"])
        for _ in range(30):
            qt = s * (r @ q.T).T + t
            d = ((p[:, None, :] - qt[None, :, :]) ** 2).sum(2)
            j = d.argmin(1)
            dist = d[np.arange(len(p)), j]
            keep = dist <= np.percentile(dist, 80)
            r, s, t = similarity(q[j[keep]], p[keep])
        qt = s * (r @ q.T).T + t
        dist = np.sqrt(((p[:, None, :] - qt[None, :, :]) ** 2).sum(2).min(1))
        rms = float(np.sqrt((np.sort(dist)[: int(0.8 * len(dist))] ** 2).mean()))
        if best is None or rms < best[1]:
            best = (math.degrees(math.atan2(r[1, 0], r[0, 0])), rms, s / s0)
    return {"deg": best[0] - ref["profile_pitch_offset"], "rms_rel": best[1] / meta["height"], "scale_ratio": best[2]}


def profile_pose(prof, ref, detected):
    out = {"yaw": {"deg": None, "confidence": 0.0}, "pitch": {"deg": None, "confidence": 0.0},
           "roll": {"deg": None, "confidence": 0.0, "note": "not observable in a side view; the front photo carries roll"}}
    cp = curve_pitch(prof, ref)
    if cp is not None:
        conf = float(math.exp(-cp["rms_rel"] / ref["profile_curve_rms_scale"]))
        out["pitch"] = {"deg": cp["deg"], "confidence": conf, "estimator": "silhouette curve fit", "rms_rel": cp["rms_rel"]}
    floor = ref["profile_yaw_floor_deg"]
    if detected is not None:
        frontal = angles(A_FRONT.T @ orthonormal(detected["matrix"][:3, :3]) @ np.array(ref["reference"]).T @ A_FRONT)["yaw"]
        k = ref["profile_yaw_gain"]
        out["yaw"] = {"deg": -(90.0 - abs(frontal)) * k, "confidence": 1.0, "estimator": "landmark transform (face found)",
                      "landmark_yaw_from_frontal": frontal}
    else:
        out["yaw"] = {"deg": 0.0, "confidence": 0.5, "estimator": "no face found by the landmarker",
                      "note": f"within about {floor:g} deg of a true side view toward the camera; turned-away views are not measured"}
    return out


def face_mask(shape, pts):
    mask = np.zeros(shape[:2], np.uint8)
    cv2.fillConvexPoly(mask, cv2.convexHull(pts[FACE_OVAL, :2].astype(np.int32)), 1)
    return mask.astype(bool)


def sharpness(img, mask):
    ys, xs = np.nonzero(mask)
    if len(ys) < 100:
        return None
    y0, y1, x0, x1 = ys.min(), ys.max() + 1, xs.min(), xs.max() + 1
    gray = cv2.cvtColor(img[y0:y1, x0:x1], cv2.COLOR_BGR2GRAY)
    m = mask[y0:y1, x0:x1].astype(np.uint8)
    size = (max(8, int(round(256 * (x1 - x0) / (y1 - y0)))), 256)
    g = cv2.resize(gray, size, interpolation=cv2.INTER_AREA).astype(np.float32)
    m = cv2.erode(cv2.resize(m, size, interpolation=cv2.INTER_NEAREST), np.ones((5, 5), np.uint8)).astype(bool)
    lap = cv2.Laplacian(g, cv2.CV_32F, ksize=3)
    return float(lap[m].var())


def clipping(img, mask):
    y = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)[mask]
    if len(y) == 0:
        return None, None
    return float((y >= 250).mean()), float((y <= 5).mean())


def background(img, subject, face):
    lab = cv2.cvtColor(img, cv2.COLOR_BGR2LAB).astype(np.float32)
    h, w = lab.shape[:2]
    outside = None
    if subject is not None and subject.mean() < 0.85:
        grow = max(5, int(0.012 * min(h, w))) | 1
        outside = ~cv2.dilate(subject.astype(np.uint8), np.ones((grow, grow), np.uint8)).astype(bool)
    if outside is None or outside.mean() < 0.05:
        b = max(4, int(0.03 * min(h, w)))
        outside = np.zeros((h, w), bool)
        outside[:b] = True
        outside[: int(0.6 * h), :b] = True
        outside[: int(0.6 * h), -b:] = True
    px = lab[outside]
    bg = np.median(px, axis=0)
    spread = float(np.percentile(np.linalg.norm(px - bg, axis=1), 95))
    contrast = float(np.linalg.norm(np.median(lab[face], axis=0) - bg)) if face.any() else 0.0
    return spread, contrast


def check(name, value, ok, threshold, reason):
    return {"name": name, "value": value, "threshold": threshold, "pass": bool(ok), "reason": None if ok else reason}


def pose_checks(view, pose, t, conf_min):
    out = []
    for axis in ("yaw", "pitch", "roll"):
        p = pose[axis]
        limit = t[f"{view}_{axis}_deg"]
        name = f"{view}.{axis}"
        if limit <= 0:
            out.append(check(name, p["deg"], True, "not gated", None))
        elif p["deg"] is None or p["confidence"] < conf_min:
            out.append(check(name, p["deg"], False, f"|{axis}| <= {limit:g} deg",
                             f"the {view} {axis} could not be measured (confidence {p['confidence']:.2f} < {conf_min:g})"))
        else:
            ok = abs(p["deg"]) <= limit
            out.append(check(name, round(p["deg"], 2), ok, f"|{axis}| <= {limit:g} deg",
                             f"the head in the {view} photo is rotated {p['deg']:+.1f} deg in {axis} (limit {limit:g})"))
    return out


def check_front(path, det, ref, t):
    img = pw.read_image(path)
    h, w = img.shape[:2]
    faces = faces_in(det, img)
    checks = [check("front.faces", len(faces), len(faces) == 1, "== 1",
                    "no face was found" if not faces else f"{len(faces)} faces were found; the photo must show one person")]
    report = {"path": path, "size": [w, h], "faces": len(faces)}
    if not faces:
        report["checks"] = checks
        return report
    face = faces[0]
    pts = face["points"]
    pose = front_pose(face, ref, (w, h))
    report["pose"] = {k: {kk: (round(vv, 3) if isinstance(vv, float) else vv) for kk, vv in v.items()} for k, v in pose.items()}
    checks += pose_checks("front", pose, t, t["min_pose_confidence"])
    height = float(np.linalg.norm(pts[152, :2] - pts[10, :2]))
    checks.append(check("front.face_height", round(height, 1), height >= t["front_min_face_px"], f">= {t['front_min_face_px']:g} px",
                        f"the face is {height:.0f} px tall (forehead to chin); at least {t['front_min_face_px']:g} px is needed"))
    mask = face_mask(img.shape, pts)
    sharp = sharpness(img, mask)
    checks.append(check("front.sharpness", sharp and round(sharp, 2), sharp is not None and sharp >= t["front_min_sharpness"],
                        f">= {t['front_min_sharpness']:g}", f"the face is blurred (Laplacian variance {sharp or 0:.1f})"))
    hi, lo = clipping(img, mask)
    checks.append(check("front.highlights", round(hi, 4), hi <= t["max_clipped_highlights"], f"<= {t['max_clipped_highlights']:g}",
                        f"{100 * hi:.1f} % of the face is clipped white (overexposed or a hard highlight)"))
    checks.append(check("front.shadows", round(lo, 4), lo <= t["max_clipped_shadows"], f"<= {t['max_clipped_shadows']:g}",
                        f"{100 * lo:.1f} % of the face is clipped black (underexposed or a hard shadow)"))
    bs = face["blendshapes"]
    blink = max(bs.get("eyeBlinkLeft", 0.0), bs.get("eyeBlinkRight", 0.0))
    checks.append(check("front.eyes_open", round(blink, 3), blink <= t["max_eye_blink"], f"eye blink <= {t['max_eye_blink']:g}",
                        f"the eyes look closed or narrowed (blink score {blink:.2f})"))
    jaw = bs.get("jawOpen", 0.0)
    checks.append(check("front.mouth_closed", round(jaw, 3), jaw <= t["max_jaw_open"], f"jaw open <= {t['max_jaw_open']:g}",
                        f"the mouth looks open (jaw-open score {jaw:.2f}); a neutral, closed mouth is needed"))
    sil = pw.silhouette(img)
    spread, contrast = background(img, sil, mask)
    checks.append(check("front.background", round(spread, 2), spread <= t["max_background_spread"],
                        f"spread <= {t['max_background_spread']:g}", f"the background is not plain (colour spread {spread:.1f})"))
    checks.append(check("front.contrast", round(contrast, 2), contrast >= t["front_min_contrast"],
                        f">= {t['front_min_contrast']:g}", f"the subject does not stand out from the background ({contrast:.1f})"))
    report["blendshapes"] = {k: round(bs.get(k, 0.0), 3) for k in ("eyeBlinkLeft", "eyeBlinkRight", "jawOpen", "mouthClose")}
    report["checks"] = checks
    return report


def check_profile(path, side, det, ref, t):
    img = pw.read_image(path)
    h, w = img.shape[:2]
    upright = img if side == "right" else cv2.flip(img, 1)
    prof = pw.analyse_profile(upright)
    report = {"path": path, "size": [w, h], "side": side}
    if prof is None:
        report["checks"] = [check("profile.silhouette", None, False, "found", "no head silhouette was found (is the background plain?)")]
        return report
    r, l = prof["bump_right_px"], prof["bump_left_px"]
    floor = 0.01 * (prof["bottom"] - prof["top"])
    facing = "unknown"
    if r > 2.0 * l and r > floor:
        facing = side
    elif l > 2.0 * r and l > floor:
        facing = "left" if side == "right" else "right"
    checks = [check("profile.facing", facing, facing == side, f"shows the {side} side",
                    f"the profile shows the {facing} side, but {side} was declared (mirrored or mislabelled)")]
    mask = prof["mask"]
    ys, xs = np.nonzero(mask[prof["top"]:prof["bottom"]])
    head = (xs.min(), prof["top"] + ys.min(), xs.max(), prof["top"] + ys.max()) if len(xs) else (0, 0, w, h)
    others, own = 0, None
    for f in faces_in(det, upright):
        cx, cy = f["points"][:, 0].mean(), f["points"][:, 1].mean()
        if head[0] <= cx <= head[2] and head[1] <= cy <= head[3]:
            own = own or f
        else:
            others += 1
    checks.append(check("profile.faces", others, others == 0, "no face outside the subject",
                        f"{others} other face(s) were found beside the subject"))
    keys = prof["keys"] or {}
    if keys.get("nasion") and keys.get("chin_bottom"):
        height = float(keys["chin_bottom"][1] - keys["nasion"][1])
    else:
        height = 0.0
    checks.append(check("profile.face_height", round(height, 1), height >= t["profile_min_face_px"], f">= {t['profile_min_face_px']:g} px",
                        f"the face is {height:.0f} px from nasion to chin; at least {t['profile_min_face_px']:g} px is needed"
                        if height else "the profile points (nasion, chin) were not found"))
    pose = profile_pose(prof, ref, own)
    report["pose"] = {k: {kk: (round(vv, 3) if isinstance(vv, float) else vv) for kk, vv in v.items()} for k, v in pose.items()}
    checks += pose_checks("profile", pose, t, t["min_pose_confidence"])
    fm = np.zeros(mask.shape, bool)
    if keys.get("nasion") and keys.get("chin_bottom") and keys.get("nose_tip"):
        unit = height
        x1 = int(keys["nose_tip"][0] + 0.05 * unit)
        x0 = int(x1 - 0.95 * unit)
        y0 = int(keys["nasion"][1] - 0.35 * unit)
        y1 = int(keys["chin_bottom"][1])
        fm[max(y0, 0):max(y1, 0), max(x0, 0):max(x1, 0)] = True
        fm &= mask
    sharp = sharpness(upright, fm) if fm.any() else None
    checks.append(check("profile.sharpness", sharp and round(sharp, 2), sharp is not None and sharp >= t["profile_min_sharpness"],
                        f">= {t['profile_min_sharpness']:g}", f"the face is blurred (Laplacian variance {sharp or 0:.1f})"))
    hi, lo = clipping(upright, fm) if fm.any() else (1.0, 1.0)
    checks.append(check("profile.highlights", round(hi, 4), hi <= t["max_clipped_highlights"], f"<= {t['max_clipped_highlights']:g}",
                        f"{100 * hi:.1f} % of the face is clipped white (overexposed or a hard highlight)"))
    checks.append(check("profile.shadows", round(lo, 4), lo <= t["max_clipped_shadows"], f"<= {t['max_clipped_shadows']:g}",
                        f"{100 * lo:.1f} % of the face is clipped black (underexposed or a hard shadow)"))
    spread, contrast = background(upright, mask, fm if fm.any() else mask)
    checks.append(check("profile.background", round(spread, 2), spread <= t["max_background_spread"],
                        f"spread <= {t['max_background_spread']:g}", f"the background is not plain (colour spread {spread:.1f})"))
    checks.append(check("profile.contrast", round(contrast, 2), contrast >= t["profile_min_contrast"],
                        f">= {t['profile_min_contrast']:g}", f"the subject does not stand out from the background ({contrast:.1f})"))
    report["checks"] = checks
    return report


def run_check(args):
    t = thresholds(args.get("thresholds"))
    ref = model()
    allow = set(args.get("allow") or [])
    det = landmarker(args["landmarker"])
    photos = {}
    if args.get("front"):
        photos["front"] = check_front(args["front"], det, ref, t)
    if args.get("profile"):
        photos["profile"] = check_profile(args["profile"], args.get("profile_side", "right"), det, ref, t)
    if not photos:
        pw.fail("give a front photo, a profile photo or both", "invalid_input")
    failures, allowed = [], []
    for name, rep in photos.items():
        rep["pass"] = all(c["pass"] or c["name"] in allow for c in rep["checks"])
        for c in rep["checks"]:
            if not c["pass"]:
                (allowed if c["name"] in allow else failures).append(f"{c['name']}: {c['reason']}")
    result = {"pass": not failures, "failures": failures, "allowed": allowed, "photos": photos, "thresholds": t,
              "calibration": ref.get("calibration_summary")}
    if args.get("run"):
        os.makedirs(args["run"], exist_ok=True)
        pw.write_json(os.path.join(args["run"], "gate.json"), result)
    return result


def main(argv):
    if len(argv) != 3 or argv[1] != "check":
        pw.fail("usage: photo_gate.py check <args.json>", "invalid_input")
    with open(argv[2], encoding="utf-8") as fh:
        args = json.load(fh)
    pw.emit({"ok": True, **run_check(args)})


if __name__ == "__main__":
    main(sys.argv)
