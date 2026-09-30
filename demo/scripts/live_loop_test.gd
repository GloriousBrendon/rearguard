extends "res://tests/test_case.gd"

## Task 1.7: the aim range against a real rearguard-server process (built with
## `cargo build -p rearguard-server`), plus the exit-leak check.
##
## Tests that need the server binary skip when it has not been built, unless
## REARGUARD_REQUIRE_EXTENSION=1 (as in CI), where they fail instead.

const AimRange := preload("res://scripts/aim_range.gd")
const Recorder := preload("res://scripts/recorder.gd")
const SCENE := preload("res://scenes/aim_range.tscn")

## Test master secret (public test data, not a secret).
const MASTER_HEX := "5e1f00d2c3b4a5968778695a4b3c2d1e0f10213243546576879809a1b2c3d4e5"


func _server_binary() -> String:
	var name := "rearguard-server.exe" if OS.get_name() == "Windows" else "rearguard-server"
	return ProjectSettings.globalize_path("res://").path_join("../target/debug").path_join(name).simplify_path()


func _server_available() -> bool:
	if not extension_loaded():
		return false
	if FileAccess.file_exists(_server_binary()):
		return true
	if OS.get_environment("REARGUARD_REQUIRE_EXTENSION") == "1":
		failures.append("%s: server binary missing (cargo build -p rearguard-server)" % _current)
	else:
		print("        skipped %s: rearguard-server not built" % _current)
	return false


## Stops a server and waits for its log drain.
func _stop_server(server: Dictionary) -> void:
	OS.kill(server.pid)


## Starts the server on a free loopback port; returns {pid, addr, db} or {} on failure.
func _start_server(name: String) -> Dictionary:
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
	var proc: Dictionary = OS.execute_with_pipe(_server_binary(), PackedStringArray(["run", "--config", dir.path_join("server.json")]))
	if proc.is_empty():
		failures.append("%s: could not start the server" % _current)
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
	failures.append("%s: server did not report its address" % _current)
	OS.kill(proc.pid)
	return {}


func _run_scene(extra: Dictionary) -> Dictionary:
	var node: AimRange = SCENE.instantiate()
	var c := AimRange.default_config()
	c.merge({"scenario": "flick", "seed": 5, "duration": 3.0, "bot": true, "probe": true}, true)
	c.merge(extra, true)
	node.config = c
	tree.root.add_child(node)
	var path: String = await node.run_finished
	var result := {"path": path, "run_info": node.run_info.duplicate(), "verdict_line": node.verdict_line}
	node.queue_free()
	await tree.process_frame
	return result


# Acceptance 1 (and 5): a session through the live loop produces telemetry in the core
# schema, a verdict, and stored evidence.
func test_live_session_produces_telemetry_verdict_and_evidence() -> void:
	if not _server_available():
		return
	var server := _start_server("e2e")
	if server.is_empty():
		return
	var out := "user://test/live_e2e.jsonl"
	var r := await _run_scene({"server": server.addr, "out": out, "duration": 5.0})
	_stop_server(server)
	var info: Dictionary = r.run_info
	check_eq(info.server_status, "connected", "connected to %s" % server.addr)
	check_eq(info.probe_seed, "server", "the server's seed drives the drift")
	check_eq(info.probe_enabled, true)
	check(r.verdict_line.contains("finished") and r.verdict_line.contains("verdict score"), r.verdict_line)
	var records := Recorder.read_file(ProjectSettings.globalize_path(out))
	check(records.size() > 50, "telemetry records: %d" % records.size())
	if records.is_empty():
		return
	var header := records[0]
	check_eq(header.format, "rearguard.telemetry")
	check(str(header.match_id).begins_with("m-"), "server-assigned match id: %s" % header.match_id)
	check_eq(records[-1].type, "end")
	# The evidence is in the server's database.
	var session_id := int(r.verdict_line.get_slice("server session ", 1).get_slice(":", 0))
	var output := []
	var code := OS.execute(_server_binary(), PackedStringArray(["verdict", "--db", server.db, "--session", str(session_id)]), output, true)
	check_eq(code, 0, "stored verdict for session %d" % session_id)
	var stored := "".join(output)
	check(stored.contains("\"status\": \"Finished\""), stored)
	check(stored.contains("\"angle_mismatches\": 0"), "the server's replay matches every angle the game computed")
	check(stored.contains("\"records\": %d" % (records.size())), "server stored every record: %s" % stored.get_slice("\"records\"", 1).get_slice(",", 0))
	# No seed material anywhere a player or operator can read.
	var env_text := FileAccess.get_file_as_string(ProjectSettings.globalize_path(out) + ".env.json")
	check(not env_text.contains(MASTER_HEX) and not stored.contains(MASTER_HEX), "master secret not shown")


