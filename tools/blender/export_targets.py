import fnmatch, gzip, json, os, re, sys, time

import numpy as np

MOVE_EPS = 1e-7
SCALE = 0.1
MACRO_DIRS = ("macrodetails", "breast")
OPPOSITE_SUFFIXES = [("incr", "decr"), ("in", "out"), ("up", "down"), ("forward", "backward"), ("open", "close"),
                     ("convex", "concave"), ("compress", "uncompress"), ("min", "max")]
MACRO_TOKENS = {
    "gender": ("female", "male"),
    "age": ("baby", "child", "young", "old"),
    "muscle": ("minmuscle", "averagemuscle", "maxmuscle"),
    "weight": ("minweight", "averageweight", "maxweight"),
    "ethnic": ("african", "asian", "caucasian", "universal"),
    "height": ("minheight", "maxheight"),
    "proportions": ("idealproportions", "uncommonproportions"),
    "cupsize": ("mincup", "averagecup", "maxcup"),
    "firmness": ("minfirmness", "averagefirmness", "maxfirmness"),
}
MPFB_KEY_TOKENS = {"$md": "macrodetails", "$af": "african", "$as": "asian", "$ca": "caucasian", "$fe": "female", "$ma": "male",
                   "$bb": "baby", "$ch": "child", "$yn": "young", "$ol": "old", "$mu": "muscle", "$wg": "weight", "$av": "average",
                   "$id": "ideal", "$pr": "proportions", "$fi": "firmness", "$un": "uncommon", "$hg": "height", "$ht": "height"}


def log(message):
    print(f"GFTGT {message}", flush=True)


def read_target(path):
    with gzip.open(path, "rt") as handle:
        rows = [line for line in handle if line.strip() and not line.lstrip().startswith("#")]
    if not rows:
        return np.zeros(0, np.int64), np.zeros((0, 3))
    table = np.array(" ".join(rows).split(), dtype=np.float64).reshape(-1, 4)
    index = table[:, 0].astype(np.int64)
    mh = table[:, 1:]
    return index, np.stack([mh[:, 0], -mh[:, 2], mh[:, 1]], axis=1) * SCALE


def enumerate_targets(root):
    found = []
    for folder, _, files in os.walk(root):
        for name in files:
            if name.endswith(".target.gz"):
                rel = os.path.relpath(os.path.join(folder, name), root).replace(os.sep, "/")
                found.append(rel[:-len(".target.gz")])
    return sorted(found)


def opposite_by_name(short):
    for a, b in OPPOSITE_SUFFIXES:
        for mine, theirs in ((a, b), (b, a)):
            if short.endswith("-" + mine):
                return short[:-len(mine)] + theirs, mine
    return None, None


def mirror_by_name(short):
    for pattern, swap in ((r"^l-", "r-"), (r"^r-", "l-")):
        if re.match(pattern, short):
            return swap + short[2:]
    for mine, theirs in (("-left-", "-right-"), ("-right-", "-left-"), ("-left", "-right"), ("-right", "-left")):
        if mine in short:
            return short.replace(mine, theirs, 1)
    return None


def macro_variables(key):
    if not key.split("/")[0] in MACRO_DIRS:
        return None
    short = key.split("/")[-1]
    tokens = short.split("-")
    found = {}
    for variable, values in MACRO_TOKENS.items():
        for token in tokens:
            if token in values:
                found[variable] = token
    return found or None


def slider_index(target_json):
    index = {}
    for group, body in target_json.items():
        for category in body.get("categories", []):
            for side, short in category.get("opposites", {}).items():
                if short:
                    index[short] = {"group": group, "slider": category["name"], "side": side}
    for group, body in target_json.items():
        for category in body.get("categories", []):
            opposites = category.get("opposites", {})
            for side, short in opposites.items():
                if not short:
                    continue
                polarity, hand = side.split("-")
                other = opposites.get(("positive" if polarity == "negative" else "negative") + "-" + hand, "")
                index[short]["opposite"] = other or None
    return index


