import { tr } from "../i18n.js";

const SHAPES = [
  "browDownLeft", "browDownRight", "browInnerUp", "browOuterUpLeft", "browOuterUpRight", "cheekPuff", "cheekSquintLeft", "cheekSquintRight",
  "eyeBlinkLeft", "eyeBlinkRight", "eyeLookDownLeft", "eyeLookDownRight", "eyeLookInLeft", "eyeLookInRight", "eyeLookOutLeft", "eyeLookOutRight",
  "eyeLookUpLeft", "eyeLookUpRight", "eyeSquintLeft", "eyeSquintRight", "eyeWideLeft", "eyeWideRight", "jawForward", "jawLeft", "jawOpen",
  "jawRight", "mouthClose", "mouthDimpleLeft", "mouthDimpleRight", "mouthFrownLeft", "mouthFrownRight", "mouthFunnel", "mouthLeft",
  "mouthLowerDownLeft", "mouthLowerDownRight", "mouthPressLeft", "mouthPressRight", "mouthPucker", "mouthRight", "mouthRollLower",
  "mouthRollUpper", "mouthShrugLower", "mouthShrugUpper", "mouthSmileLeft", "mouthSmileRight", "mouthStretchLeft", "mouthStretchRight",
  "mouthUpperUpLeft", "mouthUpperUpRight", "noseSneerLeft", "noseSneerRight",
];
const SPEEDS = [0.25, 0.5, 1, 2];
let current = null;

function decode(b64, Type) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new Type(bytes.buffer);
}

function el(tag, attrs = {}, text = "") {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  if (text) e.textContent = text;
  return e;
}

function qmat(q) {
  const [x, y, z, w] = q;
  return [
    1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w),
    2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w),
    2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y),
  ];
}

function toDesigner(q) {
  return [q[0], -q[2], q[1], q[3]];
}

function angles(q) {
  const m = qmat(q);
  const n = [m[2], m[5], m[8]];
  const yaw = Math.atan2(n[0], n[2]);
  const pitch = Math.asin(Math.max(-1, Math.min(1, n[1])));
  const u = [m[1], m[4], m[7]];
  const cy = Math.cos(-yaw), sy = Math.sin(-yaw);
  const u1 = [cy * u[0] + sy * u[2], u[1], -sy * u[0] + cy * u[2]];
  const cp = Math.cos(pitch), sp = Math.sin(pitch);
  const u2 = [u1[0], cp * u1[1] - sp * u1[2], sp * u1[1] + cp * u1[2]];
  const d = 180 / Math.PI;
  return [yaw * d, pitch * d, Math.atan2(u2[0], u2[1]) * d];
}

export function validClip(c) {
  if (!c || c.format !== "gazeforge.clip/1") return tr("timeline.bad_format");
  if (!Array.isArray(c.names) || !Array.isArray(c.weights) || !Array.isArray(c.times) || !c.times.length) return tr("timeline.bad_empty");
  if (c.weights.length !== c.times.length || c.weights.some((r) => r.length !== c.names.length)) return tr("timeline.bad_shape");
  return null;
}

