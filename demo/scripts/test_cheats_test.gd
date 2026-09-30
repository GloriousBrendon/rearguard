# SPDX-License-Identifier: MIT OR Apache-2.0
extends "res://tests/test_case.gd"

## Task 1.8: the test cheats. Their environment guard, their refusal outside the test
## environment, each one playing a scenario end to end against a real rearguard-server,
## and their ground-truth labels staying out of the telemetry.

const AimRange := preload("res://scripts/aim_range.gd")
const Recorder := preload("res://scripts/recorder.gd")
const LiveServer := preload("res://tests/live_server.gd")
const Cheats := preload("res://scripts/test_cheats/cheats.gd")
const TestEnv := preload("res://scripts/test_cheats/test_env.gd")
const SCENE := preload("res://scenes/aim_range.tscn")

const ON := {TestEnv.ENV_FLAG: "1"}


## Sets (or with "" unsets) the guard's variables for this process; returns the old
## values for _restore_env.
func _set_env(values: Dictionary) -> Dictionary:
	var old := TestEnv.process_environment()
	for k in [TestEnv.ENV_FLAG, TestEnv.ENV_ALLOWLIST]:
		var v := str(values.get(k, ""))
		if v.is_empty():
			OS.unset_environment(k)
		else:
			OS.set_environment(k, v)
	return old


func _restore_env(old: Dictionary) -> void:
	_set_env(old)


func test_guard_needs_the_flag_and_a_loopback_or_allowlisted_server() -> void:
	check(not TestEnv.refusal("127.0.0.1:7461", {}).is_empty(), "no REARGUARD_TEST_ENV")
	check(not TestEnv.refusal("127.0.0.1:7461", {TestEnv.ENV_FLAG: "0"}).is_empty(), "REARGUARD_TEST_ENV=0")
	check(not TestEnv.refusal("127.0.0.1:7461", {TestEnv.ENV_FLAG: "true"}).is_empty(), "only 1 counts")
	check(not TestEnv.refusal("", ON).is_empty(), "no server")
	for ok in ["127.0.0.1:7461", "127.10.20.30:1", "[::1]:7461"]:
		check_eq(TestEnv.refusal(ok, ON), "", ok)
	for bad in ["192.0.2.1:7461", "localhost:7461", "127.0.0.1.example.com:7461", "10.0.0.1:7461",
			"[2001:db8::1]:7461", "127.0.0.256:1", "0.0.0.0:7461"]:
		check(not TestEnv.refusal(bad, ON).is_empty(), "%s must be refused" % bad)
	var listed := {TestEnv.ENV_FLAG: "1", TestEnv.ENV_ALLOWLIST: " 192.0.2.1 , 198.51.100.7:9000,"}
	check_eq(TestEnv.refusal("192.0.2.1:7461", listed), "", "allowlisted host")
	check_eq(TestEnv.refusal("198.51.100.7:9000", listed), "", "allowlisted host:port")
	check(not TestEnv.refusal("198.51.100.7:9001", listed).is_empty(), "other port of an allowlisted host:port")
	check(not TestEnv.refusal("192.0.2.2:7461", listed).is_empty(), "host not on the list")


func test_every_cheat_checks_the_guard_scenario_and_name() -> void:
	for cheat in Cheats.names():
		var scenario: String = Cheats.CHEATS[cheat][1]
		var c := {"cheat": cheat, "scenario": scenario, "server": "127.0.0.1:7461"}
		check_eq(Cheats.refusal(c, ON), "", cheat)
		check(Cheats.refusal(c, {}).contains(TestEnv.ENV_FLAG), "%s without the flag" % cheat)
		c.server = "203.0.113.5:7461"
		check(Cheats.refusal(c, ON).contains("neither loopback"), "%s against a remote server" % cheat)
		c.server = "127.0.0.1:7461"
		c.scenario = "tracking"
		check(Cheats.refusal(c, ON).contains("scenario"), "%s in the wrong scenario" % cheat)
		check_eq(Cheats.label_for(cheat), "cheat:" + cheat)
	check(Cheats.refusal({"cheat": "wallhack", "scenario": "flick", "server": "127.0.0.1:1"}, ON)
			.contains("unknown test cheat"), "unknown name")


func test_parse_args_cheat() -> void:
	var c := AimRange.parse_args(PackedStringArray(["--cheat", "flick-aimbot", "--server", "127.0.0.1:7461"]))
	check_eq(c.cheat, "flick-aimbot")
	check_eq(c.bot, true, "a cheat replaces the mouse")
	check_eq(c.quit, true)


## Runs the aim range in a separate Godot process; returns {code, text}.
func _child_run(args: PackedStringArray) -> Dictionary:
	var all := PackedStringArray(["--headless", "--path", ProjectSettings.globalize_path("res://"),
			"--fixed-fps", "120", "--"])
	all.append_array(args)
	var output := []
	var code := OS.execute(OS.get_executable_path(), all, output, true)
	return {"code": code, "text": "".join(output)}


# Acceptance 2: without the environment guard, a cheat exits with an error before
# anything runs.
func test_cheat_exits_with_an_error_without_the_guard() -> void:
	var old := _set_env({})
	for cheat in Cheats.names():
		var out := ProjectSettings.globalize_path("user://test/refused-%s.jsonl" % cheat)
		DirAccess.remove_absolute(out)
		var r := _child_run(PackedStringArray(["--cheat", cheat, "--scenario", Cheats.CHEATS[cheat][1],
				"--server", "127.0.0.1:9", "--duration", "2", "--out", out]))
		check_eq(r.code, AimRange.EXIT_REFUSED, "%s exit status without %s: %s" % [cheat, TestEnv.ENV_FLAG, r.text.right(300)])
		check(r.text.contains("test cheat '%s' refused" % cheat) and r.text.contains(TestEnv.ENV_FLAG),
				"%s says why: %s" % [cheat, r.text.right(300)])
		check(not FileAccess.file_exists(out), "%s recorded nothing" % cheat)
	# With the flag but a server that is not on this machine: refused too.
	_set_env(ON)
	var r := _child_run(PackedStringArray(["--cheat", "flick-aimbot", "--server", "192.0.2.1:7461", "--duration", "2"]))
	check_eq(r.code, AimRange.EXIT_REFUSED, "remote server: " + r.text.right(300))
	check(r.text.contains("neither loopback"), r.text.right(300))
	_restore_env(old)


