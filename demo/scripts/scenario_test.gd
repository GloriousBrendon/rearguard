extends "res://tests/test_case.gd"

const Scenario := preload("res://scripts/scenario.gd")

const HZ := 120


func _positions(kind: Scenario.Kind, seed_value: int, duration_s: float) -> Array[PackedFloat64Array]:
	var s := Scenario.new(kind, seed_value, duration_s)
	var out: Array[PackedFloat64Array] = []
	if kind == Scenario.Kind.TRACKING:
		for tick in roundi(duration_s * HZ) + 1:
			out.append(s.target_at(float(tick) / HZ, 0))
	else:
		for i in s.static_targets.size():
			out.append(s.target_at(0.0, i))
	return out


# Acceptance 3: the same scenario seed gives the same target positions.
func test_same_seed_same_positions() -> void:
	for kind: Scenario.Kind in Scenario.Kind.values():
		var a := _positions(kind, 42, 30.0)
		var b := _positions(kind, 42, 30.0)
		check(a.size() > 0, "%s has targets" % Scenario.kind_name(kind))
		check_eq(a, b, Scenario.kind_name(kind))


func test_different_seed_different_positions() -> void:
	for kind: Scenario.Kind in Scenario.Kind.values():
		check(_positions(kind, 42, 30.0) != _positions(kind, 43, 30.0), Scenario.kind_name(kind))


# Pinned values for seed 42. These catch a change in Godot's RNG, in the scenario
# generator, or a platform difference; update them only on a deliberate change.
func test_pinned_positions_seed_42() -> void:
	var flick := Scenario.new(Scenario.Kind.FLICK, 42, 30.0)
	var spray := Scenario.new(Scenario.Kind.SPRAY, 42, 30.0)
	var tracking := Scenario.new(Scenario.Kind.TRACKING, 42, 30.0)
	var got := {
		"flick0": flick.target_at(0.0, 0), "flick1": flick.target_at(0.0, 1),
		"spray0": spray.target_at(0.0, 0),
		"tracking_2.5s": tracking.target_at(2.5, 0),
	}
	var want := PINNED
	for key: String in want:
		var w: Array = want[key]
		var g: PackedFloat64Array = got[key]
		check_near(g[0], w[0], 1e-9, key + " yaw")
		check_near(g[1], w[1], 1e-9, key + " pitch")


const PINNED := {
	"flick0": [-30.53038406372070312, 9.77097129821777344],
	"flick1": [-16.14505577087402344, -8.87209892272949219],
	"spray0": [-7.63259601593017578, 3.29516196250915527],
	"tracking_2.5s": [11.9061125280379656, -1.05303847789764404],
}


func test_tracking_stays_in_lane_and_moves_continuously() -> void:
	for seed_value in [1, 2, 3, 42, 1000]:
		var s := Scenario.new(Scenario.Kind.TRACKING, seed_value, 60.0)
		var prev := s.target_at(0.0, 0)
		var max_step := Scenario.TRACKING_SPEED_MAX_DEG_S / HZ + 1e-9
		for tick in range(1, 60 * HZ + 1):
			var p := s.target_at(float(tick) / HZ, 0)
			check(absf(p[0]) <= Scenario.TRACKING_YAW_RANGE_DEG, "in lane, seed %d" % seed_value)
			check(absf(p[0] - prev[0]) <= max_step, "no jump at tick %d, seed %d" % [tick, seed_value])
			prev = p


func test_recoil_pattern_is_fixed_and_spray_only() -> void:
	var a := Scenario.new(Scenario.Kind.SPRAY, 1, 10.0)
	var b := Scenario.new(Scenario.Kind.SPRAY, 999, 10.0)
	check_eq(a.magazine_size(), 30, "magazine")
	for shot in 30:
		check_eq(a.recoil_kick(shot), b.recoil_kick(shot), "shot %d independent of seed" % shot)
		check(a.recoil_kick(shot) != PackedFloat64Array([0.0, 0.0]), "shot %d kicks" % shot)
	var flick := Scenario.new(Scenario.Kind.FLICK, 1, 10.0)
	check_eq(flick.recoil_kick(0), PackedFloat64Array([0.0, 0.0]), "no recoil in flick")
	check_eq(flick.magazine_size(), 0, "no magazine in flick")
