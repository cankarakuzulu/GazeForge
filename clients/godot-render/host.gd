extends SceneTree

const MAGIC := 0x48524647
const VERSION := 2
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

const ALBEDO := Color(0.80, 0.74, 0.68)
const WRAP := 0.3
const SKY_TOP := Color(0.159, 0.169, 0.183)
const SKY_BOTTOM := Color(0.087, 0.081, 0.081)
const RIG := [[-40.0, 30.0, 1.0, 5600.0], [50.0, 10.0, 0.35, 6500.0], [160.0, 35.0, 0.8, 7500.0]]

var peer := StreamPeerTCP.new()
var rd: RenderingDevice
var base := []
var lod_index := []
var lod_mesh := []
var dirty := []
var current_lod := -1
var body := MeshInstance3D.new()
var material: Material
var env_node := WorldEnvironment.new()
var lights := Node3D.new()
var rig := []
var marker := MeshInstance3D.new()
var slots := {}
var done := false
var frame_delay_ms := 0
var sync_readback := false
var pending := 0
var ready_frames := []


func _initialize() -> void:
	var port := 0
	var args := OS.get_cmdline_user_args()
	for i in args.size():
		if args[i] == "--port" and i + 1 < args.size():
			port = int(args[i + 1])
		if args[i] == "--frame-delay-ms" and i + 1 < args.size():
			frame_delay_ms = int(args[i + 1])
	sync_readback = args.has("--sync-readback")
	RenderingServer.render_loop_enabled = false
	root.disable_3d = true
	rd = RenderingServer.get_rendering_device()
	if rd == null:
		sync_readback = true
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


func kelvin(k: float) -> Color:
	var t := k / 100.0
	var r := 255.0
	var g := 0.0
	var b := 255.0
	if t <= 66.0:
		g = 99.4708025861 * log(t) - 161.1195681661
		b = 0.0 if t <= 19.0 else 138.5177312231 * log(t - 10.0) - 305.0447927307
	else:
		r = 329.698727446 * pow(t - 60.0, -0.1332047592)
		g = 288.1221695283 * pow(t - 60.0, -0.0755148492)
	return Color(clampf(r, 0, 255) / 255.0, clampf(g, 0, 255) / 255.0, clampf(b, 0, 255) / 255.0)


