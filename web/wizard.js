const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

export function createWizards({ api, root, toast, prepare, openSpec }) {
  const W = { list: null, def: null, state: null, at: 0, polling: 0, busy: false, prepared: null, dev: false };
  const box = root;

  function close() {
    box.classList.add("hidden");
    clearTimeout(W.polling);
    W.def = null;
    W.state = null;
  }

  async function open() {
    box.classList.remove("hidden");
    W.def = null;
    W.list = W.list || (await api("wizards.list", { developer: W.dev })).wizards;
    renderList();
  }

  function renderList() {
    const groups = [...new Set(W.list.map((w) => w.group))];
    box.querySelector(".wz-body").innerHTML = `<label class="wz-dev" title="tasks that serve building GazeForge itself: macro tools, raw dense-base gates"><input type="checkbox" ${W.dev ? "checked" : ""}> developer tasks</label>` + groups.map((g) => `<div class="wz-group"><b>${esc(g)}</b>${W.list.filter((w) => w.group === g).map((w) =>
      `<button class="wz-pick" data-id="${esc(w.id)}"><span>${esc(w.title)}</span><small>${esc(w.summary)}</small></button>`).join("")}</div>`).join("") || `<div class="note">no wizards</div>`;
    box.querySelector(".wz-title").textContent = "Tasks";
    box.querySelector(".wz-steps").innerHTML = "";
    for (const b of box.querySelectorAll(".wz-pick")) b.addEventListener("click", () => begin(b.dataset.id).catch((e) => fail(e)));
    box.querySelector(".wz-dev input").addEventListener("change", async (e) => { W.dev = e.target.checked; W.list = (await api("wizards.list", { developer: W.dev })).wizards; renderList(); });
  }

  function fail(e) {
    W.busy = false;
    box.querySelector(".wz-status").textContent = e.message || String(e);
    box.querySelector(".wz-status").className = "wz-status bad";
  }

  async function begin(id) {
    W.def = await api("wizards.get", { id });
    const ctx = await prepare();
    W.state = await api("wizards.start", { wizard: id, project: ctx.project, values: ctx.values });
    W.prepared = await ctx.after(W.state.dir);
    W.at = 0;
    box.querySelector(".wz-title").textContent = W.def.title;
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
    const tip = esc(schema.description || "");
    const lab = `<label for="${id}" title="${tip}">${esc(label || name)}</label>`;
    if (Array.isArray(schema.enum)) {
      return `<div class="wz-row">${lab}<select id="${id}" data-name="${esc(name)}" data-kind="enum"><option value=""></option>${schema.enum.map((o) =>
        `<option ${String(o) === String(v) ? "selected" : ""}>${esc(o)}</option>`).join("")}</select></div>`;
    }
    if (schema.type === "boolean") return `<div class="wz-row">${lab}<input id="${id}" type="checkbox" data-name="${esc(name)}" data-kind="bool" ${v ? "checked" : ""}></div>`;
    if (schema.type === "number" || schema.type === "integer") {
      return `<div class="wz-row">${lab}<input id="${id}" type="number" data-name="${esc(name)}" data-kind="${schema.type}" step="${schema.type === "integer" ? 1 : "any"}"
        ${schema.minimum !== undefined ? `min="${schema.minimum}"` : ""} ${schema.maximum !== undefined ? `max="${schema.maximum}"` : ""} value="${v ?? ""}"></div>`;
    }
    if (!schema.type) {
      return `<div class="wz-row">${lab}<textarea id="${id}" data-name="${esc(name)}" data-kind="json" rows="2">${esc(v === undefined ? "" : typeof v === "string" ? v : JSON.stringify(v))}</textarea></div>`;
    }
    const hint = f?.widget ? `${f.widget.replace("_", " ")}${f.ext ? ` ${f.ext}` : ""}` : "";
    const pick = f?.widget === "file_open" ? `<button class="small wz-upload" data-for="${id}" title="Choose a file on this computer; it is copied into the run">browse…</button>` : "";
    return `<div class="wz-row">${lab}<div class="wz-file-in"><input id="${id}" type="text" data-name="${esc(name)}" data-kind="text" placeholder="${esc(hint)}" value="${esc(v ?? "")}">${pick}</div></div>`;
  }

  function render() {
    const steps = W.def.steps;
    box.querySelector(".wz-steps").innerHTML = steps.map((s, i) =>
      `<button class="wz-step ${i === W.at ? "now" : ""} st-${esc(stepState(i).state)}" data-i="${i}">${i + 1}. ${esc(s.title)}<small>${esc(stepState(i).state)}${s.optional ? " · optional" : ""}</small></button>`).join("");
    for (const b of box.querySelectorAll(".wz-step")) b.addEventListener("click", () => { W.at = +b.dataset.i; render(); });
    const i = W.at, s = stepDef(i), st = stepState(i);
    const visible = (s.fields || []).filter((f) => !f.hidden);
    const named = new Set((s.fields || []).map((f) => f.name));
    const common = visible.filter((f) => f.common), advanced = visible.filter((f) => !f.common);
    const rest = Object.keys(s.input_schema?.properties || {}).filter((n) => !named.has(n));
    const hidden = (s.fields || []).filter((f) => f.hidden).map((f) => `<div class="wz-row wz-bound"><label>${esc(f.name)}</label><code>${esc(JSON.stringify(current(i, f.name) ?? null))}</code></div>`);
    box.querySelector(".wz-body").innerHTML = `
      <p class="wz-help">${esc(s.help || s.summary || "")}</p>
      ${s.command ? `<p class="wz-cmd">runs <code>${esc(s.command)}</code> (${esc(s.kind)})</p>` : ""}
      <div class="wz-form">${common.map((f) => widget(i, f.name, f.label, fieldSchema(s, f), f)).join("")}</div>
      ${advanced.length || rest.length ? `<details class="wz-adv"><summary>advanced (${advanced.length + rest.length})</summary>
        ${advanced.map((f) => widget(i, f.name, f.label, fieldSchema(s, f), f)).join("")}
        ${rest.map((n) => widget(i, n, n, fieldSchema(s, n), null)).join("")}</details>` : ""}
      ${hidden.length ? `<details class="wz-adv"><summary>from earlier steps (${hidden.length})</summary>${hidden.join("")}</details>` : ""}
      <div class="wz-result">${result(i)}</div>`;
    const status = box.querySelector(".wz-status");
    status.className = `wz-status st-${st.state}`;
    status.textContent = st.state === "stale" ? "an earlier step changed: run this step again" : st.state;
    box.querySelector(".wz-run").textContent = s.command ? (st.state === "done" ? "run again" : "run") : "use these";
    box.querySelector(".wz-run").disabled = W.busy;
    box.querySelector(".wz-next").disabled = i >= steps.length - 1 || !(st.state === "done" || s.optional);
    box.querySelector(".wz-back").disabled = i === 0;
    for (const b of box.querySelectorAll(".wz-upload")) b.addEventListener("click", () => upload(b.dataset.for));
    for (const b of box.querySelectorAll(".wz-open-spec")) b.addEventListener("click", () => openSpec?.(b.dataset.store, b.dataset.spec));
    if (st.state === "running") poll();
  }

  function upload(inputId) {
    const chooser = document.createElement("input");
    chooser.type = "file";
    chooser.accept = "image/*";
    chooser.addEventListener("change", async () => {
      const file = chooser.files?.[0];
      if (!file) return;
      const name = file.name.replace(/[^A-Za-z0-9._-]/g, "_").replace(/^\.+/, "") || "upload";
      try {
        const r = await fetch(`/v1/upload?run=${encodeURIComponent(W.state.run)}&name=${encodeURIComponent(name)}`, { method: "POST", body: file });
        const env = await r.json();
        if (!env.ok) throw new Error(env.error?.message || "upload failed");
        box.querySelector(`#${CSS.escape(inputId)}`).value = env.result.path;
      } catch (e) { fail(e); }
    });
    chooser.click();
  }

  function result(i) {
    const s = stepDef(i), st = stepState(i);
    if (st.error) return `<div class="wz-err"><b>${esc(st.error.code)}</b> ${esc(st.error.message)}${st.error.hint ? `<br><small>${esc(st.error.hint)}</small>` : ""}
      ${st.error.details && Object.keys(st.error.details).length ? `<details><summary>report</summary><pre>${esc(JSON.stringify(st.error.details, null, 1))}</pre></details>` : ""}</div>`;
    if (!st.result) return st.state === "running" ? `<div class="wz-progress"><div class="bar"><i style="width:0%"></i></div><span class="msg">submitted</span></div>` : "";
    const shows = (s.shows || []).map((sh) => {
      if (sh.kind === "images") {
        const list = (sh.from ? resolve(sh.from) : null) || [];
        return `<div class="wz-images">${list.filter((x) => x?.[sh.path]).map((x) =>
          `<figure><img src="/v1/wizard-file?run=${encodeURIComponent(W.state.run)}&path=${encodeURIComponent(x[sh.path])}" alt=""><figcaption>${esc(x[sh.label] ?? "")}</figcaption></figure>`).join("")}</div>`;
      }
      if (sh.kind === "spec") {
        const spec = sh.from ? resolve(sh.from) : null, store = sh.store ? resolveRun(sh.store) : null;
        return spec ? `<div class="wz-file">wrote <code>${esc(spec)}</code> <button class="wz-open-spec" data-spec="${esc(spec)}" data-store="${esc(store ?? "")}">open in the designer</button></div>` : "";
      }
      if (sh.kind === "glb" || sh.kind === "file") {
        const path = sh.from ? resolve(sh.from) : null;
        return path ? `<div class="wz-file">wrote <code>${esc(path)}</code></div>` : "";
      }
      return `<details open><summary>report</summary><pre>${esc(JSON.stringify(st.result, null, 1))}</pre></details>`;
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
      else if (kind === "json") { try { input[n] = JSON.parse(v); } catch { input[n] = v; } }
      else input[n] = v;
    }
    return input;
  }

  async function run() {
    if (W.busy) return;
    W.busy = true;
    try {
      W.state = await api("wizards.step", { run: W.state.run, step: stepDef(W.at).id, input: collect() });
      W.busy = false;
      const st = stepState(W.at);
      if (st.state === "done" && !stepDef(W.at).command && W.at < W.def.steps.length - 1) W.at++;
      render();
      if (st.error) toast(`${stepDef(W.at).title}: ${st.error.message}`);
    } catch (e) { fail(e); }
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
          if (p && bar) { bar.style.width = `${p.total ? Math.round((100 * p.done) / p.total) : 0}%`; msg.textContent = `${p.stage} ${p.done}/${p.total} ${p.message || ""}`; }
        }
        const s = await api("wizards.state", { run: W.state.run });
        const changed = JSON.stringify(s.steps.map((x) => x.state)) !== JSON.stringify(W.state.steps.map((x) => x.state));
        W.state = s;
        if (changed) render(); else poll();
      } catch (e) { fail(e); }
    }, 500);
  }

  box.querySelector(".wz-close").addEventListener("click", close);
  box.querySelector(".wz-run").addEventListener("click", run);
  box.querySelector(".wz-next").addEventListener("click", () => { W.at++; render(); });
  box.querySelector(".wz-back").addEventListener("click", () => { W.at--; render(); });
  box.querySelector(".wz-home").addEventListener("click", () => { clearTimeout(W.polling); W.def = null; W.state = null; renderList(); });
  return { open, close, state: () => W };
}
