# SPDX-License-Identifier: MIT OR Apache-2.0

## One scenario run: aim state, weapon, targets, scoring and recording. It has no
## scene or rendering dependency, so the headless tests drive it directly; the
## aim_range scene feeds it real (or bot) input events and physics ticks.
##
## Scenario time advances only in step_tick(), one physics tick at a time, so target
## positions depend on the tick count and never on wall-clock time.
extends RefCounted

const AimModel := preload("res://scripts/aim_model.gd")
const Scenario := preload("res://scripts/scenario.gd")
const Recorder := preload("res://scripts/recorder.gd")

## 600 rounds per minute.
const FIRE_INTERVAL_S := 0.1

var aim := AimModel.new()
var scenario: Scenario
var recorder: Recorder
var physics_hz: int
## Called with every record (dictionaries in the rearguard.telemetry v1 schema), in
## order: the telemetry file and the server uplink (see telemetry_out.gd).
var listeners: Array[Callable] = []

var tick := 0
var started := false
var finished := false
var shots := 0
var hits := 0
var target_index := 0

var _trigger_down := false
var _next_fire_tick := 0
var _burst_shot := 0
var _target_since_tick := 0


func _init(p_scenario: Scenario, p_physics_hz: int, p_recorder: Recorder = null) -> void:
	scenario = p_scenario
	physics_hz = p_physics_hz
	recorder = p_recorder


## Starts the run and writes the rearguard.telemetry header. `session_fields` may set
## `match_id` and `player_id` (as the server assigned them; default a local match) and
## `probe_start_us` (default: this start time).
func start(session_fields: Dictionary, ts_us: int, frame: int) -> void:
	if started:
		return
	started = true
	var header := {
		"type": "header",
		"ts_us": ts_us,
		"frame": frame,
		"tick": tick,
		"format": Recorder.FORMAT,
		"version": Recorder.VERSION,
		"match_id": session_fields.get("match_id", "aimrange-local"),
		"player_id": session_fields.get("player_id", 0),
		"source": "client",
		"probe_start_us": session_fields.get("probe_start_us", ts_us),
		"deg_per_count": aim.deg_per_count,
		"physics_hz": physics_hz,
		"scenario": Scenario.kind_name(scenario.kind),
		"scenario_seed": scenario.seed_value,
		"duration_s": scenario.duration_s,
		"recoil_pattern": Scenario.RECOIL_PATTERN_ID,
		"target_distance_m": Scenario.TARGET_DISTANCE_M,
		"target_radius_m": Scenario.TARGET_RADIUS_M,
	}
	_record(header)
	_record_target(ts_us, frame)


func time_s() -> float:
	return float(tick) / physics_hz


func current_target() -> PackedFloat64Array:
	return scenario.target_at(time_s(), target_index)


## One raw relative mouse delta, in counts, before sensitivity.
func handle_motion(raw_counts: Vector2, ts_us: int, frame: int) -> void:
	if not started or finished:
		return
	aim.event_us = ts_us
	var view := aim.apply_sensitivity(raw_counts)
	_record({
		"type": "move", "ts_us": ts_us, "frame": frame, "tick": tick,
		"dx": raw_counts.x, "dy": raw_counts.y,
		"yaw": view[0], "pitch": view[1],
	})


## Fire button pressed or released.
func handle_trigger(pressed: bool, ts_us: int, frame: int) -> void:
	if not started or finished or pressed == _trigger_down:
		return
	_trigger_down = pressed
	_record({
		"type": "button", "ts_us": ts_us, "frame": frame, "tick": tick,
		"pressed": pressed,
	})
	if pressed:
		_try_fire(ts_us, frame)
		return
	# Releasing ends the burst. In spray, a round is one trigger hold.
	if scenario.kind == Scenario.Kind.SPRAY and _burst_shot > 0:
		_advance_target(ts_us, frame)
	_burst_shot = 0


