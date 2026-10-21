# gazeforge_physics (Godot 4 addon)

Reads the `GAZEFORGE_physics` glTF extension: the spring-bone chains GazeForge builds for hair, ropes and capes worn
with `"physics": "springbone"`, and the character's capsule colliders. Format:
[GAZEFORGE_physics](../../../../extensions/GAZEFORGE_physics/README.md).

On import, the addon adds a `SpringBoneSimulator3D` named `GazeForgeSprings` under the character's `Skeleton3D`: one
setting per chain, from its root joint to its end joint, with the file's `spring_defaults` (stiffness, drag, gravity,
hit radius), and one `SpringBoneCollisionCapsule3D` child per collider, attached to its bone. Every chain collides with
every capsule. Nothing else in the scene changes; a character without the extension imports exactly as before.

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
`--fixed-fps 60` makes the run deterministic.
