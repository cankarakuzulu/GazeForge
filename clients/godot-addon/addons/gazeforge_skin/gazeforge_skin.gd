@tool
class_name GazeForgeSkin
extends RefCounted

const META := "gazeforge_skin"


static func materials(root: Node) -> Array[ShaderMaterial]:
	var found: Array[ShaderMaterial] = []
	_collect(root, found)
	return found


static func set_parameter(root: Node, parameter: StringName, value: float) -> int:
	var count := 0
	for m in materials(root):
		if String(parameter) in parameters(m):
			m.set_shader_parameter(parameter, clampf(value, 0.0, 1.0))
			count += 1
	return count


static func parameters(material: Material) -> PackedStringArray:
	var out := PackedStringArray()
	if material == null or not material.has_meta(META):
		return out
	for layer in material.get_meta(META).get("layers", []):
		out.append(layer["parameter"])
		if layer.get("variant_parameter", "") != "":
			out.append(layer["variant_parameter"])
	return out


static func make_unique(root: Node) -> int:
	var count := 0
	var copies := {}
	for node in _meshes(root):
		var mi: MeshInstance3D = node
		if _is_skin(mi.material_override):
			mi.material_override = _copy(mi.material_override, copies)
		if mi.mesh == null:
			continue
		for s in mi.mesh.get_surface_count():
			var m := mi.get_surface_override_material(s)
			if m == null:
				m = mi.mesh.surface_get_material(s)
			if _is_skin(m):
				mi.set_surface_override_material(s, _copy(m, copies))
			if _is_skin(mi.get_active_material(s)):
				count += 1
	return count


static func _is_skin(m: Material) -> bool:
	return m is ShaderMaterial and m.has_meta(META)


static func _copy(m: Material, copies: Dictionary) -> Material:
	if not copies.has(m):
		copies[m] = m.duplicate()
	return copies[m]


static func _collect(root: Node, found: Array[ShaderMaterial]) -> void:
	for node in _meshes(root):
		var mi: MeshInstance3D = node
		var candidates: Array = [mi.material_override]
		if mi.mesh != null:
			for s in mi.mesh.get_surface_count():
				candidates.append(mi.get_active_material(s))
		for m in candidates:
			if _is_skin(m) and not found.has(m):
				found.append(m)


static func _meshes(root: Node) -> Array:
	var out := []
	if root is MeshInstance3D:
		out.append(root)
	for child in root.get_children():
		out.append_array(_meshes(child))
	return out
