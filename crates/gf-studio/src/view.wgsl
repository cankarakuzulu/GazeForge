struct U {
  mvp: mat4x4<f32>,
  eye: vec4<f32>,
  cam_s: vec4<f32>,
  cam_u: vec4<f32>,
  cam_f: vec4<f32>,
  light_dir: array<vec4<f32>, 4>,
  light_col: array<vec4<f32>, 4>,
  sky: vec4<f32>,
  ground: vec4<f32>,
  base_col: vec4<f32>,
  wire_col: vec4<f32>,
  bg_col: vec4<f32>,
  interior_col: vec4<f32>,
  o0: vec4<f32>,
  o1: vec4<f32>,
  o2: vec4<f32>,
  o3: vec4<f32>,
  o4: vec4<f32>,
  o5: vec4<f32>,
  o6: vec4<f32>,
  planes: array<vec4<f32>, 6>,
  box_min: vec4<f32>,
  box_max: vec4<f32>,
  col: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(1) @binding(0) var<storage, read> idx: array<u32>;
@group(1) @binding(1) var<storage, read> pos: array<f32>;
@group(1) @binding(2) var<storage, read> nrm: array<f32>;
@group(1) @binding(3) var<storage, read> refp: array<f32>;
@group(1) @binding(4) var<storage, read> uvs: array<f32>;
@group(1) @binding(5) var<storage, read> eflag: array<u32>;
@group(1) @binding(6) var<storage, read> verts: array<u32>;

fn ival(x: f32) -> i32 { return i32(floor(x + 0.5)); }
fn p3(i: u32) -> vec3<f32> { return vec3<f32>(pos[i * 3u], pos[i * 3u + 1u], pos[i * 3u + 2u]); }
fn n3(i: u32) -> vec3<f32> { return vec3<f32>(nrm[i * 3u], nrm[i * 3u + 1u], nrm[i * 3u + 2u]); }
fn r3(i: u32) -> vec3<f32> { return vec3<f32>(refp[i * 3u], refp[i * 3u + 1u], refp[i * 3u + 2u]); }

fn hemi(n: vec3<f32>) -> vec3<f32> { return mix(u.ground.xyz, u.sky.xyz, n.z * 0.5 + 0.5); }

fn tangent_of(n: vec3<f32>) -> vec3<f32> {
  let h = cross(vec3<f32>(0.0, 0.0, 1.0), n);
  var t = h;
  if (u.o0.w >= 0.5) { t = cross(n, h); }
  let l = length(t);
  if (l > 1e-4) { return t / l; }
  return vec3<f32>(1.0, 0.0, 0.0);
}

fn shade(model: i32, n: vec3<f32>, v: vec3<f32>, albedo: vec3<f32>) -> vec3<f32> {
  var c = albedo * hemi(n);
  let count = ival(u.o0.z);
  for (var i = 0; i < 4; i++) {
    if (i >= count) { break; }
    let l = u.light_dir[i].xyz;
    let lc = u.light_col[i].xyz;
    let nl = dot(n, l);
    if (model == 0) {
      c += albedo * lc * max((nl + 0.3) / 1.3, 0.0);
    } else if (model == 1) {
      c += albedo * lc * max(nl, 0.0);
    } else if (model == 2) {
      let h = normalize(l + v);
      let sp = pow(max(dot(n, h), 0.0), 48.0) * step(0.0, nl);
      c += lc * (albedo * max(nl, 0.0) + 0.35 * sp);
    } else {
      let t = tangent_of(n);
      let h = normalize(l + v);
      let th = dot(t, h);
      let sp = pow(sqrt(max(1.0 - th * th, 0.0)), 80.0) * smoothstep(-0.1, 0.25, nl);
      c += lc * (albedo * max(nl, 0.0) * 0.8 + 0.45 * sp);
    }
  }
  return c;
}

fn clipped(p: vec3<f32>, skip: i32) -> bool {
  let count = ival(u.o4.x);
  for (var i = 0; i < 6; i++) {
    if (i >= count) { break; }
    if (i != skip && dot(u.planes[i].xyz, p) > u.planes[i].w) { return true; }
  }
  return u.o4.z > 0.5 && all(p > u.box_min.xyz) && all(p < u.box_max.xyz);
}

fn inv2(m: mat2x2<f32>) -> mat2x2<f32> {
  let det = m[0][0] * m[1][1] - m[1][0] * m[0][1];
  return mat2x2<f32>(m[1][1] / det, -m[0][1] / det, -m[1][0] / det, m[0][0] / det);
}

fn glsl_mod(x: f32, y: f32) -> f32 { return x - y * floor(x / y); }

fn ramp(t0: f32) -> vec3<f32> {
  let t = clamp(t0, 0.0, 1.0) * 4.0;
  let a = vec3<f32>(0.16, 0.26, 0.85);
  let b = vec3<f32>(0.10, 0.72, 0.82);
  let c = vec3<f32>(0.32, 0.80, 0.30);
  let d = vec3<f32>(0.96, 0.84, 0.20);
  let e = vec3<f32>(0.86, 0.14, 0.10);
  if (t < 1.0) { return mix(a, b, t); }
  if (t < 2.0) { return mix(b, c, t - 1.0); }
  if (t < 3.0) { return mix(c, d, t - 2.0); }
  return mix(d, e, t - 3.0);
}

fn diverge(t0: f32) -> vec3<f32> {
  let t = clamp(t0, -1.0, 1.0);
  if (t < 0.0) { return mix(vec3<f32>(0.93), vec3<f32>(0.12, 0.32, 0.86), -t); }
  return mix(vec3<f32>(0.93), vec3<f32>(0.86, 0.16, 0.10), t);
}

fn checker(t: vec2<f32>) -> vec3<f32> {
  let g = floor(t * 16.0);
  let k = glsl_mod(g.x + g.y, 2.0);
  let c = mix(vec3<f32>(0.22), vec3<f32>(0.92), k);
  return c * vec3<f32>(0.7 + 0.3 * fract(t.x), 0.7 + 0.3 * fract(t.y), 0.85);
}

fn matcap(n: vec3<f32>) -> vec3<f32> {
  let q = vec3<f32>(dot(n, u.cam_s.xyz), dot(n, u.cam_u.xyz), -dot(n, u.cam_f.xyz));
  let key = clamp(dot(q, normalize(vec3<f32>(-0.45, 0.55, 0.7))), 0.0, 1.0);
  let rim = pow(1.0 - clamp(q.z, 0.0, 1.0), 2.5);
  let c = mix(vec3<f32>(0.10, 0.09, 0.09), vec3<f32>(0.80, 0.72, 0.66), key);
  return c + vec3<f32>(1.0, 0.97, 0.92) * pow(key, 24.0) * 0.45 + vec3<f32>(0.35, 0.42, 0.55) * rim * 0.6;
}

fn view_dir(p: vec3<f32>) -> vec3<f32> {
  if (u.o0.y > 0.5) { return -u.cam_f.xyz; }
  return normalize(u.eye.xyz - p);
}

struct MeshOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) vp: vec3<f32>,
  @location(1) vn: vec3<f32>,
  @location(2) bary: vec3<f32>,
  @location(3) gcol: vec3<f32>,
  @location(4) disp: f32,
  @location(5) uv: vec2<f32>,
  @location(6) @interpolate(flat) fnrm: vec3<f32>,
  @location(7) @interpolate(flat) tq: f32,
  @location(8) @interpolate(flat) tc: f32,
  @location(9) @interpolate(flat) eflags: u32,
};

