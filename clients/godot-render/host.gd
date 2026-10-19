extends SceneTree

const MAGIC := 0x48524647
const VERSION := 1
const HELLO := 1
const HELLO_ACK := 2
const MESH := 3
const POSITIONS := 4
const RENDER := 5
const FRAME := 6
const READBACK := 7
const VERTS := 8
const CONFIG := 9
const CONFIG_ACK := 10
const BYE := 11
const ERROR := 12
const MESH_ACK := 13
const POSITIONS_ACK := 14

var peer := StreamPeerTCP.new()
var mesh := ArrayMesh.new()
var body := MeshInstance3D.new()
var arrays := []
var material: Material
var env_node := WorldEnvironment.new()
var lights := Node3D.new()
var marker := MeshInstance3D.new()
var slots := {}
var done := false
var frame_delay_ms := 0


func _initialize() -> void:
	var port := 0
	var args := OS.get_cmdline_user_args()
	for i in args.size():
		if args[i] == "--port" and i + 1 < args.size():
			port = int(args[i + 1])
		if args[i] == "--frame-delay-ms" and i + 1 < args.size():
			frame_delay_ms = int(args[i + 1])
	RenderingServer.render_loop_enabled = false
	root.disable_3d = true
	if not args.has("--visible"):
		DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_MINIMIZED)
	build_scene()
	if peer.connect_to_host("127.0.0.1", port) != OK:
		printerr("render host: cannot connect to port %d" % port)
		done = true
		return
	var t0 := Time.get_ticks_msec()
	while peer.get_status() == StreamPeerTCP.STATUS_CONNECTING and Time.get_ticks_msec() - t0 < 10000:
		peer.poll()
		OS.delay_msec(2)
	if peer.get_status() != StreamPeerTCP.STATUS_CONNECTED:
		printerr("render host: no connection on port %d" % port)
		done = true
		return
	peer.set_no_delay(true)


func build_scene() -> void:
	var env := Environment.new()
	env.background_mode = Environment.BG_COLOR
	env.background_color = Color(0.12, 0.12, 0.13)
	env.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	env.ambient_light_color = Color(1, 1, 1)
	env.ambient_light_energy = 0.35
	env.tonemap_mode = Environment.TONE_MAPPER_LINEAR
	env_node.environment = env
	root.add_child(env_node)
	for d in [[Vector3(-0.35, -0.45, -1.0), 1.0], [Vector3(0.8, -0.2, -0.6), 0.45], [Vector3(0.1, -0.3, 1.0), 0.5]]:
		var l := DirectionalLight3D.new()
		l.light_energy = d[1]
		l.look_at_from_position(Vector3.ZERO, d[0], Vector3.UP if abs(d[0].normalized().y) < 0.95 else Vector3.FORWARD)
		lights.add_child(l)
	root.add_child(lights)
	var m := StandardMaterial3D.new()
	m.albedo_color = Color(0.78, 0.75, 0.72)
	m.roughness = 0.6
	m.cull_mode = BaseMaterial3D.CULL_DISABLED
	material = m
	body.mesh = mesh
	root.add_child(body)
	var sm := SphereMesh.new()
	sm.radius = 1.0
	sm.height = 2.0
	var mm := StandardMaterial3D.new()
	mm.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mm.no_depth_test = true
	mm.albedo_color = Color(1, 0, 1)
	mm.render_priority = 127
	sm.material = mm
	marker.mesh = sm
	marker.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	marker.visible = false
	root.add_child(marker)


func _process(_delta: float) -> bool:
	if done:
		return true
	peer.poll()
	if peer.get_status() != StreamPeerTCP.STATUS_CONNECTED:
		return true
	var handled := 0
	while peer.get_available_bytes() >= 16:
		if not handle():
			return true
		handled += 1
	if handled == 0:
		OS.delay_usec(300)
	return false


func read_exact(n: int) -> PackedByteArray:
	if n == 0:
		return PackedByteArray()
	var r: Array = peer.get_data(n)
	if r[0] != OK or r[1].size() != n:
		return PackedByteArray()
	return r[1]