## Advances scenario time by one physics tick.
func step_tick(ts_us: int, frame: int) -> void:
	if not started or finished:
		return
	tick += 1
	if _trigger_down:
		_try_fire(ts_us, frame)
	if scenario.kind == Scenario.Kind.FLICK \
			and tick - _target_since_tick >= roundi(Scenario.FLICK_TIMEOUT_S * physics_hz):
		_advance_target(ts_us, frame)
	if time_s() >= scenario.duration_s:
		_end(ts_us, frame, true)


## Ends the run early (window closed, scene freed). The end record says so.
func abort(ts_us: int, frame: int) -> void:
	if started and not finished:
		_end(ts_us, frame, false)


func _end(ts_us: int, frame: int, complete: bool) -> void:
	finished = true
	_record({
		"type": "end", "ts_us": ts_us, "frame": frame, "tick": tick,
		"shots": shots, "hits": hits, "complete": complete,
	})


func magazine_empty() -> bool:
	var size := scenario.magazine_size()
	return size > 0 and _burst_shot >= size


func trigger_down() -> bool:
	return _trigger_down


## Angle in degrees between the view direction and the given [yaw, pitch].
static func angular_distance_deg(a: PackedFloat64Array, b: PackedFloat64Array) -> float:
	var da := direction(a)
	var db := direction(b)
	var dot := da[0] * db[0] + da[1] * db[1] + da[2] * db[2]
	return rad_to_deg(acos(clampf(dot, -1.0, 1.0)))


## Unit view direction for [yaw, pitch] in degrees (camera looks along -Z at 0, 0).
static func direction(angles: PackedFloat64Array) -> PackedFloat64Array:
	var yaw := deg_to_rad(angles[0])
	var pitch := deg_to_rad(angles[1])
	return PackedFloat64Array([-sin(yaw) * cos(pitch), sin(pitch), -cos(yaw) * cos(pitch)])


func _try_fire(ts_us: int, frame: int) -> void:
	if tick < _next_fire_tick or magazine_empty():
		return
	var view := aim.view()
	var target := current_target()
	var hit := angular_distance_deg(view, target) <= scenario.target_angular_radius_deg()
	_record({
		"type": "fire", "ts_us": ts_us, "frame": frame, "tick": tick,
		"shot": shots, "burst_shot": _burst_shot,
		"yaw": view[0], "pitch": view[1],
		"target": target_index, "target_yaw": target[0], "target_pitch": target[1],
		"hit": hit,
	})
	shots += 1
	if hit:
		hits += 1
	_next_fire_tick = tick + roundi(FIRE_INTERVAL_S * physics_hz)
	var kick := scenario.recoil_kick(_burst_shot)
	_burst_shot += 1
	if kick[0] != 0.0 or kick[1] != 0.0:
		aim.event_us = ts_us
		view = aim.apply_recoil(kick)
		_record({
			"type": "recoil", "ts_us": ts_us, "frame": frame, "tick": tick,
			"shot": shots - 1, "burst_shot": _burst_shot - 1,
			"kick_yaw": kick[0], "kick_pitch": kick[1],
			"yaw": view[0], "pitch": view[1],
		})
	if hit and scenario.kind == Scenario.Kind.FLICK:
		_advance_target(ts_us, frame)


func _advance_target(ts_us: int, frame: int) -> void:
	target_index += 1
	_target_since_tick = tick
	_record_target(ts_us, frame)


func _record_target(ts_us: int, frame: int) -> void:
	if scenario.kind == Scenario.Kind.TRACKING:
		return # The tracking target moves every tick; it is a pure function of the tick.
	var target := current_target()
	_record({
		"type": "target", "ts_us": ts_us, "frame": frame, "tick": tick,
		"target": target_index, "target_yaw": target[0], "target_pitch": target[1],
	})


func _record(record: Dictionary) -> void:
	if recorder != null:
		recorder.write(record)
	for listener in listeners:
		listener.call(record)
