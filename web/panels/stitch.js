import { tr } from '../i18n.js';

const KEY = 'gf.stitch.v1';
const CONTROLS = [
  { id: 'blend_mm', min: 0, max: 300, step: 5, value: 60 },
  { id: 'neck_offset_mm', min: -60, max: 60, step: 1, value: 0 },
  { id: 'head_scale', min: 0.8, max: 1.25, step: 0.01, value: 1 },
  { id: 'body_scale', min: 0.8, max: 1.25, step: 0.01, value: 1 },
  { id: 'smooth', min: 0, max: 20, step: 1, value: 0 },
  { id: 'tangent', min: 0, max: 1, step: 0.05, value: 0.5 },
];
const BODY_COLOR = [0.78, 0.74, 0.70];
const FAIL_COLOR = [0.86, 0.22, 0.18];
const WEAR_COLORS = [[0.25, 0.36, 0.62], [0.55, 0.30, 0.22], [0.30, 0.52, 0.34], [0.62, 0.52, 0.24], [0.46, 0.32, 0.56], [0.30, 0.55, 0.56]];

function load() {
  try {
    return JSON.parse(localStorage.getItem(KEY) || '{}') || {};
  } catch (e) {
    return {};
  }
}

function save(state) {
  try {
    localStorage.setItem(KEY, JSON.stringify(state));
  } catch (e) {
    return;
  }
}

function bytes(b64) {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out.buffer;
}

function el(tag, attrs, children) {
  const e = document.createElement(tag);
  Object.entries(attrs || {}).forEach(([k, v]) => {
    if (k === 'text') e.textContent = v;
    else e.setAttribute(k, v);
  });
  (children || []).forEach((c) => e.appendChild(c));
  return e;
}

function clampControl(c, v) {
  const x = Number(v);
  return Number.isFinite(x) ? Math.max(c.min, Math.min(c.max, x)) : c.value;
}