func send(type: int, payload: PackedByteArray, tail := PackedByteArray()) -> void:
	var h := PackedByteArray()
	h.resize(16)
	h.encode_u32(0, MAGIC)
	h.encode_u16(4, VERSION)
	h.encode_u16(6, type)
	h.encode_u32(8, payload.size() + tail.size())
	h.encode_u32(12, 0)
	peer.put_data(h + payload)
	if tail.size() > 0:
		peer.put_data(tail)


func fail(text: String) -> bool:
	send(ERROR, text.to_utf8_buffer())
	return false


func handle() -> bool:
	var h := read_exact(16)
	if h.size() != 16 or h.decode_u32(0) != MAGIC:
		return fail("bad header")
	if h.decode_u16(4) != VERSION:
		return fail("protocol version %d, this host speaks %d" % [h.decode_u16(4), VERSION])
	var type := h.decode_u16(6)
	var length := h.decode_u32(8)
	var p := read_exact(length)
	if p.size() != length:
		return fail("short payload")
	match type:
		HELLO:
			var info := {"version": VERSION, "godot": Engine.get_version_info().string, "adapter": RenderingServer.get_video_adapter_name(),
				"api": RenderingServer.get_video_adapter_api_version(), "method": ProjectSettings.get_setting("rendering/renderer/rendering_method")}
			var out := PackedByteArray()
			out.resize(4)
			out.encode_u32(0, VERSION)
			send(HELLO_ACK, out + JSON.stringify(info).to_utf8_buffer())
		CONFIG:
			send(CONFIG_ACK, JSON.stringify(configure(JSON.parse_string(p.get_string_from_utf8()))).to_utf8_buffer())
		MESH:
			var t0 := Time.get_ticks_usec()
			var n := p.decode_u32(0)
			var ni := p.decode_u32(4)
			var o := 8
			arrays = []
			arrays.resize(Mesh.ARRAY_MAX)
			arrays[Mesh.ARRAY_VERTEX] = p.slice(o, o + n * 12).to_vector3_array()
			o += n * 12
			arrays[Mesh.ARRAY_NORMAL] = p.slice(o, o + n * 12).to_vector3_array()
			o += n * 12
			arrays[Mesh.ARRAY_TEX_UV] = p.slice(o, o + n * 8).to_vector2_array()
			o += n * 8
			arrays[Mesh.ARRAY_INDEX] = p.slice(o, o + ni * 4).to_int32_array()
			rebuild()
			var out := PackedByteArray()
			out.resize(8)
			out.encode_u32(0, n)
			out.encode_float(4, (Time.get_ticks_usec() - t0) / 1000.0)
			send(MESH_ACK, out)
		POSITIONS:
			var t0 := Time.get_ticks_usec()
			var n := p.decode_u32(8)
			if arrays.is_empty() or arrays[Mesh.ARRAY_VERTEX].size() != n:
				return fail("positions for %d vertices, the mesh has %d" % [n, 0 if arrays.is_empty() else arrays[Mesh.ARRAY_VERTEX].size()])
			arrays[Mesh.ARRAY_VERTEX] = p.slice(12, 12 + n * 12).to_vector3_array()
			arrays[Mesh.ARRAY_NORMAL] = p.slice(12 + n * 12, 12 + n * 24).to_vector3_array()
			rebuild()
			var out := PackedByteArray()
			out.resize(12)
			out.encode_u64(0, p.decode_u64(0))
			out.encode_float(8, (Time.get_ticks_usec() - t0) / 1000.0)
			send(POSITIONS_ACK, out)
		RENDER:
			render(p)
		READBACK:
			var v := PackedVector3Array()
			if mesh.get_surface_count() > 0:
				v = mesh.surface_get_arrays(0)[Mesh.ARRAY_VERTEX]
			var out := PackedByteArray()
			out.resize(4)
			out.encode_u32(0, v.size())
			send(VERTS, out, v.to_byte_array())
		BYE:
			done = true
		_:
			return fail("unknown message type %d" % type)
	return true


func rebuild() -> void:
	mesh.clear_surfaces()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	mesh.surface_set_material(0, material)


