## Developer overlay (debug builds only): server link, probe, uplink traffic, the
## server's live verdict and frame times, in a corner of the screen.
##
## aim_range.gd creates it only when OS.is_debug_build(), and the extension's verdict and
## traffic accessors exist only in debug builds of the library, so release builds have
## neither. It shows identifiers and numbers, never seed material.
extends CanvasLayer

## Seconds between live verdict requests.
const VERDICT_EVERY_S := 5.0
## Seconds between text refreshes.
const REFRESH_EVERY_S := 0.25

var _range: Node
var _label: Label
var _next_refresh := 0.0
var _next_verdict := 0.0
var _started_ms := -1
var _frames := PackedInt64Array()
var _last_us := 0


func watch(aim_range: Node) -> void:
	_range = aim_range


func _ready() -> void:
	layer = 10
	_label = Label.new()
	_label.name = "DevOverlay"
	_label.position = Vector2(12.0, 70.0)
	_label.add_theme_color_override("font_color", Color(1.0, 0.9, 0.3))
	add_child(_label)


func _process(delta: float) -> void:
	if _range == null:
		return
	var now := Time.get_ticks_usec()
	if _last_us > 0:
		_frames.append(now - _last_us)
		if _frames.size() > 600:
			_frames = _frames.slice(_frames.size() - 600)
	_last_us = now
	_next_refresh -= delta
	_next_verdict -= delta
	var client: RefCounted = _range.client
	if client != null and _next_verdict <= 0.0 and client.has_method("request_verdict"):
		client.request_verdict()
		_next_verdict = VERDICT_EVERY_S
	if _next_refresh <= 0.0:
		_label.text = text()
		_next_refresh = REFRESH_EVERY_S


## The overlay's text (also used by tests).
func text() -> String:
	var lines := PackedStringArray(["DEV OVERLAY (debug build)"])
	var info: Dictionary = _range.run_info
	var client: RefCounted = _range.client
	if client != null:
		lines.append("server %s: %s, session %d, match %s" % [
			info.get("server", ""), client.status(), client.session_id(), client.match_id()])
	else:
		lines.append("server: %s" % info.get("server_status", "not used"))
	lines.append("probe: %s, %d ppm, seed from %s" % [
		"on" if info.get("probe_enabled", false) else "off",
		info.get("probe_amplitude_ppm", 0), info.get("probe_seed", "none")])
	if client != null and client.has_method("stats"):
		var s: Dictionary = client.stats()
		if _started_ms < 0:
			_started_ms = Time.get_ticks_msec()
		var minutes := maxf((Time.get_ticks_msec() - _started_ms) / 60000.0, 1e-6)
		lines.append("uplink: %d records, %.1f KiB sent (%.1f KiB/min), chunks acked %d/%d, resumes %d, dropped %d" % [
			s.records_queued, s.bytes_sent / 1024.0, s.bytes_sent / 1024.0 / minutes,
			s.chunks_acked, s.chunks_sent, s.resumes, s.records_dropped])
	if client != null and client.has_method("verdict"):
		var v: Dictionary = client.verdict()
		if v.is_empty():
			lines.append("verdict: none yet")
		else:
			lines.append("verdict (%s): score %.2f, flagged %s, confidence %.3f, %d windows (%d flagged)" % [
				v.status, v.score, v.flagged, v.confidence, v.windows, v.flagged_windows])
			var e: Dictionary = v.error
			var st: Dictionary = v.steps
			lines.append("  fire-time: %d pairs, r %.3f, slope %.3f, kappa %.2f, z %.1f | steps: %d pairs, z %.1f" % [
				e.pairs, e.r, e.slope, e.kappa, e.z, st.pairs, st.z])
	if _frames.size() > 10:
		var sorted := _frames.duplicate()
		sorted.sort()
		lines.append("frame: p50 %.2f ms, p99 %.2f ms (last %d frames)" % [
			sorted[sorted.size() / 2] / 1000.0, sorted[int(0.99 * (sorted.size() - 1))] / 1000.0, sorted.size()])
	return "\n".join(lines)
