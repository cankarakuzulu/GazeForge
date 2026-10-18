const params = new URLSearchParams(location.search);
const store = params.get("store");
const opt = (k) => params.get(k) || undefined;
const $ = (id) => document.getElementById(id);

async function api(name, input) {
  const r = await fetch(`/v1/${name}`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(input) });
  const env = await r.json();
  if (!env.ok) throw new Error(`${name}: ${env.error?.message || r.status}`);
  return env.result;
}

const decodeF32 = (b64) => {
  const s = atob(b64);
  const u = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i);
  return new Float32Array(u.buffer);
};

const D = {
  targets: [], index: new Map(), w: null, w0: null, start: {}, macros: null, macroW: {}, macroW0: {}, macroDelta: {},
  base: {}, kernel: 0, wasm: null, n: 0, tri: 0, settled: null, preview: null, settleCount: 0, settling: false,
  stats: { frames: [], used: [], intervals: [], settles: [], errors: [], flips: [] }, measure: params.get("measure") === "1",
  camera: { yaw: 0, pitch: 0.08, dist: 0.42, centre: [0, 0, 0] }, shade: 0, dirty: false, pendingSettle: 0, lastSent: null,
  hooks: { beforeFrame: null },
};
window.gfDesigner = D;

function setStatus(kind, text) {
  const el = $("status");
  el.className = `status ${kind}`;
  el.textContent = text;
  $("label").textContent = kind === "preview" ? "approximate preview: release the slider to settle" : "";
}

function views() {
  const e = D.wasm, mem = e.memory.buffer, k = D.kernel, n3 = D.n * 3;
  if (D.mem === mem) return D.v;
  D.mem = mem;
  D.v = {
    weights: new Float32Array(mem, e.gf_weights(k), D.targets.length),
    anchor: new Float32Array(mem, e.gf_anchor(k), n3),
    extra: new Float32Array(mem, e.gf_extra(k), n3),
    reference: new Float32Array(mem, e.gf_reference(k), n3),
    positions: new Float32Array(mem, e.gf_positions(k), n3),
    normals: new Float32Array(mem, e.gf_normals(k), n3),
  };
  return D.v;
}

async function loadKernel(pack, blob) {
  const { instance } = await WebAssembly.instantiateStreaming(fetch("gf_wasm.wasm"), {});
  const e = instance.exports;
  D.wasm = e;
  const L = pack.layout;
  const put = (name, Ctor) => {
    const src = new Ctor(blob, L[name].offset, L[name].count);
    const ptr = e.gf_alloc(src.byteLength);
    new Uint8Array(e.memory.buffer, ptr, src.byteLength).set(new Uint8Array(src.buffer, src.byteOffset, src.byteLength));
    return ptr;
  };
  const tris = put("triangles", Uint32Array), offs = put("offsets", Uint32Array), idx = put("index", Uint32Array);
  const del = put("delta", Int16Array), sc = put("scale", Float32Array);
  D.kernel = e.gf_kernel_new(pack.vertices, tris, pack.triangles, pack.targets.length, offs, idx, pack.entries, del, sc);
  if (!D.kernel) throw new Error("the preview kernel refused the pack");
  D.n = pack.vertices;
  D.tri = pack.triangles;
  D.indices = new Uint32Array(blob, L.triangles.offset, L.triangles.count).slice();
}

let gl = null;

async function context() {
  for (let attempt = 0; attempt < 8; attempt++) {
    let canvas = $("gl");
    if (attempt > 0) {
      const fresh = canvas.cloneNode(false);
      canvas.replaceWith(fresh);
      canvas = fresh;
      await new Promise((r) => setTimeout(r, 400));
    }
    const g = canvas.getContext("webgl2", { antialias: true, preserveDrawingBuffer: true });
    if (g && !g.isContextLost()) {
      canvas.addEventListener("webglcontextlost", (e) => e.preventDefault());
      canvas.addEventListener("webglcontextrestored", () => { setupGL(); request(true); });
      D.contextAttempts = attempt + 1;
      return g;
    }
  }
  return null;
}
const VS = `#version 300 es
in vec3 p; in vec3 n; uniform mat4 mvp; out vec3 vn; out vec3 vp;
void main() { vn = n; vp = p; gl_Position = mvp * vec4(p, 1.0); }`;
const FS = `#version 300 es
precision highp float; in vec3 vn; in vec3 vp; uniform int mode; uniform vec3 eye; out vec4 o;
void main() {
  vec3 n = normalize(vn);
  if (mode == 1) { o = vec4(n * 0.5 + 0.5, 1.0); return; }
  vec3 v = normalize(eye - vp);
  vec3 key = normalize(vec3(-0.4, -0.8, 0.6)), fill = normalize(vec3(0.7, -0.5, 0.1));
  float d = max(dot(n, key), 0.0) * 0.75 + max(dot(n, fill), 0.0) * 0.25 + 0.12;
  float rim = pow(1.0 - max(dot(n, v), 0.0), 3.0) * 0.25;
  vec3 c = vec3(0.80, 0.74, 0.68) * d + rim;
  if (dot(n, v) < 0.0) c = mix(c, vec3(0.8, 0.1, 0.1), 0.5);
  o = vec4(c, 1.0);
}`;

