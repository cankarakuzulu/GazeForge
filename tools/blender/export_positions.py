import sys
import bpy
import numpy as np

out = sys.argv[sys.argv.index("--") + 1]
head = bpy.data.objects["Head"]
world = np.array(head.matrix_world)
local = np.zeros(len(head.data.vertices) * 3)
head.data.vertices.foreach_get("co", local)
positions = (local.reshape(-1, 3) @ world[:3, :3].T + world[:3, 3]).astype(np.float32)
np.save(out, positions)
print(f"GFPOS {len(positions):,} vertices from {bpy.data.filepath} -> {out}", flush=True)
