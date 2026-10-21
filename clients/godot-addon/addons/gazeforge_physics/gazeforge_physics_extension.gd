@tool
class_name GazeForgePhysicsExtension
extends GLTFDocumentExtension

const EXTENSION := "GAZEFORGE_physics"
const SIMULATOR := "GazeForgeSprings"
const FRAME_NODE := "Character"
const PIN_LIMIT := 0.5
const Cloth := preload("res://addons/gazeforge_physics/gazeforge_cloth.gd")


func _get_supported_extensions() -> PackedStringArray:
	return PackedStringArray([EXTENSION])


func _import_post(state: GLTFState, root: Node) -> Error:
	var ext = (state.json.get("extensions", {}) as Dictionary).get(EXTENSION)
	if not ext is Dictionary:
		return OK
	var pins := {}
	for c in ext.get("cloth", []):
		var nodes: Array = c.get("nodes", [])
		if not nodes.is_empty():
			pins[str(nodes[0])] = float_attribute(state, str(nodes[0]), str(c.get("pin", "_PIN")))
	var result := build(root, ext, joint_names(state.json), pins)
	if result.has("error"):
		push_error("%s: %s; the character imports without its physics" % [EXTENSION, result["error"]])
	return OK


static func float_attribute(state: GLTFState, node_name: String, attribute: String) -> PackedFloat32Array:
	var json: Dictionary = state.json
	var out := PackedFloat32Array()
	for node in json.get("nodes", []):
		if str(node.get("name", "")) != node_name or not node.has("mesh"):
			continue
		var prim: Dictionary = json["meshes"][int(node["mesh"])]["primitives"][0]
		if not prim.get("attributes", {}).has(attribute):
			return out
		var acc: Dictionary = json["accessors"][int(prim["attributes"][attribute])]
		var view: Dictionary = json["bufferViews"][int(acc["bufferView"])]
		var buf: PackedByteArray = state.buffers[int(view.get("buffer", 0))]
		var base := int(view.get("byteOffset", 0)) + int(acc.get("byteOffset", 0))
		var stride := int(view.get("byteStride", 4))
		for i in int(acc["count"]):
			out.append(buf.decode_float(base + i * stride))
		return out
	return out


static func joint_names(json: Dictionary) -> PackedStringArray:
	var out := PackedStringArray()
	var skins: Array = json.get("skins", [])
	var nodes: Array = json.get("nodes", [])
	if skins.is_empty():
		return out
	for n in skins[0].get("joints", []):
		out.append(str(nodes[int(n)].get("name", "")) if int(n) < nodes.size() else "")
	return out


static func build(root: Node, ext: Dictionary, names: PackedStringArray, pins := {}) -> Dictionary:
	var out := {}
	if not (ext.get("springs", []) as Array).is_empty():
		out = build_springs(root, ext, names)
		if out.has("error"):
			return out
	if not (ext.get("cloth", []) as Array).is_empty():
		var cloth := build_cloth(root, ext, pins)
		if cloth.has("error"):
			return cloth
		out.merge(cloth)
	return out


