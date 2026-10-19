const TW = 2048;
const DEG = Math.PI / 180;

export const MODES = [
  { id: "clay", label: "clay" },
  { id: "flat", label: "flat" },
  { id: "gouraud", label: "Gouraud" },
  { id: "blinn", label: "Blinn-Phong" },
  { id: "aniso", label: "anisotropic (Kajiya-Kay)" },
  { id: "matcap", label: "matcap" },
  { id: "normals", label: "normals" },
  { id: "hidden", label: "hidden line" },
  { id: "quality", label: "triangle quality (min angle)" },
  { id: "displacement", label: "displacement from reference (mm)" },
  { id: "curvature", label: "mean curvature (1/m)" },
  { id: "uv", label: "UV checker" },
];
export const MODE_INDEX = Object.fromEntries(MODES.map((m, i) => [m.id, i]));

export const LIGHT_PRESETS = {
  studio: { label: "studio three-point", ambient: 0.16, ground: 0.4, lights: [{ az: -40, el: 30, i: 1.0, k: 5600 }, { az: 50, el: 10, i: 0.35, k: 6500 }, { az: 160, el: 35, i: 0.8, k: 7500 }] },
  flat: { label: "flat", ambient: 0.9, ground: 1.0, lights: [] },
  rim: { label: "rim", ambient: 0.06, ground: 0.4, lights: [{ az: 150, el: 20, i: 1.1, k: 7000 }, { az: -150, el: 20, i: 1.1, k: 7000 }, { az: 0, el: 10, i: 0.15, k: 5000 }] },
  top: { label: "top", ambient: 0.1, ground: 0.3, lights: [{ az: 0, el: 85, i: 1.2, k: 5500 }] },
  key: { label: "single key", ambient: 0.08, ground: 0.4, lights: [{ az: -35, el: 25, i: 1.2, k: 5200 }] },
};
export const MAX_LIGHTS = 4;
export const LAYOUTS = { single: [0], side: [0, 2], quad: [1, 2, 3, 0] };

export function kelvinRGB(k) {
  const t = k / 100;
  let r, g, b;
  if (t <= 66) {
    r = 255;
    g = 99.4708025861 * Math.log(t) - 161.1195681661;
    b = t <= 19 ? 0 : 138.5177312231 * Math.log(t - 10) - 305.0447927307;
  } else {
    r = 329.698727446 * Math.pow(t - 60, -0.1332047592);
    g = 288.1221695283 * Math.pow(t - 60, -0.0755148492);
    b = 255;
  }
  return [r, g, b].map((x) => Math.pow(Math.min(255, Math.max(0, x)) / 255, 2.2));
}

