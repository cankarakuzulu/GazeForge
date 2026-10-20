@tool
class_name GazeForgeSkinExtension
extends GLTFDocumentExtension

const EXTENSION := "GAZEFORGE_skin"
const META := "gazeforge_skin"
const MAX_MASK_TEXTURES := 4
const CHANNELS := ["r", "g", "b", "a"]
const DEBUG_MODES := ["", "albedo", "roughness"]
# A positive m + a - 1 in float32 is at least ~6e-8, so dividing it by any spread at or below TINY saturates to 1 just as
# the exact spread would; flooring the divisor here keeps 0 / 0 out of the shader without changing a covered texel.
const TINY := 1e-30
const INTERNAL := [
	"gf_base_color_factor", "gf_base_color", "gf_metallic_factor", "gf_roughness_factor", "gf_metallic_roughness",
	"gf_normal", "gf_normal_scale", "gf_occlusion", "gf_occlusion_strength", "gf_emissive_factor", "gf_emissive",
	"gf_alpha_cutoff", "gf_mask0", "gf_mask1", "gf_mask2", "gf_mask3", "gf_bc", "gf_base", "gf_rough", "gf_metal",
	"gf_mr", "gf_m0", "gf_m1", "gf_m2", "gf_m3", "gf_cov", "gf_k", "gf_lc", "gf_lr",
]

# Exported because GLTFDocument imports with a copy of each registered extension, and a copy keeps only stored properties.
@export var debug_mode := ""


func _get_supported_extensions() -> PackedStringArray:
	return PackedStringArray([EXTENSION])


func _import_post_parse(state: GLTFState) -> Error:
	var json: Dictionary = state.json
	var materials_json: Array = json.get("materials", [])
	var materials := state.get_materials()
	var swap := {}
	var masks := {}
	for i in mini(materials_json.size(), materials.size()):
		var entry = materials_json[i]
		if not entry is Dictionary or not entry.get("extensions") is Dictionary:
			continue
		var ext = entry["extensions"].get(EXTENSION)
		if ext == null or materials[i] == null or materials[i].has_meta(META):
			continue
		var label := "material %d '%s'" % [i, str(entry.get("name", ""))]
		var result := build_material(state, entry, ext, materials[i], masks, debug_mode)
		if result.has("error"):
			push_error("%s: %s %s; keeping the core material" % [EXTENSION, label, result["error"]])
			continue
		if result.has("material"):
			swap[materials[i]] = result["material"]
			materials[i] = result["material"]
	if swap.is_empty():
		return OK
	state.set_materials(materials)
	for gltf_mesh in state.get_meshes():
		var mesh: ImporterMesh = gltf_mesh.mesh
		if mesh != null:
			for s in mesh.get_surface_count():
				var m := mesh.get_surface_material(s)
				if swap.has(m):
					mesh.set_surface_material(s, swap[m])
		var instance := gltf_mesh.get_instance_materials()
		for s in instance.size():
			if swap.has(instance[s]):
				instance[s] = swap[instance[s]]
		gltf_mesh.set_instance_materials(instance)
	return OK


