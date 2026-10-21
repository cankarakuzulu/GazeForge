@tool
class_name GazeForgePhysicsExtension
extends GLTFDocumentExtension

const EXTENSION := "GAZEFORGE_physics"
const SIMULATOR := "GazeForgeSprings"
const FRAME_NODE := "Character"


func _get_supported_extensions() -> PackedStringArray:
	return PackedStringArray([EXTENSION])


func _import_post(state: GLTFState, root: Node) -> Error:
	var ext = (state.json.get("extensions", {}) as Dictionary).get(EXTENSION)
	if not ext is Dictionary:
		return OK
	var result := build(root, ext, joint_names(state.json))
	if result.has("error"):
		push_error("%s: %s; the character imports without spring bones" % [EXTENSION, result["error"]])
	return OK


static func joint_names(json: Dictionary) -> PackedStringArray:
	var out := PackedStringArray()
	var skins: Array = json.get("skins", [])
	var nodes: Array = json.get("nodes", [])
	if skins.is_empty():
		return out
	for n in skins[0].get("joints", []):
		out.append(str(nodes[int(n)].get("name", "")) if int(n) < nodes.size() else "")
	return out


static func build(root: Node, ext: Dictionary, names: PackedStringArray) -> Dictionary:
	var springs: Array = ext.get("springs", [])
	if springs.is_empty():
		return {}
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
