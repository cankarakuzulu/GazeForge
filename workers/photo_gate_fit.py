"""Per-person head shape for the reference-photo gate's front pose.

The head targets (photo_gate_shape.npz: their deltas on 478 landmark vertices and on profile-edge vertices of the base) are
fitted to this person's photos with weights bounded to [0, 1]: front landmarks by perspective reprojection (PnP pose), and the
profile's keys (nasion, nose tip, lips, chin) by a similarity in the side plane against the vertices that carry those keys on
the base's own silhouette. The profile supplies the depth that a single front photo cannot separate from pitch. The front pose
is then PnP against the fitted head's landmarks. Pose and shape alternate for a few rounds.

Frames: metres, x to the subject's left, -y forward, z up. Head rotation R = Rz(yaw) Rx(-pitch) Ry(roll).
"""
import math
import os

import cv2
import numpy as np

import photo_worker as pw

SHAPE_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "photo_gate_shape.npz")
R0_CV = np.array([[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]])
SIDE = np.array([[0.0, -1.0, 0.0], [0.0, 0.0, 1.0]])
FIT_IDS = [33, 133, 362, 263, 159, 145, 386, 374, 168, 6, 197, 195, 5, 4, 1, 2, 98, 327, 61, 291, 0, 17, 37, 267, 84, 314, 199,
           175]
PROFILE_KEYS = ["nasion", "nose_tip", "subnasale", "upper_lip", "stomion", "lower_lip", "sulcus", "chin"]
STEP = 0.0005


def angles(r):
    b = math.asin(max(-1.0, min(1.0, r[2, 1])))
    return {"yaw": math.degrees(math.atan2(-r[0, 1], r[1, 1])), "pitch": -math.degrees(b),
            "roll": math.degrees(math.atan2(-r[2, 0], r[2, 2]))}


def side_keys(x):
    q = x @ SIDE.T
    top = float(q[:, 1].max())
    b = np.floor((top - q[:, 1]) / STEP).astype(np.int64)
    order = np.lexsort((-q[:, 0], b))
    first = np.ones(len(order), bool)
    first[1:] = b[order][1:] != b[order][:-1]
    rows = order[first]
    n = int(b.max()) + 1
    raster = np.full(n, np.nan)
    raster[b[rows]] = q[rows, 0]
    good = np.flatnonzero(~np.isnan(raster))
    keys = pw.profile_keys(np.interp(np.arange(n), good, raster[good]), 0, n) or {}
    out = {}
    for name in PROFILE_KEYS:
        k = keys.get(name)
        if k:
            p = np.array([k[0], top - (k[1] + 0.5) * STEP])
            out[name] = int(rows[np.argmin(((q[rows] - p) ** 2).sum(1))])
    return out


class Shape:
    def __init__(self, path=SHAPE_PATH):
        d = np.load(path)
        self.deltas = d["deltas"].astype(np.float64)
        self.base = d["base"].astype(np.float64)
        self.row = {int(i): k for k, i in enumerate(d["landmarks"])}
        self.sym = d["sym"].astype(np.int64)
        self.t = len(self.deltas)
        if "key_rows" in d:
            self.key_rows = {str(n): int(r) for n, r in zip(d["key_names"], d["key_rows"])}
        else:
            self.key_rows = side_keys(self.base)

    def points(self, w):
        return self.base + np.tensordot(w, self.deltas, 1)


def bounded_lsq(g, r, w0, iterations=300):
    v = np.ones(len(g))
    for _ in range(30):
        v = g @ v
        v /= np.linalg.norm(v) or 1.0
    lip = 1.05 * float(v @ g @ v) or 1.0
    w = w0.copy()
    y, tk = w.copy(), 1.0
    for _ in range(iterations):
        nxt = np.clip(y - (g @ y - r) / lip, 0.0, 1.0)
        tn = 0.5 * (1 + math.sqrt(1 + 4 * tk * tk))
        y = nxt + (tk - 1) / tn * (nxt - w)
        w, tk = nxt, tn
    return w


def camera(size, focal_ratio):
    w, h = size
    f = focal_ratio * max(w, h)
    return np.array([[f, 0.0, w / 2], [0.0, f, h / 2], [0.0, 0.0, 1.0]])


