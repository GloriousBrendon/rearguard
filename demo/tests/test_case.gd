## Base class for *_test.gd files. The runner calls every method named test_*.
## No third-party test framework: a failed check records a message and the test
## carries on, so one run reports every failure.
extends RefCounted

var failures: PackedStringArray = []
## The runner's SceneTree, for tests that need to run a scene.
var tree: SceneTree
var _current := ""


func check(condition: bool, message: String) -> void:
	if not condition:
		failures.append("%s: %s" % [_current, message])


func check_eq(actual: Variant, expected: Variant, message: String = "") -> void:
	if actual != expected:
		failures.append("%s: %s expected %s, got %s" % [_current, message, str(expected), str(actual)])


func check_near(actual: float, expected: float, tolerance: float, message: String = "") -> void:
	if absf(actual - expected) > tolerance:
		failures.append("%s: %s expected %s +/- %s, got %s" % [
			_current, message, str(expected), str(tolerance), str(actual)])


## True when the Rust extension is loaded. Otherwise the calling test should return:
## it is skipped, or fails when REARGUARD_REQUIRE_EXTENSION=1 (as in CI).
func extension_loaded() -> bool:
	if ClassDB.class_exists("RearguardProbe") and ClassDB.class_exists("RearguardRecorder"):
		return true
	if OS.get_environment("REARGUARD_REQUIRE_EXTENSION") == "1":
		failures.append("%s: Rust extension not loaded (cargo build -p rearguard-godot)" % _current)
	else:
		print("        skipped %s: Rust extension not built" % _current)
	return false
