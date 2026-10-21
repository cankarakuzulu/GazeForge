# GAZEFORGE_physics

A glTF 2.0 extension that says which meshes of a character are cloth, which joints are spring-bone chains, and what the
body they move against looks like, so any engine can simulate them with its own solver. Nothing in the file moves by itself: a reader that does not know the
extension shows the wearables skinned to the body, as exported.

Status: version 1. Written by GazeForge's `export-glb` for wearables worn with `"physics": "cloth"` or
`"physics": "springbone"`. Readers: the Godot 4 addon `clients/godot-addon/addons/gazeforge_physics` (spring-bone
chains, cloth and colliders).

## Placement

At the root of the document, under `extensions.GAZEFORGE_physics`. The writer lists `GAZEFORGE_physics` in
`extensionsUsed` and never in `extensionsRequired`.

```json
{
  "extensions": {
    "GAZEFORGE_physics": {
      "version": 1,
      "frame": "glTF: metres, y up, in the Character root's space (the root's transform applies, like the meshes')",
      "pin": {"near_m": 0.005, "far_m": 0.03},
      "cloth": [
        {"wearable": "cape", "nodes": ["cape_LOD0", "cape_LOD1", "cape_LOD2"], "pin": "_PIN"}
      ],
      "springs": [
        {"wearable": "ponytail01", "chain": 0, "joints": [53, 54, 55, 56, 57], "parent_bone": "head", "root_vertices": 78}
      ],
      "spring_defaults": {"stiffness": 1.0, "drag": 0.4, "gravity": 1.0, "hit_radius_m": 0.02},
      "colliders": [
        {"joint": 11, "bone": "head", "head": [0.0, 1.62, 0.01], "tail": [0.0, 1.78, 0.02], "radius": 0.087, "from_vertices": 4076},
        {"joint": 46, "bone": "upperarm_l", "head": [0.17, 1.42, -0.03], "tail": [0.36, 1.31, -0.05], "radius": 0.047, "from_vertices": 284}
      ]
    }
  }
}
```

## Cloth

Each `cloth` entry names one wearable and the mesh node that carries it on every level of detail (the same names as the
nodes in the scene). Every primitive of those meshes has a custom vertex attribute named by `pin` (`_PIN`, one float per
vertex):

- `1` - pinned: the vertex follows its skin weights exactly (the cloth lies on the body there).
- `0` - free: the vertex is simulated.
- in between - a blend: drive the vertex towards its skinned position with this strength.

The writer sets it from each vertex's rest distance outward from that level's body surface: `1` within `pin.near_m`,
`0` beyond `pin.far_m`, smoothstep between. The meshes stay skinned (`JOINTS_0` / `WEIGHTS_0`), so the skinned position
is always available as the pin target.

## Springs

A wearable worn with `"physics": "springbone"` (hair, ropes, a cape) gets chains of extra joints, appended to the skin
after the body's bones so the body's joint numbers never change. Each `springs` entry is one chain:

- `joints` - the chain's joints in the skin's `joints`, root first; each joint's parent is the one before it, and the
  root's parent is `parent_bone`. Joint nodes are named `<wearable>_chain<chain>_<k>`.
- `parent_bone` - the body bone the chain hangs from: the bone that drives most of its root vertices.
- `root_vertices` - how many of the wearable's vertices the chain hangs from.

The writer builds them from the wearable's own mesh: the vertices lying on the body of the anchor bone (the bone that
drives the topmost vertices resting on the body, the head for hair) are the roots; every other vertex is measured by its
distance over the mesh from the nearest root (a separate piece, such as a hair card, joins through its nearest point);
vertices are grouped by the roots they hang from, groups lying along each other merge, and each group gets a root joint
plus four joints at the centres of four equal shares of its vertices, nearest first. A vertex is skinned between the two
chain joints either side of it, blended with the body's weights by its `_PIN`-style pin where it rests on the anchor
bone's body (elsewhere the colliders keep it off the body).

`spring_defaults` are starting values in the shape of VRM's spring bones (stiffness, drag, gravity, hit radius), not
tuned per item.

## Colliders

Capsules for the body, one per bone (fingers and toes left out), in the Character root's space at the rest pose:

- `joint` - the index of the bone in the skin's `joints` (the order of the rig's bones), `bone` its name.
- `head`, `tail` - the capsule's segment, the bone's own head and tail on this character.
- `radius` - the 10th-percentile distance from that segment of the body and head vertices the bone drives most (its
  strongest weight), so a heavy character gets fat capsules and a thin one thin capsules, and a capsule stays inside
  its body part. (Until the first reader the median was used; on a flat torso a round capsule that size stood out of
  the back by about 10 cm, and a cape hung over it started inside it.)
- `from_vertices` - how many vertices the radius was measured over.

A reader attaches each capsule to its joint and moves it with the skeleton.

## Limits (version 1)

- No baked animation yet.
- Chains always have four segments; no per-chain stiffness or radius yet.
- No per-cloth stiffness, mass or damping: the solver's defaults apply.
- Colliders are capsules only and do not collide with each other's cloth (no self-collision data).
