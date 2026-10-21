# gazeforge_physics (Godot 4 addon)

Reads the `GAZEFORGE_physics` glTF extension: the spring-bone chains GazeForge builds for hair, ropes and capes worn
with `"physics": "springbone"`, cloth worn with `"physics": "cloth"`, and the character's capsule colliders. Format:
[GAZEFORGE_physics](../../../../extensions/GAZEFORGE_physics/README.md).

On import, the addon adds a `SpringBoneSimulator3D` named `GazeForgeSprings` under the character's `Skeleton3D`: one
setting per chain, from its root joint to its end joint, with the file's `spring_defaults` (stiffness, drag, gravity,
hit radius), and one `SpringBoneCollisionCapsule3D` child per collider, attached to its bone. Every chain collides with
every capsule.

For each cloth item, the addon adds a `GazeForgeCloth` (a `SoftBody3D`) beside the item's finest mesh and hides that
mesh. Its pinned points are the vertices whose `_PIN` is at least 0.5; every physics frame it moves them to their
skinned positions (the vertex's own bone weights, up to four bones), so the pinned cloth follows the body exactly and the
rest hangs. The body's colliders become capsule `AnimatableBody3D`s the cloth collides with, moved with their bones.
Coarser levels of the item stay as imported (skinned, not simulated). The cloth renders both sides of its faces (a
duplicate of the item's material with culling off): simulated cloth folds and shows its back. Nothing else in the scene changes; a character
without the extension imports exactly as before.

Tested with Godot 4.7.2 (Forward+), through the editor import and through `GLTFDocument` at run time.

## Install

1. Copy `addons/gazeforge_physics` into your project's `addons/` folder.
2. Project > Project Settings > Plugins: enable "GazeForge physics".
3. Import (or reimport) your `.glb` files.

## Using it

The chains move on their own while the skeleton animates. Tune them on the `GazeForgeSprings` node (per setting:
stiffness, drag, gravity, radius), or turn the simulator off with `active`. To read a chain's moved pose from a script,
read it inside `Skeleton3D.skeleton_updated`: a skeleton modifier's result exists only during the skeleton's update and
is restored afterwards, so a read at any other time sees the pose without the springs.

## Things to know (measured on Godot 4.7.2)

- `SpringBoneCollision3D` takes its offset, radius and length in world units, rotated by the bone but not scaled by the
  skeleton, and the simulator's joint radius likewise. GazeForge characters carry their size as a scale on the
  `Character` root, so the addon multiplies every length by the skeleton's scale in the imported scene. If you scale the
  character again in your game, scale the capsules and the setting radii with it.
- Turning a setting's "all child collisions" off at run time does not take effect until the simulator's children
  change; remove a capsule node to take it out.
- The simulator and the capsules are given their bone indices as well as their names: set by name only while the
  scene is being built, they never resolve, and the editor import then saves capsules without their offsets.
- A `SoftBody3D` builds its points from its mesh and its global transform only when it enters the world. A mesh
  assigned while the node is outside the tree logs an error per vertex; one assigned once it is inside leaves it with
  no points. So the cloth keeps its mesh in `cloth_mesh`, takes it in `_ready` and enters the tree again. It also
  resets its own transform to keep its points in world space, so its place under the skeleton is kept in
  `mesh_transform`.
- Soft-body point *i* is glTF vertex *i*: no welding, no reordering.
- A pin attached to a node with `set_point_pinned(i, true, path)` did not follow that node in our tests; the cloth
  drives its pins with `PhysicsServer3D.soft_body_move_point` instead. That costs a few bone transforms per pinned
  vertex per frame in GDScript; cloth that lies on the body almost everywhere (a suit) is better left skinned.
- The editor import hands `_import_post` `ImporterMeshInstance3D` nodes; a run-time `GLTFDocument` scene has
  `MeshInstance3D`. The addon reads either.

## Tests

`tests/physics_test.gd` checks an exported character (made by `gazeforge export-glb --body-rig ...` with a spring-bone
wearable; not shipped here):

```
godot --headless --fixed-fps 60 --path clients/godot-addon --script res://tests/physics_test.gd -- CHARACTER.glb [REPORT.json]
```

It passes when there is one setting per chain with the right root and end bones, one capsule per collider sitting on
its collider (within 1 mm), the chains swing when the head turns with the simulator on and follow it rigidly with it
off, and the capsules hold the chains out of the body: with them removed the joints go deeper than with them. A `res://`
path tests the editor import instead (copy the glb into the project and run `godot --headless --import` first).
`--fixed-fps 60` makes the run deterministic. Joints within 3 cm of their chain's root are left out of the depth (their
root drags them in when it enters a capsule); the report keeps their depth too.

`tests/cloth_test.gd` checks a character with a cloth item the same way:

```
godot --headless --fixed-fps 60 --path clients/godot-addon --script res://tests/cloth_test.gd -- CHARACTER.glb [REPORT.json]
```

It passes when each cloth item became a `GazeForgeCloth` pinning exactly the vertices with `_PIN` >= 0.5, the pinned
points sit on their vertices at rest (1 mm) and on their skinned positions while the spine bends (1 mm), the free cloth
stays finite and within 0.5 m of rest, and with the capsule bodies the free cloth reaches less than half as deep into
the body as with them removed (cloth within 10 cm of a pin is left out: the pins hold it). Bends ramp over half a
second; a bend in one frame teleports the capsules through the cloth.

`tests/cloth_render.gd` renders the same character (Forward+, a window, no `--headless`) at rest and bent, for an eye:
a gate cannot see a cape that is culled from behind, which is how the double-sided material above was found.