static func build_material(state: GLTFState, entry: Dictionary, ext, core: Material, mask_cache := {}, debug := "") -> Dictionary:
	if not debug in DEBUG_MODES:
		return {"error": "debug mode '%s' is not one of %s" % [debug, DEBUG_MODES]}
	var json: Dictionary = state.json
	var textures: Array = json.get("textures", [])
	var checked := validate(ext, textures.size())
	if checked.has("error"):
		return checked
	var layers: Array = checked["layers"]
	if layers.is_empty():
		return {}
	if not core is BaseMaterial3D:
		return {"error": "has no core BaseMaterial3D to build on"}
	var mask_slots: Array = []
	for layer in layers:
		if not layer["texture"] in mask_slots:
			mask_slots.append(layer["texture"])
	if mask_slots.size() > MAX_MASK_TEXTURES:
		return {"error": "uses %d mask textures; at most %d are supported" % [mask_slots.size(), MAX_MASK_TEXTURES]}
	var mask_textures := []
	for t in mask_slots:
		if not mask_cache.has(t):
			mask_cache[t] = load_mask(state, t)
		if mask_cache[t] is String:
			return {"error": "mask texture %d: %s" % [t, mask_cache[t]]}
		mask_textures.append(mask_cache[t])
	var base: BaseMaterial3D = core
	var pbr: Dictionary = entry.get("pbrMetallicRoughness", {}) if entry.get("pbrMetallicRoughness") is Dictionary else {}
	var spec := {
		"base_color_factor": _numbers(pbr.get("baseColorFactor"), [1.0, 1.0, 1.0, 1.0]),
		"metallic_factor": float(pbr.get("metallicFactor", 1.0)) if _is_num(pbr.get("metallicFactor", 1.0)) else 1.0,
		"roughness_factor": float(pbr.get("roughnessFactor", 1.0)) if _is_num(pbr.get("roughnessFactor", 1.0)) else 1.0,
		"emissive_factor": _numbers(entry.get("emissiveFactor"), [0.0, 0.0, 0.0]),
		"double_sided": bool(entry.get("doubleSided", false)),
		"alpha_mode": str(entry.get("alphaMode", "OPAQUE")),
		"alpha_cutoff": float(entry.get("alphaCutoff", 0.5)),
		"vertex_color": base.vertex_color_use_as_albedo,
		"textures": {},
	}
	var refs := {
		"base_color": [pbr.get("baseColorTexture"), base.albedo_texture],
		"metallic_roughness": [pbr.get("metallicRoughnessTexture"), base.roughness_texture],
		"normal": [entry.get("normalTexture"), base.normal_texture],
		"occlusion": [entry.get("occlusionTexture"), base.ao_texture],
		"emissive": [entry.get("emissiveTexture"), base.emission_texture],
	}
	var texture_values := {}
	for key in refs:
		var info = refs[key][0]
		var tex: Texture2D = refs[key][1]
		if info is Dictionary and tex != null:
			spec["textures"][key] = {
				"uv2": int(info.get("texCoord", 0)) == 1,
				"scale": float(info.get("scale", 1.0)),
				"strength": float(info.get("strength", 1.0)),
			}
			texture_values["gf_" + key] = tex
	var shader := Shader.new()
	shader.resource_name = "%s %s" % [EXTENSION, core.resource_name]
	shader.code = shader_code(spec, layers, mask_slots, debug)
	var declared := {}
	for u in shader.get_shader_uniform_list():
		declared[u["name"]] = true
	if declared.is_empty():
		return {"error": "generated a shader that does not compile; a parameter is probably not a usable shader uniform name (a shading-language keyword or built-in)"}
	for layer in layers:
		for p in [layer["parameter"], layer.get("variant_parameter", "")]:
			if p != "" and not declared.has(p):
				return {"error": "parameter '%s' is not a usable shader uniform name" % p}
	var material := ShaderMaterial.new()
	material.resource_name = core.resource_name
	material.shader = shader
	for key in texture_values:
		material.set_shader_parameter(key, texture_values[key])
	for i in mask_textures.size():
		material.set_shader_parameter("gf_mask%d" % i, mask_textures[i])
	var info_layers := []
	for layer in layers:
		info_layers.append({"name": layer["name"], "kind": layer["kind"], "parameter": layer["parameter"],
				"variant_parameter": layer.get("variant_parameter", "")})
	material.set_meta(META, {"version": 1, "debug": debug, "layers": info_layers})
	return {"material": material}