@vertex
fn vs_mesh(@builtin(vertex_index) c: u32) -> MeshOut {
  var o: MeshOut;
  let t = c / 3u;
  let k = c - t * 3u;
  let i0 = idx[t * 3u];
  let i1 = idx[t * 3u + 1u];
  let i2 = idx[t * 3u + 2u];
  let q0 = p3(i0);
  let q1 = p3(i1);
  let q2 = p3(i2);
  var me = i0;
  var p = q0;
  if (k == 1u) { me = i1; p = q1; } else if (k == 2u) { me = i2; p = q2; }
  let vn = n3(me);
  o.vn = vn;
  o.vp = p;
  let e1 = q1 - q0;
  let e2 = q2 - q0;
  let cr = cross(e1, e2);
  let a2 = length(cr);
  o.fnrm = vn;
  if (a2 > 0.0) { o.fnrm = cr / a2 * u.o3.x; }
  o.bary = vec3<f32>(select(0.0, 1.0, k == 0u), select(0.0, 1.0, k == 1u), select(0.0, 1.0, k == 2u));
  o.tq = 0.0;
  o.tc = 0.0;
  o.disp = 0.0;
  o.uv = vec2<f32>(0.0, 0.0);
  o.eflags = 0u;
  o.gcol = vec3<f32>(0.0);
  let mode = ival(u.o0.x);
  if (mode == 8) {
    if (a2 > 0.0) {
      let a = normalize(e1);
      let b = normalize(e2);
      let d = normalize(q2 - q1);
      let A0 = acos(clamp(dot(a, b), -1.0, 1.0));
      let A1 = acos(clamp(dot(-a, d), -1.0, 1.0));
      o.tq = degrees(min(A0, min(A1, 3.14159265 - A0 - A1)));
    }
  } else if (mode == 10) {
    if (a2 > 1e-14) {
      let n0 = n3(i0);
      let n1 = n3(i1);
      let n2 = n3(i2);
      let X = normalize(e1);
      let N = cr / a2;
      let Y = cross(N, X);
      let d1 = n1 - n0;
      let d2 = n2 - n0;
      let E = mat2x2<f32>(dot(e1, X), dot(e1, Y), dot(e2, X), dot(e2, Y));
      let Dn = mat2x2<f32>(dot(d1, X), dot(d1, Y), dot(d2, X), dot(d2, Y));
      let S = Dn * inv2(E);
      o.tc = 0.5 * (S[0][0] + S[1][1]);
    }
  } else if (mode == 9 && u.o3.y > 0.5) {
    let dd = p - r3(me);
    o.disp = length(dd) * 1000.0 * select(1.0, -1.0, dot(dd, vn) < 0.0);
  } else if (mode == 11 && u.o3.z > 0.5) {
    o.uv = vec2<f32>(uvs[c * 2u], uvs[c * 2u + 1u]);
  } else if (mode == 2) {
    o.gcol = shade(2, normalize(vn), view_dir(p), u.base_col.xyz);
  }
  if (u.o3.w > 0.5) { o.eflags = eflag[t]; }
  o.clip = u.mvp * vec4<f32>(p, 1.0);
  return o;
}