# Acceptance 4: with no server to reach, the run goes offline (local seed) and still
# records a complete session; there is simply no verdict.
func test_unreachable_server_falls_back_to_offline() -> void:
	if not extension_loaded():
		return
	var out := "user://test/live_unreachable.jsonl"
	# Port 9 (discard) on loopback: nothing listens there.
	var r := await _run_scene({"server": "127.0.0.1:9", "out": out})
	var info: Dictionary = r.run_info
	check_eq(info.server_status, "unreachable")
	check_eq(info.probe_seed, "random", "the drift still runs, from a local seed")
	check(r.verdict_line.contains("no server session (unreachable)"), r.verdict_line)
	var records := Recorder.read_file(ProjectSettings.globalize_path(out))
	check(records.size() > 20 and records[-1].type == "end" and records[-1].complete, "complete local recording")
	if not records.is_empty():
		check_eq(records[0].match_id, "aimrange-local")


# Acceptance 4: the server dies mid-session. The game is never blocked: it finishes,
# keeps its complete local recording, and reports that no verdict came.
func test_server_lost_mid_session() -> void:
	if not _server_available():
		return
	var server := _start_server("lost")
	if server.is_empty():
		return
	var node: AimRange = SCENE.instantiate()
	var c := AimRange.default_config()
	c.merge({"scenario": "spray", "seed": 2, "duration": 6.0, "bot": true, "probe": true,
			"server": server.addr, "out": "user://test/live_lost.jsonl", "verdict_timeout_s": 1.5}, true)
	node.config = c
	tree.root.add_child(node)
	# Let the session get going, then kill the server.
	while node.session == null or node.session.tick < 120:
		await tree.process_frame
	_stop_server(server)
	var started := Time.get_ticks_msec()
	var path: String = await node.run_finished
	var took := Time.get_ticks_msec() - started
	var line: String = node.verdict_line
	var info: Dictionary = node.run_info.duplicate()
	node.queue_free()
	await tree.process_frame
	check_eq(info.server_status, "connected", "started connected")
	check(not line.contains("verdict score"), "no verdict after losing the server: %s" % line)
	check(line.contains("reconnecting") or line.contains("lost"), line)
	check(took < 20000, "finished promptly (%d ms)" % took)
	var records := Recorder.read_file(path)
	check(records.size() > 50 and records[-1].type == "end" and records[-1].complete, "complete local recording")


## Runs the aim range in a separate Godot process and returns its exit code and output.
func _child_run(extra_args: PackedStringArray) -> Dictionary:
	var args := PackedStringArray(["--headless", "--path", ProjectSettings.globalize_path("res://"),
			"--fixed-fps", "120", "--", "--bot", "--scenario", "spray", "--duration", "2",
			"--out", ProjectSettings.globalize_path("user://test/child.jsonl")])
	args.append_array(extra_args)
	var output := []
	var code := OS.execute(OS.get_executable_path(), args, output, true)
	return {"code": code, "text": "".join(output)}


# Acceptance 6: with the probe (and the uplink) on, the process exits cleanly: no
# leaked objects or resources reported at exit.
func test_process_exits_without_leaks_with_the_probe_on() -> void:
	if not extension_loaded():
		return
	var runs := [PackedStringArray()]
	var server := {}
	if _server_available():
		server = _start_server("leak")
		if not server.is_empty():
			runs.append(PackedStringArray(["--server", server.addr]))
	for extra in runs:
		var r := _child_run(extra)
		var what := "probe with %s" % ("server" if extra.size() > 0 else "local seed")
		check_eq(r.code, 0, what + " exit status")
		check(r.text.contains("finished"), what + " ran: " + r.text.right(300))
		for bad in ["leaked at exit", "still in use at exit", "ERROR"]:
			check(not r.text.contains(bad), "%s: '%s' in output: %s" % [what, bad, r.text.right(400)])
	if not server.is_empty():
		_stop_server(server)
