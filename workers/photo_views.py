"""Photo views for the direct fit (fit.photos): per photo, face landmarks, the subject's outline, the profile keys of a side
view and, for an extra view, the reference-photo gate's quality checks; one JSON object on stdout, non-zero exit on failure.

usage: photo_views.py <views|sheet> <args.json>
views: run, landmarker, photos [{name, path, role: front|profile|extra, side: right|left (profile)}], thresholds (optional
       overrides of the gate's limits). Writes <run>/views.json and <run>/views/<name>_outline.npy (rows u, v, outward
       normal u, v; pixels, the edge half a pixel outside the boundary pixels). An extra view is checked for quality only (one
       face, face size, sharpness, clipping, eyes open, mouth closed, plain background); its pose is solved by the fit.
sheet: run, fitted (dense positions .npy), quads (dense corner_vertices .npy), out (png), width (optional, per tile).
       Rows: one per view; photo, clay at the solved camera, photo and clay at 50 % with both outlines.
Limits: the outline assumes a plain background; hair and clothing are part of the outline.
"""
import json
import os
import sys

import cv2
import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import photo_gate as pg
import photo_worker as pw


def outline(mask, step=2, sigma=3.0):
    cs, _ = cv2.findContours(mask.astype(np.uint8), cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_NONE)
    if not cs:
        return None
    c = max(cs, key=len)[:, 0, :].astype(np.float64) + 0.5
    n = len(c)
    k = int(3 * sigma)
    g = np.exp(-0.5 * (np.arange(-k, k + 1) / sigma) ** 2)
    g /= g.sum()
    pad = np.concatenate([c[-k:], c, c[:k]])
    s = np.stack([np.convolve(pad[:, i], g, mode="valid") for i in range(2)], 1)
    t = np.roll(s, -1, 0) - np.roll(s, 1, 0)
    nrm = np.stack([t[:, 1], -t[:, 0]], 1)
    nrm /= np.maximum(np.linalg.norm(nrm, axis=1, keepdims=True), 1e-9)
    h, w = mask.shape
    probe = np.floor(c + 2.5 * nrm).astype(int)
    ok = (probe[:, 0] >= 0) & (probe[:, 0] < w) & (probe[:, 1] >= 0) & (probe[:, 1] < h)
    inside = np.zeros(n, bool)
    inside[ok] = mask[probe[ok, 1], probe[ok, 0]]
    if inside.mean() > 0.5:
        nrm = -nrm
    border = (c[:, 0] < 3) | (c[:, 0] > w - 3) | (c[:, 1] < 3) | (c[:, 1] > h - 3)
    keep = np.arange(0, n, step)
    keep = keep[~border[keep]]
    return np.c_[c[keep] + 0.5 * nrm[keep], nrm[keep]]


def quality(img, face, t, sil):
    pts = face["points"]
    checks = []
    height = float(np.linalg.norm(pts[152, :2] - pts[10, :2]))
    checks.append(pg.check("extra.face_height", round(height, 1), height >= t["front_min_face_px"], f">= {t['front_min_face_px']:g} px",
                           f"the face is {height:.0f} px tall; at least {t['front_min_face_px']:g} px is needed"))
    mask = pg.face_mask(img.shape, pts)
    sharp = pg.sharpness(img, mask)
    checks.append(pg.check("extra.sharpness", sharp and round(sharp, 2), sharp is not None and sharp >= t["front_min_sharpness"],
                           f">= {t['front_min_sharpness']:g}", f"the face is blurred (Laplacian variance {sharp or 0:.1f})"))
    hi, lo = pg.clipping(img, mask)
    checks.append(pg.check("extra.highlights", round(hi, 4), hi <= t["max_clipped_highlights"], f"<= {t['max_clipped_highlights']:g}",
                           f"{100 * hi:.1f} % of the face is clipped white"))
    checks.append(pg.check("extra.shadows", round(lo, 4), lo <= t["max_clipped_shadows"], f"<= {t['max_clipped_shadows']:g}",
                           f"{100 * lo:.1f} % of the face is clipped black"))
    bs = face["blendshapes"]
    blink = max(bs.get("eyeBlinkLeft", 0.0), bs.get("eyeBlinkRight", 0.0))
    checks.append(pg.check("extra.eyes_open", round(blink, 3), blink <= t["max_eye_blink"], f"eye blink <= {t['max_eye_blink']:g}",
                           f"the eyes look closed or narrowed (blink score {blink:.2f})"))
    rel = float(np.linalg.norm(pts[14, :2] - pts[13, :2])) / max(height, 1e-6)
    checks.append(pg.check("extra.mouth_closed", round(rel, 4), rel <= t["max_lip_gap"], f"lip gap <= {t['max_lip_gap']:g} of the face height",
                           f"the mouth looks open ({100 * rel:.1f} % of the face height)"))
    spread, _ = pg.background(img, sil, mask)
    checks.append(pg.check("extra.background", round(spread, 2), spread <= t["max_background_spread"],
                           f"spread <= {t['max_background_spread']:g}", f"the background is not plain (colour spread {spread:.1f})"))
    return checks


