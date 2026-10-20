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
const GROUPS = [["brow", /^brow/], ["eye", /^eye/], ["cheek_nose", /^(cheek|nose)/], ["jaw", /^jaw/], ["mouth", /^mouth/]];
const PRESETS = {
  neutral: {},
  smile: { mouthSmileLeft: 1, mouthSmileRight: 1, cheekSquintLeft: 0.5, cheekSquintRight: 0.5, eyeSquintLeft: 0.3, eyeSquintRight: 0.3 },
  blink: { eyeBlinkLeft: 1, eyeBlinkRight: 1 },
  surprise: { jawOpen: 0.4, browInnerUp: 1, browOuterUpLeft: 1, browOuterUpRight: 1, eyeWideLeft: 1, eyeWideRight: 1 },
  frown: { mouthFrownLeft: 1, mouthFrownRight: 1, browDownLeft: 0.8, browDownRight: 0.8, mouthShrugLower: 0.4 },
  jaw_open: { jawOpen: 1 },
};
const MANUAL = [["rx", "deg", 30], ["ry", "deg", 30], ["rz", "deg", 30], ["tx", "mm", 10], ["ty", "mm", 10], ["tz", "mm", 10]];

function decode(b64, Type) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new Type(bytes.buffer);
}

function rotation(r) {
  const t = Math.hypot(r[0], r[1], r[2]);
  if (t < 1e-12) return [1, -r[2], r[1], r[2], 1, -r[0], -r[1], r[0], 1];
  const k = [r[0] / t, r[1] / t, r[2] / t], s = Math.sin(t), c = Math.cos(t), v = 1 - c;
  return [
    c + k[0] * k[0] * v, k[0] * k[1] * v - k[2] * s, k[0] * k[2] * v + k[1] * s,
    k[1] * k[0] * v + k[2] * s, c + k[1] * k[1] * v, k[1] * k[2] * v - k[0] * s,
    k[2] * k[0] * v - k[1] * s, k[2] * k[1] * v + k[0] * s, c + k[2] * k[2] * v,
  ];
}

function el(tag, attrs = {}, text = "") {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  if (text) e.textContent = text;
  return e;
}

