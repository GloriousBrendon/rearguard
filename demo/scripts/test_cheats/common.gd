# SPDX-License-Identifier: MIT OR Apache-2.0
## Shared pieces of the test cheats (task 1.8). They mirror the simulator's cheat
## models (crates/rearguard-sim/src/aimbot.rs), stepped once per physics tick instead of
## once per millisecond.
extends RefCounted

const RangeSession := preload("res://scripts/range_session.gd")


## Whole counts that turn the view by `delta` [d_yaw, d_pitch] degrees at multiplier
## `gain`. +x turns right (yaw down) and +y looks down, as in aim_model.gd.
static func counts_for(delta: PackedFloat64Array, deg_per_count: float, gain: float) -> Vector2:
	return Vector2(roundf(-delta[0] / (deg_per_count * gain)), roundf(-delta[1] / (deg_per_count * gain)))


## Target minus view, degrees per axis, read from the range's own state the way a
## memory-reading cheat would.
static func aim_error(session: RangeSession) -> PackedFloat64Array:
	var view := session.aim.view()
	var target := session.current_target()
	return PackedFloat64Array([target[0] - view[0], target[1] - view[1]])


## Seconds as whole physics ticks.
static func ticks(seconds: float, session: RangeSession) -> int:
	return maxi(roundi(seconds * session.physics_hz), 0)


## Log-normal with the given mean and standard deviation of the result.
static func lognormal(rng: RandomNumberGenerator, mean: float, sd: float) -> float:
	var s2 := log(1.0 + (sd * sd) / (mean * mean))
	return exp(log(mean) - s2 / 2.0 + sqrt(s2) * rng.randfn(0.0, 1.0))


## Trigger handling shared by the aimbots: taps only when the weapon can fire again
## (600 rounds per minute, plus a tick), and tells a miss (a shot fired and the target
## is still there) from a hit. The trigger is down only on the tick of the tap.
class Tap extends RefCounted:
	## The 100 ms fire interval (12 ticks at 120 Hz) plus one tick.
	var interval_ticks := 13
	var _ready_tick := 0
	var _pressed_tick := -1
	var _pressed_shots := 0
	var _pressed_target := 0

	## Call first on every tick. True if the last tap's shot missed.
	func step(session: RangeSession) -> bool:
		if _pressed_tick < 0:
			return false
		if session.shots > _pressed_shots:
			_pressed_tick = -1
			return session.target_index == _pressed_target
		if session.tick - _pressed_tick > 2 * interval_ticks:
			_pressed_tick = -1 # The tap never fired; do not wait on it forever.
		return false

	func ready(session: RangeSession) -> bool:
		return _pressed_tick < 0 and session.tick >= _ready_tick

	func press(session: RangeSession) -> void:
		_pressed_tick = session.tick
		_pressed_shots = session.shots
		_pressed_target = session.target_index
		_ready_tick = session.tick + interval_ticks