def pairing(key, known, sliders):
    category = key.rsplit("/", 1)[0]
    short = key.split("/")[-1]
    result = {}
    entry = sliders.get(short) if sliders.get(short, {}).get("group") == category else None
    if entry:
        result.update({"slider": entry["slider"], "side": entry["side"], "source": "target.json"})
        if entry.get("opposite"):
            result["opposite"] = f"{category}/{entry['opposite']}"
    if "opposite" not in result:
        other, pole = opposite_by_name(short)
        if other and f"{category}/{other}" in known:
            result["opposite"] = f"{category}/{other}"
            result["pole"] = pole
            result.setdefault("source", "name")
    mirror = mirror_by_name(short)
    if mirror and f"{category}/{mirror}" in known:
        result["mirror"] = f"{category}/{mirror}"
    return result


def merge(out_dir):
    parts = sorted(p for p in os.listdir(out_dir) if p.startswith("catalog_part_") and p.endswith(".json"))
    targets, skipped, meta = {}, {}, {}
    for part in parts:
        data = json.load(open(os.path.join(out_dir, part)))
        targets.update(data["targets"])
        skipped.update(data["not_on_head"])
        meta = data["meta"]
    files_bytes = sum(os.path.getsize(os.path.join(out_dir, t["file"])) for t in targets.values())
    catalog = {"meta": {**meta, "parts": parts, "count": len(targets), "not_on_head_count": len(skipped), "bytes": files_bytes},
               "targets": dict(sorted(targets.items())), "not_on_head": dict(sorted(skipped.items()))}
    json.dump(catalog, open(os.path.join(out_dir, "catalog.json"), "w"), indent=1)
    log(f"merged {len(parts)} parts: {len(targets)} targets on the head, {len(skipped)} not on the head, {files_bytes:,} bytes of target files")


