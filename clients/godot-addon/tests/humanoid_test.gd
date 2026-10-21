extends SceneTree

# usage: godot --headless --path clients/godot-addon --script res://tests/humanoid_test.gd -- CHARACTER.glb...
# Each CHARACTER.glb is an export-glb character with --body-rig PRESET (made by the exporter, not shipped).
# A path outside the project is read at run time: the humanoid extension must attach a BoneMap whose required profile
# bones are bones of the skeleton, the Left / Right bones must sit on the character's left / right (it faces +Z, so
# its left is +X), and the map shipped as addons/gazeforge_humanoid/bone_maps/<preset>.tres must equal it.
# A res:// path is loaded as the editor imported it with that shipped map as its retarget bone map (Godot's own
# retargeter): the skeleton's bones must then carry the profile names, and an animation of LeftUpperArm made on those
# names must move the left hand and leave the right one still.

const Extension := preload("res://addons/gazeforge_humanoid/gazeforge_humanoid_extension.gd")
const REQUIRED := ["Hips", "Spine", "Head", "LeftShoulder", "LeftUpperArm", "LeftLowerArm", "LeftHand", "RightShoulder", "RightUpperArm",
	"RightLowerArm", "RightHand", "LeftUpperLeg", "LeftLowerLeg", "LeftFoot", "RightUpperLeg", "RightLowerLeg", "RightFoot"]


func _initialize() -> void:
	run.call_deferred()


func run() -> void:
	var failed := 0
	for glb in OS.get_cmdline_user_args():
		var problems: PackedStringArray
		if glb.begins_with("res://"):
			problems = await check_retargeted(glb)
		else:
			problems = check_runtime(glb)
		print("%s %s %s" % ["PASS" if problems.is_empty() else "FAIL", glb.get_file(), "; ".join(problems)])
		if not problems.is_empty():
			failed += 1
	print("HUMANOID %s: %d file(s) failed" % ["PASS" if failed == 0 else "FAIL", failed])
	quit(1 if failed else 0)


func check_runtime(glb: String) -> PackedStringArray:
	var problems := PackedStringArray()
	var ext := Extension.new()
	GLTFDocument.register_gltf_document_extension(ext)
	var doc := GLTFDocument.new()
	var state := GLTFState.new()
	doc.append_from_file(glb, state)
	var root: Node = doc.generate_scene(state)
	GLTFDocument.unregister_gltf_document_extension(ext)
	var preset := str(state.json["extensions"][Extension.EXTENSION]["preset"])
	var sk := find_meta(root)
	if sk == null:
		root.free()
		return PackedStringArray(["no skeleton carries %s" % Extension.META])
	var map: BoneMap = sk.get_meta(Extension.META)
	for p in REQUIRED:
		if sk.find_bone(map.get_skeleton_bone_name(p)) < 0:
			problems.append("%s -> '%s' is not a bone" % [p, map.get_skeleton_bone_name(p)])
	var at := func(p: String) -> Vector3: return sk.get_bone_global_rest(sk.find_bone(map.get_skeleton_bone_name(p))).origin
	var mid: Vector3 = at.call("Hips")
	for pair in [["LeftHand", "RightHand"], ["LeftFoot", "RightFoot"], ["LeftUpperArm", "RightUpperArm"]]:
		if not (at.call(pair[0]).x > mid.x and at.call(pair[1]).x < mid.x):
			problems.append("%s at x %.3f and %s at x %.3f: not on the character's left and right" % [pair[0], at.call(pair[0]).x, pair[1], at.call(pair[1]).x])
	var shipped_path := "res://addons/gazeforge_humanoid/bone_maps/%s.tres" % preset
	if not ResourceLoader.exists(shipped_path):
		problems.append("no shipped map %s" % shipped_path)
	else:
		var shipped: BoneMap = load(shipped_path)
		var profile := SkeletonProfileHumanoid.new()
		for i in profile.bone_size:
			var p := profile.get_bone_name(i)
			if shipped.get_skeleton_bone_name(p) != map.get_skeleton_bone_name(p):
				problems.append("shipped %s: %s -> '%s', the file says '%s'" % [preset, p, shipped.get_skeleton_bone_name(p), map.get_skeleton_bone_name(p)])
	root.free()
	return problems


func check_retargeted(glb: String) -> PackedStringArray:
	var problems := PackedStringArray()
	var scene = load(glb)
	if not scene is PackedScene:
		return PackedStringArray(["did not load"])
	var root: Node = (scene as PackedScene).instantiate()
	get_root().add_child(root)
	await process_frame
	var sk := find_skeleton(root)
	for p in REQUIRED:
		if sk.find_bone(p) < 0:
			problems.append("no bone named %s after retargeting" % p)
	if not problems.is_empty():
		root.queue_free()
		return problems
	var anim := Animation.new()
	anim.length = 1.0
	var track := anim.add_track(Animation.TYPE_ROTATION_3D)
	anim.track_set_path(track, NodePath("%s:LeftUpperArm" % String(root.get_path_to(sk))))
	var rest := sk.get_bone_rest(sk.find_bone("LeftUpperArm")).basis.get_rotation_quaternion()
	anim.rotation_track_insert_key(track, 0.0, rest)
	anim.rotation_track_insert_key(track, 1.0, rest * Quaternion(Vector3.FORWARD, deg_to_rad(60)))
	var lib := AnimationLibrary.new()
	lib.add_animation("raise", anim)
	var player := AnimationPlayer.new()
	root.add_child(player)
	player.add_animation_library("", lib)
	var hand := func(n: String) -> Vector3: return sk.get_bone_global_pose(sk.find_bone(n)).origin
	var left0: Vector3 = hand.call("LeftHand")
	var right0: Vector3 = hand.call("RightHand")
	player.play("raise")
	player.seek(1.0, true)
	await process_frame
	var moved_left: float = hand.call("LeftHand").distance_to(left0)
	var moved_right: float = hand.call("RightHand").distance_to(right0)
	print("  retargeted: LeftUpperArm turned 60 deg by an animation on profile names moved LeftHand %.1f mm, RightHand %.3f mm" % [moved_left * 1000.0, moved_right * 1000.0])
	if moved_left < 0.1:
		problems.append("the left hand moved only %.1f mm" % (moved_left * 1000.0))
	if moved_right > 0.001:
		problems.append("the right hand moved %.2f mm" % (moved_right * 1000.0))
	root.queue_free()
	return problems


func find_meta(n: Node) -> Skeleton3D:
	if n is Skeleton3D and n.has_meta(Extension.META):
		return n
	for c in n.get_children():
		var s := find_meta(c)
		if s != null:
			return s
	return null


func find_skeleton(n: Node) -> Skeleton3D:
	if n is Skeleton3D:
		return n
	for c in n.get_children():
		var s := find_skeleton(c)
		if s != null:
			return s
	return null