static func validate(ext, texture_count: int) -> Dictionary:
	if not ext is Dictionary:
		return {"error": "extension is not an object"}
	if not _is_int(ext.get("version")) or int(ext["version"]) != 1:
		return {"error": "version is %s; only the integer 1 is supported" % _show(ext.get("version"))}
	if not ext.get("live") is Array:
		return {"error": "'live' is missing or not an array"}
	var name_re := RegEx.create_from_string("^[A-Za-z_][A-Za-z0-9_]*$")
	var names := {}
	var parameters := {}
	var layers := []
	for i in ext["live"].size():
		var raw = ext["live"][i]
		var at := "live[%d]" % i
		if not raw is Dictionary:
			return {"error": "%s is not an object" % at}
		if not raw.get("name") is String or raw["name"] == "":
			return {"error": "%s has no name" % at}
		if names.has(raw["name"]):
			return {"error": "%s name '%s' is not unique" % [at, raw["name"]]}
		names[raw["name"]] = true
		if not raw.get("kind") is String:
			return {"error": "%s kind is not a string" % at}
		var layer := {"name": raw["name"], "kind": raw["kind"]}
		var err := _check_state(raw, at, name_re, parameters, layer, "")
		if err != "":
			return {"error": err}
		var mask = raw.get("mask")
		if not mask is Dictionary or not _is_int(mask.get("index")) or not _is_int(mask.get("channel")):
			return {"error": "%s mask is not {index, channel} with integer values" % at}
		if int(mask["channel"]) < 0 or int(mask["channel"]) > 3:
			return {"error": "%s mask channel %d is outside 0..3" % [at, int(mask["channel"])]}
		if int(mask["index"]) < 0 or int(mask["index"]) >= texture_count:
			return {"error": "%s mask texture %d does not exist" % [at, int(mask["index"])]}
		layer["texture"] = int(mask["index"])
		layer["channel"] = int(mask["channel"])
		if not _is_num(raw.get("spread")) or float(raw["spread"]) <= 0.0 or float(raw["spread"]) > 1.0:
			return {"error": "%s spread %s is not a number in (0, 1]" % [at, _show(raw.get("spread"))]}
		if not _unit(raw.get("opacity")):
			return {"error": "%s opacity %s is not a number in [0, 1]" % [at, _show(raw.get("opacity"))]}
		layer["spread"] = float(raw["spread"])
		layer["opacity"] = float(raw["opacity"])
		if raw.has("variant") and raw["variant"] != null:
			if not raw["variant"] is Dictionary:
				return {"error": "%s variant is not an object" % at}
			err = _check_state(raw["variant"], at + ".variant", name_re, parameters, layer, "variant_")
			if err != "":
				return {"error": err}
		layers.append(layer)
	return {"layers": layers}


static func _check_state(raw: Dictionary, at: String, name_re: RegEx, parameters: Dictionary, layer: Dictionary, prefix: String) -> String:
	var p = raw.get("parameter")
	if not p is String or name_re.search(p) == null:
		return "%s parameter %s is not a name matching [A-Za-z_][A-Za-z0-9_]*" % [at, str(p)]
	if parameters.has(p):
		return "%s parameter '%s' is not unique within the material" % [at, p]
	if p in INTERNAL:
		return "%s parameter '%s' is a name this reader uses internally" % [at, p]
	parameters[p] = true
	if not _unit(raw.get("default")):
		return "%s default %s is not a number in [0, 1]" % [at, _show(raw.get("default"))]
	var c = raw.get("colour")
	if not c is Array or c.size() != 3 or not (_unit(c[0]) and _unit(c[1]) and _unit(c[2])):
		return "%s colour is not three numbers in [0, 1]" % at
	if not _unit(raw.get("roughness")):
		return "%s roughness %s is not a number in [0, 1]" % [at, _show(raw.get("roughness"))]
	layer[prefix + "parameter"] = p
	layer[prefix + "default"] = float(raw["default"])
	layer[prefix + "colour"] = [float(c[0]), float(c[1]), float(c[2])]
	layer[prefix + "roughness"] = float(raw["roughness"])
	return ""


static func load_mask(state: GLTFState, texture_index: int):
	var json: Dictionary = state.json
	var textures: Array = json.get("textures", [])
	var images: Array = json.get("images", [])
	var texture = textures[texture_index]
	if not texture is Dictionary or not _is_int(texture.get("source")):
		return "has no image source"
	var source := int(texture["source"])
	if source < 0 or source >= images.size() or not images[source] is Dictionary:
		return "image %d does not exist" % source
	var image_json: Dictionary = images[source]
	var bytes := PackedByteArray()
	if _is_int(image_json.get("bufferView")):
		var views := state.get_buffer_views()
		var view := int(image_json["bufferView"])
		if view < 0 or view >= views.size():
			return "bufferView %d does not exist" % view
		bytes = views[view].load_buffer_view_data(state)
	elif image_json.get("uri") is String:
		var uri: String = image_json["uri"]
		if uri.begins_with("data:"):
			bytes = Marshalls.base64_to_raw(uri.substr(uri.find(",") + 1))
		else:
			bytes = FileAccess.get_file_as_bytes(state.base_path.path_join(uri.uri_decode()))
	if bytes.is_empty():
		return "image %d has no data" % source
	var image := Image.new()
	if image.load_png_from_buffer(bytes) != OK:
		return "image %d is not a readable PNG" % source
	if image.get_format() != Image.FORMAT_RGBA8:
		image.convert(Image.FORMAT_RGBA8)
	image.generate_mipmaps()
	var tex := ImageTexture.create_from_image(image)
	tex.resource_name = str(image_json.get("name", "mask %d" % texture_index))
	return tex


