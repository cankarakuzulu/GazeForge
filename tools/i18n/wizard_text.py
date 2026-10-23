"""Merge the task (wizard) texts into the locale: wizard.<id>.title / .summary, wizard.group.<group>,
wizard.<id>.<step>.title / .help and wizard.<id>.<step>.<field> for every field label, as gf-api's wizards::ui_texts names
them. The gf-api test every_task_text_has_its_locale_entry writes the missing or stale entries to target/wizard_texts.json
and fails; this script merges that file.

usage: py -3 tools/i18n/wizard_text.py [ENTRIES.json]   (default target/wizard_texts.json)
Rewrites web/locales/en.json in place (sorted, indent 1), keeping every other entry.
"""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def main():
    src = sys.argv[1] if len(sys.argv) > 1 else os.path.join(ROOT, "target", "wizard_texts.json")
    entries = json.load(open(src, encoding="utf-8"))
    path = os.path.join(ROOT, "web", "locales", "en.json")
    en = json.load(open(path, encoding="utf-8"))
    added = sum(1 for k in entries if k not in en)
    changed = sum(1 for k, v in entries.items() if k in en and en[k] != v)
    en.update(entries)
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(json.dumps(dict(sorted(en.items())), ensure_ascii=False, indent=1) + "\n")
    print(json.dumps({"added": added, "changed": changed, "keys": len(en)}))


if __name__ == "__main__":
    main()
