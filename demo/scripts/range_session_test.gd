extends "res://tests/test_case.gd"

const AimModel := preload("res://scripts/aim_model.gd")
const RangeSession := preload("res://scripts/range_session.gd")
const Recorder := preload("res://scripts/recorder.gd")
const Scenario := preload("res://scripts/scenario.gd")
const ScriptedBot := preload("res://scripts/scripted_bot.gd")

const HZ := 120


## Runs a scenario with the scripted bot, driving the session directly (no scene),
## with synthetic but strictly increasing timestamps. Returns the records as emitted
## (exact values, in the rearguard.telemetry schema).
func _run_bot(kind: Scenario.Kind, seed_value: int, duration_s: float,
		look_hook: Callable = Callable(), recoil_hook: Callable = Callable()) -> Array[Dictionary]:
	var rec := Recorder.new()
	var s := RangeSession.new(Scenario.new(kind, seed_value, duration_s), HZ, rec)
	if look_hook.is_valid():
		s.aim.look_hook = look_hook
	if recoil_hook.is_valid():
		s.aim.recoil_hook = recoil_hook
	var ts := 1000
	s.start({}, ts, 0)
	var bot := ScriptedBot.new(7)
	while not s.finished:
		var p := bot.plan(s)
		for m: Vector2 in p.moves:
			ts += 250
			s.handle_motion(m, ts, s.tick)
		if p.trigger != s.trigger_down():
			ts += 250
			s.handle_trigger(p.trigger, ts, s.tick)
		ts += 250
		s.step_tick(ts, s.tick)
	return rec.records



func _types(records: Array[Dictionary]) -> Dictionary:
	var counts := {}
	for r in records:
		counts[r.type] = counts.get(r.type, 0) + 1
	return counts


# A replay of the same seeds is identical, record for record.
func test_same_seeds_same_run() -> void:
	for kind: Scenario.Kind in Scenario.Kind.values():
		var a := _run_bot(kind, 5, 4.0)
		var b := _run_bot(kind, 5, 4.0)
		check_eq(a, b, Scenario.kind_name(kind))


# Acceptance 2 at session level: explicit identity hooks change nothing, and every
# recorded view angle equals the closed-form result from the recorded raw deltas
# and the fixed recoil pattern (what a server-side replay would compute).
func test_identity_hooks_leave_session_unchanged() -> void:
	var identity := func(d: PackedFloat64Array) -> PackedFloat64Array: return d
	for kind: Scenario.Kind in Scenario.Kind.values():
		var name := Scenario.kind_name(kind)
		var records := _run_bot(kind, 9, 5.0)
		check_eq(_run_bot(kind, 9, 5.0, identity, identity), records, name + " explicit identity")
		var k: float = records[0].deg_per_count
		var yaw := 0.0
		var pitch := 0.0
		var checked := 0
		for r in records:
			if r.type == "move":
				yaw += -float(r.dx) * k
				pitch = clampf(pitch + -float(r.dy) * k, -AimModel.PITCH_LIMIT_DEG, AimModel.PITCH_LIMIT_DEG)
			elif r.type == "recoil":
				var kick: Array = Scenario.RECOIL_PATTERN[int(r.burst_shot)]
				check_eq([r.kick_yaw, r.kick_pitch], kick, name + " pattern kick")
				yaw += float(kick[0])
				pitch = clampf(pitch + float(kick[1]), -AimModel.PITCH_LIMIT_DEG, AimModel.PITCH_LIMIT_DEG)
			else:
				continue
			checked += 1
			if r.yaw != yaw or r.pitch != pitch:
				check_eq([r.yaw, r.pitch], [yaw, pitch], "%s record %d" % [name, checked])
				break
		check(checked > 100, "%s replayed %d records" % [name, checked])


