# SPDX-License-Identifier: MIT OR Apache-2.0
## Test cheat (task 1.8), flick scenario: the computed-flick aimbot. 30 ms after a
## target appears (and once the weapon can fire), one injected move of the exact counts
## to the target, then a tap. It re-aims only after a real miss. Mirrors
## `FlickAimbot` in crates/rearguard-sim/src/aimbot.rs. It does not know the probe seed.
extends RefCounted

const Common := preload("res://scripts/test_cheats/common.gd")
const RangeSession := preload("res://scripts/range_session.gd")

## Delay after a target appears, seconds.
var reaction_s := 0.03

var _tap := Common.Tap.new()
var _engaged := -1
var _act_tick := -1


func _init(_seed: int) -> void:
	pass # Deterministic: no noise.


## Returns {"moves": Array[Vector2] of raw counts, "trigger": bool} for this tick.
func plan(session: RangeSession) -> Dictionary:
	var moves: Array[Vector2] = []
	var trigger := false
	if _tap.step(session):
		_act_tick = session.tick + Common.ticks(0.03, session)
	if session.target_index != _engaged:
		_engaged = session.target_index
		_act_tick = session.tick + Common.ticks(reaction_s, session)
	if _act_tick >= 0 and session.tick >= _act_tick and _tap.ready(session):
		_act_tick = -1
		var counts := Common.counts_for(Common.aim_error(session), session.aim.deg_per_count, 1.0)
		if counts != Vector2.ZERO:
			moves.append(counts)
		_tap.press(session)
		trigger = true
	return {"moves": moves, "trigger": trigger}
