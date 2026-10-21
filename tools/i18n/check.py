"""UI text gate: every user-facing string of both clients comes from the locale table.

usage: py -3 tools/i18n/check.py [--json]

The table is web/locales/en.json (flat keys, {name} placeholders); other locales are web/locales/<lang>.json with a
subset of its keys. The browser reads it through web/i18n.js (tr, data-i18n, data-i18n-title, data-i18n-placeholder),
the studio embeds it through crates/gf-studio/src/i18n.rs (tr, trf).

Refused:
  literal  a letter-bearing literal in a UI sink: HTML text nodes and title/placeholder/alt/aria-label attributes
           (index.html and string templates in web/**/*.js), .textContent/.title/.placeholder assignments, toast(),
           setStatus(), createTextNode(), label:, text: and new Error() in JS, and the text argument of the
           panels' el(tag, attrs, text) helper; label/button/heading/strong/selectable_label/
           on_hover_text/hint_text/menu_button calls, RichText/Button/CollapsingHeader/Window::new, checkbox and
           selectable_value texts, tip(), status assignments and Some("...".into()) in the studio
  missing  a key used in code (tr("k") / trf("k", ...) / data-i18n*="k") that the table lacks
  unused   a table key no code names (keys under a dynamic prefix such as mode.* are exempt, see DYNAMIC)
  case     a value that starts with a lower-case letter, outside fragments (a key holding ".frag.", or under unit.); a value
           starting with a placeholder, digit or punctuation is not judged
  escape   a value holding a backslash followed by n or t (the newline was escaped twice, so it shows literally)
  locale   a key in another locale that en.json lacks, or a placeholder set that differs from en.json's

Limits (what this does NOT see): a literal assigned to a variable first and shown later; text built by string
concatenation outside the sinks above; strings the server sends (wizard definitions, command errors, job stages),
which stay English on purpose and are looked up by key with the server text as fallback; Rust strings built in a
helper that returns String and is shown by a caller. ALLOW lists literals that are developer-facing (thrown to panel
authors, not users); every entry there is a reviewed exception.
"""
import json
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
EN = os.path.join(ROOT, "web", "locales", "en.json")
FRAGMENTS = (".frag.", "unit.")
DYNAMIC = ("shortcut_key.", "skin.kind.", "skin.style.", "skin.add.", "skin.preset.", "skin.variant.")
ALLOW = {
    "web/designer.js": ["pushUndo needs { label, undo(), redo() }", "registerPanel needs { id, title, order, mount(rootEl, api) }", "setPose needs "],
    "web/viewer.js": ["shader: ", "program: "],
    "web/wizard.js": ["not a glb", "accessor "],
    "web/panels/topology.js": ["LOD0", "topology.inspect"],
    "crates/gf-studio/src/topology.rs": ["LOD0"],
}
LETTERS = r"[A-Za-z]{2}"
SEP = "\x01"
JS_SINKS = [
    re.compile(r">([^<>{}$`\"'\x01]*" + LETTERS + r"[^<>{}$`\"'\x01]*)[<\x01]|\x01([^<>{}$`\"'\x01]*" + LETTERS + r"[^<>{}$`\"'\x01]*)<"),
    re.compile(r"(?<![\w-])(?:title|placeholder|alt|aria-label)=\"([^\"$\x01]*" + LETTERS + r"[^\"\x01]*)\""),
    re.compile(r"\.(?:textContent|innerText|title|placeholder)\s*=\s*[\"'`]([^\"'`$]*" + LETTERS + r")"),
    re.compile(r"\btoast\(\s*[\"'`]([^\"'`$]*" + LETTERS + r")"),
    re.compile(r"\bsetStatus\([^,]+,\s*[\"'`]([^\"'`$]*" + LETTERS + r")"),
    re.compile(r"\bcreateTextNode\(\s*[\"'`]([^\"'`$]*" + LETTERS + r")"),
    re.compile(r"\blabel:\s*[\"'`]([^\"'`$]*" + LETTERS + r")"),
    re.compile(r"\bnew Error\(\s*[\"'`]([^\"'`$]*" + LETTERS + r"[^\"'`]*)"),
    re.compile(r"\btext:\s*[\"'`]([^\"'`$]*" + LETTERS + r")"),
    re.compile(r"\bel\([\"'][a-z]+[\"'],[^()]*?,\s*[\"'`]([^\"'`$]*" + LETTERS + r")(?![^\"'`$]*[\"'`]\s*:)"),
]
RS_SINKS = [
    re.compile(r"\.(?:label|button|small_button|heading|strong|selectable_label|on_hover_text|hint_text|menu_button)\((?:&?format!\()?\"([^\"]*" + LETTERS + r"[^\"]*)\""),
    re.compile(r"(?:RichText|Button|CollapsingHeader|Window)::new\((?:&?format!\()?\"([^\"]*" + LETTERS + r"[^\"]*)\""),
    re.compile(r"\bcheckbox\([^,]+,\s*(?:&?format!\()?\"([^\"]*" + LETTERS + r"[^\"]*)\""),
    re.compile(r"\bselectable_value\([^\"]*\"([^\"]*" + LETTERS + r"[^\"]*)\""),
    re.compile(r"\btip\(\s*\"([^\"]*" + LETTERS + r"[^\"]*)\""),
    re.compile(r"\bstatus = (?:format!\()?\"([^\"]*" + LETTERS + r"[^\"]*)\""),
    re.compile(r"Some\(\"([^\"]*" + LETTERS + r"[^\"]*)\"\.into\(\)\)"),
]
RS_SKIP = {"ui_script.rs", "script.rs", "main.rs", "i18n.rs"}
KEY_ARG = re.compile(r"\b(trf?|tr_or)\(\s*(&format!\()?\"[^\"]*\"")
KEY_USE = [
    re.compile(r"\btrf?\(\s*[\"']([A-Za-z0-9_.\-]+)[\"']"),
    re.compile(r"data-i18n(?:-title|-placeholder)?=\"([A-Za-z0-9_.\-]+)\""),
]
PREFIX_USE = [
    re.compile(r"\btrf?\(\s*`([A-Za-z0-9_.\-]+\.)\$\{"),
    re.compile(r"\b(?:trf?|tr_or)\(\s*&format!\(\"([A-Za-z0-9_.\-]+\.)\{"),
]


