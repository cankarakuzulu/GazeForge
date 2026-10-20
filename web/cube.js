import { tr } from "./i18n.js";
const FACES = [
  ["front", [0, -1, 0], [1, 0, 0], [0, 0, 1]],
  ["back", [0, 1, 0], [-1, 0, 0], [0, 0, 1]],
  ["top", [0, 0, 1], [1, 0, 0], [0, 1, 0]],
  ["bottom", [0, 0, -1], [1, 0, 0], [0, -1, 0]],
  ["left", [1, 0, 0], [0, 1, 0], [0, 0, 1]],
  ["right", [-1, 0, 0], [0, -1, 0], [0, 0, 1]],
];
const LABELS = ["front", "back", "top", "bottom", "left", "right"];
const BORDER = 0.6;
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const add = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const mul = (a, s) => [a[0] * s, a[1] * s, a[2] * s];
const norm = (a) => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };
const faceOf = (n) => FACES.findIndex((f) => dot(f[1], n) > 0.5);
const NS = "http://www.w3.org/2000/svg";

export function elementName(bits) { return FACES.filter((_, i) => bits & (1 << i)).map((f) => f[0]).join("-"); }
export function parseElement(name) {
  let bits = 0;
  for (const part of String(name).split("-")) { const i = FACES.findIndex((f) => f[0] === part); if (i < 0) return null; bits |= 1 << i; }
  const fs = FACES.filter((_, i) => bits & (1 << i));
  if (!fs.length || fs.length > 3 || fs.some((a) => fs.some((b) => dot(a[1], b[1]) < -0.5))) return null;
  return bits;
}
export function elementDir(bits) { return norm(FACES.reduce((a, f, i) => (bits & (1 << i) ? add(a, f[1]) : a), [0, 0, 0])); }
export function yawPitch(d) { return [Math.hypot(d[0], d[1]) < 1e-9 ? 0 : Math.atan2(d[0], -d[1]), Math.asin(Math.max(-1, Math.min(1, d[2])))]; }
export function allElements() {
  const out = [];
  for (let bits = 1; bits < 64; bits++) if (parseElement(elementName(bits)) === bits) out.push(bits);
  return out;
}

export class CubeGeom {
  constructor(cx, cy, r, basis) { Object.assign(this, { cx, cy, r, s: basis.s, u: basis.u, back: basis.back }); }
  screen(p) { return [this.cx + dot(p, this.s) * this.r, this.cy - dot(p, this.u) * this.r]; }
  local(f, x, y) { const [, n, a, b] = FACES[f]; return add(n, add(mul(a, x), mul(b, y))); }
  facing(f) { return dot(FACES[f][1], this.back); }
  visible() { return [0, 1, 2, 3, 4, 5].filter((f) => this.facing(f) > 0.02).sort((a, b) => this.facing(a) - this.facing(b)); }
  pick(qx, qy) {
    for (const f of this.visible()) {
      const [, , a, b] = FACES[f], o = this.screen(this.local(f, 0, 0)), c = this.screen([0, 0, 0]);
      const sa = [this.screen(a)[0] - c[0], this.screen(a)[1] - c[1]], sb = [this.screen(b)[0] - c[0], this.screen(b)[1] - c[1]];
      const d = [qx - o[0], qy - o[1]], det = sa[0] * sb[1] - sa[1] * sb[0];
      if (Math.abs(det) < 1e-6) continue;
      const x = (d[0] * sb[1] - d[1] * sb[0]) / det, y = (sa[0] * d[1] - sa[1] * d[0]) / det;
      if (Math.abs(x) > 1 || Math.abs(y) > 1) continue;
      let bits = 1 << f;
      if (Math.abs(x) > BORDER) bits |= 1 << faceOf(mul(a, Math.sign(x)));
      if (Math.abs(y) > BORDER) bits |= 1 << faceOf(mul(b, Math.sign(y)));
      return bits;
    }
    return null;
  }
  region(bits, f) {
    if (!(bits & (1 << f))) return null;
    const [, , a, b] = FACES[f];
    let want = bits & ~(1 << f);
    const span = (axis) => {
      const p = faceOf(axis), m = faceOf(mul(axis, -1));
      if (want & (1 << p)) { want &= ~(1 << p); return [BORDER, 1]; }
      if (want & (1 << m)) { want &= ~(1 << m); return [-1, -BORDER]; }
      return [-BORDER, BORDER];
    };
    const xs = span(a), ys = span(b);
    return want === 0 ? [xs[0], xs[1], ys[0], ys[1]] : null;
  }
  pointOf(bits) {
    let best = null;
    for (const f of this.visible()) {
      const r = this.region(bits, f);
      if (!r) continue;
      const p = this.screen(this.local(f, (r[0] + r[1]) / 2, (r[2] + r[3]) / 2)), t = this.facing(f);
      if (t > 0.15 && (!best || t > best.t)) best = { t, p };
    }
    return best ? best.p : null;
  }
  quad(f, r) { return [[r[0], r[2]], [r[1], r[2]], [r[1], r[3]], [r[0], r[3]]].map(([x, y]) => this.screen(this.local(f, x, y))); }
}

