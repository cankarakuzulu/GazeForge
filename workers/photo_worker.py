"""Image-to-character worker: one subcommand per stage; one JSON object on stdout, non-zero exit on failure.

usage: photo_worker.py <ingest|landmarks|shell|base|texture|review> <args.json>

Needs numpy, opencv-python and mediapipe (FaceLandmarker task API) and a face landmarker .task model (args "landmarker").
Frames: dense frame is metres, x to the subject's left, -y forward, z up. Photo pixels are (column, row) from the top left.
A profile is always processed upright: facing image right (a left profile is mirrored first and says so).
Limits: cameras are orthographic; the silhouette assumes a plain background; renders are a software z-buffer.
"""
import json
import os
import shutil
import struct
import sys
import time

import cv2
import numpy as np

STABLE = [33, 133, 362, 263, 159, 145, 386, 374, 168, 6, 197, 195, 5, 4, 1, 2, 98, 327, 61, 291, 0, 17, 13, 14, 37, 267, 84, 314, 199, 175]
LABELLED = [1, 2, 13, 14, 0, 17, 61, 291, 152, 168, 199, 175]


def emit(obj, code=0):
    print(json.dumps(obj), flush=True)
    sys.exit(code)


def fail(message, code="worker_failed", **details):
    emit({"ok": False, "error": {"code": code, "message": message, "details": details}}, 1)


def read_image(path):
    img = cv2.imread(path, cv2.IMREAD_COLOR)
    if img is None:
        fail(f"cannot read the image {path}", "not_found")
    return img


def write_json(path, obj):
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(obj, fh, indent=1)


def landmarker(model):
    from mediapipe.tasks import python as mp_python
    from mediapipe.tasks.python import vision
    if not os.path.isfile(model):
        fail(f"the face landmarker model {model} does not exist", "not_found")
    with open(model, "rb") as fh:
        base = mp_python.BaseOptions(model_asset_buffer=fh.read())
    opts = vision.FaceLandmarkerOptions(base_options=base, num_faces=1, min_face_detection_confidence=0.2,
                                        min_face_presence_confidence=0.2)
    return vision.FaceLandmarker.create_from_options(opts)


def detect(det, bgr):
    import mediapipe as mp
    rgb = np.ascontiguousarray(cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB))
    res = det.detect(mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb))
    if not res.face_landmarks:
        return None
    h, w = bgr.shape[:2]
    return np.array([[p.x * w, p.y * h, p.z * w] for p in res.face_landmarks[0]])


def yaw_asymmetry(pts):
    mid = 0.5 * (pts[33, 0] + pts[263, 0])
    return float((pts[1, 0] - mid) / max(abs(pts[263, 0] - pts[33, 0]), 1e-6))


