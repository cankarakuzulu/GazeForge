"""Copy MakeHuman's standard rigs and weights (CC0 assets of MPFB) into crates/gf-core/makehuman/rigs/ (embedded by gf-core) as deterministic
gzip files (no name, mtime 0), and print each original's and each copy's sha256 for THIRD_PARTY.md.

usage: python tools/third_party/pack_makehuman_rigs.py MPFB_CHECKOUT
"""
import glob
import gzip
import hashlib
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
src = os.path.join(sys.argv[1], "src", "mpfb", "data", "rigs", "standard")
out = os.path.join(ROOT, "crates", "gf-core", "makehuman", "rigs")
os.makedirs(out, exist_ok=True)
rows = []
for path in sorted(glob.glob(os.path.join(src, "*.json"))):
    raw = open(path, "rb").read()
    name = os.path.basename(path)
    with open(os.path.join(out, name + ".gz"), "wb") as fh:
        with gzip.GzipFile(filename="", mode="wb", fileobj=fh, mtime=0, compresslevel=9) as gz:
            gz.write(raw)
    packed = open(os.path.join(out, name + ".gz"), "rb").read()
    assert gzip.decompress(packed) == raw
    rows.append((name, len(raw), hashlib.sha256(raw).hexdigest(), len(packed), hashlib.sha256(packed).hexdigest()))
for r in rows:
    print(f"| `{r[0]}` | {r[1]:,} | `{r[2]}` | {r[3]:,} | `{r[4]}` |")
print(f"{len(rows)} files, {sum(r[1] for r in rows):,} bytes raw, {sum(r[3] for r in rows):,} gzipped")