static func build_cloth(root: Node, ext: Dictionary, pins: Dictionary) -> Dictionary:
	var colliders: Array = ext.get("colliders", [])
	var made := []
	for c in ext.get("cloth", []):
		var nodes: Array = c.get("nodes", [])
		if nodes.is_empty():
			continue
		var mi := root.find_child(str(nodes[0]), true, false) as Node3D
		var source := mesh_of(mi)
		if source.is_empty():
			return {"error": "cloth %s: no mesh node %s" % [str(c.get("wearable")), str(nodes[0])]}
		var array_mesh: ArrayMesh = source["mesh"]
		var skeleton := mi.get_node_or_null(source["skeleton"]) as Skeleton3D
		if skeleton == null:
			skeleton = mi.get_parent() as Skeleton3D
		var pin: PackedFloat32Array = pins.get(str(nodes[0]), PackedFloat32Array())
		if skeleton == null or source["skin"] == null or pin.size() != array_mesh.surface_get_array_len(0):
			return {"error": "cloth %s: needs a skinned mesh under a skeleton and one pin weight per vertex (%d weights, %d vertices)" % [str(c.get("wearable")), pin.size(), array_mesh.surface_get_array_len(0)]}
		var pinned := PackedInt32Array()
		for i in pin.size():
			if pin[i] >= PIN_LIMIT:
				pinned.append(i)
		var to_skeleton := frame_to_skeleton(root, skeleton)
		var bodies := []
		for k in colliders:
			var bone := skeleton.find_bone(str(k.get("bone", "")))
			if bone < 0:
				continue
			var rest := skeleton.get_bone_global_rest(bone).affine_inverse()
			bodies.append({"bone": str(k["bone"]), "radius": float(k.get("radius", 0.05)),
					"head": rest * (to_skeleton * vec(k.get("head"))), "tail": rest * (to_skeleton * vec(k.get("tail")))})
		var cloth := Cloth.new()
		cloth.name = "%s_Cloth" % str(c.get("wearable", mi.name))
		cloth.cloth_mesh = array_mesh
		cloth.mesh_transform = mi.transform
		cloth.transform = mi.transform
		cloth.skin_source = source["skin"]
		cloth.pinned = pinned
		cloth.colliders = bodies
		mi.get_parent().add_child(cloth)
		cloth.owner = root
		cloth.skeleton_path = NodePath("..") if mi.get_parent() == skeleton else cloth.get_path_to(skeleton)
		mi.visible = false
		made.append(cloth)
	return {"cloth": made}


static func build_springs(root: Node, ext: Dictionary, names: PackedStringArray) -> Dictionary:
	var springs: Array = ext.get("springs", [])
	var chains := []
	for s in springs:
		var joints: Array = s.get("joints", [])
		if joints.size() < 2:
			return {"error": "spring %s has fewer than two joints" % str(s.get("chain"))}
		var bone_names := PackedStringArray()
		for j in joints:
			if int(j) < 0 or int(j) >= names.size():
				return {"error": "spring %s names joint %d, the skin has %d" % [str(s.get("chain")), int(j), names.size()]}
			bone_names.append(names[int(j)])
		chains.append(bone_names)
	var skeleton := find_skeleton(root, chains[0][0])
	if skeleton == null:
		return {"error": "no Skeleton3D has the bone %s" % chains[0][0]}
	for bone_names in chains:
		for k in bone_names.size():
			var b := skeleton.find_bone(bone_names[k])
			if b < 0:
				return {"error": "the skeleton has no bone %s" % bone_names[k]}
			if k > 0 and skeleton.get_bone_parent(b) != skeleton.find_bone(bone_names[k - 1]):
				return {"error": "%s is not the child of %s, so it is not a chain" % [bone_names[k], bone_names[k - 1]]}
	var to_skeleton := frame_to_skeleton(root, skeleton)
	var world := world_scale(root, skeleton)
	var defaults: Dictionary = ext.get("spring_defaults", {})
	var sim := SpringBoneSimulator3D.new()
	sim.name = SIMULATOR
	skeleton.add_child(sim)
	sim.owner = root
	sim.setting_count = chains.size()
	for i in chains.size():
		sim.set_root_bone_name(i, chains[i][0])
		sim.set_end_bone_name(i, chains[i][chains[i].size() - 1])
		sim.set_root_bone(i, skeleton.find_bone(chains[i][0]))
		sim.set_end_bone(i, skeleton.find_bone(chains[i][chains[i].size() - 1]))
		sim.set_stiffness(i, float(defaults.get("stiffness", 1.0)))
		sim.set_drag(i, float(defaults.get("drag", 0.4)))
		sim.set_gravity(i, float(defaults.get("gravity", 1.0)))
		sim.set_radius(i, float(defaults.get("hit_radius_m", 0.02)) * world)
		sim.set_enable_all_child_collisions(i, true)
	var made := 0
	var skipped := PackedStringArray()
	for c in ext.get("colliders", []):
		var bone := skeleton.find_bone(str(c.get("bone", "")))
		if bone < 0:
			skipped.append(str(c.get("bone", "")))
			continue
		var rest := skeleton.get_bone_global_rest(bone).affine_inverse()
		var head: Vector3 = rest * (to_skeleton * vec(c.get("head")))
		var tail: Vector3 = rest * (to_skeleton * vec(c.get("tail")))
		var capsule := SpringBoneCollisionCapsule3D.new()
		capsule.name = "Collider_%s" % c.get("bone")
		sim.add_child(capsule)
		capsule.owner = root
		capsule.bone_name = str(c.get("bone"))
		capsule.bone = bone
		capsule.radius = float(c.get("radius", 0.05)) * world
		capsule.mid_height = head.distance_to(tail) * world
		capsule.position_offset = (head + tail) * 0.5 * world
		if head.distance_to(tail) > 1e-6:
			capsule.rotation_offset = Quaternion(Vector3.UP, (tail - head).normalized())
		made += 1
	if not skipped.is_empty():
		push_warning("%s: no bone for colliders %s" % [EXTENSION, ", ".join(skipped)])
	return {"simulator": sim, "chains": chains.size(), "colliders": made}


