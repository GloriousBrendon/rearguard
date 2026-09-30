# SPDX-License-Identifier: MIT OR Apache-2.0
## Test cheat (task 1.8), spray scenario: the recoil macro. The scripted player
## (scripts/scripted_bot.gd) acquires each target and holds the trigger. While it holds,
## it keeps its hand still (tremor only) and the macro injects, after every shot, the
## exact nominal counts that cancel that shot's kick in recoil pattern v1, with the
## rounding remainder carried. Mirrors `RecoilControl::Macro` in
## crates/rearguard-sim/src/human.rs. It does not know the probe seed.
extends RefCounted

const RangeSession := preload("res://scripts/range_session.gd")
const ScriptedBot := preload("res://scripts/scripted_bot.gd")

## Standard deviation of the resting hand's tremor during a burst, counts per tick.
var tremor_counts := 0.3
## Raw events per physics tick, as for the scripted player.
var events_per_tick := 2
## Shots whose kick the macro has cancelled.
var cancelled := 0

var _player: ScriptedBot
var _rng := RandomNumberGenerator.new()
var _shots_seen := 0
var _burst_shot := 0
var _carry := Vector2.ZERO


func _init(p_seed: int) -> void:
	_player = ScriptedBot.new(p_seed)
	_rng.seed = p_seed + 0x5EED


## Returns {"moves": Array[Vector2] of raw counts, "trigger": bool} for this tick.
func plan(session: RangeSession) -> Dictionary:
	var p := _player.plan(session)
	if not session.trigger_down():
		# Acquisition: the player aims and presses.
		_shots_seen = session.shots
		_burst_shot = 0
		_carry = Vector2.ZERO
		return p
	var want := Vector2(_rng.randfn(0.0, tremor_counts), _rng.randfn(0.0, tremor_counts)) + _carry
	while _shots_seen < session.shots:
		# A kick up (+pitch) is cancelled by moving the mouse down (+dy), and a kick left
		# (+yaw) by moving it right (+dx).
		var kick := session.scenario.recoil_kick(_burst_shot)
		want += Vector2(kick[0], kick[1]) / session.aim.deg_per_count
		_shots_seen += 1
		_burst_shot += 1
		cancelled += 1
	var whole := want.round()
	_carry = want - whole
	var moves: Array[Vector2] = []
	var remaining := whole
	for i in events_per_tick:
		var left := events_per_tick - i
		var step := (remaining / left).round() if left > 1 else remaining
		remaining -= step
		if step != Vector2.ZERO:
			moves.append(step)
	return {"moves": moves, "trigger": p.trigger}
