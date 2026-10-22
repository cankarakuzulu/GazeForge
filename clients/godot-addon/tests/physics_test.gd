extends SceneTree

# usage: godot --headless --path clients/godot-addon --script res://tests/physics_test.gd -- CHARACTER.glb [REPORT.json]
# CHARACTER.glb is an export-glb character worn with "physics": "springbone" (not shipped here: it is made by the
# exporter). A path outside the project is read at run time with GLTFDocument; a res:// path is loaded as the editor
# imported it (run godot --headless --import first), which checks that the settings survive the saved scene. Passes when the import builds one SpringBoneSimulator3D setting per spring, each from its root to its end
# joint, one capsule per collider whose segment starts at its bone's rest head (the frames agree) and whose node sits on
# the collider's centre, when turning the first chain's parent bone makes the chain's tip lag behind it with the simulator
# on and follow it rigidly with it off, and when the capsules hold the chains: turning the head both ways, the joints go
# deeper into the body than at rest by less than half as much with the capsules as with them removed. Removed, not
# switched off: a setting's "all child collisions" turned off at run time does not take effect. The turns ramp over
# half a second (a turn in one frame teleports the capsules through the chains), and joints within 3 cm of their
# chain's root are left out of the depth (a root fixed to a bone that enters a capsule drags them in; the report keeps
# their depth as with_joints_near_a_root_mm).
# Each spring's collider_radii must become its own shrunk copy of that capsule (the original excluded for that chain,
# the copy for every other) and each setting's radius its hit_radius_m; settle_tip_mm reports how far each chain's tip
# moves from the rest pose with nothing moving, the number the bake's tip_settled_mm reproduces.
# The tip is sampled inside Skeleton3D.skeleton_updated: a SkeletonModifier3D's result exists only during that update and
# is restored afterwards, so a read from outside sees the unmodified pose and reports a still chain.

const Extension := preload("res://addons/gazeforge_physics/gazeforge_physics_extension.gd")
const TURN_DEG := 60.0
const SETTLE_FRAMES := 60
const SWING_FRAMES := 90
const CONTACT_MM := 5.0
const RAMP_FRAMES := 30.0
const ROOT_ZONE_M := 0.03


func _initialize() -> void:
	run()


