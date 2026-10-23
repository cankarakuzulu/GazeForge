import { tr } from "../i18n.js";

const bytes = (b64) => {
  const s = atob(b64);
  const u = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i);
  return u.buffer;
};
const fmt = (x) => (x == null ? "-" : Number.isInteger(x) ? String(x) : Number(x).toFixed(2));
const short = (r) => (r ? String(r).slice(0, 17) : "");
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const BAD = "#ff1f9e";
const ROWS = [
  ["rest_5", "rest", "below_5"], ["rest_10", "rest", "below_10"], ["rest_p1", "rest", "min_angle_p1"],
  ["range_5", "shape_range", "below_5"], ["range_10", "shape_range", "below_10"], ["range_p1", "shape_range", "min_angle_p1"],
  ["range_turned", "shape_range", "turned"],
];

function minAngle(P, a, b, c) {
  const p = [a, b, c].map((i) => [P[i * 3], P[i * 3 + 1], P[i * 3 + 2]]);
  const e = [0, 1, 2].map((i) => { const q = p[(i + 1) % 3], r = p[(i + 2) % 3]; return (q[0] - r[0]) ** 2 + (q[1] - r[1]) ** 2 + (q[2] - r[2]) ** 2; });
  const k = e[0] <= e[1] && e[0] <= e[2] ? 0 : e[1] <= e[2] ? 1 : 2;
  const o = p[k], q = p[(k + 1) % 3], r = p[(k + 2) % 3];
  const u = [q[0] - o[0], q[1] - o[1], q[2] - o[2]], v = [r[0] - o[0], r[1] - o[1], r[2] - o[2]];
  const x = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
  return (Math.atan2(Math.hypot(x[0], x[1], x[2]), u[0] * v[0] + u[1] * v[1] + u[2] * v[2]) * 180) / Math.PI;
}

function badLines(P, T, below, dx = 0) {
  const lines = [];
  let count = 0;
  for (let t = 0; t < T.length; t += 3) {
    if (minAngle(P, T[t], T[t + 1], T[t + 2]) >= below) continue;
    count++;
    for (let k = 0; k < 3; k++) {
      const a = T[t + k] * 3, b = T[t + ((k + 1) % 3)] * 3;
      lines.push(P[a] + dx, P[a + 1], P[a + 2], P[b] + dx, P[b + 1], P[b + 2]);
    }
  }
  return { lines: new Float32Array(lines), count };
}

