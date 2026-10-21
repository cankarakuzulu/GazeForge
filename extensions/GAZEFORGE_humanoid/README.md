# GAZEFORGE_humanoid

A glTF 2.0 extension that says which bone of a character's skeleton is which humanoid bone, so an engine can retarget
humanoid animation onto it without a hand-made mapping. A reader that does not know the extension sees the skeleton
exactly as exported.

Status: version 1. Written by GazeForge's `export-glb` when the body is rigged with a rig preset that has a humanoid
map (`--body-rig game_engine`, `game_engine_with_breast`, `mixamo`, `mixamo_unity`, `default`, `cmu_mb`; `openpose` is a
joint set, not a skeleton, and has none). Readers: the Godot 4 addon `clients/godot-addon/addons/gazeforge_humanoid`.

## Placement

At the root of the document, under `extensions.GAZEFORGE_humanoid`; listed in `extensionsUsed`, never in
`extensionsRequired`.

```json
{
  "extensions": {
    "GAZEFORGE_humanoid": {
      "version": 1,
      "preset": "mixamo",
      "godot": {"Hips": "mixamorig:Hips", "Spine": "mixamorig:Spine", "LeftUpperArm": "mixamorig:LeftArm"},
      "unity": {"Hips": "mixamorig:Hips", "LeftThumbProximal": "mixamorig:LeftHandThumb1"}
    }
  }
}
```

- `godot` - Godot's `SkeletonProfileHumanoid` bone name to the name of a skin joint node of this file.
- `unity` - Unity's humanoid (`HumanBodyBones`) name to the same. It is the `godot` table with the thumb named one step
  later (Godot `ThumbMetacarpal` / `ThumbProximal` / `ThumbDistal` are Unity `ThumbProximal` / `ThumbIntermediate` /
  `ThumbDistal`) and without `Root`, which Unity's humanoid does not have.

Only bones the preset has are listed; every map has all the profile's required bones (hips, spine, head, both
shoulders, upper and lower arms, hands, upper and lower legs and feet). Names are the glTF node names as written; an
engine that renames nodes on import (Godot turns `mixamorig:Hips` into `mixamorig_Hips`) maps them through its own
original-name record.

## How the maps were chosen

From each preset's own hierarchy: a mapped bone is always under the bone its profile parent maps to, and every Left
bone sits on the character's left (checked in GazeForge's tests against the presets' rest positions). Bones a profile
has no place for (twist bones, MakeHuman's `shoulder01`, `metacarpal*`, `neck02`, the extra spine segments, the face
bones of the `default` preset) stay unmapped and keep following their parents. Choices a reader may want to know:

- `default`: Spine `spine05`, Chest `spine03`, UpperChest `spine01` (the clavicles' and the neck's parent); toes
  `toe1-1` (the big toe's first bone).
- `cmu_mb`: one thumb bone (`LThumb` as ThumbProximal) and one finger bone (`LeftHandFinger1` as IndexProximal).
