extends "res://tests/test_case.gd"

const AimRange := preload("res://scripts/aim_range.gd")
const AimModel := preload("res://scripts/aim_model.gd")
const ProbeHooks := preload("res://scripts/probe_hooks.gd")
const Recorder := preload("res://scripts/recorder.gd")
const SCENE := preload("res://scenes/aim_range.tscn")


## Runs the real scene with the scripted bot. Bot events go through
## Input.parse_input_event and the scene's _input, like hardware events.
## The probe is off unless `extra` turns it on: its drift depends on the real
## timestamps, so two probed runs never replay identically. Returns the telemetry the
## extension wrote (empty without the extension) and fills `last_node_info`.
func _run_scene(scenario: String, seed_value: int, out: String, extra: Dictionary = {}) -> Array[Dictionary]:
	var node: AimRange = SCENE.instantiate()
	var c := AimRange.default_config()
	c.merge({"scenario": scenario, "seed": seed_value, "duration": 2.0, "bot": true, "out": out,
			"probe": false}, true)
	c.merge(extra, true)
	node.config = c
	tree.root.add_child(node)
	var path: String = await node.run_finished
	last_node_info = {"run_info": node.run_info.duplicate(), "verdict_line": node.verdict_line}
	node.queue_free()
	await tree.process_frame
	if not FileAccess.file_exists(path):
		return []
	return Recorder.read_file(path)


var last_node_info := {}


func _environment(out: String) -> Dictionary:
	var env: Variant = JSON.parse_string(FileAccess.get_file_as_string(ProjectSettings.globalize_path(out) + ".env.json"))
	return env if env is Dictionary else {}


func _strip_timing(records: Array[Dictionary]) -> Array[Dictionary]:
	var out: Array[Dictionary] = []
	for r in records:
		var d := r.duplicate()
		d.erase("ts_us")
		d.erase("frame")
		d.erase("probe_start_us") # A timestamp too (header only).
		out.append(d)
	return out


func test_parse_args() -> void:
	var c := AimRange.parse_args(PackedStringArray(
			["--scenario", "spray", "--seed", "9", "--duration", "5", "--bot", "--out", "x.jsonl"]))
	check_eq(c.scenario, "spray")
	check_eq(c.seed, 9)
	check_eq(c.duration, 5.0)
	check_eq(c.bot, true)
	check_eq(c.quit, true, "bot quits when done")
	check_eq(c.out, "x.jsonl")
	check_eq(c.probe, true, "probe on by default")
	check_eq(c.record, true, "recording on by default")
	check_eq(c.server, "", "offline by default")
	c = AimRange.parse_args(PackedStringArray(["--no-probe", "--probe-seed-file", "s.hex",
			"--probe-amplitude-ppm", "20000", "--server", "127.0.0.1:7461", "--no-record",
			"--frame-stats", "f.json"]))
	check_eq(c.probe, false)
	check_eq(c.probe_seed_file, "s.hex")
	check_eq(c.probe_amplitude_ppm, 20000)
	check_eq(c.server, "127.0.0.1:7461")
	check_eq(c.record, false)
	check_eq(c.frame_stats, "f.json")


# Task 1.7 acceptance 6 (regression): the probe's hooks must not keep the aim model
# alive. They used to capture the model they were stored on, a reference cycle that
# leaked at exit ("resources still in use"). No extension needed: a fake probe will do.
class FakeProbe extends RefCounted:
	func sensitivity_multiplier(_ts: int) -> float:
		return 1.01

	func recoil_scale(_ts: int) -> float:
		return 0.99


func test_probe_hooks_do_not_keep_the_model_alive() -> void:
	var aim := AimModel.new()
	ProbeHooks.install(aim, FakeProbe.new())
	aim.event_us = 5
	aim.apply_sensitivity(Vector2(-100, 0))
	check(absf(aim.view()[0] - 100 * AimModel.DEFAULT_DEG_PER_COUNT * 1.01) < 1e-12, "hook applied")
	var model: WeakRef = weakref(aim)
	aim = null
	check(model.get_ref() == null, "the model is freed once released")


