extends Logger

var errors: PackedStringArray = []
var _lock := Mutex.new()


func _log_error(function: String, file: String, line: int, code: String, rationale: String, editor_notify: bool,
		error_type: int, script_backtraces: Array[ScriptBacktrace]) -> void:
	_lock.lock()
	errors.append(("%s %s" % [code, rationale]).strip_edges())
	_lock.unlock()


func take() -> PackedStringArray:
	_lock.lock()
	var out := errors
	errors = PackedStringArray()
	_lock.unlock()
	return out
