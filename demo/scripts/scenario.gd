# SPDX-License-Identifier: MIT OR Apache-2.0

## Scenario definitions. Every target position is a pure function of the scenario
## kind and seed (and, for tracking, the scenario time), so a seed replays exactly.
##
## Targets are placed by view angle [yaw_deg, pitch_deg] at TARGET_DISTANCE_M from
## the player, using the same angle convention as aim_model.gd.
extends RefCounted

enum Kind { TRACKING, FLICK, SPRAY }

const KIND_NAMES := {"tracking": Kind.TRACKING, "flick": Kind.FLICK, "spray": Kind.SPRAY}

const TARGET_DISTANCE_M := 10.0
const TARGET_RADIUS_M := 0.3

## Flick: static targets, one at a time; the next appears on a hit or timeout.
const FLICK_TARGET_COUNT := 256
const FLICK_YAW_RANGE_DEG := 40.0
const FLICK_PITCH_MIN_DEG := -10.0
const FLICK_PITCH_MAX_DEG := 20.0
const FLICK_TIMEOUT_S := 2.5

## Tracking: one target strafing left and right in piecewise-linear segments.
const TRACKING_YAW_RANGE_DEG := 30.0
const TRACKING_SEGMENT_MIN_S := 0.3
const TRACKING_SEGMENT_MAX_S := 1.2
const TRACKING_SPEED_MIN_DEG_S := 15.0
const TRACKING_SPEED_MAX_DEG_S := 50.0
const TRACKING_PITCH_MIN_DEG := -2.0
const TRACKING_PITCH_MAX_DEG := 6.0

## Spray: one static target per round; a round is one trigger hold.
const SPRAY_ROUND_COUNT := 64
const SPRAY_YAW_RANGE_DEG := 10.0
const SPRAY_PITCH_MIN_DEG := 0.0
const SPRAY_PITCH_MAX_DEG := 5.0

## Fixed recoil pattern, version 1: per-shot kick [d_yaw, d_pitch] in degrees for a
## 30-round magazine. Climbs for ten shots, then pulls left, then right. The seed
## never changes it.
const RECOIL_PATTERN_ID := "v1"
const RECOIL_PATTERN: Array = [
	[0.00, 0.90], [0.05, 1.00], [-0.05, 1.10], [0.05, 1.10], [0.00, 1.00],
	[-0.05, 0.90], [0.05, 0.80], [0.00, 0.70], [0.05, 0.60], [0.00, 0.50],
	[0.35, 0.30], [0.40, 0.30], [0.40, 0.25], [0.35, 0.25], [0.30, 0.20],
	[0.30, 0.20], [0.25, 0.20], [0.25, 0.15], [0.20, 0.15], [0.20, 0.15],
	[-0.40, 0.20], [-0.45, 0.20], [-0.45, 0.15], [-0.40, 0.15], [-0.35, 0.15],
	[-0.35, 0.10], [-0.30, 0.10], [-0.30, 0.10], [-0.25, 0.10], [-0.25, 0.10],
]

var kind: Kind
var seed_value: int
var duration_s: float

## Flick and spray: [yaw, pitch] per target index.
var static_targets: Array[PackedFloat64Array] = []
## Tracking: [start_s, start_yaw_deg, speed_deg_s] per segment, start_s ascending.
var tracking_segments: Array[PackedFloat64Array] = []
var tracking_pitch_deg: float = 0.0


static func kind_from_name(kind_name: String) -> int:
	return KIND_NAMES.get(kind_name, -1)


static func kind_name(k: Kind) -> String:
	return KIND_NAMES.find_key(k)


