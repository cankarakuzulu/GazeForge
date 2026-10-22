extends SceneTree

# usage: godot --headless --path clients/godot-addon --script res://tests/face_on_body_test.gd -- CHARACTER.glb...
# Each CHARACTER.glb is an export-glb character with --body-rig PRESET and --rig face|face2 (made by the exporter, not
# shipped): one skeleton holding the body's bones and the face joints. Every animation track must name a bone of that
# skeleton; playing the jawOpen animation to its end must turn the jaw; turning the neck on top of it must carry the
# posed jaw along (the face rides the body).


func _initialize() -> void:
	run.call_deferred()


func run() -> void:
	var failed := 0
	for glb in OS.get_cmdline_user_args():
		var problems := check(glb)
		print("%s %s %s" % ["PASS" if problems.is_empty() else "FAIL", glb.get_file(), "; ".join(problems)])
		if not problems.is_empty():
			failed += 1
	print("FACE ON BODY %s: %d file(s) failed" % ["PASS" if failed == 0 else "FAIL", failed])
	quit(1 if failed else 0)


func find(node: Node, type: String) -> Node:
	if node.is_class(type):
		return node
	for child in node.get_children():
		var hit := find(child, type)
		if hit:
			return hit
	return null


func check(glb: String) -> PackedStringArray:
	var problems := PackedStringArray()
	var doc := GLTFDocument.new()
	var state := GLTFState.new()
	if doc.append_from_file(glb, state) != OK:
		return PackedStringArray(["the glb does not load"])
	var scene := doc.generate_scene(state)
	get_root().add_child(scene)
	var skeleton := find(scene, "Skeleton3D") as Skeleton3D
	var player := find(scene, "AnimationPlayer") as AnimationPlayer
	if skeleton == null or player == null:
		return PackedStringArray(["no skeleton or no animation player"])
	var tracks := 0
	for name in player.get_animation_list():
		var anim := player.get_animation(name)
		for t in anim.get_track_count():
			var path := anim.track_get_path(t)
			if path.get_subname_count() > 0:
				tracks += 1
				if skeleton.find_bone(path.get_subname(0)) < 0:
					problems.append("%s: track on a bone the skeleton lacks: %s" % [name, path])
	if tracks == 0:
		problems.append("no bone tracks at all")
	var jaw := skeleton.find_bone("jaw")
	var neck := skeleton.get_bone_parent(skeleton.find_bone("head"))
	if jaw < 0 or neck < 0 or not player.has_animation("jawOpen"):
		problems.append("no jaw bone, no neck above the head, or no jawOpen animation")
		return problems
	var rest := skeleton.get_bone_global_pose(jaw)
	player.play("jawOpen")
	player.seek(player.get_animation("jawOpen").length, true)
	player.advance(0)
	var opened := skeleton.get_bone_global_pose(jaw)
	var turn := rad_to_deg(rest.basis.get_rotation_quaternion().angle_to(opened.basis.get_rotation_quaternion()))
	if turn < 5.0:
		problems.append("jawOpen turns the jaw only %.2f deg" % turn)
	skeleton.set_bone_pose_rotation(neck, Quaternion(Vector3.RIGHT, deg_to_rad(30.0)) * skeleton.get_bone_pose_rotation(neck))
	var carried := skeleton.get_bone_global_pose(jaw)
	var moved := carried.origin.distance_to(opened.origin)
	var kept := rad_to_deg(opened.basis.get_rotation_quaternion().angle_to(carried.basis.get_rotation_quaternion()))
	if moved < 0.01:
		problems.append("turning the neck moves the opened jaw only %.4f m" % moved)
	print("  %s: %d bones, %d animations, %d bone tracks, jaw opens %.1f deg, a 30 deg neck turn moves it %.1f mm and turns it %.1f deg" % [
		glb.get_file(), skeleton.get_bone_count(), player.get_animation_list().size(), tracks, turn, moved * 1000.0, kept])
	scene.queue_free()
	return problems
