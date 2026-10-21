extends SceneTree

# usage: godot --headless --fixed-fps 60 --path clients/godot-addon --script res://tests/cloth_test.gd -- CHARACTER.glb [REPORT.json]
# CHARACTER.glb is an export-glb character wearing an item with "physics": "cloth" (made by the exporter, not shipped).
# A res:// path is loaded as the editor imported it. Passes when every cloth item became a GazeForgeCloth whose pinned
# points are exactly the vertices with _PIN >= 0.5 in the file, the pinned points sit on their skinned positions at rest
# (within 1 mm of the rest vertex, an independent target) and while the spine bends (within 1 mm of the skin), the free
# cloth stays finite and hangs (no point further than 0.5 m from rest), and with the capsule bodies the free points
# reach less than half as deep into the body as with the bodies removed whenever the bend pushes them in (by more than
# 5 mm). Bends ramp over half a second (a bend in one frame teleports the capsules through the cloth); free points
# within 10 cm of a pin are left out of the depth, since the pins hold them. The scene is read only after a frame:
# a node added in _initialize is not in the tree until then, and its global transforms read as identity.

const Extension := preload("res://addons/gazeforge_physics/gazeforge_physics_extension.gd")
const Cloth := preload("res://addons/gazeforge_physics/gazeforge_cloth.gd")
const SETTLE := 120
const BEND_FRAMES := 90
const BEND_DEG := 35.0
const BEND_BONE := "spine_01"
const PIN_ZONE_M := 0.1
const RAMP_FRAMES := 30.0


func _initialize() -> void:
	run()


func run() -> void:
	var args := OS.get_cmdline_user_args()
	var glb: String = args[0] if args.size() > 0 else ""
	var report_path: String = args[1] if args.size() > 1 else ""
	var problems := PackedStringArray()
	var report := {"glb": glb}
	var file := read_glb(glb)
	var root: Node = null
	if glb.begins_with("res://"):
		report["path"] = "editor import"
		var scene = load(glb)
		root = (scene as PackedScene).instantiate() if scene is PackedScene else null
	else:
		report["path"] = "runtime GLTFDocument"
		var ext := Extension.new()
		GLTFDocument.register_gltf_document_extension(ext)
		var doc := GLTFDocument.new()
		var state := GLTFState.new()
		if doc.append_from_file(glb, state) == OK:
			root = doc.generate_scene(state)
		GLTFDocument.unregister_gltf_document_extension(ext)
	if root == null or file.is_empty():
		finish(report, PackedStringArray(["%s did not load" % glb]), report_path)
		return
	get_root().add_child(root)
	await process_frame
	var entries: Array = file["json"]["extensions"]["GAZEFORGE_physics"].get("cloth", [])
	var cloths := []
	collect(root, cloths)
	report["cloth_items"] = cloths.size()
	if cloths.size() != entries.size() or cloths.is_empty():
		problems.append("%d cloth nodes for %d cloth items" % [cloths.size(), entries.size()])
		finish(report, problems, report_path)
		return
	var cloth: Cloth = cloths[0]
	var pin := attribute(file, str(entries[0]["nodes"][0]), "_PIN")
	var want := PackedInt32Array()
	for i in pin.size():
		if pin[i] >= 0.5:
			want.append(i)
	report["pinned"] = cloth.pinned.size()
	report["vertices"] = pin.size()
	if cloth.pinned != want:
		problems.append("pinned %d points, the file pins %d" % [cloth.pinned.size(), want.size()])
	var skeleton := cloth.get_node(cloth.skeleton_path) as Skeleton3D
	var original := root.find_child(str(entries[0]["nodes"][0]), true, false) as MeshInstance3D
	var verts: PackedVector3Array = cloth.cloth_mesh.surface_get_arrays(0)[Mesh.ARRAY_VERTEX]
	var rest := PackedVector3Array()
	var to_world := original.global_transform
	report["original_hidden"] = not original.visible
	for v in verts:
		rest.append(to_world * v)
	for k in SETTLE:
		await physics_frame
	var at_rest := 0.0
	for i in cloth.pinned:
		at_rest = maxf(at_rest, cloth.get_point_transform(i).distance_to(rest[i]))
	report["pinned_off_rest_mm"] = at_rest * 1000.0
	if at_rest > 0.001:
		problems.append("at rest a pinned point sits %.2f mm from its vertex" % (at_rest * 1000.0))
	var hang := 0.0
	var finite := true
	for i in verts.size():
		var p := cloth.get_point_transform(i)
		finite = finite and p.is_finite()
		hang = maxf(hang, p.distance_to(rest[i]))
	report["free_moved_from_rest_mm"] = hang * 1000.0
	if not finite or hang > 0.5:
		problems.append("the cloth did not hang: furthest point %.0f mm from rest, finite %s" % [hang * 1000.0, finite])
	var bends := {}
	for deg in [BEND_DEG, -BEND_DEG]:
		var with_bodies: Dictionary = await bend(cloth, skeleton, deg)
		bends["%+d" % int(deg)] = {"with_bodies": with_bodies}
	var bodies := cloth.get_children().filter(func(n): return n is AnimatableBody3D)
	for b in bodies:
		cloth.remove_child(b)
	for deg in [BEND_DEG, -BEND_DEG]:
		bends["%+d" % int(deg)]["without_bodies"] = await bend(cloth, skeleton, deg)
	for b in bodies:
		cloth.add_child(b)
	report["bends"] = bends
	for key in bends:
		var on: Dictionary = bends[key]["with_bodies"]
		var off: Dictionary = bends[key]["without_bodies"]
		if on["pinned_off_skin_mm"] > 1.0:
			problems.append("bending %s a pinned point is %.2f mm off its skinned position" % [key, on["pinned_off_skin_mm"]])
		if off["deepest_mm"] > 5.0 and on["deepest_mm"] > off["deepest_mm"] * 0.5:
			problems.append("bending %s the cloth reaches %.1f mm into the body with the capsule bodies and %.1f mm without" % [key, on["deepest_mm"], off["deepest_mm"]])
	finish(report, problems, report_path)