function mount(root, api) {
  const saved = load();
  const state = {
    store: saved.store || '',
    body: saved.body || '',
    show: saved.show !== false,
    seam: saved.seam !== false,
    auto: saved.auto === true,
    values: Object.fromEntries(CONTROLS.map((c) => [c.id, saved.values && saved.values[c.id] !== undefined ? clampControl(c, saved.values[c.id]) : c.value])),
  };
  let last = null;
  let busy = false;
  let pending = false;
  let stale = false;
  let wanted = false;

  const store = el('input', { type: 'text', placeholder: tr('stitch.store_placeholder'), value: state.store });
  const body = el('input', { type: 'text', placeholder: tr('stitch.body_placeholder'), value: state.body });
  const show = el('input', { type: 'checkbox' });
  show.checked = state.show;
  const seam = el('input', { type: 'checkbox' });
  seam.checked = state.seam;
  const auto = el('input', { type: 'checkbox' });
  auto.checked = state.auto;
  const status = el('div', { class: 'gf-stitch-status', text: tr('stitch.not_stitched') });
  const go = el('button', { type: 'button', text: tr('stitch.go') });
  const sliders = {};
  const rows = CONTROLS.map((c) => {
    const input = el('input', { type: 'range', min: c.min, max: c.max, step: c.step, value: state.values[c.id] });
    const out = el('span', { text: String(state.values[c.id]) });
    input.addEventListener('input', () => { out.textContent = input.value; });
    input.addEventListener('change', () => {
      const from = state.values[c.id];
      const to = Number(input.value);
      if (from === to) return;
      setValue(c.id, to);
      api.pushUndo({ label: `${tr(`stitch.control.${c.id}`)} ${from} -> ${to}`, undo: () => setValue(c.id, from), redo: () => setValue(c.id, to) });
    });
    sliders[c.id] = { input, out };
    return el('label', { class: 'gf-stitch-row' }, [el('span', { text: tr(`stitch.control.${c.id}`) }), input, out]);
  });

  function setValue(id, v) {
    state.values[id] = v;
    sliders[id].input.value = v;
    sliders[id].out.textContent = String(v);
    persist();
    stitch();
  }

  function persist() {
    state.store = store.value.trim();
    state.body = body.value.trim();
    state.show = show.checked;
    state.seam = seam.checked;
    state.auto = auto.checked;
    save(state);
  }

  let drawnWear = 0;

  function draw() {
    const visible = last && state.show && !stale;
    if (!visible) api.setExtraMesh('stitch-body', null);
    else api.setExtraMesh('stitch-body', { positions: last.positions, indices: last.indices, normals: last.normals, color: last.pass ? BODY_COLOR : FAIL_COLOR, label: last.pass ? tr('stitch.mesh_ok') : tr('stitch.mesh_failed') });
    const wear = visible ? last.wearables : [];
    wear.forEach((w, i) => api.setExtraMesh(`wear-${i}`, { positions: w.positions, indices: w.indices, normals: w.normals, color: w.color || WEAR_COLORS[i % WEAR_COLORS.length], label: w.name }));
    for (let i = wear.length; i < drawnWear; i++) api.setExtraMesh(`wear-${i}`, null);
    drawnWear = wear.length;
    if (!last || !state.seam || stale) api.setOverlay('stitch-seam', null);
    else api.setOverlay('stitch-seam', { lines: last.lines, color: last.pass ? [1.0, 0.35, 0.2] : [1.0, 0.0, 0.0] });
    api.setPose('stitch-head', visible && last.head && last.headScale !== 1 ? last.head : null);
    api.requestRender();
  }

  function report(r, pass) {
    const g = r.gates || {};
    const n = (g.normals && g.normals.seam_dihedral_deg) || {};
    const lines = [
      `${pass ? tr('stitch.pass') : tr('stitch.fail')} - ${tr('stitch.body_line', { mode: r.body.mode, vertices: r.body.vertices, seams: r.body.seam_points })}`,
      tr('stitch.gap_line', { mm: (g.watertight.max_gap_m * 1000).toFixed(4), open: g.topology.open_seam_edges }),
      tr('stitch.angle_line', { max: (n.max || 0).toFixed(1), mean: (n.mean || 0).toFixed(1) }),
      tr('stitch.band_line', { flips: g.band.flips, folds: g.band.folded_edges, body: g.interpenetration.body_vertices_inside_head, head: g.interpenetration.head_vertices_inside_body }),
    ].concat((g.failed || []).map((f) => `- ${f}`));
    status.textContent = lines.join('\n');
    status.dataset.pass = pass ? 'yes' : 'no';
  }

  function mesh(r, pass) {
    const m = r.mesh;
    return {
      pass,
      positions: new Float32Array(bytes(m.positions)),
      normals: new Float32Array(bytes(m.normals)),
      indices: new Uint32Array(bytes(m.indices)),
      lines: new Float32Array(bytes(m.seam_lines)),
      head: m.head_positions ? new Float32Array(bytes(m.head_positions)) : null,
      headScale: m.head_scale ?? 1,
      wearables: (m.wearables || []).map((w) => ({
        name: w.name,
        positions: new Float32Array(bytes(w.positions)),
        normals: new Float32Array(bytes(w.normals)),
        indices: new Uint32Array(bytes(w.indices)),
        color: Array.isArray(w.color) ? w.color : null,
      })),
      values: { ...state.values },
    };
  }

  async function stitch() {
    persist();
    const character = api.character();
    const storePath = state.store || character.store;
    if ((!state.body && !character.body) || !storePath) {
      status.textContent = tr('stitch.need_body');
      status.dataset.pass = '';
      return;
    }
    if (busy) {
      pending = true;
      return;
    }
    wanted = true;
    busy = true;
    status.textContent = tr('stitch.stitching');
    status.dataset.pass = '';
    try {
      const input = Object.assign({ store: storePath, character, return_mesh: true }, state.values);
      if (state.body) input.body = state.body;
      if (character.macro) input.macro = character.macro;
      let r;
      let pass = true;
      try {
        r = await api.call('character.stitch', input);
      } catch (e) {
        if (e && e.code === 'gate_failed' && e.details && e.details.mesh) {
          r = e.details;
          pass = false;
        } else throw e;
      }
      last = mesh(r, pass);
      stale = false;
      report(r, pass);
      draw();
    } catch (e) {
      last = null;
      draw();
      status.textContent = tr('stitch.failed', { error: e && e.message ? e.message : e });
      status.dataset.pass = 'no';
    } finally {
      busy = false;
      if (pending) {
        pending = false;
        stitch();
      }
    }
  }

  go.addEventListener('click', stitch);
  [store, body].forEach((i) => i.addEventListener('change', persist));
  show.addEventListener('change', () => {
    const to = show.checked;
    persist();
    draw();
    api.pushUndo({ label: to ? tr('stitch.undo_show_body') : tr('stitch.undo_hide_body'), undo: () => { show.checked = !to; persist(); draw(); }, redo: () => { show.checked = to; persist(); draw(); } });
  });
  seam.addEventListener('change', () => {
    const to = seam.checked;
    persist();
    draw();
    api.pushUndo({ label: to ? tr('stitch.undo_show_seam') : tr('stitch.undo_hide_seam'), undo: () => { seam.checked = !to; persist(); draw(); }, redo: () => { seam.checked = to; persist(); draw(); } });
  });
  auto.addEventListener('change', persist);
  api.onCharacterChange((c, info) => {
    if (!last) return;
    if (info && info.kind === 'slider') {
      if (!stale) {
        stale = true;
        draw();
        status.dataset.pass = 'stale';
        status.textContent = state.auto ? tr('stitch.changed_auto') : tr('stitch.changed');
      }
      return;
    }
    if (state.auto && state.show) stitch();
    else if (!stale) {
      stale = true;
      draw();
      status.dataset.pass = 'stale';
      status.textContent = tr('stitch.changed');
    }
  });

  root.appendChild(el('div', { class: 'gf-stitch' }, [
    el('label', { class: 'gf-stitch-row' }, [el('span', { text: tr('stitch.store') }), store]),
    el('label', { class: 'gf-stitch-row' }, [el('span', { text: tr('stitch.body_pack') }), body]),
    ...rows,
    el('label', { class: 'gf-stitch-row' }, [show, el('span', { text: tr('stitch.show_body') })]),
    el('label', { class: 'gf-stitch-row' }, [seam, el('span', { text: tr('stitch.seam_overlay') })]),
    el('label', { class: 'gf-stitch-row' }, [auto, el('span', { text: tr('stitch.auto') })]),
    go,
    status,
  ]));
  status.style.whiteSpace = 'pre-wrap';

  return {
    getState: () => {
      const resources = {};
      if (state.store) resources.store = state.store;
      if (state.body) resources.body = state.body;
      return { resources, show: state.show, seam: state.seam, auto: state.auto, values: { ...state.values }, stitched: wanted };
    },
    setState: (s) => {
      if (!s || typeof s !== 'object') return;
      const r = s.resources && typeof s.resources === 'object' ? s.resources : s;
      store.value = typeof r.store === 'string' ? r.store : '';
      body.value = typeof r.body === 'string' ? r.body : '';
      show.checked = s.show !== false;
      seam.checked = s.seam !== false;
      auto.checked = s.auto === true;
      for (const c of CONTROLS) {
        const v = s.values && s.values[c.id] !== undefined ? clampControl(c, s.values[c.id]) : c.value;
        state.values[c.id] = v;
        sliders[c.id].input.value = v;
        sliders[c.id].out.textContent = String(v);
      }
      persist();
      last = null;
      stale = false;
      wanted = !!s.stitched;
      draw();
      status.textContent = tr('stitch.not_stitched');
      status.dataset.pass = '';
      if (s.stitched && state.body) stitch();
    },
  };
}

window.GF.registerPanel({ id: 'stitch', title: 'Body', order: 50, mount });