func build_scene() -> void:
	var env := Environment.new()
	env.background_mode = Environment.BG_COLOR
	env.background_color = Color(0.12, 0.12, 0.13)
	var sh := Shader.new()
	sh.code = "shader_type sky;\nuniform vec3 top;\nuniform vec3 bottom;\nvoid sky() { COLOR = mix(bottom, top, EYEDIR.z * 0.5 + 0.5); }\n"
	var skm := ShaderMaterial.new()
	skm.shader = sh
	skm.set_shader_parameter("top", Vector3(SKY_TOP.r, SKY_TOP.g, SKY_TOP.b))
	skm.set_shader_parameter("bottom", Vector3(SKY_BOTTOM.r, SKY_BOTTOM.g, SKY_BOTTOM.b))
	var sky := Sky.new()
	sky.sky_material = skm
	sky.radiance_size = Sky.RADIANCE_SIZE_32
	env.sky = sky
	env.ambient_light_source = Environment.AMBIENT_SOURCE_SKY
	env.ambient_light_sky_contribution = 1.0
	env.ambient_light_energy = 1.0
	env.reflected_light_source = Environment.REFLECTION_SOURCE_DISABLED
	env.tonemap_mode = Environment.TONE_MAPPER_LINEAR
	env_node.environment = env
	root.add_child(env_node)
	for d in RIG:
		var l := DirectionalLight3D.new()
		l.light_energy = d[2] * (1.0 + WRAP)
		l.light_color = kelvin(d[3])
		l.light_specular = 0.0
		var az: float = deg_to_rad(d[0])
		var el: float = deg_to_rad(d[1])
		rig.append([l, Vector3(sin(az) * cos(el), sin(el), cos(az) * cos(el))])
		lights.add_child(l)
	root.add_child(lights)
	var m := StandardMaterial3D.new()
	m.albedo_color = ALBEDO
	m.roughness = WRAP
	m.metallic_specular = 0.0
	m.diffuse_mode = BaseMaterial3D.DIFFUSE_LAMBERT_WRAP
	m.specular_mode = BaseMaterial3D.SPECULAR_DISABLED
	m.cull_mode = BaseMaterial3D.CULL_DISABLED
	material = m
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
	var t0 := Time.get_ticks_usec()
	var worked := false
	var flushes := 0
	while Time.get_ticks_usec() - t0 < 20000:
		peer.poll()
		if peer.get_status() != StreamPeerTCP.STATUS_CONNECTED:
			return true
		var handled := 0
		while peer.get_available_bytes() >= 16:
			if not handle():
				return true
			handled += 1
		if handled > 0:
			worked = true
			flushes = 0
			continue
		if pending > 0 and flushes < 8:
			flushes += 1
			RenderingServer.force_draw(false)
			flush_frames()
			continue
		break
	if not worked:
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
				"api": RenderingServer.get_video_adapter_api_version(), "method": ProjectSettings.get_setting("rendering/renderer/rendering_method"),
				"readback": "sync" if sync_readback else "async"}
			var out := PackedByteArray()
			out.resize(4)
			out.encode_u32(0, VERSION)
			send(HELLO_ACK, out + JSON.stringify(info).to_utf8_buffer())
		CONFIG:
			send(CONFIG_ACK, JSON.stringify(configure(JSON.parse_string(p.get_string_from_utf8()))).to_utf8_buffer())
		MESH:
			var t0 := Time.get_ticks_usec()
			var n := p.decode_u32(0)
			var nl := p.decode_u32(4)
			var o := 8
			base = []
			base.resize(Mesh.ARRAY_MAX)
			base[Mesh.ARRAY_VERTEX] = p.slice(o, o + n * 12).to_vector3_array()
			o += n * 12
			base[Mesh.ARRAY_NORMAL] = p.slice(o, o + n * 12).to_vector3_array()
			o += n * 12
			base[Mesh.ARRAY_TEX_UV] = p.slice(o, o + n * 8).to_vector2_array()
			o += n * 8
			lod_index = []
			lod_mesh = []
			dirty = []
			for l in nl:
				var ni := p.decode_u32(o)
				o += 4
				lod_index.append(p.slice(o, o + ni * 4).to_int32_array())
				o += ni * 4
				lod_mesh.append(ArrayMesh.new())
				dirty.append(true)
			current_lod = -1
			use_lod(0)
			var out := PackedByteArray()
			out.resize(8)
			out.encode_u32(0, n)
			out.encode_float(4, (Time.get_ticks_usec() - t0) / 1000.0)
			send(MESH_ACK, out)
		POSITIONS:
			var t0 := Time.get_ticks_usec()
			var n := p.decode_u32(8)
			if base.is_empty() or base[Mesh.ARRAY_VERTEX].size() != n:
				return fail("positions for %d vertices, the mesh has %d" % [n, 0 if base.is_empty() else base[Mesh.ARRAY_VERTEX].size()])
			base[Mesh.ARRAY_VERTEX] = p.slice(12, 12 + n * 12).to_vector3_array()
			base[Mesh.ARRAY_NORMAL] = p.slice(12 + n * 12, 12 + n * 24).to_vector3_array()
			for l in dirty.size():
				dirty[l] = true
			clean(current_lod)
			var out := PackedByteArray()
			out.resize(12)
			out.encode_u64(0, p.decode_u64(0))
			out.encode_float(8, (Time.get_ticks_usec() - t0) / 1000.0)
			send(POSITIONS_ACK, out)
		RENDER:
			render(p)
		READBACK:
			var v := PackedVector3Array()
			var ni := 0
			if current_lod >= 0:
				clean(current_lod)
				var a: Array = lod_mesh[current_lod].surface_get_arrays(0)
				v = a[Mesh.ARRAY_VERTEX]
				ni = a[Mesh.ARRAY_INDEX].size()
			var out := PackedByteArray()
			out.resize(12)
			out.encode_u32(0, v.size())
			out.encode_u32(4, maxi(current_lod, 0))
			out.encode_u32(8, ni)
			send(VERTS, out, v.to_byte_array())
		BYE:
			done = true
		_:
			return fail("unknown message type %d" % type)
	return true