func test_headless_bot_run_records_through_input_path() -> void:
	if not extension_loaded():
		return
	var out := "user://test/scene_a.jsonl"
	var records := await _run_scene("tracking", 4, out)
	check(records.size() > 10, "records: %d" % records.size())
	if records.is_empty():
		return
	var header := records[0]
	check_eq(header.format, "rearguard.telemetry", "the core telemetry schema")
	check_eq(int(header.version), 1)
	check_eq(header.source, "client")
	var env := _environment(out)
	check_eq(env.get("source"), "bot")
	check_eq(env.get("accumulated_input"), false, "accumulation disabled")
	check_eq(env.get("delta_source"), "InputEventMouseMotion.screen_relative")
	check_eq(records[-1].type, "end")
	var moves := records.filter(func(r: Dictionary) -> bool: return r.type == "move")
	check(moves.size() > 50, "moves reached the session via _input: %d" % moves.size())
	var last := -1
	for r in records:
		check(int(r.ts_us) >= last, "real timestamps monotonic")
		last = int(r.ts_us)


func test_headless_scene_replays_identically() -> void:
	if not extension_loaded():
		return
	var a := await _run_scene("spray", 11, "user://test/scene_b.jsonl")
	var b := await _run_scene("spray", 11, "user://test/scene_c.jsonl")
	check(a.size() > 10, "records: %d" % a.size())
	check_eq(_strip_timing(a), _strip_timing(b), "same seeds, same run apart from timing")


# Task 1.5 acceptance 3: the aim range applies the drift through the two hooks. A
# second probe loaded from the same seed file recomputes every multiplier from the
# recorded timestamps; replaying the telemetry with them gives back every angle.
func test_probe_drift_applied_through_hooks() -> void:
	if not extension_loaded():
		return
	var seed_path := "user://test/probe_scene_seed.hex"
	DirAccess.remove_absolute(ProjectSettings.globalize_path(seed_path))
	var out := "user://test/scene_probe.jsonl"
	var records := await _run_scene("spray", 5, out, {
		"probe": true, "probe_seed_file": seed_path, "probe_amplitude_ppm": 20000, "duration": 3.0})
	check(records.size() > 10, "records: %d" % records.size())
	if records.is_empty():
		return
	var header := records[0]
	var env := _environment(out)
	check_eq(env.get("probe_enabled"), true, "probe enabled")
	check_eq(int(env.get("probe_amplitude_ppm", 0)), 20000)
	check_eq(env.get("probe_seed"), "file")
	var seed_hex := FileAccess.get_file_as_string(ProjectSettings.globalize_path(seed_path)).strip_edges()
	check(seed_hex.length() == 64, "seed file written")
	check(not FileAccess.get_file_as_string(ProjectSettings.globalize_path(out)).contains(seed_hex), "seed not in the telemetry")
	check(not JSON.stringify(env).contains(seed_hex), "seed not in the environment file")

	var probe: RefCounted = ClassDB.instantiate("RearguardProbe")
	check_eq(probe.start_session_from_file(ProjectSettings.globalize_path(seed_path), false, 20000,
			int(header.probe_start_us)), OK, "reload seed")
	var dpc := float(header.deg_per_count)
	var yaw := 0.0
	var pitch := 0.0
	var nominal_yaw := 0.0
	var nominal_pitch := 0.0
	var moves := 0
	var recoils := 0
	var worst := 0.0
	for r in records:
		var k := 1.0
		var d := PackedFloat64Array()
		if r.type == "move":
			k = probe.sensitivity_multiplier(int(r.ts_us))
			d = PackedFloat64Array([-float(r.dx) * dpc, -float(r.dy) * dpc])
			moves += 1
		elif r.type == "recoil":
			k = probe.recoil_scale(int(r.ts_us))
			d = PackedFloat64Array([float(r.kick_yaw), float(r.kick_pitch)])
			recoils += 1
		else:
			continue
		yaw += d[0] * k
		pitch = clampf(pitch + d[1] * k, -89.0, 89.0)
		nominal_yaw += d[0]
		nominal_pitch = clampf(nominal_pitch + d[1], -89.0, 89.0)
		# Godot's JSON parser can be one ulp off, so allow a hair of tolerance.
		worst = maxf(worst, maxf(absf(yaw - float(r.yaw)), absf(pitch - float(r.pitch))))
	check(moves > 50 and recoils > 5, "moves %d recoils %d" % [moves, recoils])
	check(worst < 1e-9, "replay with the probe's multipliers matches (worst %s)" % worst)
	var drift := maxf(absf(nominal_yaw - yaw), absf(nominal_pitch - pitch))
	check(drift > 1e-4, "the drift really moved the view (%s deg)" % drift)
	check(records[-1].type == "end" and records[-1].complete == true, "complete end record")