def silhouette(img):
    lab = cv2.cvtColor(img, cv2.COLOR_BGR2LAB).astype(np.float32)
    h, w = lab.shape[:2]
    border = np.concatenate([lab[:8].reshape(-1, 3), lab[:h // 2, :8].reshape(-1, 3), lab[:h // 2, -8:].reshape(-1, 3)])
    bg = np.median(border, axis=0)
    spread = np.percentile(np.linalg.norm(border - bg, axis=1), 99)
    mask = (np.linalg.norm(lab - bg, axis=2) > max(14.0, 2.5 * spread)).astype(np.uint8)
    mask = cv2.morphologyEx(mask, cv2.MORPH_OPEN, np.ones((3, 3), np.uint8))
    mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, np.ones((9, 9), np.uint8))
    count, labels, stats, _ = cv2.connectedComponentsWithStats(mask, 8)
    if count < 2:
        return None
    keep = 1 + int(np.argmax(stats[1:, cv2.CC_STAT_AREA]))
    mask = (labels == keep).astype(np.uint8)
    flood = mask.copy()
    pad = np.zeros((h + 2, w + 2), np.uint8)
    cv2.floodFill(flood, pad, (0, 0), 1)
    return (mask | (1 - flood)).astype(bool)


def edges(mask):
    h = mask.shape[0]
    right = np.full(h, np.nan)
    left = np.full(h, np.nan)
    for v in range(h):
        cols = np.flatnonzero(mask[v])
        if len(cols):
            right[v], left[v] = cols[-1], cols[0]
    return right, left


def head_rows(right, left):
    valid = np.flatnonzero(~np.isnan(right))
    if len(valid) < 50:
        return None
    top = int(valid[0])
    width = right - left
    span = valid[-1] - top
    ref = np.nanmedian(width[top:top + int(0.4 * span)])
    below = np.flatnonzero((np.arange(len(width)) > top + 0.3 * span) & (width > 1.5 * ref))
    bottom = int(below[0]) if len(below) else int(valid[-1])
    return top, bottom


def bump(edge, rows, k, sign):
    best, at = 0.0, None
    for v in rows:
        if v - k < 0 or v + k >= len(edge) or np.isnan(edge[v - k]) or np.isnan(edge[v + k]) or np.isnan(edge[v]):
            continue
        b = min(sign * (edge[v] - edge[v - k]), sign * (edge[v] - edge[v + k]))
        if b > best:
            best, at = b, v
    return best, at


def smooth_edge(edge):
    out = edge.copy()
    good = ~np.isnan(edge)
    idx = np.flatnonzero(good)
    if len(idx) == 0:
        return out
    filled = np.interp(np.arange(len(edge)), idx, edge[idx])
    med = cv2.medianBlur(filled.astype(np.float32).reshape(-1, 1), 5).ravel()
    out[good] = np.convolve(med, np.ones(3) / 3, mode="same")[good]
    return out


def profile_keys(right, top, bottom):
    rows = np.arange(top, bottom)
    r = right
    lower = top + int(0.25 * (bottom - top))
    cand = [v for v in rows if v >= lower and not np.isnan(r[v])]
    if not cand:
        return None
    tip = max(cand, key=lambda v: r[v])
    unit = tip - top

    def arg(lo, hi, fn):
        span = [v for v in range(int(lo), int(hi)) if 0 <= v < len(r) and not np.isnan(r[v])]
        return fn(span, key=lambda v: r[v]) if span else None

    keys = {"nose_tip": tip}
    glabella = arg(tip - 0.40 * unit, tip - 0.15 * unit, max)
    if glabella is not None:
        chord = [v for v in range(glabella + 1, int(tip - 0.05 * unit)) if not np.isnan(r[v])]
        depth = lambda v: r[glabella] + (r[tip] - r[glabella]) * (v - glabella) / (tip - glabella) - r[v]
        keys["nasion"] = max(chord, key=depth) if chord else None
    else:
        keys["nasion"] = None
    keys["subnasale"] = arg(tip + 2, tip + 0.22 * unit, min)
    if keys["subnasale"] is not None:
        keys["upper_lip"] = arg(keys["subnasale"] + 2, keys["subnasale"] + 0.2 * unit, max)
    if keys.get("upper_lip") is not None:
        keys["stomion"] = arg(keys["upper_lip"] + 1, keys["upper_lip"] + 0.12 * unit, min)
    if keys.get("stomion") is not None:
        keys["lower_lip"] = arg(keys["stomion"] + 1, keys["stomion"] + 0.15 * unit, max)
    if keys.get("lower_lip") is not None:
        keys["sulcus"] = arg(keys["lower_lip"] + 2, keys["lower_lip"] + 0.15 * unit, min)
    if keys.get("sulcus") is not None:
        keys["chin"] = arg(keys["sulcus"] + 2, keys["sulcus"] + 0.22 * unit, max)
    if keys.get("chin") is not None:
        c = keys["chin"]
        drop = [v for v in range(c, min(bottom + int(0.5 * unit), len(r))) if not np.isnan(r[v]) and r[v] < r[c] - 0.15 * unit]
        keys["chin_bottom"] = drop[0] if drop else None
    return {k: (None if v is None else [float(r[v]), float(v)]) for k, v in keys.items()}


def analyse_profile(img):
    mask = silhouette(img)
    if mask is None:
        return None
    size = max(5, int(0.012 * img.shape[0])) | 1
    opened = cv2.morphologyEx(mask.astype(np.uint8), cv2.MORPH_OPEN, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (size, size))).astype(bool)
    right, left = edges(opened)
    rows = head_rows(right, left)
    if rows is None:
        return None
    top, bottom = rows
    k = max(4, int(0.04 * (bottom - top)))
    rng = range(top, bottom)
    score_r, at_r = bump(right, rng, k, 1.0)
    score_l, at_l = bump(left, rng, k, -1.0)
    rs = smooth_edge(right)
    first = profile_keys(rs, top, bottom)
    if first:
        tip = int(first["nose_tip"][1])
        cut = int(tip - 0.08 * (tip - top))
        size = max(9, int(0.025 * img.shape[0])) | 1
        seg = rs[top:cut + 2 * size]
        if len(seg) > 10 and not np.isnan(seg).all():
            win = np.ones((size, 1), np.uint8)
            col = np.nan_to_num(seg, nan=float(np.nanmin(seg))).astype(np.float32).reshape(-1, 1)
            opened = cv2.dilate(cv2.erode(col, win), win).ravel()[:cut - top]
            rs[top:cut] = np.where(np.isnan(rs[top:cut]), np.nan, opened)
    return {"mask": mask, "right": rs, "left": left, "top": top, "bottom": bottom, "k": k,
            "bump_right_px": float(score_r), "bump_left_px": float(score_l), "bump_right_row": at_r, "bump_left_row": at_l,
            "keys": profile_keys(rs, top, bottom)}


def similarity2(src, dst):
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


def curve_fit(prof, template):
    p, meta = curve_segment(prof)
    if p is None:
        return None
    q = np.array(template["profile_curve"], float)
    s0 = meta["height"] / template["profile_curve_height"]
    best = None
    for start in (-20.0, -10.0, 0.0, 10.0, 20.0):
        a = np.radians(start)
        r = np.array([[np.cos(a), -np.sin(a)], [np.sin(a), np.cos(a)]])
        s = s0
        t = np.array(meta["nasion"]) - s * r @ np.array(template["profile_curve_nasion"])
        for _ in range(30):
            qt = s * (r @ q.T).T + t
            d = ((p[:, None, :] - qt[None, :, :]) ** 2).sum(2)
            j = d.argmin(1)
            dist = d[np.arange(len(p)), j]
            keep = dist <= np.percentile(dist, 80)
            r, s, t = similarity2(q[j[keep]], p[keep])
        qt = s * (r @ q.T).T + t
        dist = np.sqrt(((p[:, None, :] - qt[None, :, :]) ** 2).sum(2).min(1))
        rms = float(np.sqrt((np.sort(dist)[: int(0.8 * len(dist))] ** 2).mean()))
        if best is None or rms < best[1]:
            best = (float(np.degrees(np.arctan2(r[1, 0], r[0, 0]))), rms, s / s0)
    return {"deg": best[0], "rms_rel": best[1] / meta["height"], "scale_ratio": best[2]}


def profile_template():
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "photo_gate_model.json")
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def profile_facing(upright, prof, side, template):
    r, l = prof["bump_right_px"], prof["bump_left_px"]
    floor = 0.01 * (prof["bottom"] - prof["top"])
    other = "left" if side == "right" else "right"
    if r > 2.0 * l and r > floor:
        return side
    if l > 2.0 * r and l > floor:
        return other
    if template is None:
        return "unknown"
    mirrored = analyse_profile(cv2.flip(upright, 1))
    own = curve_fit(prof, template)
    flip = curve_fit(mirrored, template) if mirrored is not None else None
    a = own["rms_rel"] if own else float("inf")
    b = flip["rms_rel"] if flip else float("inf")
    if a < 0.5 * b:
        return side
    if b < 0.5 * a:
        return other
    return "unknown"