# The editor import hands _import_post ImporterMeshInstance3D nodes (they become MeshInstance3D later); a run-time
# GLTFDocument scene has MeshInstance3D already.
static func mesh_of(node: Node) -> Dictionary:
	if node is MeshInstance3D and node.mesh is ArrayMesh:
		return {"mesh": node.mesh, "skin": node.skin, "skeleton": node.skeleton}
	if node is ImporterMeshInstance3D and node.mesh != null:
		return {"mesh": node.mesh.get_mesh(), "skin": node.skin, "skeleton": node.skeleton_path}
	return {}


static func find_skeleton(node: Node, bone: String) -> Skeleton3D:
	if node is Skeleton3D and (node as Skeleton3D).find_bone(bone) >= 0:
		return node
	for child in node.get_children():
		var found := find_skeleton(child, bone)
		if found != null:
			return found
	return null


static func frame_to_skeleton(root: Node, skeleton: Skeleton3D) -> Transform3D:
	var frame := root.find_child(FRAME_NODE, true, false) as Node3D
	if root is Node3D and root.name == FRAME_NODE:
		frame = root
	var up := Transform3D.IDENTITY
	var node: Node = skeleton
	while node != null and node != frame and node != root:
		if node is Node3D:
			up = (node as Node3D).transform * up
		node = node.get_parent()
	if node == frame or frame == null:
		return up.affine_inverse()
	var frame_up := Transform3D.IDENTITY
	var f: Node = frame
	while f != null and f != root:
		if f is Node3D:
			frame_up = (f as Node3D).transform * frame_up
		f = f.get_parent()
	return up.affine_inverse() * frame_up


# SpringBoneCollision3D applies position_offset, radius and mid_height in world units (rotated by the bone, not scaled by
# the skeleton), and the simulator's joint radius likewise: measured on Godot 4.7.2 with a skeleton scaled x2, a sphere's
# centre landed at offset / 2 in skeleton space and a chain settled at radius / 2 from it. The character's root carries
# its similarity scale, so every length is multiplied by the skeleton's scale in the imported scene; scaling the character
# again in a game leaves the colliders at the imported size.
static func world_scale(root: Node, skeleton: Skeleton3D) -> float:
	var basis := Basis.IDENTITY
	var node: Node = skeleton
	while node != null:
		if node is Node3D:
			basis = (node as Node3D).transform.basis * basis
		if node == root:
			break
		node = node.get_parent()
	return basis.get_scale().x


static func vec(a) -> Vector3:
	return Vector3(float(a[0]), float(a[1]), float(a[2])) if a is Array and a.size() == 3 else Vector3.ZERO
