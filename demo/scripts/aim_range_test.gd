extends "res://tests/test_case.gd"

const AimRange := preload("res://scripts/aim_range.gd")
const Recorder := preload("res://scripts/recorder.gd")
const SCENE := preload("res://scenes/aim_range.tscn")


## Runs the real scene with the scripted bot. Bot events go through
## Input.parse_input_event and the scene's _input, like hardware events.
## The probe is off unless `extra` turns it on: its drift depends on the real
## timestamps, so two probed runs never replay identically.
func _run_scene(scenario: String, seed_value: int, out: String, extra: Dictionary = {}) -> Array[Dictionary]:
	var node: AimRange = SCENE.instantiate()
	var c := AimRange.default_config()
	c.merge({"scenario": scenario, "seed": seed_value, "duration": 2.0, "bot": true, "out": out,
			"probe": false}, true)
	c.merge(extra, true)
	node.config = c
	tree.root.add_child(node)
	var path: String = await node.run_finished
	node.queue_free()
	await tree.process_frame
	return Recorder.read_file(path)


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
	c = AimRange.parse_args(PackedStringArray(["--no-probe", "--probe-seed-file", "s.hex",
			"--probe-amplitude-ppm", "20000", "--telemetry", "t.jsonl"]))
	check_eq(c.probe, false)
	check_eq(c.probe_seed_file, "s.hex")
	check_eq(c.probe_amplitude_ppm, 20000)
	check_eq(c.telemetry, "t.jsonl")


func test_headless_bot_run_records_through_input_path() -> void:
	var records := await _run_scene("tracking", 4, "user://test/scene_a.jsonl")
	check(records.size() > 10, "records: %d" % records.size())
	if records.is_empty():
		return
	var header := records[0]
	check_eq(header.source, "bot")
	check_eq(header.accumulated_input, false, "accumulation disabled")
	check_eq(header.delta_source, "InputEventMouseMotion.screen_relative")
	check_eq(records[-1].type, "end")
	var moves := records.filter(func(r: Dictionary) -> bool: return r.type == "move")
	check(moves.size() > 50, "moves reached the session via _input: %d" % moves.size())
	var last := -1
	for r in records:
		check(int(r.ts_us) >= last, "real timestamps monotonic")
		last = int(r.ts_us)


func test_headless_scene_replays_identically() -> void:
	var a := await _run_scene("spray", 11, "user://test/scene_b.jsonl")
	var b := await _run_scene("spray", 11, "user://test/scene_c.jsonl")
	check(a.size() > 10, "records: %d" % a.size())
	check_eq(_strip_timing(a), _strip_timing(b), "same seeds, same run apart from timing")


# Task 1.5 acceptance 3: the aim range applies the drift through the two hooks. A
# second probe loaded from the same seed file recomputes every multiplier from the
# recorded timestamps; replaying the recording with them gives back every angle.
func test_probe_drift_applied_through_hooks() -> void:
	if not extension_loaded():
		return
	var seed_path := "user://test/probe_scene_seed.hex"
	DirAccess.remove_absolute(ProjectSettings.globalize_path(seed_path))
	var telemetry_path := "user://test/scene_probe.telemetry.jsonl"
	var records := await _run_scene("spray", 5, "user://test/scene_probe.jsonl", {
		"probe": true, "probe_seed_file": seed_path, "probe_amplitude_ppm": 20000,
		"telemetry": telemetry_path, "duration": 3.0})
	check(records.size() > 10, "records: %d" % records.size())
	if records.is_empty():
		return
	var header := records[0]
	check_eq(header.probe_enabled, true, "probe enabled")
	check_eq(int(header.probe_amplitude_ppm), 20000)
	check_eq(header.probe_seed, "file")
	check(not JSON.stringify(header).contains(FileAccess.get_file_as_string(
			ProjectSettings.globalize_path(seed_path)).strip_edges()), "seed not in the header")

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

	# The same session in the rearguard.telemetry schema, written by the extension.
	var lines := FileAccess.get_file_as_string(ProjectSettings.globalize_path(telemetry_path)).strip_edges().split("\n")
	check(lines.size() > 10, "telemetry lines: %d" % lines.size())
	if lines.size() > 1:
		var t: Dictionary = JSON.parse_string(lines[0])
		check_eq(t.format, "rearguard.telemetry")
		check_eq(int(t.probe_start_us), int(header.probe_start_us))
		var telemetry_moves := 0
		for line in lines:
			if line.begins_with('{"type":"move"'):
				telemetry_moves += 1
		check_eq(telemetry_moves, moves, "every move mirrored")
		check(lines[-1].begins_with('{"type":"end"'), "ends with end")
