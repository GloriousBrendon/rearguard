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
