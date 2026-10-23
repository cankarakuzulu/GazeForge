import { tr } from "./i18n.js";
const TW = 2048;
const DEG = Math.PI / 180;

export const MODES = [
  { id: "clay" },
  { id: "flat" },
  { id: "gouraud" },
  { id: "blinn" },
  { id: "aniso" },
  { id: "matcap" },
  { id: "normals" },
  { id: "hidden" },
  { id: "quality" },
  { id: "displacement" },
  { id: "curvature" },
  { id: "uv" },
];
export const MODE_INDEX = Object.fromEntries(MODES.map((m, i) => [m.id, i]));
const LIT_MODES = new Set([0, 1, 3, 4]);

export const LIGHT_PRESETS = {
  studio: { ambient: 0.16, ground: 0.4, lights: [{ az: -40, el: 30, i: 1.0, k: 5600 }, { az: 50, el: 10, i: 0.35, k: 6500 }, { az: 160, el: 35, i: 0.8, k: 7500 }] },
  flat: { ambient: 0.9, ground: 1.0, lights: [] },
  rim: { ambient: 0.06, ground: 0.4, lights: [{ az: 150, el: 20, i: 1.1, k: 7000 }, { az: -150, el: 20, i: 1.1, k: 7000 }, { az: 0, el: 10, i: 0.15, k: 5000 }] },
  top: { ambient: 0.1, ground: 0.3, lights: [{ az: 0, el: 85, i: 1.2, k: 5500 }] },
  key: { ambient: 0.08, ground: 0.4, lights: [{ az: -35, el: 25, i: 1.2, k: 5200 }] },
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
uniform vec3 baseCol; uniform float aniso; uniform int matOn;
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
uniform highp usampler2D flagT; uniform highp usampler2D regT; uniform sampler2D chartT; uniform int gridOn; uniform int hasChart;
uniform mat4 mvp; uniform float windSign; uniform uint hidMask; uniform int hasRegion; uniform int hasRef; uniform int hasUV; uniform int hasFlags;
${COMMON}
out vec3 vp; out vec3 vn; out vec3 bary; out vec3 gcol; out float disp; out vec2 uv; out vec2 ch;
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
  if (matOn == 1 && hasUV == 1) uv = texelFetch(uvT, at(c), 0).xy;
  if (hasFlags == 1) eflags = texelFetch(flagT, at(t), 0).r;
  ch = vec2(0.0);
  if (gridOn == 1 && hasChart == 1) {
    vec2 c0 = texelFetch(chartT, at(i0), 0).xy;
    vec2 cm = k == 0 ? c0 : texelFetch(chartT, at(me), 0).xy;
    cm.x -= 360.0 * floor((cm.x - c0.x + 180.0) / 360.0);
    ch = cm;
  }
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
in vec3 vp; in vec3 vn; in vec3 bary; in vec3 gcol; in float disp; in vec2 uv; in vec2 ch;
flat in vec3 fn; flat in float tq; flat in float tc; flat in uint eflags;
${COMMON}
${CLIP}
uniform int gridOn; uniform int hasChart; uniform vec2 gridStep; uniform vec3 gridInk;
uniform int wireOn; uniform float wireW; uniform vec3 wireCol; uniform int xrayPass; uniform int wireFade; uniform int backHi; uniform int seamOn;
uniform vec3 bgCol; uniform vec3 interiorCol; uniform float heatMax; uniform int hasRef; uniform int hasUV; uniform int hasFlags; uniform float exposure;
uniform vec4 matCol; uniform int matHasTex; uniform sampler2D matTex; uniform float matRough; uniform int matGlass;
out vec4 o;
vec3 glints(vec3 n, vec3 v, float rough) {
  float sh = 2.0 / max(rough * rough, 1e-4);
  vec3 s = vec3(0.0);
  for (int i = 0; i < 4; i++) {
    if (i >= nLights) break;
    vec3 h = normalize(lightDir[i] + v);
    s += lightCol[i] * pow(max(dot(n, h), 0.0), sh) * step(0.0, dot(n, lightDir[i]));
  }
  return s * (sh + 2.0) / 8.0;
}
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
  if (matOn == 1 && matGlass == 1) {
    if (!front) discard;
    float f = 0.04 + 0.96 * pow(1.0 - clamp(dot(n, v), 0.0, 1.0), 5.0);
    vec3 s = (glints(n, v, matRough) + hemi(reflect(-v, n))) * f * exp2(exposure);
    o = vec4(min(s, vec3(1.0)), clamp(matCol.a * (1.0 - f) + f, 0.0, 1.0));
    return;
  }
  vec3 alb = baseCol;
  if (matOn == 1) alb = matCol.rgb * (matHasTex == 1 ? pow(texture(matTex, uv).rgb, vec3(2.2)) : vec3(1.0));
  float hl = 0.55 + 0.45 * abs(dot(n, v));
  vec3 c;
  bool lit = true;
  if (!front && nPlanes > 0) { c = interiorCol * (0.45 + 0.55 * abs(dot(n, v))); lit = false; }
  else if (mode == 0) c = shade(0, n, v, alb);
  else if (mode == 1) { vec3 f = normalize(fn); if (!front) f = -f; c = shade(1, f, v, alb); }
  else if (mode == 2) c = gcol;
  else if (mode == 3) c = shade(matOn == 1 ? 1 : 2, n, v, alb);
  else if (mode == 4) c = shade(matOn == 1 ? 1 : 3, n, v, alb);
  else if (mode == 5) { c = matcap(n); lit = false; }
  else if (mode == 6) { c = normalize(vn) * 0.5 + 0.5; lit = false; }
  else if (mode == 7) { c = bgCol; lit = false; }
  else if (mode == 8) { c = ramp(1.0 - clamp(tq / heatMax, 0.0, 1.0)) * hl; lit = false; }
  else if (mode == 9) { c = (hasRef == 1 ? diverge(disp / heatMax) : vec3(0.55)) * hl; lit = false; }
  else if (mode == 10) { c = diverge(tc / heatMax) * hl; lit = false; }
  else { c = (hasUV == 1 ? checker(uv) : vec3(0.55)) * hl; lit = false; }
  if (lit && matOn == 1) c += glints(n, v, matRough) * 0.04;
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
  if (gridOn == 1 && hasChart == 1) {
    vec2 g = vec2(ch.x / gridStep.x + 0.5, ch.y / gridStep.y);
    vec2 fw2 = max(fwidth(g), vec2(1e-6));
    vec2 dl = abs(fract(g + 0.5) - 0.5) / fw2;
    float major = 1.0 - smoothstep(0.6, 1.6, min(dl.x, dl.y));
    vec2 fw3 = fw2 * 3.0, d3 = abs(fract(g * 3.0 + 0.5) - 0.5) / fw3;
    float minor = (1.0 - smoothstep(0.3, 1.1, min(d3.x, d3.y))) * (1.0 - smoothstep(0.1, 0.25, max(fw3.x, fw3.y)));
    float mid = 1.0 - smoothstep(0.6, 1.8, abs(ch.x - 360.0 * floor(ch.x / 360.0 + 0.5)) / max(fwidth(ch.x), 1e-6));
    c = mix(c, gridInk, max(major * 0.85, minor * 0.45));
    c = mix(c, vec3(0.92, 0.22, 0.18), mid * 0.85);
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
      box: { on: false, lo: [0.4, 0, 0.4], hi: [1, 0.5, 1] }, hiddenRegions: 0, faceGrid: false, normalGrid: false, gridLabels: true, eyesGrid: "grid",
    };
    while (this.opts.lights.length < MAX_LIGHTS) this.opts.lights.push({ on: false, az: 0, el: 0, i: 0.5, k: 6500 });
    this.bounds = { lo: [-0.1, -0.1, -0.1], hi: [0.1, 0.1, 0.1], centre: [0, 0, 0], radius: 0.15 };
    this.pivotFlash = 0;
    this.chartDef = null;
    this.chartAnchors = [];
    this.markerSets = new Map();
    this.flash = null;
    this.overlayKey = "";
    this.overlayStats = { labels: 0, markers: 0 };
    if (labels && labels.parentElement) {
      this.gridLayer = document.createElement("div");
      this.gridLayer.className = "gridlayer";
      labels.parentElement.insertBefore(this.gridLayer, labels.nextSibling);
    }
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
    const units = { idxT: 0, posT: 1, nrmT: 2, refT: 3, uvT: 4, flagT: 5, regT: 6, chartT: 7, matTex: 8 };
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
      rgba: this.materialTexture(1, 1, new Uint8Array([255, 255, 255, 255])),
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
      m.chart = m.chartCPU ? this.tex("f2", m.n, m.chartCPU) : null;
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
    const m = { id: "head", label: tr("mesh.head"), kind: "head", n, visible: true, color: [0.80, 0.74, 0.68], pos: this.tex("f3", n), nrm: this.tex("f3", n), ref: null, region: null, lods: [], cpuPos: null };
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

  setChart(uv, def, anchors) {
    const m = this.meshes.get("head");
    this.drop(m.chart);
    m.chartCPU = uv;
    m.chart = this.tex("f2", m.n, uv);
    this.chartDef = def;
    this.chartAnchors = anchors || [];
    this.overlayKey = "";
  }

  setMarkers(id, list) {
    if (list == null) this.markerSets.delete(id); else this.markerSets.set(id, list);
    this.overlayKey = "";
  }

  updateHead(positions, normals) {
    const m = this.meshes.get("head");
    if (!m) return false;
    this.upload(m.pos, positions);
    this.upload(m.nrm, normals);
    m.nrmCPU = normals;
    m.poseVersion = (m.poseVersion || 0) + 1;
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
    if (old && old.kind !== "head") { this.drop(old.chart); this.drop(old.pos); this.drop(old.nrm); for (const l of old.lods) { this.drop(l.idx); this.drop(l.verts); this.drop(l.uv); this.drop(l.flags); } this.meshes.delete(id); }
    if (!spec) return;
    const pos = spec.positions instanceof Float32Array ? spec.positions : Float32Array.from(spec.positions);
    const idx = spec.indices instanceof Uint32Array ? spec.indices : Uint32Array.from(spec.indices);
    const n = pos.length / 3;
    const nrm = spec.normals ? (spec.normals instanceof Float32Array ? spec.normals : Float32Array.from(spec.normals)) : vertexNormals(pos, idx, n);
    let corner = null;
    if (spec.uvs && spec.uvs.length === n * 2) {
      corner = new Float32Array(idx.length * 2);
      for (let i = 0; i < idx.length; i++) { corner[2 * i] = spec.uvs[2 * idx[i]]; corner[2 * i + 1] = spec.uvs[2 * idx[i] + 1]; }
    }
    const m = { id, label: spec.label || id, kind, n, visible: old ? old.visible : true, color: parseColor(spec.color, [0.72, 0.70, 0.68]),
                pos: this.tex("f3", n, pos), nrm: this.tex("f3", n, nrm), ref: null, region: null, lods: [this.makeLod(idx, corner)], cpuPos: pos, nrmCPU: nrm, wind: windingSign(pos, idx),
                parts: spec.parts && spec.look ? spec.parts : null, look: spec.parts && spec.look ? spec.look : null };
    this.meshes.set(id, m);
  }

  materialTexture(width, height, rgba) {
    const gl = this.gl, t = gl.createTexture();
    gl.activeTexture(gl.TEXTURE8);
    gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, width, height, 0, gl.RGBA, gl.UNSIGNED_BYTE, rgba);
    gl.generateMipmap(gl.TEXTURE_2D);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    gl.activeTexture(gl.TEXTURE0);
    return t;
  }

  drawParts(p, m, glass) {
    const gl = this.gl;
    for (const part of m.parts) {
      const look = m.look[part.look];
      if (!look || !!look.glass !== glass) continue;
      gl.uniform1i(p.u("matOn"), 1);
      gl.uniform4fv(p.u("matCol"), look.colour);
      gl.uniform1f(p.u("matRough"), look.roughness);
      gl.uniform1i(p.u("matGlass"), look.glass ? 1 : 0);
      gl.uniform1i(p.u("matHasTex"), look.tex ? 1 : 0);
      gl.activeTexture(gl.TEXTURE8);
      gl.bindTexture(gl.TEXTURE_2D, look.tex || this.dummy.rgba);
      gl.drawArrays(gl.TRIANGLES, part.first * 3, part.count * 3);
    }
    gl.uniform1i(p.u("matOn"), 0);
    gl.activeTexture(gl.TEXTURE0);
  }

  poseExtra(id, rot, centre) {
    const e = this.meshes.get(id);
    if (!e || e.kind === "head") return false;
    if (!e.rest) e.rest = { pos: e.cpuPos, nrm: e.nrmCPU };
    let pos = e.rest.pos, nrm = e.rest.nrm;
    if (rot) {
      const c = centre || [0, 0, 0], P = e.rest.pos, N = e.rest.nrm;
      pos = new Float32Array(P.length);
      nrm = new Float32Array(N.length);
      for (let g = 0; g < P.length; g += 3) {
        const x = P[g] - c[0], y = P[g + 1] - c[1], z = P[g + 2] - c[2], a = N[g], b = N[g + 1], d = N[g + 2];
        pos[g] = rot[0] * x + rot[1] * y + rot[2] * z + c[0];
        pos[g + 1] = rot[3] * x + rot[4] * y + rot[5] * z + c[1];
        pos[g + 2] = rot[6] * x + rot[7] * y + rot[8] * z + c[2];
        nrm[g] = rot[0] * a + rot[1] * b + rot[2] * d;
        nrm[g + 1] = rot[3] * a + rot[4] * b + rot[5] * d;
        nrm[g + 2] = rot[6] * a + rot[7] * b + rot[8] * d;
      }
    }
    if (pos === e.cpuPos) return true;
    this.drop(e.pos);
    this.drop(e.nrm);
    e.pos = this.tex("f3", e.n, pos);
    e.nrm = this.tex("f3", e.n, nrm);
    e.cpuPos = pos;
    e.nrmCPU = nrm;
    return true;
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

  visibleMeshes() {
    const hideEyes = this.opts.faceGrid && this.chartDef && this.opts.eyesGrid === "hide";
    return [...this.meshes.values()].filter((m) => m.visible && m.cpuPos && !(hideEyes && m.kind === "eyes"));
  }

  render(dt) {
    const gl = this.gl, c = this.canvas, dpr = window.devicePixelRatio || 1;
    const w = Math.round(c.clientWidth * dpr), h = Math.round(c.clientHeight * dpr);
    if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
    const rects = this.rects();
    let moving = false;
    for (const r of rects) moving = this.cams[r.cam].step(dt) || moving;
    this.dark = document.documentElement.dataset.theme !== "light";
    this.syncEyeChart();
    gl.enable(gl.SCISSOR_TEST);
    for (const r of rects) this.renderViewport(r, dpr, h);
    gl.disable(gl.SCISSOR_TEST);
    this.placeLabels(rects);
    if (this.placeOverlay(rects)) moving = true;
    if (this.pivotFlash > 0) { this.pivotFlash -= dt; moving = true; }
    if (this.flash) { if (performance.now() < this.flash.until) moving = true; else { this.flash = null; moving = true; } }
    return moving;
  }

  markerList() {
    const out = [];
    for (const v of this.markerSets.values()) {
      const list = typeof v === "function" ? v() : v;
      if (Array.isArray(list)) for (const mk of list) if (mk && mk.pos) out.push(mk);
    }
    return out;
  }

  rulerAxes(M) {
    const pick = (v) => { let k = 0; for (let i = 1; i < 3; i++) if (Math.abs(v[i]) > Math.abs(v[k])) k = i; return Math.abs(v[k]) > 0.999 ? { k, sign: Math.sign(v[k]) } : null; };
    const s = pick(M.s), u = pick(M.u);
    return s && u ? { s, u } : null;
  }

  rulerSteps(pxPerMm) {
    const major = pxPerMm * 10 >= 9 ? 10 : pxPerMm * 50 >= 9 ? 50 : 100;
    const minor = pxPerMm >= 6 ? 1 : pxPerMm * 2 >= 6 ? 2 : pxPerMm * 5 >= 6 && major === 10 ? 5 : 0;
    return { major, minor };
  }

  drawRuler(M, r, cam) {
    const gl = this.gl, bg = this.bg, ink = this.dark ? [0.75, 0.80, 0.90] : [0.20, 0.26, 0.36];
    const tone = (a) => bg.map((x, i) => x + (ink[i] - x) * a);
    const labels = [];
    this.rulerLabels = this.rulerLabels || [];
    this.rulerLabels[r.slot] = labels;
    if (M.ortho) {
      const ax = this.rulerAxes(M);
      const c = cam.cur.target, hh = M.halfH, hw = M.halfH * M.aspect, pxPerMm = r.h / (2 * hh * 1000), st = this.rulerSteps(pxPerMm);
      const every = (gap) => [10, 20, 50, 100, 200, 500].find((s) => s % st.major === 0 && s * pxPerMm >= gap) || 1000;
      const labelStep = { true: every(44), false: every(18) };
      const along = (dir, t) => mul(dir, t);
      const groups = { major: [], minor: [], zero: [] };
      const lines = (axis, dir, other, half, otherHalf, vertical) => {
        if (!axis) return;
        const k = axis.k, sign = axis.sign, lo = c[k] - half, hi = c[k] + half;
        const step = st.minor || st.major;
        for (let mm = Math.ceil((lo * 1000) / step) * step; mm <= hi * 1000; mm += step) {
          const t = (mm / 1000 - c[k]) * sign, base = add(c, along(dir, t));
          const seg = [...add(base, along(other, -otherHalf * 1.05)), ...add(base, along(other, otherHalf * 1.05))];
          const g = mm === 0 ? "zero" : mm % st.major === 0 ? "major" : "minor";
          groups[g].push(...seg);
          if (g !== "minor" && this.opts.gridLabels && mm % labelStep[vertical] === 0) {
            const f = (t / (vertical ? hw : hh) + 1) / 2;
            labels.push(vertical ? { x: r.x + f * r.w, y: r.y + r.h - 14, text: `${mm}` } : { x: r.x + 4, y: r.y + (1 - f) * r.h, text: `${mm}` });
          }
        }
      };
      if (ax) {
        lines(ax.s, M.s, M.u, hw, hh, true);
        lines(ax.u, M.u, M.s, hh, hw, false);
        const name = (a) => "xyz"[a.k];
        labels.push({ x: r.x + r.w - 92, y: r.y + r.h - 30, text: `${name(ax.s)} → · ${name(ax.u)} ↑ · mm`, cls: "axis" });
      } else {
        labels.push({ x: r.x + r.w - 160, y: r.y + r.h - 30, text: tr("viewer.mm_grid_hint"), cls: "axis" });
      }
      if (groups.minor.length) this.drawLines(M, groups.minor, tone(0.16), gl.LINES);
      if (groups.major.length) this.drawLines(M, groups.major, tone(0.42), gl.LINES);
      if (groups.zero.length) this.drawLines(M, groups.zero, tone(0.75), gl.LINES);
      return;
    }
    const f = M.f, b = this.bounds;
    let k = 0;
    for (let i = 1; i < 3; i++) if (Math.abs(f[i]) > Math.abs(f[k])) k = i;
    const plane = f[k] > 0 ? b.hi[k] + 0.02 : b.lo[k] - 0.02, others = [0, 1, 2].filter((i) => i !== k), out = [], ext = b.radius * 1.3;
    for (const a of others) {
      const o2 = others.find((i) => i !== a);
      for (let mm = Math.ceil(((b.centre[a] - ext) * 1000) / 10) * 10; mm <= (b.centre[a] + ext) * 1000; mm += 10) {
        const p = [0, 0, 0], q = [0, 0, 0];
        p[k] = q[k] = plane; p[a] = q[a] = mm / 1000;
        p[o2] = b.centre[o2] - ext; q[o2] = b.centre[o2] + ext;
        out.push(...p, ...q);
      }
    }
    this.drawLines(M, out, tone(0.3), gl.LINES);
    labels.push({ x: r.x + r.w - 150, y: r.y + r.h - 30, text: tr("viewer.back_plane", { axis: "xyz"[k] }), cls: "axis" });
  }

  project(M, r, p) {
    const m = M.mvp, x = m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12], y = m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13], w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
    if (w <= 1e-9) return null;
    const nx = x / w, ny = y / w;
    if (Math.abs(nx) > 1 || Math.abs(ny) > 1) return null;
    return { x: r.x + ((nx + 1) / 2) * r.w, y: r.y + ((1 - ny) / 2) * r.h };
  }

  facing(M, p, n) {
    const v = M.ortho ? mul(M.f, -1) : norm(sub(M.eye, p));
    return dot(n, v);
  }

  syncEyeChart() {
    const def = this.chartDef, want = this.opts.faceGrid && def && this.opts.eyesGrid === "grid";
    if (!want) return;
    for (const m of this.meshes.values()) {
      if (m.kind !== "eyes" || !m.cpuPos || m.chartSrc === m.cpuPos) continue;
      const c = def.centre_mm.map((x) => x / 1000), f = def.front, top = def.top_mm / 1000, uv = new Float32Array(m.n * 2), P = m.cpuPos;
      for (let i = 0; i < m.n; i++) {
        const rx = P[3 * i] - c[0], ry = P[3 * i + 1] - c[1];
        let u = (Math.atan2(f[0] * ry - f[1] * rx, f[0] * rx + f[1] * ry) * 180) / Math.PI;
        if (u <= -180) u += 360;
        uv[2 * i] = u;
        uv[2 * i + 1] = (top - P[3 * i + 2]) * 1000;
      }
      this.drop(m.chart);
      m.chart = this.tex("f2", m.n, uv);
      m.chartSrc = m.cpuPos;
    }
  }

  overlayObstacles() {
    const b = this.canvas.getBoundingClientRect(), out = [], view = this.canvas.parentElement;
    const add = (el, kind) => {
      if (!el) return;
      const q = el.getBoundingClientRect();
      if (q.width > 0 && q.height > 0) out.push({ x0: q.left - b.left, y0: q.top - b.top, x1: q.right - b.left, y1: q.bottom - b.top, kind });
    };
    if (this.labels) for (const s of this.labels.querySelectorAll(".vp span")) add(s, "caption");
    if (view) {
      for (const s of view.querySelectorAll(".overlay .row, .overlay .label")) add(s, "toolbar");
      for (const s of view.querySelectorAll(".vcube")) add(s, "cube");
      add(view.querySelector(".addr:not(.hidden)"), "card");
    }
    return out;
  }

  axisInfo() {
    const head = this.meshes.get("head"), def = this.chartDef, ch = head?.chartCPU;
    if (!ch || !def) return null;
    if (this.axisCache && this.axisCache.ch === ch) return this.axisCache;
    const span = def.axis_span_deg || 45, front = def.letters.indexOf(def.front_column || "M"), cols = def.letters.length;
    const cellKey = (g) => {
      const k = Math.floor(ch[2 * g] / def.column_deg + 0.5), col = (((k + front) % cols) + cols) % cols, row = Math.floor(Math.max(0, ch[2 * g + 1]) / def.row_mm) + 1;
      return col * 4096 + row;
    };
    const idx = head.lods[0].idxCPU, cells = new Set();
    for (let t = 0; t < idx.length; t += 3) {
      const u0 = ch[2 * idx[t]], us = [u0, ch[2 * idx[t + 1]], ch[2 * idx[t + 2]]].map((u) => u - 360 * Math.round((u - u0) / 360));
      const raw = [u0, ch[2 * idx[t + 1]], ch[2 * idx[t + 2]]];
      if (Math.max(...us) - Math.min(...us) >= span || Math.max(...raw) - Math.min(...raw) >= 180) for (let k = 0; k < 3; k++) cells.add(cellKey(idx[t + k]));
    }
    const vertex = new Uint8Array(head.n);
    for (let g = 0; g < head.n; g++) vertex[g] = cells.has(cellKey(g)) ? 1 : 0;
    this.axisCache = { ch, cells, vertex, cellKey, front };
    return this.axisCache;
  }

  zbufFor(r, M) {
    if (!this.zbufs) this.zbufs = new Map();
    const have = this.zbufs.get(r.slot);
    if (have) return have;
    const S = 2, W = Math.max(1, Math.ceil(r.w / S)), H = Math.max(1, Math.ceil(r.h / S)), buf = new Float32Array(W * H).fill(Infinity);
    const m = M.mvp, f = M.f, st = this.vp[r.slot], P = this.planes(), box = this.boxWorld(), o = this.opts;
    const cut = (x, y, z) => P.some((q) => q.n[0] * x + q.n[1] * y + q.n[2] * z > q.d) || (o.box.on && x > box.lo[0] && x < box.hi[0] && y > box.lo[1] && y < box.hi[1] && z > box.lo[2] && z < box.hi[2]);
    for (const mesh of this.visibleMeshes()) {
      const pos = mesh.cpuPos, idx = this.lodOf(mesh, st).idxCPU, n = pos.length / 3;
      const X = new Float32Array(n), Y = new Float32Array(n), Z = new Float32Array(n);
      for (let i = 0; i < n; i++) {
        const x = pos[3 * i], y = pos[3 * i + 1], z = pos[3 * i + 2], w = m[3] * x + m[7] * y + m[11] * z + m[15];
        if (w <= 1e-9) { Z[i] = NaN; continue; }
        X[i] = ((m[0] * x + m[4] * y + m[8] * z + m[12]) / w + 1) * 0.5 * W;
        Y[i] = (1 - (m[1] * x + m[5] * y + m[9] * z + m[13]) / w) * 0.5 * H;
        Z[i] = f[0] * x + f[1] * y + f[2] * z;
      }
      const clipping = P.length > 0 || o.box.on;
      for (let t = 0; t < idx.length; t += 3) {
        const a = idx[t], b = idx[t + 1], c = idx[t + 2];
        if (Number.isNaN(Z[a]) || Number.isNaN(Z[b]) || Number.isNaN(Z[c])) continue;
        if (clipping && cut((pos[3 * a] + pos[3 * b] + pos[3 * c]) / 3, (pos[3 * a + 1] + pos[3 * b + 1] + pos[3 * c + 1]) / 3, (pos[3 * a + 2] + pos[3 * b + 2] + pos[3 * c + 2]) / 3)) continue;
        const x0 = Math.max(0, Math.floor(Math.min(X[a], X[b], X[c]))), x1 = Math.min(W - 1, Math.ceil(Math.max(X[a], X[b], X[c])));
        const y0 = Math.max(0, Math.floor(Math.min(Y[a], Y[b], Y[c]))), y1 = Math.min(H - 1, Math.ceil(Math.max(Y[a], Y[b], Y[c])));
        if (x0 > x1 || y0 > y1) continue;
        const area = (X[b] - X[a]) * (Y[c] - Y[a]) - (X[c] - X[a]) * (Y[b] - Y[a]);
        if (Math.abs(area) < 1e-12) continue;
        const ia = 1 / area;
        for (let py = y0; py <= y1; py++) {
          const sy = py + 0.5;
          for (let px = x0; px <= x1; px++) {
            const sx = px + 0.5;
            const w0 = ((X[b] - sx) * (Y[c] - sy) - (X[c] - sx) * (Y[b] - sy)) * ia, w1 = ((X[c] - sx) * (Y[a] - sy) - (X[a] - sx) * (Y[c] - sy)) * ia, w2 = 1 - w0 - w1;
            if (w0 < -1e-6 || w1 < -1e-6 || w2 < -1e-6) continue;
            const d = w0 * Z[a] + w1 * Z[b] + w2 * Z[c], k = py * W + px;
            if (d < buf[k]) buf[k] = d;
          }
        }
      }
    }
    const cam = this.cams[r.cam], perPx = M.ortho ? (2 * M.halfH) / r.h : (2 * M.tanh * cam.cur.dist) / r.h;
    const zb = { W, H, S, buf, tol: Math.max(0.002, 3 * S * perPx) };
    this.zbufs.set(r.slot, zb);
    return zb;
  }

  zVisible(zb, M, r, p) {
    if (!zb) return true;
    const s = this.project(M, r, p);
    if (!s) return false;
    const px = Math.floor((s.x - r.x) / zb.S), py = Math.floor((s.y - r.y) / zb.S);
    if (px < 0 || py < 0 || px >= zb.W || py >= zb.H) return false;
    return M.f[0] * p[0] + M.f[1] * p[1] + M.f[2] * p[2] <= zb.buf[py * zb.W + px] + zb.tol;
  }

  placeOverlay(rects) {
    if (!this.gridLayer) return false;
    const o = this.opts, head = this.meshes.get("head"), marks = this.markerList(), now = performance.now();
    const viewKey = JSON.stringify([rects.map((r) => [r.x, r.y, r.w, r.h, this.cams[r.cam].cur, this.vp[r.slot].lod]), o.hiddenRegions, this.planes().map((q) => [q.n, q.d]), o.box,
      head?.poseVersion, o.faceGrid, o.eyesGrid, [...this.meshes.values()].map((m) => [m.id, m.visible, m.cpuPos?.length])]);
    if (viewKey !== this.viewKey) { this.viewKey = viewKey; this.viewAt = now; this.zbufs = null; }
    const resting = now - (this.viewAt || 0) >= 120;
    const obstacles = this.overlayObstacles();
    const key = JSON.stringify([viewKey, resting, o.normalGrid, o.gridLabels, marks.map((m) => [m.pos, m.text]), this.hover, this.dark, obstacles]);
    if (key === this.overlayKey) return !resting;
    this.overlayKey = key;
    const parts = [], cellsOut = [], edgesOut = [];
    let ruler = 0, rulerDropped = 0;
    const P = this.planes(), box = this.boxWorld();
    const cut = (p) => P.some((q) => dot(q.n, p) > q.d) || (o.box.on && p.every((x, k) => x > box.lo[k] && x < box.hi[k]));
    const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
    const blocked = (bx) => obstacles.some((q) => bx.x0 < q.x1 && bx.x1 > q.x0 && bx.y0 < q.y1 && bx.y1 > q.y0);
    const wantFace = !!(o.faceGrid && o.gridLabels && this.chartDef && head && head.visible && head.cpuPos && head.nrmCPU && head.chartCPU);
    const ax = wantFace ? this.axisInfo() : null;
    for (const r of rects) {
      const M = this.lastM?.[r.slot];
      if (!M) continue;
      const rulerBoxes = [];
      if (o.normalGrid) for (const l of this.rulerLabels?.[r.slot] || []) {
        const w = l.text.length * 6.2 + 8, bx = l.cls ? { x0: l.x, x1: l.x + w, y0: l.y, y1: l.y + 14 } : { x0: l.x, x1: l.x + w, y0: l.y - 7, y1: l.y + 7 };
        if (blocked(bx)) { rulerDropped++; continue; }
        if (l.cls) rulerBoxes.push(bx);
        parts.push(`<div class="rl${l.cls ? ` ${l.cls}` : ""}" style="left:${l.x.toFixed(1)}px;top:${l.y.toFixed(1)}px">${esc(l.text)}</div>`);
        ruler++;
      }
      const zb = resting && (wantFace || marks.length) ? this.zbufFor(r, M) : null;
      if (wantFace && resting && ax) {
        const def = this.chartDef, letters = def.letters, cols = letters.length, pos = head.cpuPos, nrm = head.nrmCPU, ch = head.chartCPU, reg = head.regionCPU, hid = o.hiddenRegions >>> 0;
        const seen = [];
        let yTop = Infinity, yBot = -Infinity;
        for (let g = 0; g < head.n; g++) {
          if (ax.vertex[g] || (reg && hid && ((hid >>> reg[g]) & 1))) continue;
          const p = [pos[g * 3], pos[g * 3 + 1], pos[g * 3 + 2]];
          if (this.facing(M, p, [nrm[g * 3], nrm[g * 3 + 1], nrm[g * 3 + 2]]) < 0.1 || cut(p)) continue;
          const s = this.project(M, r, p);
          if (!s || !this.zVisible(zb, M, r, p)) continue;
          seen.push(g, s.x, s.y);
          yTop = Math.min(yTop, s.y);
          yBot = Math.max(yBot, s.y);
        }
        const mid = (yTop + yBot) / 2, colUp = new Map(), colDown = new Map(), colAll = new Map(), rowAcc = new Map(), acc = (m, k, v) => { const a = m.get(k) || []; a.push(v); m.set(k, a); };
        for (let i = 0; i < seen.length; i += 3) {
          const g = seen[i], sx = seen[i + 1], sy = seen[i + 2];
          const x = ch[2 * g] / def.column_deg + 0.5, k = Math.floor(x);
          if (Math.abs(x - k - 0.5) < 0.3) { const c = (((k + ax.front) % cols) + cols) % cols; acc(sy < mid ? colUp : colDown, c, sx); acc(colAll, c, sx); }
          const y = Math.max(0, ch[2 * g + 1]) / def.row_mm, row = Math.floor(y);
          if (Math.abs(y - row - 0.5) < 0.3) acc(rowAcc, row + 1, sy);
        }
        const median = (a) => { const s = a.slice().sort((x, y) => x - y); return s[s.length >> 1]; };
        const pick = (m, gap, lo, hi, name) => {
          const items = [...m].filter(([, a]) => a.length >= 3).map(([k, a]) => ({ k, text: name(k), at: median(a), n: a.length })).filter((it) => it.at > lo && it.at < hi).sort((a, b) => b.n - a.n), kept = [];
          for (const it of items) { const w = gap(it.text); if (!kept.some((q) => Math.abs(q.at - it.at) < (q.w + w) / 2 + 2)) kept.push({ ...it, w }); }
          return kept;
        };
        let colsUp = pick(colUp, (t) => t.length * 7 + 8, r.x + 8, r.x + r.w - 8, (k) => letters[k]);
        let colsDown = pick(colDown, (t) => t.length * 7 + 8, r.x + 8, r.x + r.w - 8, (k) => letters[k]);
        const named = new Set([...colsUp, ...colsDown].map((it) => it.k));
        for (const it of pick(new Map([...colAll].filter(([k]) => !named.has(k))), (t) => t.length * 7 + 8, r.x + 8, r.x + r.w - 8, (k) => letters[k])) {
          if (!colsUp.some((q) => Math.abs(q.at - it.at) < (q.w + it.w) / 2 + 2)) colsUp.push(it);
        }
        const all = [...colsUp, ...colsDown], ref = all.length ? all.reduce((a, b) => (b.n > a.n ? b : a)).k : 0;
        const unwrap = (k) => { let d = (((k - ref) % cols) + cols) % cols; if (d >= cols / 2) d -= cols; return d; };
        const monotone = (list, sign) => {
          const sorted = [...list].sort((a, b) => a.at - b.at), dp = [], prev = [];
          sorted.forEach((it, i) => {
            dp[i] = it.n; prev[i] = -1;
            for (let j = 0; j < i; j++) if (sign * unwrap(sorted[j].k) < sign * unwrap(it.k) && dp[j] + it.n > dp[i]) { dp[i] = dp[j] + it.n; prev[i] = j; }
          });
          const keep = [];
          let i = dp.length ? dp.indexOf(Math.max(...dp)) : -1;
          for (; i >= 0; i = prev[i]) keep.unshift(sorted[i]);
          return { keep, total: dp.length ? Math.max(...dp) : 0 };
        };
        const dir = [1, -1].map((sg) => ({ sg, total: monotone(colsUp, sg).total + monotone(colsDown, sg).total })).sort((a, b) => b.total - a.total)[0].sg;
        const best = (list) => { const a = monotone(list, dir), b = monotone(list, -dir); return (b.total > a.total ? b : a).keep; };
        colsUp = best(colsUp);
        colsDown = best(colsDown);
        const rowsKept = pick(rowAcc, () => 13, r.y + 8, r.y + r.h - 8, (k) => `${k}`);
        const onRuler = (bx) => rulerBoxes.some((q) => bx.x0 < q.x1 && bx.x1 > q.x0 && bx.y0 < q.y1 && bx.y1 > q.y0);
        const put = (it, x, y, kind, edge) => {
          const w = it.text.length * 7 + 8, box = (yy) => ({ x0: x - w / 2, x1: x + w / 2, y0: yy - 7, y1: yy + 7 });
          if (edge === "bottom" && onRuler(box(y))) y -= 18;
          const bx = box(y);
          if (blocked(bx) || onRuler(bx)) return;
          parts.push(`<div class="gl edge" style="left:${x.toFixed(1)}px;top:${y.toFixed(1)}px">${it.text}</div>`);
          edgesOut.push({ slot: r.slot, kind, edge, text: it.text, x: +x.toFixed(1), y: +y.toFixed(1), n: it.n });
        };
        const bottom = r.y + r.h - (o.normalGrid ? 32 : 14);
        const bars = obstacles.filter((q) => q.kind === "toolbar" && q.x0 < r.x + r.w && q.x1 > r.x && q.y1 > r.y && q.y0 < r.y + r.h * 0.25);
        const top = Math.max(r.y + 12, ...bars.map((q) => q.y1 + 10));
        for (const it of colsUp) put(it, it.at, top, "col", "top");
        for (const it of colsDown) put(it, it.at, bottom, "col", "bottom");
        this.edgeSplit = this.edgeSplit || [];
        this.edgeSplit[r.slot] = mid;
        const rowsAll = [...rowAcc].filter(([, a]) => a.length >= 3).map(([k, a]) => ({ k, text: `${k}`, at: median(a), n: a.length, w: 13 })).filter((it) => it.at > r.y + 8 && it.at < r.y + r.h - 8).sort((a, b) => a.at - b.at);
        const gaps = rowsAll.slice(1).map((it, i) => it.at - rowsAll[i].at), roomy = !gaps.length || median(gaps) >= 30, leftFree = !o.normalGrid || !M.ortho;
        const side = (list) => { const kept = []; for (const it of list) if (!kept.some((q) => Math.abs(q.at - it.at) < 15)) kept.push(it); return kept; };
        const rightRows = side(roomy || !leftFree ? rowsKept : rowsAll.filter((it) => it.k % 2 === 0)), leftRows = leftFree ? side(roomy ? rowsKept : rowsAll.filter((it) => it.k % 2 === 1)) : [];
        for (const it of rightRows) put(it, r.x + r.w - 14, it.at, "row", "right");
        for (const it of leftRows) put(it, r.x + 16, it.at, "row", "left");
        const cand = [], placed = [];
        for (const a of this.chartAnchors) {
          const g = a.g;
          if (ax.vertex[g] || (reg && hid && ((hid >>> reg[g]) & 1))) continue;
          const p = [pos[g * 3], pos[g * 3 + 1], pos[g * 3 + 2]], f = this.facing(M, p, [nrm[g * 3], nrm[g * 3 + 1], nrm[g * 3 + 2]]);
          if (f < 0.45 || cut(p)) continue;
          const pxPerMm = M.ortho ? r.h / (2 * M.halfH * 1000) : r.h / (2 * M.tanh * Math.max(1e-4, dot(sub(p, M.eye), M.f)) * 1000);
          if (def.row_mm * pxPerMm * f < 56) continue;
          const s = this.project(M, r, p);
          if (s && this.zVisible(zb, M, r, p)) cand.push({ s, f, g, text: `${letters[a.col]}${a.row}` });
        }
        cand.sort((x, y) => y.f - x.f);
        for (const c of cand) {
          const w = c.text.length * 6.5 + 8, bx = { x0: c.s.x - w / 2, x1: c.s.x + w / 2, y0: c.s.y - 7, y1: c.s.y + 7 };
          if (blocked(bx) || placed.some((q) => Math.abs(q.x - c.s.x) < (q.w + w) / 2 + 2 && Math.abs(q.y - c.s.y) < 15)) continue;
          placed.push({ x: c.s.x, y: c.s.y, w });
          parts.push(`<div class="gl" style="left:${c.s.x.toFixed(1)}px;top:${c.s.y.toFixed(1)}px">${c.text}</div>`);
          cellsOut.push({ slot: r.slot, text: c.text, g: c.g, x: +c.s.x.toFixed(1), y: +c.s.y.toFixed(1) });
        }
      }
      if (wantFace && this.hover && this.hover.slot === r.slot) {
        const s = this.project(M, r, this.hover.pos);
        if (s) parts.push(`<div class="gl hover" style="left:${(s.x + 12).toFixed(1)}px;top:${(s.y - 26).toFixed(1)}px">${esc(this.hover.text)}</div>`);
      }
      for (const mk of marks) {
        if (!mk.text) continue;
        const s = this.project(M, r, mk.pos);
        if (!s || (mk.nrm && this.facing(M, mk.pos, mk.nrm) < 0) || cut(mk.pos) || !this.zVisible(zb, M, r, mk.pos)) continue;
        parts.push(`<div class="pin" style="left:${(s.x + 6).toFixed(1)}px;top:${(s.y - 18).toFixed(1)}px">${esc(mk.text)}</div>`);
      }
    }
    this.gridLayer.innerHTML = parts.join("");
    this.overlayStats = { labels: cellsOut.length, edge: edgesOut.length, cells: cellsOut, edges: edgesOut, markers: marks.length, ruler, rulerDropped, resting, obstacles };
    return !resting;
  }

  meshUniforms(p, M, st) {
    const gl = this.gl, o = this.opts;
    gl.uniform1i(p.u("matOn"), 0);
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
    gl.uniform1i(p.u("gridOn"), o.faceGrid && this.chartDef ? 1 : 0);
    gl.uniform2fv(p.u("gridStep"), this.chartDef ? [this.chartDef.column_deg, this.chartDef.row_mm] : [15, 10]);
    gl.uniform3fv(p.u("gridInk"), this.dark ? [0.30, 0.95, 0.85] : [0.02, 0.36, 0.55]);
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
      bind(7, m.chart, D.f2);
      gl.activeTexture(gl.TEXTURE8);
      gl.bindTexture(gl.TEXTURE_2D, D.rgba);
      gl.uniform1i(p.u("hasChart"), m.chart && (m.kind === "head" || this.opts.eyesGrid === "grid") ? 1 : 0);
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
    this.lastM = this.lastM || [];
    this.lastM[r.slot] = M;
    if (o.normalGrid) { gl.disable(gl.DEPTH_TEST); gl.depthMask(false); this.drawRuler(M, r, cam); gl.depthMask(true); }
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
    const lit = LIT_MODES.has(st.mode);
    for (const m of meshes) {
      const lod = this.lodOf(m, st);
      this.bindMesh(mp, m, lod);
      if (lit && m.look) this.drawParts(mp, m, false);
      else gl.drawArrays(gl.TRIANGLES, 0, lod.tri * 3);
    }
    if (lit && meshes.some((m) => m.look)) {
      gl.enable(gl.BLEND);
      gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
      gl.depthMask(false);
      for (const m of meshes) if (m.look) { this.bindMesh(mp, m, this.lodOf(m, st)); this.drawParts(mp, m, true); }
      gl.depthMask(true);
      gl.disable(gl.BLEND);
    }
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
    const marks = this.markerList();
    if (marks.length) {
      const toward = (p) => { const d = M.ortho ? mul(M.f, -1) : norm(sub(M.eye, p)); return add(p, mul(d, 0.0012)); };
      for (const mk of marks) this.drawLines(M, toward(mk.pos), mk.color || [1, 0.2, 0.55], gl.POINTS, -1, (mk.size || 9) * dpr);
    }
    gl.disable(gl.DEPTH_TEST);
    if (this.flash && performance.now() < this.flash.until) {
      const t = this.flash.pos, s = this.bounds.radius * 0.05, k = 0.5 + 0.5 * Math.sin(performance.now() / 90);
      this.drawLines(M, [t[0] - s, t[1], t[2], t[0] + s, t[1], t[2], t[0], t[1] - s, t[2], t[0], t[1] + s, t[2], t[0], t[1], t[2] - s, t[0], t[1], t[2] + s], [1, 0.25 + 0.6 * k, 0.1], gl.LINES);
      this.drawLines(M, t, [1, 0.25 + 0.6 * k, 0.1], gl.POINTS, -1, 11 * dpr);
    }
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
    const key = `${this.layout}|${this.active}|${rects.map((r) => `${r.x},${r.y},${r.w},${r.h}${this.cams[r.cam].ortho}${this.cams[r.cam].name}${this.vp[r.slot].mode}${this.vp[r.slot].lod}`).join(",")}`;
    if (key === this.labelKey) return;
    this.labelKey = key;
    this.labels.innerHTML = "";
    for (const r of rects) {
      const el = document.createElement("div");
      el.className = `vp${r.slot === this.active && rects.length > 1 ? " active" : ""}`;
      el.style.left = `${r.x}px`; el.style.top = `${r.y}px`; el.style.width = `${r.w}px`; el.style.height = `${r.h}px`;
      const cam = this.cams[r.cam], st = this.vp[r.slot], head = this.meshes.get("head");
      const lod = head && head.lods.length > 1 ? ` · LOD${st.lod}` : "";
      el.innerHTML = `<span>${tr(`view.${cam.name}`, null, cam.name)} · ${cam.ortho ? tr("view.ortho") : tr("ui.persp")} · ${tr(`mode.${MODES[st.mode].id}`)}${lod}</span>`;
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
        best = { t: dist, point: hit, mesh: m.id, slot: r.slot, triangle: t / 3, bary: [1 - u - v, u, v], corners: [idx[t], idx[t + 1], idx[t + 2]],
                 lod: m.kind === "head" ? Math.min(st.lod, m.lods.length - 1) : 0 };
      }
    }
    return best;
  }

  screenOf(p, slot = this.active) {
    const r = this.rects().find((x) => x.slot === slot) || this.rects()[0], M = this.lastM?.[r.slot];
    if (!M) return null;
    const b = this.canvas.getBoundingClientRect(), s = this.project(M, r, p);
    return s ? { x: s.x + b.left, y: s.y + b.top, rx: s.x, ry: s.y, rect: r } : null;
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
