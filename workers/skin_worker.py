"""Skin projection from photos onto a game base's texels; one JSON object on stdout.

usage: skin_worker.py project <args.json>
args: run, views (views.json), cameras (fit_photos.json: perspective cameras by view name), dir (the arrays skin.project
      wrote: texel_tri.npy, texel_bary.npy, positions.npy, normals.npy, triangles.npy, regions.npy), brows (remove|keep).
writes dir/projected.npy (N x 3, linear RGB, delit) and dir/confidence.npy (N x 1), N = size * size.

Per view: each covered texel's surface point goes through the view's camera; it counts when the fitted mesh's z-buffer
shows it, weighted by how squarely it faces the camera and by its distance from the mesh's outline in the photo. Eyes and
brows are masked by the front photo's landmark polygons (an eye's pixels belong to the eyeball, not the lid), the scalp and
the inside of the mouth by their skin regions. Shading is a 9-term spherical-harmonics fit of luminance against the surface
normal on plain skin; dividing by it delights the colour (cast shadows and ambient occlusion are not modelled). Each view is
gain-matched to the front on their overlap, then the views are blended with cubed weights.
"""
import json
import os
import sys
import time

import cv2
import numpy as np

REGIONS = ["lips", "mouth_inner", "nose", "cheeks", "forehead", "chin", "upper_lip", "beard", "under_eye", "eyelids", "ears",
           "scalp", "neck", "t_zone"]
R = {n: i for i, n in enumerate(REGIONS)}
EYES = [[33, 7, 163, 144, 145, 153, 154, 155, 133, 173, 157, 158, 159, 160, 161, 246],
        [263, 249, 390, 373, 374, 380, 381, 382, 362, 398, 384, 385, 386, 387, 388, 466]]
BROWS = [[70, 63, 105, 66, 107, 55, 65, 52, 53, 46], [300, 293, 334, 296, 336, 285, 295, 282, 283, 276]]


def emit(obj, code=0):
    print(json.dumps(obj))
    sys.exit(code)


def fail(message, code="worker_failed"):
    emit({"ok": False, "error": {"code": code, "message": message}}, 1)


def lin(c):
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def raster_depth(xy, z, w, h):
    zbuf = np.full((h, w), np.inf)
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
    return zbuf


def sh9(n):
    x, y, z = n[:, 0], n[:, 1], n[:, 2]
    return np.stack([np.ones_like(x), x, y, z, x * y, y * z, x * z, x * x - y * y, 3 * z * z - 1], 1)


def smoothstep(e0, e1, x):
    t = np.clip((x - e0) / (e1 - e0), 0, 1)
    return t * t * (3 - 2 * t)


class Camera:
    def __init__(self, c):
        self.rot = np.array(c["rotation"], float)
        self.t = np.array(c["translation"], float)
        self.f = float(c["focal"])
        self.pp = np.array(c["principal"], float)
        self.centre = np.array(c["centre"], float)

    def project(self, x):
        c = x @ self.rot.T + self.t
        return self.f * c[:, :2] / c[:, 2:3] + self.pp, c[:, 2]


def sample(img, uv):
    n, cols = len(uv), 4096
    rows = -(-n // cols)
    m = np.zeros((rows * cols, 2), np.float32)
    m[:n] = uv - 0.5
    out = cv2.remap(img, m[:, 0].reshape(rows, cols), m[:, 1].reshape(rows, cols), cv2.INTER_LINEAR, borderMode=cv2.BORDER_REPLICATE)
    return out.reshape(rows * cols, -1)[:n]


def foreground(bgr):
    h, w = bgr.shape[:2]
    lab = cv2.cvtColor(bgr, cv2.COLOR_BGR2LAB).astype(np.float32)
    border = np.concatenate([lab[:8].reshape(-1, 3), lab[:, :8].reshape(-1, 3), lab[:, -8:].reshape(-1, 3)])
    ref = np.median(border, axis=0)
    spread = np.median(np.abs(border - ref), axis=0).sum() + 1.0
    d = np.abs(lab - ref).sum(axis=2)
    fg = (d > max(12.0, 6.0 * spread)).astype(np.uint8)
    fg = cv2.morphologyEx(fg, cv2.MORPH_CLOSE, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (9, 9)))
    n, labels, stats, _ = cv2.connectedComponentsWithStats(1 - fg, 4)
    touching = set(np.unique(np.concatenate([labels[0], labels[-1], labels[:, 0], labels[:, -1]]))) - {0} if n > 1 else set()
    background = np.isin(labels, list(touching)) & (fg == 0)
    return cv2.erode((~background).astype(np.uint8), np.ones((5, 5), np.uint8)) > 0


