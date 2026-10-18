import hashlib, json, os, sys
import bpy
import numpy as np
from mathutils import kdtree

args = sys.argv[sys.argv.index("--") + 1:]
OUT = args[0]
BAKES = args[1:]
UV = os.environ.get("GF_UV", "head")
CAGE = os.environ.get("GF_CAGE", "")
CAGE_OBJECT = os.environ.get("GF_CAGE_OBJECT", "Head")
ALLOW_POSITION_ORDER = os.environ.get("GF_ALLOW_POSITION_ORDER") == "1"
SAME_POINT_M = 1e-6
SIGNATURE_SEED = 20261018


def log(message):
    print(f"GFEXPORT {message}", flush=True)


def refuse(message):
    print(f"GFEXPORT REFUSED: {message}", flush=True)
    sys.exit(2)


def world_positions(obj, mesh):
    world = np.array(obj.matrix_world)
    local = np.zeros(len(mesh.vertices) * 3)
    mesh.vertices.foreach_get("co", local)
    return local.reshape(-1, 3) @ world[:3, :3].T + world[:3, 3]


def topology(mesh):
    sizes = np.zeros(len(mesh.polygons), np.int64)
    mesh.polygons.foreach_get("loop_total", sizes)
    corners = np.zeros(len(mesh.loops), np.int64)
    mesh.loops.foreach_get("vertex_index", corners)
    return sizes, corners


def load_cage():
    if not CAGE:
        return None
    with bpy.data.libraries.load(CAGE, link=False) as (source, target):
        if CAGE_OBJECT not in source.objects:
            refuse(f"{CAGE} has no object '{CAGE_OBJECT}'")
        target.objects = [CAGE_OBJECT]
    return target.objects[0]


def with_level4(obj):
    bpy.context.scene.collection.objects.link(obj)
    modifier = obj.modifiers.new("cc", "SUBSURF")
    modifier.subdivision_type = "CATMULL_CLARK"
    modifier.levels = modifier.render_levels = 4
    return obj


def cage_copy(cage):
    copy = cage.copy()
    copy.data = cage.data.copy()
    copy.modifiers.clear()
    if copy.data.shape_keys:
        copy.shape_key_clear()
    return with_level4(copy)


def pydata_rebuild(cage):
    sizes, corners = topology(cage.data)
    starts = np.concatenate([[0], np.cumsum(sizes)[:-1]])
    faces = [tuple(corners[s:s + n]) for s, n in zip(starts, sizes)]
    mesh = bpy.data.meshes.new("gf_pydata")
    mesh.from_pydata(world_positions(cage, cage.data).tolist(), [], faces)
    return with_level4(bpy.data.objects.new("gf_pydata", mesh))


def evaluate(obj, local=None):
    if local is not None:
        obj.data.vertices.foreach_set("co", np.ascontiguousarray(local, dtype=np.float64).ravel())
        obj.data.update()
        bpy.context.view_layer.update()
    evaluated = obj.evaluated_get(bpy.context.evaluated_depsgraph_get())
    mesh = evaluated.to_mesh()
    if local is None:
        positions = world_positions(obj, mesh)
    else:
        flat = np.zeros(len(mesh.vertices) * 3)
        mesh.vertices.foreach_get("co", flat)
        positions = flat.reshape(-1, 3)
    sizes, corners = topology(mesh)
    evaluated.to_mesh_clear()
    return positions, sizes, corners


def nearest(points, queries, second=False):
    tree = kdtree.KDTree(len(points))
    for index, point in enumerate(points):
        tree.insert(point.tolist(), index)
    tree.balance()
    found = np.zeros(len(queries), np.int64)
    gap = np.zeros(len(queries))
    runner_up = np.zeros(len(queries))
    for index, point in enumerate(queries):
        if second:
            hits = tree.find_n(point.tolist(), 2)
            found[index], gap[index], runner_up[index] = hits[0][1], hits[0][2], hits[1][2]
        else:
            _, found[index], gap[index] = tree.find(point.tolist())
    return found, gap, runner_up


