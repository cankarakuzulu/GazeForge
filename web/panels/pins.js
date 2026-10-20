import { tr } from "../i18n.js";

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const valid = (p) => p && Number.isInteger(p.n) && Number.isInteger(p.tri) && Array.isArray(p.bary) && p.bary.length === 3 && p.bary.every(Number.isFinite);

window.GF.registerPanel({
  id: "pins",
  title: "Pins (face grid addresses)",
  order: 90,
  mount(root, api) {
    const G = api.grid;
    root.innerHTML = `<div class="pins">
      <p class="note">${tr("pins.note")}</p>
      <div class="buttons"><button class="add">${tr("pins.add")}</button><button class="export">${tr("pins.export")}</button></div>
      <div class="list"></div><div class="note status"></div></div>`;
    let pins = [];
    let next = 1;
    const list = root.querySelector(".list"), status = root.querySelector(".status");
    const markers = () => pins.map((p) => ({ pos: G.pointOf(p.tri, p.bary), nrm: G.normalOf(p.tri), text: String(p.n), color: [0.85, 0.11, 0.38] }));
    const describe = (p) => { try { return G.definition() ? G.describe(p.tri, p.bary) : null; } catch { return null; } };
    const render = () => {
      list.innerHTML = pins.map((p, i) => {
        const d = describe(p);
        return `<div class="pinrow" data-i="${i}"><b>${p.n}</b><span title="${esc(d ? d.address : "")}">${d ? `${esc(d.cell)} · ${esc(d.region)}` : "…"}</span>
          <button class="small go">${tr("pins.go")}</button><button class="small del" title="${tr("pins.delete")}">×</button>
          <input type="text" class="pnote" placeholder="${tr("pins.note_placeholder")}" value="${esc(p.note || "")}" style="grid-column: 1 / -1"></div>`;
      }).join("") || `<div class="note">${tr("pins.none")}</div>`;
    };
    const sync = () => {
      G.setMarkers("pins", pins.length ? markers : null);
      render();
    };
    const snapshot = () => JSON.stringify({ pins, next });
    const restore = (s) => { const o = JSON.parse(s); pins = o.pins; next = o.next; sync(); };
    const edit = (label, fn) => {
      const before = snapshot();
      fn();
      const after = snapshot();
      if (before === after) return;
      sync();
      api.pushUndo({ label, undo: () => restore(before), redo: () => restore(after) });
    };
    const add = (probe) => {
      if (!probe?.lod0) { status.textContent = tr("pins.probe_first"); return null; }
      const d = G.describe(probe.lod0.tri, probe.lod0.bary);
      const n = next;
      edit(tr("pins.undo_add", { n, cell: d.cell }), () => {
        pins.push({ n, tri: probe.lod0.tri, bary: probe.lod0.bary.map((x) => +x.toFixed(6)), dense: d.dense, game: d.game, note: "",
                    ...(probe.lod > 0 ? { from_lod: probe.lod, lod0_error_mm: +(probe.lod0.err_mm || 0).toFixed(4) } : {}) });
        next = n + 1;
      });
      status.textContent = tr("pins.added", { n, address: d.address }) + (probe.lod > 0 ? tr("pins.added_lod", { level: probe.lod, mm: (probe.lod0.err_mm || 0).toFixed(3) }) : "");
      return n;
    };
    const exportDoc = async () => {
      await G.ensure();
      const def = G.definition();
      return {
        format: "gazeforge.pins/1",
        character: G.character(),
        chart: { column_deg: def.column_deg, row_mm: def.row_mm, letters: def.letters, nose_tip_dense: def.nose_tip_dense, top_mm: def.top_mm },
        pins: pins.map((p) => {
          const d = G.describe(p.tri, p.bary);
          return { n: p.n, address: d.address, cell: d.cell, region: d.region, game: d.game, dense: d.dense, lod0_triangle: p.tri, bary: p.bary,
                   position_mm: d.point.map((x) => +(x * 1000).toFixed(2)), note: p.note || "", ...(p.from_lod ? { placed_on_lod: p.from_lod, lod0_error_mm: p.lod0_error_mm } : {}) };
        }),
      };
    };
    G.onPin(add);
    root.querySelector(".add").addEventListener("click", () => add(G.lastProbe()));
    root.querySelector(".export").addEventListener("click", async () => {
      const doc = await exportDoc();
      const url = URL.createObjectURL(new Blob([JSON.stringify(doc, null, 1)], { type: "application/json" }));
      const a = document.createElement("a");
      a.href = url;
      a.download = "gazeforge-pins.json";
      a.click();
      setTimeout(() => URL.revokeObjectURL(url), 2000);
      status.textContent = tr("pins.exported", { n: doc.pins.length });
    });
    list.addEventListener("click", (e) => {
      const row = e.target.closest(".pinrow");
      if (!row) return;
      const p = pins[Number(row.dataset.i)];
      if (e.target.classList.contains("go")) {
        const d = describe(p);
        G.goTo(d ? d.address : `game v${p.game}`).catch((err) => { status.textContent = err.message; });
      }
      if (e.target.classList.contains("del")) edit(tr("pins.undo_delete", { n: p.n }), () => { pins = pins.filter((x) => x !== p); });
    });
    list.addEventListener("change", (e) => {
      if (!e.target.classList.contains("pnote")) return;
      const p = pins[Number(e.target.closest(".pinrow").dataset.i)];
      edit(tr("pins.undo_note", { n: p.n }), () => { p.note = e.target.value; });
    });
    root.gfPins = { add, exportDoc, pins: () => JSON.parse(JSON.stringify(pins)) };
    render();
    return {
      getState: () => ({ pins, next }),
      setState: (s) => {
        pins = Array.isArray(s?.pins) ? s.pins.filter(valid) : [];
        next = Number.isInteger(s?.next) ? s.next : pins.reduce((m, p) => Math.max(m, p.n + 1), 1);
        sync();
        if (pins.length) G.ensure().then(render).catch(() => {});
      },
    };
  },
});
