extends SceneTree

# usage: godot --headless --path clients/godot-addon --script res://tests/make_bone_maps.gd -- OUT_DIR CHARACTER.glb...
# Imports each exported character with the humanoid extension and saves its BoneMap as OUT_DIR/<preset>.tres (the maps
# shipped in addons/gazeforge_humanoid/bone_maps are made this way, so their bone names are Godot's imported names).

const Extension := preload("res://addons/gazeforge_humanoid/gazeforge_humanoid_extension.gd")


func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	var out: String = args[0]
	DirAccess.make_dir_recursive_absolute(out)
	var failed := 0
	for glb in args.slice(1):
		var ext := Extension.new()
		GLTFDocument.register_gltf_document_extension(ext)
		var doc := GLTFDocument.new()
		var state := GLTFState.new()
		doc.append_from_file(glb, state)
		var root: Node = doc.generate_scene(state)
		GLTFDocument.unregister_gltf_document_extension(ext)
		var preset := str(state.json["extensions"][Extension.EXTENSION]["preset"])
		var result := Extension.build(root, state.json["extensions"][Extension.EXTENSION], Extension.godot_names(state))
		if result.has("error"):
			print("MAPS FAIL %s: %s" % [glb.get_file(), result["error"]])
			failed += 1
		else:
			var path := out.path_join("%s.tres" % preset)
			var err := ResourceSaver.save(result["bone_map"], path)
			print("MAPS %s -> %s (%d)" % [glb.get_file(), path, err])
		root.free()
	quit(1 if failed else 0)