func clean(l: int) -> void:
	if l < 0 or l >= lod_mesh.size() or not dirty[l]:
		return
	var a := base.duplicate()
	a[Mesh.ARRAY_INDEX] = lod_index[l]
	var m: ArrayMesh = lod_mesh[l]
	m.clear_surfaces()
	m.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, a)
	m.surface_set_material(0, material)
	dirty[l] = false


func use_lod(l: int) -> void:
	if lod_mesh.is_empty():
		return
	l = clampi(l, 0, lod_mesh.size() - 1)
	clean(l)
	if l != current_lod:
		current_lod = l
		body.mesh = lod_mesh[l]
		RenderingServer.instance_set_base(body.get_instance(), lod_mesh[l].get_rid())


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
			for x in lod_mesh:
				if x.get_surface_count() > 0:
					x.surface_set_material(0, material)
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
			rig = []
			lights = s.instantiate()
			root.add_child(lights)
		else:
			problems.append("scene %s did not load as a PackedScene" % c["scene"])
	return {"ok": problems.is_empty(), "problems": problems, "material": material.resource_path, "environment": env_node.environment.resource_path,
		"default_lights": not rig.is_empty()}


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


func on_frame(data: PackedByteArray, req: int, w: int, h: int, draw_ms: float, asked: int) -> void:
	ready_frames.append([req, w, h, draw_ms, (Time.get_ticks_usec() - asked) / 1000.0, data])


func flush_frames() -> void:
	for f in ready_frames:
		pending -= 1
		send_frame(f[0], f[1], f[2], f[3], f[4], f[5])
	ready_frames.clear()


func send_frame(req: int, w: int, h: int, draw_ms: float, read_ms: float, data: PackedByteArray) -> void:
	var out := PackedByteArray()
	out.resize(20)
	out.encode_u32(0, req)
	out.encode_u32(4, w)
	out.encode_u32(8, h)
	out.encode_float(12, draw_ms)
	out.encode_float(16, read_ms)
	send(FRAME, out, data)


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
	var lod := p.decode_u32(96)
	if frame_delay_ms > 0:
		OS.delay_msec(frame_delay_ms)
	use_lod(lod)
	var vp: SubViewport = s["vp"]
	var cam: Camera3D = s["cam"]
	vp.size = Vector2i(w, h)
	vp.transparent_bg = (flags & 4) != 0
	var basis := Basis(Vector3(f[0], f[1], f[2]), Vector3(f[3], f[4], f[5]), Vector3(f[6], f[7], f[8]))
	var xf := Transform3D(basis, Vector3(f[9], f[10], f[11]))
	cam.transform = xf
	RenderingServer.camera_set_transform(cam.get_camera_rid(), xf)
	if flags & 1:
		cam.set_orthogonal(a, near, far)
	else:
		cam.set_perspective(a, near, far)
	for r in rig:
		var d: Vector3 = basis * r[1]
		var l: DirectionalLight3D = r[0]
		var up := Vector3.UP if abs(d.normalized().y) < 0.95 else Vector3.FORWARD
		var lx := Transform3D(Basis.looking_at(-d, up), Vector3.ZERO)
		l.transform = lx
		RenderingServer.instance_set_transform(l.get_instance(), lx)
	marker.visible = (flags & 2) != 0
	RenderingServer.instance_set_visible(marker.get_instance(), marker.visible)
	if marker.visible:
		marker.transform = Transform3D(Basis.from_scale(Vector3.ONE * mk[3]), Vector3(mk[0], mk[1], mk[2]))
		RenderingServer.instance_set_transform(marker.get_instance(), marker.transform)
	var t0 := Time.get_ticks_usec()
	vp.render_target_update_mode = SubViewport.UPDATE_ONCE
	RenderingServer.force_draw(false)
	var t1 := Time.get_ticks_usec()
	if sync_readback:
		flush_frames()
		var img := vp.get_texture().get_image()
		if img.get_format() != Image.FORMAT_RGBA8:
			img.convert(Image.FORMAT_RGBA8)
		send_frame(req, img.get_width(), img.get_height(), (t1 - t0) / 1000.0, (Time.get_ticks_usec() - t1) / 1000.0, img.get_data())
		return
	pending += 1
	var tex := RenderingServer.texture_get_rd_texture(vp.get_texture().get_rid())
	rd.texture_get_data_async(tex, 0, on_frame.bind(req, w, h, (t1 - t0) / 1000.0, t1))
	flush_frames()
