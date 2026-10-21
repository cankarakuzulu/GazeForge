# GAZEFORGE_physics

A glTF 2.0 extension that says which meshes of a character are cloth and what the body they move against looks like, so
any engine can simulate them with its own solver. Nothing in the file moves by itself: a reader that does not know the
extension shows the wearables skinned to the body, as exported.

Status: version 1. Written by GazeForge's `export-glb` for wearables worn with `"physics": "cloth"`. No reader ships yet.

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

## Colliders

Capsules for the body, one per bone (fingers and toes left out), in the Character root's space at the rest pose:

- `joint` - the index of the bone in the skin's `joints` (the order of the rig's bones), `bone` its name.
- `head`, `tail` - the capsule's segment, the bone's own head and tail on this character.
- `radius` - the median distance from that segment of the body and head vertices the bone drives most (its strongest
  weight), so a heavy character gets fat capsules and a thin one thin capsules.
- `from_vertices` - how many vertices the radius was measured over.

A reader attaches each capsule to its joint and moves it with the skeleton.

## Limits (version 1)

- Cloth only: no spring-bone chains (ropes, chains, hair strands) and no baked animation yet.
- No per-cloth stiffness, mass or damping: the solver's defaults apply.
- Colliders are capsules only and do not collide with each other's cloth (no self-collision data).
