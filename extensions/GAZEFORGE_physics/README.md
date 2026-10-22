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
        {"wearable": "ponytail01", "chain": 0, "joints": [53, 54, 55, 56, 57], "parent_bone": "head", "root_vertices": 78,
         "hit_radius_m": 0.02, "collider_radii": []},
        {"wearable": "ponytail01", "chain": 1, "joints": [58, 59, 60, 61, 62], "parent_bone": "head", "root_vertices": 6,
         "hit_radius_m": 0.02, "collider_radii": [{"collider": 12, "radius": 0.018}, {"collider": 15, "radius": 0.035}]}
      ],
      "spring_defaults": {"stiffness": 1.0, "drag": 0.4, "gravity": 0.1, "hit_radius_m": 0.02},
      "baked": [{"animation": "sway_baked", "clip": "sway", "format": "bones", "fps": 30.0, "seconds": 4.0}],
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
- `hit_radius_m` - the chain's joint radius (optional; `spring_defaults.hit_radius_m` when absent).
- `collider_radii` - capsules this chain sees thinner than the others do (optional): `collider` is an index into
  `colliders`, `radius` the radius this chain uses for it.

The chain's modelled shape must be a legal rest, or a solver throws it out of the body at the first frame. So the
writer measures each chain's joints at rest against every capsule: a capsule the joints come within its radius plus the
hit radius (and 1 mm) of is listed in `collider_radii`, shrunk to the joints' distance from its segment less the hit
radius and that 1 mm, and the hit radius itself is lowered to the joints' nearest distance from any capsule segment if
that is smaller. The shrunk capsule keeps the rest legal and still stops the chain going deeper. (Measured on
MakeHuman's ponytail: a short tuft at the nape started 11.6 mm inside the head capsule and 4.6 mm inside the neck's;
with the full capsules it settled 40 mm off its modelled shape, with those two left out it sank 35 mm into the neck
when the head turned, with them shrunk it settles 2 mm off and goes 5 mm in.)

The writer builds them from the wearable's own mesh: the vertices lying on the body of the anchor bone (the bone that
drives the topmost vertices resting on the body, the head for hair) are the roots; every other vertex is measured by its
distance over the mesh from the nearest root (a separate piece, such as a hair card, joins through its nearest point);
vertices are grouped by the roots they hang from, groups lying along each other merge, and each group gets a root joint
plus four joints at the centres of four equal shares of its vertices, nearest first. A vertex is skinned between the two
chain joints either side of it, blended with the body's weights by its `_PIN`-style pin where it rests on the anchor
bone's body (elsewhere the colliders keep it off the body).

`spring_defaults` are starting values in the shape of VRM's spring bones (stiffness, drag, gravity, hit radius), not
tuned per item. Gravity is 0.1: with stiffness 1 it lets hair settle a few degrees under its weight without leaving its
modelled shape (measured on the ponytail: the tip settles 26 mm from where it was modelled at 0.1, 65 mm at 1.0, the
value files carried until 2026-10-22).

## Baked animation

`baked` lists the animations the writer simulated offline (`export-glb --bake`, `wearable bake`), for engines that run no
spring solver. Each entry names the glTF `animation`, the `clip` it was driven by (the built-in `sway`, or an animation
already in the file), the `format`, the frame rate and the length:

- `bones` - the clip's channels plus a rotation track for every chain joint but each chain's last (its tip, which keeps
  its rest rotation). An engine that also runs the springs live should not play this animation with them on.
- cloth, in either format: one morph target per frame on every cloth node (each level of detail simulated on its
  own), built through the inverse of the vertex's skin at that frame, so a vertex with `_PIN` 1 has no delta.
- `vertex` - the chain joints are taken out of the swinging meshes' skin (each vertex keeps its body weights,
  renormalised, or its chain's `parent_bone`), and every frame becomes one morph target of those meshes (`POSITION` and
  `NORMAL` deltas, named `<animation>_<frame>`), played by a `weights` track that blends from one frame's target to the
  next. The chain joints stay in the skin but drive no vertex. Larger: one target per frame per level of detail.

The solver is VRM's: each joint's tail moves by its inertia times (1 - drag), by stiffness toward its rest direction
and by gravity down in the world, keeps its length, and is pushed out of each capsule by the capsule's radius plus the
hit radius; it steps at 60 Hz or finer and starts from a one-second settle at the clip's first pose. Godot's
`SpringBoneSimulator3D` given the same file settles every chain tip where the bake does: to 0.001 mm with the capsules
off, within 0.03 mm (MakeHuman's ponytail) and 0.4-1.6 mm (its long hair, resting on the back) with them on.


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

- Baked cloth's levels of detail are simulated separately and can differ slightly from one another.
- Chains always have four segments; no per-chain stiffness yet.
- A baked animation does not loop exactly: the hair ends where the motion left it (a few degrees from where it began).
- No per-cloth stiffness, mass or damping: the solver's defaults apply.
- Colliders are capsules only and do not collide with each other's cloth (no self-collision data).
