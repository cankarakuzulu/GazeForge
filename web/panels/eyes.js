const FIELDS = [
  ["out mm", "offset_mm", 0, -15, 15, 0.1],
  ["up mm", "offset_mm", 1, -15, 15, 0.1],
  ["forward mm", "offset_mm", 2, -15, 15, 0.1],
  ["pitch up deg", "rotation_deg", 0, -45, 45, 0.5],
  ["yaw out deg", "rotation_deg", 1, -45, 45, 0.5],
  ["roll deg", "rotation_deg", 2, -45, 45, 0.5],
  ["scale", "scale", -1, 0.5, 1.5, 0.005],
];
const FOLLOW = "makehuman-helper/1";
const blank = () => ({ offset_mm: [0, 0, 0], rotation_deg: [0, 0, 0], scale: 1 });
const full = (a) => ({ ...blank(), ...(a || {}), offset_mm: [...(a?.offset_mm || [0, 0, 0])], rotation_deg: [...(a?.rotation_deg || [0, 0, 0])] });
const read = (a, f) => (f[2] < 0 ? a[f[1]] : a[f[1]][f[2]]);
const write = (a, f, x) => { if (f[2] < 0) a[f[1]] = x; else a[f[1]][f[2]] = x; };

window.GF.registerPanel({
  id: "eyes",
  title: "Eyes",
  order: 20,
  mount(root, api) {
    const E = api.eyes;
    let linked = true;
    const spec = () => {
      const s = E.get() || { follow: "none", left: blank(), right: blank() };
      return { ...s, left: full(s.left), right: full(s.right) };
    };
    const commit = (label, before, after) => api.pushUndo({ label, undo: () => E.set(before), redo: () => E.set(after) });
    let inventory = null;
    api.call("eyes.inventory", {}).then((r) => { inventory = r; render(); }).catch(() => {});
    const modelPicker = (s, rig) => {
      if (!inventory || !rig) return "";
      const current = s.model?.id || "";
      const groups = inventory.families.map((f) => `<optgroup label="${f}">${inventory.models.filter((m) => m.family === f)
        .map((m) => `<option value="${m.id}" ${m.id === current ? "selected" : ""}>${m.name || m.id}</option>`).join("")}</optgroup>`).join("");
      const radius = s.model?.radius_mm ?? 12;
      return `<label class="eye-row" title="a premade eye: shaded sclera, iris and cornea, built at the eye rig's places">eye model
          <select class="model"><option value="" ${current ? "" : "selected"}>${E.hasEyes() && !current ? "eyes file" : "none"}</option>${groups}</select></label>
        ${current ? `<label class="eye-row" title="eyeball radius in mm; double click: back to 12">radius mm
          <input type="range" class="radius" min="5" max="20" step="0.1" value="${radius}"><output>${radius}</output></label>` : ""}`;
    };
    const render = () => {
      const s = spec(), rig = s.rig || E.rig();
      if (!E.hasEyes() && !rig) { root.innerHTML = `<div class="note">open the page with an eyes file (&amp;eyes=), or use a catalog with an eye rig, to place eyeballs</div>`; return; }
      const sides = linked ? [["both eyes", ["left", "right"]]] : [["left eye", ["left"]], ["right eye", ["right"]]];
      root.innerHTML = `<div class="eyes">
        ${modelPicker(s, rig)}
        <label title="move and scale each eye with its socket, the way MakeHuman moves its helper eyeball; the eyelid clearance uses the moved eyes">
          <input type="checkbox" class="follow" ${s.follow === FOLLOW ? "checked" : ""} ${rig || s.follow === FOLLOW ? "" : "disabled"}> follow the sockets</label>
        ${rig ? "" : `<div class="note">this catalog has no eye rig (store import-eyerig makes one)</div>`}
        <label title="one set of sliders for both eyes; out and yaw mirror between the sides"><input type="checkbox" class="link" ${linked ? "checked" : ""}> link left and right</label>
        ${sides.map(([title, which]) => `<div class="eye-side"><b>${title}</b>${FIELDS.map((f, i) => `
          <label class="eye-row" title="${f[0]} relative to the socket; double click: back to 0">${f[0]}
            <input type="range" data-i="${i}" data-sides="${which.join(",")}" min="${f[3]}" max="${f[4]}" step="${f[5]}" value="${read(s[which[0]], f)}">
            <output>${read(s[which[0]], f)}</output></label>`).join("")}</div>`).join("")}
      </div>`;
      root.querySelector(".follow").addEventListener("change", (e) => {
        const before = E.get(), after = { ...spec(), follow: e.target.checked ? FOLLOW : "none", rig: e.target.checked ? rig : s.rig };
        if (!after.rig) delete after.rig;
        E.set(after, "eyes-panel");
        commit(`eyes ${e.target.checked ? "follow the sockets" : "stay where the eyes file puts them"}`, before, after);
      });
      root.querySelector(".link").addEventListener("change", (e) => { linked = e.target.checked; render(); });
      root.querySelector(".model")?.addEventListener("change", (e) => {
        const before = E.get(), after = spec();
        if (e.target.value) { after.model = { id: e.target.value, radius_mm: after.model?.radius_mm ?? 12 }; after.rig = rig; }
        else delete after.model;
        E.set(after, "eyes-panel");
        commit(e.target.value ? `eye model ${e.target.value}` : "no eye model", before, after);
        render();
      });
      const radius = root.querySelector(".radius");
      if (radius) {
        let rBefore = null;
        const setRadius = (x) => { const next = spec(); next.model = { ...next.model, radius_mm: x }; E.set(next, "eyes-panel"); radius.nextElementSibling.textContent = x; };
        radius.addEventListener("pointerdown", () => { rBefore = E.get(); });
        radius.addEventListener("input", () => { if (!rBefore) rBefore = E.get(); setRadius(+radius.value); });
        radius.addEventListener("change", () => { commit(`eye radius ${radius.value} mm`, rBefore, E.get()); rBefore = null; });
        radius.addEventListener("dblclick", () => { const b = E.get(); radius.value = 12; setRadius(12); commit("eye radius back to 12 mm", b, E.get()); });
      }
      let before = null;
      for (const el of root.querySelectorAll("input[type=range][data-i]")) {
        const f = FIELDS[+el.dataset.i], which = el.dataset.sides.split(",");
        const apply = (x) => {
          const next = spec();
          for (const side of which) write(next[side], f, x);
          E.set(next, "eyes-panel");
          el.nextElementSibling.textContent = x;
          return next;
        };
        el.addEventListener("pointerdown", () => { before = E.get(); });
        el.addEventListener("input", () => { if (!before) before = E.get(); apply(+el.value); });
        el.addEventListener("change", () => { const after = E.get(); commit(`${which.length > 1 ? "eyes" : which[0]} ${f[0]} to ${el.value}`, before, after); before = null; });
        el.addEventListener("dblclick", () => { const b = E.get(); el.value = f[1] === "scale" ? 1 : 0; const after = apply(+el.value); commit(`${which.length > 1 ? "eyes" : which[0]} ${f[0]} back`, b, after); });
      }
    };
    render();
    E.onChange((origin) => { if (origin !== "eyes-panel") render(); });
  },
});
