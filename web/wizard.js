const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

import { tr } from "./i18n.js";

const humanize = (n) => { const s = n.replace(/_/g, " "); return s.charAt(0).toUpperCase() + s.slice(1); };

export function readGlbMesh(buffer, preferred) {
  const view = new DataView(buffer);
  if (view.getUint32(0, true) !== 0x46546c67) throw new Error("not a glb");
  const jsonLen = view.getUint32(12, true);
  const doc = JSON.parse(new TextDecoder().decode(new Uint8Array(buffer, 20, jsonLen)));
  const binStart = 20 + jsonLen + 8;
  const mesh = doc.meshes.find((m) => m.name === preferred) || doc.meshes[0];
  const prim = mesh.primitives[0];
  const read = (index) => {
    const a = doc.accessors[index], bv = doc.bufferViews[a.bufferView], at = binStart + (bv.byteOffset || 0) + (a.byteOffset || 0);
    const n = a.count * ({ SCALAR: 1, VEC2: 2, VEC3: 3, VEC4: 4 })[a.type];
    if (a.componentType === 5126) return new Float32Array(buffer.slice(at, at + n * 4));
    if (a.componentType === 5125) return new Uint32Array(buffer.slice(at, at + n * 4));
    if (a.componentType === 5123) return Uint32Array.from(new Uint16Array(buffer.slice(at, at + n * 2)));
    throw new Error(`accessor ${index}: component type ${a.componentType}`);
  };
  const g = read(prim.attributes.POSITION), positions = new Float32Array(g.length);
  for (let i = 0; i < g.length; i += 3) { positions[i] = g[i]; positions[i + 1] = -g[i + 2]; positions[i + 2] = g[i + 1]; }
  return { name: mesh.name, positions, indices: read(prim.indices) };
}

