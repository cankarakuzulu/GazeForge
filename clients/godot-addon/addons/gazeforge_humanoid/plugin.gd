@tool
extends EditorPlugin

const Extension := preload("res://addons/gazeforge_humanoid/gazeforge_humanoid_extension.gd")

var extension: GLTFDocumentExtension


func _enter_tree() -> void:
	extension = Extension.new()
	GLTFDocument.register_gltf_document_extension(extension)


func _exit_tree() -> void:
	if extension != null:
		GLTFDocument.unregister_gltf_document_extension(extension)
		extension = null
