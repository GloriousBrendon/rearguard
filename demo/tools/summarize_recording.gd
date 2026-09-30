## Prints input statistics for a recording, to compare display drivers and platforms:
##   godot --headless --path demo --script res://tools/summarize_recording.gd -- FILE.jsonl
##
## Raw, unaccelerated input shows whole-number deltas whose sum depends only on how
## far the mouse moved. Accelerated or scaled input shows fractional deltas and sums
## that grow with speed (see "Checking a platform" in README.md).
extends SceneTree

const Recorder := preload("res://scripts/recorder.gd")


func _initialize() -> void:
	var args := OS.get_cmdline_user_args()
	if args.is_empty():
		printerr("usage: ... --script res://tools/summarize_recording.gd -- FILE.jsonl")
		quit(2)
		return
	for path in args:
		_summarize(path)
	quit(0)


func _summarize(path: String) -> void:
	var records := Recorder.read_file(path)
	if records.is_empty() or records[0].get("type") != "header" or records[0].get("format") != Recorder.FORMAT:
		printerr("%s: not a rearguard.telemetry recording" % path)
		return
	var moves := records.filter(func(r: Dictionary) -> bool: return r.type == "move")
	var per_frame := {}
	var sum := Vector2.ZERO
	var abs_sum := Vector2.ZERO
	var fractional := 0
	var gaps: Array[int] = []
	var last_ts := -1
	for r: Dictionary in moves:
		var d := Vector2(r.dx, r.dy)
		sum += d
		abs_sum += d.abs()
		if d.x != roundf(d.x) or d.y != roundf(d.y):
			fractional += 1
		per_frame[r.frame] = per_frame.get(r.frame, 0) + 1
		if last_ts >= 0:
			gaps.append(int(r.ts_us) - last_ts)
		last_ts = int(r.ts_us)
	var frame_counts: Array = per_frame.values()
	frame_counts.sort()
	gaps.sort()
	var same_ts := gaps.filter(func(g: int) -> bool: return g < 50).size()
	print("%s" % path)
	# The runtime details live next to the recording (<recording>.env.json).
	var env: Variant = JSON.parse_string(FileAccess.get_file_as_string(path + ".env.json"))
	if not env is Dictionary:
		env = {}
	print("  source %s, driver %s, os %s, godot %s, accumulated_input %s" % [
		env.get("source", "?"), env.get("display_driver", "?"), env.get("os", "?"),
		env.get("godot_version", "?"), env.get("accumulated_input", "?")])
	print("  move events %d over %d frames; per frame median %s, max %s" % [
		moves.size(), per_frame.size(),
		frame_counts[frame_counts.size() / 2] if not frame_counts.is_empty() else 0,
		frame_counts.max() if not frame_counts.is_empty() else 0])
	print("  sum dx %.3f dy %.3f; sum |dx| %.3f |dy| %.3f; fractional deltas %d" % [
		sum.x, sum.y, abs_sum.x, abs_sum.y, fractional])
	if not gaps.is_empty():
		print("  gap between events (us): median %d, p99 %d; %d of %d under 50 us (same batch)" % [
			gaps[gaps.size() / 2], gaps[mini(gaps.size() - 1, gaps.size() * 99 / 100)],
			same_ts, gaps.size()])
