## Installs the input-probe drift on an AimModel's two hooks (task 1.5).
##
## The multipliers come from the Rust extension (RearguardProbe, crates/rearguard-godot),
## which computes them in rearguard-core from the event timestamp; this script only
## multiplies the angle change by them. The hooks read AimModel.event_us, which
## RangeSession sets to the event's timestamp before each apply_* call.
extends RefCounted


## Whether the extension is loaded.
static func available() -> bool:
	return ClassDB.class_exists("RearguardProbe") and ClassDB.class_exists("RearguardRecorder")


## Routes `aim`'s look and recoil changes through `probe`'s multipliers.
static func install(aim: RefCounted, probe: RefCounted) -> void:
	aim.look_hook = func(d: PackedFloat64Array) -> PackedFloat64Array:
		var k: float = probe.sensitivity_multiplier(aim.event_us)
		return PackedFloat64Array([d[0] * k, d[1] * k])
	aim.recoil_hook = func(d: PackedFloat64Array) -> PackedFloat64Array:
		var k: float = probe.recoil_scale(aim.event_us)
		return PackedFloat64Array([d[0] * k, d[1] * k])