# Acceptance 6: per input event the raw delta, a monotonic timestamp and the view
# angle, plus fire events; header first, end last.
func test_recording_contents() -> void:
	for kind: Scenario.Kind in Scenario.Kind.values():
		var name := Scenario.kind_name(kind)
		var exact := _run_bot(kind, 3, 5.0)
		var records := exact
		var header := records[0]
		check_eq(header.type, "header", name)
		check_eq(header.format, Recorder.FORMAT, name)
		check_eq(int(header.version), Recorder.VERSION, name)
		check_eq(header.scenario, name, name)
		check_eq(header.source, "client", name)
		check_eq(header.match_id, "aimrange-local", name + " offline match id")
		check_eq(int(header.probe_start_us), int(header.ts_us), name + " probe starts with the session")
		for key in ["player_id", "deg_per_count", "physics_hz", "duration_s", "recoil_pattern",
				"target_distance_m", "target_radius_m"]:
			check(header.has(key), "%s header has %s" % [name, key])
		check_eq(int(header.scenario_seed), 3, name)
		check_eq(records[-1].type, "end", name)
		check_eq(records[-1].complete, true, name + " complete")
		var counts := _types(records)
		check(counts.get("move", 0) > 50, "%s moves: %s" % [name, counts])
		check(counts.get("fire", 0) > 0, "%s fires: %s" % [name, counts])
		check(counts.get("button", 0) > 0, "%s buttons: %s" % [name, counts])
		if kind == Scenario.Kind.SPRAY:
			check(counts.get("recoil", 0) > 0, "spray recoils: %s" % counts)
		var last_ts := -1
		for r in records:
			for key in ["type", "ts_us", "frame", "tick"]:
				check(r.has(key), "%s %s has %s" % [name, r.type, key])
			check(int(r.ts_us) >= last_ts, "%s timestamps monotonic" % name)
			last_ts = int(r.ts_us)
			if r.type == "move":
				for key in ["dx", "dy", "yaw", "pitch"]:
					check(r.has(key), "%s move has %s" % [name, key])
			elif r.type == "fire":
				for key in ["shot", "yaw", "pitch", "target", "hit"]:
					check(r.has(key), "%s fire has %s" % [name, key])


func test_scoring_counts_hits_and_shots() -> void:
	for kind: Scenario.Kind in Scenario.Kind.values():
		var records := _run_bot(kind, 3, 5.0)
		var fires := records.filter(func(r: Dictionary) -> bool: return r.type == "fire")
		var hits := fires.filter(func(r: Dictionary) -> bool: return r.hit)
		check_eq(int(records[-1].shots), fires.size(), Scenario.kind_name(kind) + " shots")
		check_eq(int(records[-1].hits), hits.size(), Scenario.kind_name(kind) + " hits")
		check(hits.size() > 0, Scenario.kind_name(kind) + " bot hits something")


func test_spray_round_is_one_trigger_hold_capped_at_magazine() -> void:
	var records := _run_bot(Scenario.Kind.SPRAY, 3, 8.0)
	var per_target := {}
	for r in records:
		if r.type == "fire":
			per_target[r.target] = per_target.get(r.target, 0) + 1
	check(per_target.size() >= 2, "several rounds: %s" % per_target)
	for t in per_target:
		check(per_target[t] <= 30, "round %s fired %d" % [t, per_target[t]])


func test_hit_geometry() -> void:
	var r := Scenario.new(Scenario.Kind.FLICK, 1, 1.0).target_angular_radius_deg()
	check_near(RangeSession.angular_distance_deg(
			PackedFloat64Array([0.0, 0.0]), PackedFloat64Array([r * 0.99, 0.0])), r * 0.99, 1e-6, "yaw")
	check_near(RangeSession.angular_distance_deg(
			PackedFloat64Array([10.0, 20.0]), PackedFloat64Array([10.0, 20.0])), 0.0, 1e-5, "same")


func test_abort_writes_incomplete_end() -> void:
	var rec := Recorder.new()
	var s := RangeSession.new(Scenario.new(Scenario.Kind.FLICK, 1, 10.0), HZ, rec)
	s.start({}, 0, 0)
	s.step_tick(1, 1)
	s.abort(2, 2)
	s.abort(3, 3)
	check_eq(rec.records.size(), 3, "header, target, one end")
	check_eq(rec.records[-1].type, "end")
	check_eq(rec.records[-1].complete, false)
	check(s.finished, "no further input after abort")