export function parseColor(c, fallback = [0.7, 0.7, 0.7]) {
  if (Array.isArray(c) || c instanceof Float32Array) return [c[0], c[1], c[2]].map(Number);
  if (typeof c === "string" && /^#?[0-9a-f]{6}$/i.test(c)) {
    const h = c.replace("#", "");
    return [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
  }
  return fallback.slice();
}

const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const add = (a, b) => [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
const mul = (a, s) => [a[0] * s, a[1] * s, a[2] * s];
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const norm = (a) => { const l = Math.hypot(a[0], a[1], a[2]) || 1; return [a[0] / l, a[1] / l, a[2] / l]; };
const wrapAngle = (a) => Math.atan2(Math.sin(a), Math.cos(a));

export function vertexNormals(pos, idx, n) {
  const out = new Float32Array(n * 3);
  for (let t = 0; t < idx.length; t += 3) {
    const a = idx[t] * 3, b = idx[t + 1] * 3, c = idx[t + 2] * 3;
    const ux = pos[b] - pos[a], uy = pos[b + 1] - pos[a + 1], uz = pos[b + 2] - pos[a + 2];
    const vx = pos[c] - pos[a], vy = pos[c + 1] - pos[a + 1], vz = pos[c + 2] - pos[a + 2];
    const nx = uy * vz - uz * vy, ny = uz * vx - ux * vz, nz = ux * vy - uy * vx;
    for (const v of [a, b, c]) { out[v] += nx; out[v + 1] += ny; out[v + 2] += nz; }
  }
  for (let v = 0; v < n * 3; v += 3) {
    const l = Math.hypot(out[v], out[v + 1], out[v + 2]) || 1;
    out[v] /= l; out[v + 1] /= l; out[v + 2] /= l;
  }
  return out;
}

export function windingSign(pos, idx) {
  let vol = 0;
  for (let t = 0; t < idx.length; t += 3) {
    const a = idx[t] * 3, b = idx[t + 1] * 3, c = idx[t + 2] * 3;
    const p = [pos[a], pos[a + 1], pos[a + 2]], q = [pos[b], pos[b + 1], pos[b + 2]], r = [pos[c], pos[c + 1], pos[c + 2]];
    vol += dot(p, cross(q, r));
  }
  return vol < 0 ? -1 : 1;
}

export function edgeFlags(idx, uvs) {
  const tri = idx.length / 3, flags = new Uint8Array(tri), seen = new Map();
  const uvAt = (t, v) => { for (let k = 0; k < 3; k++) if (idx[t * 3 + k] === v) return [uvs[(t * 3 + k) * 2], uvs[(t * 3 + k) * 2 + 1]]; return [0, 0]; };
  for (let t = 0; t < tri; t++) for (let k = 0; k < 3; k++) {
    const a = idx[t * 3 + ((k + 1) % 3)], b = idx[t * 3 + ((k + 2) % 3)];
    const key = Math.min(a, b) * 4194304 + Math.max(a, b);
    const first = seen.get(key);
    if (first === undefined) { seen.set(key, t * 3 + k); continue; }
    if (first < 0) continue;
    seen.set(key, -1);
    if (uvs) {
      const t1 = Math.floor(first / 3), k1 = first % 3;
      let seam = false;
      for (const v of [a, b]) { const p = uvAt(t1, v), q = uvAt(t, v); if (Math.abs(p[0] - q[0]) > 1e-5 || Math.abs(p[1] - q[1]) > 1e-5) seam = true; }
      if (seam) { flags[t1] |= 8 << k1; flags[t] |= 8 << k; }
    }
  }
  let boundary = 0, seams = 0;
  for (const c of seen.values()) if (c >= 0) { flags[Math.floor(c / 3)] |= 1 << (c % 3); boundary++; }
  for (let t = 0; t < tri; t++) for (let k = 0; k < 3; k++) if (flags[t] & (8 << k)) seams++;
  return { flags, boundary, seams: seams / 2 };
}

export class Camera {
  constructor(o = {}) {
    this.goal = { yaw: 0, pitch: 0.08, dist: 0.45, target: [0, 0, 0], fov: 30 };
    this.ortho = !!o.ortho;
    this.name = o.name || "";
    this.set(o, true);
  }
  set(o, instant = false) {
    for (const k of ["yaw", "pitch", "dist", "fov"]) if (o[k] !== undefined) this.goal[k] = o[k];
    if (o.target) this.goal.target = Array.from(o.target);
    if (o.ortho !== undefined) this.ortho = o.ortho;
    this.goal.pitch = Math.max(-Math.PI / 2 + 1e-3, Math.min(Math.PI / 2 - 1e-3, this.goal.pitch));
    this.goal.dist = Math.max(0.02, Math.min(8, this.goal.dist));
    this.goal.fov = Math.max(5, Math.min(100, this.goal.fov));
    if (instant || !this.cur) this.cur = { ...this.goal, target: this.goal.target.slice() };
  }
  view(yaw, pitch) {
    const cur = this.cur;
    this.set({ yaw: cur.yaw + wrapAngle(yaw - cur.yaw), pitch });
  }
  step(dt) {
    const a = 1 - Math.exp(-Math.min(dt, 0.1) * 18), c = this.cur, g = this.goal;
    let moving = false;
    const ease = (x, y, eps) => { const d = y - x; if (Math.abs(d) > eps) { moving = true; return x + d * a; } return y; };
    c.yaw = ease(c.yaw, g.yaw, 1e-4);
    c.pitch = ease(c.pitch, g.pitch, 1e-4);
    c.fov = ease(c.fov, g.fov, 1e-3);
    c.dist = Math.exp(ease(Math.log(c.dist), Math.log(g.dist), 1e-4));
    c.target = c.target.map((x, i) => ease(x, g.target[i], 1e-6));
    return moving;
  }
  basis(s = this.cur) {
    const cp = Math.cos(s.pitch), sp = Math.sin(s.pitch), cy = Math.cos(s.yaw), sy = Math.sin(s.yaw);
    const back = [sy * cp, -cy * cp, sp], side = [cy, sy, 0], f = mul(back, -1);
    return { back, s: side, u: cross(side, f), f };
  }
  matrices(aspect) {
    const c = this.cur, b = this.basis();
    const tanh = Math.tan((c.fov * DEG) / 2);
    const halfH = c.dist * tanh;
    const eyeDist = this.ortho ? Math.max(c.dist, 3) : c.dist;
    const eye = add(c.target, mul(b.back, eyeDist));
    const near = this.ortho ? 0.005 : Math.max(0.001, c.dist * 0.03), far = eyeDist + 4;
    const { s, u, f } = b;
    const view = [s[0], u[0], -f[0], 0, s[1], u[1], -f[1], 0, s[2], u[2], -f[2], 0, -dot(s, eye), -dot(u, eye), dot(f, eye), 1];
    let proj;
    if (this.ortho) {
      const w = halfH * aspect;
      proj = [1 / w, 0, 0, 0, 0, 1 / halfH, 0, 0, 0, 0, -2 / (far - near), 0, 0, 0, -(far + near) / (far - near), 1];
    } else {
      const t = 1 / tanh;
      proj = [t / aspect, 0, 0, 0, 0, t, 0, 0, 0, 0, (far + near) / (near - far), -1, 0, 0, (2 * far * near) / (near - far), 0];
    }
    const m = new Float32Array(16);
    for (let col = 0; col < 4; col++) for (let r = 0; r < 4; r++) {
      let x = 0;
      for (let k = 0; k < 4; k++) x += proj[k * 4 + r] * view[col * 4 + k];
      m[col * 4 + r] = x;
    }
    return { mvp: m, eye, s, u, f, halfH, aspect, ortho: this.ortho, tanh };
  }
  ray(nx, ny, aspect) {
    const M = this.matrices(aspect);
    if (M.ortho) return { o: add(M.eye, add(mul(M.s, nx * M.halfH * aspect), mul(M.u, ny * M.halfH))), d: M.f };
    return { o: M.eye, d: norm(add(M.f, add(mul(M.s, nx * M.tanh * aspect), mul(M.u, ny * M.tanh)))) };
  }
  snapshot() { return { ...this.goal, target: this.goal.target.slice(), ortho: this.ortho }; }
}

const COMMON = `
uniform int mode; uniform vec3 eye; uniform vec3 camS; uniform vec3 camU; uniform vec3 camF; uniform int ortho;
uniform int nLights; uniform vec3 lightDir[4]; uniform vec3 lightCol[4]; uniform vec3 sky; uniform vec3 ground;
uniform vec3 baseCol; uniform float aniso;
vec3 hemi(vec3 n) { return mix(ground, sky, n.z * 0.5 + 0.5); }
vec3 tangentOf(vec3 n) {
  vec3 h = cross(vec3(0.0, 0.0, 1.0), n);
  vec3 t = aniso < 0.5 ? h : cross(n, h);
  float l = length(t);
  return l > 1e-4 ? t / l : vec3(1.0, 0.0, 0.0);
}
vec3 shade(int model, vec3 n, vec3 v, vec3 albedo) {
  vec3 c = albedo * hemi(n);
  for (int i = 0; i < 4; i++) {
    if (i >= nLights) break;
    vec3 l = lightDir[i];
    float nl = dot(n, l);
    if (model == 0) {
      c += albedo * lightCol[i] * max((nl + 0.3) / 1.3, 0.0);
    } else if (model == 1) {
      c += albedo * lightCol[i] * max(nl, 0.0);
    } else if (model == 2) {
      vec3 h = normalize(l + v);
      float sp = pow(max(dot(n, h), 0.0), 48.0) * step(0.0, nl);
      c += lightCol[i] * (albedo * max(nl, 0.0) + 0.35 * sp);
    } else {
      vec3 t = tangentOf(n);
      vec3 h = normalize(l + v);
      float th = dot(t, h);
      float sp = pow(sqrt(max(1.0 - th * th, 0.0)), 80.0) * smoothstep(-0.1, 0.25, nl);
      c += lightCol[i] * (albedo * max(nl, 0.0) * 0.8 + 0.45 * sp);
    }
  }
  return c;
}`;

const MESH_VS = `#version 300 es
precision highp float; precision highp int; precision highp sampler2D; precision highp usampler2D;
uniform highp usampler2D idxT; uniform sampler2D posT; uniform sampler2D nrmT; uniform sampler2D refT; uniform sampler2D uvT;
uniform highp usampler2D flagT; uniform highp usampler2D regT;
uniform mat4 mvp; uniform float windSign; uniform uint hidMask; uniform int hasRegion; uniform int hasRef; uniform int hasUV; uniform int hasFlags;
${COMMON}
out vec3 vp; out vec3 vn; out vec3 bary; out vec3 gcol; out float disp; out vec2 uv;
flat out vec3 fn; flat out float tq; flat out float tc; flat out uint eflags;
ivec2 at(int i) { return ivec2(i & 2047, i >> 11); }
void main() {
  int c = gl_VertexID; int t = c / 3; int k = c - t * 3;
  int i0 = int(texelFetch(idxT, at(t * 3), 0).r), i1 = int(texelFetch(idxT, at(t * 3 + 1), 0).r), i2 = int(texelFetch(idxT, at(t * 3 + 2), 0).r);
  if (hasRegion == 1 && hidMask != 0u) {
    uint r0 = texelFetch(regT, at(i0), 0).r, r1 = texelFetch(regT, at(i1), 0).r, r2 = texelFetch(regT, at(i2), 0).r;
    uint hid = ((hidMask >> r0) & 1u) + ((hidMask >> r1) & 1u) + ((hidMask >> r2) & 1u);
    if (hid >= 2u) { gl_Position = vec4(2.0, 2.0, 2.0, 1.0); return; }
  }
  vec3 p0 = texelFetch(posT, at(i0), 0).xyz, p1 = texelFetch(posT, at(i1), 0).xyz, p2 = texelFetch(posT, at(i2), 0).xyz;
  int me = k == 0 ? i0 : (k == 1 ? i1 : i2);
  vec3 p = k == 0 ? p0 : (k == 1 ? p1 : p2);
  vn = texelFetch(nrmT, at(me), 0).xyz;
  vp = p;
  vec3 e1 = p1 - p0, e2 = p2 - p0, cr = cross(e1, e2);
  float a2 = length(cr);
  fn = a2 > 0.0 ? cr / a2 * windSign : vn;
  bary = vec3(k == 0, k == 1, k == 2);
  tq = 0.0; tc = 0.0; disp = 0.0; uv = vec2(0.0); eflags = 0u; gcol = vec3(0.0);
  if (mode == 8) {
    if (a2 > 0.0) {
      vec3 a = normalize(e1), b = normalize(e2), d = normalize(p2 - p1);
      float A0 = acos(clamp(dot(a, b), -1.0, 1.0)), A1 = acos(clamp(dot(-a, d), -1.0, 1.0));
      tq = degrees(min(A0, min(A1, 3.14159265 - A0 - A1)));
    }
  } else if (mode == 10) {
    if (a2 > 1e-14) {
      vec3 n0 = texelFetch(nrmT, at(i0), 0).xyz, n1 = texelFetch(nrmT, at(i1), 0).xyz, n2 = texelFetch(nrmT, at(i2), 0).xyz;
      vec3 X = normalize(e1), N = cr / a2, Y = cross(N, X), d1 = n1 - n0, d2 = n2 - n0;
      mat2 E = mat2(dot(e1, X), dot(e1, Y), dot(e2, X), dot(e2, Y));
      mat2 Dn = mat2(dot(d1, X), dot(d1, Y), dot(d2, X), dot(d2, Y));
      mat2 S = Dn * inverse(E);
      tc = 0.5 * (S[0][0] + S[1][1]);
    }
  } else if (mode == 9 && hasRef == 1) {
    vec3 dd = p - texelFetch(refT, at(me), 0).xyz;
    disp = length(dd) * 1000.0 * (dot(dd, vn) < 0.0 ? -1.0 : 1.0);
  } else if (mode == 11 && hasUV == 1) {
    uv = texelFetch(uvT, at(c), 0).xy;
  } else if (mode == 2) {
    vec3 v = ortho == 1 ? -camF : normalize(eye - p);
    gcol = shade(2, normalize(vn), v, baseCol);
  }
  if (hasFlags == 1) eflags = texelFetch(flagT, at(t), 0).r;
  gl_Position = mvp * vec4(p, 1.0);
}`;

const CLIP = `
uniform vec4 planes[6]; uniform int nPlanes; uniform int skipPlane; uniform int boxOn; uniform vec3 boxMin; uniform vec3 boxMax;
bool clipped(vec3 p) {
  for (int i = 0; i < 6; i++) {
    if (i >= nPlanes) break;
    if (i != skipPlane && dot(planes[i].xyz, p) > planes[i].w) return true;
  }
  return boxOn == 1 && all(greaterThan(p, boxMin)) && all(lessThan(p, boxMax));
}`;

const MESH_FS = `#version 300 es
precision highp float; precision highp int;
in vec3 vp; in vec3 vn; in vec3 bary; in vec3 gcol; in float disp; in vec2 uv;
flat in vec3 fn; flat in float tq; flat in float tc; flat in uint eflags;
${COMMON}
${CLIP}
uniform int wireOn; uniform float wireW; uniform vec3 wireCol; uniform int xrayPass; uniform int wireFade; uniform int backHi; uniform int seamOn;
uniform vec3 bgCol; uniform vec3 interiorCol; uniform float heatMax; uniform int hasRef; uniform int hasUV; uniform int hasFlags; uniform float exposure;
out vec4 o;
vec3 ramp(float t) {
  t = clamp(t, 0.0, 1.0) * 4.0;
  vec3 a = vec3(0.16, 0.26, 0.85), b = vec3(0.10, 0.72, 0.82), c = vec3(0.32, 0.80, 0.30), d = vec3(0.96, 0.84, 0.20), e = vec3(0.86, 0.14, 0.10);
  return t < 1.0 ? mix(a, b, t) : t < 2.0 ? mix(b, c, t - 1.0) : t < 3.0 ? mix(c, d, t - 2.0) : mix(d, e, t - 3.0);
}
vec3 diverge(float t) {
  t = clamp(t, -1.0, 1.0);
  return t < 0.0 ? mix(vec3(0.93), vec3(0.12, 0.32, 0.86), -t) : mix(vec3(0.93), vec3(0.86, 0.16, 0.10), t);
}
vec3 checker(vec2 t) {
  vec2 g = floor(t * 16.0);
  float k = mod(g.x + g.y, 2.0);
  vec3 c = mix(vec3(0.22), vec3(0.92), k);
  return c * vec3(0.7 + 0.3 * fract(t.x), 0.7 + 0.3 * fract(t.y), 0.85);
}
vec3 matcap(vec3 n) {
  vec3 q = vec3(dot(n, camS), dot(n, camU), -dot(n, camF));
  float key = clamp(dot(q, normalize(vec3(-0.45, 0.55, 0.7))), 0.0, 1.0);
  float rim = pow(1.0 - clamp(q.z, 0.0, 1.0), 2.5);
  vec3 c = mix(vec3(0.10, 0.09, 0.09), vec3(0.80, 0.72, 0.66), key);
  return c + vec3(1.0, 0.97, 0.92) * pow(key, 24.0) * 0.45 + vec3(0.35, 0.42, 0.55) * rim * 0.6;
}
void main() {
  if (clipped(vp)) discard;
  vec3 fw = fwidth(bary);
  vec3 d = bary / max(fw, vec3(1e-6));
  float edge = min(d.x, min(d.y, d.z));
  float wire = 1.0 - smoothstep(wireW - 0.5, wireW + 0.5, edge);
  if (wireFade == 1) wire *= 1.0 - 0.8 * smoothstep(0.15, 0.5, max(fw.x, max(fw.y, fw.z)));
  if (xrayPass == 1) { if (wire <= 0.02) discard; o = vec4(wireCol, 0.35 * wire); return; }
  bool front = gl_FrontFacing;
  vec3 n = normalize(vn);
  if (!front) n = -n;
  vec3 v = ortho == 1 ? -camF : normalize(eye - vp);
  float hl = 0.55 + 0.45 * abs(dot(n, v));
  vec3 c;
  bool lit = true;
  if (!front && nPlanes > 0) { c = interiorCol * (0.45 + 0.55 * abs(dot(n, v))); lit = false; }
  else if (mode == 0) c = shade(0, n, v, baseCol);
  else if (mode == 1) { vec3 f = normalize(fn); if (!front) f = -f; c = shade(1, f, v, baseCol); }
  else if (mode == 2) c = gcol;
  else if (mode == 3) c = shade(2, n, v, baseCol);
  else if (mode == 4) c = shade(3, n, v, baseCol);
  else if (mode == 5) { c = matcap(n); lit = false; }
  else if (mode == 6) { c = normalize(vn) * 0.5 + 0.5; lit = false; }
  else if (mode == 7) { c = bgCol; lit = false; }
  else if (mode == 8) { c = ramp(1.0 - clamp(tq / heatMax, 0.0, 1.0)) * hl; lit = false; }
  else if (mode == 9) { c = (hasRef == 1 ? diverge(disp / heatMax) : vec3(0.55)) * hl; lit = false; }
  else if (mode == 10) { c = diverge(tc / heatMax) * hl; lit = false; }
  else { c = (hasUV == 1 ? checker(uv) : vec3(0.55)) * hl; lit = false; }
  if (lit) c = pow(max(c * exp2(exposure), vec3(0.0)), vec3(1.0 / 2.2));
  if (backHi == 1 && !front && nPlanes == 0) c = mix(c, vec3(0.92, 0.10, 0.16), 0.65);
  if (seamOn == 1 && hasFlags == 1) {
    float b = 1e9, s = 1e9;
    for (int k = 0; k < 3; k++) {
      if ((eflags & (1u << uint(k))) != 0u) b = min(b, d[k]);
      if ((eflags & (8u << uint(k))) != 0u) s = min(s, d[k]);
    }
    float w2 = wireW + 1.0;
    c = mix(c, vec3(0.10, 0.80, 1.00), 1.0 - smoothstep(w2 - 0.5, w2 + 0.5, s));
    c = mix(c, vec3(1.00, 0.55, 0.05), 1.0 - smoothstep(w2 - 0.5, w2 + 0.5, b));
  }
  if (wireOn == 1 || mode == 7) c = mix(c, wireCol, wire);
  o = vec4(c, 1.0);
}`;

const POINT_VS = `#version 300 es
precision highp float; precision highp int; precision highp sampler2D; precision highp usampler2D;
uniform highp usampler2D idxT; uniform sampler2D posT; uniform highp usampler2D regT; uniform uint hidMask; uniform int hasRegion;
uniform mat4 mvp; uniform float psize; out vec3 vp;
ivec2 at(int i) { return ivec2(i & 2047, i >> 11); }
void main() {
  int i = int(texelFetch(idxT, at(gl_VertexID), 0).r);
  if (hasRegion == 1 && ((hidMask >> texelFetch(regT, at(i), 0).r) & 1u) != 0u) { gl_Position = vec4(2.0, 2.0, 2.0, 1.0); return; }
  vp = texelFetch(posT, at(i), 0).xyz;
  gl_Position = mvp * vec4(vp, 1.0);
  gl_PointSize = psize;
}`;

const POINT_FS = `#version 300 es
precision highp float; precision highp int;
in vec3 vp; uniform vec3 col; out vec4 o;
${CLIP}
void main() { if (clipped(vp)) discard; o = vec4(col, 1.0); }`;

const LINE_VS = `#version 300 es
precision highp float;
in vec3 a; uniform mat4 mvp; uniform float psize; out vec3 vp;
void main() { vp = a; gl_Position = mvp * vec4(a, 1.0); gl_PointSize = psize; }`;

const LINE_FS = `#version 300 es
precision highp float; precision highp int;
in vec3 vp; uniform vec3 col; uniform int capPass; out vec4 o;
${CLIP}
void main() {
  if (capPass == 1) {
    if (clipped(vp)) discard;
    float h = step(0.5, fract((gl_FragCoord.x + gl_FragCoord.y) / 10.0));
    o = vec4(mix(col, col * 0.78, h), 1.0);
    return;
  }
  o = vec4(col, 1.0);
}`;

export class Viewer {
  constructor(gl, labels) {
    this.gl = gl;
    this.canvas = gl.canvas;
    this.labels = labels;
    this.meshes = new Map();
    this.overlays = new Map();
    this.cams = [
      new Camera({ name: "perspective" }),
      new Camera({ name: "front", ortho: true, yaw: 0, pitch: 0 }),
      new Camera({ name: "side", ortho: true, yaw: Math.PI / 2, pitch: 0 }),
      new Camera({ name: "top", ortho: true, yaw: 0, pitch: Math.PI / 2 }),
    ];
    this.layout = "single";
    this.active = 0;
    this.vp = [0, 1, 2, 3].map(() => ({ mode: 0, lod: 0 }));
    this.opts = {
      wire: false, wireW: 0.75, wireFade: true, xray: false, points: false, pointSize: 3, backHi: false, seams: false,
      heat: { quality: 30, displacement: 3, curvature: 50 }, aniso: 0, exposure: 0, ambient: 0.16, ground: 0.4, follow: true,
      lights: LIGHT_PRESETS.studio.lights.map((l) => ({ on: true, ...l })), caps: true, planeGizmo: true, lightGizmo: false,
      clip: [0, 1, 2].map((i) => ({ on: false, axis: ["x", "y", "z"][i], az: 0, el: 0, offset: 0, flip: false, slab: false, thick: 0.01 })),
      box: { on: false, lo: [0.4, 0, 0.4], hi: [1, 0.5, 1] }, hiddenRegions: 0,
    };
    while (this.opts.lights.length < MAX_LIGHTS) this.opts.lights.push({ on: false, az: 0, el: 0, i: 0.5, k: 6500 });
    this.bounds = { lo: [-0.1, -0.1, -0.1], hi: [0.1, 0.1, 0.1], centre: [0, 0, 0], radius: 0.15 };
    this.pivotFlash = 0;
    this.setup();
  }

  setup() {
    const gl = this.gl;
    const compile = (vs, fs, units = {}) => {
      const sh = (type, src) => {
        const s = gl.createShader(type);
        gl.shaderSource(s, src);
        gl.compileShader(s);
        if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(`shader: ${gl.getShaderInfoLog(s)}; context lost: ${gl.isContextLost()}`);
        return s;
      };
      const p = gl.createProgram();
      gl.attachShader(p, sh(gl.VERTEX_SHADER, vs));
      gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
      gl.linkProgram(p);
      if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(`program: ${gl.getProgramInfoLog(p)}; context lost: ${gl.isContextLost()}`);
      const locs = new Map();
      p.u = (name) => { if (!locs.has(name)) locs.set(name, gl.getUniformLocation(p, name)); return locs.get(name); };
      gl.useProgram(p);
      for (const [name, unit] of Object.entries(units)) gl.uniform1i(p.u(name), unit);
      return p;
    };
    const units = { idxT: 0, posT: 1, nrmT: 2, refT: 3, uvT: 4, flagT: 5, regT: 6 };
    this.meshProg = compile(MESH_VS, MESH_FS, units);
    this.pointProg = compile(POINT_VS, POINT_FS, { idxT: 0, posT: 1, regT: 6 });
    this.lineProg = compile(LINE_VS, LINE_FS);
    this.kinds = {
      f3: { internal: gl.RGB32F, format: gl.RGB, type: gl.FLOAT, comps: 3 },
      f2: { internal: gl.RG32F, format: gl.RG, type: gl.FLOAT, comps: 2 },
      u1: { internal: gl.R32UI, format: gl.RED_INTEGER, type: gl.UNSIGNED_INT, comps: 1 },
      b1: { internal: gl.R8UI, format: gl.RED_INTEGER, type: gl.UNSIGNED_BYTE, comps: 1 },
    };
    this.dummy = {
      f3: this.tex("f3", 1, new Float32Array(3)), f2: this.tex("f2", 1, new Float32Array(2)),
      u1: this.tex("u1", 1, new Uint32Array(1)), b1: this.tex("b1", 1, new Uint8Array(1)),
    };
    this.emptyVao = gl.createVertexArray();
    this.lineVao = gl.createVertexArray();
    this.lineBuf = gl.createBuffer();
    gl.bindVertexArray(this.lineVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.lineBuf);
    gl.enableVertexAttribArray(gl.getAttribLocation(this.lineProg, "a"));
    gl.vertexAttribPointer(gl.getAttribLocation(this.lineProg, "a"), 3, gl.FLOAT, false, 0, 0);
    gl.bindVertexArray(null);
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
    const dbg = gl.getExtension("WEBGL_debug_renderer_info");
    this.renderer = dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
  }

  restore() {
    this.setup();
    for (const m of this.meshes.values()) {
      m.pos = this.tex("f3", m.n, m.kind === "head" ? undefined : m.cpuPos);
      m.nrm = this.tex("f3", m.n, m.kind === "head" ? undefined : m.nrmCPU);
      m.ref = m.refCPU ? this.tex("f3", m.n, m.refCPU) : null;
      m.region = m.regionCPU ? this.tex("b1", m.n, m.regionCPU) : null;
      for (const l of m.lods) {
        l.idx = this.tex("u1", l.idxCPU.length, l.idxCPU);
        l.verts = this.tex("u1", l.vertsCPU.length, l.vertsCPU);
        l.uv = l.uvCPU ? this.tex("f2", l.tri * 3, l.uvCPU) : null;
        l.flags = this.tex("b1", l.tri, l.flagsCPU);
      }
    }
    this.labelKey = "";
  }

  tex(kind, count, data) {
    const gl = this.gl, k = this.kinds[kind];
    const h = Math.max(1, Math.ceil(count / TW));
    const t = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texStorage2D(gl.TEXTURE_2D, 1, k.internal, TW, h);
    for (const p of [gl.TEXTURE_MIN_FILTER, gl.TEXTURE_MAG_FILTER]) gl.texParameteri(gl.TEXTURE_2D, p, gl.NEAREST);
    for (const p of [gl.TEXTURE_WRAP_S, gl.TEXTURE_WRAP_T]) gl.texParameteri(gl.TEXTURE_2D, p, gl.CLAMP_TO_EDGE);
    const r = { t, count, kind };
    if (data) this.upload(r, data);
    return r;
  }

  upload(r, data) {
    const gl = this.gl, k = this.kinds[r.kind];
    gl.bindTexture(gl.TEXTURE_2D, r.t);
    const full = Math.floor(r.count / TW), rest = r.count - full * TW;
    if (full) gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, TW, full, k.format, k.type, data, 0);
    if (rest) gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, full, rest, 1, k.format, k.type, data, full * TW * k.comps);
  }

  drop(r) { if (r && r.t) this.gl.deleteTexture(r.t); }

  makeLod(indices, uvs) {
    const idx = indices instanceof Uint32Array ? indices : Uint32Array.from(indices);
    let top = 0;
    for (let i = 0; i < idx.length; i++) if (idx[i] > top) top = idx[i];
    const seen = new Uint8Array(top + 1), verts = [];
    for (let i = 0; i < idx.length; i++) if (!seen[idx[i]]) { seen[idx[i]] = 1; verts.push(idx[i]); }
    const vertsCPU = Uint32Array.from(verts);
    const lod = { tri: idx.length / 3, idxCPU: idx, idx: this.tex("u1", idx.length, idx), vertsCPU, verts: this.tex("u1", vertsCPU.length, vertsCPU), uv: null, flags: null, uvCPU: null, edgeStats: null };
    this.setLodUVs(lod, uvs || null);
    return lod;
  }

  setLodUVs(lod, uvs) {
    this.drop(lod.uv);
    this.drop(lod.flags);
    lod.uvCPU = uvs;
    lod.uv = uvs ? this.tex("f2", lod.tri * 3, uvs) : null;
    const ef = edgeFlags(lod.idxCPU, uvs);
    lod.flagsCPU = ef.flags;
    lod.flags = this.tex("b1", lod.tri, ef.flags);
    lod.edgeStats = { boundary: ef.boundary, seams: ef.seams };
  }

  setHead(n, indices, uvs) {
    const m = { id: "head", label: "head", kind: "head", n, visible: true, color: [0.80, 0.74, 0.68], pos: this.tex("f3", n), nrm: this.tex("f3", n), ref: null, region: null, lods: [], cpuPos: null };
    m.lods.push(this.makeLod(indices, uvs));
    this.meshes.set("head", m);
    return m;
  }

  addHeadLod(indices, uvs, label) {
    const m = this.meshes.get("head");
    const lod = this.makeLod(indices, uvs);
    lod.label = label;
    m.lods.push(lod);
    return lod;
  }

  updateHead(positions, normals) {
    const m = this.meshes.get("head");
    if (!m) return false;
    this.upload(m.pos, positions);
    this.upload(m.nrm, normals);
    m.cpuPos = positions;
    if (!m.wind) m.wind = windingSign(positions, m.lods[0].idxCPU);
  }

  setReference(positions) {
    const m = this.meshes.get("head");
    m.refCPU = positions || null;
    if (!positions) { this.drop(m.ref); m.ref = null; return; }
    if (!m.ref) m.ref = this.tex("f3", m.n);
    this.upload(m.ref, positions);
  }

  setRegions(regionOfVertex, names) {
    const m = this.meshes.get("head");
    this.drop(m.region);
    m.regionCPU = regionOfVertex;
    m.region = this.tex("b1", m.n, regionOfVertex);
    this.regionNames = names;
  }

  setExtra(id, spec, kind = "extra") {
    const old = this.meshes.get(id);
    if (old && old.kind !== "head") { this.drop(old.pos); this.drop(old.nrm); for (const l of old.lods) { this.drop(l.idx); this.drop(l.verts); this.drop(l.uv); this.drop(l.flags); } this.meshes.delete(id); }
    if (!spec) return;
    const pos = spec.positions instanceof Float32Array ? spec.positions : Float32Array.from(spec.positions);
    const idx = spec.indices instanceof Uint32Array ? spec.indices : Uint32Array.from(spec.indices);
    const n = pos.length / 3;
    const nrm = spec.normals ? (spec.normals instanceof Float32Array ? spec.normals : Float32Array.from(spec.normals)) : vertexNormals(pos, idx, n);
    const m = { id, label: spec.label || id, kind, n, visible: old ? old.visible : true, color: parseColor(spec.color, [0.72, 0.70, 0.68]),
                pos: this.tex("f3", n, pos), nrm: this.tex("f3", n, nrm), ref: null, region: null, lods: [this.makeLod(idx, null)], cpuPos: pos, nrmCPU: nrm, wind: windingSign(pos, idx) };
    this.meshes.set(id, m);
  }

  setOverlay(id, spec) {
    const old = this.overlays.get(id);
    if (!spec) { this.overlays.delete(id); return; }
    const lines = spec.lines ? Float32Array.from(spec.lines) : null, points = spec.points ? Float32Array.from(spec.points) : null;
    this.overlays.set(id, { id, lines, points, color: parseColor(spec.color, [1, 0.8, 0.1]), visible: old ? old.visible : true });
  }

  computeBounds(positions) {
    const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
    for (let i = 0; i < positions.length; i += 3) for (let k = 0; k < 3; k++) { const x = positions[i + k]; if (x < lo[k]) lo[k] = x; if (x > hi[k]) hi[k] = x; }
    return { lo, hi, centre: lo.map((x, k) => (x + hi[k]) / 2), radius: Math.hypot(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]) / 2 };
  }

  frameBox(b, cams = this.cams, instant = false) {
    for (const cam of cams) {
      const half = (cam.goal.fov * DEG) / 2;
      const dist = cam.ortho ? (b.radius * 0.8) / Math.tan(half) : (b.radius * 0.8) / Math.sin(half);
      cam.set({ target: b.centre, dist }, instant);
    }
  }

  rects() {
    const w = this.canvas.clientWidth, h = this.canvas.clientHeight, slots = LAYOUTS[this.layout];
    if (slots.length === 1) return [{ x: 0, y: 0, w, h, slot: 0, cam: slots[0] }];
    if (slots.length === 2) { const hw = Math.floor(w / 2); return [{ x: 0, y: 0, w: hw, h, slot: 0, cam: slots[0] }, { x: hw, y: 0, w: w - hw, h, slot: 1, cam: slots[1] }]; }
    const hw = Math.floor(w / 2), hh = Math.floor(h / 2);
    return [{ x: 0, y: 0, w: hw, h: hh }, { x: hw, y: 0, w: w - hw, h: hh }, { x: 0, y: hh, w: hw, h: h - hh }, { x: hw, y: hh, w: w - hw, h: h - hh }].map((r, i) => ({ ...r, slot: i, cam: slots[i] }));
  }

  rectAt(clientX, clientY) {
    const b = this.canvas.getBoundingClientRect(), x = clientX - b.left, y = clientY - b.top;
    return this.rects().find((r) => x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h) || this.rects()[0];
  }

  activeCam() { const r = this.rects(); return this.cams[(r[this.active] || r[0]).cam]; }
  activeVp() { const r = this.rects(); return this.vp[(r[this.active] || r[0]).slot]; }

  planes() {
    const out = [], b = this.bounds;
    for (const c of this.opts.clip) {
      if (!c.on) continue;
      let n = c.axis === "x" ? [1, 0, 0] : c.axis === "y" ? [0, 1, 0] : c.axis === "z" ? [0, 0, 1] : [Math.cos(c.el * DEG) * Math.sin(c.az * DEG), -Math.cos(c.el * DEG) * Math.cos(c.az * DEG), Math.sin(c.el * DEG)];
      if (c.flip) n = mul(n, -1);
      const d = dot(n, b.centre) + c.offset;
      out.push({ n, d, src: c });
      if (c.slab) out.push({ n: mul(n, -1), d: -(d - c.thick), src: c });
    }
    return out.slice(0, 6);
  }

  boxWorld() {
    const b = this.bounds, bx = this.opts.box, ext = sub(b.hi, b.lo);
    return { lo: b.lo.map((x, k) => x + ext[k] * Math.min(bx.lo[k], bx.hi[k])), hi: b.lo.map((x, k) => x + ext[k] * Math.max(bx.lo[k], bx.hi[k])) };
  }

  lightsWorld(M) {
    const dirs = [], cols = [];
    for (const l of this.opts.lights) {
      if (!l.on || dirs.length >= MAX_LIGHTS) continue;
      const ce = Math.cos(l.el * DEG), se = Math.sin(l.el * DEG), sa = Math.sin(l.az * DEG), ca = Math.cos(l.az * DEG);
      const d = this.opts.follow ? norm(add(add(mul(M.s, sa * ce), mul(M.u, se)), mul(M.f, -ca * ce))) : norm([sa * ce, -ca * ce, se]);
      dirs.push(d);
      cols.push(mul(kelvinRGB(l.k), l.i));
    }
    return { dirs, cols };
  }

  visibleMeshes() { return [...this.meshes.values()].filter((m) => m.visible && m.cpuPos); }

  render(dt) {
    const gl = this.gl, c = this.canvas, dpr = window.devicePixelRatio || 1;
    const w = Math.round(c.clientWidth * dpr), h = Math.round(c.clientHeight * dpr);
    if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
    const rects = this.rects();
    let moving = false;
    for (const r of rects) moving = this.cams[r.cam].step(dt) || moving;
    this.dark = matchMedia("(prefers-color-scheme: dark)").matches;
    gl.enable(gl.SCISSOR_TEST);
    for (const r of rects) this.renderViewport(r, dpr, h);
    gl.disable(gl.SCISSOR_TEST);
    this.placeLabels(rects);
    if (this.pivotFlash > 0) { this.pivotFlash -= dt; moving = true; }
    return moving;
  }

  meshUniforms(p, M, st) {
    const gl = this.gl, o = this.opts;
    gl.uniformMatrix4fv(p.u("mvp"), false, M.mvp);
    gl.uniform3fv(p.u("eye"), M.eye);
    gl.uniform3fv(p.u("camS"), M.s);
    gl.uniform3fv(p.u("camU"), M.u);
    gl.uniform3fv(p.u("camF"), M.f);
    gl.uniform1i(p.u("ortho"), M.ortho ? 1 : 0);
    gl.uniform1i(p.u("mode"), st.mode);
    const L = this.lightsWorld(M), dirs = new Float32Array(12), cols = new Float32Array(12);
    L.dirs.forEach((d, i) => dirs.set(d, i * 3));
    L.cols.forEach((x, i) => cols.set(x, i * 3));
    gl.uniform1i(p.u("nLights"), L.dirs.length);
    gl.uniform3fv(p.u("lightDir"), dirs);
    gl.uniform3fv(p.u("lightCol"), cols);
    gl.uniform3fv(p.u("sky"), mul([0.92, 0.96, 1.04], o.ambient));
    gl.uniform3fv(p.u("ground"), mul([0.42, 0.36, 0.32].map((x, i) => x + ([0.92, 0.96, 1.04][i] - x) * o.ground), o.ambient));
    gl.uniform1f(p.u("aniso"), o.aniso);
    gl.uniform1f(p.u("exposure"), o.exposure);
    gl.uniform1i(p.u("wireOn"), o.wire ? 1 : 0);
    gl.uniform1f(p.u("wireW"), o.wireW);
    gl.uniform1i(p.u("wireFade"), o.wireFade ? 1 : 0);
    gl.uniform3fv(p.u("wireCol"), this.dark ? [0.92, 0.92, 0.95] : [0.08, 0.10, 0.14]);
    gl.uniform1i(p.u("xrayPass"), 0);
    gl.uniform1i(p.u("backHi"), o.backHi ? 1 : 0);
    gl.uniform1i(p.u("seamOn"), o.seams ? 1 : 0);
    gl.uniform3fv(p.u("bgCol"), this.bg);
    gl.uniform3fv(p.u("interiorCol"), [0.95, 0.55, 0.15]);
    const heat = [o.heat.quality, o.heat.displacement, o.heat.curvature][st.mode - 8];
    gl.uniform1f(p.u("heatMax"), Math.max(1e-6, heat || 1));
    gl.uniform1ui(p.u("hidMask"), o.hiddenRegions >>> 0);
    this.clipUniforms(p, -1);
  }

  clipUniforms(p, skip) {
    const gl = this.gl, P = this.planes(), arr = new Float32Array(24);
    P.forEach((q, i) => arr.set([...q.n, q.d], i * 4));
    gl.uniform4fv(p.u("planes"), arr);
    gl.uniform1i(p.u("nPlanes"), P.length);
    gl.uniform1i(p.u("skipPlane"), skip);
    const box = this.boxWorld();
    gl.uniform1i(p.u("boxOn"), this.opts.box.on ? 1 : 0);
    gl.uniform3fv(p.u("boxMin"), box.lo);
    gl.uniform3fv(p.u("boxMax"), box.hi);
    return P;
  }

  bindMesh(p, m, lod) {
    const gl = this.gl, D = this.dummy;
    const bind = (unit, r, dummy) => { gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, (r || dummy).t); };
    bind(0, lod.idx, D.u1); bind(1, m.pos, D.f3); bind(2, m.nrm, D.f3); bind(3, m.ref, D.f3); bind(4, lod.uv, D.f2); bind(5, lod.flags, D.b1); bind(6, m.region, D.b1);
    if (p === this.meshProg) {
      gl.uniform1f(p.u("windSign"), m.wind || 1);
      gl.uniform1i(p.u("hasRef"), m.ref ? 1 : 0);
      gl.uniform1i(p.u("hasUV"), lod.uv ? 1 : 0);
      gl.uniform1i(p.u("hasFlags"), lod.flags ? 1 : 0);
      const lin = m.color.map((x) => Math.pow(x, 2.2));
      gl.uniform3fv(p.u("baseCol"), lin);
    }
    gl.uniform1i(p.u("hasRegion"), m.region ? 1 : 0);
    gl.frontFace((m.wind || 1) > 0 ? gl.CCW : gl.CW);
  }

  lodOf(m, st) { return m.kind === "head" ? m.lods[Math.min(st.lod, m.lods.length - 1)] : m.lods[0]; }

  renderViewport(r, dpr, H) {
    const gl = this.gl, o = this.opts, cam = this.cams[r.cam], st = this.vp[r.slot];
    const x = Math.round(r.x * dpr), y = Math.round(H - (r.y + r.h) * dpr), w = Math.max(1, Math.round(r.w * dpr)), h = Math.max(1, Math.round(r.h * dpr));
    gl.viewport(x, y, w, h);
    gl.scissor(x, y, w, h);
    this.bg = this.dark ? [0.11, 0.11, 0.12] : [0.93, 0.93, 0.92];
    gl.clearColor(...this.bg, 1);
    gl.clearStencil(0);
    gl.colorMask(true, true, true, true);
    gl.depthMask(true);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT | gl.STENCIL_BUFFER_BIT);
    const M = cam.matrices(w / h);
    const meshes = this.visibleMeshes();
    gl.bindVertexArray(this.emptyVao);
    gl.enable(gl.DEPTH_TEST);
    gl.depthFunc(gl.LEQUAL);
    gl.disable(gl.BLEND);
    gl.disable(gl.STENCIL_TEST);
    gl.disable(gl.CULL_FACE);
    if (o.points) { gl.enable(gl.POLYGON_OFFSET_FILL); gl.polygonOffset(1, 1); }
    const mp = this.meshProg;
    gl.useProgram(mp);
    this.meshUniforms(mp, M, st);
    for (const m of meshes) { const lod = this.lodOf(m, st); this.bindMesh(mp, m, lod); gl.drawArrays(gl.TRIANGLES, 0, lod.tri * 3); }
    gl.disable(gl.POLYGON_OFFSET_FILL);
    const P = this.planes();
    if (o.caps && P.length) {
      gl.enable(gl.STENCIL_TEST);
      for (let j = 0; j < P.length; j++) {
        gl.clear(gl.STENCIL_BUFFER_BIT);
        gl.colorMask(false, false, false, false);
        gl.depthMask(false);
        gl.disable(gl.DEPTH_TEST);
        gl.stencilFunc(gl.ALWAYS, 0, 0xff);
        gl.stencilOp(gl.KEEP, gl.KEEP, gl.INVERT);
        gl.useProgram(mp);
        gl.bindVertexArray(this.emptyVao);
        for (const m of meshes) { const lod = this.lodOf(m, st); this.bindMesh(mp, m, lod); gl.drawArrays(gl.TRIANGLES, 0, lod.tri * 3); }
        gl.colorMask(true, true, true, true);
        gl.depthMask(true);
        gl.enable(gl.DEPTH_TEST);
        gl.depthFunc(gl.LESS);
        gl.stencilFunc(gl.NOTEQUAL, 0, 0xff);
        gl.stencilOp(gl.KEEP, gl.KEEP, gl.KEEP);
        const q = P[j], t1 = norm(Math.abs(q.n[2]) < 0.9 ? cross(q.n, [0, 0, 1]) : cross(q.n, [1, 0, 0])), t2 = cross(q.n, t1);
        const c0 = sub(this.bounds.centre, mul(q.n, dot(q.n, this.bounds.centre) - q.d)), s = this.bounds.radius * 4;
        const corner = (a, b) => add(c0, add(mul(t1, a * s), mul(t2, b * s)));
        this.drawLines(M, [...corner(-1, -1), ...corner(1, -1), ...corner(1, 1), ...corner(-1, -1), ...corner(1, 1), ...corner(-1, 1)], [0.36, 0.45, 0.62], gl.TRIANGLES, j);
        gl.depthFunc(gl.LEQUAL);
      }
      gl.disable(gl.STENCIL_TEST);
    }
    if (o.wire && o.xray) {
      gl.useProgram(mp);
      gl.bindVertexArray(this.emptyVao);
      gl.uniform1i(mp.u("xrayPass"), 1);
      gl.depthFunc(gl.GREATER);
      gl.depthMask(false);
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      for (const m of meshes) { const lod = this.lodOf(m, st); this.bindMesh(mp, m, lod); gl.drawArrays(gl.TRIANGLES, 0, lod.tri * 3); }
      gl.disable(gl.BLEND);
      gl.depthMask(true);
      gl.depthFunc(gl.LEQUAL);
    }
    if (o.points) {
      const pp = this.pointProg;
      gl.useProgram(pp);
      gl.bindVertexArray(this.emptyVao);
      gl.uniformMatrix4fv(pp.u("mvp"), false, M.mvp);
      gl.uniform1f(pp.u("psize"), o.pointSize * dpr);
      gl.uniform3fv(pp.u("col"), [0.95, 0.35, 0.10]);
      gl.uniform1ui(pp.u("hidMask"), o.hiddenRegions >>> 0);
      this.clipUniforms(pp, -1);
      for (const m of meshes) {
        const lod = this.lodOf(m, st);
        this.bindMesh(pp, m, lod);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, lod.verts.t);
        gl.drawArrays(gl.POINTS, 0, lod.vertsCPU.length);
      }
    }
    gl.disable(gl.DEPTH_TEST);
    for (const ov of this.overlays.values()) {
      if (!ov.visible) continue;
      if (ov.lines && ov.lines.length >= 6) this.drawLines(M, ov.lines, ov.color, gl.LINES);
      if (ov.points && ov.points.length >= 3) this.drawLines(M, ov.points, ov.color, gl.POINTS, -1, 7 * dpr);
    }
    const centre = this.bounds.centre, rad = this.bounds.radius;
    if (o.lightGizmo) {
      const L = this.lightsWorld(M);
      L.dirs.forEach((d, i) => {
        const tip = add(centre, mul(d, rad * 1.25)), col = L.cols[i].map((v) => Math.min(1, Math.pow(v, 1 / 2.2)));
        this.drawLines(M, [...add(centre, mul(d, rad * 0.55)), ...tip], col, gl.LINES);
        this.drawLines(M, tip, col, gl.POINTS, -1, 12 * dpr);
      });
    }
    if (o.planeGizmo && P.length) {
      for (const q of P) {
        const t1 = norm(Math.abs(q.n[2]) < 0.9 ? cross(q.n, [0, 0, 1]) : cross(q.n, [1, 0, 0])), t2 = cross(q.n, t1);
        const c0 = sub(centre, mul(q.n, dot(q.n, centre) - q.d)), s = rad * 0.9;
        const k = [[-1, -1], [1, -1], [1, 1], [-1, 1]].map(([a, b]) => add(c0, add(mul(t1, a * s), mul(t2, b * s))));
        this.drawLines(M, [...k[0], ...k[1], ...k[1], ...k[2], ...k[2], ...k[3], ...k[3], ...k[0], ...c0, ...add(c0, mul(q.n, rad * 0.3))], [0.25, 0.55, 0.95], gl.LINES);
      }
    }
    if (o.box.on) {
      const b = this.boxWorld(), v = (i) => [i & 1 ? b.hi[0] : b.lo[0], i & 2 ? b.hi[1] : b.lo[1], i & 4 ? b.hi[2] : b.lo[2]];
      const e = [];
      for (let i = 0; i < 8; i++) for (const bit of [1, 2, 4]) if (!(i & bit)) e.push(...v(i), ...v(i | bit));
      this.drawLines(M, e, [0.9, 0.3, 0.6], gl.LINES);
    }
    if (this.pivotFlash > 0 && r.slot === this.active) {
      const t = cam.cur.target, s = rad * 0.06;
      this.drawLines(M, [t[0] - s, t[1], t[2], t[0] + s, t[1], t[2], t[0], t[1] - s, t[2], t[0], t[1] + s, t[2], t[0], t[1], t[2] - s, t[0], t[1], t[2] + s], [1, 0.85, 0.2], gl.LINES);
    }
    gl.enable(gl.DEPTH_TEST);
  }

  drawLines(M, data, col, prim, capPlane = -1, psize = 1) {
    const gl = this.gl, p = this.lineProg, arr = data instanceof Float32Array ? data : new Float32Array(data);
    gl.useProgram(p);
    gl.bindVertexArray(this.lineVao);
    gl.bindBuffer(gl.ARRAY_BUFFER, this.lineBuf);
    gl.bufferData(gl.ARRAY_BUFFER, arr, gl.STREAM_DRAW);
    gl.uniformMatrix4fv(p.u("mvp"), false, M.mvp);
    gl.uniform3fv(p.u("col"), col);
    gl.uniform1f(p.u("psize"), psize);
    gl.uniform1i(p.u("capPass"), capPlane >= 0 ? 1 : 0);
    if (capPlane >= 0) this.clipUniforms(p, capPlane);
    gl.drawArrays(prim, 0, arr.length / 3);
    gl.bindVertexArray(this.emptyVao);
  }

  placeLabels(rects) {
    if (!this.labels) return;
    const key = `${this.layout}|${this.active}|${rects.map((r) => `${this.cams[r.cam].ortho}${this.cams[r.cam].name}${this.vp[r.slot].mode}${this.vp[r.slot].lod}`).join(",")}`;
    if (key === this.labelKey) return;
    this.labelKey = key;
    this.labels.innerHTML = "";
    for (const r of rects) {
      const el = document.createElement("div");
      el.className = `vp${r.slot === this.active && rects.length > 1 ? " active" : ""}`;
      el.style.left = `${r.x}px`; el.style.top = `${r.y}px`; el.style.width = `${r.w}px`; el.style.height = `${r.h}px`;
      const cam = this.cams[r.cam], st = this.vp[r.slot], head = this.meshes.get("head");
      const lod = head && head.lods.length > 1 ? ` · LOD${st.lod}` : "";
      el.innerHTML = `<span>${cam.name} · ${cam.ortho ? "ortho" : "persp"} · ${MODES[st.mode].label}${lod}</span>`;
      this.labels.appendChild(el);
    }
  }

  pick(clientX, clientY) {
    const r = this.rectAt(clientX, clientY), b = this.canvas.getBoundingClientRect();
    const nx = ((clientX - b.left - r.x) / r.w) * 2 - 1, ny = 1 - ((clientY - b.top - r.y) / r.h) * 2;
    const cam = this.cams[r.cam], { o, d } = cam.ray(nx, ny, r.w / r.h), st = this.vp[r.slot];
    const P = this.planes(), box = this.boxWorld();
    const cut = (p) => P.some((q) => dot(q.n, p) > q.d) || (this.opts.box.on && p.every((x, k) => x > box.lo[k] && x < box.hi[k]));
    let best = null;
    for (const m of this.visibleMeshes()) {
      const pos = m.cpuPos, idx = this.lodOf(m, st).idxCPU;
      for (let t = 0; t < idx.length; t += 3) {
        const a = idx[t] * 3, bb = idx[t + 1] * 3, c = idx[t + 2] * 3;
        const e1 = [pos[bb] - pos[a], pos[bb + 1] - pos[a + 1], pos[bb + 2] - pos[a + 2]], e2 = [pos[c] - pos[a], pos[c + 1] - pos[a + 1], pos[c + 2] - pos[a + 2]];
        const pv = cross(d, e2), det = dot(e1, pv);
        if (Math.abs(det) < 1e-14) continue;
        const inv = 1 / det, tv = [o[0] - pos[a], o[1] - pos[a + 1], o[2] - pos[a + 2]], u = dot(tv, pv) * inv;
        if (u < 0 || u > 1) continue;
        const qv = cross(tv, e1), v = dot(d, qv) * inv;
        if (v < 0 || u + v > 1) continue;
        const dist = dot(e2, qv) * inv;
        if (dist <= 0 || (best && dist >= best.t)) continue;
        const hit = add(o, mul(d, dist));
        if (cut(hit)) continue;
        best = { t: dist, point: hit, mesh: m.id, slot: r.slot };
      }
    }
    return best;
  }

  screenshot(which = "viewport") {
    const dpr = window.devicePixelRatio || 1;
    if (which === "layout") return this.canvas.toDataURL("image/png");
    const r = this.rects()[this.active] || this.rects()[0];
    const out = document.createElement("canvas");
    out.width = Math.round(r.w * dpr);
    out.height = Math.round(r.h * dpr);
    out.getContext("2d").drawImage(this.canvas, Math.round(r.x * dpr), Math.round(r.y * dpr), out.width, out.height, 0, 0, out.width, out.height);
    return out.toDataURL("image/png");
  }
}
