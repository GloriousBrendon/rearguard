extends "res://tests/test_case.gd"

## The Rust extension (crates/rearguard-godot) against the rearguard-core golden
## vectors. tests/probe_golden.json is generated from core, and a Rust test in
## rearguard-godot checks it byte for byte.

const GOLDEN := "res://tests/probe_golden.json"


func _golden() -> Dictionary:
	return JSON.parse_string(FileAccess.get_file_as_string(GOLDEN))


## The IEEE 754 bits of `x` as 16 hex digits, most significant first.
func _bits(x: float) -> String:
	var b := PackedByteArray()
	b.resize(8)
	b.encode_double(0, x)
	var s := ""
	for i in range(7, -1, -1):
		s += "%02x" % b[i]
	return s


func _probe() -> RefCounted:
	return ClassDB.instantiate("RearguardProbe")


func test_binding_matches_core_golden_vectors() -> void:
	if not extension_loaded():
		return
	var g := _golden()
	var probe := _probe()
	check_eq(probe.start_session(String(g.epoch_seed_hex).hex_decode(), int(g.amplitude_ppm), 0), OK, "start")
	check_eq(g.rows.size(), 20, "golden rows")
	for row in g.rows:
		var stream := int(row.stream)
		var tick := String(row.tick).to_int()
		var what := "stream %d tick %d" % [stream, tick]
		check_eq(probe.drift_ppb(stream, tick), int(row.ppb), what + " ppb")
		check_eq(_bits(probe.multiplier_at_tick(stream, tick)), row.multiplier_bits, what + " multiplier bits")


func test_timestamp_api_uses_the_probe_tick_rule() -> void:
	if not extension_loaded():
		return
	var g := _golden()
	var probe := _probe()
	var start := 5_000_000
	probe.start_session(String(g.epoch_seed_hex).hex_decode(), int(g.amplitude_ppm), start)
	for row in g.rows:
		var tick := String(row.tick).to_int()
		if tick > 1_000_000:
			continue
		# Any timestamp within the tick's millisecond gives that tick's value.
		for ts in [start + tick * 1000, start + tick * 1000 + 999]:
			var k: float = probe.sensitivity_multiplier(ts) if int(row.stream) == 0 else probe.recoil_scale(ts)
			check_eq(_bits(k), row.multiplier_bits, "stream %d ts %d" % [int(row.stream), ts])
	check_eq(probe.probe_start_us(), start)
	check_eq(probe.amplitude_ppm(), 5000)


func test_seed_file_sessions() -> void:
	if not extension_loaded():
		return
	var g := _golden()
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path("user://test"))
	var path := ProjectSettings.globalize_path("user://test/golden_seed.hex")
	var f := FileAccess.open(path, FileAccess.WRITE)
	f.store_string(String(g.epoch_seed_hex) + "\n")
	f.close()
	var probe := _probe()
	check_eq(probe.start_session_from_file(path, false, 5000, 0), OK, "golden seed file")
	for row in g.rows:
		check_eq(_bits(probe.multiplier_at_tick(int(row.stream), String(row.tick).to_int())), row.multiplier_bits)

	var fresh := ProjectSettings.globalize_path("user://test/fresh_seed.hex")
	DirAccess.remove_absolute(fresh)
	check_eq(_probe().start_session_from_file(fresh, false, 5000, 0), ERR_FILE_NOT_FOUND, "missing file")
	var a := _probe()
	check_eq(a.start_session_from_file(fresh, true, 5000, 0), OK, "created")
	check_eq(FileAccess.get_file_as_string(fresh).strip_edges().length(), 64, "64 hex digits")
	var b := _probe()
	check_eq(b.start_session_from_file(fresh, false, 5000, 0), OK, "reread")
	for tick in [0, 777, 123456]:
		check_eq(a.drift_ppb(0, tick), b.drift_ppb(0, tick), "same seed, same drift")


func test_invalid_input_is_refused_and_no_session_is_identity() -> void:
	if not extension_loaded():
		return
	var probe := _probe()
	check_eq(probe.is_active(), false)
	check_eq(probe.sensitivity_multiplier(123), 1.0, "no session")
	check_eq(probe.start_session(PackedByteArray([1, 2, 3]), 5000, 0), ERR_INVALID_DATA, "short seed")
	var seed := String(_golden().epoch_seed_hex).hex_decode()
	check_eq(probe.start_session(seed, 20001, 0), ERR_INVALID_PARAMETER, "amplitude above 2%")
	check_eq(probe.start_session(seed, 5000, -1), ERR_INVALID_PARAMETER, "negative start")
	check_eq(probe.start_session_random(20000, 0), OK, "random")
	check(probe.is_active(), "active")
	check_eq(probe.max_amplitude_ppm(), 20000)
	check_eq(probe.default_amplitude_ppm(), 5000)
	probe.stop()
	check_eq(probe.is_active(), false, "stopped")
	check_eq(probe.recoil_scale(10_000), 1.0, "stopped is identity")


func test_recorder_writes_rearguard_telemetry() -> void:
	if not extension_loaded():
		return
	var path := ProjectSettings.globalize_path("user://test/recorder.telemetry.jsonl")
	DirAccess.make_dir_recursive_absolute(path.get_base_dir())
	var rec: RefCounted = ClassDB.instantiate("RearguardRecorder")
	var header := {
		"ts_us": 1000, "frame": 1, "tick": 0, "match_id": "m", "player_id": 3, "source": "client",
		"probe_start_us": 1000, "deg_per_count": 0.022, "physics_hz": 120, "scenario": "flick",
		"scenario_seed": 7, "duration_s": 60, "recoil_pattern": "v1",
		"target_distance_m": 10.0, "target_radius_m": 0.3,
	}
	var missing := header.duplicate()
	missing.erase("scenario")
	check_eq(rec.open(path, missing), ERR_INVALID_PARAMETER, "missing field")
	check_eq(rec.open(path, header), OK, "open")
	check(rec.record_target(1000, 1, 0, 0, 12.5, -3.0), "target")
	check(rec.record_move(9000, 2, 1, -3.0, 2.0, 0.066, -0.044), "move")
	check(rec.record_button(9004, 2, 1, true), "button")
	check(rec.record_fire(9004, 2, 1, 0, 0, 0.066, -0.044, 0, 12.5, -3.0, false), "fire")
	check(rec.record_end(20000, 3, 2, 1, 0, true), "end")
	check_eq(rec.close(), OK, "close")
	var lines := FileAccess.get_file_as_string(path).strip_edges().split("\n")
	check_eq(lines.size(), 6, "records")
	if lines.size() == 6:
		var h: Dictionary = JSON.parse_string(lines[0])
		check_eq(h.type, "header")
		check_eq(h.format, "rearguard.telemetry")
		check_eq(int(h.version), 1)
		check_eq(int(h.player_id), 3)
		var m: Dictionary = JSON.parse_string(lines[2])
		check_eq(m.type, "move")
		check_eq(m.dx, -3.0)
		check(lines[2].begins_with('{"type":"move","ts_us":9000,'), lines[2])
	var bad: RefCounted = ClassDB.instantiate("RearguardRecorder")
	bad.open(path, header)
	check_eq(bad.record_move(-1, 0, 0, 0.0, 0.0, 0.0, 0.0), false, "negative timestamp refused")
	check_eq(bad.record_button(5, 0, 0, true), false, "stays failed")