function mount(root, api) {
  const params = new URLSearchParams(location.search);
  const S = { rig: null, clip: null, map: [], t: 0, playing: false, loop: true, speed: 1, applyHead: true, last: 0, raf: 0, busy: false, timer: 0, frames: [] };
  const status = el("div", { class: "note" }, tr("timeline.no_clip"));
  const rigNote = el("div", { class: "note" }, tr("timeline.deltas_none"));
  const fileIn = el("input", { type: "file", accept: ".json,application/json", title: tr("timeline.file_title") });
  const loadRig = el("button", {}, tr("timeline.load_deltas"));
  const row1 = el("div", { class: "row" });
  row1.append(loadRig, fileIn);
  const runRow = el("div", { class: "row" });
  const src = el("input", { type: "text", placeholder: tr("timeline.src_placeholder"), size: "28" });
  const glb = el("input", { type: "text", placeholder: tr("timeline.glb_placeholder"), size: "20" });
  const outp = el("input", { type: "text", placeholder: tr("timeline.out_placeholder"), size: "20" });
  const runV = el("button", { title: tr("timeline.capture_title") }, tr("timeline.capture"));
  const runC = el("button", { title: tr("timeline.csv_title") }, tr("timeline.csv"));
  runRow.append(src, glb, outp, runV, runC);
  const transport = el("div", { class: "row" });
  const play = el("button", {}, tr("timeline.play"));
  const loop = el("input", { type: "checkbox", title: tr("timeline.loop") });
  loop.checked = true;
  const speed = el("select", { title: tr("timeline.speed") });
  for (const s of SPEEDS) speed.appendChild(el("option", { value: String(s) }, `${s}x`));
  speed.value = "1";
  const head = el("input", { type: "checkbox", title: tr("timeline.head_title") });
  head.checked = true;
  transport.append(play, el("label", {}, ` ${tr("timeline.loop")} `), loop, el("label", {}, ` ${tr("timeline.speed")} `), speed, el("label", {}, ` ${tr("timeline.head")} `), head);
  const scrub = el("input", { type: "range", min: "0", max: "0", step: "1", value: "0", style: "width:100%", "aria-label": tr("timeline.scrub") });
  const readout = el("div", { class: "note" }, "");
  root.append(row1, rigNote, runRow, transport, scrub, readout, status);

  function remap() {
    S.map = S.clip ? S.clip.names.map((n) => SHAPES.indexOf(n)) : [];
  }

  function frameAt(t) {
    const c = S.clip, fps = c.fps || 30, n = c.times.length;
    const f = Math.max(0, Math.min(n - 1, t * fps));
    const i = Math.min(n - 1, Math.floor(f)), j = Math.min(n - 1, i + 1), a = f - i;
    return { i, j, a };
  }

  function weightsAt(t) {
    const { i, j, a } = frameAt(t), W = S.clip.weights;
    return W[i].map((w, k) => w + a * (W[j][k] - w));
  }

  function headAt(t) {
    const h = S.clip.head;
    if (!h || !h.rotation_xyzw) return null;
    const { i, j, a } = frameAt(t), q0 = h.rotation_xyzw[i];
    let q1 = h.rotation_xyzw[j];
    if (q0[0] * q1[0] + q0[1] * q1[1] + q0[2] * q1[2] + q0[3] * q1[3] < 0) q1 = q1.map((x) => -x);
    const q = q0.map((x, k) => x + a * (q1[k] - x));
    const n = Math.hypot(...q);
    return q.map((x) => x / n);
  }

  function pivot() {
    const j = S.rig?.joints?.find((x) => x.name === "head");
    return j ? j.position : [0, 0, 0];
  }

  function evaluate(t) {
    const R = S.rig, p = new Float32Array(R.rest), w = weightsAt(t);
    w.forEach((v, k) => {
      const s = S.map[k];
      if (s < 0 || !v) return;
      for (let e = R.offsets[s]; e < R.offsets[s + 1]; e++) {
        const g = R.index[e] * 3;
        p[g] += v * R.delta[3 * e]; p[g + 1] += v * R.delta[3 * e + 1]; p[g + 2] += v * R.delta[3 * e + 2];
      }
    });
    const q = headAt(t);
    if (q && S.applyHead) {
      const m = qmat(toDesigner(q)), c = pivot();
      for (let g = 0; g < p.length; g += 3) {
        const x = p[g] - c[0], y = p[g + 1] - c[1], z = p[g + 2] - c[2];
        p[g] = m[0] * x + m[1] * y + m[2] * z + c[0];
        p[g + 1] = m[3] * x + m[4] * y + m[5] * z + c[1];
        p[g + 2] = m[6] * x + m[7] * y + m[8] * z + c[2];
      }
    }
    return { p, q, w };
  }

  function show() {
    if (!S.clip) return;
    const n = S.clip.times.length, fps = S.clip.fps || 30;
    const { i } = frameAt(S.t);
    scrub.value = String(i);
    let text = tr("timeline.readout", { t: S.t.toFixed(2), length: ((n - 1) / fps).toFixed(2), frame: i + 1, frames: n });
    if (!S.rig) {
      readout.textContent = text + tr("timeline.frag.need_deltas");
      return;
    }
    const t0 = performance.now();
    const { p, q, w } = evaluate(S.t);
    api.setPose("timeline", p);
    api.poseExtraMesh?.("eyes", q && S.applyHead ? qmat(toDesigner(q)) : null, pivot());
    if (q) {
      const a = angles(q), c = pivot(), m = qmat(toDesigner(q));
      text += tr("timeline.frag.head", { yaw: a[0].toFixed(1), pitch: a[1].toFixed(1), roll: a[2].toFixed(1) });
      const tip = [c[0] - 0.15 * m[1], c[1] - 0.15 * m[4], c[2] - 0.15 * m[7]];
      api.setOverlay("timeline-head", { lines: new Float32Array([...c, ...tip]), points: new Float32Array(c), color: [0.2, 0.75, 1.0] });
    }
    const top = w.map((v, k) => [v, S.clip.names[k]]).sort((x, y) => y[0] - x[0]).slice(0, 3).filter((x) => x[0] > 0.05);
    if (top.length) text += ", " + top.map(([v, nm]) => `${nm} ${v.toFixed(2)}`).join(", ");
    readout.textContent = text;
    S.frames.push(performance.now() - t0);
    if (S.frames.length > 600) S.frames.shift();
    api.requestRender();
  }

  function tick(now) {
    if (!S.playing) return;
    const dt = S.last ? (now - S.last) / 1000 : 0;
    S.last = now;
    const dur = (S.clip.times.length - 1) / (S.clip.fps || 30);
    S.t += dt * S.speed;
    if (S.t > dur) {
      if (S.loop && dur > 0) S.t %= dur;
      else { S.t = dur; setPlaying(false); }
    }
    show();
    if (S.playing) S.raf = requestAnimationFrame(tick);
  }

  function setPlaying(on) {
    S.playing = !!on && !!S.clip;
    play.textContent = S.playing ? tr("timeline.pause") : tr("timeline.play");
    cancelAnimationFrame(S.raf);
    if (S.playing) { S.last = 0; S.raf = requestAnimationFrame(tick); }
  }

  function setClip(clip, label) {
    const bad = validClip(clip);
    if (bad) { status.textContent = tr("timeline.error", { error: bad }); return false; }
    const before = S.clip;
    const apply = (c) => {
      setPlaying(false);
      S.clip = c; S.t = 0; remap();
      scrub.max = c ? String(c.times.length - 1) : "0";
      if (!c) { api.setPose("timeline", null); api.setOverlay("timeline-head", null); api.poseExtraMesh?.("eyes", null); api.requestRender(); status.textContent = tr("timeline.no_clip"); return; }
      const unmapped = c.names.filter((n, k) => S.map[k] < 0).length;
      status.textContent = tr("timeline.clip", { name: c.name ?? tr("timeline.clip_default"), label, frames: c.times.length, fps: c.fps, channels: c.names.length }) + (unmapped ? tr("timeline.frag.unmapped", { n: unmapped }) : "") + (c.head ? tr("timeline.frag.head_rotation") : "") + (c.experimental ? tr("timeline.frag.experimental") : "");
      show();
    };
    apply(clip);
    api.pushUndo({ label: tr("timeline.undo_load", { name: clip.name ?? tr("timeline.clip_default") }), undo: () => apply(before), redo: () => apply(clip) });
    return true;
  }

  async function loadDeltas() {
    if (S.busy) return;
    S.busy = true;
    rigNote.textContent = tr("timeline.deltas_loading");
    const input = { ...api.character(), morphs: SHAPES.map((n) => (params.get("face_prefix") ?? "faceunits/") + n), rounds: 0 };
    if (params.get("face_morphs")) input.morph_dir = params.get("face_morphs");
    if (params.get("eyes")) input.eyes = params.get("eyes");
    try {
      const r = await api.call("rig.face", input);
      S.rig = { joints: r.joints, rest: decode(r.rest_f32_b64, Float32Array), offsets: r.blend.offsets, index: decode(r.blend.index_u32_b64, Uint32Array), delta: decode(r.blend.delta_f32_b64, Float32Array) };
      rigNote.textContent = tr("timeline.deltas_loaded", { shapes: SHAPES.length, vertices: r.vertices });
      show();
    } catch (e) {
      rigNote.textContent = tr("timeline.deltas_error", { error: e.message ?? e });
    }
    S.busy = false;
  }

  async function run(kind) {
    const path = src.value.trim(), out = outp.value.trim();
    if (!path || !out) { status.textContent = tr("timeline.need_paths"); return; }
    const input = { out, return_clip: true };
    if (glb.value.trim()) input.glb = glb.value.trim();
    else input.morphs = SHAPES;
    try {
      let result;
      if (kind === "csv") {
        status.textContent = tr("timeline.importing");
        result = await api.call("animation.from_csv", { ...input, csv: path });
      } else {
        if (/\.(png|jpe?g|bmp|webp)$/i.test(path) || !/\.[a-z0-9]{2,4}$/i.test(path)) { input.frames = path; input.fps = 30; } else input.video = path;
        if (params.get("landmarker")) input.landmarker = params.get("landmarker");
        const job = await api.call("jobs.submit", { command: "animation.from_video", input });
        for (;;) {
          const rec = await api.call("jobs.status", { id: job.id, wait_s: 2 });
          status.textContent = tr("timeline.capturing", { state: tr(`job.state.${rec.state}`, null, rec.state) });
          if (rec.state === "done") { result = rec.result?.result; break; }
          if (rec.state === "failed" || rec.state === "cancelled") throw new Error(rec.result?.error?.message ?? rec.error ?? rec.state);
        }
      }
      setClip(result.clip, kind === "csv" ? tr("timeline.label_csv") : tr("timeline.label_capture"));
    } catch (e) {
      status.textContent = tr("timeline.error", { error: e.message ?? e });
    }
  }

  loadRig.addEventListener("click", loadDeltas);
  fileIn.addEventListener("change", async () => {
    const f = fileIn.files?.[0];
    if (!f) return;
    try { setClip(JSON.parse(await f.text()), f.name); } catch (e) { status.textContent = tr("timeline.error", { error: e.message }); }
  });
  runV.addEventListener("click", () => run("video"));
  runC.addEventListener("click", () => run("csv"));
  play.addEventListener("click", () => setPlaying(!S.playing));
  loop.addEventListener("change", () => { S.loop = loop.checked; });
  speed.addEventListener("change", () => { S.speed = Number(speed.value); });
  head.addEventListener("change", () => { S.applyHead = head.checked; show(); });
  scrub.addEventListener("input", () => { if (!S.clip) return; setPlaying(false); S.t = Number(scrub.value) / (S.clip.fps || 30); show(); });
  api.onCharacterChange(() => {
    if (!S.rig) return;
    clearTimeout(S.timer);
    S.timer = setTimeout(loadDeltas, 1200);
  });

  current = {
    getState: () => ({ version: 1, t: S.t, speed: S.speed, loop: S.loop, applyHead: S.applyHead, playing: S.playing, clip: S.clip }),
    setState: (st) => {
      if (!st || typeof st !== "object") return;
      if (st.clip && !validClip(st.clip)) {
        setPlaying(false);
        S.clip = st.clip; remap(); scrub.max = String(st.clip.times.length - 1);
        status.textContent = tr("timeline.restored", { name: st.clip.name ?? tr("timeline.clip_default"), frames: st.clip.times.length });
      }
      if (typeof st.speed === "number") { S.speed = st.speed; speed.value = String(st.speed); }
      if (typeof st.loop === "boolean") { S.loop = st.loop; loop.checked = st.loop; }
      if (typeof st.applyHead === "boolean") { S.applyHead = st.applyHead; head.checked = st.applyHead; }
      if (typeof st.t === "number") S.t = st.t;
      show();
      if (st.playing) setPlaying(true);
    },
    stats: () => ({ evaluate_ms: [...S.frames], playing: S.playing, t: S.t, clip_frames: S.clip?.times.length ?? 0, rig: !!S.rig }),
    setClip: (c) => setClip(c, "api"),
    setPlaying,
  };
}

window.GF.registerPanel({
  id: "timeline", title: "Timeline", order: 50, mount,
  getState: () => current?.getState() ?? null,
  setState: (state) => current?.setState(state),
  harness: () => current,
});