def profile_keys(img, side):
    upright = img if side == "right" else cv2.flip(img, 1)
    prof = pw.analyse_profile(upright)
    if prof is None or not prof.get("keys"):
        return None
    w = img.shape[1]
    out = {}
    for name in ("nose_tip", "nasion", "subnasale", "stomion"):
        at = prof["keys"].get(name)
        if at:
            out[name] = [float(at[0]) if side == "right" else float(w - at[0]), float(at[1])]
    return out


def overlay(img, rec, ol, path):
    out = img.copy()
    r = max(1, int(round(max(img.shape[:2]) / 600)))
    if ol is not None:
        for u, v in ol[:, :2].astype(int):
            cv2.circle(out, (int(u), int(v)), r, (0, 0, 255), -1)
    for u, v in rec.get("landmarks") or []:
        cv2.circle(out, (int(round(u)), int(round(v))), r, (0, 220, 0), -1)
    for key in (rec.get("keys") or {}).values():
        if isinstance(key, (list, tuple)) and len(key) >= 2:
            cv2.circle(out, (int(round(key[0])), int(round(key[1]))), 4 * r, (0, 220, 255), max(1, r))
    cv2.imwrite(path, out)
    return path


def views(args):
    run = args["run"]
    os.makedirs(os.path.join(run, "views"), exist_ok=True)
    t = pg.thresholds(args.get("thresholds"))
    det = pg.landmarker(args["landmarker"])
    photos = args.get("photos") or []
    names = [p.get("name") for p in photos]
    if len(set(names)) != len(names) or not all(names):
        pw.fail("every photo needs a unique name", "invalid_input")
    roles = [p.get("role") for p in photos]
    if "front" not in roles or "profile" not in roles:
        pw.fail("at least one front and one profile photo are required", "invalid_input")
    out, failures, warnings = [], [], []
    required = {roles.index("front"), roles.index("profile")}
    for i, p in enumerate(photos):
        if i in required:
            img = pw.read_image(p["path"])
        else:
            img = cv2.imread(p["path"], cv2.IMREAD_COLOR)
            if img is None:
                message = f"{p['name']}: cannot read the image"
                out.append({"name": p["name"], "path": p["path"], "role": p.get("role"), "pass": False, "dropped": True,
                            "warnings": [message]})
                warnings.append(f"{message} (dropped)")
                continue
        h, w = img.shape[:2]
        role = p["role"]
        faces = pg.faces_in(det, img)
        face = faces[0] if len(faces) == 1 else None
        sil = pw.silhouette(img)
        rec = {"name": p["name"], "path": p["path"], "role": role, "size": [w, h], "faces": len(faces),
               "landmarks": face["points"][:, :2].tolist() if face is not None and role != "profile" else None}
        problems = []
        if role == "profile":
            rec["side"] = p.get("side", "right")
            rec["keys"] = profile_keys(img, rec["side"])
            if not rec["keys"] or not all(k in rec["keys"] for k in ("nose_tip", "nasion")):
                problems.append(f"{p['name']}: the profile keys (nose tip, nasion) were not found")
        if sil is None:
            rec["outline"] = None
            problems.append(f"{p['name']}: no outline found")
        else:
            ol = outline(sil)
            path = os.path.join(run, "views", f"{p['name']}_outline.npy")
            np.save(path, ol.astype(np.float64))
            rec["outline"] = path
            rec["outline_points"] = int(len(ol))
        if role == "extra":
            if face is None:
                rec["checks"] = [pg.check("extra.faces", len(faces), False, "== 1", "a face must be found to solve the pose")]
            else:
                rec["checks"] = quality(img, face, t, sil)
            rec["pass"] = all(c["pass"] for c in rec["checks"])
            problems += [f"{p['name']}: {c['name']}: {c['reason']}" for c in rec["checks"] if not c["pass"]]
        elif role == "front" and face is None:
            problems.append(f"{p['name']}: no face found in the front photo")
        rec["overlay"] = overlay(img, rec, ol if sil is not None else None, os.path.join(run, "views", f"{p['name']}_overlay.png"))
        if i in required:
            failures += problems
        elif problems:
            rec["pass"] = False
            rec["dropped"] = True
            rec["warnings"] = problems
            warnings += [f"{m} (dropped)" for m in problems]
        out.append(rec)
    det.close()
    result = {"views": out, "failures": failures, "warnings": warnings, "dropped": [r["name"] for r in out if r.get("dropped")], "pass": not failures}
    pw.write_json(os.path.join(run, "views.json"), result)
    if failures:
        pw.fail("; ".join(failures), "gate_failed", views=out)
    pw.emit({"ok": True, **result})


