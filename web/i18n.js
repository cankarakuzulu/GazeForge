let table = {};
let fallback = {};
const missing = new Set();

async function read(lang) {
  const r = await fetch(`locales/${lang}.json`);
  if (!r.ok) throw new Error(`locales/${lang}.json: ${r.status}`);
  return r.json();
}

export async function loadLocale(lang) {
  fallback = await read("en");
  table = fallback;
  const want = (lang || "en").toLowerCase().split("-")[0];
  if (want !== "en") {
    try { table = { ...fallback, ...(await read(want)) }; } catch (e) { console.warn(`i18n: no usable table for ${want} (${e.message}); English shown`); table = fallback; }
  }
  document.documentElement.lang = table === fallback ? "en" : want;
  return table;
}

export function has(key) {
  return key in table;
}

export function tr(key, vars, otherwise) {
  let s = table[key] ?? fallback[key];
  if (s === undefined) {
    if (otherwise === undefined && !missing.has(key)) { missing.add(key); console.warn(`i18n: no text for ${key}`); }
    s = otherwise ?? key;
  }
  return vars ? s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m)) : s;
}

export function readable(id) {
  const s = String(id).split(/[-_]/).filter(Boolean).map((w) => tr(`target.frag.${w}`, null, w)).join(" ");
  return s.charAt(0).toUpperCase() + s.slice(1);
}

export function applyDom(root = document) {
  for (const el of root.querySelectorAll("[data-i18n]")) el.textContent = tr(el.dataset.i18n);
  for (const el of root.querySelectorAll("[data-i18n-title]")) el.title = tr(el.dataset.i18nTitle);
  for (const el of root.querySelectorAll("[data-i18n-placeholder]")) el.placeholder = tr(el.dataset.i18nPlaceholder);
}

export function missingKeys() {
  return [...missing];
}
