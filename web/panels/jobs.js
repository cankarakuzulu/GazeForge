import { tr } from "../i18n.js";

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const when = (ms) => (ms ? new Date(ms).toLocaleTimeString() : "");
const took = (j) => (j.finished_ms && j.started_ms ? `${((j.finished_ms - j.started_ms) / 1000).toFixed(1)} s` : "");

window.GF.registerPanel({
  id: "jobs",
  title: "Jobs",
  order: 90,
  mount(root, api) {
    root.innerHTML = `<div class="jobs-list"><div class="note">${tr("jobs.none")}</div></div>`;
    const list = root.querySelector(".jobs-list");
    let timer = 0;
    const render = (r) => {
      if (!r.jobs.length) { list.innerHTML = `<div class="note">${esc(tr("jobs.none_in", { dir: r.jobs_dir }))}</div>`; return; }
      list.innerHTML = r.jobs.map((j) => {
        const p = j.progress, frac = p && p.total ? Math.round((100 * p.done) / p.total) : 0;
        const live = j.state === "running" || j.state === "queued";
        return `<div class="job st-${esc(j.state)}" title="${esc(j.id)}">
          <div class="job-head"><b title="${esc(j.command)}">${esc(tr(`job.cmd.${j.command}`, null, j.command))}</b><span class="job-state">${esc(tr(`job.state.${j.state}`, null, j.state))}${j.ok === false ? ` ${tr("jobs.failed")}` : ""}</span>
            ${live ? `<button class="small job-cancel" data-id="${esc(j.id)}">${tr("jobs.cancel")}</button>` : ""}</div>
          <div class="note">${when(j.submitted_ms)} ${took(j)}${j.error ? ` · ${esc(j.error)}` : ""}</div>
          ${live && p ? `<div class="wz-progress"><div class="bar"><i style="width:${frac}%"></i></div><span class="msg">${esc(p.stage)} ${p.done}/${p.total}</span></div>` : ""}
        </div>`;
      }).join("");
      for (const b of list.querySelectorAll(".job-cancel")) {
        b.addEventListener("click", async () => { b.disabled = true; try { await api.call("jobs.cancel", { id: b.dataset.id }); } catch (e) { b.textContent = e.message; } refresh(); });
      }
    };
    const refresh = async () => {
      clearTimeout(timer);
      try {
        const r = await api.call("jobs.list", { limit: 30 });
        render(r);
        timer = setTimeout(refresh, r.jobs.some((j) => j.state === "running" || j.state === "queued") ? 1000 : 5000);
      } catch (e) {
        list.innerHTML = `<div class="note">${esc(e.message)}</div>`;
        timer = setTimeout(refresh, 10000);
      }
    };
    refresh();
  },
});