export function createWizards({ api, root, toast, prepare, openSpec, showMesh, canShow }) {
  const W = { list: null, def: null, state: null, at: 0, polling: 0, busy: false, prepared: null, dev: false };
  const box = root;

  function close() {
    box.classList.add("hidden");
    peek(false);
    clearTimeout(W.polling);
    W.def = null;
    W.state = null;
  }

  async function open() {
    box.classList.remove("hidden");
    if (W.def && W.state) { render(); return; }
    W.def = null;
    W.list = W.list || (await api("wizards.list", { developer: W.dev })).wizards;
    W.project = (await prepare()).project;
    W.runs = (await api("wizards.runs", { project: W.project, limit: 8 }).catch(() => ({ runs: [] }))).runs;
    renderList();
  }

  function renderList() {
    const groups = [...new Set(W.list.map((w) => w.group))];
    box.querySelector(".wz-body").innerHTML = `<label class="wz-dev" title="${tr("wizard.dev_title")}"><input type="checkbox" ${W.dev ? "checked" : ""}> ${tr("wizard.dev")}</label>` + groups.map((g) => `<div class="wz-group"><b>${esc(tr(`wizard.group.${g}`, null, g))}</b>${W.list.filter((w) => w.group === g).map((w) =>
      `<button class="wz-pick" data-id="${esc(w.id)}"><span>${esc(tr(`wizard.${w.id}.title`, null, w.title))}</span><small>${esc(tr(`wizard.${w.id}.summary`, null, w.summary))}</small></button>`).join("")}</div>`).join("") || `<div class="note">${tr("wizard.none")}</div>`;
    if (W.runs?.length) {
      box.querySelector(".wz-body").innerHTML += `<div class="wz-group"><b>${esc(tr("wizard.recent", { project: W.project }))}</b>${W.runs.map((r) =>
        `<button class="wz-resume" data-run="${esc(r.run)}" data-wizard="${esc(r.wizard)}"><span>${esc(tr(`wizard.${r.wizard}.title`, null, r.title))} <code>${esc(r.run)}</code></span><small>${esc(r.started || "")} · ${r.steps.map((s) => `${esc(s.id)} ${esc(tr(`wizard.state.${s.state}`, null, s.state))}`).join(", ")}</small></button>`).join("")}</div>`;
    }
    box.querySelector(".wz-title").textContent = tr("ui.wz-title");
    box.querySelector(".wz-steps").innerHTML = "";
    for (const sel of [".wz-run", ".wz-next", ".wz-back"]) box.querySelector(sel).disabled = true;
    for (const b of box.querySelectorAll(".wz-pick")) b.addEventListener("click", () => begin(b.dataset.id).catch((e) => fail(e)));
    for (const b of box.querySelectorAll(".wz-resume")) b.addEventListener("click", () => resume(b.dataset.wizard, b.dataset.run).catch((e) => fail(e)));
    box.querySelector(".wz-dev input").addEventListener("change", async (e) => { W.dev = e.target.checked; W.list = (await api("wizards.list", { developer: W.dev })).wizards; renderList(); });
  }

  function fail(e) {
    W.busy = false;
    box.querySelector(".wz-status").textContent = e.plain || e.message || String(e);
    box.querySelector(".wz-status").className = "wz-status bad";
  }

  async function begin(id) {
    W.def = await api("wizards.get", { id });
    const ctx = await prepare();
    W.state = await api("wizards.start", { wizard: id, project: ctx.project, values: W.lastGlb ? { ...ctx.values, glb: W.lastGlb } : ctx.values });
    W.prepared = await ctx.after(W.state.dir);
    if (W.prepared?.character) W.state = await api("wizards.state", { run: W.state.run });
    W.at = 0;
    box.querySelector(".wz-title").textContent = tr(`wizard.${W.def.id}.title`, null, W.def.title);
    render();
  }

  async function resume(wizard, run) {
    W.def = await api("wizards.get", { id: wizard });
    W.state = await api("wizards.state", { run });
    W.prepared = {};
    const open = W.state.steps.findIndex((s) => s.state !== "done");
    W.at = open < 0 ? W.state.steps.length - 1 : open;
    box.querySelector(".wz-title").textContent = tr(`wizard.${W.def.id}.title`, null, W.def.title);
    render();
  }

  function stepDef(i) { return W.def.steps[i]; }
  function stepState(i) { return W.state.steps[i]; }

  function fieldSchema(step, f) {
    const name = typeof f === "string" ? f : f.name;
    return step.input_schema?.properties?.[name] || (typeof f === "string" ? null : f.schema) || { type: "string" };
  }

  function current(i, name) {
    const s = stepState(i);
    if (s.input && name in s.input) return s.input[name];
    if (s.proposed && name in s.proposed) return s.proposed[name];
    return W.prepared?.[stepDef(i).id]?.[name];
  }

  function widget(i, name, label, schema, f) {
    const v = current(i, name);
    const id = `wz-f-${i}-${name}`;
    const full = tr(`wizard.${W.def.id}.${stepDef(i).id}.${name}`, null, label || name);
    const cut = full.indexOf(" (");
    const short = cut > 0 ? full.slice(0, cut) : full, more = cut > 0 ? full.slice(cut + 2).replace(/\)$/, "") : "";
    const tip = esc([more, schema.description || ""].filter(Boolean).join("\n"));
    const lab = `<label for="${id}" title="${tip}">${esc(short)}</label>`;
    if (Array.isArray(schema.enum)) {
      return `<div class="wz-row">${lab}<select id="${id}" data-name="${esc(name)}" data-kind="enum"><option value=""></option>${schema.enum.map((o) =>
        `<option ${String(o) === String(v) ? "selected" : ""}>${esc(o)}</option>`).join("")}</select></div>`;
    }
    if (schema.type === "boolean") return `<div class="wz-row">${lab}<input id="${id}" type="checkbox" data-name="${esc(name)}" data-kind="bool" ${v ? "checked" : ""}></div>`;
    if (schema.type === "number" || schema.type === "integer") {
      return `<div class="wz-row">${lab}<input id="${id}" type="number" data-name="${esc(name)}" data-kind="${schema.type}" step="${schema.type === "integer" ? 1 : "any"}"
        ${schema.minimum !== undefined ? `min="${schema.minimum}"` : ""} ${schema.maximum !== undefined ? `max="${schema.maximum}"` : ""} value="${v ?? ""}"></div>`;
    }
    if (!schema.type || schema.type === "object" || schema.type === "array") {
      return `<div class="wz-row">${lab}<textarea id="${id}" data-name="${esc(name)}" data-kind="json" data-strict="${schema.type ? 1 : 0}" rows="2">${esc(v === undefined ? "" : typeof v === "string" ? v : JSON.stringify(v))}</textarea></div>`;
    }
    const hint = f?.widget ? `${tr(`wizard.widget.${f.widget}`, null, f.widget)}${f.ext ? ` ${f.ext}` : ""}` : "";
    const accept = [...new Set((f?.ext || "").split(/\s+/).filter(Boolean).map((e) => `.${e.split(".").pop()}`))].join(",");
    const pick = f?.widget === "file_open" ? `<button class="small wz-upload" data-for="${id}" data-field="${esc(name)}" data-accept="${esc(accept)}" title="${tr("wizard.browse_title")}">${tr("wizard.browse")}</button>` : "";
    return `<div class="wz-row">${lab}<div class="wz-file-in"><input id="${id}" type="text" data-name="${esc(name)}" data-kind="text" placeholder="${esc(hint)}" value="${esc(v ?? "")}">${pick}</div></div>`;
  }

  function render() {
    const glb = (W.state?.steps || []).map((s) => s?.result?.out).filter((o) => typeof o === "string" && o.endsWith(".glb")).pop();
    if (glb && W.def.id === "export") W.lastGlb = glb;
    const steps = W.def.steps;
    box.querySelector(".wz-steps").innerHTML = steps.map((s, i) =>
      `<button class="wz-step ${i === W.at ? "now" : ""} st-${esc(stepState(i).state)}" data-i="${i}">${i + 1}. ${esc(tr(`wizard.${W.def.id}.${s.id}.title`, null, s.title))}<small>${esc(tr(`wizard.state.${stepState(i).state}`, null, stepState(i).state))}${s.optional ? ` · ${tr("wizard.optional")}` : ""}</small></button>`).join("");
    for (const b of box.querySelectorAll(".wz-step")) b.addEventListener("click", () => { W.at = +b.dataset.i; render(); });
    const i = W.at, s = stepDef(i), st = stepState(i);
    const visible = (s.fields || []).filter((f) => !f.hidden);
    const named = new Set((s.fields || []).map((f) => f.name));
    const common = visible.filter((f) => f.common), advanced = visible.filter((f) => !f.common);
    const rest = W.dev ? Object.keys(s.input_schema?.properties || {}).filter((n) => !named.has(n)) : [];
    const hidden = (s.fields || []).filter((f) => f.hidden).map((f) => `<div class="wz-row wz-bound"><label>${esc(tr(`wizard.field.${f.name}`, null, humanize(f.name)))}</label><code>${esc(JSON.stringify(current(i, f.name) ?? null))}</code></div>`);
    box.querySelector(".wz-body").innerHTML = `
      <p class="wz-help">${esc(tr(`wizard.${W.def.id}.${s.id}.help`, null, s.help || s.summary || ""))}</p>
      ${s.command ? `<p class="wz-cmd">${tr("wizard.runs", { command: `<code>${esc(s.command)}</code>`, kind: esc(s.kind) })}</p>` : ""}
      <div class="wz-form">${common.map((f) => widget(i, f.name, f.label, fieldSchema(s, f), f)).join("")}</div>
      ${advanced.length || rest.length ? `<details class="wz-adv"><summary>${tr("wizard.advanced", { n: advanced.length + rest.length })}</summary>
        ${advanced.map((f) => widget(i, f.name, f.label, fieldSchema(s, f), f)).join("")}
        ${rest.map((n) => widget(i, n, tr(`wizard.field.${n}`, null, humanize(n)), fieldSchema(s, n), null)).join("")}</details>` : ""}
      ${hidden.length ? `<details class="wz-adv"><summary>${tr("wizard.earlier", { n: hidden.length })}</summary>${hidden.join("")}</details>` : ""}
      <div class="wz-result">${result(i)}</div>`;
    const status = box.querySelector(".wz-status");
    status.className = `wz-status st-${st.state}`;
    status.textContent = st.state === "stale" ? tr("wizard.stale_note") : tr(`wizard.state.${st.state}`, null, st.state);
    box.querySelector(".wz-run").textContent = s.command ? (st.state === "done" ? tr("wizard.run_again") : tr("ui.wz-run")) : tr("wizard.use");
    box.querySelector(".wz-run").disabled = W.busy;
    box.querySelector(".wz-cancel").classList.toggle("hidden", st.state !== "running" || !st.job);
    box.querySelector(".wz-next").disabled = i >= steps.length - 1 || !(st.state === "done" || s.optional);
    box.querySelector(".wz-back").disabled = i === 0;
    for (const b of box.querySelectorAll(".wz-upload")) b.addEventListener("click", () => upload(b.dataset.for, b.dataset.field, b.dataset.accept));
    for (const b of box.querySelectorAll(".wz-open-spec")) b.addEventListener("click", () => openSpec?.(b.dataset.store, b.dataset.spec));
    for (const b of box.querySelectorAll(".wz-show-glb")) b.addEventListener("click", () => showGlb(b.dataset.path).catch((e) => fail(e)));
    if (st.state === "running") poll();
  }

  async function showGlb(path) {
    const r = await fetch(`/v1/wizard-file?run=${encodeURIComponent(W.state.run)}&path=${encodeURIComponent(path)}`);
    if (!r.ok) throw new Error(tr("wizard.cannot_read", { path }));
    const m = readGlbMesh(await r.arrayBuffer(), "Head_LOD0");
    for (let i = 0; i < m.positions.length; i += 3) m.positions[i] += 0.3;
    if (!showMesh("export", { positions: m.positions, indices: m.indices, color: [0.62, 0.78, 0.92], label: tr("wizard.exported_label", { name: m.name }) })) throw new Error(tr("wizard.no_viewport"));
    peek(true);
    toast(tr("wizard.exported_toast", { name: m.name, triangles: m.indices.length / 3 }));
  }

  function peek(on) {
    box.classList.toggle("peek", on);
    box.querySelector(".wz-peek").textContent = on ? tr("wizard.back_to_task") : tr("ui.wz-peek");
  }

  function upload(inputId, field, accept) {
    const chooser = document.createElement("input");
    chooser.type = "file";
    if (accept) chooser.accept = accept;
    chooser.addEventListener("change", async () => {
      const file = chooser.files?.[0];
      if (!file) return;
      const name = `${field}_${file.name.replace(/[^A-Za-z0-9._-]/g, "_").replace(/\.+$/, "")}`.slice(-128).replace(/^[._]+/, "") || "upload";
      try {
        const r = await fetch(`/v1/upload?run=${encodeURIComponent(W.state.run)}&name=${encodeURIComponent(name)}`, { method: "POST", body: file });
        const env = await r.json();
        if (!env.ok) throw new Error(env.error?.message || tr("wizard.upload_failed"));
        box.querySelector(`#${CSS.escape(inputId)}`).value = env.result.path;
      } catch (e) { fail(e); }
    });
    chooser.click();
  }

  function result(i) {
    const s = stepDef(i), st = stepState(i);
    if (st.error) return `<div class="wz-err"><b>${esc(st.error.code)}</b> ${esc(st.error.message)}${st.error.hint ? `<br><small>${esc(st.error.hint)}</small>` : ""}
      ${st.error.details && Object.keys(st.error.details).length ? `<details><summary>${tr("wizard.report")}</summary><pre>${esc(JSON.stringify(st.error.details, null, 1))}</pre></details>` : ""}</div>`;
    if (!st.result) return st.state === "running" ? `<div class="wz-progress"><div class="bar busy"><i></i></div><span class="msg">${progressText(null, 0)}</span></div>` : "";
    const shows = (s.shows || []).map((sh) => {
      if (sh.kind === "images") {
        const list = (sh.from ? resolve(sh.from) : null) || [];
        return `<div class="wz-images">${list.filter((x) => x?.[sh.path]).map((x) =>
          `<figure><img src="/v1/wizard-file?run=${encodeURIComponent(W.state.run)}&path=${encodeURIComponent(x[sh.path])}" alt=""><figcaption>${esc(x[sh.label] ?? "")}</figcaption></figure>`).join("")}</div>`;
      }
      if (sh.kind === "spec") {
        const spec = sh.from ? resolve(sh.from) : null, store = sh.store ? resolveRun(sh.store) : null;
        return spec ? `<div class="wz-file">${tr("wizard.wrote", { path: `<code>${esc(spec)}</code>` })} <button class="wz-open-spec" data-spec="${esc(spec)}" data-store="${esc(store ?? "")}">${tr("wizard.open_spec")}</button></div>` : "";
      }
      if (sh.kind === "glb" || sh.kind === "file") {
        const path = sh.from ? resolve(sh.from) : null;
        const look = sh.kind === "glb" && canShow?.() ? ` <button class="wz-show-glb" data-path="${esc(path)}" title="${tr("wizard.show_glb_title")}">${tr("wizard.show_glb")}</button>` : "";
        const name = String(path ?? "").split(/[\\/]/).pop();
        const get = sh.kind === "glb" ? ` <a class="wz-download" download="${esc(name)}" href="/v1/wizard-file?run=${encodeURIComponent(W.state.run)}&path=${encodeURIComponent(path ?? "")}">${tr("wizard.download")}</a>` : "";
        return path ? `<div class="wz-file">${tr("wizard.wrote", { path: `<code>${esc(path)}</code>` })}${look}${get}</div>` : "";
      }
      return `<details><summary>${tr("wizard.report")}</summary><pre>${esc(JSON.stringify(st.result, null, 1))}</pre></details>`;
    });
    return shows.join("") || `<pre>${esc(JSON.stringify(st.result, null, 1))}</pre>`;
  }

  function resolveRun(binding) {
    const key = binding.replace(/^\$run\./, "");
    return W.state.values?.[key] ?? null;
  }

  function resolve(binding) {
    const path = binding.replace(/^\$/, "").split(".");
    const k = W.def.steps.findIndex((s) => s.id === path[0]);
    if (k < 0) return null;
    let at = path[1] === "input" ? stepState(k).input : stepState(k).result;
    for (const p of path.slice(2)) at = at?.[p];
    return at ?? null;
  }

  function collect() {
    const input = {};
    for (const el of box.querySelectorAll(".wz-body [data-name]")) {
      const n = el.dataset.name, kind = el.dataset.kind;
      if (kind === "bool") { if (el.checked) input[n] = true; continue; }
      const v = el.value.trim();
      if (v === "") continue;
      if (kind === "number" || kind === "integer") input[n] = Number(v);
      else if (kind === "json") {
        try { input[n] = JSON.parse(v); } catch {
          if (el.dataset.strict === "1" || /^[[{]/.test(v)) throw new Error(tr("wizard.not_json", { field: n, text: v.slice(0, 40) }));
          input[n] = v;
        }
      }
      else input[n] = v;
    }
    return input;
  }

  async function run() {
    if (W.busy || !W.def || !W.state) return;
    W.busy = true;
    try {
      const input = collect();
      W.state = await api("wizards.step", { run: W.state.run, step: stepDef(W.at).id, input });
      W.busy = false;
      const st = stepState(W.at);
      if (st.state === "done" && !stepDef(W.at).command && W.at < W.def.steps.length - 1) W.at++;
      render();
      if (st.error) toast(`${stepDef(W.at).title}: ${st.error.message}`);
    } catch (e) { fail(e); }
  }

  function progressText(p, s) {
    if (p && p.total > 0) {
      const counted = p.stage ? tr("wizard.progress_of", { stage: p.stage, done: p.done, total: p.total }) : tr("wizard.progress_count", { done: p.done, total: p.total });
      return p.message ? `${counted} · ${p.message}` : counted;
    }
    return p && p.stage ? tr("wizard.stage_for", { stage: p.stage, s }) : tr("wizard.running_for", { s });
  }

  function poll() {
    clearTimeout(W.polling);
    W.polling = setTimeout(async () => {
      if (!W.state) return;
      try {
        const i = W.at, job = stepState(i).job;
        if (job) {
          const j = await api("jobs.status", { id: job });
          const p = j.progress, bar = box.querySelector(".wz-progress i"), msg = box.querySelector(".wz-progress .msg");
          if (!W.since || W.since.job !== job) W.since = { job, t: Date.now() };
          if (bar) {
            const counted = p && p.total > 0;
            bar.parentElement.classList.toggle("busy", !counted);
            bar.style.width = counted ? `${Math.round((100 * p.done) / p.total)}%` : "";
            msg.textContent = progressText(p, Math.round((Date.now() - W.since.t) / 1000));
          }
        }
        const s = await api("wizards.state", { run: W.state.run });
        const changed = JSON.stringify(s.steps.map((x) => x.state)) !== JSON.stringify(W.state.steps.map((x) => x.state));
        W.state = s;
        if (changed) render(); else poll();
      } catch (e) { fail(e); }
    }, 500);
  }

  async function cancel() {
    const job = W.state && stepState(W.at).job;
    if (!job) return;
    try {
      await api("jobs.cancel", { id: job });
      W.state = await api("wizards.state", { run: W.state.run });
      render();
    } catch (e) { fail(e); }
  }

  box.querySelector(".wz-close").addEventListener("click", close);
  box.querySelector(".wz-cancel").addEventListener("click", cancel);
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape" || box.classList.contains("hidden")) return;
    e.preventDefault();
    const at = document.activeElement;
    if (at && box.contains(at) && at.matches("input, select, textarea")) { at.blur(); return; }
    box.classList.add("hidden");
    peek(false);
  });
  box.querySelector(".wz-peek").addEventListener("click", () => peek(!box.classList.contains("peek")));
  box.querySelector(".wz-run").addEventListener("click", run);
  box.querySelector(".wz-next").addEventListener("click", () => { if (W.def) { W.at++; render(); } });
  box.querySelector(".wz-back").addEventListener("click", () => { if (W.def) { W.at--; render(); } });
  box.querySelector(".wz-home").addEventListener("click", () => { clearTimeout(W.polling); W.def = null; W.state = null; open().catch((e) => fail(e)); });
  async function openTask(id) {
    box.classList.remove("hidden");
    await begin(id);
  }

  return { open, openTask, close, state: () => W };
}