export class ViewCube {
  constructor(layer, viewer, actions) {
    this.layer = layer;
    this.V = viewer;
    this.actions = actions;
    this.items = [];
    this.key = "";
    this.hover = null;
  }

  spots() {
    return this.V.rects().map((r) => {
      const radius = Math.min(r.w, r.h) >= 360 ? 28 : 20, size = Math.round(radius * 3.5);
      return { slot: r.slot, cam: r.cam, radius, size, left: r.x + r.w - 10 - size, top: r.y + 8 };
    });
  }

  geom(spot) {
    const cam = this.V.cams[spot.cam];
    return new CubeGeom(spot.size / 2, spot.size / 2, spot.radius, cam.basis(cam.cur));
  }

  build(spots) {
    this.layer.innerHTML = "";
    this.items = spots.map((spot) => {
      const wrap = document.createElement("div");
      wrap.className = "vcube";
      wrap.style.left = `${spot.left}px`;
      wrap.style.top = `${spot.top}px`;
      wrap.style.width = `${spot.size}px`;
      const svg = document.createElementNS(NS, "svg");
      svg.setAttribute("width", spot.size);
      svg.setAttribute("height", spot.size);
      svg.setAttribute("class", "vcube-svg");
      const btns = document.createElement("div");
      btns.className = "vcube-btns";
      btns.innerHTML = `<button class="ib mini" data-act="home" title="${tr("cube.home")}"><svg class="ico"><use href="#i-house"/></svg></button>` +
        `<button class="ib mini" data-act="three" title="${tr("cube.three")}"><svg class="ico"><use href="#i-rotate-3d"/></svg></button>`;
      wrap.append(svg, btns);
      this.layer.appendChild(wrap);
      const item = { spot, wrap, svg };
      btns.addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) this.actions[b.dataset.act](spot.slot); });
      this.bindPointer(item);
      return item;
    });
  }

  bindPointer(item) {
    const svg = item.svg;
    let down = null;
    const local = (e) => { const b = svg.getBoundingClientRect(); return [e.clientX - b.left, e.clientY - b.top]; };
    svg.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      svg.setPointerCapture(e.pointerId);
      down = { x: e.clientX, y: e.clientY, moved: false };
      this.actions.activate(item.spot.slot);
    });
    svg.addEventListener("pointermove", (e) => {
      if (down) {
        const dx = e.clientX - down.x, dy = e.clientY - down.y;
        if (!down.moved && Math.hypot(dx, dy) < 3) return;
        down.moved = true;
        down.x = e.clientX;
        down.y = e.clientY;
        this.actions.orbit(item.spot.slot, dx, dy);
        return;
      }
      const [x, y] = local(e), h = this.geom(item.spot).pick(x, y), next = h === null ? null : [item.spot.slot, h];
      if (JSON.stringify(next) !== JSON.stringify(this.hover)) { this.hover = next; this.key = ""; this.update(); }
    });
    const up = (e) => {
      const d = down;
      down = null;
      if (!d || d.moved) return;
      const [x, y] = local(e), bits = this.geom(item.spot).pick(x, y);
      if (bits !== null) this.actions.snap(item.spot.slot, bits);
    };
    svg.addEventListener("pointerup", up);
    svg.addEventListener("pointercancel", () => { down = null; });
    svg.addEventListener("pointerleave", () => { if (this.hover && this.hover[0] === item.spot.slot) { this.hover = null; this.key = ""; this.update(); } });
  }

  update() {
    const spots = this.spots();
    const layoutKey = JSON.stringify(spots);
    if (layoutKey !== this.layoutKey || this.items.length !== spots.length) { this.layoutKey = layoutKey; this.build(spots); this.key = ""; }
    const key = JSON.stringify([layoutKey, spots.map((s) => { const c = this.V.cams[s.cam].cur; return [c.yaw, c.pitch]; }), this.hover, this.V.active]);
    if (key === this.key) return;
    this.key = key;
    for (const it of this.items) this.draw(it);
  }

  draw(it) {
    const g = this.geom(it.spot), parts = [], hover = this.hover && this.hover[0] === it.spot.slot ? this.hover[1] : null;
    const pts = (q) => q.map((p) => `${p[0].toFixed(2)},${p[1].toFixed(2)}`).join(" ");
    parts.push(`<circle class="cbg" cx="${g.cx}" cy="${g.cy}" r="${(it.spot.radius * 1.66).toFixed(1)}"/>`);
    for (const f of g.visible()) {
      const t = g.facing(f);
      parts.push(`<polygon class="cf" style="opacity:${(0.82 + 0.18 * t).toFixed(3)}" points="${pts(g.quad(f, [-1, 1, -1, 1]))}"/>`);
      const r = hover !== null ? g.region(hover, f) : null;
      if (r) parts.push(`<polygon class="ch" points="${pts(g.quad(f, r))}"/>`);
      if (t > 0.2) {
        const [, n, a, b] = FACES[f], c = g.screen(n), sa = [dot(a, g.s) * g.r, -dot(a, g.u) * g.r], sb = [dot(b, g.s) * g.r, -dot(b, g.u) * g.r];
        const k = 1 / 64;
        parts.push(`<text class="ct" style="opacity:${Math.min(1, (t - 0.2) / 0.4).toFixed(3)}" transform="matrix(${(sa[0] * k).toFixed(4)} ${(sa[1] * k).toFixed(4)} ${(-sb[0] * k).toFixed(4)} ${(-sb[1] * k).toFixed(4)} ${c[0].toFixed(2)} ${c[1].toFixed(2)})">${tr(`cube.${LABELS[f]}`)}</text>`);
      }
    }
    it.svg.innerHTML = parts.join("");
    it.wrap.classList.toggle("active", this.V.rects().length > 1 && it.spot.slot === this.V.active);
  }

  pointOf(slot, name) {
    const spot = this.spots().find((s) => s.slot === slot), it = this.items.find((x) => x.spot.slot === slot), bits = parseElement(name);
    if (!spot || !it || bits === null) return null;
    const p = this.geom(spot).pointOf(bits);
    if (!p) return null;
    const b = it.svg.getBoundingClientRect();
    return { x: b.left + p[0], y: b.top + p[1], dir: elementDir(bits) };
  }

  buttonOf(slot, act) {
    const it = this.items.find((x) => x.spot.slot === slot), b = it?.wrap.querySelector(`button[data-act="${act}"]`)?.getBoundingClientRect();
    return b ? { x: b.left + b.width / 2, y: b.top + b.height / 2 } : null;
  }
}