func run() -> void:
	var args := OS.get_cmdline_user_args()
	var glb: String = args[0] if args.size() > 0 else ""
	var report_path: String = args[1] if args.size() > 1 else ""
	var problems := PackedStringArray()
	var report := {"glb": glb}
	var json := read_glb_json(glb)
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
	if root == null or json.is_empty():
		finish(report, PackedStringArray(["%s did not load" % glb]), report_path)
		return
	get_root().add_child(root)
	var phys: Dictionary = json["extensions"]["GAZEFORGE_physics"]
	var springs: Array = phys.get("springs", [])
	var names := Extension.joint_names(json)
	var sim := root.find_child(Extension.SIMULATOR, true, false) as SpringBoneSimulator3D
	if sim == null:
		finish(report, PackedStringArray(["no %s node after import" % Extension.SIMULATOR]), report_path)
		return
	var skeleton := sim.get_parent() as Skeleton3D
	report["settings"] = sim.setting_count
	report["world_scale"] = Extension.world_scale(root, skeleton)
	report["setting0"] = {"stiffness": sim.get_stiffness(0), "drag": sim.get_drag(0), "gravity": sim.get_gravity(0), "radius": sim.get_radius(0),
						  "root": sim.get_root_bone(0), "end": sim.get_end_bone(0), "joints": sim.get_joint_count(0),
						  "all_child_collisions": sim.are_all_child_collisions_enabled(0)}
	if sim.setting_count != springs.size():
		problems.append("%d settings for %d springs" % [sim.setting_count, springs.size()])
	for i in mini(sim.setting_count, springs.size()):
		var joints: Array = springs[i]["joints"]
		if sim.get_root_bone_name(i) != names[int(joints[0])] or sim.get_end_bone_name(i) != names[int(joints[joints.size() - 1])]:
			problems.append("setting %d runs %s..%s, the spring %s..%s" % [i, sim.get_root_bone_name(i), sim.get_end_bone_name(i), names[int(joints[0])], names[int(joints[joints.size() - 1])]])
	var colliders: Array = phys.get("colliders", [])
	var capsules := sim.get_children().filter(func(n): return n is SpringBoneCollisionCapsule3D and not str(n.name).contains("_chain"))
	report["colliders"] = capsules.size()
	if capsules.size() != colliders.size():
		problems.append("%d capsules for %d colliders" % [capsules.size(), colliders.size()])
	var to_skeleton := Extension.frame_to_skeleton(root, skeleton)
	var worst := 0.0
	for c in colliders:
		var bone := skeleton.find_bone(str(c["bone"]))
		if bone >= 0:
			worst = maxf(worst, skeleton.get_bone_global_rest(bone).origin.distance_to(to_skeleton * Extension.vec(c["head"])))
	report["collider_head_off_bone_mm"] = worst * 1000.0
	if worst > 0.001:
		problems.append("a collider head sits %.2f mm from its bone's rest head: the frames disagree" % (worst * 1000.0))
	for k in 5:
		await process_frame
	var placed := {"unresolved": PackedStringArray(), "worst_mm": 0.0, "worst": ""}
	var by_bone := {}
	for c in colliders:
		by_bone[str(c["bone"])] = c
	for cap in capsules:
		var c: Dictionary = by_bone.get(str(cap.bone_name), {})
		if cap.get_bone() < 0:
			placed["unresolved"].append(str(cap.bone_name))
		if c.is_empty():
			continue
		var want: Vector3 = skeleton.global_transform * (to_skeleton * ((Extension.vec(c["head"]) + Extension.vec(c["tail"])) * 0.5))
		var off: float = cap.global_position.distance_to(want) * 1000.0
		if off > placed["worst_mm"]:
			placed["worst_mm"] = off
			placed["worst"] = str(cap.bone_name)
	report["capsules_placed"] = placed
	if not placed["unresolved"].is_empty():
		problems.append("capsules with no bone: %s" % ", ".join(placed["unresolved"]))
	if placed["worst_mm"] > 1.0:
		problems.append("capsule %s sits %.1f mm from its collider's centre" % [placed["worst"], placed["worst_mm"]])
	var exclusions := []
	var hit_default: float = float(phys.get("spring_defaults", {}).get("hit_radius_m", 0.02))
	for i in springs.size():
		for r in springs[i].get("collider_radii", []):
			if int(r.get("collider", -1)) < 0 or int(r.get("collider", -1)) >= colliders.size():
				problems.append("spring %d shrinks collider %s, the file has %d" % [i, str(r.get("collider")), colliders.size()])
	if not problems.is_empty():
		finish(report, problems, report_path)
		return
	var own_of := []
	for i in springs.size():
		var own := []
		for r in springs[i].get("collider_radii", []):
			var name := "Collider_%s_chain%d" % [colliders[int(r["collider"])]["bone"], i]
			own.append(name)
			var node := sim.get_node_or_null(NodePath(name)) as SpringBoneCollisionCapsule3D
			if node == null:
				problems.append("no capsule %s for spring %d's shrunk collider" % [name, i])
			elif absf(node.radius - float(r["radius"]) * float(report["world_scale"])) > 1e-6:
				problems.append("%s has radius %.5f, the spring gives %.5f" % [name, node.radius, float(r["radius"]) * float(report["world_scale"])])
		own_of.append(own)
	for i in mini(sim.setting_count, springs.size()):
		var expect := []
		for r in springs[i].get("collider_radii", []):
			expect.append("Collider_%s" % colliders[int(r["collider"])]["bone"])
		for other in springs.size():
			if other != i:
				expect.append_array(own_of[other])
		expect.sort()
		var got := []
		for e in sim.get_exclude_collision_count(i):
			var path := sim.get_exclude_collision_path(i, e)
			var n := sim.get_node_or_null(path)
			got.append(str(n.name) if n != null else "unresolved %s" % path)
		var order := []
		for e in sim.get_exclude_collision_count(i):
			var n := sim.get_node_or_null(sim.get_exclude_collision_path(i, e))
			order.append(n.get_index() if n != null else -1)
		var ascending := order.duplicate()
		ascending.sort()
		if order != ascending:
			problems.append("setting %d lists its excluded capsules out of child order %s: the simulator walks its deny list once in child order and ignores such entries" % [i, str(order)])
		got.sort()
		if got != expect:
			problems.append("setting %d excludes %s, the springs make %s" % [i, str(got), str(expect)])
		var radius: float = float(springs[i].get("hit_radius_m", hit_default)) * float(report["world_scale"])
		if absf(sim.get_radius(i) - radius) > 1e-6:
			problems.append("setting %d has radius %.5f, the spring's hit radius makes %.5f" % [i, sim.get_radius(i), radius])
		exclusions.append({"setting": i, "excluded": got, "radius": sim.get_radius(i)})
	report["exclusions"] = exclusions
	report["settle_tip_mm"] = await settle(skeleton, sim, names, springs)
	if "no-colliders" in args:
		report["colliders_disabled"] = true
		for i in sim.setting_count:
			sim.set_enable_all_child_collisions(i, false)
	var depth := {}
	for turn in [TURN_DEG, -TURN_DEG]:
		var on_: Dictionary = await contact(skeleton, sim, names, springs, turn, true)
		var off_: Dictionary = await contact(skeleton, sim, names, springs, turn, false)
		var with_colliders: float = on_["mm"]
		var without: float = off_["mm"]
		depth["%+d" % int(turn)] = {"colliders_on_mm": with_colliders, "on_where": on_["where"], "colliders_off_mm": without, "off_where": off_["where"],
				"with_joints_near_a_root_mm": on_["all_joints_mm"]}
		if without > CONTACT_MM and with_colliders > without * 0.5:
			problems.append("turning %+d deg the chains go %.1f mm deeper into the body than at rest with colliders and %.1f mm without: the colliders do not hold them" % [int(turn), with_colliders, without])
	report["deepest_joint_in_a_capsule"] = depth
	var on := await swing(skeleton, sim, names, springs[0], true)
	var off := await swing(skeleton, sim, names, springs[0], false)
	report["swing_on"] = on
	report["swing_off"] = off
	for run_ in [on, off]:
		if run_["swing_samples"] < 1:
			problems.append("no skeleton update was seen after the turn (simulator %s): the swing was not observed" % ("on" if run_["active"] else "off"))
	if not on["finite"]:
		problems.append("the chain went non-finite with the simulator on")
	if on["lag_mm"] < 5.0:
		problems.append("with the simulator on the tip lags only %.2f mm: the chain does not swing" % on["lag_mm"])
	if off["lag_mm"] > 0.5:
		problems.append("with the simulator off the tip still moves %.2f mm against its parent: something else drives it" % off["lag_mm"])
	finish(report, problems, report_path)