def pydata_to_identity(cage, copy, identity):
    rebuilt = pydata_rebuild(cage)
    pydata_positions, pydata_sizes, _ = evaluate(rebuilt)
    log(f"from_pydata rebuild of the cage (the bake order): {len(pydata_positions):,} vertices, "
        f"{len(pydata_sizes):,} faces")
    mappings = []
    for seed in (SIGNATURE_SEED, SIGNATURE_SEED + 1):
        signature = np.random.default_rng(seed).uniform(0.0, 1.0, (len(cage.data.vertices), 3))
        copy_signature = evaluate(copy, signature)[0]
        rebuilt_signature = evaluate(rebuilt, signature)[0]
        mapping, gap, runner_up = nearest(rebuilt_signature, copy_signature, second=True)
        repeated = len(mapping) - len(np.unique(mapping))
        log(f"  random cage coordinates seed {seed}: match gap max {gap.max():.3e}, nearest OTHER vertex min {runner_up.min():.3e} "
            f"(margin x{runner_up.min() / max(gap.max(), 1e-30):,.0f}), {repeated:,} repeated, order differs from the identity on "
            f"{int((mapping != np.arange(len(mapping))).sum()):,}")
        if repeated or runner_up.min() < 10 * gap.max():
            refuse("the from_pydata order could not be mapped to the identity by topology")
        mappings.append(mapping)
    disagree = int((mappings[0] != mappings[1]).sum())
    geometric = np.linalg.norm(pydata_positions[mappings[0]] - identity, axis=1).max()
    log(f"  the two seeds disagree on {disagree:,} vertices; real positions through the map vs the identity max {geometric * 1000:.6f} mm")
    if disagree or geometric > SAME_POINT_M:
        refuse("the from_pydata order could not be mapped to the identity by topology")
    for obj in (rebuilt, copy):
        obj.modifiers.clear()
    return pydata_positions, mappings[0]


os.makedirs(OUT, exist_ok=True)
head = bpy.data.objects["Head"]
mesh = head.data
positions = world_positions(head, mesh).astype(np.float32)
sizes, corners = topology(mesh)
uv = np.zeros(len(mesh.loops) * 2)
mesh.uv_layers[UV].data.foreach_get("uv", uv)
np.save(os.path.join(OUT, "positions.npy"), positions)
np.save(os.path.join(OUT, "face_sizes.npy"), sizes.astype(np.uint32))
np.save(os.path.join(OUT, "corner_vertices.npy"), corners.astype(np.uint32))
np.save(os.path.join(OUT, "corner_uvs.npy"), uv.reshape(-1, 2).astype(np.float32))
log(f"dense base: {len(positions):,} vertices, {len(sizes):,} faces, {len(corners):,} corners, UV layer '{UV}'")

cage = load_cage()
order_info = {"method": None, "cage": CAGE, "cage_object": CAGE_OBJECT, "position_matched_vertices": 0}
if cage is None:
    reason = "no cage given (set GF_CAGE=<blend holding the cage Head>)"
else:
    copy = cage_copy(cage)
    identity, cage_sizes, cage_corners = evaluate(copy)
    same = len(cage_sizes) == len(sizes) and len(cage_corners) == len(corners) and \
        bool(np.array_equal(cage_sizes, sizes)) and bool(np.array_equal(cage_corners, corners))
    gap = np.linalg.norm(identity - positions, axis=1) if len(identity) == len(positions) else np.array([np.inf])
    log(f"cage '{cage.name}' from {CAGE}: {len(cage.data.vertices):,} vertices, level 4 -> {len(identity):,} vertices, "
        f"{len(cage_sizes):,} faces; topology {'IDENTICAL' if same else 'DIFFERENT'} to the dense base")
    log(f"|level 4 - dense| by index: median {np.median(gap) * 1000:.6f} mm, max {gap.max() * 1000:.4f} mm, "
        f"{int((gap > 1e-6).sum()):,} vertices over 1e-6 m (the dense base's own edits, e.g. the lid collision)")
    reason = None if same else "the subdivided cage's face_sizes/corner_vertices differ from the dense base's"
