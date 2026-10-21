@tool
class_name GazeForgeHumanoidExtension
extends GLTFDocumentExtension

const EXTENSION := "GAZEFORGE_humanoid"
const META := "gazeforge_bone_map"


func _get_supported_extensions() -> PackedStringArray:
	return PackedStringArray([EXTENSION])


func _import_post(state: GLTFState, root: Node) -> Error:
	var ext = (state.json.get("extensions", {}) as Dictionary).get(EXTENSION)
	if not ext is Dictionary:
		return OK
	var result := build(root, ext, godot_names(state))
	if result.has("error"):
		push_error("%s: %s; the character imports without a bone map" % [EXTENSION, result["error"]])
	return OK


# Godot renames nodes it cannot use as they are ("mixamorig:Hips" becomes "mixamorig_Hips"); the glTF state keeps both.
static func godot_names(state: GLTFState) -> Dictionary:
	var out := {}
	for node in state.get_nodes():
		if node.original_name != "":
			out[node.original_name] = node.resource_name
	return out


static func build(root: Node, ext: Dictionary, names: Dictionary) -> Dictionary:
	var table: Dictionary = ext.get("godot", {})
	if table.is_empty():
		return {"error": "no godot table"}
	var skeleton := find_skeleton(root, str(names.get(table.values()[0], table.values()[0])))
	if skeleton == null:
		return {"error": "no Skeleton3D has the bone %s" % table.values()[0]}
	var map := bone_map(table, names)
	var missing := PackedStringArray()
	for profile in table:
		if skeleton.find_bone(map.get_skeleton_bone_name(profile)) < 0:
			missing.append("%s (%s)" % [profile, map.get_skeleton_bone_name(profile)])
	if not missing.is_empty():
		return {"error": "the skeleton has no bone for %s" % ", ".join(missing)}
	skeleton.set_meta(META, map)
	return {"bone_map": map, "skeleton": skeleton}


static func bone_map(table: Dictionary, names: Dictionary) -> BoneMap:
	var map := BoneMap.new()
	map.profile = SkeletonProfileHumanoid.new()
	for profile in table:
		map.set_skeleton_bone_name(profile, str(names.get(table[profile], table[profile])))
	return map


static func find_skeleton(node: Node, bone: String) -> Skeleton3D:
	if node is Skeleton3D and (node as Skeleton3D).find_bone(bone) >= 0:
		return node
	for child in node.get_children():
		var found := find_skeleton(child, bone)
		if found != null:
			return found
	return null