function program() {
  const sh = (type, src) => {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(`shader: ${gl.getShaderInfoLog(s)}; context lost: ${gl.isContextLost()}`);
    return s;
  };
  const p = gl.createProgram();
  gl.attachShader(p, sh(gl.VERTEX_SHADER, VS));
  gl.attachShader(p, sh(gl.FRAGMENT_SHADER, FS));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(`program: ${gl.getProgramInfoLog(p)}; context lost: ${gl.isContextLost()}`);
  return p;
}

function setupGL() {
  if (!gl) throw new Error("WebGL2 is not available in this browser (on some Windows boxes Chrome needs --use-angle=gl)");
  D.prog = program();
  D.vao = gl.createVertexArray();
  gl.bindVertexArray(D.vao);
  const buf = (loc, data) => {
    const b = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, b);
    gl.bufferData(gl.ARRAY_BUFFER, data.byteLength, gl.DYNAMIC_DRAW);
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, 3, gl.FLOAT, false, 0, 0);
    return b;
  };
  D.pbuf = buf(gl.getAttribLocation(D.prog, "p"), new Float32Array(D.n * 3));
  D.nbuf = buf(gl.getAttribLocation(D.prog, "n"), new Float32Array(D.n * 3));
  const ib = gl.createBuffer();
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ib);
  gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, D.indices, gl.STATIC_DRAW);
  gl.enable(gl.DEPTH_TEST);
  const dbg = gl.getExtension("WEBGL_debug_renderer_info");
  D.renderer = dbg ? gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
}

function mat(yaw, pitch, dist, centre, aspect) {
  const eye = [centre[0] + dist * Math.sin(yaw) * Math.cos(pitch), centre[1] - dist * Math.cos(yaw) * Math.cos(pitch), centre[2] + dist * Math.sin(pitch)];
  const f = [centre[0] - eye[0], centre[1] - eye[1], centre[2] - eye[2]];
  const norm = (a) => { const l = Math.hypot(...a); return a.map((x) => x / l); };
  const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
  const fz = norm(f), s = norm(cross(fz, [0, 0, 1])), u = cross(s, fz);
  const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  const view = [s[0], u[0], -fz[0], 0, s[1], u[1], -fz[1], 0, s[2], u[2], -fz[2], 0, -dot(s, eye), -dot(u, eye), dot(fz, eye), 1];
  const fov = 0.5, near = 0.01, far = 5, t = 1 / Math.tan(fov / 2);
  const proj = [t / aspect, 0, 0, 0, 0, t, 0, 0, 0, 0, (far + near) / (near - far), -1, 0, 0, (2 * far * near) / (near - far), 0];
  const m = new Float32Array(16);
  for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) {
    let x = 0;
    for (let k = 0; k < 4; k++) x += proj[k * 4 + r] * view[c * 4 + k];
    m[c * 4 + r] = x;
  }
  return { m, eye };
}

function draw() {
  const c = gl.canvas, dpr = window.devicePixelRatio || 1;
  const w = Math.round(c.clientWidth * dpr), h = Math.round(c.clientHeight * dpr);
  if (c.width !== w || c.height !== h) { c.width = w; c.height = h; }
  gl.viewport(0, 0, w, h);
  const bg = matchMedia("(prefers-color-scheme: dark)").matches ? [0.11, 0.11, 0.12] : [0.93, 0.93, 0.92];
  gl.clearColor(...bg, 1);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
  gl.useProgram(D.prog);
  const cam = D.camera, { m, eye } = mat(cam.yaw, cam.pitch, cam.dist, cam.centre, w / h);
  gl.uniformMatrix4fv(gl.getUniformLocation(D.prog, "mvp"), false, m);
  gl.uniform3fv(gl.getUniformLocation(D.prog, "eye"), eye);
  gl.uniform1i(gl.getUniformLocation(D.prog, "mode"), D.shade);
  gl.bindVertexArray(D.vao);
  gl.drawElements(gl.TRIANGLES, D.tri * 3, gl.UNSIGNED_INT, 0);
}

