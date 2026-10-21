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
  let genitals = [];
  let physics = [];
  let storeWear = [];
  let wearKey = '';
  const rows = new Map();
  const note = el('div', { class: 'gf-body-note' });
  const filter = el('input', { type: 'text', class: 'wide', placeholder: tr('body.filter') });
  const compoundBox = el('div', { class: 'gf-body-compounds' });
  const familyBox = el('div', { class: 'gf-body-families' });
  const reset = el('button', { type: 'button', text: tr('body.reset') });
  const genitalsPick = el('select', { 'data-body': 'genitals' });
  const genitalsRow = el('label', { class: 'gf-stitch-row' }, [el('span', { text: tr('body.genitals') }), genitalsPick]);
  const wearList = el('div', { class: 'gf-body-wear' });
  const wearAdd = el('select', { 'data-body': 'wear-add' });
  const content = el('div', {}, [
    genitalsRow,
    el('div', { class: 'gf-body-head', text: tr('body.wearables') }), wearList, wearAdd,
    el('div', { class: 'gf-body-head', text: tr('body.compounds') }), compoundBox,
    el('div', { class: 'gf-body-head', text: tr('body.regional') }), filter, familyBox, reset,
  ]);
  root.appendChild(el('div', { class: 'gf-body' }, [note, content]));

  function apply(next, label) {
    const prev = api.body.get();
    if (JSON.stringify(prev) === JSON.stringify(next)) {
      sync();
      return;
    }
    api.body.set(next);
    api.pushUndo({ label, undo: () => { api.body.set(prev); sync(); }, redo: () => { api.body.set(next); sync(); } });
    sync();
  }

  function slider(rowKey, label, lo, hi, commit) {
    const input = el('input', { type: 'range', min: lo, max: hi, step: 0.01, value: 0 });
    const out = el('span', { text: '0' });
    input.addEventListener('input', () => { out.textContent = Number(input.value).toFixed(2); });
    input.addEventListener('change', () => commit(Number(input.value)));
    const row = el('label', { class: 'gf-stitch-row' }, [el('span', { text: label }), input, out]);
    rows.set(rowKey, { input, out, row, label });
    return row;
  }

  function build() {
    rows.clear();
    genitalsPick.replaceChildren(...['none', ...genitals].map((k) => el('option', { value: k, text: tr(`body.genitals.${k}`) })));
    genitalsRow.style.display = genitals.length ? '' : 'none';
    compoundBox.replaceChildren(...compounds.map((name) => {
      const label = tr(`body.compound.${name}`);
      return slider(`c:${name}`, label, -1, 1, (v) => {
        const b = api.body.get();
        if (b) apply(withCompound(b, name, v), `${label} ${v.toFixed(2)}`);
      });
    }));
    const families = [...new Set(pairs.map((p) => p.family))];
    familyBox.replaceChildren(...families.map((family) => {
      const details = el('details', { 'data-family': family }, [el('summary', { text: tr(`body.family.${family}`) })]);
      for (const p of pairs.filter((q) => q.family === family)) {
        const label = tr(`body.slider.${p.id}`);
        details.appendChild(slider(`p:${p.id}`, label, p.decr ? -1 : 0, p.incr ? 1 : 0, (v) => {
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

  function wearName(asset) {
    const w = storeWear.find((x) => x.wearable === asset);
    return `${w ? w.name : '?'} (${asset.replace('sha256:', '').slice(0, 8)})`;
  }

  function withWear(body, edit) {
    const next = JSON.parse(JSON.stringify(body));
    next.wearables = edit(next.wearables || []);
    if (!next.wearables.length) delete next.wearables;
    return next;
  }

  function syncWear(body) {
    const worn = body.wearables || [];
    wearList.replaceChildren(...(worn.length ? worn.map((w) => {
      const hide = el('input', { type: 'checkbox', 'data-wear-hide': w.asset, title: tr('body.wear_hide_tip') });
      hide.checked = w.hide_body !== false;
      hide.addEventListener('change', () => {
        const b = api.body.get();
        if (b) apply(withWear(b, (list) => list.map((x) => (x.asset === w.asset ? { ...x, hide_body: hide.checked } : x))), tr('body.wear_hide_line', { name: wearName(w.asset), state: hide.checked ? '✓' : '✗' }));
      });
      const kind = el('select', { 'data-wear-physics': w.asset, title: tr('body.wear_physics_tip') });
      kind.replaceChildren(...['none', ...physics].map((k) => el('option', { value: k, text: tr(`body.physics.${k}`) })));
      kind.value = w.physics || 'none';
      kind.addEventListener('change', () => {
        const b = api.body.get();
        if (b) {
          apply(withWear(b, (list) => list.map((x) => {
            if (x.asset !== w.asset) return x;
            const next = { ...x };
            if (kind.value === 'none') delete next.physics;
            else next.physics = kind.value;
            return next;
          })), tr('body.wear_physics_line', { name: wearName(w.asset), kind: tr(`body.physics.${kind.value}`) }));
        }
      });
      const z = (storeWear.find((x) => x.wearable === w.asset) || {}).z_depth ?? 50;
      const setLayer = (layer) => {
        const b = api.body.get();
        if (!b) return;
        apply(withWear(b, (list) => list.map((x) => {
          if (x.asset !== w.asset) return x;
          const next = { ...x };
          if (layer === null) delete next.layer;
          else next.layer = layer;
          return next;
        })), layer === null ? tr('body.wear_layer_off_line', { name: wearName(w.asset), z }) : tr('body.wear_layer_line', { name: wearName(w.asset), layer }));
      };
      const own = el('input', { type: 'checkbox', 'data-wear-layer-on': w.asset, title: tr('body.wear_layer_tip') });
      own.checked = typeof w.layer === 'number';
      own.addEventListener('change', () => setLayer(own.checked ? z : null));
      const layer = el('input', { type: 'number', min: '0', max: '1000', step: '0.5', class: 'gf-wear-layer', 'data-wear-layer': w.asset, title: tr('body.wear_layer_value_tip') });
      layer.value = String(own.checked ? w.layer : z);
      layer.disabled = !own.checked;
      layer.addEventListener('change', () => {
        const v = Number(layer.value);
        if (Number.isFinite(v) && v >= 0 && v <= 1000 && v !== w.layer) setLayer(v);
        else layer.value = String(w.layer);
      });
      const openingM = typeof w.opening_m === 'number' ? w.opening_m : 0.06;
      const opening = el('input', { type: 'number', min: '0', max: '20', step: '0.5', class: 'gf-wear-layer', 'data-wear-opening': w.asset, title: tr('body.wear_opening_tip') });
      opening.value = String(Math.round(openingM * 1000) / 10);
      opening.addEventListener('change', () => {
        const cm = Number(opening.value);
        const b = api.body.get();
        if (!b || !Number.isFinite(cm) || cm < 0 || cm > 20) { opening.value = String(Math.round(openingM * 1000) / 10); return; }
        const m = Math.round(cm * 10) / 1000;
        if (m === openingM) return;
        apply(withWear(b, (list) => list.map((x) => {
          if (x.asset !== w.asset) return x;
          const next = { ...x };
          if (m === 0.06) delete next.opening_m;
          else next.opening_m = m;
          return next;
        })), tr('body.wear_opening_line', { name: wearName(w.asset), cm: Math.round(m * 1000) / 10 }));
      });
      const kindDefault = (storeWear.find((x) => x.wearable === w.asset) || {}).transparent ? 0.5 : 1;
      const coarseNow = typeof w.coarse_power === 'number' ? w.coarse_power : kindDefault;
      const coarse = el('input', { type: 'number', min: '0.25', max: '1', step: '0.05', class: 'gf-wear-layer', 'data-wear-coarse': w.asset, title: tr('body.wear_coarse_tip') });
      coarse.value = String(coarseNow);
      coarse.addEventListener('change', () => {
        const p = Math.round(Number(coarse.value) * 100) / 100;
        const b = api.body.get();
        if (!b || !Number.isFinite(p) || p < 0.25 || p > 1) { coarse.value = String(coarseNow); return; }
        if (p === coarseNow) return;
        apply(withWear(b, (list) => list.map((x) => {
          if (x.asset !== w.asset) return x;
          const next = { ...x };
          if (p === kindDefault) delete next.coarse_power;
          else next.coarse_power = p;
          return next;
        })), tr('body.wear_coarse_line', { name: wearName(w.asset), value: p }));
      });
      const remove = el('button', { type: 'button', class: 'small', 'data-wear-remove': w.asset, text: tr('body.wear_remove') });
      remove.addEventListener('click', () => {
        const b = api.body.get();
        if (b) apply(withWear(b, (list) => list.filter((x) => x.asset !== w.asset)), tr('body.wear_remove_line', { name: wearName(w.asset) }));
      });
      return el('div', { class: 'gf-wear-item', 'data-wear': w.asset }, [
        el('div', { class: 'gf-wear-name', text: wearName(w.asset), title: wearName(w.asset) }),
        el('div', { class: 'gf-wear-controls' }, [el('label', {}, [hide, el('span', { text: tr('body.wear_hide') })]), el('label', {}, [el('span', { text: tr('body.wear_physics') }), kind]), el('label', {}, [own, el('span', { text: tr('body.wear_layer') }), layer]), el('label', {}, [el('span', { text: tr('body.wear_opening') }), opening]), el('label', {}, [el('span', { text: tr('body.wear_coarse') }), coarse]), remove]),
      ]);
    }) : [el('div', { class: 'gf-body-note', text: tr('body.wear_none') })]));
    const free = storeWear.filter((x) => !worn.some((w) => w.asset === x.wearable));
    wearAdd.replaceChildren(el('option', { value: '', text: free.length ? tr('body.wear_add') : tr('body.wear_store_empty') }), ...free.map((x) => el('option', { value: x.wearable, text: wearName(x.wearable) })));
    wearAdd.disabled = !free.length;
    wearAdd.value = '';
  }

  function sync() {
    const body = api.body.get();
    if (!body) return;
    syncWear(body);
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
    genitalsPick.value = body.genitals || 'none';
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
        genitals = r.genitals || [];
        physics = r.physics || [];
        if (character.store !== wearKey) {
          wearKey = character.store;
          storeWear = ((await api.call('wearable.list', { store: character.store })).wearables) || [];
        }
        note.textContent = '';
        build();
      } catch (e) {
        pairs = [];
        compounds = [];
        genitals = [];
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
  wearAdd.addEventListener('change', () => {
    const asset = wearAdd.value;
    const b = api.body.get();
    if (!asset || !b) return;
    apply(withWear(b, (list) => (list.some((x) => x.asset === asset) ? list : [...list, { asset, hide_body: true }])), tr('body.wear_add_line', { name: wearName(asset) }));
  });
  genitalsPick.addEventListener('change', () => {
    const b = api.body.get();
    if (!b) return;
    const next = JSON.parse(JSON.stringify(b));
    if (genitalsPick.value === 'none') delete next.genitals;
    else next.genitals = genitalsPick.value;
    apply(next, tr('body.genitals_line', { kind: tr(`body.genitals.${genitalsPick.value}`) }));
  });
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
