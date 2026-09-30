## Forwards RangeSession records (rearguard.telemetry dictionaries) to the Rust
## extension's outputs: a RearguardRecorder (a JSON Lines file) or a RearguardClient
## (the server). Both take the same typed record_* calls; this script only unpacks the
## dictionary.
extends RefCounted


## Sends one record to `out`. A recorder opens `path` on the header and closes on the
## end record; a client sends the header as its first record and stays open for the
## verdict.
static func forward(out: RefCounted, r: Dictionary, path: String = "") -> void:
	var recorder := out.is_class("RearguardRecorder")
	if r.type == "header":
		if recorder:
			var err: int = out.open(path, r)
			if err != OK:
				push_error("telemetry: cannot write %s: %s" % [path, error_string(err)])
		else:
			out.record_header(r)
		return
	var t := int(r.ts_us)
	var f := int(r.frame)
	var k := int(r.tick)
	match r.type:
		"move":
			out.record_move(t, f, k, float(r.dx), float(r.dy), r.yaw, r.pitch)
		"button":
			out.record_button(t, f, k, r.pressed)
		"fire":
			out.record_fire(t, f, k, r.shot, r.burst_shot, r.yaw, r.pitch,
					r.target, r.target_yaw, r.target_pitch, r.hit)
		"recoil":
			out.record_recoil(t, f, k, r.shot, r.burst_shot, r.kick_yaw, r.kick_pitch, r.yaw, r.pitch)
		"target":
			out.record_target(t, f, k, r.target, r.target_yaw, r.target_pitch)
		"end":
			out.record_end(t, f, k, r.shots, r.hits, r.complete)
			if recorder:
				out.close()