def ingest(args):
    run = args["run"]
    side = args.get("profile_side", "right")
    front_src, profile_src = args["front"], args["profile"]
    front, profile = read_image(front_src), read_image(profile_src)
    front_path, profile_path = os.path.join(run, "front.png"), os.path.join(run, "profile.png")
    cv2.imwrite(front_path, front)
    cv2.imwrite(profile_path, profile)
    upright = profile if side == "right" else cv2.flip(profile, 1)
    upright_path = os.path.join(run, "profile_upright.png")
    cv2.imwrite(upright_path, upright)
    det = landmarker(args["landmarker"])
    failures = []
    pts = detect(det, front)
    yaw = None
    if pts is None:
        failures.append("no face found in the front photo")
    else:
        yaw = yaw_asymmetry(pts)
        if abs(yaw) > 0.08:
            failures.append(f"the front photo is turned (nose offset {yaw:+.3f} of the eye span, limit 0.08)")
    prof = analyse_profile(upright)
    facing = None
    ratio = {}
    if prof is None:
        failures.append("no silhouette found in the profile photo")
    else:
        facing = profile_facing(upright, prof, side, profile_template())
        if facing != side:
            failures.append(f"the profile photo shows the {facing} side, but {side} was declared (mirrored or mislabelled)")
        keys = prof["keys"] or {}
        if pts is not None and facing == side and all(keys.get(k) for k in ("subnasale", "stomion", "chin_bottom")):
            stomion = 0.5 * (pts[13, 1] + pts[14, 1])
            rf = (stomion - pts[2, 1]) / max(pts[152, 1] - pts[2, 1], 1e-6)
            rp = (keys["stomion"][1] - keys["subnasale"][1]) / max(keys["chin_bottom"][1] - keys["subnasale"][1], 1e-6)
            gap = abs(rf - rp) / max(rf, 1e-6)
            limit = args.get("max_ratio_gap")
            ratio = {"measure": "subnasale-to-stomion over subnasale-to-chin-bottom", "front": float(rf), "profile": float(rp),
                     "relative_gap": float(gap), "limit": None if limit is None else float(limit)}
            if limit is not None and gap > float(limit):
                failures.append(f"front and profile disagree on proportions (upper lip over lower face: {rf:.3f} vs {rp:.3f})")
        elif facing == side:
            failures.append("the profile keys (subnasale, stomion, chin bottom) were not all found")
    h, w = front.shape[:2]
    result = {"front": {"source": front_src, "path": front_path, "size": [w, h]},
              "profile": {"source": profile_src, "path": profile_path, "upright_path": upright_path, "side": side,
                          "mirrored_for_processing": side != "right", "size": [upright.shape[1], upright.shape[0]]},
              "facing": {"pass": not failures, "failures": failures, "front_yaw": yaw, "profile_facing": facing,
                         "bump_right_px": prof and prof["bump_right_px"], "bump_left_px": prof and prof["bump_left_px"]},
              "consistency": ratio}
    write_json(os.path.join(run, "ingest.json"), result)
    emit({"ok": True, **result})


def put_text(img, text, xy, scale=0.5, colour=(255, 255, 255)):
    cv2.putText(img, text, xy, cv2.FONT_HERSHEY_SIMPLEX, scale, (0, 0, 0), 3, cv2.LINE_AA)
    cv2.putText(img, text, xy, cv2.FONT_HERSHEY_SIMPLEX, scale, colour, 1, cv2.LINE_AA)


def landmarks(args):
    run = args["run"]
    ing = json.load(open(os.path.join(run, "ingest.json"), encoding="utf-8"))
    det = landmarker(args["landmarker"])
    front = read_image(ing["front"]["path"])
    pts = detect(det, front)
    if pts is None:
        fail("no face found in the front photo")
    small = cv2.resize(front, None, fx=0.85, fy=0.85, interpolation=cv2.INTER_AREA)
    again = detect(det, small)
    agreement = None
    if again is not None:
        d = np.linalg.norm(again[STABLE, :2] / 0.85 - pts[STABLE, :2], axis=1)
        agreement = {"median_px": float(np.median(d)), "max_px": float(d.max()), "worst": int(STABLE[int(np.argmax(d))])}
    ov = front.copy()
    for p in pts:
        cv2.circle(ov, (int(round(p[0])), int(round(p[1]))), 1, (0, 255, 0), -1)
    for i in STABLE:
        cv2.circle(ov, (int(round(pts[i, 0])), int(round(pts[i, 1]))), 3, (0, 0, 255), 1)
    for i in LABELLED:
        put_text(ov, str(i), (int(pts[i, 0]) + 5, int(pts[i, 1]) - 4), 0.4)
    front_overlay = os.path.join(run, "overlay_front.png")
    cv2.imwrite(front_overlay, ov)
    upright = read_image(ing["profile"]["upright_path"])
    prof = analyse_profile(upright)
    if prof is None or prof["keys"] is None:
        fail("the profile silhouette could not be read")
    keys = prof["keys"]
    tip, nasion = keys["nose_tip"], keys.get("nasion")
    unit = tip[1] - prof["top"]
    lo = (nasion[1] if nasion else tip[1]) - 0.5 * unit
    end = keys.get("chin_bottom") or keys.get("chin") or tip
    hi = end[1] + 0.35 * unit
    line = [[float(prof["right"][v]) + 0.5, float(v) + 0.5] for v in range(max(int(lo), 0), min(int(hi), upright.shape[0]))
            if not np.isnan(prof["right"][v])]
    pv = upright.copy()
    contour = (prof["mask"].astype(np.uint8) * 255)
    cs, _ = cv2.findContours(contour, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_NONE)
    cv2.drawContours(pv, cs, -1, (255, 128, 0), 1)
    for u, v in line[::2]:
        cv2.circle(pv, (int(u), int(v)), 1, (0, 255, 0), -1)
    for name, at in keys.items():
        if at:
            cv2.circle(pv, (int(at[0]), int(at[1])), 5, (0, 0, 255), 1)
            put_text(pv, name, (int(at[0]) + 8, int(at[1]) + 4), 0.45)
    profile_overlay = os.path.join(run, "overlay_profile.png")
    cv2.imwrite(profile_overlay, pv)
    h, w = front.shape[:2]
    out = {"front": {"size": [w, h], "points": pts.tolist(), "overlay": front_overlay, "rerun_agreement": agreement},
           "profile": {"size": [upright.shape[1], upright.shape[0]], "line": line, "keys": keys, "overlay": profile_overlay,
                       "head_rows": [prof["top"], prof["bottom"]]},
           "summary": {"front_points": len(pts), "profile_line_rows": len(line), "rerun_agreement": agreement,
                       "profile_keys": sorted(k for k, v in keys.items() if v), "overlays": [front_overlay, profile_overlay]}}
    write_json(os.path.join(run, "landmarks.json"), out)
    emit({"ok": True, "summary": out["summary"]})


