extends SceneTree

const Checks := preload("res://tests/skin_checks.gd")


func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	var assets: String = args[0] if args.size() > 0 else "res://tests/assets"
	var report_path: String = args[1] if args.size() > 1 else ""
	var report := {"path": "editor import", "files": {}}
	var failed := 0
	for path in Checks.expect_files(assets):
		var expect: Dictionary = Checks.read_json(path)
		var glb := assets.path_join(expect["glb"])
		var scene = load(glb)
		var res := {"problems": PackedStringArray(["%s did not load as a PackedScene (%s)" % [glb, scene]]), "report": {}}
		if scene is PackedScene:
			var root := (scene as PackedScene).instantiate()
			res = Checks.check(root, expect, null)
			res["report"]["imported_path"] = ResourceLoader.get_dependencies(glb)
			if expect["glb"] == "skin_valid.glb":
				var mat: ShaderMaterial = Checks.surfaces(root)["skin"][0]
				res["report"]["texture_classes"] = {}
				for u in ["gf_base_color", "gf_metallic_roughness", "gf_normal", "gf_occlusion", "gf_mask0", "gf_mask1"]:
					var t = mat.get_shader_parameter(u)
					res["report"]["texture_classes"][u] = [t.get_class() if t != null else "null", t.resource_path if t != null else ""]
				res["report"]["shader_code_skin"] = mat.shader.code
				res["report"]["editor_extracted_masks"] = extracted_masks(assets, expect)
			root.free()
		report["files"][expect["glb"]] = {"ok": res["problems"].is_empty(), "problems": res["problems"], "report": res["report"]}
		print("%s %s %s" % ["PASS" if res["problems"].is_empty() else "FAIL", expect["glb"], "; ".join(res["problems"])])
		if not res["problems"].is_empty():
			failed += 1
	report["failed"] = failed
	if report_path != "":
		var f := FileAccess.open(report_path, FileAccess.WRITE)
		f.store_string(JSON.stringify(report, " "))
		f.close()
	print("IMPORT %s: %d file(s) failed" % ["PASS" if failed == 0 else "FAIL", failed])
	quit(0 if failed == 0 else 1)


func extracted_masks(assets: String, expect: Dictionary) -> Dictionary:
	var out := {}
	for mask in expect["materials"]["skin"]["masks"]:
		var name := "mask0" if int(mask["texture"]) == 3 else "mask1"
		var path := assets.path_join("skin_valid_%s.png" % name)
		if not ResourceLoader.exists(path):
			out[name] = "not extracted"
			continue
		var tex: Texture2D = load(path)
		var img := tex.get_image()
		var info := {"class": tex.get_class(), "format": img.get_format(), "compressed": img.is_compressed()}
		if img.is_compressed():
			img.decompress()
		img.clear_mipmaps()
		img.convert(Image.FORMAT_RGBA8)
		var want: PackedByteArray = String(mask["rgba_hex"]).hex_decode()
		var data := img.get_data()
		var diff := 0
		var max_abs := 0
		for i in mini(want.size(), data.size()):
			var d := absi(int(data[i]) - int(want[i]))
			if d != 0:
				diff += 1
				max_abs = maxi(max_abs, d)
		info["differing_bytes"] = diff
		info["max_abs_diff"] = max_abs
		out[name] = info
	return out