func contact(skeleton: Skeleton3D, sim: SpringBoneSimulator3D, names: PackedStringArray, springs: Array, turn_deg: float, collide: bool) -> Dictionary:
	var capsules := sim.get_children().filter(func(n): return n is SpringBoneCollisionCapsule3D)
	if not collide:
		for cap in capsules:
			sim.remove_child(cap)
	sim.active = true
	await process_frame
	sim.reset()
	var bones := []
	var free := {}
	for s in springs:
		var root_at := skeleton.get_bone_global_rest(skeleton.find_bone(names[int(s["joints"][0])])).origin
		for j in s["joints"].slice(1):
			var b := skeleton.find_bone(names[int(j)])
			bones.append(b)
			free[b] = skeleton.get_bone_global_rest(b).origin.distance_to(root_at) >= ROOT_ZONE_M
	var scale := Extension.world_scale(get_root(), skeleton)
	var worst := {"depth": 0.0, "rest": {}, "capsule": "", "all_joints": 0.0}
	var measure := func() -> void:
		var inv := skeleton.global_transform.affine_inverse()
		for b in bones:
			var p := skeleton.get_bone_global_pose(b).origin
			var deepest := -INF
			var which := ""
			for cap in capsules:
				var rest := skeleton.get_bone_global_pose(cap.get_bone()) * Transform3D(Basis(cap.rotation_offset), cap.position_offset / scale)
				var axis := rest.basis.y.normalized()
				var half: float = cap.mid_height * 0.5 / scale
				var s := clampf((p - rest.origin).dot(axis), -half, half)
				var d: float = cap.radius / scale - p.distance_to(rest.origin + axis * s)
				if d > deepest:
					deepest = d
					which = str(cap.bone_name)
			if not worst["rest"].has(b):
				worst["rest"][b] = maxf(deepest, 0.0)
			worst["all_joints"] = maxf(worst["all_joints"], deepest - worst["rest"][b])
			if free[b] and deepest - worst["rest"][b] > worst["depth"]:
				worst["depth"] = deepest - worst["rest"][b]
				worst["capsule"] = "%s into %s" % [skeleton.get_bone_name(b), which]
	var parent := skeleton.get_bone_parent(skeleton.find_bone(names[int(springs[0]["joints"][0])]))
	var rest_rotation := skeleton.get_bone_pose_rotation(parent)
	for k in SETTLE_FRAMES:
		await process_frame
	skeleton.skeleton_updated.connect(measure)
	await process_frame
	for k in SWING_FRAMES:
		skeleton.set_bone_pose_rotation(parent, rest_rotation * Quaternion(Vector3.RIGHT, deg_to_rad(turn_deg) * minf(1.0, float(k + 1) / RAMP_FRAMES)))
		await process_frame
	skeleton.skeleton_updated.disconnect(measure)
	skeleton.set_bone_pose_rotation(parent, rest_rotation)
	for k in SETTLE_FRAMES:
		await process_frame
	if not collide:
		for cap in capsules:
			sim.add_child(cap)
	return {"mm": worst["depth"] * 1000.0, "where": worst["capsule"], "all_joints_mm": worst["all_joints"] * 1000.0}


