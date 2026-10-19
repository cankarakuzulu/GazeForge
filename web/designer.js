import { Viewer, MODES, MODE_INDEX, LIGHT_PRESETS, MAX_LIGHTS, LAYOUTS, vertexNormals } from "./viewer.js";

const params = new URLSearchParams(location.search);
let store = params.get("store");
const opt = (k) => params.get(k) || undefined;
const $ = (id) => document.getElementById(id);

async function api(name, input) {
  const r = await fetch(`/v1/${name}`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(input) });
  const env = await r.json();
  if (!env.ok) {
    const e = new Error(`${name}: ${env.error?.message || r.status}`);
    Object.assign(e, { code: env.error?.code, details: env.error?.details, hint: env.error?.hint });
    throw e;
  }
  return env.result;
}

const bytesOf = (b64) => {
  const s = atob(b64);
  const u = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i);
  return u.buffer;
};
const decodeF32 = (b64) => new Float32Array(bytesOf(b64));
const decodeU32 = (b64) => new Uint32Array(bytesOf(b64));

const D = {
  targets: [], index: new Map(), w: null, w0: null, start: {}, macros: null, macroW: {}, macroW0: {}, macroDelta: {},
  base: {}, kernel: 0, wasm: null, n: 0, tri: 0, settled: null, preview: null, settleCount: 0, settling: false,
  stats: { frames: [], used: [], intervals: [], settles: [], errors: [], flips: [] }, measure: params.get("measure") === "1",
  dirty: false, pendingSettle: 0, lastSent: null, hooks: { beforeFrame: null },
  history: { done: [], undone: [], limit: 300 }, poses: new Map(), listeners: [], panelDefs: [], panels: [], optWriters: [],
  link: true, refSource: "start", lastTarget: -1, lastPick: null, tab: "shape", preset: "studio",
  panelHooks: new Map(), pendingVis: new Map(),
  project: { name: null, doc: null, active: 0, savedSig: null, dirty: false, autosaved: false, saving: false, leaving: false, autosaves: 0, recovered: false },
};
window.gfDesigner = D;
let V = null;
let gl = null;

const SHORTCUTS = [
  ["Ctrl+Z", "undo"], ["Ctrl+Y / Ctrl+Shift+Z", "redo"], ["F", "frame the head (active view)"], ["Shift+F", "frame the selection (last slider's region, else the last picked point)"],
  ["double-click", "set the orbit pivot on the surface point"], ["left drag", "orbit"], ["right / middle / Shift+left drag", "pan"], ["wheel", "zoom (Alt+wheel moves clip plane 1)"],
  ["O or 5", "perspective / orthographic (matched framing)"], ["1 / Shift+1", "front / back view"], ["3 / Shift+3", "left / right side view"], ["7 / Shift+7", "top / bottom view"], ["9", "three-quarter view"],
  ["[ / ]", "narrower / wider field of view (framing kept)"], ["L", "layout: single, side by side, quad"], ["N", "next viewport"], ["M / Shift+M", "next / previous render mode"], [", / .", "coarser / finer level of detail"],
  ["W", "wireframe overlay"], ["X", "hidden edges through the surface (x-ray wire)"], ["P", "vertex points"], ["B", "back-face highlight"], ["E", "seams and boundary edges"],
  ["C", "clip plane 1 on / off"], ["G", "light gizmo"], ["H / Alt+H", "hide the head / show everything"], ["I", "isolate the head"], ["K / Shift+K", "PNG of the active view / the whole layout"],
  ["Ctrl+N / Ctrl+O", "new / open project"], ["Ctrl+S / Ctrl+Shift+S", "save / save the project as"],
  ["?", "this help"], ["Esc", "close the help"],
];
D.shortcuts = SHORTCUTS;

function setStatus(kind, text) {
  const el = $("status");
  el.className = `status ${kind}`;
  el.textContent = text;
  $("label").textContent = kind === "preview" ? "approximate preview: release the slider to settle" : "";
}

function toast(text) {
  const el = $("toast");
  el.textContent = text;
  el.classList.add("on");
  clearTimeout(D.toastTimer);
  D.toastTimer = setTimeout(() => el.classList.remove("on"), 2200);
}

const SKEY = "gazeforge.designer.v1";
function loadSettings() { try { return JSON.parse(localStorage.getItem(SKEY) || "{}") || {}; } catch { return {}; } }
function saveSettings() {
  clearTimeout(D.saveTimer);
  D.saveTimer = setTimeout(() => {
    try {
      const { hiddenRegions, box, ...opts } = V.opts;
      localStorage.setItem(SKEY, JSON.stringify({ opts, layout: V.layout, vp: V.vp, link: D.link, refSource: D.refSource, tab: D.tab, preset: D.preset,
        cams: V.cams.map((c) => ({ fov: c.goal.fov, ortho: c.ortho })) }));
    } catch {}
  }, 300);
}
function applySettings(s) {
  if (!s || typeof s !== "object") return;
  const o = s.opts || {};
  for (const k of Object.keys(V.opts)) {
    if (k === "hiddenRegions" || k === "box" || o[k] === undefined) continue;
    if (k === "lights" && Array.isArray(o.lights)) o.lights.slice(0, MAX_LIGHTS).forEach((l, i) => Object.assign(V.opts.lights[i], l));
    else if (k === "clip" && Array.isArray(o.clip)) o.clip.slice(0, 3).forEach((c, i) => Object.assign(V.opts.clip[i], c));
    else if (k === "heat" && typeof o.heat === "object") Object.assign(V.opts.heat, o.heat);
    else if (typeof o[k] === typeof V.opts[k]) V.opts[k] = o[k];
  }
  if (LAYOUTS[s.layout]) V.layout = s.layout;
  if (Array.isArray(s.vp)) s.vp.slice(0, 4).forEach((x, i) => { if (MODES[x?.mode]) V.vp[i].mode = x.mode; if (Number.isInteger(x?.lod)) V.vp[i].lod = x.lod; });
  if (Array.isArray(s.cams)) s.cams.slice(0, 4).forEach((c, i) => { if (c && Number.isFinite(c.fov)) V.cams[i].set({ fov: c.fov, ortho: !!c.ortho }, true); });
  if (typeof s.link === "boolean") D.link = s.link;
  if (["start", "neutral", "snapshot"].includes(s.refSource)) D.refSource = s.refSource;
  if (typeof s.tab === "string") D.tab = s.tab;
  if (typeof s.preset === "string") D.preset = s.preset;
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
  D.rows3 = { offs: new Uint32Array(blob, L.offsets.offset, L.offsets.count), index: new Uint32Array(blob, L.index.offset, L.index.count),
              delta: new Int16Array(blob, L.delta.offset, L.delta.count), scale: new Float32Array(blob, L.scale.offset, L.scale.count) };
}

function targetVertices(t, fraction = 0.25) {
  const R = D.rows3, out = [];
  let top = 0;
  const mags = [];
  for (let e = R.offs[t]; e < R.offs[t + 1]; e++) {
    const m = Math.hypot(R.delta[e * 3], R.delta[e * 3 + 1], R.delta[e * 3 + 2]) * R.scale[t];
    mags.push(m);
    if (m > top) top = m;
  }
  for (let e = R.offs[t], j = 0; e < R.offs[t + 1]; e++, j++) if (mags[j] >= top * fraction && top > 0) out.push(R.index[e]);
  return out;
}

function buildRegions() {
  const R = D.rows3, cats = [...new Set(D.targets.map((t) => t.category))].filter((c) => c !== "head");
  const names = ["rest of head", ...cats], best = new Float32Array(D.n), reg = new Uint8Array(D.n);
  let skipped = 0;
  D.targets.forEach((t, ti) => {
    const id = names.indexOf(t.category);
    if (id <= 0) return;
    const lo = R.offs[ti], hi = R.offs[ti + 1], mag = new Float32Array(hi - lo);
    let top = 0;
    for (let e = lo; e < hi; e++) { mag[e - lo] = Math.hypot(R.delta[e * 3], R.delta[e * 3 + 1], R.delta[e * 3 + 2]); top = Math.max(top, mag[e - lo]); }
    if (top <= 0) return;
    let wide = 0;
    for (let j = 0; j < mag.length; j++) if (mag[j] >= 0.5 * top) wide++;
    if (wide > 0.2 * D.n) { skipped++; return; }
    for (let e = lo; e < hi; e++) {
      const g = R.index[e], f = mag[e - lo] / top;
      if (f >= 0.35 && f > best[g]) { best[g] = f; reg[g] = id; }
    }
  });
  const counts = names.map(() => 0);
  for (let g = 0; g < D.n; g++) counts[reg[g]]++;
  D.regions = { names, counts, skipped_wide_targets: skipped };
  V.setRegions(reg, names);
}