def polygon_mask(shape, polys, dilate):
    mask = np.zeros(shape, np.uint8)
    for p in polys:
        cv2.fillPoly(mask, [np.round(p).astype(np.int32)], 255)
    if dilate > 0:
        mask = cv2.dilate(mask, cv2.getStructuringElement(cv2.MORPH_ELLIPSE, (2 * dilate + 1, 2 * dilate + 1)))
    return mask > 0


def project(args):
    started = time.time()
    d = args["dir"]
    tri_of = np.load(os.path.join(d, "texel_tri.npy")).reshape(-1)
    bary = np.load(os.path.join(d, "texel_bary.npy")).astype(np.float64)
    pos = np.load(os.path.join(d, "positions.npy")).astype(np.float64)
    nrm = np.load(os.path.join(d, "normals.npy")).astype(np.float64)
    tris = np.load(os.path.join(d, "triangles.npy")).astype(np.int64)
    regions = np.load(os.path.join(d, "regions.npy")).astype(np.float32)
    n_tex = len(tri_of)
    cov = np.flatnonzero(tri_of >= 0)
    t = tris[tri_of[cov]]
    b = np.c_[bary[cov], 1 - bary[cov].sum(1)]
    P = np.einsum("nk,nkj->nj", b, pos[t])
    N = np.einsum("nk,nkj->nj", b, nrm[t])
    N /= np.maximum(np.linalg.norm(N, axis=1, keepdims=True), 1e-12)
    W = np.einsum("nk,nkj->nj", b.astype(np.float32), regions[t])
    views = json.load(open(args["views"], encoding="utf-8"))["views"]
    cams = json.load(open(args["cameras"], encoding="utf-8"))["cameras"]
    images, zbufs, uvs, depths, masks = {}, {}, {}, {}, {}
    for v in views:
        name = v["name"]
        if name not in cams:
            continue
        cam = Camera(cams[name])
        img = cv2.imread(v["path"], cv2.IMREAD_COLOR)
        if img is None:
            fail(f"cannot read {v['path']}", "invalid_input")
        h, w = img.shape[:2]
        fg = foreground(img)
        pv, dv = cam.project(pos)
        zbufs[name] = raster_depth(pv[tris], dv[tris], w, h)
        uvs[name], depths[name] = cam.project(P)
        images[name] = (cam, lin(img[:, :, ::-1].astype(np.float32) / 255.0), v)
        inside = np.isfinite(zbufs[name])
        dist = cv2.distanceTransform(inside.astype(np.uint8), cv2.DIST_L2, 5)
        head = inside & (np.arange(h)[:, None] < np.percentile(np.nonzero(inside)[0], 70))
        masks[name] = (dist, fg, float((head & ~fg).sum() / max(head.sum(), 1)))
    front = next((v for v in views if v.get("role") == "front"), None)
    excluded = np.zeros(len(cov), bool)
    if front is not None and front["name"] in images and len(front.get("landmarks") or []) >= 468:
        lm = np.array(front["landmarks"], float)
        cam, img, _ = images[front["name"]]
        mask = polygon_mask(img.shape[:2], [lm[e] for e in EYES], 7)
        if args.get("brows", "remove") == "remove":
            mask |= polygon_mask(img.shape[:2], [lm[e] for e in BROWS], 9)
        uv = uvs[front["name"]]
        facing = np.einsum("nj,nj->n", N, cam.centre[None, :] - P)
        h, w = mask.shape
        x = np.clip(uv[:, 0].astype(int), 0, w - 1)
        y = np.clip(uv[:, 1].astype(int), 0, h - 1)
        excluded = mask[y, x] & (facing > 0)
    skin = (1 - W[:, R["scalp"]]) * (1 - W[:, R["mouth_inner"]]) * (~excluded)
    core = ((W[:, R["forehead"]] > 0.5) | (W[:, R["cheeks"]] > 0.5) | (W[:, R["nose"]] > 0.5) | (W[:, R["t_zone"]] > 0.5)) & (W[:, R["beard"]] < 0.3) & ~excluded
    plain = (W[:, R["lips"]] < 0.1) & (W[:, R["eyelids"]] < 0.3) & (W[:, R["under_eye"]] < 0.5) & (W[:, R["beard"]] < 0.5) & (W[:, R["scalp"]] < 0.2) & ~excluded
    albedo, weight, report = {}, {}, {}
    max_outside = float(args.get("max_outside", 0.04))
    for name, (cam, img, v) in images.items():
        h, w = img.shape[:2]
        dist, fg, outside = masks[name]
        if outside > max_outside:
            report[name] = {"used": False, "outside_fraction": round(outside, 4),
                            "reason": f"{100 * outside:.1f} % of the fitted head projects onto the background in this photo (limit {100 * max_outside:g} %): its camera does not register the mesh"}
            continue
        uv, dep = uvs[name], depths[name]
        x = np.clip(uv[:, 0].astype(int), 0, w - 1)
        y = np.clip(uv[:, 1].astype(int), 0, h - 1)
        inside = (uv[:, 0] >= 0) & (uv[:, 0] < w) & (uv[:, 1] >= 0) & (uv[:, 1] < h)
        visible = inside & (dep <= zbufs[name][y, x] + 0.0015) & fg[y, x]
        to_cam = cam.centre[None, :] - P
        to_cam /= np.maximum(np.linalg.norm(to_cam, axis=1, keepdims=True), 1e-12)
        facing = np.clip(np.einsum("nj,nj->n", N, to_cam), 0, 1)
        edge = smoothstep(2.0, 18.0, dist[y, x])
        wv = visible * facing ** 2 * smoothstep(0.15, 0.35, facing) * edge * skin
        rgb = sample(img, uv)
        wv = wv * (rgb[:, 2] <= 1.05 * rgb[:, 0])
        Y = rgb @ np.array([0.2126, 0.7152, 0.0722])
        fit = (wv > 0.3) & core
        if fit.sum() < 200:
            report[name] = {"visible": int(visible.sum()), "used": False, "reason": "fewer than 200 plain skin texels seen squarely"}
            continue
        B = sh9(N)
        if fit.sum() < 5000:
            B = B[:, :4]
        sel = np.flatnonzero(fit)
        coef = None
        for _ in range(3):
            A = B[sel] * wv[sel, None]
            ridge = 1e-3 * np.trace(A.T @ A) / A.shape[1]
            coef = np.linalg.solve(A.T @ A + ridge * np.eye(A.shape[1]), A.T @ (Y[sel] * wv[sel]))
            res = Y[sel] / np.maximum(B[sel] @ coef, 1e-6)
            keep = np.abs(res - np.median(res)) < 2.5 * np.median(np.abs(res - np.median(res))) + 1e-6
            sel = sel[keep]
        shade = B @ coef
        shade = np.clip(shade / np.median(shade[fit]), 0.25, 3.0)
        alb = rgb / shade[:, None]
        sat_t = (rgb.max(1) - rgb.min(1)) / np.maximum(rgb.max(1), 1e-6)
        y_ref, s_ref = np.median(Y[fit]), np.median(sat_t[fit])
        y_img = img @ np.array([0.2126, 0.7152, 0.0722], np.float32)
        s_img = (img.max(2) - img.min(2)) / np.maximum(img.max(2), 1e-6)
        hair_px = (((y_img < 0.4 * y_ref) | (s_img < 0.4 * s_ref)) & fg).astype(np.uint8)
        hair_px = cv2.morphologyEx(hair_px, cv2.MORPH_OPEN, np.ones((3, 3), np.uint8))
        away = cv2.distanceTransform(1 - hair_px, cv2.DIST_L2, 5)
        clear = smoothstep(3.0, 12.0, away[y, x])
        exempt = (W[:, R["beard"]] >= 0.5) | (W[:, R["lips"]] >= 0.3)
        wv = wv * np.where(exempt, 1.0, clear)
        report_hair = int(((clear < 0.5) & ~exempt & visible).sum())
        ratio = Y / np.maximum(shade * np.median(Y[fit] / shade[fit]), 1e-6)
        wv = wv * np.exp(-np.maximum(0.0, ratio - 1.25) / 0.15)
        albedo[name], weight[name] = alb, wv
        pred = shade * np.median(Y[fit] / shade[fit])
        report[name] = {"visible": int(visible.sum()), "used": True, "outside_fraction": round(outside, 4),
                        "fit_texels": int(len(sel)), "hair_texels": report_hair, "sh": [round(float(c), 5) for c in coef],
                        "shading_rel_err_median": round(float(np.median(np.abs(Y[fit] - pred[fit]) / np.maximum(pred[fit], 1e-6))), 4)}
    if not albedo:
        fail("no view saw enough plain skin to project", "invalid_input")
    ref = front["name"] if front is not None and front["name"] in albedo else max(albedo, key=lambda k: weight[k].sum())
    for name in albedo:
        if name == ref:
            report[name]["gain"] = [1.0, 1.0, 1.0]
            continue
        both = (weight[ref] > 0.15) & (weight[name] > 0.15)
        if both.sum() < 500:
            report[name]["gain"] = [1.0, 1.0, 1.0]
            report[name]["gain_note"] = f"only {int(both.sum())} overlapping texels with {ref}; not matched"
            continue
        g = np.median(albedo[ref][both] / np.maximum(albedo[name][both], 1e-6), axis=0)
        albedo[name] = albedo[name] * g[None, :]
        report[name]["gain"] = [round(float(x), 4) for x in g]
        report[name]["overlap"] = int(both.sum())
    num = np.zeros((len(cov), 3))
    den = np.zeros(len(cov))
    for name in albedo:
        s = weight[name] ** 3
        num += s[:, None] * albedo[name]
        den += s
    blend = np.where(den[:, None] > 1e-12, num / np.maximum(den, 1e-12)[:, None], 0.0)
    strongest = np.max(np.stack([weight[k] for k in albedo]), axis=0)
    conf = smoothstep(0.02, 0.25, strongest)
    for name in albedo:
        others = [k for k in albedo if k != name]
        if not others:
            continue
        on = sum(weight[k] ** 3 for k in others)
        rest = sum((weight[k] ** 3)[:, None] * albedo[k] for k in others) / np.maximum(on, 1e-12)[:, None]
        both = (weight[name] > 0.3) & (np.max(np.stack([weight[k] for k in others]), axis=0) > 0.3) & plain
        if both.sum() >= 200:
            report[name]["disagreement_with_others_median"] = round(float(np.median(np.abs(albedo[name][both] - rest[both]).sum(1) / np.maximum(rest[both].sum(1), 1e-6))), 4)
            report[name]["disagreement_texels"] = int(both.sum())
    out_rgb = np.zeros((n_tex, 3), np.float32)
    out_rgb[cov] = blend
    out_conf = np.zeros((n_tex, 1), np.float32)
    out_conf[cov, 0] = conf
    np.save(os.path.join(d, "projected.npy"), out_rgb)
    np.save(os.path.join(d, "confidence.npy"), out_conf)
    emit({"ok": True, "reference": ref, "views": report, "covered_texels": int(len(cov)), "confident_fraction": float((conf > 0.5).mean()),
          "excluded_texels": int(excluded.sum()), "seconds": time.time() - started})


def main(argv):
    if len(argv) != 3 or argv[1] != "project":
        fail("usage: skin_worker.py project <args.json>", "invalid_input")
    project(json.load(open(argv[2], encoding="utf-8")))


if __name__ == "__main__":
    main(sys.argv)
