import sys
import bpy

args = sys.argv[sys.argv.index("--") + 1:]
OBJ, OUT, SWAP = args[0], args[1], len(args) > 2 and args[2] == "swap"
bpy.ops.wm.obj_import(filepath=OBJ, forward_axis="Y", up_axis="Z")
low = bpy.context.selected_objects[0]
low.name = "GameHead"
low.data.uv_layers[0].name = "head"
for polygon in low.data.polygons:
    polygon.use_smooth = True
if SWAP:
    high = bpy.data.objects["Head"]
    material = high.data.materials[:]
    bpy.data.objects.remove(high, do_unlink=True)
    low.name = "Head"
    for m in material:
        low.data.materials.append(m)
print(f"GFIMPORT {len(low.data.vertices):,} vertices, {len(low.data.polygons):,} faces as {low.name} -> {OUT}", flush=True)
bpy.ops.wm.save_as_mainfile(filepath=OUT, relative_remap=True)