function computePreview() {
  const v = views();
  for (let i = 0; i < D.targets.length; i++) v.weights[i] = D.w[i] - D.w0[i];
  let extra = 0;
  v.extra.fill(0);
  for (const [name, d] of Object.entries(D.macroDelta)) {
    const dm = (D.macroW[name] ?? 0) - (D.macroW0[name] ?? 0);
    if (dm === 0) continue;
    const s = dm > 0 ? d.up : d.down;
    for (let i = 0; i < s.length; i++) v.extra[i] += dm * s[i];
    extra = 1;
  }
  return D.wasm.gf_preview(D.kernel, extra);
}

function frame(ts) {
  D.raf = 0;
  const t0 = performance.now();
  if (D.hooks.beforeFrame) D.hooks.beforeFrame();
  if (D.dirty) {
    D.lastUsed = computePreview();
    const v = views();
    gl.bindBuffer(gl.ARRAY_BUFFER, D.pbuf);
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, v.positions);
    gl.bindBuffer(gl.ARRAY_BUFFER, D.nbuf);
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, v.normals);
    D.dirty = false;
  }
  draw();
  if (D.measure) gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(4));
  const t1 = performance.now();
  if (D.dragging) {
    D.stats.frames.push(t1 - t0);
    D.stats.used.push(D.lastUsed);
    if (D.lastTs) D.stats.intervals.push(ts - D.lastTs);
    performance.measure("gf-frame", { start: t0, end: t1 });
  }
  D.lastTs = ts;
  D.frameCount = (D.frameCount || 0) + 1;
}

function request(dirty = true) {
  if (dirty) D.dirty = true;
  if (!D.raf) D.raf = requestAnimationFrame(frame);
}

function changed() {
  for (let i = 0; i < D.targets.length; i++) if (D.w[i] !== D.w0[i]) return true;
  return Object.keys(D.macroW).some((k) => D.macroW[k] !== D.macroW0[k]);
}

function weightsMap() {
  const out = {};
  D.targets.forEach((t, i) => { if (D.w[i] > 0) out[t.id] = D.w[i]; });
  return out;
}

function settleInput() {
  const input = { store, pack: D.base.pack, catalog: D.base.catalog, game: D.base.game, placement: D.start.placement, name: D.start.name, weights: weightsMap(), residual: D.start.residual };
  if (D.macros?.available && (D.start.macros || Object.keys(D.macroW).some((k) => D.macroW[k] !== D.macroBase[k]))) {
    input.macros = { sliders: { ...D.macroW }, mode: D.start.macros?.mode || "gf", head_scale: D.start.macros?.head_scale ?? 1 };
    input.macro = opt("macro");
  }
  if (opt("eyes")) input.eyes = opt("eyes");
  return input;
}

async function settle(full) {
  if (D.settling) { D.pendingSettle = 1; return; }
  clearTimeout(D.fullTimer);
  D.settling = true;
  setStatus("busy", "settling");
  const wSent = D.w.slice(), mSent = { ...D.macroW };
  const t0 = performance.now();
  let r;
  try {
    const input = settleInput();
    if (params.get("fast") === "1" && !full) input.fast = true;
    r = await api("designer.settle", input);
  } catch (e) {
    D.settling = false;
    setStatus("preview", `settle failed: ${e.message}`);
    return;
  }
  const pos = decodeF32(r.positions_f32_b64);
  const rtt = performance.now() - t0;
  const v = views();
  const same = wSent.every((x, i) => x === D.w[i]) && Object.keys(mSent).every((k) => mSent[k] === D.macroW[k]);
  if (D.settleCount > 0 && same) {
    computePreview();
    const p = views().positions;
    const err = new Float32Array(D.n);
    for (let g = 0; g < D.n; g++) err[g] = Math.hypot(p[g * 3] - pos[g * 3], p[g * 3 + 1] - pos[g * 3 + 1], p[g * 3 + 2] - pos[g * 3 + 2]) * 1000;
    views().reference.set(pos);
    const flipsVsSettled = D.wasm.gf_flips(D.kernel, 1), flipsVsAnchor = D.wasm.gf_flips(D.kernel, 0);
    D.stats.errors.push(Array.from(err));
    D.stats.flips.push({ vs_settled: flipsVsSettled, vs_anchor: flipsVsAnchor });
  }
  D.stats.settles.push({ rtt_ms: rtt, server: r.timing_ms, same, draft: !!r.approximate });
  views().anchor.set(pos);
  D.w0 = wSent;
  D.macroW0 = mSent;
  D.macroDelta = {};
  D.settled = r;
  D.settleCount++;
  D.settling = false;
  showMeasure(r.measurements);
  const kind = r.approximate ? `${r.placement} draft (approximate, ${r.draft.rounds} of ${r.draft.full_rounds} fit rounds; save and export use the full placement)` : r.placement;
  setStatus(changed() ? "preview" : "settled", changed() ? "preview (approximate)" : `settled ${kind} ${rtt.toFixed(0)} ms`);
  $("info").textContent = `${D.n} vertices, ${D.tri} triangles, ${D.targets.length} targets loaded, ${D.renderer || ""}`;
  request(true);
  if (D.pendingSettle) { D.pendingSettle = 0; settle(); }
  else if (r.approximate && !changed()) D.fullTimer = setTimeout(() => { if (!changed() && !D.settling) settle(true); }, 3000);
}