def camera_of(c):
    r = np.array(c["rotation"], float)
    return r, np.array(c["translation"], float), float(c["focal"]), np.array(c["principal"], float)


def clay(pos, quads, cam, size):
    r, t, f, pp = camera_of(cam)
    cs = pos @ r.T + t
    w, h = size
    u = pp[0] + f * cs[:, 0] / cs[:, 2]
    v = pp[1] + f * cs[:, 1] / cs[:, 2]
    a, b, c, d = (pos[quads[:, i]] for i in range(4))
    fn = np.cross(c - a, d - b)
    vn = np.zeros_like(pos)
    for i in range(4):
        np.add.at(vn, quads[:, i], fn)
    vn /= np.maximum(np.linalg.norm(vn, axis=1, keepdims=True), 1e-12)
    view = -(pos + (r.T @ t))
    view /= np.maximum(np.linalg.norm(view, axis=1, keepdims=True), 1e-12)
    shade = np.clip(np.abs((vn * view).sum(1)), 0, 1)
    ui, vi = np.floor(u).astype(np.int64), np.floor(v).astype(np.int64)
    ok = (ui >= 0) & (ui < w) & (vi >= 0) & (vi < h) & (cs[:, 2] > 0)
    order = np.argsort(-cs[ok, 2])
    img = np.zeros((h, w), np.float32)
    hit = np.zeros((h, w), bool)
    for du in (0, 1):
        for dv in (0, 1):
            uu = np.clip(ui[ok][order] + du, 0, w - 1)
            vv = np.clip(vi[ok][order] + dv, 0, h - 1)
            img[vv, uu] = shade[ok][order]
            hit[vv, uu] = True
    closed = cv2.morphologyEx(hit.astype(np.uint8), cv2.MORPH_CLOSE, np.ones((7, 7), np.uint8)).astype(bool)
    for _ in range(4):
        grown = cv2.dilate(img, np.ones((3, 3), np.uint8))
        hole = closed & ~hit
        img[hole] = grown[hole]
        hit = hit | (hole & (grown > 0))
    g = (40 + 200 * img ** 0.8).astype(np.uint8)
    out = np.full((h, w, 3), 30, np.uint8)
    out[closed] = np.stack([g, g, g], -1)[closed]
    return out, closed


def sheet(args):
    run = args["run"]
    fitted = json.load(open(os.path.join(run, "fit_photos.json"), encoding="utf-8"))
    views_rec = {v["name"]: v for v in json.load(open(os.path.join(run, "views.json"), encoding="utf-8"))["views"]}
    pos = np.load(args["fitted"]).astype(np.float64)
    quads = np.load(args["quads"]).astype(np.int64).reshape(-1, 4)
    tile = int(args.get("width", 420))
    rows = []
    for name, cam in fitted["cameras"].items():
        rec = views_rec[name]
        photo = pw.read_image(rec["path"])
        h, w = photo.shape[:2]
        img, hit = clay(pos, quads, cam, (w, h))
        mix = cv2.addWeighted(photo, 0.5, img, 0.5, 0)
        if rec.get("outline"):
            ol = np.load(rec["outline"])
            for u, v in ol[:, :2].astype(int):
                cv2.circle(mix, (int(u), int(v)), 1, (0, 0, 255), -1)
        cs, _ = cv2.findContours(hit.astype(np.uint8), cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_NONE)
        cv2.drawContours(mix, cs, -1, (0, 255, 0), 1)
        tiles = [cv2.resize(x, (tile, int(round(tile * h / w))), interpolation=cv2.INTER_AREA) for x in (photo, img, mix)]
        stats = fitted.get("views", {}).get(name, {})
        label = f"{name}  outline {stats.get('outline_rms_mm', float('nan')):.2f} mm rms"
        if stats.get("landmark_rms_px") is not None:
            label += f"  landmarks {stats['landmark_rms_px']:.1f} px"
        if stats.get("left_out"):
            label += "  (left out: predicted)"
        pw.put_text(tiles[0], label, (8, 20), 0.45)
        rows.append(np.concatenate(tiles, 1))
    out = np.concatenate(rows, 0)
    cv2.imwrite(args["out"], out)
    pw.emit({"ok": True, "sheet": args["out"], "rows": len(rows), "size": [int(out.shape[1]), int(out.shape[0])]})


def main(argv):
    if len(argv) != 3:
        pw.fail("usage: photo_views.py <views|sheet> <args.json>", "invalid_input")
    with open(argv[2], encoding="utf-8") as fh:
        args = json.load(fh)
    stages = {"views": views, "sheet": sheet}
    if argv[1] not in stages:
        pw.fail(f"unknown stage {argv[1]}", "invalid_input")
    stages[argv[1]](args)


if __name__ == "__main__":
    main(sys.argv)
