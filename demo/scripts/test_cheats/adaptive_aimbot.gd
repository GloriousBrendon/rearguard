# SPDX-License-Identifier: MIT OR Apache-2.0
## Test cheat (task 1.8), flick scenario: the adaptive aimbot. A computed flick that
## knows a small sensitivity drift exists, but not the seed. After each flick it reads
## the view change its own move produced (view change / nominal change, from the game),
## keeps a running estimate of the multiplier, and divides the next flick's counts by
## it. Human-like reaction delays. Mirrors `AdaptiveAimbot` in
## crates/rearguard-sim/src/aimbot.rs.
extends RefCounted

const Common := preload("res://scripts/test_cheats/common.gd")
const RangeSession := preload("res://scripts/range_session.gd")

## Mean reaction after a target appears, seconds (log-normal, sd 0.03 s).
var reaction_s := 0.15
## Weight of the newest measurement in the running estimate.
var smoothing := 0.5
## The current multiplier estimate.
var estimate := 1.0
## Flicks measured so far.
var measurements := 0

var _rng := RandomNumberGenerator.new()
var _tap := Common.Tap.new()
var _engaged := -1
var _act_tick := -1
## A flick whose outcome is not yet measured: its tick, the view before it and its
## nominal view change.
var _pending_tick := -1
var _before := PackedFloat64Array()
var _nominal := PackedFloat64Array()


func _init(p_seed: int) -> void:
	_rng.seed = p_seed


## Returns {"moves": Array[Vector2] of raw counts, "trigger": bool} for this tick.
func plan(session: RangeSession) -> Dictionary:
	var moves: Array[Vector2] = []
	var trigger := false
	if _tap.step(session):
		_act_tick = session.tick + Common.ticks(0.03, session)
	# The flick was applied when it was injected, so its effect is visible now.
	if _pending_tick >= 0 and session.tick > _pending_tick:
		_pending_tick = -1
		var after := session.aim.view()
		var actual := PackedFloat64Array([after[0] - _before[0], after[1] - _before[1]])
		var nn := _nominal[0] * _nominal[0] + _nominal[1] * _nominal[1]
		if nn > 0.0:
			var observed := (actual[0] * _nominal[0] + actual[1] * _nominal[1]) / nn
			estimate += smoothing * (observed - estimate)
			measurements += 1
	if session.target_index != _engaged:
		_engaged = session.target_index
		_act_tick = session.tick + Common.ticks(Common.lognormal(_rng, reaction_s, 0.03), session)
	if _act_tick >= 0 and session.tick >= _act_tick and _pending_tick < 0 and _tap.ready(session):
		_act_tick = -1
		var per_count := session.aim.deg_per_count
		var counts := Common.counts_for(Common.aim_error(session), per_count, estimate)
		if counts != Vector2.ZERO:
			moves.append(counts)
			_pending_tick = session.tick
			_before = session.aim.view()
			_nominal = PackedFloat64Array([-counts.x * per_count, -counts.y * per_count])
		_tap.press(session)
		trigger = true
	return {"moves": moves, "trigger": trigger}