function scheduleSettle() {
  clearTimeout(D.timer);
  clearTimeout(D.fullTimer);
  D.timer = setTimeout(settle, 150);
}

function showMeasure(m) {
  const rows = Object.entries(m || {}).map(([k, x]) => `<tr><td>${k}</td><td>${x.mm.toFixed(2)}</td><td>${x.delta_mm >= 0 ? "+" : ""}${x.delta_mm.toFixed(2)}</td></tr>`);
  $("measure").innerHTML = rows.join("");
}

function sliderRow(label, value, onInput, onChange, onDown) {
  const row = document.createElement("div");
  row.className = "slider";
  row.innerHTML = `<span class="name" title="${label}">${label}</span><input type="range" min="0" max="1" step="0.001"><span class="value"></span>`;
  const input = row.querySelector("input"), out = row.querySelector(".value");
  input.value = value;
  out.textContent = Number(value).toFixed(3);
  input.addEventListener("input", () => { out.textContent = Number(input.value).toFixed(3); onInput(Number(input.value)); });
  input.addEventListener("change", onChange);
  if (onDown) input.addEventListener("pointerdown", onDown);
  return { row, input, out };
}

function buildSliders() {
  const list = $("list"), cats = $("category");
  let current = "";
  D.rows = [];
  D.targets.forEach((t, i) => {
    if (t.category !== current) {
      current = t.category;
      const h = document.createElement("div");
      h.className = "group";
      h.dataset.category = current;
      h.textContent = `${current} (${D.categories[current]})`;
      list.appendChild(h);
      const o = document.createElement("option");
      o.value = o.textContent = current;
      cats.appendChild(o);
    }
    const s = sliderRow(t.id.split("/").pop(), D.w[i], (x) => {
      D.w[i] = x;
      s.row.classList.toggle("changed", x !== (D.start.weights[t.id] || 0));
      setStatus("preview", "preview (approximate)");
      request(true);
    }, scheduleSettle);
    s.row.dataset.id = t.id;
    s.row.dataset.category = t.category;
    s.input.addEventListener("dblclick", () => { s.input.value = D.start.weights[t.id] || 0; s.input.dispatchEvent(new Event("input")); scheduleSettle(); });
    list.appendChild(s.row);
    D.rows.push(s);
  });
  const filter = () => {
    const q = $("search").value.toLowerCase(), c = cats.value;
    D.rows.forEach((s) => s.row.classList.toggle("hidden", !(s.row.dataset.id.toLowerCase().includes(q) && (!c || s.row.dataset.category === c))));
    list.querySelectorAll(".group").forEach((h) => h.classList.toggle("hidden", !!c && h.dataset.category !== c));
  };
  $("search").addEventListener("input", filter);
  cats.addEventListener("change", filter);
  $("reset").addEventListener("click", () => {
    D.targets.forEach((t, i) => { D.w[i] = D.start.weights[t.id] || 0; D.rows[i].input.value = D.w[i]; D.rows[i].out.textContent = D.w[i].toFixed(3); D.rows[i].row.classList.remove("changed"); });
    Object.assign(D.macroW, D.macroStart);
    (D.macroRows || []).forEach((m) => { m.input.value = D.macroW[m.name]; m.out.textContent = Number(D.macroW[m.name]).toFixed(3); });
    setStatus("preview", "preview (approximate)");
    request(true);
    scheduleSettle();
  });
}

