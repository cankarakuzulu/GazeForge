extends SceneTree

const Extension := preload("res://addons/gazeforge_skin/gazeforge_skin_extension.gd")
const SkinApi := preload("res://addons/gazeforge_skin/gazeforge_skin.gd")
const Checks := preload("res://tests/skin_checks.gd")
const ErrorLog := preload("res://tests/error_log.gd")


func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	var assets: String = args[0] if args.size() > 0 else "res://tests/assets"
	var report_path: String = args[1] if args.size() > 1 else ""
	var log := ErrorLog.new()
	OS.add_logger(log)
	var ext := Extension.new()
	GLTFDocument.register_gltf_document_extension(ext)
	var report := {"path": "runtime", "files": {}}
	var failed := 0
	for path in Checks.expect_files(assets):
		var expect: Dictionary = Checks.read_json(path)
		log.take()
		var doc := GLTFDocument.new()
		var state := GLTFState.new()
		var err := doc.append_from_file(assets.path_join(expect["glb"]), state)
		var root: Node = doc.generate_scene(state) if err == OK else null
		var errors: PackedStringArray = log.take()
		var res := {"problems": PackedStringArray(["append_from_file failed %d" % err]), "report": {}}
		if root != null:
			res = Checks.check(root, expect, errors)
			if expect["glb"] == "skin_valid.glb":
				var problems: PackedStringArray = res["problems"]
				problems.append_array(check_state(state))
				res["report"]["shader_code_skin"] = Checks.surfaces(root)["skin"][0].shader.code
				problems.append_array(check_helper(root, doc, state))
				res["problems"] = problems
			root.free()
		report["files"][expect["glb"]] = {"ok": res["problems"].is_empty(), "problems": res["problems"], "report": res["report"]}
		print("%s %s %s" % ["PASS" if res["problems"].is_empty() else "FAIL", expect["glb"], "; ".join(res["problems"])])
		if not res["problems"].is_empty():
			failed += 1
	report["literals"] = check_literals()
	if not report["literals"].is_empty():
		failed += 1
		print("FAIL literals: ", "; ".join(report["literals"]))
	report["double_registration"] = check_double_registration(assets, log)
	if not report["double_registration"].is_empty():
		failed += 1
		print("FAIL double registration: ", "; ".join(report["double_registration"]))
	GLTFDocument.unregister_gltf_document_extension(ext)
	OS.remove_logger(log)
	report["failed"] = failed
	if report_path != "":
		var f := FileAccess.open(report_path, FileAccess.WRITE)
		f.store_string(JSON.stringify(report, " "))
		f.close()
	print("RUNTIME %s: %d file(s) failed" % ["PASS" if failed == 0 else "FAIL", failed])
	quit(0 if failed == 0 else 1)


func check_state(state: GLTFState) -> PackedStringArray:
	var problems := PackedStringArray()
	var mats := state.get_materials()
	if not (mats[0] is ShaderMaterial and mats[2] is ShaderMaterial and mats[1] is BaseMaterial3D):
		problems.append("state materials not swapped: %s" % [mats])
	for gm in state.get_meshes():
		var mesh: ImporterMesh = gm.mesh
		for s in mesh.get_surface_count():
			var m := mesh.get_surface_material(s)
			if m.resource_name != "plain" and not m is ShaderMaterial:
				problems.append("ImporterMesh %s surface %d still has %s" % [gm.mesh.resource_name, s, m])
	return problems