def run(args):
    import bpy
    from mathutils import kdtree

    out_dir, dense_dir = args[0], args[1]
    patterns = args[2].split(",") if len(args) > 2 and args[2] else ["*"]
    tag = args[3] if len(args) > 3 else "all"
    root = os.path.join(os.environ["BLENDER_USER_RESOURCES"], "extensions", "user_default", "mpfb", "data", "targets")
    started = time.time()

    head = bpy.data.objects["Head"]
    body = bpy.data.objects["Human"]
    head_world = np.array(head.matrix_world)
    local = np.zeros(len(head.data.vertices) * 3)
    head.data.vertices.foreach_get("co", local)
    base = local.reshape(-1, 3) @ head_world[:3, :3].T + head_world[:3, 3]

    def body_positions(masked):
        masks = [m for m in body.modifiers if m.type == "MASK"]
        states = [m.show_viewport for m in masks]
        for m in masks:
            m.show_viewport = masked
        bpy.context.view_layer.update()
        evaluated = body.evaluated_get(bpy.context.evaluated_depsgraph_get())
        mesh = evaluated.to_mesh()
        flat = np.zeros(len(mesh.vertices) * 3)
        mesh.vertices.foreach_get("co", flat)
        evaluated.to_mesh_clear()
        for m, state in zip(masks, states):
            m.show_viewport = state
        bpy.context.view_layer.update()
        world = np.array(body.matrix_world)
        return flat.reshape(-1, 3) @ world[:3, :3].T + world[:3, 3]

    def nearest(points, queries):
        tree = kdtree.KDTree(len(points))
        for index, point in enumerate(points):
            tree.insert(point.tolist(), index)
        tree.balance()
        found = np.zeros(len(queries), np.int64)
        gap = np.zeros(len(queries))
        for index, point in enumerate(queries):
            _, found[index], gap[index] = tree.find(point.tolist())
        return found, gap

    full_body = body_positions(False)
    head_to_body, gap = nearest(full_body, base)
    matched = gap <= 0.5e-3
    log(f"body {len(full_body):,} vertices unmasked; head {len(base):,}: {int(matched.sum()):,} matched within 0.5 mm "
        f"(max gap {gap.max() * 1000:.4f} mm), {len(base) - len(set(head_to_body[matched].tolist())):,} sharing a body vertex")
    masked_body = body_positions(True)
    masked_map, masked_gap = nearest(masked_body, base)
    log(f"masked body {len(masked_body):,} vertices: its head->body indices equal the unmasked ones on "
        f"{int((masked_map == head_to_body).sum()):,} of {len(base):,}")
    M3 = np.array(body.matrix_world)[:3, :3]

    mesh = head.data.copy()
    mesh.name = "gfA3_cage"
    mesh.vertices.foreach_set("co", base.ravel())
    cage = bpy.data.objects.new("gfA3_cage", mesh)
    bpy.context.scene.collection.objects.link(cage)
    modifier = cage.modifiers.new("cc", "SUBSURF")
    modifier.subdivision_type = "CATMULL_CLARK"
    modifier.levels = modifier.render_levels = 4

    def subdivide(control):
        cage.data.vertices.foreach_set("co", np.ascontiguousarray(control, dtype=np.float64).ravel())
        cage.data.update()
        bpy.context.view_layer.update()
        evaluated = cage.evaluated_get(bpy.context.evaluated_depsgraph_get())
        dense_mesh = evaluated.to_mesh()
        flat = np.zeros(len(dense_mesh.vertices) * 3)
        dense_mesh.vertices.foreach_get("co", flat)
        evaluated.to_mesh_clear()
        return flat.reshape(-1, 3)

    positions = np.load(os.path.join(dense_dir, "positions.npy")).astype(np.float64)
    clock = time.time()
    sub_base = subdivide(base)
    log(f"subdivided cage: {len(sub_base):,} vertices in {time.time() - clock:.2f} s; dense base has {len(positions):,}")
    evaluated = cage.evaluated_get(bpy.context.evaluated_depsgraph_get())
    dense_mesh = evaluated.to_mesh()
    sub_sizes = np.zeros(len(dense_mesh.polygons), np.int64)
    dense_mesh.polygons.foreach_get("loop_total", sub_sizes)
    sub_corners = np.zeros(len(dense_mesh.loops), np.int64)
    dense_mesh.loops.foreach_get("vertex_index", sub_corners)
    evaluated.to_mesh_clear()
    want_sizes = np.load(os.path.join(dense_dir, "face_sizes.npy"))
    want_corners = np.load(os.path.join(dense_dir, "corner_vertices.npy"))
    same_topology = len(sub_sizes) == len(want_sizes) and len(sub_corners) == len(want_corners) and \
        bool((sub_sizes == want_sizes).all()) and bool((sub_corners == want_corners).all())
    order = None
    if len(sub_base) == len(positions):
        gap = np.linalg.norm(sub_base - positions, axis=1)
        log(f"identity order: topology {'IDENTICAL' if same_topology else 'DIFFERENT'} to face_sizes/corner_vertices.npy; "
            f"|subdivided - positions.npy| max {gap.max() * 1000:.6f} mm, {int((gap > 1e-6).sum()):,} vertices over 1e-6 m")
        if same_topology:
            order = np.arange(len(positions))
    if order is None:
        log("WARNING topology differs: falling back to the order by position")
        order, order_gap = nearest(sub_base, positions)
        log(f"order by position: gap median {np.median(order_gap) * 1000:.6f} mm, max {order_gap.max() * 1000:.6f} mm, "
            f"{len(order) - len(np.unique(order)):,} repeated")

    target_json = json.load(open(os.path.join(root, "target.json")))
    sliders = slider_index(target_json)
    everything = enumerate_targets(root)
    known = set(everything)
    chosen = [k for k in everything if any(fnmatch.fnmatch(k, p) for p in patterns)]
    log(f"{len(everything)} target files under {root}; {len(chosen)} chosen by {patterns}")

    key_check = []
    if body.data.shape_keys:
        basis = np.zeros(len(body.data.vertices) * 3)
        body.data.shape_keys.key_blocks[0].data.foreach_get("co", basis)
        by_short = {}
        for k in everything:
            by_short.setdefault(k.split("/")[-1], []).append(k)
        for block in body.data.shape_keys.key_blocks[1:]:
            name = block.name
            for token, word in sorted(MPFB_KEY_TOKENS.items(), key=lambda kv: -len(kv[0])):
                name = name.replace(token, word)
            name = re.sub(r"^macrodetails-", "", name)
            hits = by_short.get(name, [])
            if len(hits) != 1:
                key_check.append({"key": block.name, "decoded": name, "files": hits})
                continue
            co = np.zeros(len(basis))
            block.data.foreach_get("co", co)
            key_delta = (co - basis).reshape(-1, 3)
            index, delta = read_target(os.path.join(root, hits[0] + ".target.gz"))
            mine = np.zeros_like(key_delta)
            if len(index):
                mine[index] = delta @ M3.T
            key_check.append({"key": block.name, "file": hits[0], "max_diff_mm_body": float(np.abs(mine - key_delta).max() * 1000),
                              "max_diff_mm_head": float(np.abs(mine[head_to_body] - key_delta[head_to_body]).max() * 1000),
                              "max_key_mm_head": float(np.linalg.norm(key_delta[head_to_body], axis=1).max() * 1000)})
            log(f"shape key check {block.name} -> {hits[0]}: max |file - key| body {key_check[-1]['max_diff_mm_body']:.6f} mm, "
                f"head {key_check[-1]['max_diff_mm_head']:.6f} mm (key moves the head {key_check[-1]['max_key_mm_head']:.3f} mm)")

    os.makedirs(out_dir, exist_ok=True)
    targets, skipped = {}, {}
    subdiv_time = 0.0
    for number, key in enumerate(chosen):
        index, delta = read_target(os.path.join(root, key + ".target.gz"))
        body_delta = np.zeros_like(full_body)
        if len(index):
            body_delta[index] = delta @ M3.T
        cage_delta = np.where(matched[:, None], body_delta[head_to_body], 0.0)
        cage_moved = np.linalg.norm(cage_delta, axis=1)
        if not (cage_moved > 0).any():
            reason = "empty file" if len(index) == 0 else "no head vertices in the file"
            skipped[key] = {"reason": reason, "file_rows": int(len(index)),
                            "body_max_mm": float(np.linalg.norm(delta, axis=1).max() * 1000) if len(index) else 0.0}
            continue
        clock = time.time()
        dense = subdivide(cage_delta)[order]
        subdiv_time += time.time() - clock
        moved = np.linalg.norm(dense, axis=1)
        keep = np.flatnonzero(moved > MOVE_EPS).astype(np.uint32)
        rel = key + ".npz"
        path = os.path.join(out_dir, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        np.savez(path, indices=keep, deltas=dense[keep].astype(np.float32))
        entry = {"name": key.split("/")[-1], "category": key.rsplit("/", 1)[0], "file": rel, "moved": int(len(keep)),
                 "max_mm": float(moved.max() * 1000), "cage_moved": int((cage_moved > 0).sum()), "file_rows": int(len(index)),
                 "bytes": os.path.getsize(path)}
        entry.update(pairing(key, known, sliders))
        macro = macro_variables(key)
        if macro:
            entry["macro"] = macro
        targets[key] = entry
        if number % 25 == 0 or number == len(chosen) - 1:
            log(f"[{number + 1}/{len(chosen)}] {key}: {len(keep):,} moved, max {entry['max_mm']:.3f} mm, "
                f"{time.time() - started:.0f} s elapsed")
    meta = {"source_blend": os.path.basename(bpy.data.filepath), "dense_vertices": int(len(positions)), "cage_vertices": int(len(base)),
            "move_eps_m": MOVE_EPS, "units": "metres, Blender axes (MakeHuman x, -z, y times 0.1)",
            "subdivision": "Catmull-Clark level 4, Blender SUBSURF defaults", "head_matched": int(matched.sum()),
            "macro_system": "macrodetails/macro.json under the MPFB targets root: a macro slider maps to weights over these files",
            "shape_key_check": key_check}
    json.dump({"meta": meta, "targets": targets, "not_on_head": skipped},
              open(os.path.join(out_dir, f"catalog_part_{tag}.json"), "w"), indent=1)
    log(f"done {tag}: {len(targets)} baked, {len(skipped)} not on the head, subdivision {subdiv_time:.1f} s, "
        f"total {time.time() - started:.1f} s")


if __name__ == "__main__":
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else sys.argv[1:]
    if argv and argv[0] == "merge":
        merge(argv[1])
    else:
        run(argv)