function buildMacros() {
  const box = $("macro-list");
  D.macroBase = {};
  D.macroStart = {};
  for (const s of D.macros.sliders) { D.macroBase[s.name] = s.base; D.macroStart[s.name] = s.value; D.macroW[s.name] = s.value; }
  D.macroW0 = { ...D.macroW };
  if (!D.macros.available) {
    box.innerHTML = `<div class="note">unavailable: ${D.macros.reason}</div>`;
    return;
  }
  D.macroRows = D.macros.sliders.map((m) => {
    const s = sliderRow(m.name, m.value, (x) => { D.macroW[m.name] = x; setStatus("preview", "preview (approximate)"); request(true); }, scheduleSettle, async () => {
      if (D.macroDelta[m.name]) return;
      const input = { ...settleInput(), macro: opt("macro"), slider: m.name, macros: { sliders: { ...D.macroW0 }, mode: D.start.macros?.mode || "gf", head_scale: D.start.macros?.head_scale ?? 1 } };
      delete input.eyes;
      delete input.uvs;
      try {
        const r = await api("designer.macro_delta", input);
        D.macroDelta[m.name] = { up: decodeF32(r.up_f32_b64), down: decodeF32(r.down_f32_b64) };
        request(true);
      } catch (e) {
        $("label").textContent = `macro preview unavailable: ${e.message}`;
      }
    });
    s.name = m.name;
    box.appendChild(s.row);
    return s;
  });
}

function orbit() {
  const c = gl.canvas;
  let drag = null;
  c.addEventListener("pointerdown", (e) => { drag = { x: e.clientX, y: e.clientY }; c.setPointerCapture(e.pointerId); });
  c.addEventListener("pointermove", (e) => {
    if (!drag) return;
    D.camera.yaw -= (e.clientX - drag.x) * 0.008;
    D.camera.pitch = Math.max(-1.4, Math.min(1.4, D.camera.pitch + (e.clientY - drag.y) * 0.008));
    drag = { x: e.clientX, y: e.clientY };
    request(false);
  });
  c.addEventListener("pointerup", () => { drag = null; });
  c.addEventListener("wheel", (e) => { D.camera.dist = Math.max(0.1, Math.min(2, D.camera.dist * Math.exp(e.deltaY * 0.001))); request(false); e.preventDefault(); }, { passive: false });
  document.querySelectorAll("[data-cam]").forEach((b) => b.addEventListener("click", () => { D.camera.yaw = (Number(b.dataset.cam) * Math.PI) / 180; D.camera.pitch = 0.08; request(false); }));
  $("shade").addEventListener("change", (e) => { D.shade = Number(e.target.value); request(false); });
  window.addEventListener("resize", () => request(false));
}

async function save() {
  const out = $("out").value.trim();
  if (!out) { $("saved").textContent = "give a path"; return; }
  try {
    const input = { ...settleInput(), out };
    delete input.eyes;
    const r = await api("designer.save", input);
    $("saved").textContent = `saved ${r.out} (${r.revision})`;
  } catch (e) {
    $("saved").textContent = e.message;
  }
}

async function main() {
  if (!store) {
    setStatus("preview", "open this page with ?store=<store path> (optional &spec=, &macro=, &eyes=)");
    return;
  }
  const sl = await api("designer.sliders", { store, spec: opt("spec"), macro: opt("macro") });
  D.base = { pack: sl.pack, catalog: sl.catalog, game: sl.game };
  D.start = sl.start;
  D.categories = sl.categories;
  D.macros = sl.macros;
  const order = [...sl.targets].sort((a, b) => a.category.localeCompare(b.category) || a.id.localeCompare(b.id));
  const pack = await api("designer.pack", { store, ...D.base, targets: order.map((t) => t.id) });
  D.targets = pack.targets.map((id) => order.find((t) => t.id === id));
  D.w = D.targets.map((t) => D.start.weights[t.id] || 0);
  D.w0 = D.w.slice();
  const hex = pack.blob.replace("sha256:", "");
  const blob = await (await fetch(`/v1/blob/${hex}?store=${encodeURIComponent(store)}`)).arrayBuffer();
  await loadKernel(pack, blob);
  gl = await context();
  setupGL();
  buildMacros();
  buildSliders();
  orbit();
  $("out").value = opt("out") || "";
  $("save").addEventListener("click", save);
  D.pack = pack;
  await settle();
  const a = views().anchor;
  const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
  for (let g = 0; g < D.n; g++) for (let k = 0; k < 3; k++) { lo[k] = Math.min(lo[k], a[g * 3 + k]); hi[k] = Math.max(hi[k], a[g * 3 + k]); }
  D.camera.centre = lo.map((x, k) => (x + hi[k]) / 2);
  D.camera.dist = Math.max(hi[0] - lo[0], hi[2] - lo[2]) * 1.9;
  request(true);
  D.ready = true;
}

main().catch((e) => { setStatus("preview", `error: ${e.message}`); D.error = e.message; });