def read_glb(path):
    with open(path, "rb") as fh:
        data = fh.read()
    if data[:4] != b"glTF":
        fail(f"{path} is not a binary glTF", "invalid_input")
    off, doc, binary = 12, None, b""
    while off + 8 <= len(data):
        clen, ctype = struct.unpack("<II", data[off:off + 8])
        chunk = data[off + 8:off + 8 + clen]
        if ctype == 0x4E4F534A:
            doc = json.loads(chunk)
        elif ctype == 0x004E4942:
            binary = chunk
        off += 8 + clen
    return doc, binary


def accessor(doc, binary, i):
    a = doc["accessors"][i]
    bv = doc["bufferViews"][a["bufferView"]]
    comps = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}[a["type"]]
    dtype = np.dtype({5120: np.int8, 5121: np.uint8, 5122: np.int16, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}[a["componentType"]])
    start = bv.get("byteOffset", 0) + a.get("byteOffset", 0)
    stride = bv.get("byteStride") or dtype.itemsize * comps
    arr = np.ndarray((a["count"], comps), dtype, buffer=binary, offset=start, strides=(stride, dtype.itemsize))
    return np.array(arr)


def node_matrix(node):
    if "matrix" in node:
        return np.array(node["matrix"], float).reshape(4, 4).T
    t = np.array(node.get("translation", [0, 0, 0]), float)
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    r = np.array([[1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
                  [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
                  [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)]])
    m = np.eye(4)
    m[:3, :3] = r * np.array(node.get("scale", [1, 1, 1]), float)
    m[:3, 3] = t
    return m


def glb_meshes(path, only_lod0=False):
    doc, binary = read_glb(path)
    images = {}
    out = []

    def texture_of(material):
        if material is None:
            return None, np.array([0.8, 0.8, 0.8])
        pbr = doc["materials"][material].get("pbrMetallicRoughness", {})
        factor = np.array(pbr.get("baseColorFactor", [0.8, 0.8, 0.8, 1])[:3])
        tex = pbr.get("baseColorTexture")
        if tex is None:
            return None, factor
        src = doc["textures"][tex["index"]]["source"]
        if src not in images:
            bv = doc["bufferViews"][doc["images"][src]["bufferView"]]
            raw = np.frombuffer(binary, np.uint8, bv["byteLength"], bv.get("byteOffset", 0))
            images[src] = cv2.imdecode(raw, cv2.IMREAD_COLOR)
        return images[src], factor

    def visit(n, parent):
        node = doc["nodes"][n]
        m = parent @ node_matrix(node)
        if "mesh" in node:
            mesh = doc["meshes"][node["mesh"]]
            name = node.get("name", "") + mesh.get("name", "")
            if not only_lod0 or "LOD0" in name or "LOD" not in name:
                for prim in mesh["primitives"]:
                    if prim.get("mode", 4) != 4:
                        continue
                    pos = accessor(doc, binary, prim["attributes"]["POSITION"]).astype(float)
                    pos = pos @ m[:3, :3].T + m[:3, 3]
                    uv = accessor(doc, binary, prim["attributes"]["TEXCOORD_0"]).astype(float) if "TEXCOORD_0" in prim["attributes"] else None
                    idx = accessor(doc, binary, prim["indices"]).reshape(-1, 3).astype(np.int64) if "indices" in prim else np.arange(len(pos)).reshape(-1, 3)
                    nrm = accessor(doc, binary, prim["attributes"]["NORMAL"]).astype(float) @ m[:3, :3].T if "NORMAL" in prim["attributes"] else None
                    img, factor = texture_of(prim.get("material"))
                    out.append({"positions": pos, "uv": uv, "triangles": idx, "image": img, "factor": factor, "name": name, "normals": nrm})
        for c in node.get("children", []):
            visit(c, m)

    scene = doc.get("scenes", [{"nodes": list(range(len(doc["nodes"])))}])[doc.get("scene", 0)]
    for n in scene["nodes"]:
        visit(n, np.eye(4))
    if only_lod0 and out:
        out = out[:1]
    return out


def vertex_normals(pos, tri):
    fn = np.cross(pos[tri[:, 1]] - pos[tri[:, 0]], pos[tri[:, 2]] - pos[tri[:, 0]])
    n = np.zeros_like(pos)
    for k in range(3):
        np.add.at(n, tri[:, k], fn)
    n /= np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-20)
    if (np.einsum("ij,ij->i", n, pos - pos.mean(0))).mean() < 0:
        n = -n
    return n


def sample(image, uv):
    h, w = image.shape[:2]
    x = np.clip((uv[:, 0] % 1.0) * w, 0, w - 1).astype(int)
    y = np.clip((uv[:, 1] % 1.0) * h, 0, h - 1).astype(int)
    return image[y, x]


