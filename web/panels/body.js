import { tr } from '../i18n.js';

function el(tag, attrs, children) {
  const e = document.createElement(tag);
  Object.entries(attrs || {}).forEach(([k, v]) => {
    if (k === 'text') e.textContent = v;
    else e.setAttribute(k, v);
  });
  (children || []).forEach((c) => e.appendChild(c));
  return e;
}

function weight(body, id) {
  return (id && body.weights && Number(body.weights[id])) || 0;
}

function pairValue(body, p) {
  return weight(body, p.incr) - weight(body, p.decr);
}

function put(map, key, v) {
  if (v === 0) delete map[key];
  else map[key] = v;
}

function tidy(body) {
  for (const f of ['weights', 'compounds']) if (body[f] && !Object.keys(body[f]).length) delete body[f];
  return body;
}

function withPair(body, p, v) {
  const next = JSON.parse(JSON.stringify(body));
  next.weights = next.weights || {};
  if (p.incr) put(next.weights, p.incr, Math.max(v, 0));
  if (p.decr) put(next.weights, p.decr, Math.max(-v, 0));
  return tidy(next);
}

function withCompound(body, name, v) {
  const next = JSON.parse(JSON.stringify(body));
  next.compounds = next.compounds || {};
  put(next.compounds, name, v);
  return tidy(next);
}

function mount(root, api) {
  let key = '';
  let pairs = [];
  let compounds = [];
  const rows = new Map();
  const note = el('div', { class: 'gf-body-note' });
  const filter = el('input', { type: 'text', class: 'wide', placeholder: tr('body.filter') });
  const compoundBox = el('div', { class: 'gf-body-compounds' });
  const familyBox = el('div', { class: 'gf-body-families' });
  const reset = el('button', { type: 'button', text: tr('body.reset') });
  const content = el('div', {}, [
    el('div', { class: 'gf-body-head', text: tr('body.compounds') }), compoundBox,
    el('div', { class: 'gf-body-head', text: tr('body.regional') }), filter, familyBox, reset,
  ]);
  root.appendChild(el('div', { class: 'gf-body' }, [note, content]));

  function apply(next, label) {
    const prev = api.body.get();
    if (JSON.stringify(prev) === JSON.stringify(next)) return;
    api.body.set(next);
    api.pushUndo({ label, undo: () => { api.body.set(prev); sync(); }, redo: () => { api.body.set(next); sync(); } });
    sync();
  }

  function slider(rowKey, label, lo, commit) {
    const input = el('input', { type: 'range', min: lo, max: 1, step: 0.01, value: 0 });
    const out = el('span', { text: '0' });
    input.addEventListener('input', () => { out.textContent = Number(input.value).toFixed(2); });
    input.addEventListener('change', () => commit(Number(input.value)));
    const row = el('label', { class: 'gf-stitch-row' }, [el('span', { text: label }), input, out]);
    rows.set(rowKey, { input, out, row, label });
    return row;
  }

  function build() {
    rows.clear();
    compoundBox.replaceChildren(...compounds.map((name) => {
      const label = tr(`body.compound.${name}`);
      return slider(`c:${name}`, label, -1, (v) => {
        const b = api.body.get();
        if (b) apply(withCompound(b, name, v), `${label} ${v.toFixed(2)}`);
      });
    }));
    const families = [...new Set(pairs.map((p) => p.family))];
    familyBox.replaceChildren(...families.map((family) => {
      const details = el('details', { 'data-family': family }, [el('summary', { text: tr(`body.family.${family}`) })]);
      for (const p of pairs.filter((q) => q.family === family)) {
        const label = tr(`body.slider.${p.id}`);
        details.appendChild(slider(`p:${p.id}`, label, p.decr ? -1 : 0, (v) => {
          const b = api.body.get();
          if (b) apply(withPair(b, p, v), `${label} ${v.toFixed(2)}`);
        }));
      }
      return details;
    }));
    applyFilter();
  }

  function applyFilter() {
    const needle = filter.value.trim().toLowerCase();
    for (const [k, r] of rows) {
      if (!k.startsWith('p:')) continue;
      r.row.style.display = !needle || r.label.toLowerCase().includes(needle) || k.slice(2).includes(needle) ? '' : 'none';
    }
    for (const d of familyBox.querySelectorAll('details')) {
      const shown = [...d.querySelectorAll('label')].some((l) => l.style.display !== 'none');
      d.style.display = shown ? '' : 'none';
      if (needle) d.open = shown;
    }
  }

  function sync() {
    const body = api.body.get();
    if (!body) return;
    for (const name of compounds) {
      const r = rows.get(`c:${name}`);
      const v = Number(body.compounds && body.compounds[name]) || 0;
      r.input.value = v;
      r.out.textContent = v.toFixed(2);
    }
    for (const p of pairs) {
      const r = rows.get(`p:${p.id}`);
      const v = pairValue(body, p);
      r.input.value = v;
      r.out.textContent = v.toFixed(2);
    }
    for (const d of familyBox.querySelectorAll('details')) {
      const fam = pairs.filter((p) => p.family === d.dataset.family);
      const set = fam.filter((p) => pairValue(body, p) !== 0).length;
      d.querySelector('summary').textContent = tr('body.family_line', { family: tr(`body.family.${d.dataset.family}`), set, count: fam.length });
    }
    reset.disabled = !body.weights && !body.compounds;
  }

  async function refresh() {
    const character = api.character();
    const body = character.body;
    if (!body || !body.targets) {
      key = '';
      note.textContent = tr('body.no_body');
      content.style.display = 'none';
      return;
    }
    const k = `${character.store}|${body.targets}|${character.catalog}`;
    if (k !== key) {
      key = k;
      try {
        const r = await api.call('body.sliders', { store: character.store, targets: body.targets, catalog: character.catalog });
        pairs = r.pairs || [];
        compounds = (r.compounds || []).filter((c) => !(c.missing && c.missing.length)).map((c) => c.name);
        note.textContent = '';
        build();
      } catch (e) {
        pairs = [];
        compounds = [];
        build();
        note.textContent = tr('body.sliders_failed', { error: e && e.message ? e.message : e });
        content.style.display = 'none';
        return;
      }
    }
    content.style.display = '';
    sync();
  }

  filter.addEventListener('input', applyFilter);
  reset.addEventListener('click', () => {
    const b = api.body.get();
    if (!b) return;
    const next = JSON.parse(JSON.stringify(b));
    delete next.weights;
    delete next.compounds;
    apply(next, tr('body.reset'));
  });
  api.onCharacterChange((c, info) => { if (!info || info.kind !== 'slider') refresh(); });
  refresh();
  return {};
}

window.GF.registerPanel({ id: 'body', title: 'Body shape', order: 45, mount });
