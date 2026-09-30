# SPDX-License-Identifier: MIT OR Apache-2.0
## Test cheat (task 1.8), flick scenario: the humanised aimbot. A log-normal reaction
## of about 200 ms, then a minimum-jerk path (Fitts'-law duration) planned once to a
## random point on the target, with jitter, then a click after a short delay. The path
## is planned from the angles at its start, so its execution is open-loop. After a miss
## it plans a new path. Mirrors `HumanisedAimbot` in crates/rearguard-sim/src/aimbot.rs.
## It does not know the probe seed.
extends RefCounted

const Common := preload("res://scripts/test_cheats/common.gd")
const RangeSession := preload("res://scripts/range_session.gd")

## Per-run parameters, drawn from the simulator's moderate humanisation ranges.
var reaction_s: float
var duration_scale: float
var aim_offset_deg: float
var jitter_counts: float
var click_s: float

var _rng := RandomNumberGenerator.new()
var _tap := Common.Tap.new()
var _engaged := -1
var _plan_tick := -1
var _press_tick := -1
## The planned path: start tick, length in ticks, total counts per axis.
var _path_start := -1
var _path_ticks := 1
var _path_total := PackedFloat64Array([0.0, 0.0])
var _sent := PackedFloat64Array([0.0, 0.0])


func _init(p_seed: int) -> void:
	_rng.seed = p_seed
	reaction_s = _rng.randf_range(0.17, 0.23)
	duration_scale = _rng.randf_range(0.8, 1.2)
	aim_offset_deg = _rng.randf_range(0.10, 0.20)
	jitter_counts = _rng.randf_range(0.1, 0.3)
	click_s = _rng.randf_range(0.04, 0.07)


## Returns {"moves": Array[Vector2] of raw counts, "trigger": bool} for this tick.
func plan(session: RangeSession) -> Dictionary:
	var moves: Array[Vector2] = []
	var trigger := false
	if _tap.step(session):
		_plan_tick = session.tick + Common.ticks(0.1, session) # A miss: plan again.
	if session.target_index != _engaged:
		_engaged = session.target_index
		_path_start = -1
		_press_tick = -1
		_plan_tick = session.tick + Common.ticks(Common.lognormal(_rng, reaction_s, 0.04), session)
	if _plan_tick >= 0 and session.tick >= _plan_tick:
		_plan_tick = -1
		_start_path(session)
	if _path_start >= 0:
		var u := minf(float(session.tick - _path_start) / _path_ticks, 1.0)
		var s := u * u * u * (10.0 - 15.0 * u + 6.0 * u * u)
		var counts := Vector2.ZERO
		for axis in 2:
			var jitter := jitter_counts * _rng.randfn(0.0, 1.0) if u < 1.0 else 0.0
			var want := roundf(_path_total[axis] * s + jitter)
			counts[axis] = want - _sent[axis]
			_sent[axis] = want
		if counts != Vector2.ZERO:
			moves.append(counts)
		if u >= 1.0:
			_path_start = -1
			_press_tick = session.tick + Common.ticks(Common.lognormal(_rng, click_s, 0.01), session)
	if _press_tick >= 0 and session.tick >= _press_tick and _tap.ready(session):
		_press_tick = -1
		_tap.press(session)
		trigger = true
	return {"moves": moves, "trigger": trigger}


func _start_path(session: RangeSession) -> void:
	var error := Common.aim_error(session)
	var delta := PackedFloat64Array([
		error[0] + _rng.randfn(0.0, aim_offset_deg), error[1] + _rng.randfn(0.0, aim_offset_deg)])
	var dist := sqrt(delta[0] * delta[0] + delta[1] * delta[1])
	var seconds := (0.12 + 0.08 * log(1.0 + dist / session.scenario.target_angular_radius_deg()) / log(2.0)) \
			* duration_scale
	var per_count := session.aim.deg_per_count
	_path_start = session.tick
	_path_ticks = maxi(Common.ticks(seconds, session), 1)
	_path_total = PackedFloat64Array([-delta[0] / per_count, -delta[1] / per_count])
	_sent = PackedFloat64Array([0.0, 0.0])