static func shader_code(spec: Dictionary, layers: Array, mask_slots: Array, debug := "") -> String:
	var t: Dictionary = spec["textures"]
	var modes := ["blend_mix", "depth_draw_opaque", "cull_disabled" if spec["double_sided"] else "cull_back"]
	if debug != "":
		modes.append("unshaded")
	var out := PackedStringArray()
	out.append("shader_type spatial;")
	out.append("render_mode %s;" % ", ".join(modes))
	var bcf: Array = spec["base_color_factor"]
	out.append("uniform vec4 gf_base_color_factor = vec4(%s, %s, %s, %s);" % [_f(bcf[0]), _f(bcf[1]), _f(bcf[2]), _f(bcf[3])])
	out.append("uniform float gf_metallic_factor : hint_range(0.0, 1.0) = %s;" % _f(spec["metallic_factor"]))
	out.append("uniform float gf_roughness_factor : hint_range(0.0, 1.0) = %s;" % _f(spec["roughness_factor"]))
	var ef: Array = spec["emissive_factor"]
	var emissive: bool = t.has("emissive") or ef[0] != 0.0 or ef[1] != 0.0 or ef[2] != 0.0
	if emissive:
		out.append("uniform vec3 gf_emissive_factor = vec3(%s, %s, %s);" % [_f(ef[0]), _f(ef[1]), _f(ef[2])])
	if spec["alpha_mode"] == "MASK":
		out.append("uniform float gf_alpha_cutoff = %s;" % _f(spec["alpha_cutoff"]))
	var sampler := "filter_linear_mipmap, repeat_enable"
	if t.has("base_color"):
		out.append("uniform sampler2D gf_base_color : source_color, %s;" % sampler)
	if t.has("metallic_roughness"):
		out.append("uniform sampler2D gf_metallic_roughness : %s;" % sampler)
	if t.has("normal"):
		out.append("uniform sampler2D gf_normal : hint_normal, %s;" % sampler)
		out.append("uniform float gf_normal_scale = %s;" % _f(t["normal"]["scale"]))
	if t.has("occlusion"):
		out.append("uniform sampler2D gf_occlusion : %s;" % sampler)
		out.append("uniform float gf_occlusion_strength : hint_range(0.0, 1.0) = %s;" % _f(t["occlusion"]["strength"]))
	if t.has("emissive"):
		out.append("uniform sampler2D gf_emissive : source_color, %s;" % sampler)
	for i in mask_slots.size():
		out.append("uniform sampler2D gf_mask%d : hint_default_black, %s;" % [i, sampler])
	for layer in layers:
		out.append("uniform float %s : hint_range(0.0, 1.0) = %s;" % [layer["parameter"], _f(layer["default"])])
		if layer.has("variant_parameter"):
			out.append("uniform float %s : hint_range(0.0, 1.0) = %s;" % [layer["variant_parameter"], _f(layer["variant_default"])])
	out.append("")
	out.append("void fragment() {")
	out.append("\tvec4 gf_bc = gf_base_color_factor;")
	if t.has("base_color"):
		out.append("\tgf_bc *= texture(gf_base_color, %s);" % _uv(t["base_color"]))
	if spec["vertex_color"]:
		out.append("\tgf_bc *= COLOR;")
	out.append("\tvec3 gf_base = gf_bc.rgb;")
	out.append("\tfloat gf_rough = gf_roughness_factor;")
	out.append("\tfloat gf_metal = gf_metallic_factor;")
	if t.has("metallic_roughness"):
		out.append("\tvec4 gf_mr = texture(gf_metallic_roughness, %s);" % _uv(t["metallic_roughness"]))
		out.append("\tgf_rough *= gf_mr.g;")
		out.append("\tgf_metal *= gf_mr.b;")
	for i in mask_slots.size():
		out.append("\tvec4 gf_m%d = texture(gf_mask%d, UV);" % [i, i])
	for layer in layers:
		var m := "gf_m%d.%s" % [mask_slots.find(layer["texture"]), CHANNELS[layer["channel"]]]
		var a: String = layer["parameter"]
		var colour: Array = layer["colour"]
		var lc := "vec3(%s, %s, %s)" % [_f(colour[0]), _f(colour[1]), _f(colour[2])]
		var lr := _f(layer["roughness"])
		if layer.has("variant_parameter"):
			var vc: Array = layer["variant_colour"]
			lc = "mix(%s, vec3(%s, %s, %s), %s)" % [lc, _f(vc[0]), _f(vc[1]), _f(vc[2]), layer["variant_parameter"]]
			lr = "mix(%s, %s, %s)" % [lr, _f(layer["variant_roughness"]), layer["variant_parameter"]]
		out.append("\t{")
		out.append("\t\tfloat gf_cov = clamp((%s + %s - 1.0) / %s, 0.0, 1.0);" % [m, a, _f(maxf(layer["spread"], TINY))])
		out.append("\t\tvec3 gf_lc = %s;" % lc)
		out.append("\t\tfloat gf_lr = %s;" % lr)
		out.append("\t\tfloat gf_k = gf_cov * %s;" % _f(layer["opacity"]))
		out.append("\t\tgf_base = mix(gf_base, gf_lc, gf_k);")
		out.append("\t\tgf_rough = mix(gf_rough, gf_lr, gf_k);")
		out.append("\t}")
	if debug == "albedo":
		out.append("\tALBEDO = gf_base;")
	elif debug == "roughness":
		out.append("\tALBEDO = vec3(gf_rough);")
	else:
		out.append("\tALBEDO = gf_base;")
		out.append("\tROUGHNESS = gf_rough;")
		out.append("\tMETALLIC = gf_metal;")
		if t.has("normal"):
			out.append("\tNORMAL_MAP = texture(gf_normal, %s).rgb;" % _uv(t["normal"]))
			out.append("\tNORMAL_MAP_DEPTH = gf_normal_scale;")
		if t.has("occlusion"):
			out.append("\tAO = mix(1.0, texture(gf_occlusion, %s).r, gf_occlusion_strength);" % _uv(t["occlusion"]))
		if emissive:
			if t.has("emissive"):
				out.append("\tEMISSION = gf_emissive_factor * texture(gf_emissive, %s).rgb;" % _uv(t["emissive"]))
			else:
				out.append("\tEMISSION = gf_emissive_factor;")
		if spec["alpha_mode"] == "BLEND":
			out.append("\tALPHA = gf_bc.a;")
		elif spec["alpha_mode"] == "MASK":
			out.append("\tALPHA = gf_bc.a;")
			out.append("\tALPHA_SCISSOR_THRESHOLD = gf_alpha_cutoff;")
	out.append("}")
	return "\n".join(out) + "\n"


static func _uv(info: Dictionary) -> String:
	return "UV2" if info["uv2"] else "UV"


static func _f(x) -> String:
	var v := float(x) if absf(float(x)) >= TINY else 0.0
	if v == 0.0 or absf(v) >= 1e-3:
		return "%.9f" % v
	var e := floori(log(absf(v)) / log(10.0))
	return "%.9fe%d" % [v / pow(10.0, e), e]


static func _numbers(x, fallback: Array) -> Array:
	if not x is Array or x.size() != fallback.size():
		return fallback
	for v in x:
		if not _is_num(v):
			return fallback
	return x


static func _is_num(x) -> bool:
	return typeof(x) == TYPE_FLOAT or typeof(x) == TYPE_INT


static func _show(x) -> String:
	if typeof(x) == TYPE_NIL:
		return "missing"
	if _is_int(x):
		return str(int(x))
	if _is_num(x):
		return str(x)
	return "the %s %s" % [type_string(typeof(x)).to_lower(), JSON.stringify(x)]


static func _is_int(x) -> bool:
	return _is_num(x) and floor(float(x)) == float(x)


static func _unit(x) -> bool:
	return _is_num(x) and float(x) >= 0.0 and float(x) <= 1.0