def pnp(obj, img, k, guess=None):
    if guess is None:
        c = obj.mean(0) + np.array([0.0, -0.85, 0.0])
        rvec, _ = cv2.Rodrigues(R0_CV)
        tvec = (-R0_CV @ c).reshape(3, 1)
    else:
        rvec, tvec = guess
    ok, rvec, tvec = cv2.solvePnP(obj, img.astype(np.float64), k, None, rvec.copy(), tvec.copy(), useExtrinsicGuess=True,
                                  flags=cv2.SOLVEPNP_ITERATIVE)
    if not ok:
        return None
    proj, _ = cv2.projectPoints(obj, rvec, tvec, k, None)
    rms = float(np.sqrt(((proj.reshape(-1, 2) - img) ** 2).sum(1).mean()))
    r, _ = cv2.Rodrigues(rvec)
    return {"r": r, "t": tvec.ravel(), "rvec": rvec, "tvec": tvec, "rms": rms}


def similarity(src, dst):
    ms, md = src.mean(0), dst.mean(0)
    a, b = src - ms, dst - md
    u, sv, vt = np.linalg.svd(b.T @ a / len(a))
    d = np.diag([1.0, np.sign(np.linalg.det(u @ vt))])
    r = u @ d @ vt
    s = (sv * np.diag(d)).sum() / max((a ** 2).sum() / len(a), 1e-12)
    return r, s, md - s * r @ ms


def photo_keys(prof):
    keys = (prof or {}).get("keys") or {}
    return {k: np.array([v[0], -v[1]]) for k, v in keys.items() if k in PROFILE_KEYS and v}


def rot(yaw, pitch, roll):
    a, b, c = math.radians(yaw), math.radians(-pitch), math.radians(roll)
    rz = np.array([[math.cos(a), -math.sin(a), 0], [math.sin(a), math.cos(a), 0], [0, 0, 1]])
    rx = np.array([[1, 0, 0], [0, math.cos(b), -math.sin(b)], [0, math.sin(b), math.cos(b)]])
    ry = np.array([[math.cos(c), 0, math.sin(c)], [0, 1, 0], [-math.sin(c), 0, math.cos(c)]])
    return rz @ rx @ ry


def side_edge(x, r):
    q = x @ r.T @ SIDE.T
    b = np.floor(q[:, 1] / STEP).astype(np.int64)
    order = np.lexsort((-q[:, 0], b))
    first = np.ones(len(order), bool)
    first[1:] = b[order][1:] != b[order][:-1]
    e = q[order[first]]
    e = e[np.argsort(-e[:, 1])]
    tan = np.gradient(e, axis=0)
    tan /= np.maximum(np.linalg.norm(tan, axis=1, keepdims=True), 1e-12)
    nrm = np.c_[-tan[:, 1], tan[:, 0]]
    return e, nrm * np.where(nrm[:, 0:1] < 0, -1.0, 1.0)


def edge_rms(edge, keys, x, key_rows, r, iterations=10, trim=0.8):
    mk = x[key_rows] @ r.T @ SIDE.T
    r2, s, t = similarity(keys, mk)
    th = math.atan2(r2[1, 0], r2[0, 0])
    q, nq = side_edge(x, r)
    for it in range(iterations + 1):
        c, sn = math.cos(th), math.sin(th)
        m = s * edge @ np.array([[c, -sn], [sn, c]]).T + t
        j = ((m ** 2).sum(1)[:, None] - 2 * m @ q.T + (q ** 2).sum(1)[None, :]).argmin(1)
        res = ((m - q[j]) * nq[j]).sum(1)
        keep = np.abs(res) <= np.percentile(np.abs(res), 100 * trim)
        if it == iterations:
            break
        rel = m - t
        a = np.c_[(np.c_[-rel[:, 1], rel[:, 0]] * nq[j]).sum(1), nq[j]][keep]
        delta = np.linalg.lstsq(a, -res[keep], rcond=None)[0]
        th += float(delta[0])
        t = t + delta[1:]
    return float(np.sqrt((res[keep] ** 2).mean())), -math.degrees(th)