def files():
    out = [os.path.join(ROOT, "web", "index.html")]
    for d in ("web", os.path.join("web", "panels")):
        for n in sorted(os.listdir(os.path.join(ROOT, d))):
            if n.endswith(".js") and n != "i18n.js":
                out.append(os.path.join(ROOT, d, n))
    src = os.path.join(ROOT, "crates", "gf-studio", "src")
    for n in sorted(os.listdir(src)):
        if n.endswith(".rs"):
            out.append(os.path.join(src, n))
    return out


def js_literals(text):
    out = [c if c == "\n" else " " for c in text]
    i, n, stack, prev = 0, len(text), [], ""
    while i < n:
        c = text[i]
        if c == "/" and text[i:i + 2] == "//":
            while i < n and text[i] != "\n":
                i += 1
            continue
        if c == "/" and text[i:i + 2] == "/*":
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        if c == "/" and (prev == "" or prev in "(,=:[!&|?{};+-*%<>~^"):
            i += 1
            klass = False
            while i < n and text[i] != "\n":
                if text[i] == "\\":
                    i += 2
                    continue
                if text[i] == "[":
                    klass = True
                elif text[i] == "]":
                    klass = False
                elif text[i] == "/" and not klass:
                    break
                i += 1
            i += 1
            prev = "x"
            continue
        if c in "\"'":
            out[i] = SEP
            i += 1
            while i < n and text[i] != c and text[i] != "\n":
                if text[i] == "\\":
                    out[i] = text[i]
                    i += 1
                out[i] = text[i]
                i += 1
            if i < n:
                out[i] = SEP
            i += 1
            prev = "x"
            continue
        if c == "`" or (c == "}" and stack and stack[-1] == 0):
            if c == "}":
                stack.pop()
            out[i] = SEP
            i += 1
            while i < n and text[i] != "`":
                if text[i] == "\\":
                    out[i] = text[i]
                    i += 1
                elif text[i:i + 2] == "${":
                    stack.append(0)
                    out[i] = SEP
                    i += 2
                    break
                out[i] = text[i]
                i += 1
            else:
                if i < n:
                    out[i] = SEP
                i += 1
                prev = "x"
            continue
        if c == "{" and stack:
            stack[-1] += 1
        elif c == "}" and stack and stack[-1] > 0:
            stack[-1] -= 1
        if not c.isspace():
            prev = c
        i += 1
    return "".join(out)


