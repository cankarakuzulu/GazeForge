# gazeforge_humanoid (Godot 4 addon)

Lets Godot retarget humanoid animation onto GazeForge characters. Format:
[GAZEFORGE_humanoid](../../../../extensions/GAZEFORGE_humanoid/README.md).

Tested with Godot 4.7.2, through the editor import (Godot's own retargeter) and through `GLTFDocument` at run time.

## What it gives you

- **On import**, a character exported with a rig preset gets a `BoneMap` (profile `SkeletonProfileHumanoid`) built from
  the file, attached to its `Skeleton3D` as the meta `gazeforge_bone_map`. Bone names are the ones Godot imported
  (Godot turns `mixamorig:Hips` into `mixamorig_Hips`; the addon reads the original names from the glTF state).
- **Ready maps per preset** in `bone_maps/` (`game_engine`, `game_engine_with_breast`, `mixamo`, `mixamo_unity`,
  `default`, `cmu_mb`), for Godot's retargeting.

## Retargeting in the editor

1. Copy `addons/gazeforge_humanoid` into your project and enable "GazeForge humanoid" (Project Settings > Plugins).
2. Double-click the character's `.glb`, select `Skeleton3D`, and under Retarget > Bone Map load
   `res://addons/gazeforge_humanoid/bone_maps/<preset>.tres` (the preset it was exported with: its sidecar
   `.gazeforge.json` says it under `body.humanoid.preset`). Reimport.
3. The skeleton's bones now carry the humanoid names (`Hips`, `LeftUpperArm`, ...), so animation libraries made for
   Godot's humanoid profile play on it.

The same setting in a `.import` file, if you prefer to write it:

```
_subresources={
"nodes": {
"PATH:Character/Skeleton3D": {
"retarget/bone_map": Resource("res://addons/gazeforge_humanoid/bone_maps/mixamo.tres")
}
}
}
```

## Tests

`tests/humanoid_test.gd` (not shipped characters: export them with `gazeforge export-glb --body-rig <preset>`):

```
godot --headless --path clients/godot-addon --script res://tests/humanoid_test.gd -- CHARACTER.glb...
```

A path outside the project checks the run-time import: the attached map covers every required profile bone with a real
bone, Left / Right bones sit on the character's left / right, and the shipped map equals the one built from the file.
A `res://` path checks a character imported with the shipped map as its retarget bone map: the bones carry the profile
names, and an animation of `LeftUpperArm` made on those names moves the left hand and leaves the right one still.
`tests/make_bone_maps.gd` remakes `bone_maps/` from exported characters.