function mount(root, api) {
  const params = new URLSearchParams(location.search);
  const S = {
    rig: null, weights: Object.fromEntries(SHAPES.map((n) => [n, 0])), mode: "blend", overlay: true, joint: 0,
    manual: {}, busy: false, stale: true, timer: 0,
  };
  const status = el("div", { class: "note" }, tr("rig.not_loaded"));
  const bar = el("div", { class: "row" });
  const modeSel = el("select", { title: tr("rig.mode_title") });
  for (const v of ["blend", "joints"]) modeSel.appendChild(el("option", { value: v }, tr(`rig.mode.${v}`)));
  const overlayBox = el("input", { type: "checkbox", title: tr("rig.overlay_title") });
  overlayBox.checked = true;
  const loadBtn = el("button", {}, tr("rig.load"));
  bar.append(loadBtn, el("label", {}, ` ${tr("rig.mode")} `), modeSel, el("label", {}, ` ${tr("rig.joints")} `), overlayBox);
  const presets = el("div", { class: "row" });
  for (const name of Object.keys(PRESETS)) {
    const b = el("button", {}, tr(`rig.preset.${name}`));
    b.addEventListener("click", () => applyPreset(name));
    presets.appendChild(b);
  }
  const sliders = el("div");
  const inputs = {};
  for (const [title, re] of GROUPS) {
    const d = el("details");
    d.appendChild(el("summary", {}, tr(`rig.group.${title}`)));
    for (const n of SHAPES.filter((s) => re.test(s))) {
      const row = el("div", { class: "row" });
      const lab = el("label", { style: "display:inline-block;min-width:11em" }, n);
      const r = el("input", { type: "range", min: "0", max: "1", step: "0.01", value: "0" });
      let from = null;
      r.addEventListener("input", () => { if (from === null) from = S.weights[n]; S.weights[n] = Number(r.value); update(); });
      r.addEventListener("change", () => {
        const to = Number(r.value), was = from ?? S.weights[n];
        from = null;
        S.weights[n] = to;
        if (to !== was) api.pushUndo({ label: `${n} ${was.toFixed(2)} -> ${to.toFixed(2)}`, undo: () => setWeights({ [n]: was }), redo: () => setWeights({ [n]: to }) });
      });
      inputs[n] = { range: r, label: lab };
      row.append(lab, r);
      d.appendChild(row);
    }
    sliders.appendChild(d);
  }
  const jointBox = el("details");
  jointBox.appendChild(el("summary", {}, tr("rig.joint_pose")));
  const jointSel = el("select");
  jointBox.appendChild(jointSel);
  const manualInputs = {};
  for (const [k, unit, lim] of MANUAL) {
    const row = el("div", { class: "row" });
    const r = el("input", { type: "range", min: String(-lim), max: String(lim), step: unit === "deg" ? "0.5" : "0.1", value: "0" });
    let from = null;
    r.addEventListener("input", () => { if (from === null) from = manualValue(S.joint, k); setManual(S.joint, k, Number(r.value)); update(); });
    r.addEventListener("change", () => {
      const j = S.joint, to = Number(r.value), was = from ?? manualValue(j, k);
      from = null;
      setManual(j, k, to);
      if (to !== was) api.pushUndo({ label: `${jointName(j)} ${tr(`rig.manual.${k}`)} ${was} -> ${to}`, undo: () => { setManual(j, k, was); syncInputs(); update(); }, redo: () => { setManual(j, k, to); syncInputs(); update(); } });
    });
    manualInputs[k] = r;
    row.append(el("label", { style: "display:inline-block;min-width:4em" }, tr(`rig.manual.${k}`)), r);
    jointBox.appendChild(row);
  }
  const clearJoint = el("button", {}, tr("rig.reset_joint"));
  jointBox.appendChild(clearJoint);
  root.append(status, bar, presets, sliders, jointBox);

  const jointName = (j) => S.rig?.joints[j]?.name ?? String(j);
  const manualValue = (j, k) => S.manual[j]?.[k] ?? 0;
  function setManual(j, k, v) {
    S.manual[j] = { ...(S.manual[j] || {}), [k]: v };
  }
  function syncInputs() {
    for (const n of SHAPES) inputs[n].range.value = String(S.weights[n]);
    for (const [k] of MANUAL) manualInputs[k].value = String(manualValue(S.joint, k));
    modeSel.value = S.mode;
    overlayBox.checked = S.overlay;
  }
  function setWeights(w) {
    Object.assign(S.weights, w);
    syncInputs();
    update();
  }
  function applyPreset(name) {
    const before = { ...S.weights };
    const after = Object.fromEntries(SHAPES.map((n) => [n, PRESETS[name][n] ?? 0]));
    api.pushUndo({ label: tr("rig.undo_preset", { name: tr(`rig.preset.${name}`) }), undo: () => setWeights(before), redo: () => setWeights(after) });
    setWeights(after);
  }
  modeSel.addEventListener("change", () => {
    const was = S.mode, to = modeSel.value;
    api.pushUndo({ label: tr("rig.undo_mode", { from: tr(`rig.mode.${was}`), to: tr(`rig.mode.${to}`) }), undo: () => { S.mode = was; syncInputs(); update(); }, redo: () => { S.mode = to; syncInputs(); update(); } });
    S.mode = to;
    update();
  });
  overlayBox.addEventListener("change", () => {
    const to = overlayBox.checked;
    api.pushUndo({ label: to ? tr("rig.undo_show_overlay") : tr("rig.undo_hide_overlay"), undo: () => { S.overlay = !to; syncInputs(); update(); }, redo: () => { S.overlay = to; syncInputs(); update(); } });
    S.overlay = to;
    update();
  });
  jointSel.addEventListener("change", () => { S.joint = Number(jointSel.value); syncInputs(); });
  clearJoint.addEventListener("click", () => {
    const j = S.joint, before = { ...(S.manual[j] || {}) };
    api.pushUndo({ label: tr("rig.undo_reset_joint", { joint: jointName(j) }), undo: () => { S.manual[j] = before; syncInputs(); update(); }, redo: () => { delete S.manual[j]; syncInputs(); update(); } });
    delete S.manual[j];
    syncInputs();
    update();
  });
  loadBtn.addEventListener("click", () => load());

  async function load() {
    if (S.busy) { S.stale = true; return; }
    S.busy = true;
    S.wanted = true;
    S.stale = false;
    status.textContent = tr("rig.fitting");
    const input = { ...api.character(), morphs: SHAPES.map((n) => (params.get("face_prefix") ?? "faceunits/") + n) };
    if (params.get("face_morphs")) input.morph_dir = params.get("face_morphs");
    if (params.get("eyes")) input.eyes = params.get("eyes");
    try {
      const r = await api.call("rig.face", input);
      const n = r.vertices;
      S.rig = {
        n, joints: r.joints, poses: r.poses, fit: r.fit,
        rest: decode(r.rest_f32_b64, Float32Array),
        ij: decode(r.influence_joints_u8_b64, Uint8Array), iw: decode(r.influence_weights_f32_b64, Float32Array),
        offsets: r.blend.offsets, index: decode(r.blend.index_u32_b64, Uint32Array), delta: decode(r.blend.delta_f32_b64, Float32Array),
      };
      jointSel.innerHTML = "";
      r.joints.forEach((j, k) => { if (j.posed) jointSel.appendChild(el("option", { value: String(k) }, j.name)); });
      if (![...jointSel.options].some((o) => Number(o.value) === S.joint)) S.joint = Number(jointSel.value);
      jointSel.value = String(S.joint);
      syncInputs();
      r.fit.forEach((f, k) => {
        const i = inputs[SHAPES[k]];
        if (!i) return;
        i.label.title = tr("rig.fit_title", { p95: f.p95_mm.toFixed(2), max: f.max_mm.toFixed(2), pct: (100 * f.explained).toFixed(1) });
        i.label.textContent = SHAPES[k] + (f.poorly_expressed ? " *" : "");
      });
      const poor = r.fit.filter((f) => f.poorly_expressed).length;
      status.textContent = tr("rig.loaded", { joints: r.joints.length, shapes: SHAPES.length, poor, ms: Math.round(r.timing_ms.total) });
    } catch (e) {
      status.textContent = tr("rig.error", { error: e.message ?? e });
    }
    S.busy = false;
    update();
    if (S.stale) load();
  }

  function jointPoses() {
    const R = S.rig, J = R.joints.length, out = [];
    for (let j = 0; j < J; j++) out.push([0, 0, 0, 0, 0, 0]);
    SHAPES.forEach((name, s) => {
      const w = S.weights[name];
      if (!w) return;
      for (let j = 0; j < J; j++) for (let c = 0; c < 6; c++) out[j][c] += w * R.poses[s][j][c];
    });
    for (const [j, m] of Object.entries(S.manual)) {
      const d = Math.PI / 180;
      out[j][0] += (m.rx ?? 0) * d; out[j][1] += (m.ry ?? 0) * d; out[j][2] += (m.rz ?? 0) * d;
      out[j][3] += (m.tx ?? 0) / 1000; out[j][4] += (m.ty ?? 0) / 1000; out[j][5] += (m.tz ?? 0) / 1000;
    }
    return out;
  }

  function evaluate() {
    const R = S.rig, n = R.n, p = new Float32Array(R.rest);
    if (S.mode === "blend") {
      SHAPES.forEach((name, s) => {
        const w = S.weights[name];
        if (!w) return;
        for (let e = R.offsets[s]; e < R.offsets[s + 1]; e++) {
          const g = R.index[e] * 3;
          p[g] += w * R.delta[3 * e]; p[g + 1] += w * R.delta[3 * e + 1]; p[g + 2] += w * R.delta[3 * e + 2];
        }
      });
      return p;
    }
    const poses = jointPoses();
    const mats = poses.map((q) => rotation(q));
    for (let v = 0; v < n; v++) {
      const x = R.rest[3 * v], y = R.rest[3 * v + 1], z = R.rest[3 * v + 2];
      let ox = 0, oy = 0, oz = 0;
      for (let k = 0; k < 4; k++) {
        const w = R.iw[4 * v + k];
        if (!w) continue;
        const j = R.ij[4 * v + k], m = mats[j], c = R.joints[j].position, t = poses[j];
        const dx = x - c[0], dy = y - c[1], dz = z - c[2];
        ox += w * (m[0] * dx + m[1] * dy + m[2] * dz + c[0] + t[3]);
        oy += w * (m[3] * dx + m[4] * dy + m[5] * dz + c[1] + t[4]);
        oz += w * (m[6] * dx + m[7] * dy + m[8] * dz + c[2] + t[5]);
      }
      p[3 * v] = ox; p[3 * v + 1] = oy; p[3 * v + 2] = oz;
    }
    return p;
  }

  function overlay() {
    if (!S.rig || !S.overlay) { api.setOverlay("rig-joints", null); return; }
    const poses = S.mode === "joints" ? jointPoses() : null;
    const at = (j) => { const c = S.rig.joints[j].position, t = poses ? poses[j] : [0, 0, 0, 0, 0, 0]; return [c[0] + t[3], c[1] + t[4], c[2] + t[5]]; };
    const points = [], lines = [];
    S.rig.joints.forEach((jt, j) => {
      points.push(...at(j));
      if (jt.parent != null) lines.push(...at(jt.parent), ...at(j));
    });
    api.setOverlay("rig-joints", { points: new Float32Array(points), lines: new Float32Array(lines), color: [1.0, 0.55, 0.15] });
  }

  function update() {
    if (!S.rig) return;
    const active = SHAPES.some((n) => S.weights[n]) || (S.mode === "joints" && Object.keys(S.manual).length);
    api.setPose("rig", active ? evaluate() : null);
    overlay();
    api.requestRender();
  }

  api.onCharacterChange((c, info) => {
    if (!S.rig) return;
    if (info && info.kind === "slider") {
      api.setPose("rig", null);
      api.setOverlay("rig-joints", null);
      status.textContent = tr("rig.changed");
      return;
    }
    clearTimeout(S.timer);
    S.timer = setTimeout(() => load(), 300);
  });
  syncInputs();
  return {
    getState: () => ({ loaded: !!S.rig || S.wanted === true, mode: S.mode, overlay: S.overlay, joint: S.joint,
                       weights: Object.fromEntries(Object.entries(S.weights).filter(([, w]) => w)), manual: JSON.parse(JSON.stringify(S.manual)) }),
    setState: (s) => {
      if (!s || typeof s !== "object") return;
      S.weights = Object.fromEntries(SHAPES.map((n) => [n, Number(s.weights?.[n]) || 0]));
      S.mode = s.mode === "joints" ? "joints" : "blend";
      S.overlay = s.overlay !== false;
      S.joint = Number.isInteger(s.joint) ? s.joint : S.joint;
      S.manual = s.manual && typeof s.manual === "object" ? JSON.parse(JSON.stringify(s.manual)) : {};
      S.wanted = !!s.loaded;
      syncInputs();
      if (s.loaded) load(); else update();
    },
  };
}

window.GF.registerPanel({ id: "rig", title: "Face rig", order: 40, mount });