def out_of_plane(shape, x, keys, edge, span=10.0, step=2.5):
    """Best (yaw, roll) of the fitted head for a profile's front edge, and the rms (m) of every grid cell.
    A left-right symmetric head gives the same silhouette at +a and -a: the sign of either angle is not observable."""
    names = [n for n in PROFILE_KEYS if n in keys and n in shape.key_rows]
    if len(names) < 4 or edge is None:
        return None
    rows = np.array([shape.key_rows[n] for n in names])
    pk = np.array([keys[n] for n in names])
    vals = np.arange(-span, span + 1e-9, step)
    grid = {}
    for yaw in vals:
        for roll in vals:
            grid[(float(yaw), float(roll))] = edge_rms(edge, pk, x, rows, rot(yaw, 0.0, roll))
    best = min(grid, key=lambda k: grid[k][0])
    return {"yaw": best[0], "roll": best[1], "rms": grid[best][0], "straight_rms": grid[(0.0, 0.0)][0], "pitch": grid[(0.0, 0.0)][1],
            "grid": grid}


def fit(shape, front=None, profile=None, size=None, focal_ratio=70.0 / 36.0, rounds=3, w_landmark=1.0, w_profile=1.0,
        ridge=1e-6, w_sym=1e-6):
    """front: 478 x 2 photo landmarks (pixels); profile: photo_keys() of the upright profile, or None."""
    t = shape.t
    w = np.zeros(t)
    k = camera(size, focal_ratio) if front is not None else None
    lm_rows = np.array([shape.row[i] for i in FIT_IDS])
    names = [n for n in PROFILE_KEYS if profile and n in profile and n in shape.key_rows]
    if len(names) < 4:
        names = []
    key_rows = np.array([shape.key_rows[n] for n in names], np.int64)
    pk = np.array([profile[n] for n in names]) if names else None
    sym = np.zeros((len(shape.sym), t))
    sym[np.arange(len(shape.sym)), shape.sym[:, 0]] = 1.0
    sym[np.arange(len(shape.sym)), shape.sym[:, 1]] = -1.0
    log, cam = [], None
    img = front[FIT_IDS, :2] if front is not None else None
    if img is None and not names:
        return None
    for rnd in range(rounds + 1):
        x = shape.points(w)
        entry = {"round": rnd}
        if img is not None:
            obj = x[lm_rows]
            cam = pnp(obj, img, k, None if cam is None else (cam["rvec"], cam["tvec"]))
            if cam is None:
                return None
            entry["front_rms_px"] = cam["rms"]
        if names:
            mk = x[key_rows] @ SIDE.T
            r2, s2, t2 = similarity(pk, mk)
            mapped = s2 * pk @ r2.T + t2
            entry["profile_keys_rms_mm"] = float(1000 * np.sqrt(((mapped - mk) ** 2).sum(1).mean()))
            entry["profile_keys_pitch"] = -math.degrees(math.atan2(r2[1, 0], r2[0, 0]))
        log.append(entry)
        if rnd == rounds:
            break
        g = ridge * np.eye(t) + w_sym * sym.T @ sym
        rhs = np.zeros(t)
        if img is not None:
            xc = obj @ cam["r"].T + cam["t"]
            f = k[0, 0]
            z = xc[:, 2]
            proj = np.c_[f * xc[:, 0] / z + k[0, 2], f * xc[:, 1] / z + k[1, 2]]
            jx = np.stack([np.c_[np.ones_like(z), np.zeros_like(z), -xc[:, 0] / z],
                           np.c_[np.zeros_like(z), np.ones_like(z), -xc[:, 1] / z]], 1) @ cam["r"]
            a = np.einsum("nkc,tnc->nkt", jx, shape.deltas[:, lm_rows, :]).reshape(-1, t)
            b = ((img - proj) / (f / z)[:, None]).ravel() + a @ w
            g += w_landmark / len(b) * a.T @ a
            rhs += w_landmark / len(b) * a.T @ b
        if names:
            a = np.einsum("kc,tnc->nkt", SIDE, shape.deltas[:, key_rows, :]).reshape(-1, t)
            b = (mapped - shape.base[key_rows] @ SIDE.T).ravel()
            g += w_profile / len(b) * a.T @ a
            rhs += w_profile / len(b) * a.T @ b
        w = bounded_lsq(g, rhs, w)
    out = {"weights": w, "log": log, "points": x, "keys_used": names}
    if cam is not None:
        out.update({**angles(R0_CV.T @ cam["r"]), "rms_px": cam["rms"], "camera": cam})
    return out
