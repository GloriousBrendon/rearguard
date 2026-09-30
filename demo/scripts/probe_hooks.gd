# SPDX-License-Identifier: MIT OR Apache-2.0

## Installs the input-probe drift on an AimModel's two hooks (task 1.5).
##
## The multipliers come from the Rust extension (RearguardProbe, crates/rearguard-godot),
## which computes them in rearguard-core from the event timestamp; this script only
## multiplies the angle change by them.
##
## The hooks capture the model's EventClock and the probe, never the model itself:
## the model holds the hooks, so capturing it would be a reference cycle that leaks at
## exit ("resources still in use").
extends RefCounted


## Whether the extension is loaded.
static func available() -> bool:
	return ClassDB.class_exists("RearguardProbe") and ClassDB.class_exists("RearguardRecorder")


## Routes `aim`'s look and recoil changes through `probe`'s multipliers.
static func install(aim: RefCounted, probe: RefCounted) -> void:
	var clock: RefCounted = aim.clock
	aim.look_hook = func(d: PackedFloat64Array) -> PackedFloat64Array:
		var k: float = probe.sensitivity_multiplier(clock.us)
		return PackedFloat64Array([d[0] * k, d[1] * k])
	aim.recoil_hook = func(d: PackedFloat64Array) -> PackedFloat64Array:
		var k: float = probe.recoil_scale(clock.us)
		return PackedFloat64Array([d[0] * k, d[1] * k])
