extends RefCounted

const SkinApi := preload("res://addons/gazeforge_skin/gazeforge_skin.gd")
const META := "gazeforge_skin"
const CORE_TEXTURES := {"skin": ["gf_base_color", "gf_metallic_roughness", "gf_normal", "gf_occlusion"], "skin_b": ["gf_base_color"]}


static func read_json(path: String):
	return JSON.parse_string(FileAccess.get_file_as_string(path))


static func expect_files(dir: String) -> PackedStringArray:
	var out := PackedStringArray()
	for f in DirAccess.get_files_at(dir):
		if f.ends_with(".expect.json"):
			out.append(dir.path_join(f))
	out.sort()
	return out


static func surfaces(root: Node) -> Dictionary:
	var by_name := {}
	for mi in _meshes(root):
		if mi.mesh == null:
			continue
		for s in mi.mesh.get_surface_count():
			var m: Material = mi.get_active_material(s)
			var key := m.resource_name if m != null else "<none>"
			if not by_name.has(key):
				by_name[key] = []
			by_name[key].append(m)
	return by_name


static func defaults_in_code(code: String) -> Dictionary:
	var out := {}
	var re := RegEx.create_from_string("uniform float ([A-Za-z_][A-Za-z0-9_]*) : hint_range\\(0\\.0, 1\\.0\\) = ([0-9.eE+-]+);")
	for m in re.search_all(code):
		out[m.get_string(1)] = float(m.get_string(2))
	return out


static func check(root: Node, expect: Dictionary, errors) -> Dictionary:
	var problems := PackedStringArray()
	var report := {}
	var found := surfaces(root)
	for name in expect["materials"]:
		var want: Dictionary = expect["materials"][name]
		var got: Array = found.get(name, [])
		var r := {"surfaces": got.size()}
		report[name] = r
		if got.size() != int(want["surfaces"]):
			problems.append("%s: %d surfaces, expected %d" % [name, got.size(), int(want["surfaces"])])
		for m in got:
			if want["shader"]:
				if not (m is ShaderMaterial and m.has_meta(META)):
					problems.append("%s: surface material is %s, expected a GAZEFORGE_skin ShaderMaterial" % [name, m])
			elif not m is BaseMaterial3D:
				problems.append("%s: surface material is %s, expected the core BaseMaterial3D" % [name, m])
		if got.is_empty():
			continue
		if not want["shader"]:
			r["class"] = got[0].get_class()
			if want.has("refusal") and errors != null:
				var hit := false
				for e in errors:
					if "GAZEFORGE_skin: material 0 'skin'" in e and want["refusal"] in e:
						hit = true
				r["refusal_logged"] = hit
				if not hit:
					problems.append("%s: no refusal error mentioning '%s' (errors: %s)" % [name, want["refusal"], errors])
			continue
		var mat = got[0]
		if not mat is ShaderMaterial:
			continue
		for other in got:
			if other != mat:
				problems.append("%s: surfaces carry different material objects" % name)
		var declared := {}
		for u in mat.shader.get_shader_uniform_list():
			declared[u["name"]] = true
		var code_defaults := defaults_in_code(mat.shader.code)
		var param_names: Array = want["parameters"].keys()
		r["parameters"] = Array(SkinApi.parameters(mat))
		if r["parameters"] != param_names:
			problems.append("%s: parameters %s, expected %s" % [name, r["parameters"], param_names])
		r["defaults"] = {}
		for p in param_names:
			if not declared.has(p):
				problems.append("%s: uniform '%s' is not declared" % [name, p])
			var d = code_defaults.get(p)
			r["defaults"][p] = d
			if d == null or absf(float(d) - float(want["parameters"][p])) > 1e-7:
				problems.append("%s: default of '%s' is %s, expected %s" % [name, p, d, want["parameters"][p]])
			var rs = RenderingServer.shader_get_parameter_default(mat.shader.get_rid(), p)
			if rs != null and absf(float(rs) - float(want["parameters"][p])) > 1e-7:
				problems.append("%s: renderer default of '%s' is %s, expected %s" % [name, p, rs, want["parameters"][p]])
		for t in CORE_TEXTURES.get(name, []):
			if not mat.get_shader_parameter(t) is Texture2D:
				problems.append("%s: core texture '%s' is not set" % [name, t])
		r["masks"] = []
		for mask in want["masks"]:
			var tex = mat.get_shader_parameter(mask["uniform"])
			var mr := {"uniform": mask["uniform"], "class": str(tex.get_class()) if tex != null else "null"}
			r["masks"].append(mr)
			if not tex is Texture2D:
				problems.append("%s: %s is not a texture" % [name, mask["uniform"]])
				continue
			var img: Image = tex.get_image()
			if img == null:
				problems.append("%s: %s has no readable image" % [name, mask["uniform"]])
				continue
			mr["format"] = img.get_format()
			mr["compressed"] = img.is_compressed()
			mr["size"] = [img.get_width(), img.get_height()]
			if img.is_compressed() or img.get_format() != Image.FORMAT_RGBA8 or img.get_width() != int(mask["width"]) or img.get_height() != int(mask["height"]):
				problems.append("%s: %s is %s format %d compressed %s, expected RGBA8 %dx%d" % [name, mask["uniform"], mr["size"], img.get_format(), img.is_compressed(), int(mask["width"]), int(mask["height"])])
				continue
			var want_bytes: PackedByteArray = String(mask["rgba_hex"]).hex_decode()
			var data := img.get_data().slice(0, want_bytes.size())
			var diff := 0
			var max_abs := 0
			for i in want_bytes.size():
				var dd := absi(int(data[i]) - int(want_bytes[i]))
				if dd != 0:
					diff += 1
					max_abs = maxi(max_abs, dd)
			mr["bytes"] = want_bytes.size()
			mr["differing_bytes"] = diff
			mr["max_abs_diff"] = max_abs
			if diff != 0:
				problems.append("%s: %s differs from the PNG in %d of %d bytes (max %d)" % [name, mask["uniform"], diff, want_bytes.size(), max_abs])
	if errors != null:
		var skin_errors := 0
		for e in errors:
			if "GAZEFORGE_skin" in e:
				skin_errors += 1
		var refusals := 0
		for name in expect["materials"]:
			if expect["materials"][name].has("refusal"):
				refusals += 1
		report["skin_errors"] = skin_errors
		if refusals == 0 and skin_errors != 0:
			problems.append("unexpected GAZEFORGE_skin errors: %s" % [errors])
	return {"problems": problems, "report": report}


static func _meshes(root: Node) -> Array:
	var out := []
	if root is MeshInstance3D:
		out.append(root)
	for child in root.get_children():
		out.append_array(_meshes(child))
	return out