func bend(cloth: Cloth, skeleton: Skeleton3D, deg: float) -> Dictionary:
	var bone := skeleton.find_bone(BEND_BONE)
	var rest_rotation := skeleton.get_bone_pose_rotation(bone)
	var deepest := 0.0
	var off_skin := 0.0
	var free := []
	var pinned_set := {}
	for i in cloth.pinned:
		pinned_set[i] = true
	var verts: PackedVector3Array = cloth.cloth_mesh.surface_get_arrays(0)[Mesh.ARRAY_VERTEX]
	for i in verts.size():
		if pinned_set.has(i):
			continue
		var near_pin := false
		for j in cloth.pinned:
			if verts[i].distance_to(verts[j]) < PIN_ZONE_M:
				near_pin = true
				break
		if not near_pin:
			free.append(i)
	var scale := skeleton.global_transform.basis.get_scale().x
	var start := {}
	for i in free:
		start[i] = depth(cloth, skeleton, scale, cloth.get_point_transform(i))
	var where := ""
	for k in BEND_FRAMES:
		skeleton.set_bone_pose_rotation(bone, rest_rotation * Quaternion(Vector3.RIGHT, deg_to_rad(deg) * minf(1.0, float(k + 1) / RAMP_FRAMES)))
		await physics_frame
		for n in cloth.pinned.size():
			off_skin = maxf(off_skin, cloth.get_point_transform(cloth.pinned[n]).distance_to(cloth.skinned_target(n)))
		for i in free:
			var d: float = depth(cloth, skeleton, scale, cloth.get_point_transform(i)) - maxf(start[i], 0.0)
			if d > deepest:
				deepest = d
				where = "point %d" % i
	skeleton.set_bone_pose_rotation(bone, rest_rotation)
	for k in SETTLE:
		await physics_frame
	return {"deepest_mm": deepest * 1000.0, "where": where, "pinned_off_skin_mm": off_skin * 1000.0}


func depth(cloth: Cloth, skeleton: Skeleton3D, scale: float, p: Vector3) -> float:
	var deepest := -INF
	for c in cloth.colliders:
		var w := skeleton.global_transform * skeleton.get_bone_global_pose(skeleton.find_bone(str(c["bone"])))
		var h: Vector3 = w * c["head"]
		var t: Vector3 = w * c["tail"]
		var s := clampf((p - h).dot(t - h) / maxf((t - h).length_squared(), 1e-12), 0.0, 1.0)
		deepest = maxf(deepest, float(c["radius"]) * scale - p.distance_to(h + (t - h) * s))
	return deepest


func collect(n: Node, out: Array) -> void:
	if n is Cloth:
		out.append(n)
	for c in n.get_children():
		collect(c, out)


func read_glb(path: String) -> Dictionary:
	var bytes := FileAccess.get_file_as_bytes(path)
	if bytes.size() < 20 or bytes.decode_u32(0) != 0x46546C67:
		return {}
	var jl := bytes.decode_u32(12)
	var parsed = JSON.parse_string(bytes.slice(20, 20 + jl).get_string_from_utf8())
	return {"json": parsed, "bin": bytes.slice(20 + jl + 8)} if parsed is Dictionary else {}


func attribute(file: Dictionary, node_name: String, name: String) -> PackedFloat32Array:
	var json: Dictionary = file["json"]
	var out := PackedFloat32Array()
	for node in json["nodes"]:
		if str(node.get("name", "")) == node_name and node.has("mesh"):
			var acc: Dictionary = json["accessors"][int(json["meshes"][int(node["mesh"])]["primitives"][0]["attributes"][name])]
			var view: Dictionary = json["bufferViews"][int(acc["bufferView"])]
			var base := int(view.get("byteOffset", 0)) + int(acc.get("byteOffset", 0))
			for i in int(acc["count"]):
				out.append((file["bin"] as PackedByteArray).decode_float(base + i * int(view.get("byteStride", 4))))
	return out


func finish(report: Dictionary, problems: PackedStringArray, report_path: String) -> void:
	report["problems"] = problems
	report["ok"] = problems.is_empty()
	if report_path != "":
		var f := FileAccess.open(report_path, FileAccess.WRITE)
		f.store_string(JSON.stringify(report, " "))
		f.close()
	print(JSON.stringify(report))
	print("CLOTH %s" % ("PASS" if problems.is_empty() else "FAIL: " + "; ".join(problems)))
	quit(0 if problems.is_empty() else 1)