func configure(c: Variant) -> Dictionary:
	var problems := []
	if typeof(c) != TYPE_DICTIONARY:
		return {"ok": false, "problems": ["config is not a JSON object"]}
	if c.has("background"):
		var b: Array = c["background"]
		env_node.environment.background_color = Color(b[0], b[1], b[2])
	if c.get("material", null) is String:
		var m = load(c["material"])
		if m is Material:
			material = m
			if mesh.get_surface_count() > 0:
				mesh.surface_set_material(0, material)
		else:
			problems.append("material %s did not load as a Material" % c["material"])
	if c.get("environment", null) is String:
		var e = load(c["environment"])
		if e is Environment:
			env_node.environment = e
		else:
			problems.append("environment %s did not load as an Environment" % c["environment"])
	if c.get("scene", null) is String:
		var s = load(c["scene"])
		if s is PackedScene:
			lights.queue_free()
			lights = s.instantiate()
			root.add_child(lights)
		else:
			problems.append("scene %s did not load as a PackedScene" % c["scene"])
	return {"ok": problems.is_empty(), "problems": problems, "material": material.resource_path, "environment": env_node.environment.resource_path}


func slot(i: int) -> Dictionary:
	if not slots.has(i):
		var vp := SubViewport.new()
		vp.render_target_update_mode = SubViewport.UPDATE_DISABLED
		vp.msaa_3d = Viewport.MSAA_4X
		var cam := Camera3D.new()
		cam.keep_aspect = Camera3D.KEEP_HEIGHT
		vp.add_child(cam)
		root.add_child(vp)
		cam.make_current()
		slots[i] = {"vp": vp, "cam": cam}
	return slots[i]


func render(p: PackedByteArray) -> void:
	var req := p.decode_u32(0)
	var s := slot(p.decode_u32(4))
	var w := p.decode_u32(8)
	var h := p.decode_u32(12)
	var flags := p.decode_u32(16)
	var a := p.decode_float(20)
	var near := p.decode_float(24)
	var far := p.decode_float(28)
	var f := p.slice(32, 80).to_float32_array()
	var mk := p.slice(80, 96).to_float32_array()
	var vp: SubViewport = s["vp"]
	var cam: Camera3D = s["cam"]
	vp.size = Vector2i(w, h)
	vp.transparent_bg = (flags & 4) != 0
	var xf := Transform3D(Basis(Vector3(f[0], f[1], f[2]), Vector3(f[3], f[4], f[5]), Vector3(f[6], f[7], f[8])), Vector3(f[9], f[10], f[11]))
	cam.transform = xf
	RenderingServer.camera_set_transform(cam.get_camera_rid(), xf)
	if flags & 1:
		cam.set_orthogonal(a, near, far)
	else:
		cam.set_perspective(a, near, far)
	marker.visible = (flags & 2) != 0
	RenderingServer.instance_set_visible(marker.get_instance(), marker.visible)
	if marker.visible:
		marker.transform = Transform3D(Basis.from_scale(Vector3.ONE * mk[3]), Vector3(mk[0], mk[1], mk[2]))
		RenderingServer.instance_set_transform(marker.get_instance(), marker.transform)
	var t0 := Time.get_ticks_usec()
	vp.render_target_update_mode = SubViewport.UPDATE_ONCE
	RenderingServer.force_draw(false)
	var t1 := Time.get_ticks_usec()
	var img := vp.get_texture().get_image()
	if img.get_format() != Image.FORMAT_RGBA8:
		img.convert(Image.FORMAT_RGBA8)
	var t2 := Time.get_ticks_usec()
	if frame_delay_ms > 0:
		OS.delay_msec(frame_delay_ms)
	var out := PackedByteArray()
	out.resize(20)
	out.encode_u32(0, req)
	out.encode_u32(4, img.get_width())
	out.encode_u32(8, img.get_height())
	out.encode_float(12, (t1 - t0) / 1000.0)
	out.encode_float(16, (t2 - t1) / 1000.0)
	send(FRAME, out, img.get_data())