func _init(p_kind: Kind, p_seed: int, p_duration_s: float) -> void:
	kind = p_kind
	seed_value = p_seed
	duration_s = p_duration_s
	var rng := RandomNumberGenerator.new()
	rng.seed = p_seed
	match kind:
		Kind.FLICK:
			for i in FLICK_TARGET_COUNT:
				static_targets.append(PackedFloat64Array([
					rng.randf_range(-FLICK_YAW_RANGE_DEG, FLICK_YAW_RANGE_DEG),
					rng.randf_range(FLICK_PITCH_MIN_DEG, FLICK_PITCH_MAX_DEG),
				]))
		Kind.SPRAY:
			for i in SPRAY_ROUND_COUNT:
				static_targets.append(PackedFloat64Array([
					rng.randf_range(-SPRAY_YAW_RANGE_DEG, SPRAY_YAW_RANGE_DEG),
					rng.randf_range(SPRAY_PITCH_MIN_DEG, SPRAY_PITCH_MAX_DEG),
				]))
		Kind.TRACKING:
			tracking_pitch_deg = rng.randf_range(TRACKING_PITCH_MIN_DEG, TRACKING_PITCH_MAX_DEG)
			_build_tracking_segments(rng)


func target_angular_radius_deg() -> float:
	return rad_to_deg(atan(TARGET_RADIUS_M / TARGET_DISTANCE_M))


## Target [yaw, pitch] for the given scenario time and target index. Tracking uses
## only the time; flick and spray use only the index.
func target_at(time_s: float, index: int) -> PackedFloat64Array:
	if kind == Kind.TRACKING:
		return _tracking_target_at(time_s)
	return static_targets[index % static_targets.size()]


## Recoil kick for the given shot of a burst. Only spray has recoil.
func recoil_kick(shot_in_burst: int) -> PackedFloat64Array:
	if kind != Kind.SPRAY or shot_in_burst >= RECOIL_PATTERN.size():
		return PackedFloat64Array([0.0, 0.0])
	var k: Array = RECOIL_PATTERN[shot_in_burst]
	return PackedFloat64Array([k[0], k[1]])


func magazine_size() -> int:
	return RECOIL_PATTERN.size() if kind == Kind.SPRAY else 0


func _build_tracking_segments(rng: RandomNumberGenerator) -> void:
	var t := 0.0
	var yaw := 0.0
	while t < duration_s:
		var seg_s := rng.randf_range(TRACKING_SEGMENT_MIN_S, TRACKING_SEGMENT_MAX_S)
		var speed := rng.randf_range(TRACKING_SPEED_MIN_DEG_S, TRACKING_SPEED_MAX_DEG_S)
		if rng.randi_range(0, 1) == 0:
			speed = -speed
		# Keep the target inside the lane: turn back if the segment would leave it,
		# and shorten the segment if it would still overshoot.
		if absf(yaw + speed * seg_s) > TRACKING_YAW_RANGE_DEG:
			speed = -speed
		var end_yaw := yaw + speed * seg_s
		if absf(end_yaw) > TRACKING_YAW_RANGE_DEG:
			var bound := signf(speed) * TRACKING_YAW_RANGE_DEG
			seg_s = (bound - yaw) / speed
			end_yaw = bound
		tracking_segments.append(PackedFloat64Array([t, yaw, speed]))
		t += seg_s
		yaw = end_yaw


func _tracking_target_at(time_s: float) -> PackedFloat64Array:
	# Binary search for the last segment starting at or before time_s.
	var lo := 0
	var hi := tracking_segments.size() - 1
	while lo < hi:
		var mid := (lo + hi + 1) >> 1
		if tracking_segments[mid][0] <= time_s:
			lo = mid
		else:
			hi = mid - 1
	var seg := tracking_segments[lo]
	var elapsed := maxf(time_s - seg[0], 0.0)
	# Past the final segment the target keeps its end position.
	if lo == tracking_segments.size() - 1:
		elapsed = minf(elapsed, maxf(duration_s - seg[0], 0.0))
	var yaw := clampf(seg[1] + seg[2] * elapsed, -TRACKING_YAW_RANGE_DEG, TRACKING_YAW_RANGE_DEG)
	return PackedFloat64Array([yaw, tracking_pitch_deg])
