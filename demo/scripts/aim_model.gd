## View-angle state for the aim range, and the two hook points for the input drift.
##
## Angles are degrees, held as 64-bit floats (Vector2 is 32-bit, so it is used only
## for raw counts, which are exact). Yaw is Godot's rotation.y: positive turns left,
## and it is never wrapped. Pitch is rotation.x: positive looks up, clamped to
## +/- PITCH_LIMIT_DEG.
##
## The view angle changes in exactly two places: apply_sensitivity() and
## apply_recoil(). Each passes its angle change through a hook (look_hook,
## recoil_hook). Both hooks are identity for now; the drift replaces them later.
extends RefCounted

const PITCH_LIMIT_DEG := 89.0
## Degrees per raw count: the common "m_yaw 0.022 at sensitivity 1" convention.
const DEFAULT_DEG_PER_COUNT := 0.022

var deg_per_count: float = DEFAULT_DEG_PER_COUNT
var yaw_deg: float = 0.0
var pitch_deg: float = 0.0

## Look-path hook: takes the view change [d_yaw, d_pitch] in degrees derived from
## one raw delta and returns the change to apply.
var look_hook: Callable = identity
## Recoil-path hook: takes one shot's kick [d_yaw, d_pitch] in degrees and returns
## the kick to apply.
var recoil_hook: Callable = identity


static func identity(delta_deg: PackedFloat64Array) -> PackedFloat64Array:
	return delta_deg


## The one place sensitivity is applied. Raw counts in, resulting view out.
## Mouse right (+x) turns right (yaw decreases); mouse down (+y) looks down.
func apply_sensitivity(raw_counts: Vector2) -> PackedFloat64Array:
	var delta := PackedFloat64Array([
		-float(raw_counts.x) * deg_per_count,
		-float(raw_counts.y) * deg_per_count,
	])
	_add(look_hook.call(delta))
	return view()


## The one place recoil is applied. Kick [d_yaw, d_pitch] in degrees, positive
## pitch kicks the view up. Returns the resulting view.
func apply_recoil(kick_deg: PackedFloat64Array) -> PackedFloat64Array:
	_add(recoil_hook.call(kick_deg))
	return view()


func view() -> PackedFloat64Array:
	return PackedFloat64Array([yaw_deg, pitch_deg])


func _add(delta: PackedFloat64Array) -> void:
	yaw_deg += delta[0]
	pitch_deg = clampf(pitch_deg + delta[1], -PITCH_LIMIT_DEG, PITCH_LIMIT_DEG)
