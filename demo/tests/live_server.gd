# SPDX-License-Identifier: MIT OR Apache-2.0
## A real rearguard-server process for the Godot tests (built with
## `cargo build -p rearguard-server`), on a free loopback port with a fresh database.
## Used by scripts/live_loop_test.gd and scripts/test_cheats_test.gd.
extends RefCounted

## Test master secret (public test data, not a secret).
const MASTER_HEX := "5e1f00d2c3b4a5968778695a4b3c2d1e0f10213243546576879809a1b2c3d4e5"


static func binary() -> String:
	var name := "rearguard-server.exe" if OS.get_name() == "Windows" else "rearguard-server"
	return ProjectSettings.globalize_path("res://").path_join("../target/debug").path_join(name).simplify_path()


## True if the extension and the server binary are there. Otherwise the calling test
## should return: it is skipped, or fails when REARGUARD_REQUIRE_EXTENSION=1 (as in CI).
static func available(test: RefCounted) -> bool:
	if not test.extension_loaded():
		return false
	if FileAccess.file_exists(binary()):
		return true
	if OS.get_environment("REARGUARD_REQUIRE_EXTENSION") == "1":
		test.failures.append("%s: server binary missing (cargo build -p rearguard-server)" % test._current)
	else:
		print("        skipped %s: rearguard-server not built" % test._current)
	return false


## Starts the server; returns {pid, addr, db, proc} or {} (with a failure) on error.
static func start(test: RefCounted, name: String) -> Dictionary:
	var dir := ProjectSettings.globalize_path("user://test/live-%s" % name)
	DirAccess.make_dir_recursive_absolute(dir)
	for f in ["evidence.sqlite3", "evidence.sqlite3-journal"]:
		DirAccess.remove_absolute(dir.path_join(f))
	var secret := FileAccess.open(dir.path_join("master.hex"), FileAccess.WRITE)
	secret.store_string(MASTER_HEX + "\n")
	secret.close()
	var example := FileAccess.get_file_as_string(
			ProjectSettings.globalize_path("res://").path_join("../crates/rearguard-server/config/server.example.json").simplify_path())
	var config := example.replace("127.0.0.1:7461", "127.0.0.1:0") \
			.replace("server-master.hex", "master.hex").replace("rearguard.sqlite3", "evidence.sqlite3")
	var cf := FileAccess.open(dir.path_join("server.json"), FileAccess.WRITE)
	cf.store_string(config)
	cf.close()
	var proc: Dictionary = OS.execute_with_pipe(binary(), PackedStringArray(["run", "--config", dir.path_join("server.json")]))
	if proc.is_empty():
		test.failures.append("%s: could not start the server" % test._current)
		return {}
	var stderr: FileAccess = proc.stderr
	var started := Time.get_ticks_msec()
	while Time.get_ticks_msec() - started < 20000:
		var line := stderr.get_line()
		if line.contains("listening on "):
			# Keep `proc` (and so the server's pipes) alive with the server: closing its
			# stderr pipe would make every later log line a write error.
			return {"pid": proc.pid, "addr": line.get_slice("listening on ", 1).strip_edges(),
					"db": dir.path_join("evidence.sqlite3"), "proc": proc}
		if stderr.eof_reached():
			break
	test.failures.append("%s: server did not report its address" % test._current)
	OS.kill(proc.pid)
	return {}


static func stop(server: Dictionary) -> void:
	OS.kill(server.pid)


## Runs `rearguard-server ARGS...`; returns {code, text}.
static func cli(args: PackedStringArray) -> Dictionary:
	var output := []
	var code := OS.execute(binary(), args, output, true)
	return {"code": code, "text": "".join(output)}
