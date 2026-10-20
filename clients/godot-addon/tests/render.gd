extends SceneTree

const Extension := preload("res://addons/gazeforge_skin/gazeforge_skin_extension.gd")
const SkinApi := preload("res://addons/gazeforge_skin/gazeforge_skin.gd")
const Checks := preload("res://tests/skin_checks.gd")

var assets := "res://tests/assets"
var out_dir := ""
var report := {"shots": [], "problems": []}


func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	if args.size() > 0:
		assets = args[0]
	if args.size() > 1:
		out_dir = args[1]
	DirAccess.make_dir_recursive_absolute(out_dir)
	RenderingServer.set_default_clear_color(Color(0, 0, 0, 1))
	run.call_deferred()


func load_scene(debug: String) -> Node3D:
	var ext := Extension.new()
	ext.debug_mode = debug
	GLTFDocument.register_gltf_document_extension(ext)
	var doc := GLTFDocument.new()
	var state := GLTFState.new()
	var err := doc.append_from_file(assets.path_join("skin_valid.glb"), state)
	GLTFDocument.unregister_gltf_document_extension(ext)
	if err != OK:
		report["problems"].append("load failed %d" % err)
		return null
	var scene: Node3D = doc.generate_scene(state)
	scene.find_child("QuadCopy", true, false).visible = false
	scene.find_child("Pair", true, false).visible = false
	root.add_child(scene)
	return scene


func apply(scene: Node, values: Dictionary, defaults: Dictionary) -> void:
	for p in defaults:
		if SkinApi.set_parameter(scene, p, float(values.get(p, defaults[p]))) != 1:
			report["problems"].append("set_parameter %s did not hit exactly one material" % p)


func shot(name: String) -> void:
	for i in 4:
		await RenderingServer.frame_post_draw
	var img := root.get_texture().get_image()
	var path := out_dir.path_join(name + ".png")
	img.save_png(path)
	report["shots"].append({"name": name, "path": path, "size": [img.get_width(), img.get_height()]})


func run() -> void:
	var cfg: Dictionary = Checks.read_json(assets.path_join("render_configs.json"))
	var defaults: Dictionary = cfg["defaults"]
	report["window_size"] = [root.size.x, root.size.y]
	report["renderer"] = RenderingServer.get_current_rendering_method()
	report["adapter"] = RenderingServer.get_video_adapter_name()
	var cam := Camera3D.new()
	cam.projection = Camera3D.PROJECTION_ORTHOGONAL
	cam.size = 2.0
	cam.keep_aspect = Camera3D.KEEP_HEIGHT
	cam.position = Vector3(0, 0, 5)
	root.add_child(cam)
	cam.make_current()
	for mode in ["albedo", "roughness"]:
		var scene := load_scene(mode)
		if scene == null:
			break
		if mode == "albedo":
			var mat: ShaderMaterial = Checks.surfaces(scene)["skin"][0]
			report["renderer_defaults"] = {}
			for p in defaults:
				report["renderer_defaults"][p] = RenderingServer.shader_get_parameter_default(mat.shader.get_rid(), p)
				var d = report["renderer_defaults"][p]
				if d == null or absf(float(d) - float(defaults[p])) > 1e-7:
					report["problems"].append("renderer default of %s is %s, expected %s" % [p, d, defaults[p]])
		for c in cfg["configs"]:
			apply(scene, c["values"], defaults)
			await shot("%s_%s" % [mode, c["name"]])
		scene.queue_free()
		await process_frame
	var lit := load_scene("")
	if lit != null:
		var env := WorldEnvironment.new()
		var e := Environment.new()
		e.background_mode = Environment.BG_COLOR
		e.background_color = Color(0.05, 0.05, 0.06)
		e.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
		e.ambient_light_color = Color(0.35, 0.35, 0.38)
		e.ambient_light_energy = 0.6
		e.tonemap_mode = Environment.TONE_MAPPER_FILMIC
		env.environment = e
		root.add_child(env)
		var sun := DirectionalLight3D.new()
		sun.rotation_degrees = Vector3(-35, 40, 0)
		sun.light_energy = 2.0
		root.add_child(sun)
		cam.projection = Camera3D.PROJECTION_PERSPECTIVE
		cam.fov = 40.0
		cam.position = Vector3(1.4, 0.9, 3.3)
		cam.look_at(Vector3.ZERO)
		for name in ["c0_defaults", "c2_most", "c4_mixed"]:
			for c in cfg["configs"]:
				if c["name"] == name:
					apply(lit, c["values"], defaults)
			await shot("lit_" + name)
	var f := FileAccess.open(out_dir.path_join("render_report.json"), FileAccess.WRITE)
	f.store_string(JSON.stringify(report, " "))
	f.close()
	print("RENDER done, %d shots, %d problems: %s" % [report["shots"].size(), report["problems"].size(), report["problems"]])
	quit(0 if report["problems"].is_empty() else 1)