def splat(u, v, depth, w, h, foot=1):
    us, vs, ds, ids = [], [], [], []
    base = np.arange(len(u))
    for du in range(-foot, foot + 1):
        for dv in range(-foot, foot + 1):
            uu, vv = np.floor(u).astype(int) + du, np.floor(v).astype(int) + dv
            ok = (uu >= 0) & (uu < w) & (vv >= 0) & (vv < h)
            us.append(uu[ok]); vs.append(vv[ok]); ds.append(depth[ok]); ids.append(base[ok])
    pix = np.concatenate(vs) * w + np.concatenate(us)
    d = np.concatenate(ds)
    i = np.concatenate(ids)
    order = np.lexsort((d, pix))
    pix, d, i = pix[order], d[order], i[order]
    first = np.r_[True, pix[1:] != pix[:-1]]
    zbuf = np.full(w * h, np.inf)
    idbuf = np.full(w * h, -1, np.int64)
    zbuf[pix[first]] = d[first]
    idbuf[pix[first]] = i[first]
    return zbuf.reshape(h, w), idbuf.reshape(h, w)


def front_render(points, colours, px_size, foot=1):
    lo, hi = points.min(0), points.max(0)
    w = int(np.ceil((hi[0] - lo[0]) / px_size)) + 1
    h = int(np.ceil((hi[2] - lo[2]) / px_size)) + 1
    u = (points[:, 0] - lo[0]) / px_size
    v = (hi[2] - points[:, 2]) / px_size
    zbuf, ids = splat(u, v, points[:, 1], w, h, foot)
    img = np.full((h, w, 3), 128, np.uint8)
    hit = ids >= 0
    img[hit] = colours[ids[hit]]
    return img, ids, (lo, hi)


def surface_render(p, tri, col, px_size):
    a, b, c = (p[tri[:, k]] for k in range(3))
    weights = [(1, 0, 0), (0, 1, 0), (0, 0, 1), (0.5, 0.5, 0), (0, 0.5, 0.5), (0.5, 0, 0.5), (1 / 3, 1 / 3, 1 / 3),
               (2 / 3, 1 / 6, 1 / 6), (1 / 6, 2 / 3, 1 / 6), (1 / 6, 1 / 6, 2 / 3)]
    pts = np.concatenate([wa * a + wb * b + wc * c for wa, wb, wc in weights])
    owner = np.concatenate([tri[:, int(np.argmax(w))] for w in weights])
    lo, hi = p.min(0), p.max(0)
    w = int(np.ceil((hi[0] - lo[0]) / px_size)) + 1
    h = int(np.ceil((hi[2] - lo[2]) / px_size)) + 1
    zbuf, sid = splat((pts[:, 0] - lo[0]) / px_size, (hi[2] - pts[:, 2]) / px_size, pts[:, 1], w, h, 0)
    ids = np.where(sid >= 0, owner[np.maximum(sid, 0)], -1)
    for _ in range(2):
        empty = ids < 0
        for dy, dx in ((0, 1), (1, 0), (0, -1), (-1, 0)):
            moved = np.roll(np.roll(ids, dy, 0), dx, 1)
            take = empty & (moved >= 0) & (ids < 0)
            ids[take] = moved[take]
    img = np.full((h, w, 3), 128, np.uint8)
    hit = ids >= 0
    img[hit] = col[ids[hit]]
    return img, ids