func settle(skeleton: Skeleton3D, sim: SpringBoneSimulator3D, names: PackedStringArray, springs: Array) -> Array:
	sim.active = true
	sim.reset()
	var tips := []
	for s in springs:
		tips.append(skeleton.find_bone(names[int(s["joints"][s["joints"].size() - 1])]))
	var moved := []
	moved.resize(tips.size())
	moved.fill(0.0)
	var sample := func() -> void:
		for k in tips.size():
			var now := skeleton.global_transform * skeleton.get_bone_global_pose(tips[k]).origin
			var rest := skeleton.global_transform * skeleton.get_bone_global_rest(tips[k]).origin
			moved[k] = now.distance_to(rest) * 1000.0
	skeleton.skeleton_updated.connect(sample)
	for k in SETTLE_FRAMES:
		await process_frame
	skeleton.skeleton_updated.disconnect(sample)
	return moved


func swing(skeleton: Skeleton3D, sim: SpringBoneSimulator3D, names: PackedStringArray, spring: Dictionary, active: bool) -> Dictionary:
	sim.active = active
	sim.reset()
	var joints: Array = spring["joints"]
	var tip := skeleton.find_bone(names[int(joints[joints.size() - 1])])
	var parent := skeleton.get_bone_parent(skeleton.find_bone(names[int(joints[0])]))
	var rest_rotation := skeleton.get_bone_pose_rotation(parent)
	var seen := {"local": Vector3.ZERO, "samples": 0}
	var sample := func() -> void:
		seen["local"] = skeleton.get_bone_global_pose(parent).affine_inverse() * skeleton.get_bone_global_pose(tip).origin
		seen["samples"] += 1
	skeleton.skeleton_updated.connect(sample)
	for k in SETTLE_FRAMES:
		await process_frame
	var settled: Vector3 = seen["local"]
	skeleton.set_bone_pose_rotation(parent, rest_rotation * Quaternion(Vector3.RIGHT, deg_to_rad(TURN_DEG)))
	var lag := 0.0
	var finite := true
	var before: int = seen["samples"]
	for k in SWING_FRAMES:
		await process_frame
		var now: Vector3 = seen["local"]
		finite = finite and now.is_finite()
		lag = maxf(lag, now.distance_to(settled))
	var swing_samples: int = seen["samples"] - before
	skeleton.set_bone_pose_rotation(parent, rest_rotation)
	for k in SETTLE_FRAMES:
		await process_frame
	skeleton.skeleton_updated.disconnect(sample)
	return {"active": active, "parent": skeleton.get_bone_name(parent), "tip": skeleton.get_bone_name(tip), "lag_mm": lag * 1000.0, "finite": finite, "samples": seen["samples"], "swing_samples": swing_samples}


func read_glb_json(path: String) -> Dictionary:
	var bytes := FileAccess.get_file_as_bytes(path)
	if bytes.size() < 20 or bytes.decode_u32(0) != 0x46546C67:
		return {}
	var parsed = JSON.parse_string(bytes.slice(20, 20 + bytes.decode_u32(12)).get_string_from_utf8())
	return parsed if parsed is Dictionary else {}


func finish(report: Dictionary, problems: PackedStringArray, report_path: String) -> void:
	report["problems"] = problems
	report["ok"] = problems.is_empty()
	if report_path != "":
		var f := FileAccess.open(report_path, FileAccess.WRITE)
		f.store_string(JSON.stringify(report, " "))
		f.close()
	print(JSON.stringify(report))
	print("PHYSICS %s" % ("PASS" if problems.is_empty() else "FAIL: " + "; ".join(problems)))
	quit(0 if problems.is_empty() else 1)