async function context() {
  for (let attempt = 0; attempt < 8; attempt++) {
    let canvas = $("gl");
    if (attempt > 0) {
      const fresh = canvas.cloneNode(false);
      canvas.replaceWith(fresh);
      canvas = fresh;
      await new Promise((r) => setTimeout(r, 400));
    }
    const g = canvas.getContext("webgl2", { antialias: true, preserveDrawingBuffer: true, stencil: true });
    if (g && !g.isContextLost()) {
      canvas.addEventListener("webglcontextlost", (e) => {
        e.preventDefault();
        D.contextLost = (D.contextLost || 0) + 1;
        setStatus("busy", "graphics context lost; waiting for the browser to restore it");
      });
      canvas.addEventListener("webglcontextrestored", () => {
        D.contextRestored = (D.contextRestored || 0) + 1;
        if (V) { V.restore(); request(true); }
        setStatus("settled", "graphics context restored");
      });
      D.contextAttempts = attempt + 1;
      return g;
    }
  }
  return null;
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

function activePose() { let last = null; for (const p of D.poses.values()) last = p; return last; }

function frame(ts) {
  D.raf = 0;
  const t0 = performance.now();
  if (D.hooks.beforeFrame) D.hooks.beforeFrame();
  if (D.dirty && V.meshes.has("head")) {
    const pose = activePose();
    if (pose) V.updateHead(pose.positions, pose.normals);
    else {
      D.lastUsed = computePreview();
      const v = views();
      V.updateHead(v.positions, v.normals);
    }
    D.dirty = false;
  }
  const dt = D.lastTs ? Math.max(0, (ts - D.lastTs) / 1000) : 0.016;
  const moving = V.render(dt);
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
  if (moving) request(false);
}

function request(dirty = true) {
  if (dirty) D.dirty = true;
  if (!D.raf && V) D.raf = requestAnimationFrame(frame);
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

function needUV() { return V.opts.seams || V.vp.some((s) => s.mode === MODE_INDEX.uv); }

function notify(kind, extra = {}) {
  if (!D.listeners.length) return;
  if (kind === "slider") {
    if (D.sliderNotice) return;
    D.sliderNotice = setTimeout(() => { D.sliderNotice = 0; fire("slider", {}); }, 0);
    return;
  }
  setTimeout(() => fire(kind, extra), 0);
}
function fire(kind, extra) {
  const state = JSON.parse(JSON.stringify(settleInput()));
  for (const cb of D.listeners) { try { cb(state, { kind, ...extra }); } catch (e) { console.error(e); } }
}

async function settle(full) {
  if (D.settling) { D.pendingSettle = 1; return; }
  clearTimeout(D.fullTimer);
  D.settling = true;
  setStatus("busy", "settling");
  const wSent = D.w.slice(), mSent = { ...D.macroW };
  const t0 = performance.now();
  let r;
  const wantUV = needUV();
  try {
    const input = settleInput();
    if (params.get("fast") === "1" && !full) input.fast = true;
    if (wantUV) input.uvs = true;
    r = await api("designer.settle", input);
  } catch (e) {
    D.settling = false;
    setStatus("preview", `settle failed: ${e.message}`);
    return;
  }
  const pos = decodeF32(r.positions_f32_b64);
  const rtt = performance.now() - t0;
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
  if (!D.startPositions) D.startPositions = pos.slice();
  if (r.triangle_uvs_f32_b64 && V.meshes.get("head")) { V.setLodUVs(V.meshes.get("head").lods[0], decodeF32(r.triangle_uvs_f32_b64)); D.placedUVs = true; }
  D.w0 = wSent;
  D.macroW0 = mSent;
  D.macroDelta = {};
  D.settled = r;
  D.settledPositions = pos;
  D.settleCount++;
  D.settling = false;
  const kind = r.approximate ? `${r.placement} draft (approximate, ${r.draft.rounds} of ${r.draft.full_rounds} fit rounds; save and export use the full placement)` : r.placement;
  setStatus(changed() ? "preview" : "settled", changed() ? "preview (approximate)" : `settled ${kind} ${rtt.toFixed(0)} ms`);
  $("info").textContent = `${D.n} vertices, ${D.tri} triangles, ${D.targets.length} targets loaded, ${V.renderer || ""}`;
  request(true);
  notify("settle", { positions: pos, measurements: r.measurements, revision: r.revision, placement: r.placement });
  if (D.pendingSettle) { D.pendingSettle = 0; settle(); }
  else if (r.approximate && !changed()) D.fullTimer = setTimeout(() => { if (!changed() && !D.settling) settle(true); }, 3000);
}

function scheduleSettle() {
  clearTimeout(D.timer);
  clearTimeout(D.fullTimer);
  D.timer = setTimeout(settle, 150);
}

function renderHistory() {
  const H = D.history, list = $("history");
  $("undo").disabled = !H.done.length;
  $("redo").disabled = !H.undone.length;
  $("undo").title = H.done.length ? `Undo ${H.done[H.done.length - 1].label} (Ctrl+Z)` : "Nothing to undo (Ctrl+Z)";
  $("redo").title = H.undone.length ? `Redo ${H.undone[H.undone.length - 1].label} (Ctrl+Y)` : "Nothing to redo (Ctrl+Y)";
  scheduleDirty();
  if (!list) return;
  const rows = [`<li class="${H.done.length ? "" : "now"}" data-at="0">start</li>`];
  H.done.forEach((e, i) => rows.push(`<li class="${i === H.done.length - 1 ? "now" : ""}" data-at="${i + 1}">${escapeHtml(e.label)}</li>`));
  [...H.undone].reverse().forEach((e, i) => rows.push(`<li class="undone" data-at="${H.done.length + i + 1}">${escapeHtml(e.label)}</li>`));
  list.innerHTML = rows.join("");
}
const escapeHtml = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

function pushUndo(entry) {
  if (!entry || typeof entry.undo !== "function" || typeof entry.redo !== "function") throw new Error("pushUndo needs { label, undo(), redo() }");
  const H = D.history;
  H.done.push({ label: String(entry.label || "edit"), undo: entry.undo, redo: entry.redo });
  if (H.done.length > H.limit) H.done.shift();
  H.undone.length = 0;
  renderHistory();
}
function undo() {
  const e = D.history.done.pop();
  if (!e) return;
  try { e.undo(); } catch (err) { toast(`undo failed: ${err.message}`); }
  D.history.undone.push(e);
  renderHistory();
  toast(`undo: ${e.label}`);
}
function redo() {
  const e = D.history.undone.pop();
  if (!e) return;
  try { e.redo(); } catch (err) { toast(`redo failed: ${err.message}`); }
  D.history.done.push(e);
  renderHistory();
  toast(`redo: ${e.label}`);
}
function historyTo(at) {
  while (D.history.done.length > at) undo();
  while (D.history.done.length < at && D.history.undone.length) redo();
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

function showWeight(i) {
  const s = D.rows[i], t = D.targets[i];
  s.input.value = D.w[i];
  s.out.textContent = D.w[i].toFixed(3);
  s.row.classList.toggle("changed", D.w[i] !== (D.start.weights[t.id] || 0));
}

function setWeights(pairs, macros) {
  for (const [i, x] of pairs) { D.w[i] = x; D.committed[i] = x; showWeight(i); }
  if (macros) for (const [name, x] of Object.entries(macros)) {
    D.macroW[name] = x;
    D.macroCommitted[name] = x;
    const m = (D.macroRows || []).find((r) => r.name === name);
    if (m) { m.input.value = x; m.out.textContent = Number(x).toFixed(3); }
  }
  setStatus("preview", "preview (approximate)");
  request(true);
  notify("slider");
  scheduleSettle();
}

function buildSliders() {
  const list = $("list"), cats = $("category");
  let current = "";
  D.rows = [];
  D.committed = D.w.slice();
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
    const name = t.id.split("/").pop();
    const s = sliderRow(name, D.w[i], (x) => {
      D.w[i] = x;
      D.lastTarget = i;
      s.row.classList.toggle("changed", x !== (D.start.weights[t.id] || 0));
      setStatus("preview", "preview (approximate)");
      request(true);
      notify("slider");
    }, () => {
      const from = D.committed[i], to = D.w[i];
      if (from !== to) {
        D.committed[i] = to;
        pushUndo({ label: `${name} ${from.toFixed(3)} → ${to.toFixed(3)}`, undo: () => setWeights([[i, from]]), redo: () => setWeights([[i, to]]) });
      }
      scheduleSettle();
    });
    s.row.dataset.id = t.id;
    s.row.dataset.category = t.category;
    s.input.addEventListener("dblclick", () => { s.input.value = D.start.weights[t.id] || 0; s.input.dispatchEvent(new Event("input")); s.input.dispatchEvent(new Event("change")); });
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
    const before = D.w.slice(), mBefore = { ...D.macroW };
    const after = D.targets.map((t) => D.start.weights[t.id] || 0), mAfter = { ...D.macroStart };
    const apply = (w, m) => setWeights(w.map((x, i) => [i, x]), m);
    apply(after, mAfter);
    pushUndo({ label: "reset all sliders", undo: () => apply(before, mBefore), redo: () => apply(after, mAfter) });
  });
}

function buildMacros() {
  const box = $("macro-list");
  D.macroBase = {};
  D.macroStart = {};
  for (const s of D.macros.sliders) { D.macroBase[s.name] = s.base; D.macroStart[s.name] = s.value; D.macroW[s.name] = s.value; }
  D.macroW0 = { ...D.macroW };
  D.macroCommitted = { ...D.macroW };
  if (!D.macros.available) {
    box.innerHTML = `<div class="note">unavailable: ${D.macros.reason}</div>`;
    return;
  }
  D.macroRows = D.macros.sliders.map((m) => {
    const s = sliderRow(m.name, m.value, (x) => { D.macroW[m.name] = x; setStatus("preview", "preview (approximate)"); request(true); notify("slider"); }, () => {
      const from = D.macroCommitted[m.name], to = D.macroW[m.name];
      if (from !== to) {
        D.macroCommitted[m.name] = to;
        pushUndo({ label: `macro ${m.name} ${Number(from).toFixed(3)} → ${Number(to).toFixed(3)}`, undo: () => setWeights([], { [m.name]: from }), redo: () => setWeights([], { [m.name]: to }) });
      }
      scheduleSettle();
    }, async () => {
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

function slotsUsed() { return V.rects().map((r) => r.slot); }
function setMode(i) {
  const k = ((i % MODES.length) + MODES.length) % MODES.length;
  if (D.link) for (const s of slotsUsed()) V.vp[s].mode = k; else V.activeVp().mode = k;
  if (k === MODE_INDEX.displacement) ensureReference();
  if (needUV() && !D.placedUVs && D.ready) settle();
  refreshToolbar();
  request(false);
  saveSettings();
}
function setLod(i) {
  const head = V.meshes.get("head"), n = head ? head.lods.length : 1, k = Math.max(0, Math.min(n - 1, i));
  if (D.link) for (const s of slotsUsed()) V.vp[s].lod = k; else V.activeVp().lod = k;
  refreshToolbar();
  request(false);
  saveSettings();
}
function setLayout(name) {
  if (!LAYOUTS[name]) return;
  const mode = V.activeVp().mode, lod = V.activeVp().lod;
  V.layout = name;
  V.active = Math.min(V.active, LAYOUTS[name].length - 1);
  if (D.link) for (const s of slotsUsed()) { V.vp[s].mode = mode; V.vp[s].lod = lod; }
  refreshToolbar();
  request(false);
  saveSettings();
}
const NAMED = { front: [0, 0], back: [Math.PI, 0], left: [Math.PI / 2, 0], right: [-Math.PI / 2, 0], top: [0, Math.PI / 2], bottom: [0, -Math.PI / 2], threequarter: [0.61, 0.14] };
function namedView(name) {
  const v = NAMED[name];
  if (!v) return;
  const cam = V.activeCam();
  cam.view(v[0], v[1]);
  cam.name = name === "threequarter" ? "three-quarter" : name;
  refreshToolbar();
  request(false);
}
function toggleOrtho() {
  const cam = V.activeCam();
  cam.ortho = !cam.ortho;
  refreshToolbar();
  request(false);
  saveSettings();
}
function setFov(f) {
  const cam = V.activeCam(), old = cam.goal.fov, nf = Math.max(10, Math.min(90, f));
  cam.set({ fov: nf, dist: (cam.goal.dist * Math.tan((old * Math.PI) / 360)) / Math.tan((nf * Math.PI) / 360) });
  refreshToolbar();
  request(false);
  saveSettings();
}
function frameHead() {
  const head = V.meshes.get("head");
  if (!head?.cpuPos) return;
  V.frameBox(V.computeBounds(head.cpuPos), [V.activeCam()]);
  V.pivotFlash = 0.6;
  request(false);
}
function frameSelection() {
  const head = V.meshes.get("head");
  if (!head?.cpuPos) return;
  let pts = [];
  if (D.lastTarget >= 0) pts = targetVertices(D.lastTarget);
  if (pts.length) {
    const sel = new Float32Array(pts.length * 3);
    pts.forEach((g, j) => sel.set(head.cpuPos.subarray(g * 3, g * 3 + 3), j * 3));
    const b = V.computeBounds(sel);
    b.radius = Math.max(b.radius, 0.01);
    V.frameBox(b, [V.activeCam()]);
    toast(`framed ${D.targets[D.lastTarget].id} (${pts.length} vertices)`);
  } else if (D.lastPick) {
    V.frameBox({ centre: D.lastPick, radius: 0.03 }, [V.activeCam()]);
    toast("framed the picked point");
  } else { toast("nothing selected: move a slider or double-click the surface first"); return; }
  V.pivotFlash = 0.6;
  request(false);
}

async function ensureReference() {
  const head = V.meshes.get("head");
  if (!head) return;
  try {
    let ref = null;
    if (D.refSource === "start") ref = D.startPositions;
    else if (D.refSource === "snapshot") ref = head.cpuPos ? head.cpuPos.slice() : null;
    else {
      if (!D.neutral) {
        toast("settling the neutral base for the reference");
        const input = { store, ...D.base, placement: D.start.placement, name: "neutral", weights: {} };
        if (opt("eyes")) input.eyes = opt("eyes");
        D.neutral = decodeF32((await api("designer.settle", input)).positions_f32_b64);
      }
      ref = D.neutral;
    }
    V.setReference(ref);
    request(false);
  } catch (e) { toast(`reference failed: ${e.message}`); }
}

function screenshot(which) {
  V.render(0);
  const url = V.screenshot(which);
  const a = document.createElement("a");
  a.href = url;
  a.download = `gazeforge_${which}_${new Date().toISOString().replace(/[:.]/g, "-")}.png`;
  document.body.appendChild(a);
  a.click();
  a.remove();
  return url;
}

function visItems() {
  const items = [];
  for (const m of V.meshes.values()) items.push({ kind: "mesh", id: m.id, label: m.label || m.id, on: m.visible });
  for (const o of V.overlays.values()) items.push({ kind: "overlay", id: o.id, label: `${o.id} (overlay)`, on: o.visible });
  return items;
}
function visState() { return visItems().map((x) => [x.kind, x.id, x.on]); }
function applyVis(state) {
  for (const [kind, id, on] of state) { const t = kind === "mesh" ? V.meshes.get(id) : V.overlays.get(id); if (t) t.visible = on; }
  renderVisibility();
  request(false);
}
function visEdit(label, mutate) {
  const before = visState();
  mutate();
  const after = visState();
  if (JSON.stringify(before) === JSON.stringify(after)) return;
  applyVis(after);
  pushUndo({ label, undo: () => applyVis(before), redo: () => applyVis(after) });
}
function setVisible(kind, id, on) {
  visEdit(`${on ? "show" : "hide"} ${id}`, () => { const t = kind === "mesh" ? V.meshes.get(id) : V.overlays.get(id); if (t) t.visible = on; });
}
function isolate(kind, id) {
  visEdit(`isolate ${id}`, () => { for (const m of V.meshes.values()) m.visible = kind === "mesh" && m.id === id; for (const o of V.overlays.values()) o.visible = kind === "overlay" && o.id === id; });
}
function showAll() {
  visEdit("show everything", () => { for (const m of V.meshes.values()) m.visible = true; for (const o of V.overlays.values()) o.visible = true; });
  if (V.opts.hiddenRegions) setRegionMask(0, "show all regions");
}
function renderVisibility() {
  const box = $("visibility");
  if (!box) return;
  box.innerHTML = "";
  for (const it of visItems()) {
    const row = document.createElement("div");
    row.className = "vis";
    row.innerHTML = `<label><input type="checkbox" ${it.on ? "checked" : ""}> ${escapeHtml(it.label)}</label><button class="small">isolate</button>`;
    row.querySelector("input").addEventListener("change", (e) => setVisible(it.kind, it.id, e.target.checked));
    row.querySelector("button").addEventListener("click", () => isolate(it.kind, it.id));
    box.appendChild(row);
  }
}
function setRegionMask(mask, label) {
  const before = V.opts.hiddenRegions;
  if (before === mask) return;
  const apply = (m) => { V.opts.hiddenRegions = m; renderRegions(); request(false); };
  apply(mask);
  pushUndo({ label, undo: () => apply(before), redo: () => apply(mask) });
}
function renderRegions() {
  const box = $("regions");
  if (!box || !D.regions) return;
  box.innerHTML = "";
  D.regions.names.forEach((name, i) => {
    const row = document.createElement("label");
    row.className = "region";
    const hidden = (V.opts.hiddenRegions >>> i) & 1;
    row.innerHTML = `<input type="checkbox" ${hidden ? "" : "checked"}> ${escapeHtml(name)} <span class="muted">${D.regions.counts[i]}</span>`;
    row.querySelector("input").addEventListener("change", (e) => setRegionMask(e.target.checked ? V.opts.hiddenRegions & ~(1 << i) : V.opts.hiddenRegions | (1 << i), `${e.target.checked ? "show" : "hide"} region ${name}`));
    box.appendChild(row);
  });
}
function bindOpt(el) {
  const path = el.dataset.opt.split(".");
  const holder = () => path.slice(0, -1).reduce((o, k) => o[k], V.opts);
  const key = path[path.length - 1];
  const numeric = el.type === "range" || el.type === "number" || "num" in el.dataset;
  const read = () => (el.type === "checkbox" ? el.checked : numeric ? Number(el.value) : el.value);
  const out = el.parentElement.querySelector(".val");
  const write = () => {
    const v = holder()[key];
    if (el.type === "checkbox") el.checked = !!v; else el.value = v;
    if (out) out.textContent = el.dataset.fmt === "mm" ? `${(Number(v) * 1000).toFixed(1)} mm` : Number.isFinite(Number(v)) && el.type === "range" ? Number(v).toFixed(Number(el.step) < 0.1 ? 2 : 0) : "";
  };
  const isBox = path[0] === "box";
  let boxBefore = null;
  el.addEventListener("pointerdown", () => { if (isBox) boxBefore = JSON.stringify(V.opts.box); });
  el.addEventListener("input", () => {
    if (isBox && boxBefore === null) boxBefore = JSON.stringify(V.opts.box);
    holder()[key] = read();
    write();
    if (path[0] === "lights" || path[0] === "ambient" || path[0] === "ground") { D.preset = "custom"; $("preset").value = "custom"; }
    if (path[0] === "seams" && needUV() && !D.placedUVs && D.ready) settle();
    request(false);
    if (!isBox) saveSettings();
  });
  if (isBox) el.addEventListener("change", () => {
    const before = boxBefore || JSON.stringify(V.opts.box), after = JSON.stringify(V.opts.box);
    boxBefore = null;
    if (before === after) return;
    const apply = (s) => { Object.assign(V.opts.box, JSON.parse(s)); refreshOpts(); request(false); };
    pushUndo({ label: path[1] === "on" ? `${V.opts.box.on ? "hide" : "show"} box region` : "move the hide box", undo: () => apply(before), redo: () => apply(after) });
  });
  D.optWriters.push(write);
  write();
}
function refreshOpts() { for (const w of D.optWriters) w(); }

function applyPreset(name) {
  const p = LIGHT_PRESETS[name];
  if (!p) return;
  D.preset = name;
  V.opts.ambient = p.ambient;
  V.opts.ground = p.ground;
  V.opts.lights.forEach((l, i) => Object.assign(l, p.lights[i] ? { on: true, ...p.lights[i] } : { on: false }));
  refreshOpts();
  $("preset").value = name;
  request(false);
  saveSettings();
}

function toggleOpt(path, label) {
  const parts = path.split("."), holder = parts.slice(0, -1).reduce((o, k) => o[k], V.opts), key = parts[parts.length - 1];
  holder[key] = !holder[key];
  refreshOpts();
  if (path === "seams" && needUV() && !D.placedUVs && D.ready) settle();
  toast(`${label} ${holder[key] ? "on" : "off"}`);
  request(false);
  saveSettings();
}

function refreshToolbar() {
  const st = V.activeVp(), cam = V.activeCam();
  $("mode").value = String(st.mode);
  $("lod").value = String(st.lod);
  $("layout").value = V.layout;
  $("proj").textContent = cam.ortho ? "ortho" : "persp";
  $("fov").value = cam.goal.fov;
  $("fov-val").textContent = `${cam.goal.fov.toFixed(0)}°`;
  $("link").checked = D.link;
  request(false);
}

function buildControls() {
  const mode = $("mode");
  MODES.forEach((m, i) => { const o = document.createElement("option"); o.value = String(i); o.textContent = m.label; mode.appendChild(o); });
  mode.addEventListener("input", () => setMode(Number(mode.value)));
  const lod = $("lod"), head = V.meshes.get("head");
  head.lods.forEach((l, i) => { const o = document.createElement("option"); o.value = String(i); o.textContent = `LOD${i} · ${l.tri} tris`; lod.appendChild(o); });
  lod.addEventListener("input", () => setLod(Number(lod.value)));
  $("layout").addEventListener("input", (e) => setLayout(e.target.value));
  $("proj").addEventListener("click", toggleOrtho);
  $("named").addEventListener("input", (e) => { if (e.target.value) namedView(e.target.value); e.target.value = ""; });
  $("frame").addEventListener("click", frameHead);
  $("frame-sel").addEventListener("click", frameSelection);
  $("shot").addEventListener("click", (e) => screenshot(e.shiftKey ? "layout" : "viewport"));
  $("shot-layout").addEventListener("click", () => screenshot("layout"));
  $("help-btn").addEventListener("click", () => $("help").classList.toggle("hidden"));
  $("help").addEventListener("click", (e) => { if (e.target === $("help")) $("help").classList.add("hidden"); });
  $("undo").addEventListener("click", undo);
  $("redo").addEventListener("click", redo);
  $("history").addEventListener("click", (e) => { const li = e.target.closest("li"); if (li) historyTo(Number(li.dataset.at)); });
  $("fov").addEventListener("input", (e) => setFov(Number(e.target.value)));
  $("link").addEventListener("change", (e) => { D.link = e.target.checked; saveSettings(); });
  $("show-all").addEventListener("click", showAll);
  $("ref").value = D.refSource;
  $("ref").addEventListener("input", (e) => { D.refSource = e.target.value; ensureReference(); saveSettings(); });
  $("ref-now").addEventListener("click", () => { D.refSource = "snapshot"; $("ref").value = "snapshot"; ensureReference(); saveSettings(); });
  const preset = $("preset");
  for (const [k, p] of Object.entries(LIGHT_PRESETS)) { const o = document.createElement("option"); o.value = k; o.textContent = p.label; preset.appendChild(o); }
  const c = document.createElement("option"); c.value = "custom"; c.textContent = "custom"; preset.appendChild(c);
  preset.value = D.preset;
  preset.addEventListener("input", () => applyPreset(preset.value));
  const lights = $("lights");
  for (let i = 0; i < MAX_LIGHTS; i++) {
    const row = document.createElement("div");
    row.className = "light";
    row.innerHTML = `<label class="ttl"><input type="checkbox" data-opt="lights.${i}.on"> light ${i + 1}</label>
      <label>azimuth <input type="range" min="-180" max="180" step="1" data-opt="lights.${i}.az"><span class="val"></span></label>
      <label>elevation <input type="range" min="-90" max="90" step="1" data-opt="lights.${i}.el"><span class="val"></span></label>
      <label>intensity <input type="range" min="0" max="3" step="0.01" data-opt="lights.${i}.i"><span class="val"></span></label>
      <label>colour K <input type="range" min="2000" max="12000" step="100" data-opt="lights.${i}.k"><span class="val"></span></label>`;
    lights.appendChild(row);
  }
  const clips = $("clips");
  for (let i = 0; i < 3; i++) {
    const row = document.createElement("div");
    row.className = "clip";
    row.innerHTML = `<label class="ttl"><input type="checkbox" data-opt="clip.${i}.on"> plane ${i + 1}</label>
      <label>axis <select data-opt="clip.${i}.axis"><option value="x">x (left-right)</option><option value="y">y (front-back)</option><option value="z">z (up-down)</option><option value="free">free</option></select></label>
      <label>offset <input type="range" min="-0.15" max="0.15" step="0.0005" data-opt="clip.${i}.offset" data-fmt="mm"><span class="val"></span></label>
      <label>azimuth (free) <input type="range" min="-180" max="180" step="1" data-opt="clip.${i}.az"><span class="val"></span></label>
      <label>elevation (free) <input type="range" min="-90" max="90" step="1" data-opt="clip.${i}.el"><span class="val"></span></label>
      <label><input type="checkbox" data-opt="clip.${i}.flip"> keep the other side</label>
      <label><input type="checkbox" data-opt="clip.${i}.slab"> slab, thickness <input type="range" min="0.0005" max="0.05" step="0.0005" data-opt="clip.${i}.thick" data-fmt="mm"><span class="val"></span></label>`;
    clips.appendChild(row);
  }
  document.querySelectorAll("[data-opt]").forEach(bindOpt);
  document.querySelectorAll(".tabs button").forEach((b) => b.addEventListener("click", () => showTab(b.dataset.tab)));
  $("help-list").innerHTML = SHORTCUTS.map(([k, what]) => `<tr><td><kbd>${escapeHtml(k)}</kbd></td><td>${escapeHtml(what)}</td></tr>`).join("");
  showTab(D.tab);
  refreshToolbar();
  renderHistory();
  renderVisibility();
  renderRegions();
}

function showTab(name) {
  if (!$(`tab-${name}`)) name = "shape";
  D.tab = name;
  document.querySelectorAll(".tabs button").forEach((b) => b.classList.toggle("on", b.dataset.tab === name));
  document.querySelectorAll(".tab").forEach((t) => t.classList.toggle("on", t.id === `tab-${name}`));
  saveSettings();
}

function interaction() {
  const c = gl.canvas;
  let drag = null;
  c.addEventListener("contextmenu", (e) => e.preventDefault());
  c.addEventListener("pointerdown", (e) => {
    const r = V.rectAt(e.clientX, e.clientY);
    V.active = r.slot;
    drag = { x: e.clientX, y: e.clientY, pan: e.button === 1 || e.button === 2 || e.shiftKey, cam: V.cams[r.cam], h: r.h };
    c.setPointerCapture(e.pointerId);
    V.pivotFlash = 0.4;
    refreshToolbar();
  });
  c.addEventListener("pointermove", (e) => {
    if (!drag) return;
    const dx = e.clientX - drag.x, dy = e.clientY - drag.y, cam = drag.cam;
    drag.x = e.clientX;
    drag.y = e.clientY;
    if (drag.pan) {
      const b = cam.basis(cam.goal), wpp = (2 * cam.goal.dist * Math.tan((cam.goal.fov * Math.PI) / 360)) / drag.h;
      cam.set({ target: cam.goal.target.map((x, k) => x - b.s[k] * dx * wpp + b.u[k] * dy * wpp) });
    } else {
      cam.set({ yaw: cam.goal.yaw - dx * 0.008, pitch: cam.goal.pitch + dy * 0.008 });
      if (cam.name !== "perspective") cam.name = "free";
    }
    V.pivotFlash = 0.4;
    request(false);
  });
  const end = () => { drag = null; };
  c.addEventListener("pointerup", end);
  c.addEventListener("pointercancel", end);
  c.addEventListener("wheel", (e) => {
    e.preventDefault();
    if (e.altKey) {
      const p = V.opts.clip[0];
      p.on = true;
      p.offset = Math.max(-0.15, Math.min(0.15, p.offset - Math.sign(e.deltaY) * 0.001));
      refreshOpts();
      request(false);
      return;
    }
    const r = V.rectAt(e.clientX, e.clientY), cam = V.cams[r.cam];
    cam.set({ dist: cam.goal.dist * Math.exp(e.deltaY * 0.0012) });
    request(false);
  }, { passive: false });
  c.addEventListener("dblclick", (e) => {
    const hit = V.pick(e.clientX, e.clientY);
    if (!hit) { toast("no surface under the cursor"); return; }
    const r = V.rectAt(e.clientX, e.clientY);
    V.cams[r.cam].set({ target: hit.point });
    D.lastPick = hit.point;
    D.lastTarget = -1;
    V.pivotFlash = 0.8;
    toast(`pivot on ${hit.mesh} at ${hit.point.map((x) => (x * 1000).toFixed(1)).join(", ")} mm`);
    request(false);
  });
  window.addEventListener("resize", () => request(false));
  window.addEventListener("keydown", (e) => {
    const k = e.key, ctrl = e.ctrlKey || e.metaKey;
    if (ctrl && !e.altKey && D.ready) {
      const kl = k.toLowerCase();
      if (kl === "s") { e.preventDefault(); if (e.shiftKey) saveProjectAs(); else saveProject(); return; }
      if (kl === "o") { e.preventDefault(); openProject(); return; }
      if (kl === "n") { e.preventDefault(); newProject(); return; }
    }
    if (k === "Escape" && P.dialogDone) { P.dialogDone(false); return; }
    if (e.target.matches?.("input[type=text], input[type=search], textarea") || P.dialogDone) return;
    if (ctrl && (k === "z" || k === "Z") && !e.shiftKey) { e.preventDefault(); undo(); return; }
    if (ctrl && (k === "y" || ((k === "z" || k === "Z") && e.shiftKey))) { e.preventDefault(); redo(); return; }
    if (ctrl || (e.altKey && k !== "h" && k !== "H")) return;
    const code = e.code, handled = () => e.preventDefault();
    if (k === "?") { $("help").classList.toggle("hidden"); return handled(); }
    if (k === "Escape") { $("help").classList.add("hidden"); return; }
    if (code === "Digit1" || code === "Numpad1") { namedView(e.shiftKey ? "back" : "front"); return handled(); }
    if (code === "Digit3" || code === "Numpad3") { namedView(e.shiftKey ? "right" : "left"); return handled(); }
    if (code === "Digit7" || code === "Numpad7") { namedView(e.shiftKey ? "bottom" : "top"); return handled(); }
    if (code === "Digit9" || code === "Numpad9") { namedView("threequarter"); return handled(); }
    if (code === "Digit5" || code === "Numpad5" || k === "o" || k === "O") { toggleOrtho(); return handled(); }
    switch (k.toLowerCase()) {
      case "f": e.shiftKey ? frameSelection() : frameHead(); break;
      case "l": setLayout({ single: "side", side: "quad", quad: "single" }[V.layout]); break;
      case "n": V.active = (V.active + 1) % V.rects().length; V.pivotFlash = 0.4; refreshToolbar(); break;
      case "m": setMode(V.activeVp().mode + (e.shiftKey ? -1 : 1)); toast(MODES[V.activeVp().mode].label); break;
      case "w": toggleOpt("wire", "wireframe"); break;
      case "x": toggleOpt("xray", "hidden edges"); break;
      case "p": toggleOpt("points", "vertex points"); break;
      case "b": toggleOpt("backHi", "back-face highlight"); break;
      case "e": toggleOpt("seams", "seams and boundaries"); break;
      case "c": toggleOpt("clip.0.on", "clip plane 1"); break;
      case "g": toggleOpt("lightGizmo", "light gizmo"); break;
      case "h": if (e.altKey) showAll(); else setVisible("mesh", "head", !V.meshes.get("head").visible); break;
      case "i": isolate("mesh", "head"); break;
      case "k": screenshot(e.shiftKey ? "layout" : "viewport"); break;
      case "[": setFov(V.activeCam().goal.fov - 5); break;
      case "]": setFov(V.activeCam().goal.fov + 5); break;
      case ",": setLod(V.activeVp().lod + 1); break;
      case ".": setLod(V.activeVp().lod - 1); break;
      default: return;
    }
    handled();
  });
}

function sphere(centre, radius, rings = 16, segs = 24) {
  const pos = [], idx = [];
  for (let i = 0; i <= rings; i++) {
    const th = (Math.PI * i) / rings;
    for (let j = 0; j <= segs; j++) {
      const ph = (2 * Math.PI * j) / segs;
      pos.push(centre[0] + radius * Math.sin(th) * Math.cos(ph), centre[1] + radius * Math.sin(th) * Math.sin(ph), centre[2] + radius * Math.cos(th));
    }
  }
  for (let i = 0; i < rings; i++) for (let j = 0; j < segs; j++) {
    const a = i * (segs + 1) + j, b = a + segs + 1;
    idx.push(a, b, a + 1, a + 1, b, b + 1);
  }
  return { pos, idx };
}

async function loadScene() {
  const input = { store, ...D.base };
  if (opt("eyes")) input.eyes = opt("eyes");
  const sc = await api("designer.scene", input);
  D.scene = { lods: sc.lods.map((l) => ({ target: l.target, triangles: l.triangles, dropped: l.dropped_triangles })), eyes: sc.eyes };
  const head = V.setHead(D.n, D.indices, sc.triangle_uvs_f32_b64 ? decodeF32(sc.triangle_uvs_f32_b64) : null);
  head.label = "head (LOD0)";
  for (const l of sc.lods) V.addHeadLod(decodeU32(l.triangles_u32_b64), decodeF32(l.triangle_uvs_f32_b64), `LOD ${l.target}`);
  if (sc.eyes.length) {
    const pos = [], idx = [];
    for (const e of sc.eyes) { const s = sphere(e.centre, e.radius); const o = pos.length / 3; pos.push(...s.pos); idx.push(...s.idx.map((x) => x + o)); }
    V.setExtra("eyes", { positions: pos, indices: idx, color: [0.93, 0.93, 0.95], label: "eyes (spheres)" }, "eyes");
  }
}

function setPose(id, positions) {
  if (positions == null) {
    D.poses.delete(id);
  } else {
    const p = positions instanceof Float32Array ? positions : Float32Array.from(positions);
    if (p.length !== D.n * 3) throw new Error(`setPose needs ${D.n * 3} floats (game LOD0 order), got ${p.length}`);
    D.poses.delete(id);
    D.poses.set(id, { positions: p, normals: vertexNormals(p, D.indices, D.n) });
  }
  request(true);
}

function makeApi(panelId) {
  return {
    call: (command, input) => api(command, input ?? {}),
    character: () => JSON.parse(JSON.stringify(settleInput())),
    onCharacterChange: (cb) => { if (typeof cb === "function") D.listeners.push(cb); },
    setPose,
    setExtraMesh: (id, spec) => { V.setExtra(String(id), spec || null); pendingVisibility("mesh", String(id)); renderVisibility(); request(false); },
    poseExtraMesh: (id, rotation, centre) => { const ok = V.poseExtra(String(id), rotation || null, centre || null); request(false); return ok; },
    setOverlay: (id, spec) => { V.setOverlay(String(id), spec || null); pendingVisibility("overlay", String(id)); renderVisibility(); request(false); },
    pushUndo,
    requestRender: () => request(false),
    panel: panelId,
  };
}

window.GF = {
  version: 1,
  registerPanel(def) {
    if (!def || typeof def.id !== "string" || typeof def.mount !== "function") throw new Error("registerPanel needs { id, title, order, mount(rootEl, api) }");
    D.panelDefs.push(def);
  },
};

async function loadPanels() {
  const box = $("panels");
  let ids = [];
  try { ids = await (await fetch("panels/index.json", { cache: "no-store" })).json(); } catch (e) { box.innerHTML = `<div class="note">no panels/index.json: ${escapeHtml(e.message)}</div>`; return; }
  if (!Array.isArray(ids)) ids = [];
  for (const [k, id] of ids.entries()) {
    if (typeof id !== "string" || !/^[A-Za-z0-9_-]+$/.test(id)) continue;
    const before = D.panelDefs.length;
    try { await import(`./panels/${id}.js`); } catch (e) { D.panels.push({ id, error: e.message }); box.insertAdjacentHTML("beforeend", `<div class="note">panel ${escapeHtml(id)} failed to load: ${escapeHtml(e.message)}</div>`); continue; }
    for (const def of D.panelDefs.slice(before)) def._index = k;
  }
  const defs = [...D.panelDefs].sort((a, b) => (a.order ?? a._index ?? 0) - (b.order ?? b._index ?? 0) || (a._index ?? 0) - (b._index ?? 0));
  for (const def of defs) {
    const d = document.createElement("details");
    d.className = "panel";
    d.open = true;
    d.innerHTML = `<summary>${escapeHtml(def.title || def.id)}</summary><div class="panel-body"></div>`;
    box.appendChild(d);
    try {
      const ret = def.mount(d.querySelector(".panel-body"), makeApi(def.id)) || {};
      const getState = typeof ret.getState === "function" ? ret.getState : typeof def.getState === "function" ? def.getState : null;
      const setState = typeof ret.setState === "function" ? ret.setState : typeof def.setState === "function" ? def.setState : null;
      if (getState || setState) D.panelHooks.set(def.id, { getState, setState });
      D.panels.push({ id: def.id, mounted: true, state: !!getState });
    }
    catch (e) { d.querySelector(".panel-body").innerHTML = `<div class="note">mount failed: ${escapeHtml(e.message)}</div>`; D.panels.push({ id: def.id, error: e.message }); }
  }
  renderVisibility();
  if (D.settled) notify("settle", { positions: D.settledPositions, measurements: D.settled.measurements, revision: D.settled.revision, placement: D.settled.placement });
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

const P = D.project;
const PROJECT_OPTIONS = ["macro", "eyes", "face_morphs", "face_prefix"];
const RKEY = "gazeforge.designer.recent";
const rootArg = () => (opt("root") ? { root: opt("root") } : {});
const slotName = () => P.name || "untitled";

function pendingVisibility(kind, id) {
  const key = `${kind}:${id}`;
  if (!D.pendingVis.has(key)) return;
  const t = kind === "mesh" ? V.meshes.get(id) : V.overlays.get(id);
  if (t) { t.visible = D.pendingVis.get(key); D.pendingVis.delete(key); }
}

function viewerState() {
  return JSON.parse(JSON.stringify({
    opts: V.opts, layout: V.layout, active: V.active, vp: V.vp, link: D.link, refSource: D.refSource, preset: D.preset,
    cams: V.cams.map((c) => ({ yaw: c.goal.yaw, pitch: c.goal.pitch, dist: c.goal.dist, target: c.goal.target, fov: c.goal.fov, ortho: c.ortho, name: c.name })),
    visibility: visState(),
  }));
}

function applyViewerState(s) {
  if (!s || typeof s !== "object" || !s.opts) return;
  applySettings(s);
  const o = s.opts;
  if (o.box && Array.isArray(o.box.lo) && Array.isArray(o.box.hi)) Object.assign(V.opts.box, { on: !!o.box.on, lo: o.box.lo.slice(0, 3), hi: o.box.hi.slice(0, 3) });
  if (Number.isInteger(o.hiddenRegions)) V.opts.hiddenRegions = o.hiddenRegions;
  if (Array.isArray(s.cams)) s.cams.slice(0, 4).forEach((c, i) => {
    if (!c) return;
    V.cams[i].set({ yaw: c.yaw, pitch: c.pitch, dist: c.dist, target: c.target, fov: c.fov, ortho: !!c.ortho }, true);
    if (typeof c.name === "string") V.cams[i].name = c.name;
  });
  if (Number.isInteger(s.active)) V.active = Math.max(0, Math.min(s.active, LAYOUTS[V.layout].length - 1));
  if (Array.isArray(s.visibility)) for (const [kind, id, on] of s.visibility) {
    const t = kind === "mesh" ? V.meshes.get(id) : V.overlays.get(id);
    if (t) t.visible = !!on; else D.pendingVis.set(`${kind}:${id}`, !!on);
  }
  $("preset").value = D.preset;
  $("ref").value = D.refSource;
  refreshOpts();
  refreshToolbar();
  renderRegions();
  renderVisibility();
  if (V.vp.some((x) => x.mode === MODE_INDEX.displacement)) ensureReference();
  if (needUV() && !D.placedUVs) settle();
  request(false);
}

function panelStates() {
  const out = {};
  for (const [id, h] of D.panelHooks) {
    if (!h.getState) continue;
    try { out[id] = JSON.parse(JSON.stringify(h.getState() ?? null)); } catch (e) { console.warn(`panel ${id} getState: ${e.message}`); }
  }
  return out;
}

function applyPanelStates(states) {
  if (!states || typeof states !== "object") return;
  for (const [id, s] of Object.entries(states)) {
    const h = D.panelHooks.get(id);
    if (!h?.setState) continue;
    try { h.setState(s); } catch (e) { toast(`panel ${id} could not restore its state: ${e.message}`); }
  }
}

function characterInput() {
  const c = settleInput();
  for (const k of ["store", "eyes", "uvs", "fast"]) delete c[k];
  return JSON.parse(JSON.stringify(c));
}

function currentOptions() {
  const o = {};
  for (const k of PROJECT_OPTIONS) if (opt(k)) o[k] = opt(k);
  return o;
}

function projectSig() {
  return JSON.stringify({ c: characterInput(), p: panelStates(), n: $("p-notes").value });
}

function scheduleDirty() {
  if (P.dirtyTimer) return;
  P.dirtyTimer = setTimeout(() => { P.dirtyTimer = 0; updateDirty(); }, 0);
}

function updateDirty() {
  if (!D.ready || P.savedSig === null) return;
  P.dirty = P.recovered || P.relinked || projectSig() !== P.savedSig;
  const label = P.name || "untitled";
  $("p-name").textContent = label;
  $("p-name").classList.toggle("dirty", P.dirty);
  $("p-name").title = P.dirty ? "unsaved changes" : "saved";
  document.title = `${P.dirty ? "* " : ""}${label} - GazeForge Designer`;
}

function projectInput(name) {
  const doc = P.doc, a = P.active;
  const chars = doc ? doc.characters.map((c, i) => (i === a ? { id: c.id, designer: characterInput(), history: c.history } : { id: c.id, spec: c.spec, history: c.history }))
    : [{ id: "c1", designer: characterInput() }];
  return { name, ...rootArg(), store, characters: chars, active: a, panels: panelStates(), viewer: viewerState(), notes: $("p-notes").value, options: currentOptions() };
}

function readRecent() { try { const r = JSON.parse(localStorage.getItem(RKEY) || "[]"); return Array.isArray(r) ? r.filter((x) => typeof x === "string") : []; } catch { return []; } }
function addRecent(name) { try { localStorage.setItem(RKEY, JSON.stringify([name, ...readRecent().filter((x) => x !== name)].slice(0, 8))); } catch {} }

async function renderRecent() {
  const sel = $("p-recent");
  let names = readRecent();
  try {
    const l = await api("project.list", rootArg());
    const have = new Set(l.projects.filter((p) => !p.error).map((p) => p.name));
    names = [...names.filter((n) => have.has(n)), ...l.projects.filter((p) => !p.error && !names.includes(p.name)).map((p) => p.name)].slice(0, 8);
  } catch {}
  sel.innerHTML = `<option value="">recent…</option>` + names.map((n) => `<option value="${escapeHtml(n)}">${escapeHtml(n)}</option>`).join("");
}

function renderProjectTab() {
  const doc = P.doc;
  $("p-info").textContent = doc
    ? `${doc.name}: saved ${doc.modified}, created ${doc.created}; store ${doc.store}; base pack ${doc.base.pack.slice(0, 19)}…, ${doc.base.blobs} base blobs`
    : "untitled: not saved yet (Save As names it)";
  $("p-chars").innerHTML = (doc ? doc.characters : [{ id: "c1", name: D.start.name, revision: "", history: [] }])
    .map((c, i) => `<li class="${i === P.active ? "now" : ""}" data-i="${i}">${escapeHtml(c.name)} <span class="muted">${escapeHtml(c.id)} ${escapeHtml((c.revision || "").slice(0, 15))} · ${(c.history || []).length} revisions</span></li>`).join("");
  $("p-export").disabled = !P.name;
}

function leave(url) {
  P.leaving = true;
  location.href = url;
}

function projectUrl(name, extra = {}) {
  const q = new URLSearchParams();
  for (const k of ["root", "fast", "measure"]) if (params.get(k)) q.set(k, params.get(k));
  if (name) q.set("project", name);
  for (const [k, v] of Object.entries(extra)) if (v != null && v !== "") q.set(k, String(v));
  return `${location.pathname}?${q}`;
}

function dialog({ title, note = "", name = null, list = null, ok = "OK" }) {
  return new Promise((resolve) => {
    const box = $("p-dialog"), input = $("p-dlg-name"), ul = $("p-dlg-list");
    $("p-dlg-title").textContent = title;
    $("p-dlg-note").textContent = note;
    $("p-dlg-ok").textContent = ok;
    input.classList.toggle("hidden", name === null);
    input.value = name ?? "";
    ul.innerHTML = "";
    let picked = null;
    for (const it of list || []) {
      const li = document.createElement("li");
      li.textContent = it.label;
      li.dataset.value = it.value;
      li.addEventListener("click", () => { picked = it.value; ul.querySelectorAll("li").forEach((x) => x.classList.toggle("now", x === li)); });
      li.addEventListener("dblclick", () => { picked = it.value; done(true); });
      ul.appendChild(li);
    }
    const done = (yes) => {
      box.classList.add("hidden");
      $("p-dlg-ok").onclick = $("p-dlg-cancel").onclick = input.onkeydown = null;
      P.dialogDone = null;
      resolve(yes ? (name !== null ? input.value.trim() : list ? picked : true) : null);
    };
    P.dialogDone = done;
    $("p-dlg-ok").onclick = () => done(true);
    $("p-dlg-cancel").onclick = () => done(false);
    input.onkeydown = (e) => { if (e.key === "Enter") done(true); if (e.key === "Escape") done(false); };
    box.classList.remove("hidden");
    if (name !== null) input.focus();
  });
}

async function confirmDiscard(what) {
  updateDirty();
  if (!P.dirty) return true;
  return !!(await dialog({ title: "Unsaved changes", note: `${P.name || "untitled"} has unsaved changes; ${what} discards them (its autosave stays recoverable until you save or discard it).`, ok: "discard changes" }));
}

async function saveProject(asName) {
  const name = asName || P.name;
  if (!name) return saveProjectAs();
  if (D.start.macros && !D.macros?.available) { toast("this character has macro sliders: open it with its macro definition (macro) to save it"); return false; }
  const was = P.name;
  P.saving = true;
  try {
    const r = await api("project.save", projectInput(name));
    if (!was && name !== "untitled") { try { await api("project.discard", { name: "untitled", ...rootArg() }); } catch {} }
    P.name = name;
    P.doc = r.project;
    P.recovered = false;
    P.relinked = false;
    P.autosaved = false;
    P.savedSig = projectSig();
    params.set("project", name);
    params.delete("recovery");
    params.delete("spec");
    history.replaceState(null, "", `${location.pathname}?${params}`);
    addRecent(name);
    renderRecent();
    renderProjectTab();
    updateDirty();
    $("p-status").textContent = `saved ${r.path} (${r.bytes} bytes) at ${r.modified}`;
    toast(`saved ${name}`);
    return true;
  } catch (e) {
    $("p-status").textContent = `save failed: ${e.message}`;
    toast(`save failed: ${e.message}`);
    return false;
  } finally { P.saving = false; }
}

async function saveProjectAs() {
  const name = await dialog({ title: "Save project as", note: "1 to 64 letters, digits, spaces, '_', '-' or '.'", name: P.name || D.start.name || "" , ok: "save" });
  if (!name) return false;
  return saveProject(name);
}

async function openProject(name) {
  if (!name) {
    let items = [];
    try { items = (await api("project.list", rootArg())).projects.filter((p) => !p.error).map((p) => ({ value: p.name, label: `${p.name} · ${p.modified} · ${p.characters} character(s)` })); } catch (e) { toast(e.message); return; }
    if (!items.length) { toast("no saved projects yet"); return; }
    name = await dialog({ title: "Open project", list: items, ok: "open" });
    if (!name) return;
  }
  if (!(await confirmDiscard(`opening ${name}`))) return;
  leave(projectUrl(name));
}

async function newProject() {
  if (!(await confirmDiscard("a new project"))) return;
  const extra = { store };
  for (const k of PROJECT_OPTIONS) if (opt(k)) extra[k] = opt(k);
  leave(projectUrl(null, extra));
}

async function autosave(reason = "settle") {
  if (!D.ready || P.saving || P.autosaving) return;
  updateDirty();
  const name = slotName();
  if (!P.dirty) {
    if (P.autosaved) { P.autosaved = false; try { await api("project.discard", { name, ...rootArg() }); } catch {} }
    return;
  }
  const sig = projectSig();
  if (reason === "settle" && sig === P.lastAutoSig) return;
  P.autosaving = true;
  try {
    const r = await api("project.save", { ...projectInput(name), recovery: true });
    P.autosaved = true;
    P.lastAutoSig = sig;
    P.autosaves++;
    P.lastAutosave = r.modified;
    P.unresolved = r.unresolved || [];
    $("p-status").textContent = `autosaved (${reason}) at ${r.modified}` + (P.unresolved.length ? `; not found, kept as typed: ${unresolvedText(P.unresolved)}` : "");
  } catch (e) {
    $("p-status").textContent = `autosave failed: ${e.message}`;
  } finally { P.autosaving = false; }
}

function unresolvedText(list) {
  return list.map((u) => `${u.role} (${u.path})`).join(", ");
}

function relinkDialog(unresolved, message, given) {
  return new Promise((resolve) => {
    const box = $("p-relink"), rows = $("p-relink-rows");
    $("p-relink-note").textContent = `${message}. Give the new location of each, then retry.`;
    rows.innerHTML = "";
    const items = [];
    for (const u of unresolved) {
      const row = document.createElement("div");
      row.className = "relink-row";
      row.dataset.role = u.role;
      const label = document.createElement("label");
      label.textContent = `${u.role} (${u.kind}${u.file ? `, ${u.file}` : ""})`;
      const why = document.createElement("div");
      why.className = "note";
      why.textContent = u.reason || "";
      const input = document.createElement("input");
      input.type = "text";
      input.className = "wide";
      input.placeholder = "path of the file or folder";
      input.value = given[u.role] || "";
      row.append(label, why, input);
      let skip = null;
      if (u.optional) {
        const l = document.createElement("label");
        skip = document.createElement("input");
        skip.type = "checkbox";
        l.append(skip, document.createTextNode(" open without it"));
        row.append(l);
      }
      rows.append(row);
      items.push({ role: u.role, input, skip });
    }
    const done = (yes) => {
      box.classList.add("hidden");
      $("p-relink-ok").onclick = $("p-relink-cancel").onclick = null;
      if (!yes) { resolve(null); return; }
      const resources = {}, skip = [];
      for (const it of items) {
        if (it.skip?.checked) skip.push(it.role);
        else if (it.input.value.trim()) resources[it.role] = it.input.value.trim();
      }
      resolve({ resources, skip });
    };
    $("p-relink-ok").onclick = () => done(true);
    $("p-relink-cancel").onclick = () => done(false);
    box.classList.remove("hidden");
    items[0]?.input.focus();
  });
}

async function offerRecovery() {
  if (P.recovered) return;
  let slot = null;
  try { slot = (await api("project.list", rootArg())).recoveries.find((r) => r.name === slotName() && r.newer_than_saved); } catch {}
  if (!slot) return;
  P.recoveryOffer = slot;
  $("p-recover-text").textContent = `Unsaved work on ${slot.name} from ${slot.modified} was kept by autosave (the last session ended without saving).`;
  $("p-recover").classList.remove("hidden");
}

function buildProjectControls() {
  $("p-new").addEventListener("click", newProject);
  $("p-open").addEventListener("click", () => openProject());
  $("p-save").addEventListener("click", () => saveProject());
  $("p-saveas").addEventListener("click", saveProjectAs);
  $("p-recent").addEventListener("input", (e) => { const n = e.target.value; e.target.value = ""; if (n) openProject(n); });
  $("p-notes").addEventListener("input", scheduleDirty);
  $("p-export").addEventListener("click", async () => {
    try { const r = await api("project.export", { name: P.name, ...rootArg() }); $("p-status").textContent = `portable file ${r.out} (${r.bytes} bytes, ${r.character_blobs} character blobs)`; }
    catch (e) { $("p-status").textContent = `export failed: ${e.message}`; }
  });
  $("p-chars").addEventListener("click", async (e) => {
    const li = e.target.closest("li");
    if (!li || !P.doc || Number(li.dataset.i) === P.active) return;
    if (!(await confirmDiscard("switching the character"))) return;
    leave(projectUrl(P.name, { character: li.dataset.i }));
  });
  $("p-restore").addEventListener("click", () => { $("p-recover").classList.add("hidden"); leave(projectUrl(slotName(), { recovery: 1, ...(P.name ? {} : { store }) })); });
  $("p-discard").addEventListener("click", async () => {
    $("p-recover").classList.add("hidden");
    try { await api("project.discard", { name: slotName(), ...rootArg() }); toast("autosave discarded"); } catch (e) { toast(e.message); }
  });
  window.addEventListener("beforeunload", (e) => {
    updateDirty();
    if (P.dirty && !P.leaving) { e.preventDefault(); e.returnValue = ""; }
  });
  P.timer = setInterval(() => autosave("timer"), 60000);
  renderRecent();
  renderProjectTab();
}

async function loadProjectDoc() {
  const name = params.get("project");
  if (!name) return null;
  const recovery = params.get("recovery") === "1";
  let extra = {}, r;
  for (;;) {
    try {
      r = await api("project.load", { name, recovery, ...rootArg(), ...extra });
      break;
    } catch (e) {
      const un = e.details?.unresolved;
      if (!Array.isArray(un) || !un.length) throw e;
      const answer = await relinkDialog(un, e.message, extra.resources || {});
      if (!answer) throw e;
      extra = { resources: { ...(extra.resources || {}), ...answer.resources }, skip: [...new Set([...(extra.skip || []), ...answer.skip])] };
      P.relinked = true;
    }
  }
  const doc = r.project;
  P.loadUnresolved = Object.entries(r.checked?.resources || {}).filter(([, v]) => v.unresolved).map(([role, v]) => ({ role, path: v.hint, reason: v.unresolved }));
  P.name = name === "untitled" ? null : name;
  P.doc = name === "untitled" ? null : doc;
  P.recovered = recovery;
  P.loadReport = r.checked;
  if (!params.get("store")) { store = doc.store; params.set("store", store); }
  for (const k of PROJECT_OPTIONS) if (!params.get(k) && doc.options?.[k]) params.set(k, doc.options[k]);
  history.replaceState(null, "", `${location.pathname}?${params}`);
  const k = params.get("character") !== null ? Number(params.get("character")) : doc.active ?? 0;
  P.active = Number.isInteger(k) && k >= 0 && k < doc.characters.length ? k : 0;
  $("p-notes").value = doc.notes || "";
  if (P.name) addRecent(P.name);
  return doc;
}

async function main() {
  const doc = await loadProjectDoc();
  if (!store) {
    setStatus("preview", "open this page with ?store=<store path> (optional &spec=, &macro=, &eyes=) or ?project=<name>");
    return;
  }
  const startFrom = doc ? { character: doc.characters[P.active].spec } : { spec: opt("spec") };
  const sl = await api("designer.sliders", { store, ...startFrom, macro: opt("macro") });
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
  if (!gl) throw new Error("WebGL2 is not available in this browser (on some Windows boxes Chrome needs --use-angle=gl)");
  for (let attempt = 0; !V; attempt++) {
    if (gl.isContextLost()) await new Promise((r) => { gl.canvas.addEventListener("webglcontextrestored", r, { once: true }); setTimeout(r, 15000); });
    try { V = new Viewer(gl, $("vps")); } catch (e) { if (!gl.isContextLost() || attempt >= 2) throw e; }
  }
  D.viewer = V;
  D.renderer = V.renderer;
  await loadScene();
  buildRegions();
  applySettings(loadSettings());
  buildMacros();
  buildSliders();
  buildControls();
  interaction();
  $("out").value = opt("out") || "";
  $("save").addEventListener("click", save);
  D.pack = pack;
  await settle();
  const a = views().anchor;
  V.bounds = V.computeBounds(a);
  V.frameBox(V.bounds, V.cams, true);
  if (V.vp.some((s) => s.mode === MODE_INDEX.displacement)) ensureReference();
  request(true);
  await loadPanels();
  if (doc) {
    applyViewerState(doc.viewer);
    applyPanelStates(doc.panels);
  }
  buildProjectControls();
  if (P.loadUnresolved?.length) $("p-status").textContent = `not found, kept as typed: ${unresolvedText(P.loadUnresolved)}`;
  else if (P.relinked) $("p-status").textContent = "resources relinked: save to keep the new locations";
  D.ui = { setMode, setLod, setLayout, namedView, toggleOrtho, setFov, frameHead, frameSelection, applyPreset, toggleOpt, refreshOpts, undo, redo, historyTo, setVisible, isolate, showAll,
           setRegionMask, screenshot, showTab, ensureReference, settle, pushUndo, setPose, api: makeApi("harness"),
           project: { save: saveProject, saveAs: saveProjectAs, open: openProject, newProject, autosave, viewerState, applyViewerState, panelStates, projectSig, updateDirty, dialog } };
  D.ready = true;
  P.savedSig = projectSig();
  updateDirty();
  D.listeners.push((s, info) => { if (info.kind === "settle") autosave("settle"); });
  offerRecovery();
}

main().catch((e) => { setStatus("preview", `error: ${e.message}`); D.error = e.message; console.error(e); });
