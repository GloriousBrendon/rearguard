# SPDX-License-Identifier: MIT OR Apache-2.0

extends "res://tests/test_case.gd"

const AimModel := preload("res://scripts/aim_model.gd")
const Scenario := preload("res://scripts/scenario.gd")


func _deltas(count: int, seed_value: int) -> Array[Vector2]:
	var rng := RandomNumberGenerator.new()
	rng.seed = seed_value
	var out: Array[Vector2] = []
	for i in count:
		out.append(Vector2(rng.randi_range(-40, 40), rng.randi_range(-40, 40)))
	return out


# Acceptance 2: with both hooks at identity, look behaviour is exactly the plain
# "counts times sensitivity" mapping, and matches explicitly installed identity hooks.
func test_identity_look_hook_leaves_behaviour_unchanged() -> void:
	var by_default := AimModel.new()
	var explicit := AimModel.new()
	explicit.look_hook = func(d: PackedFloat64Array) -> PackedFloat64Array: return d
	var k := AimModel.DEFAULT_DEG_PER_COUNT
	var yaw := 0.0
	var pitch := 0.0
	for raw in _deltas(2000, 11):
		by_default.apply_sensitivity(raw)
		explicit.apply_sensitivity(raw)
		yaw += -float(raw.x) * k
		pitch = clampf(pitch + -float(raw.y) * k, -AimModel.PITCH_LIMIT_DEG, AimModel.PITCH_LIMIT_DEG)
		check_eq(by_default.view(), PackedFloat64Array([yaw, pitch]), "default hook vs reference")
		check_eq(explicit.view(), by_default.view(), "explicit identity vs default")


# Acceptance 2, recoil side: the fixed pattern is applied as-is.
func test_identity_recoil_hook_leaves_behaviour_unchanged() -> void:
	var m := AimModel.new()
	var yaw := 0.0
	var pitch := 0.0
	for shot in Scenario.RECOIL_PATTERN.size():
		var kick: Array = Scenario.RECOIL_PATTERN[shot]
		m.apply_recoil(PackedFloat64Array([kick[0], kick[1]]))
		yaw += kick[0]
		pitch += kick[1]
		check_eq(m.view(), PackedFloat64Array([yaw, pitch]), "after shot %d" % shot)


# The hooks really are on the path: a non-identity hook changes the result.
func test_hooks_are_on_the_only_paths() -> void:
	var m := AimModel.new()
	m.look_hook = func(d: PackedFloat64Array) -> PackedFloat64Array:
		return PackedFloat64Array([d[0] * 2.0, d[1] * 2.0])
	m.apply_sensitivity(Vector2(-10, 0))
	check_near(m.yaw_deg, 10 * AimModel.DEFAULT_DEG_PER_COUNT * 2.0, 1e-12, "doubled look")
	m.recoil_hook = func(_d: PackedFloat64Array) -> PackedFloat64Array:
		return PackedFloat64Array([0.0, 0.0])
	var before := m.view()
	m.apply_recoil(PackedFloat64Array([1.0, 1.0]))
	check_eq(m.view(), before, "suppressed recoil")


func test_sign_convention_and_pitch_clamp() -> void:
	var m := AimModel.new()
	m.apply_sensitivity(Vector2(100, 0))
	check(m.yaw_deg < 0.0, "mouse right turns right (yaw decreases)")
	m.apply_sensitivity(Vector2(0, 100))
	check(m.pitch_deg < 0.0, "mouse down looks down")
	m.apply_sensitivity(Vector2(0, -1_000_000))
	check_eq(m.pitch_deg, AimModel.PITCH_LIMIT_DEG, "pitch clamped")