def scan_text(path, text):
    rel = os.path.relpath(path, ROOT).replace(os.sep, "/")
    problems, used = [], set()
    if path.endswith(".html"):
        text = re.sub(r"<!-- icons:begin -->.*?<!-- icons:end -->", "", text, flags=re.S)
        text = re.sub(r"<script\b.*?</script>", "", text, flags=re.S)
    if path.endswith(".rs"):
        text = text.split("#[cfg(test)]")[0]
    allow = ALLOW.get(rel, [])
    if path.endswith(".rs"):
        passes = [] if os.path.basename(path) in RS_SKIP else [(text, RS_SINKS)]
    elif path.endswith(".js"):
        passes = [(js_literals(text), JS_SINKS[:2]), (text, JS_SINKS[2:])]
    else:
        passes = [(text, JS_SINKS[:2])]
    for i, line in enumerate(text.splitlines(), 1):
        for rx in KEY_USE:
            used.update(rx.findall(line))
        for rx in PREFIX_USE:
            used.update(p + "*" for p in rx.findall(line))
    for body, sinks in passes:
        for i, line in enumerate(body.splitlines(), 1):
            if path.endswith(".rs"):
                line = KEY_ARG.sub(r"\1(\2KEY", line)
            for rx in sinks:
                for m in rx.finditer(line):
                    lit = next(g for g in m.groups() if g is not None).strip()
                    if not re.search(LETTERS, re.sub(r"\$?\{[^}]*\}", "", lit)):
                        continue
                    if lit == "GazeForge" or any(lit.startswith(a) for a in allow):
                        continue
                    problems.append({"kind": "literal", "file": rel, "line": i, "text": lit[:80]})
    return problems, used


def placeholders(s):
    return sorted(set(re.findall(r"\{(\w+)\}", s)))


def check():
    table = json.load(open(EN, encoding="utf-8"))
    problems, used = [], set()
    for p in files():
        pr, u = scan_text(p, open(p, encoding="utf-8").read())
        problems += pr
        used |= u
    prefixes = tuple(u[:-1] for u in used if u.endswith("*"))
    used = {u for u in used if not u.endswith("*")}
    for key in sorted(used - set(table)):
        problems.append({"kind": "missing", "key": key})
    for key in sorted(set(table) - used):
        if not key.startswith(DYNAMIC + prefixes):
            problems.append({"kind": "unused", "key": key})
    for key, value in table.items():
        first = value.lstrip()[:1]
        if first.isalpha() and first.islower() and not (key.startswith("unit.") or ".frag." in key):
            problems.append({"kind": "case", "key": key, "text": value[:60]})
        if "\\n" in value or "\\t" in value:
            problems.append({"kind": "escape", "key": key, "text": "an escaped \\n or \\t; JSON wants the character itself"})
    loc = os.path.dirname(EN)
    for n in sorted(os.listdir(loc)):
        if n.endswith(".json") and n != "en.json":
            other = json.load(open(os.path.join(loc, n), encoding="utf-8"))
            for key, value in other.items():
                if key not in table:
                    problems.append({"kind": "locale", "file": n, "key": key, "text": "not in en.json"})
                elif placeholders(value) != placeholders(table[key]):
                    problems.append({"kind": "locale", "file": n, "key": key, "text": "placeholders differ"})
    return {"ok": not problems, "keys": len(table), "used": len(used & set(table)), "problems": problems}


if __name__ == "__main__":
    r = check()
    if "--json" in sys.argv:
        print(json.dumps(r, indent=1))
    else:
        counts = {}
        for p in r["problems"]:
            counts[p["kind"]] = counts.get(p["kind"], 0) + 1
        print(json.dumps({"ok": r["ok"], "keys": r["keys"], "used": r["used"], "counts": counts, "first": r["problems"][:40]}, indent=1))
    sys.exit(0 if r["ok"] else 1)
