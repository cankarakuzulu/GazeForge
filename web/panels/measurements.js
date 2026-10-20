import { tr } from "../i18n.js";

const signed = (x) => `${x >= 0 ? "+" : ""}${x.toFixed(2)}`;

function boxLines(p) {
  const lo = [Infinity, Infinity, Infinity], hi = [-Infinity, -Infinity, -Infinity];
  for (let i = 0; i < p.length; i += 3) for (let k = 0; k < 3; k++) { lo[k] = Math.min(lo[k], p[i + k]); hi[k] = Math.max(hi[k], p[i + k]); }
  const v = (i) => [i & 1 ? hi[0] : lo[0], i & 2 ? hi[1] : lo[1], i & 4 ? hi[2] : lo[2]];
  const lines = [];
  for (let i = 0; i < 8; i++) for (const bit of [1, 2, 4]) if (!(i & bit)) lines.push(...v(i), ...v(i | bit));
  return { lines, size: hi.map((x, k) => (x - lo[k]) * 1000) };
}

window.GF.registerPanel({
  id: "measurements",
  title: "Measurements (mm, settled)",
  order: 10,
  mount(root, api) {
    root.innerHTML = `<table><thead><tr><th>${tr("measure.col.measure")}</th><th>${tr("unit.mm")}</th><th>${tr("measure.col.change")}</th></tr></thead><tbody></tbody></table>
      <label class="line"><input type="checkbox" class="bbox"> ${tr("measure.bbox")}</label>
      <div class="note size"></div><div class="note lods"></div>`;
    const body = root.querySelector("tbody"), toggle = root.querySelector(".bbox"), size = root.querySelector(".size");
    let positions = null;
    const show = (on) => {
      toggle.checked = on;
      if (!on || !positions) { api.setOverlay("bounding-box", null); size.textContent = ""; return; }
      const b = boxLines(positions);
      api.setOverlay("bounding-box", { lines: new Float32Array(b.lines), color: "#e0a030" });
      size.textContent = tr("measure.box", { size: b.size.map((x) => x.toFixed(1)).join(" x ") });
    };
    api.onCharacterChange((state, info) => {
      if (info.kind !== "settle") return;
      body.innerHTML = Object.entries(info.measurements || {})
        .map(([k, x]) => `<tr title="${tr(`measure.${k}.what`, null, x.what || "")}"><td>${tr(`measure.${k}`, null, k)}</td><td>${x.mm.toFixed(2)}</td><td>${signed(x.delta_mm)}</td></tr>`)
        .join("");
      positions = info.positions;
      if (toggle.checked) show(true);
    });
    toggle.addEventListener("change", () => {
      const on = toggle.checked;
      show(on);
      api.pushUndo({ label: on ? tr("measure.undo_show") : tr("measure.undo_hide"), undo: () => show(!on), redo: () => show(on) });
    });
    const c = api.character();
    api.call("designer.scene", { store: c.store, pack: c.pack, catalog: c.catalog, game: c.game })
      .then((r) => { root.querySelector(".lods").textContent = tr("measure.lods", { list: [r.triangles, ...r.lods.map((l) => l.triangles)].join(" / ") }); })
      .catch((e) => { root.querySelector(".lods").textContent = tr("measure.lods_failed", { error: e.message }); });
  },
});