window.GF.registerPanel({
  id: "topology",
  title: "Topology",
  order: 80,
  mount(root, api) {
    root.innerHTML = `<div class="topology">
      <label class="line"><input type="checkbox" class="show"> ${tr("topology.show")}</label>
      <label class="line">${tr("topology.below")} <input type="range" class="below" min="2" max="30" step="1" value="10"> <span class="below-v">10</span> ${tr("unit.deg")}</label>
      <div class="note count"></div>
      <div class="note base">${tr("topology.measuring")}</div>
      <label class="line" title="${tr("topology.shapes_title")}">${tr("topology.shapes")} <input type="text" class="shapes" placeholder="${tr("topology.shapes_placeholder")}"></label>
      <div class="line"><button class="ib wide improve" title="${tr("topology.improve_title")}"><svg class="ico"><use href="#i-wand-sparkles"/></svg><span>${tr("topology.improve")}</span></button></div>
      <progress class="prog hidden" max="1" value="0"></progress>
      <div class="note msg"></div>
      <table class="nums hidden"><thead><tr><th>LOD0</th><th>${tr("topology.before")}</th><th>${tr("topology.after")}</th></tr></thead><tbody></tbody></table>
      <div class="note res"></div>
      <label class="line hidden after-row"><input type="checkbox" class="after"> ${tr("topology.after_show")}</label>
      <div class="note after-count"></div>
      <button class="use hidden" title="${tr("topology.use_title")}">${tr("topology.use")}</button></div>`;
    const $ = (s) => root.querySelector(s);
    const c0 = api.character();
    const st = { tris: null, positions: null, below: 10, show: false, job: null, result: null, after: null, afterOn: false };
    const badBtn = document.getElementById("bad-btn"), improveBtn = document.getElementById("improve-btn");

    const draw = () => {
      if (st.tris && st.positions) {
        const b = badLines(st.positions, st.tris, st.below);
        $(".count").textContent = tr("topology.count", { count: b.count, below: st.below });
        api.setOverlay("topology-bad", st.show ? { lines: b.lines, color: BAD } : null);
      }
      if (st.after && st.afterOn) {
        const b = badLines(st.after.positions, st.after.tris, st.below, st.after.dx);
        $(".after-count").textContent = tr("topology.after_count", { count: b.count, below: st.below });
        api.setOverlay("topology-after-bad", st.show ? { lines: b.lines, color: BAD } : null);
      } else {
        $(".after-count").textContent = "";
        api.setOverlay("topology-after-bad", null);
      }
      $(".show").checked = st.show;
      badBtn?.classList.toggle("on", st.show);
    };
    const setShow = (on) => { st.show = on; draw(); };
    const toggleShow = (on) => {
      setShow(on);
      api.pushUndo({ label: on ? tr("topology.undo_show") : tr("topology.undo_hide"), undo: () => setShow(!on), redo: () => setShow(on) });
    };

    api.call("topology.inspect", { store: c0.store, game: c0.game })
      .then((r) => {
        st.tris = new Uint32Array(bytes(r.triangles_u32_b64));
        const t = r.topology;
        $(".base").innerHTML = esc(tr("topology.base", { game: short(r.game), below5: fmt(r.rest.below_5), below10: fmt(r.rest.below_10), p1: fmt(r.rest.min_angle_p1) }))
          + (t ? `<br>${esc(tr("topology.improved_from", { from: short(t.from), flips: fmt(t.flips) }))}` : "");
        draw();
      })
      .catch((e) => { $(".base").textContent = `topology.inspect: ${e.message}`; });
    api.onCharacterChange((state, info) => {
      if (info.kind !== "settle") return;
      st.positions = info.positions;
      if (st.afterOn) showAfter(true).catch((e) => { $(".msg").textContent = e.message; });
      draw();
    });

    const table = (r) => {
      const l = r.levels[0];
      $(".nums tbody").innerHTML = ROWS.map(([label, group, key]) => `<tr><td>${tr(`topology.row.${label}`)}</td><td>${fmt(l[group].before[key])}</td><td>${fmt(l[group].after[key])}</td></tr>`).join("");
      $(".nums").classList.remove("hidden");
      $(".res").textContent = tr("topology.result", { flips: l.flips, shapes: r.shape_set.total, game: short(r.game) })
        + (r.levels.length > 1 ? tr("topology.lods", { list: r.levels.slice(1).map((x) => tr("topology.lod_row", { level: x.level, before: x.rest.before.below_10, after: x.rest.after.below_10 })).join(", ") }) : "");
      $(".after-row").classList.remove("hidden");
      $(".use").classList.remove("hidden");
    };

    const improve = async () => {
      if (st.job) return;
      const btn = $(".improve"), prog = $(".prog"), msg = $(".msg");
      btn.disabled = true;
      improveBtn?.classList.add("on");
      prog.classList.remove("hidden");
      prog.value = 0;
      msg.textContent = tr("topology.submitting");
      try {
        const input = { store: c0.store, game: c0.game }, shapes = $(".shapes").value.trim();
        if (shapes) input.dense = shapes;
        const sub = await api.call("jobs.submit", { command: "topology.improve", input });
        st.job = sub.id;
        for (;;) {
          const r = await api.call("jobs.status", { id: st.job, wait_s: 0.5 });
          const p = r.progress || {};
          prog.value = p.total ? p.done / p.total : 0;
          msg.textContent = `${tr(`job.state.${r.state}`, null, r.state)}${p.stage ? `: ${p.stage} ${p.done}/${p.total}` : ""}${p.message ? ` - ${p.message}` : ""}`;
          if (r.state === "done") { st.result = r.result.result; break; }
          if (r.state === "failed" || r.state === "cancelled") throw new Error(r.result?.error?.message || r.error?.message || tr("topology.job_ended", { state: r.state }));
        }
        msg.textContent = tr("topology.done", { game: short(st.result.game) });
        table(st.result);
      } catch (e) {
        msg.textContent = tr("topology.failed", { error: e.message });
      } finally {
        st.job = null;
        btn.disabled = false;
        improveBtn?.classList.remove("on");
        prog.classList.add("hidden");
      }
    };

    const showAfter = async (on) => {
      st.afterOn = on;
      $(".after").checked = on;
      if (!on || !st.result) { api.setExtraMesh("topology-after", null); draw(); return; }
      const game = st.result.game;
      const [ins, settled] = await Promise.all([
        api.call("topology.inspect", { store: c0.store, game }),
        api.call("designer.settle", { ...api.character(), game }),
      ]);
      const tris = new Uint32Array(bytes(ins.triangles_u32_b64));
      const P = new Float32Array(bytes(settled.positions_f32_b64));
      let lo = Infinity, hi = -Infinity;
      for (let i = 0; i < P.length; i += 3) { lo = Math.min(lo, P[i]); hi = Math.max(hi, P[i]); }
      const dx = (hi - lo) * 1.15;
      const shifted = new Float32Array(P);
      for (let i = 0; i < shifted.length; i += 3) shifted[i] += dx;
      st.after = { tris, positions: P, dx, game };
      api.setExtraMesh("topology-after", { positions: shifted, indices: tris, label: tr("topology.mesh_label", { game: short(game) }) });
      draw();
    };

    $(".show").addEventListener("change", (e) => toggleShow(e.target.checked));
    badBtn?.addEventListener("click", () => toggleShow(!st.show));
    $(".below").addEventListener("input", (e) => { st.below = Number(e.target.value); $(".below-v").textContent = st.below; draw(); });
    $(".improve").addEventListener("click", improve);
    improveBtn?.addEventListener("click", improve);
    $(".after").addEventListener("change", (e) => showAfter(e.target.checked).catch((err) => { $(".msg").textContent = tr("topology.after_failed", { error: err.message }); }));
    $(".use").addEventListener("click", () => {
      if (!st.result) return;
      const q = new URLSearchParams(location.search);
      q.set("game", st.result.game);
      location.assign(`${location.pathname}?${q}`);
    });
    window.gfTopology = { state: st, improve, showAfter, setShow, draw };
    return {
      getState: () => ({ show: st.show, below: st.below }),
      setState: (s) => {
        if (!s || typeof s !== "object") return;
        if (Number.isFinite(s.below)) { st.below = Math.min(30, Math.max(2, s.below)); $(".below").value = st.below; $(".below-v").textContent = st.below; }
        setShow(!!s.show);
      },
    };
  },
});