@fragment
fn fs_mesh(i: MeshOut, @builtin(front_facing) ff: bool) -> @location(0) vec4<f32> {
  let fw = fwidth(i.bary);
  if (clipped(i.vp, -1)) { discard; }
  let d = i.bary / max(fw, vec3<f32>(1e-6));
  let edge = min(d.x, min(d.y, d.z));
  let ww = u.o1.y;
  var wire = 1.0 - smoothstep(ww - 0.5, ww + 0.5, edge);
  if (u.o1.z > 0.5) { wire = wire * (1.0 - 0.8 * smoothstep(0.15, 0.5, max(fw.x, max(fw.y, fw.z)))); }
  if (u.o1.w > 0.5) {
    if (wire <= 0.02) { discard; }
    return vec4<f32>(u.wire_col.xyz, 0.35 * wire);
  }
  var front = ff;
  if (u.o3.x < 0.0) { front = !ff; }
  var n = normalize(i.vn);
  if (!front) { n = -n; }
  let v = view_dir(i.vp);
  let hl = 0.55 + 0.45 * abs(dot(n, v));
  let mode = ival(u.o0.x);
  let planes = ival(u.o4.x);
  let heat = u.o2.w;
  let base = u.base_col.xyz;
  var c = vec3<f32>(0.0);
  var lit = true;
  if (!front && planes > 0) {
    c = u.interior_col.xyz * (0.45 + 0.55 * abs(dot(n, v)));
    lit = false;
  } else if (mode == 0) {
    c = shade(0, n, v, base);
  } else if (mode == 1) {
    var f = normalize(i.fnrm);
    if (!front) { f = -f; }
    c = shade(1, f, v, base);
  } else if (mode == 2) {
    c = i.gcol;
  } else if (mode == 3) {
    c = shade(2, n, v, base);
  } else if (mode == 4) {
    c = shade(3, n, v, base);
  } else if (mode == 5) {
    c = matcap(n);
    lit = false;
  } else if (mode == 6) {
    c = normalize(i.vn) * 0.5 + 0.5;
    lit = false;
  } else if (mode == 7) {
    c = u.bg_col.xyz;
    lit = false;
  } else if (mode == 8) {
    c = ramp(1.0 - clamp(i.tq / heat, 0.0, 1.0)) * hl;
    lit = false;
  } else if (mode == 9) {
    c = select(vec3<f32>(0.55), diverge(i.disp / heat), u.o3.y > 0.5) * hl;
    lit = false;
  } else if (mode == 10) {
    c = diverge(i.tc / heat) * hl;
    lit = false;
  } else {
    c = select(vec3<f32>(0.55), checker(i.uv), u.o3.z > 0.5) * hl;
    lit = false;
  }
  if (lit) { c = pow(max(c * exp2(u.o2.z), vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2)); }
  if (u.o2.x > 0.5 && !front && planes == 0) { c = mix(c, vec3<f32>(0.92, 0.10, 0.16), 0.65); }
  if (u.o2.y > 0.5 && u.o3.w > 0.5) {
    var b = 1e9;
    var s = 1e9;
    for (var k = 0u; k < 3u; k++) {
      if ((i.eflags & (1u << k)) != 0u) { b = min(b, d[k]); }
      if ((i.eflags & (8u << k)) != 0u) { s = min(s, d[k]); }
    }
    let w2 = ww + 1.0;
    c = mix(c, vec3<f32>(0.10, 0.80, 1.00), 1.0 - smoothstep(w2 - 0.5, w2 + 0.5, s));
    c = mix(c, vec3<f32>(1.00, 0.55, 0.05), 1.0 - smoothstep(w2 - 0.5, w2 + 0.5, b));
  }
  if (u.o1.x > 0.5 || mode == 7) { c = mix(c, u.wire_col.xyz, wire); }
  return vec4<f32>(c, 1.0);
}

