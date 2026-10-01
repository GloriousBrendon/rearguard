# SPDX-License-Identifier: MIT OR Apache-2.0

## Headless test runner. Runs every res://scripts/*_test.gd and exits non-zero on
## any failure:
##   godot --headless --path demo --fixed-fps 120 --script res://tests/run_tests.gd
## Test methods may be coroutines (use await); the runner awaits each one.
## It also fails if fewer tests ran than tests/expected_test_count.txt says, so tests
## cannot disappear unnoticed (a renamed file, a method that lost its test_ prefix).
## Raise that number when adding tests.
extends SceneTree

const TEST_DIR := "res://scripts"
const EXPECTED_COUNT_FILE := "res://tests/expected_test_count.txt"


func _initialize() -> void:
	_run.call_deferred()


func _run() -> void:
	var total := 0
	var failed := PackedStringArray()
	for file in DirAccess.get_files_at(TEST_DIR):
		if not file.ends_with("_test.gd"):
			continue
		var script: GDScript = load(TEST_DIR.path_join(file))
		if script == null or not script.can_instantiate():
			# A parse error would otherwise stop the runner before it can quit.
			print("FAIL  %s: does not compile" % file)
			failed.append(file)
			continue
		for method in script.get_script_method_list():
			var method_name: String = method.name
			if not method_name.begins_with("test_"):
				continue
			var test: RefCounted = script.new()
			test.set("_current", "%s::%s" % [file, method_name])
			test.set("tree", self)
			await test.call(method_name)
			total += 1
			var failures: PackedStringArray = test.get("failures")
			if failures.is_empty():
				print("ok    %s::%s" % [file, method_name])
			else:
				print("FAIL  %s::%s" % [file, method_name])
				for f in failures:
					print("        " + f)
				failed.append("%s::%s" % [file, method_name])
	print("\n%d tests, %d failed" % [total, failed.size()])
	var expected := _expected_count()
	var enough := expected > 0 and total >= expected
	if expected <= 0:
		print("FAIL  %s is missing or holds no positive number" % EXPECTED_COUNT_FILE)
	elif not enough:
		print("FAIL  %d tests ran, expected at least %d (%s)" % [total, expected, EXPECTED_COUNT_FILE])
	quit(1 if not failed.is_empty() or not enough else 0)


## The least number of tests that must run, or 0 if the file is missing or malformed.
func _expected_count() -> int:
	var text := FileAccess.get_file_as_string(EXPECTED_COUNT_FILE).strip_edges()
	return text.to_int() if text.is_valid_int() else 0
