## Scripted synthetic player for headless runs and CI. It sees the session state
## (its own view and the current target) and produces raw mouse counts and a
## trigger state, which the scene injects through the normal input path.
##
## It is a plain aiming controller with a reaction delay and seeded noise; it reads
## nothing outside this range and is not a cheat. Deterministic for a given seed.
extends RefCounted

const RangeSession := preload("res://scripts/range_session.gd")

## Fraction of the remaining aim error corrected per physics tick.
var gain := 0.25
## Ticks to wait before reacting to a new target.
var reaction_ticks := 24
## Standard deviation of per-tick noise, in counts.
var noise_counts := 1.0
## Raw events per physics tick; > 1 mimics a high polling rate.
var events_per_tick := 2
## Fire when within this fraction of the target's angular radius.
var fire_threshold := 0.6

var _rng := RandomNumberGenerator.new()
var _last_target := -1
var _wait := 0
var _residual := Vector2.ZERO


func _init(p_seed: int) -> void:
	_rng.seed = p_seed


## Returns {"moves": Array[Vector2] of raw counts, "trigger": bool} for this tick.
func plan(session: RangeSession) -> Dictionary:
	var moves: Array[Vector2] = []
	if session.target_index != _last_target:
		_last_target = session.target_index
		_wait = reaction_ticks
	var view := session.aim.view()
	var target := session.current_target()
	if _wait > 0:
		_wait -= 1
		return {"moves": moves, "trigger": false}

	# Counts that would close `gain` of the error; +x turns right (yaw down) and
	# +y looks down, as in aim_model.gd.
	var per_count := session.aim.deg_per_count
	var want := Vector2(
		-(target[0] - view[0]) / per_count,
		-(target[1] - view[1]) / per_count,
	) * gain
	want += Vector2(_rng.randfn(0.0, noise_counts), _rng.randfn(0.0, noise_counts))
	want += _residual
	var whole := want.round()
	_residual = want - whole

	# Split into integer events, like a mouse reporting several times per tick.
	var remaining := whole
	for i in events_per_tick:
		var left := events_per_tick - i
		var step := (remaining / left).round() if left > 1 else remaining
		remaining -= step
		if step != Vector2.ZERO:
			moves.append(step)

	var err := RangeSession.angular_distance_deg(view, target)
	var on_target := err <= session.scenario.target_angular_radius_deg() * fire_threshold
	# Keep the trigger held through a spray until the magazine is empty.
	var trigger := on_target or (session.trigger_down() and session.scenario.magazine_size() > 0)
	if session.magazine_empty():
		trigger = false
	return {"moves": moves, "trigger": trigger}