func check_helper(root: Node, doc: GLTFDocument, state: GLTFState) -> PackedStringArray:
	var problems := PackedStringArray()
	var mats := SkinApi.materials(root)
	if mats.size() != 2:
		problems.append("helper found %d materials, expected 2" % mats.size())
	var n := SkinApi.set_parameter(root, "blood", 0.7)
	if n != 1:
		problems.append("set_parameter blood set %d materials, expected 1" % n)
	if SkinApi.set_parameter(root, "grime", 2.0) != 1:
		problems.append("set_parameter grime did not hit skin_b")
	if SkinApi.set_parameter(root, "nonexistent", 0.5) != 0:
		problems.append("set_parameter of an unknown parameter hit a material")
	var skin: ShaderMaterial = Checks.surfaces(root)["skin"][0]
	if not is_equal_approx(float(skin.get_shader_parameter("blood")), 0.7):
		problems.append("blood is %s after set_parameter" % skin.get_shader_parameter("blood"))
	var skin_b: ShaderMaterial = Checks.surfaces(root)["skin_b"][0]
	if not is_equal_approx(float(skin_b.get_shader_parameter("grime")), 1.0):
		problems.append("grime is %s after set_parameter(2.0), expected the clamp to 1" % skin_b.get_shader_parameter("grime"))
	var other := doc.generate_scene(state)
	if Checks.surfaces(other)["skin"][0] != skin:
		problems.append("a second scene from the same state does not share the material; the make_unique check means nothing")
	var copies := SkinApi.make_unique(root)
	if copies != 3:
		problems.append("make_unique copied %d surfaces, expected 3" % copies)
	SkinApi.set_parameter(root, "blood", 0.2)
	var mine: Array = Checks.surfaces(root)["skin"]
	var theirs: ShaderMaterial = Checks.surfaces(other)["skin"][0]
	if mine[0] == theirs or mine[0] != mine[1]:
		problems.append("make_unique: copies are not one per material within the scene and separate from the other scene")
	if not is_equal_approx(float(mine[0].get_shader_parameter("blood")), 0.2) or not is_equal_approx(float(theirs.get_shader_parameter("blood")), 0.7):
		problems.append("make_unique did not separate the scenes: %s %s" % [mine[0].get_shader_parameter("blood"), theirs.get_shader_parameter("blood")])
	problems.append_array(check_override(doc, state, theirs))
	other.free()
	return problems


func check_override(doc: GLTFDocument, state: GLTFState, shared: ShaderMaterial) -> PackedStringArray:
	var problems := PackedStringArray()
	var c := doc.generate_scene(state)
	var d := doc.generate_scene(state)
	var target: MeshInstance3D = null
	for n in c.find_children("*", "MeshInstance3D", true, false):
		if n.mesh != null and n.get_active_material(0) == shared:
			target = n
			break
	if target == null:
		problems.append("override: no mesh carries the shared skin material")
	else:
		var twin: MeshInstance3D = d.get_node(c.get_path_to(target))
		target.material_override = shared
		twin.material_override = shared
		var expected := 0
		for mi in c.find_children("*", "MeshInstance3D", true, false):
			if mi.mesh != null:
				for s in mi.mesh.get_surface_count():
					if SkinApi._is_skin(mi.get_active_material(s)):
						expected += 1
		var n_c := SkinApi.make_unique(c)
		if n_c != expected:
			problems.append("override: make_unique counted %d surfaces, %d draw with a skin copy" % [n_c, expected])
		if target.material_override == shared:
			problems.append("override: make_unique left the shared material_override in place")
		var before = shared.get_shader_parameter("paint")
		SkinApi.set_parameter(c, "paint", 0.77)
		if twin.get_active_material(0) != shared or shared.get_shader_parameter("paint") != before:
			problems.append("override: setting paint on one instance changed the other (%s)" % shared.get_shader_parameter("paint"))
		if not is_equal_approx(float(target.get_active_material(0).get_shader_parameter("paint")), 0.77):
			problems.append("override: paint on the unique instance is %s" % target.get_active_material(0).get_shader_parameter("paint"))
	c.free()
	d.free()
	return problems


func check_literals() -> PackedStringArray:
	var problems := PackedStringArray()
	for v in [0.5, 0.001, 0.000123456789, 1e-10, 1e-30, 1e-300, 5e-324]:
		var lit := Extension._f(maxf(v, Extension.TINY))
		var want := maxf(v, Extension.TINY)
		if absf(float(lit) - want) > want * 1e-8:
			problems.append("literal %s for %s reads back as %s" % [lit, v, float(lit)])
		var sh := Shader.new()
		sh.code = "shader_type spatial;\nuniform float probe = 0.5;\nvoid fragment() {\n\tALBEDO = vec3(clamp((probe - 0.5) / %s, 0.0, 1.0));\n}\n" % lit
		if sh.get_shader_uniform_list().is_empty():
			problems.append("literal %s for %s does not compile" % [lit, v])
	return problems


func check_double_registration(assets: String, log) -> PackedStringArray:
	var problems := PackedStringArray()
	var second := Extension.new()
	GLTFDocument.register_gltf_document_extension(second)
	log.take()
	var doc := GLTFDocument.new()
	var state := GLTFState.new()
	doc.append_from_file(assets.path_join("skin_valid.glb"), state)
	var root := doc.generate_scene(state)
	var errors: PackedStringArray = log.take()
	GLTFDocument.unregister_gltf_document_extension(second)
	if not errors.is_empty():
		problems.append("errors with two registered extensions: %s" % [errors])
	if not Checks.surfaces(root)["skin"][0] is ShaderMaterial:
		problems.append("skin not a ShaderMaterial with two registered extensions")
	root.free()
	return problems
