"""Bundle the Lucide icons both clients use.

usage: build_icons.py [--from PACKAGE_DIR] [--check]

ICONS below is the single list. --from copies each named icon verbatim from an extracted lucide-static package
(PACKAGE_DIR/icons/<name>.svg) into third_party/lucide/icons/ and its LICENSE beside them, refusing a package whose
version differs from VERSION. Every run then rewrites the inline sprite in web/index.html (between the icons markers)
and crates/gf-studio/src/icon_data.rs from third_party/lucide/icons/. --check writes nothing and exits 1 when either
generated file is out of date. Prints one JSON object.
"""
import json
import os
import re
import shutil
import sys

VERSION = "1.60.0"
ICONS = [
    "box", "camera", "chart-scatter", "circle-help", "columns-2", "crosshair", "eye", "file-plus", "flip-horizontal-2",
    "focus", "folder-open", "gauge", "grid-3x3", "house", "images", "keyboard", "layout-grid", "lightbulb", "moon",
    "redo-2", "rotate-3d", "rotate-ccw", "ruler", "save", "save-all", "scan-eye", "scan-search", "scissors-line-dashed",
    "slice", "square", "sun", "sun-moon", "triangle-alert", "undo-2", "vector-square", "wand-sparkles", "x",
]
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
ICON_DIR = os.path.join(ROOT, "third_party", "lucide", "icons")
HTML = os.path.join(ROOT, "web", "index.html")
RUST = os.path.join(ROOT, "crates", "gf-studio", "src", "icon_data.rs")
BEGIN, END = "<!-- icons:begin -->", "<!-- icons:end -->"


def inner(svg):
    body = re.search(r"<svg[^>]*>(.*)</svg>", svg, re.S)
    if not body:
        raise ValueError("no <svg> element")
    return " ".join(line.strip() for line in body.group(1).strip().splitlines())


def sprite():
    rows = [BEGIN, '<svg xmlns="http://www.w3.org/2000/svg" style="display:none">']
    for name in ICONS:
        with open(os.path.join(ICON_DIR, name + ".svg"), encoding="utf-8") as f:
            rows.append(f'<symbol id="i-{name}" viewBox="0 0 24 24">{inner(f.read())}</symbol>')
    rows += ["</svg>", END]
    return "\n".join(rows)


def rust():
    lines = ["pub const ICONS: &[(&str, &str)] = &["]
    for name in ICONS:
        lines.append(f'    ("{name}", include_str!("../../../third_party/lucide/icons/{name}.svg")),')
    lines.append("];")
    return "\n".join(lines) + "\n"


def main(argv):
    check = "--check" in argv
    out = {"ok": True, "icons": len(ICONS), "version": VERSION}
    if "--from" in argv:
        pkg = argv[argv.index("--from") + 1]
        with open(os.path.join(pkg, "package.json"), encoding="utf-8") as f:
            meta = json.load(f)
        if meta.get("version") != VERSION or meta.get("license") != "ISC":
            print(json.dumps({"ok": False, "error": f"package is {meta.get('name')} {meta.get('version')} ({meta.get('license')}), want {VERSION} ISC"}))
            return 1
        os.makedirs(ICON_DIR, exist_ok=True)
        for name in ICONS:
            shutil.copyfile(os.path.join(pkg, "icons", name + ".svg"), os.path.join(ICON_DIR, name + ".svg"))
        shutil.copyfile(os.path.join(pkg, "LICENSE"), os.path.join(os.path.dirname(ICON_DIR), "LICENSE"))
        out["copied"] = len(ICONS)
    missing = [n for n in ICONS if not os.path.exists(os.path.join(ICON_DIR, n + ".svg"))]
    if missing:
        print(json.dumps({"ok": False, "error": "icons missing from third_party/lucide/icons", "missing": missing}))
        return 1
    with open(HTML, encoding="utf-8", newline="") as f:
        html = f.read()
    if BEGIN not in html or END not in html:
        print(json.dumps({"ok": False, "error": f"{HTML} has no {BEGIN} ... {END} block"}))
        return 1
    new_html = html[: html.index(BEGIN)] + sprite() + html[html.index(END) + len(END):]
    new_rust = rust()
    old_rust = open(RUST, encoding="utf-8").read() if os.path.exists(RUST) else ""
    stale = [p for p, a, b in ((HTML, html, new_html), (RUST, old_rust, new_rust)) if a != b]
    out["stale"] = [os.path.relpath(p, ROOT).replace(os.sep, "/") for p in stale]
    if check:
        out["ok"] = not stale
        print(json.dumps(out))
        return 0 if not stale else 1
    with open(HTML, "w", encoding="utf-8", newline="") as f:
        f.write(new_html)
    with open(RUST, "w", encoding="utf-8", newline="\n") as f:
        f.write(new_rust)
    out["written"] = out.pop("stale")
    print(json.dumps(out))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
