import { tr } from "../i18n.js";

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const short = (ref) => String(ref || "").replace(/^sha256:/, "").slice(0, 12);
const day = (iso) => (iso ? iso.slice(0, 16).replace("T", " ") : "");

window.GF.registerPanel({
  id: "store",
  title: "Store",
  order: 85,
  mount(root, api) {
    root.innerHTML = `<div class="store-body"><div class="note">${tr("store.reading")}</div></div><button class="small store-refresh">${tr("store.refresh")}</button>`;
    const body = root.querySelector(".store-body");
    const row = (cells, title) => `<tr title="${esc(title)}">${cells.map((c) => `<td>${c}</td>`).join("")}</tr>`;
    const table = (head, rows) => `<table class="store-table"><thead><tr>${head.map((h) => `<th>${esc(h)}</th>`).join("")}</tr></thead><tbody>${rows.join("")}</tbody></table>`;
    const refresh = async () => {
      const store = api.character().store;
      try {
        const r = await api.call("store.list", { store });
        const cur = api.character();
        const mark = (ref, now) => (ref === now ? " ●" : "");
        body.innerHTML = `<div class="note">${esc(store)}</div>
          <b>${tr("store.packs")}</b>${table([tr("store.col.imported"), tr("store.col.vertices"), tr("store.col.macro_base"), tr("store.col.id")], r.packs.map((p) => row([day(p.imported), p.vertices, p.macro_base ? tr("store.yes") : tr("store.no"), short(p.pack) + mark(p.pack, cur.pack)], p.pack)))}
          <b>${tr("store.catalogs")}</b>${table([tr("store.col.imported"), tr("store.col.targets"), tr("store.col.pack"), tr("store.col.id")], r.catalogs.map((c) => row([day(c.imported), c.targets, short(c.pack), short(c.catalog) + mark(c.catalog, cur.catalog)], c.catalog)))}
          <b>${tr("store.games")}</b>${table([tr("store.col.imported"), tr("store.col.vertices"), tr("store.col.triangles"), tr("store.col.lods"), tr("store.col.id")], r.game_bases.map((g) => row([day(g.imported), g.vertices, g.triangles, g.lods, short(g.game) + mark(g.game, cur.game)], g.game)))}
          <b>${tr("store.eyerigs")}</b>${table([tr("store.col.imported"), tr("store.col.catalog"), tr("store.col.format"), tr("store.col.id")], (r.eyerigs || []).map((e) => row([day(e.imported), short(e.catalog), esc(e.format) + (e.usable ? "" : " " + tr("store.old")), short(e.eyerig)], e.eyerig)))}
          <div class="note">${tr("store.note")}</div>`;
      } catch (e) {
        body.innerHTML = `<div class="note">${esc(e.message)}</div>`;
      }
    };
    root.querySelector(".store-refresh").addEventListener("click", refresh);
    refresh();
  },
});
