class_name GazeForgeCloth
extends SoftBody3D

@export var cloth_mesh: Mesh
@export var mesh_transform: Transform3D
@export var skeleton_path: NodePath
@export var skin_source: Skin
@export var pinned: PackedInt32Array
@export var colliders: Array

var _skeleton: Skeleton3D
var _influences: Array = []
var _bodies: Array = []


func _ready() -> void:
	if Engine.is_editor_hint():
		return
	_skeleton = get_node_or_null(skeleton_path) as Skeleton3D
	if _skeleton == null or skin_source == null or cloth_mesh == null:
		push_error("GazeForgeCloth %s: no skeleton, skin or mesh; the cloth is not driven" % name)
		return
	if mesh == null:
		mesh = world_mesh(_skeleton.global_transform * mesh_transform)
		linear_stiffness = 1.0
		simulation_precision = 10
		for s in cloth_mesh.get_surface_count():
			var m := cloth_mesh.surface_get_material(s)
			if m is BaseMaterial3D and (m as BaseMaterial3D).cull_mode != BaseMaterial3D.CULL_DISABLED:
				var both := (m as BaseMaterial3D).duplicate() as BaseMaterial3D
				both.cull_mode = BaseMaterial3D.CULL_DISABLED
				set_surface_override_material(s, both)
		_reenter.call_deferred()
	var arrays := cloth_mesh.surface_get_arrays(0)
	var verts: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
	var bones: PackedInt32Array = arrays[Mesh.ARRAY_BONES]
	var weights: PackedFloat32Array = arrays[Mesh.ARRAY_WEIGHTS]
	var per := bones.size() / maxi(verts.size(), 1)
	var bind_bone := PackedInt32Array()
	for j in skin_source.get_bind_count():
		var b := skin_source.get_bind_bone(j)
		bind_bone.append(b if b >= 0 else _skeleton.find_bone(skin_source.get_bind_name(j)))
	for i in pinned:
		set_point_pinned(i, true)
		var at_rest := mesh_transform * verts[i]
		var influence := []
		for k in per:
			var w := weights[i * per + k]
			if w > 0.0:
				var bone := bind_bone[bones[i * per + k]]
				influence.append([bone, w, _skeleton.get_bone_global_rest(bone).affine_inverse() * at_rest])
		_influences.append(influence)
	var scale := _skeleton.global_transform.basis.get_scale().x
	for c in colliders:
		var bone := _skeleton.find_bone(str(c["bone"]))
		if bone < 0:
			continue
		var body := AnimatableBody3D.new()
		body.top_level = true
		var shape := CollisionShape3D.new()
		var capsule := CapsuleShape3D.new()
		capsule.radius = float(c["radius"]) * scale
		capsule.height = (c["head"] as Vector3).distance_to(c["tail"]) * scale + 2.0 * capsule.radius
		shape.shape = capsule
		body.add_child(shape)
		add_child(body)
		_bodies.append([body, bone, c["head"], c["tail"]])
	_place_bodies()


# A soft body builds its points from its mesh and global transform only when it enters the world: a mesh assigned
# while it is outside the tree logs an error per vertex, and one assigned once it is inside leaves it with no points
# (measured on Godot 4.7.2). So the importer leaves the mesh unset, and the node takes it in _ready and enters again.
# The soft body also resets its own transform to keep its points in world space, so the mesh's place under the
# skeleton comes from mesh_transform, never from transform.
func _reenter() -> void:
	var parent := get_parent()
	var at := get_index()
	parent.remove_child(self)
	top_level = true
	transform = Transform3D.IDENTITY
	parent.add_child(self)
	parent.move_child(self, at)


# A soft body ignores the scale in its transform (measured: every edge started at 1 / 1.1019 of its rest length under a
# Character root scaled 1.1019), so its mesh is baked into world space at the character's place and size, and the body
# itself sits at the identity.
func world_mesh(to_world: Transform3D) -> ArrayMesh:
	var out := ArrayMesh.new()
	var turn := to_world.basis.orthonormalized()
	for s in cloth_mesh.get_surface_count():
		var arrays := cloth_mesh.surface_get_arrays(s)
		var verts: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
		for i in verts.size():
			verts[i] = to_world * verts[i]
		arrays[Mesh.ARRAY_VERTEX] = verts
		if arrays[Mesh.ARRAY_NORMAL] != null:
			var normals: PackedVector3Array = arrays[Mesh.ARRAY_NORMAL]
			for i in normals.size():
				normals[i] = (turn * normals[i]).normalized()
			arrays[Mesh.ARRAY_NORMAL] = normals
		arrays[Mesh.ARRAY_TANGENT] = null
		arrays[Mesh.ARRAY_BONES] = null
		arrays[Mesh.ARRAY_WEIGHTS] = null
		out.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		out.surface_set_material(s, cloth_mesh.surface_get_material(s))
	return out


func _physics_process(_delta: float) -> void:
	if _skeleton == null or _influences.is_empty():
		return
	var rid := get_physics_rid()
	for n in pinned.size():
		PhysicsServer3D.soft_body_move_point(rid, pinned[n], skinned_target(n))
	_place_bodies()


func skinned_target(n: int) -> Vector3:
	var p := Vector3.ZERO
	for inf in _influences[n]:
		p += (_skeleton.get_bone_global_pose(inf[0]) * inf[2]) * inf[1]
	return _skeleton.global_transform * p


func _place_bodies() -> void:
	for entry in _bodies:
		var bone_world := _skeleton.global_transform * _skeleton.get_bone_global_pose(entry[1])
		var head: Vector3 = bone_world * entry[2]
		var tail: Vector3 = bone_world * entry[3]
		var up := (tail - head).normalized() if head.distance_to(tail) > 1e-6 else Vector3.UP
		entry[0].global_transform = Transform3D(Basis(Quaternion(Vector3.UP, up)), (head + tail) * 0.5)