# The same refusal in process: no session starts and no input is injected.
func test_refused_cheat_starts_no_session() -> void:
	var old := _set_env({})
	var node: AimRange = SCENE.instantiate()
	var c := AimRange.default_config()
	c.merge({"cheat": "flick-aimbot", "bot": true, "server": "127.0.0.1:9", "record": false}, true)
	node.config = c
	tree.root.add_child(node)
	check(node.refusal.contains(TestEnv.ENV_FLAG), "refusal: %s" % node.refusal)
	check(node.bot == null and node.session == null, "no bot and no session")
	node.queue_free()
	await tree.process_frame
	_restore_env(old)


# Acceptance 1 (and 3's client side): each cheat plays its scenario end to end against
# the server: a server-seeded drift, a final verdict, stored evidence with every angle
# replayed, and its ground-truth label in the server session's metadata only.
func test_each_cheat_completes_a_scenario_against_the_server() -> void:
	if not LiveServer.available(self):
		return
	var server := LiveServer.start(self, "cheats")
	if server.is_empty():
		return
	var old := _set_env(ON)
	var played := {}
	for cheat in Cheats.names():
		var out := "user://test/cheat-%s.jsonl" % cheat
		var node: AimRange = SCENE.instantiate()
		var c := AimRange.default_config()
		c.merge({"cheat": cheat, "scenario": Cheats.CHEATS[cheat][1], "seed": 4, "duration": 8.0,
				"bot": true, "bot_seed": 3, "server": server.addr, "out": out}, true)
		node.config = c
		tree.root.add_child(node)
		var path: String = await node.run_finished
		var bot: RefCounted = node.bot
		var info: Dictionary = node.run_info.duplicate()
		var line: String = node.verdict_line
		var hits: int = node.session.hits
		var shots: int = node.session.shots
		node.queue_free()
		await tree.process_frame
		played[cheat] = int(line.get_slice("server session ", 1).get_slice(":", 0))
		print("        %s: %d/%d hits; %s" % [cheat, hits, shots, line.get_slice(": ", 1)])

		check_eq(info.server_status, "connected", cheat)
		check_eq(info.probe_seed, "server", "%s: the server's seed drives the drift" % cheat)
		check(line.contains("finished") and line.contains("verdict score"), "%s: %s" % [cheat, line])
		# The humanised aimbot takes about 0.65 s per target, so 8 s gives it about a dozen.
		check(shots >= 8 and hits * 10 >= shots * 7, "%s: %d/%d hits" % [cheat, hits, shots])
		match cheat:
			"recoil-macro":
				check(bot.cancelled >= shots - 1 and bot.cancelled > 0, "macro cancelled %d of %d kicks" % [bot.cancelled, shots])
			"adaptive-aimbot":
				check(bot.measurements > 5, "adaptive measurements: %d" % bot.measurements)
				check_near(bot.estimate, 1.0, 0.03, "adaptive multiplier estimate")

		var records := Recorder.read_file(ProjectSettings.globalize_path(out))
		check(records.size() > 20 and records[-1].type == "end" and records[-1].complete, "%s: complete recording" % cheat)
		# The label is never in the telemetry (the recording is exactly what was streamed).
		var text := FileAccess.get_file_as_string(ProjectSettings.globalize_path(out))
		check(not text.contains(cheat) and not text.contains("cheat:"), "%s: no label in the telemetry" % cheat)
		var env: Variant = JSON.parse_string(FileAccess.get_file_as_string(ProjectSettings.globalize_path(out) + ".env.json"))
		check(env is Dictionary and env.ground_truth == Cheats.label_for(cheat) and env.cheat == cheat,
				"%s: local metadata %s" % [cheat, env])

		var stored := LiveServer.cli(PackedStringArray(["verdict", "--db", server.db, "--session", str(played[cheat])]))
		check_eq(stored.code, 0, "%s: stored verdict" % cheat)
		check(stored.text.contains("\"status\": \"Finished\""), "%s: %s" % [cheat, stored.text])
		check(stored.text.contains("\"angle_mismatches\": 0"), "%s: the server replays every angle" % cheat)
		check(stored.text.contains("\"records\": %d" % records.size()), "%s: the server has every record" % cheat)
	LiveServer.stop(server)
	_restore_env(old)

	# The evaluation harness's listing: each session with its label.
	var listing := LiveServer.cli(PackedStringArray(["labels", "--db", server.db]))
	check_eq(listing.code, 0, "labels listing")
	# Session ids use up to 63 bits, more than a JSON number parsed as a double keeps,
	# so read them as text.
	var labels := {}
	var id_re := RegEx.create_from_string("\"session_id\":(\\d+)")
	var label_re := RegEx.create_from_string("\"label\":\"([^\"]*)\"")
	for l in listing.text.split("\n", false):
		var id := id_re.search(l)
		var lab := label_re.search(l)
		if id != null and lab != null:
			labels[id.get_string(1)] = lab.get_string(1)
	for cheat in played:
		check_eq(labels.get(str(played[cheat]), ""), Cheats.label_for(cheat), "%s: label stored with its session" % cheat)
