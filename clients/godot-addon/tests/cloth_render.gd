extends SceneTree

# usage: godot --fixed-fps 60 --path clients/godot-addon --script res://tests/cloth_render.gd -- CHARACTER.glb OUT_DIR
# Renders a character wearing a cloth item (Forward+, a window): side and back views at rest after the cloth settles,
# then with the spine bent 35 degrees back and forward (ramped over half a second, held a second). A look for an eye,
# not a gate: cloth_test.gd is the gate.

const Extension := preload("res://addons/gazeforge_physics/gazeforge_physics_extension.gd")


func _initialize() -> void:
	RenderingServer.set_default_clear_color(Color(0.11, 0.11, 0.12, 1))
	run.call_deferred()


func shot(path: String) -> void:
	await RenderingServer.frame_post_draw
	root.get_texture().get_image().save_png(path)


func run() -> void:
	var args := OS.get_cmdline_user_args()
	var glb: String = args[0]
	var out: String = args[1]
	DirAccess.make_dir_recursive_absolute(out)
	root.size = Vector2i(900, 900)
	var ext := Extension.new()
	GLTFDocument.register_gltf_document_extension(ext)
	var doc := GLTFDocument.new()
	var state := GLTFState.new()
	doc.append_from_file(glb, state)
	var scene: Node = doc.generate_scene(state)
	GLTFDocument.unregister_gltf_document_extension(ext)
	root.add_child(scene)
	var sun := DirectionalLight3D.new()
	sun.rotation_degrees = Vector3(-40, 35, 0)
	root.add_child(sun)
	var fill := DirectionalLight3D.new()
	fill.rotation_degrees = Vector3(-20, -150, 0)
	fill.light_energy = 0.5
	root.add_child(fill)
	var env := WorldEnvironment.new()
	env.environment = Environment.new()
	env.environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	env.environment.ambient_light_color = Color(0.45, 0.45, 0.5)
	root.add_child(env)
	var cam := Camera3D.new()
	cam.projection = Camera3D.PROJECTION_ORTHOGONAL
	cam.size = 1.3
	root.add_child(cam)
	var views := {"side": [Vector3(3, 1.05, 0), Vector3(0, 1.05, 0)], "back": [Vector3(0, 1.05, -3), Vector3(0, 1.05, 0)],
			"three_quarter": [Vector3(2.1, 1.3, -2.1), Vector3(0, 1.05, 0)]}
	var skeleton := find_skeleton(scene)
	var bone := skeleton.find_bone("spine_01")
	var rest := skeleton.get_bone_pose_rotation(bone)
	for k in 150:
		await physics_frame
	for v in views:
		cam.look_at_from_position(views[v][0], views[v][1])
		await shot(out.path_join("cape_rest_%s.png" % v))
	for deg in [35.0, -35.0]:
		for k in 90:
			skeleton.set_bone_pose_rotation(bone, rest * Quaternion(Vector3.RIGHT, deg_to_rad(deg) * minf(1.0, float(k + 1) / 30.0)))
			await physics_frame
		for v in ["side", "three_quarter"]:
			cam.look_at_from_position(views[v][0], views[v][1])
			await shot(out.path_join("cape_bend%+d_%s.png" % [int(deg), v]))
		skeleton.set_bone_pose_rotation(bone, rest)
		for k in 120:
			await physics_frame
	print("RENDER done")
	quit()


func find_skeleton(n: Node) -> Skeleton3D:
	if n is Skeleton3D:
		return n
	for c in n.get_children():
		var s := find_skeleton(c)
		if s != null:
			return s
	return null