if reason is None:
    order_info.update(method="topology", identity_offset_max_mm=float(gap.max() * 1000),
                      identity_offset_over_1e6m=int((gap > 1e-6).sum()))
    np.save(os.path.join(OUT, "cage_level4.npy"), identity)
    pydata_positions, pydata_map = pydata_to_identity(cage, copy, identity)
elif not ALLOW_POSITION_ORDER:
    refuse(f"{reason}; set GF_ALLOW_POSITION_ORDER=1 to fall back to matching by position (mis-orders any vertex the dense "
           f"base moved, e.g. the lid collision)")
else:
    log("!" * 100)
    log(f"WARNING: {reason}. FALLING BACK TO THE ORDER BY POSITION. Every vertex the dense base moved off the plain subdivision")
    log("WARNING: (the lid collision) can take a neighbour's shape delta. These shapes are suspect.")
    log("!" * 100)
    order_info.update(method="position")

shapes = []
bake_cache = {}
for bake in BAKES:
    data = np.load(bake)
    sub_base = data["subdivided_base"]
    key = hashlib.sha256(np.ascontiguousarray(sub_base).tobytes()).hexdigest()
    if key not in bake_cache:
        if order_info["method"] == "topology":
            if len(sub_base) != len(identity):
                refuse(f"{bake}: {len(sub_base):,} vertices, the cage's level 4 has {len(identity):,}")
            as_identity = np.linalg.norm(sub_base - identity, axis=1).max()
            as_pydata = np.linalg.norm(sub_base - pydata_positions, axis=1).max()
            if as_identity <= SAME_POINT_M:
                bake_order, how = np.arange(len(identity)), f"identity order (max {as_identity * 1000:.6f} mm by index)"
            elif as_pydata <= SAME_POINT_M:
                bake_order = pydata_map
                how = f"from_pydata order (max {as_pydata * 1000:.6f} mm by index against the rebuild), mapped to the identity " \
                      f"by topology; after the map vs the identity max " \
                      f"{np.linalg.norm(sub_base[pydata_map] - identity, axis=1).max() * 1000:.6f} mm"
            else:
                refuse(f"{bake}: subdivided_base matches neither the identity order (max {as_identity * 1000:.4f} mm) nor the "
                       f"from_pydata order (max {as_pydata * 1000:.4f} mm) of this cage - baked on another cage?")
        else:
            bake_order, twin_gap, _ = nearest(sub_base, positions)
            order_info["position_matched_vertices"] = int(len(positions))
            order_info["position_gap_over_1e5m"] = int((twin_gap > 1e-5).sum())
            how = f"BY POSITION against the dense base: gap max {twin_gap.max() * 1000:.4f} mm, " \
                  f"{int((twin_gap > 1e-5).sum()):,} without an exact twin"
        bake_cache[key] = bake_order
        log(f"bake order {os.path.basename(bake)}: {how}")
    delta = data["delta"][bake_cache[key]].astype(np.float32)
    name = os.path.splitext(os.path.basename(bake))[0]
    np.save(os.path.join(OUT, f"shape_{name}.npy"), delta)
    shapes.append({"name": name, "weights": json.loads(str(data["weights"])), "max_mm": float(np.linalg.norm(delta, axis=1).max() * 1000)})
    log(f"shape {name}: max move {shapes[-1]['max_mm']:.2f} mm")
json.dump({"source": os.path.basename(bpy.data.filepath), "uv_layer": UV, "vertices": len(positions), "faces": len(sizes),
           "order": order_info, "shapes": shapes}, open(os.path.join(OUT, "manifest.json"), "w"), indent=1)
log(f"wrote {OUT} (order: {order_info['method']})")