struct PointOut {
  @builtin(position) clip: vec4<f32>,
  @location(0) vp: vec3<f32>,
};

@vertex
fn vs_point(@builtin(vertex_index) vi: u32) -> PointOut {
  var o: PointOut;
  let pi = vi / 6u;
  let corner = vi - pi * 6u;
  let p = p3(verts[pi]);
  o.vp = p;
  var off = vec2<f32>(-1.0, -1.0);
  if (corner == 1u) { off = vec2<f32>(1.0, -1.0); }
  if (corner == 2u || corner == 4u) { off = vec2<f32>(1.0, 1.0); }
  if (corner == 5u) { off = vec2<f32>(-1.0, 1.0); }
  var cl = u.mvp * vec4<f32>(p, 1.0);
  let half = u.o4.w * 0.5;
  cl.x += off.x * half * 2.0 / u.o5.x * cl.w;
  cl.y += off.y * half * 2.0 / u.o5.y * cl.w;
  o.clip = cl;
  return o;
}

@fragment
fn fs_point(i: PointOut) -> @location(0) vec4<f32> {
  if (clipped(i.vp, -1)) { discard; }
  return vec4<f32>(u.col.xyz, 1.0);
}

@vertex
fn vs_line(@location(0) a: vec3<f32>) -> PointOut {
  var o: PointOut;
  o.vp = a;
  o.clip = u.mvp * vec4<f32>(a, 1.0);
  return o;
}

@fragment
fn fs_line(i: PointOut) -> @location(0) vec4<f32> {
  if (u.o5.z > 0.5) {
    if (clipped(i.vp, ival(u.o4.y))) { discard; }
    let gx = i.clip.x + u.o6.x;
    let gy = u.o5.w - (i.clip.y + u.o6.y);
    let h = step(0.5, fract((gx + gy) / 10.0));
    return vec4<f32>(mix(u.col.xyz, u.col.xyz * 0.78, h), 1.0);
  }
  return vec4<f32>(u.col.xyz, 1.0);
}