def lift(ids, pts2d, points):
    out = {}
    h, w = ids.shape
    for i, p in enumerate(pts2d):
        u, v = int(round(p[0])), int(round(p[1]))
        found = None
        for r in range(0, 5):
            win = ids[max(v - r, 0):min(v + r + 1, h), max(u - r, 0):min(u + r + 1, w)]
            good = win[win >= 0]
            if len(good):
                found = int(good[0]) if r == 0 else int(good[len(good) // 2])
                break
        if found is not None:
            out[str(i)] = found
    return out


def directions():
    ds = []
    for x in (-1, 0, 1):
        for y in (-1, 0, 1):
            for z in (-1, 0, 1):
                if (x, y, z) != (0, 0, 0):
                    ds.append(np.array([x, y, z], float) / np.linalg.norm([x, y, z]))
    return ds


def outer_shell(pos, normals):
    diag = np.linalg.norm(pos.max(0) - pos.min(0))
    cell = diag / 500.0
    visible = np.zeros(len(pos), bool)
    for d in directions():
        a = np.cross(d, [0, 0, 1] if abs(d[2]) < 0.9 else [1, 0, 0])
        a /= np.linalg.norm(a)
        b = np.cross(d, a)
        u, v, depth = pos @ a / cell, pos @ b / cell, pos @ d
        u -= u.min(); v -= v.min()
        w, h = int(u.max()) + 2, int(v.max()) + 2
        zbuf, _ = splat(u, v, depth, w, h, 1)
        front = depth <= zbuf[np.clip(v.astype(int), 0, h - 1), np.clip(u.astype(int), 0, w - 1)] + 1.0 * cell
        visible |= front
    return visible


def outer_surface(pos, tri, cells=1000.0, tol=0.5):
    diag = np.linalg.norm(pos.max(0) - pos.min(0))
    cell = diag / cells
    a, b, c = pos[tri[:, 0]], pos[tri[:, 1]], pos[tri[:, 2]]
    cen = (a + b + c) / 3.0
    fn = np.cross(b - a, c - a)
    unit = fn / np.maximum(np.linalg.norm(fn, axis=1, keepdims=True), 1e-30)
    seen = np.zeros(len(tri), bool)
    votes = np.zeros(len(tri))
    for d in directions():
        ax = np.cross(d, [0, 0, 1] if abs(d[2]) < 0.9 else [1, 0, 0])
        ax /= np.linalg.norm(ax)
        bx = np.cross(d, ax)
        u, v, depth = cen @ ax / cell, cen @ bx / cell, cen @ d
        u -= u.min()
        v -= v.min()
        w, h = int(u.max()) + 2, int(v.max()) + 2
        zbuf, _ = splat(u, v, depth, w, h, 1)
        front = depth <= zbuf[v.astype(int), u.astype(int)] + tol * cell
        seen |= front
        votes[front] += np.sign(unit[front] @ (-d))
    return seen, votes, fn


def shell(args):
    run = args["run"]
    started = time.time()
    meshes = glb_meshes(args["glb"])
    if not meshes:
        fail("the mesh file has no triangles", "invalid_input")
    pos, tri, col, nrm = [], [], [], []
    offset = 0
    for m in meshes:
        pos.append(m["positions"])
        nrm.append(m["normals"] if m["normals"] is not None else np.full_like(m["positions"], np.nan))
        tri.append(m["triangles"] + offset)
        if m["image"] is not None and m["uv"] is not None:
            col.append(sample(m["image"], m["uv"]))
        else:
            col.append(np.tile((m["factor"][::-1] * 255).astype(np.uint8), (len(m["positions"]), 1)))
        offset += len(m["positions"])
    pos, tri, col, nrm = np.concatenate(pos), np.concatenate(tri), np.concatenate(col), np.concatenate(nrm)
    own_normals = not np.isnan(nrm).any()
    clay = vertex_normals(pos, tri) if all(m["image"] is None or m["uv"] is None for m in meshes) else None
    det = landmarker(args["landmarker"])
    up = np.array([0.0, 1.0, 0.0])
    best = None
    tried = []
    for name, f in (("+z", [0, 0, 1]), ("-z", [0, 0, -1]), ("+x", [1, 0, 0]), ("-x", [-1, 0, 0])):
        f = np.array(f, float)
        rot = np.array([np.cross(-f, up), -f, up])
        p = pos @ rot.T
        px_size = (p[:, 0].max() - p[:, 0].min()) / 700.0
        if clay is not None:
            col = np.repeat((40 + 200 * np.abs(clay @ rot[1])).astype(np.uint8)[:, None], 3, 1)
        img, ids = surface_render(p, tri, col, px_size)
        pts = detect(det, cv2.medianBlur(np.ascontiguousarray(img[:min(img.shape[:2])]), 3))
        score = None if pts is None else abs(yaw_asymmetry(pts))
        tried.append({"forward": name, "face": pts is not None, "yaw": score})
        if pts is not None and (best is None or score < best[0]):
            best = (score, name, rot, img, ids, pts)
    if best is None:
        fail("no face found on the mesh from any horizontal direction", tried=tried)
    _, name, rot, img, ids, pts = best
    p = pos @ rot.T
    mode = args.get("shell_mode") or "outer_triangles"
    layer = None
    if mode == "visible_vertices":
        if own_normals:
            n = nrm @ rot.T
            n /= np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-20)
        else:
            n = vertex_normals(p, tri)
        visible = outer_shell(p, n)
    else:
        seen, votes, fn = outer_surface(p, tri)
        oriented = fn[seen] * np.where(votes[seen] >= 0, 1.0, -1.0)[:, None]
        n = np.zeros_like(p)
        for k in range(3):
            np.add.at(n, tri[seen, k], oriented)
        length = np.linalg.norm(n, axis=1)
        visible = length > 0
        n /= np.maximum(length, 1e-30)[:, None]
        layer = {"outer_triangles": int(seen.sum()), "flipped_to_face_out": int((votes[seen] < 0).sum()),
                 "undecided": int((votes[seen] == 0).sum())}
    lm = lift(ids, pts, p)
    points = {k: p[v].tolist() for k, v in lm.items()}
    ov = img.copy()
    for q in pts:
        cv2.circle(ov, (int(q[0]), int(q[1])), 1, (0, 255, 0), -1)
    for i in STABLE:
        cv2.circle(ov, (int(pts[i, 0]), int(pts[i, 1])), 3, (0, 0, 255), 1)
    overlay = os.path.join(run, "overlay_shell.png")
    cv2.imwrite(overlay, ov)
    np.save(os.path.join(run, "shell_points.npy"), p[visible].astype(np.float32))
    np.save(os.path.join(run, "shell_normals.npy"), n[visible].astype(np.float32))
    result = {"forward": name, "tried": tried, "shell_mode": mode, "layer": layer,
              "normals": ("file" if own_normals else "computed") if mode == "visible_vertices" else "outer triangles, oriented to the viewers",
              "vertices": int(len(p)), "triangles": int(len(tri)), "outer_vertices": int(visible.sum()),
              "landmarks": len(points), "overlay": overlay, "seconds": time.time() - started}
    write_json(os.path.join(run, "shell_landmarks.json"), {"points": points, "tolerance_mm": 2.0, **result})
    emit({"ok": True, **result})


def base(args):
    run = args["run"]
    p = np.load(os.path.join(run, "base_positions.npy")).astype(float)
    n = np.load(os.path.join(run, "base_normals.npy")).astype(float)
    light = np.array([-0.35, -1.0, 0.45])
    light /= np.linalg.norm(light)
    shade = 0.25 + 0.75 * np.clip(n @ light, 0, 1)
    col = (shade[:, None] * np.array([150, 175, 205])).astype(np.uint8)
    keep = np.flatnonzero(n[:, 1] < 0.3)
    img, ids, _ = front_render(p[keep], col[keep], 0.00025, 1)
    det = landmarker(args["landmarker"])
    pts = detect(det, img)
    if pts is None:
        fail("no face found on the base render")
    lm = lift(ids, pts, p[keep])
    vertices = {k: int(keep[v]) for k, v in lm.items()}
    ov = img.copy()
    for q in pts:
        cv2.circle(ov, (int(q[0]), int(q[1])), 1, (0, 255, 0), -1)
    for i in STABLE:
        cv2.circle(ov, (int(pts[i, 0]), int(pts[i, 1])), 3, (0, 0, 255), 1)
    for i in LABELLED:
        put_text(ov, str(i), (int(pts[i, 0]) + 5, int(pts[i, 1]) - 4), 0.4)
    overlay = os.path.join(run, "overlay_base.png")
    cv2.imwrite(overlay, ov)
    write_json(os.path.join(run, "base_landmarks.json"), {"vertices": vertices, "overlay": overlay})
    emit({"ok": True, "landmarks": len(vertices), "overlay": overlay})


def raster(xy, z, w, h):
    zbuf = np.full((h, w), np.inf)
    tid = np.full((h, w), -1, np.int64)
    b0 = np.zeros((h, w))
    b1 = np.zeros((h, w))
    for t in range(len(xy)):
        (x0, y0), (x1, y1), (x2, y2) = xy[t]
        xmin, xmax = max(int(np.floor(min(x0, x1, x2))), 0), min(int(np.ceil(max(x0, x1, x2))), w - 1)
        ymin, ymax = max(int(np.floor(min(y0, y1, y2))), 0), min(int(np.ceil(max(y0, y1, y2))), h - 1)
        if xmin > xmax or ymin > ymax:
            continue
        det = (y1 - y2) * (x0 - x2) + (x2 - x1) * (y0 - y2)
        if abs(det) < 1e-12:
            continue
        xs, ys = np.meshgrid(np.arange(xmin, xmax + 1) + 0.5, np.arange(ymin, ymax + 1) + 0.5)
        l0 = ((y1 - y2) * (xs - x2) + (x2 - x1) * (ys - y2)) / det
        l1 = ((y2 - y0) * (xs - x2) + (x0 - x2) * (ys - y2)) / det
        l2 = 1 - l0 - l1
        inside = (l0 >= -1e-6) & (l1 >= -1e-6) & (l2 >= -1e-6)
        zz = l0 * z[t, 0] + l1 * z[t, 1] + l2 * z[t, 2]
        sub = zbuf[ymin:ymax + 1, xmin:xmax + 1]
        upd = inside & (zz < sub)
        sub[upd] = zz[upd]
        tid[ymin:ymax + 1, xmin:xmax + 1][upd] = t
        b0[ymin:ymax + 1, xmin:xmax + 1][upd] = l0[upd]
        b1[ymin:ymax + 1, xmin:xmax + 1][upd] = l1[upd]
    return zbuf, tid, b0, b1


def game_mesh(path):
    m = glb_meshes(path, only_lod0=True)[0]
    g = m["positions"]
    pos = np.c_[g[:, 0], -g[:, 2], g[:, 1]]
    return pos, m["triangles"], m["uv"]


class Cameras:
    def __init__(self, run):
        c = json.load(open(os.path.join(run, "cameras.json"), encoding="utf-8"))
        ing = json.load(open(os.path.join(run, "ingest.json"), encoding="utf-8"))
        self.raw = c
        f = c["front"]
        self.kind = f.get("kind", "orthographic")
        if self.kind == "perspective":
            self.rot = np.array(f["rotation"], float)
            self.t = np.array(f["translation"], float)
            self.focal = float(f["focal"])
            self.principal = np.array(f["principal"], float)
            self.forward = self.rot[2]
        else:
            self.scale = f["scale"]
            self.rows = np.array(f["rows"], float)
            self.t = np.array(f["translation"], float)
            self.forward = np.cross(self.rows[0], self.rows[1])
        p = c.get("profile_model_to_photo")
        self.profile = p
        self.side = ing["profile"]["side"]
        self.front_size = c["photo_size"]
        self.profile_size = c["profile_size"]

    def front(self, x):
        if self.kind == "perspective":
            c = x @ self.rot.T + self.t
            return self.focal * c[:, :2] / c[:, 2:3] + self.principal, c[:, 2]
        return self.scale * x @ self.rows.T + self.t, x @ self.forward

    def side_view(self, x):
        p = self.profile
        m = np.c_[-x[:, 1], -x[:, 2]]
        c, s = np.cos(p["angle"]) * p["scale"], np.sin(p["angle"]) * p["scale"]
        uv = np.c_[c * m[:, 0] - s * m[:, 1] + p["translation"][0], s * m[:, 0] + c * m[:, 1] + p["translation"][1]]
        depth = x[:, 0] if self.side == "right" else -x[:, 0]
        return uv, depth

    def side_dir(self):
        return np.array([-1.0, 0.0, 0.0]) if self.side == "right" else np.array([1.0, 0.0, 0.0])


def view_buffers(pos, tri, project, size):
    uv, depth = project(pos)
    zbuf, tid, b0, b1 = raster(uv[tri], depth[tri], size[0], size[1])
    return zbuf


def visible(zbuf, uv, depth, tol):
    h, w = zbuf.shape
    x = np.clip(uv[:, 0].astype(int), 0, w - 1)
    y = np.clip(uv[:, 1].astype(int), 0, h - 1)
    inside = (uv[:, 0] >= 0) & (uv[:, 0] < w) & (uv[:, 1] >= 0) & (uv[:, 1] < h)
    return inside & (depth <= zbuf[y, x] + tol)


def texture(args):
    run = args["run"]
    started = time.time()
    size = int(args.get("size", 2048))
    pos, tri, uvs = game_mesh(args["glb"])
    if uvs is None:
        fail("the character mesh has no UVs", "invalid_input")
    n = vertex_normals(pos, tri)
    cams = Cameras(run)
    ing = json.load(open(os.path.join(run, "ingest.json"), encoding="utf-8"))
    front = read_image(args.get("front_albedo") or ing["front"]["path"]).astype(float)
    side = read_image(args.get("profile_albedo") or ing["profile"]["upright_path"]).astype(float)
    zf = view_buffers(pos, tri, cams.front, cams.front_size)
    zp = view_buffers(pos, tri, cams.side_view, cams.profile_size) if cams.profile else None
    _, tid, b0, b1 = raster(uvs[tri] * size, np.zeros((len(tri), 3)), size, size)
    hit = tid >= 0
    t = tid[hit]
    l0, l1 = b0[hit][:, None], b1[hit][:, None]
    l2 = 1 - l0 - l1
    x = l0 * pos[tri[t, 0]] + l1 * pos[tri[t, 1]] + l2 * pos[tri[t, 2]]
    nn = l0 * n[tri[t, 0]] + l1 * n[tri[t, 1]] + l2 * n[tri[t, 2]]
    nn /= np.maximum(np.linalg.norm(nn, axis=1, keepdims=True), 1e-12)
    acc = np.zeros((len(x), 3))
    wsum = np.zeros(len(x))
    fuv, fd = cams.front(x)
    wf = np.clip(nn @ (-cams.forward), 0, 1) ** 2 * visible(zf, fuv, fd, 0.002)
    acc += wf[:, None] * front[np.clip(fuv[:, 1].astype(int), 0, front.shape[0] - 1), np.clip(fuv[:, 0].astype(int), 0, front.shape[1] - 1)]
    wsum += wf
    if zp is not None:
        puv, pd = cams.side_view(x)
        wp = np.clip(nn @ cams.side_dir(), 0, 1) ** 2 * visible(zp, puv, pd, 0.002)
        acc += wp[:, None] * side[np.clip(puv[:, 1].astype(int), 0, side.shape[0] - 1), np.clip(puv[:, 0].astype(int), 0, side.shape[1] - 1)]
        wsum += wp
    covered = wsum > 1e-3
    colour = np.zeros((len(x), 3))
    colour[covered] = acc[covered] / wsum[covered, None]
    fill = np.average(colour[covered], axis=0, weights=wsum[covered]) if covered.any() else np.array([128, 128, 128])
    colour[~covered] = fill
    tex = np.tile(fill, (size, size, 1))
    tex[hit] = colour
    tex = np.clip(tex, 0, 255).astype(np.uint8)
    path = os.path.join(run, "texture.png")
    cv2.imwrite(path, tex)
    emit({"ok": True, "texture": path, "size": size, "texels_on_mesh": int(hit.sum()), "covered_fraction": float(covered.mean()),
          "front_rms_px": cams.raw.get("front_rms_px"), "profile_rms_px": cams.raw.get("profile_rms_px"),
          "delit": bool(args.get("front_albedo")), "seconds": time.time() - started})


def render(pos, tri, uvs, n, tex, project, size, view_dir):
    uv, depth = project(pos)
    zbuf, tid, b0, b1 = raster(uv[tri], depth[tri], size[0], size[1])
    hit = tid >= 0
    t = tid[hit]
    l0, l1 = b0[hit][:, None], b1[hit][:, None]
    l2 = 1 - l0 - l1
    out_tex = np.full((size[1], size[0], 3), 110, np.uint8)
    out_clay = out_tex.copy()
    tuv = l0 * uvs[tri[t, 0]] + l1 * uvs[tri[t, 1]] + l2 * uvs[tri[t, 2]]
    out_tex[hit] = sample(tex, tuv)
    nn = l0 * n[tri[t, 0]] + l1 * n[tri[t, 1]] + l2 * n[tri[t, 2]]
    nn /= np.maximum(np.linalg.norm(nn, axis=1, keepdims=True), 1e-12)
    light = -view_dir + np.array([0.0, 0.0, 0.6]) + np.array([-0.3, 0.0, 0.0])
    light /= np.linalg.norm(light)
    shade = 0.2 + 0.8 * np.clip(nn @ light, 0, 1)
    out_clay[hit] = (shade[:, None] * np.array([175, 190, 215])).astype(np.uint8)
    return out_tex, out_clay, hit


def review(args):
    run = args["run"]
    started = time.time()
    pos, tri, uvs = game_mesh(args["glb"])
    n = vertex_normals(pos, tri)
    tex = read_image(args["texture"])
    cams = Cameras(run)
    ing = json.load(open(os.path.join(run, "ingest.json"), encoding="utf-8"))
    rows = []
    views = [("front", read_image(ing["front"]["path"]), cams.front, cams.front_size, cams.forward)]
    if cams.profile:
        views.append(("profile", read_image(ing["profile"]["upright_path"]), cams.side_view, cams.profile_size, -cams.side_dir()))
    panel = 560
    for name, photo, project, size, view_dir in views:
        rt, rc, hit = render(pos, tri, uvs, n, tex, project, size, view_dir)
        blend = photo.copy()
        blend[hit] = (0.5 * photo[hit] + 0.5 * rc[hit]).astype(np.uint8)
        ys, xs = np.nonzero(hit)
        cy, cx = int(ys.mean()), int(xs.mean())
        half = int(0.75 * max(ys.max() - ys.min(), xs.max() - xs.min()) / 2) + 20
        def crop(im):
            pad = cv2.copyMakeBorder(im, half, half, half, half, cv2.BORDER_CONSTANT, value=(110, 110, 110))
            c = pad[cy:cy + 2 * half, cx:cx + 2 * half]
            return cv2.resize(c, (panel, panel), interpolation=cv2.INTER_AREA)
        tiles = [crop(photo), crop(rt), crop(rc), crop(blend)]
        for tile, label in zip(tiles, (f"photo ({name})", "result, photo-projected", "result, clay", "photo + clay 50%")):
            put_text(tile, label, (10, 24), 0.6)
        rows.append(np.hstack(tiles))
    sheet = np.vstack(rows)
    head = np.full((44, sheet.shape[1], 3), 40, np.uint8)
    put_text(head, f"image to character review - front rms {cams.raw.get('front_rms_px', float('nan')):.2f} px, "
                   f"profile rms {cams.raw.get('profile_rms_px', float('nan')):.2f} px", (10, 30), 0.7)
    sheet = np.vstack([head, sheet])
    out = args["out"]
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    cv2.imwrite(out, sheet)
    if os.path.abspath(out) != os.path.abspath(os.path.join(run, "review_sheet.png")):
        shutil.copyfile(out, os.path.join(run, "review_sheet.png"))
    emit({"ok": True, "sheet": out, "size": [sheet.shape[1], sheet.shape[0]], "seconds": time.time() - started})


def main(argv):
    if len(argv) != 3:
        fail("usage: photo_worker.py <ingest|landmarks|shell|base|texture|review> <args.json>", "invalid_input")
    with open(argv[2], encoding="utf-8") as fh:
        args = json.load(fh)
    stages = {"ingest": ingest, "landmarks": landmarks, "shell": shell, "base": base, "texture": texture, "review": review}
    if argv[1] not in stages:
        fail(f"unknown stage {argv[1]}", "invalid_input")
    stages[argv[1]](args)


if __name__ == "__main__":
    main(sys.argv)
