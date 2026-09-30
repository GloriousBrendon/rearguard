## Aim range scene: builds the world, captures raw mouse input, drives a
## RangeSession and draws it. The input path is documented in demo/README.md.
##
## The session's telemetry (rearguard.telemetry v1) is written to a file by the Rust
## extension and, with --server, streamed live to a Rearguard server, whose session
## seed then drives the input-probe drift. See demo/README.md, "Live loop".
##
## Command-line options come after `--`, for example:
##   godot --path demo -- --scenario spray --seed 7 --server 127.0.0.1:7461
##   godot --headless --path demo --fixed-fps 120 -- --bot --scenario flick --seed 7 --duration 10
extends Node3D

const RangeSession := preload("res://scripts/range_session.gd")
const Scenario := preload("res://scripts/scenario.gd")
const AimModel := preload("res://scripts/aim_model.gd")
const ScriptedBot := preload("res://scripts/scripted_bot.gd")
const ProbeHooks := preload("res://scripts/probe_hooks.gd")
const TelemetryOut := preload("res://scripts/telemetry_out.gd")

## Marks events the scripted bot injects, so bot and hardware input never mix.
const BOT_DEVICE_ID := 0x7E57
const EYE_HEIGHT_M := 1.7

signal run_finished(recording_path: String)

## Run options. Set before the node enters the tree to skip command-line parsing.
var config: Dictionary = {}
var session: RangeSession
var bot: ScriptedBot
var recording_path := ""
## The Rust extension's RearguardProbe while the input probe runs, else null.
var probe: RefCounted = null
## The Rust extension's RearguardClient while connected to a server, else null.
var client: RefCounted = null
## The telemetry file writer (RearguardRecorder), else null.
var telemetry_file: RefCounted = null
## What the run used: probe source and amplitude, server status. Never the seed.
var run_info := {}
## The final line printed about the server's verdict, once known.
var verdict_line := ""

var _camera: Camera3D
var _target: MeshInstance3D
var _hud: Label
var _frame_seen := -1
var _events_this_frame := 0
var _max_events_per_frame := 0
var _motion_events := 0
var _waiting_since_ms := -1
var _frame_times := PackedInt64Array()
var _last_frame_us := 0


static func default_config() -> Dictionary:
	return {
		"scenario": "flick",
		"seed": 1,
		"duration": 60.0,
		"sens": AimModel.DEFAULT_DEG_PER_COUNT,
		"bot": false,
		"bot_seed": 1,
		"out": "",
		"quit": false,
		"probe": true,
		"probe_seed_file": "",
		"probe_amplitude_ppm": 5000,
		"server": "",
		"record": true,
		"frame_stats": "",
		"verdict_timeout_s": 10.0,
		# Study mode (scripts/study.gd): derive the probe's seed from a study key file,
		# with this match id (also the header's match id).
		"probe_study_key": "",
		"probe_match_id": "",
		# The developer overlay (debug builds only). A study turns it off: it would show
		# the drift amplitude and break blinding.
		"overlay": true,
		# Write <recording>.env.json next to the recording.
		"environment_file": true,
		# A first HUD line, for example "Session 2 of 6".
		"hud_title": "",
	}


## Parses `--scenario NAME --seed N --duration S --sens DEG --bot --bot-seed N
## --out PATH --quit --no-probe --probe-seed-file PATH --probe-amplitude-ppm N
## --server HOST:PORT --no-record --frame-stats PATH`. Unknown options are reported and
## ignored.
static func parse_args(args: PackedStringArray) -> Dictionary:
	var c := default_config()
	var i := 0
	while i < args.size():
		var arg := args[i]
		var value := args[i + 1] if i + 1 < args.size() else ""
		match arg:
			"--scenario": c.scenario = value; i += 1
			"--seed": c.seed = value.to_int(); i += 1
			"--duration": c.duration = value.to_float(); i += 1
			"--sens": c.sens = value.to_float(); i += 1
			"--bot-seed": c.bot_seed = value.to_int(); i += 1
			"--out": c.out = value; i += 1
			"--probe-seed-file": c.probe_seed_file = value; i += 1
			"--probe-amplitude-ppm": c.probe_amplitude_ppm = value.to_int(); i += 1
			"--server": c.server = value; i += 1
			"--frame-stats": c.frame_stats = value; i += 1
			"--no-probe": c.probe = false
			"--no-record": c.record = false
			"--bot": c.bot = true; c.quit = true
			"--quit": c.quit = true
			"--no-quit": c.quit = false
			_: push_warning("aim_range: unknown option %s" % arg)
		i += 1
	return c


func _ready() -> void:
	# Deliver every OS motion event separately instead of merging them per frame.
	Input.use_accumulated_input = false
	if config.is_empty():
		config = parse_args(OS.get_cmdline_user_args())
	var kind := Scenario.kind_from_name(config.scenario)
	if kind < 0:
		push_error("aim_range: unknown scenario '%s' (tracking, flick, spray)" % config.scenario)
		get_tree().quit(2)
		return

	recording_path = config.out
	if recording_path.is_empty():
		recording_path = "user://recordings/%s-seed%d-%s.jsonl" % [
			config.scenario, config.seed, Time.get_datetime_string_from_system(true).replace(":", "")]
	recording_path = ProjectSettings.globalize_path(recording_path)

	var scenario := Scenario.new(kind, config.seed, config.duration)
	session = RangeSession.new(scenario, Engine.physics_ticks_per_second)
	session.aim.deg_per_count = config.sens
	_build_world()
	# The developer overlay exists only in debug builds (the editor, `godot` runs and
	# debug exports); release exports never create it.
	if OS.is_debug_build() and config.overlay:
		var overlay: Node = load("res://scripts/dev_overlay.gd").new()
		add_child(overlay)
		overlay.watch(self)
	if config.bot:
		bot = ScriptedBot.new(config.bot_seed)
		_start()


func _start() -> void:
	var ts := Time.get_ticks_usec()
	var frame := Engine.get_process_frames()
	var fields := {"probe_start_us": ts}
	if not str(config.probe_match_id).is_empty():
		fields.match_id = str(config.probe_match_id)
	run_info = {
		"probe_enabled": false, "probe_amplitude_ppm": 0, "probe_start_us": ts,
		"probe_seed": "none", "server": config.server, "server_status": "not used",
	}
	if not ProbeHooks.available():
		push_warning("aim_range: Rearguard extension not loaded: no probe, no telemetry file, no server"
				+ " (build it with: cargo build -p rearguard-godot)")
	else:
		if not str(config.server).is_empty():
			_connect_server(fields, ts)
		if config.probe and probe == null:
			_start_local_probe(ts)
		if config.record:
			_open_recording()
	_write_environment()
	session.start(fields, ts, frame)


## Opens a server session. Its seed drives the probe; its ids go in the header. If the
## server cannot be reached, the run carries on offline (see README.md, "Live loop").
func _connect_server(fields: Dictionary, ts: int) -> void:
	var c: RefCounted = ClassDB.instantiate("RearguardClient")
	var name := "rearguard-aimrange/godot-%s" % Engine.get_version_info().string
	var err: int = c.connect_to(str(config.server), name)
	if err != OK:
		push_warning("aim_range: server %s unreachable (%s); running offline with a local seed,"
				% [config.server, error_string(err)] + " so no verdict will come")
		run_info.server_status = "unreachable"
		return
	client = c
	run_info.server_status = "connected"
	fields.match_id = str(c.match_id())
	fields.player_id = int(c.player_id())
	session.listeners.append(func(r: Dictionary) -> void: TelemetryOut.forward(c, r))
	if config.probe:
		var p: RefCounted = ClassDB.instantiate("RearguardProbe")
		if p.start_session_from_client(c, ts) == OK:
			probe = p
			ProbeHooks.install(session.aim, probe)
			run_info.probe_enabled = true
			run_info.probe_amplitude_ppm = int(p.amplitude_ppm())
			run_info.probe_seed = "server"
	print("aim_range: server session %d (match %s)" % [c.session_id(), c.match_id()])


## Starts the probe from a local seed: offline, or when the server was unreachable.
func _start_local_probe(ts: int) -> void:
	var p: RefCounted = ClassDB.instantiate("RearguardProbe")
	var err: int
	if not str(config.probe_study_key).is_empty():
		err = p.start_session_derived(ProjectSettings.globalize_path(config.probe_study_key),
				str(config.probe_match_id), 0, config.probe_amplitude_ppm, ts)
		run_info.probe_seed = "study key"
	elif str(config.probe_seed_file).is_empty():
		err = p.start_session_random(config.probe_amplitude_ppm, ts)
		run_info.probe_seed = "random"
	else:
		var path := ProjectSettings.globalize_path(config.probe_seed_file)
		DirAccess.make_dir_recursive_absolute(path.get_base_dir())
		err = p.start_session_from_file(path, true, config.probe_amplitude_ppm, ts)
		run_info.probe_seed = "file"
	if err != OK:
		push_error("aim_range: input probe not started: %s" % error_string(err))
		run_info.probe_seed = "none"
		return
	probe = p
	ProbeHooks.install(session.aim, probe)
	run_info.probe_enabled = true
	run_info.probe_amplitude_ppm = config.probe_amplitude_ppm


## Writes the session's telemetry (rearguard.telemetry v1) through the extension.
func _open_recording() -> void:
	DirAccess.make_dir_recursive_absolute(recording_path.get_base_dir())
	var rec: RefCounted = ClassDB.instantiate("RearguardRecorder")
	var path := recording_path
	telemetry_file = rec
	session.listeners.append(func(r: Dictionary) -> void: TelemetryOut.forward(rec, r, path))


## The runtime details the telemetry header does not carry, next to the recording as
## <recording>.env.json. Never contains the seed or where it is kept.
func _write_environment() -> void:
	if not config.record or not config.environment_file:
		return
	var env := {
		"source": "bot" if bot != null else "human",
		"bot_seed": config.bot_seed if bot != null else null,
		"godot_version": Engine.get_version_info().string,
		"os": OS.get_name(),
		"display_driver": DisplayServer.get_name(),
		"accumulated_input": Input.is_using_accumulated_input(),
		"delta_source": "InputEventMouseMotion.screen_relative",
		"clock": "Time.get_ticks_usec",
		"debug_build": OS.is_debug_build(),
	}
	env.merge(run_info)
	DirAccess.make_dir_recursive_absolute(recording_path.get_base_dir())
	var f := FileAccess.open(recording_path + ".env.json", FileAccess.WRITE)
	if f != null:
		f.store_string(JSON.stringify(env, "  ") + "\n")


func _active() -> bool:
	return session != null and session.started and not session.finished \
			and (bot != null or Input.mouse_mode == Input.MOUSE_MODE_CAPTURED)


func _input(event: InputEvent) -> void:
	if session == null:
		return
	var from_bot := event.device == BOT_DEVICE_ID
	if from_bot != (bot != null):
		return
	if event is InputEventMouseMotion:
		# Uncaptured motion is the desktop cursor (accelerated), not raw input.
		if not _active():
			return
		_count_event()
		var mm := event as InputEventMouseMotion
		session.handle_motion(mm.screen_relative, Time.get_ticks_usec(), Engine.get_process_frames())
	elif event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if bot == null and Input.mouse_mode != Input.MOUSE_MODE_CAPTURED:
			# The click that captures the mouse is not a shot.
			if event.pressed and not session.finished:
				Input.mouse_mode = Input.MOUSE_MODE_CAPTURED
				_start()
			return
		session.handle_trigger(event.pressed, Time.get_ticks_usec(), Engine.get_process_frames())
	elif event.is_action_pressed("ui_cancel") and bot == null:
		_release_mouse()


func _notification(what: int) -> void:
	if what == NOTIFICATION_APPLICATION_FOCUS_OUT and bot == null:
		_release_mouse()
	elif what == NOTIFICATION_WM_CLOSE_REQUEST or what == NOTIFICATION_EXIT_TREE:
		_close_recording()


func _close_recording() -> void:
	if session == null or not session.started:
		return
	# Writes an incomplete end record (and closes the file) if the run was cut short.
	session.abort(Time.get_ticks_usec(), Engine.get_process_frames())
	_release_extension()


## Drops every extension object: the uplink's worker stops, the probe's seed is wiped.
func _release_extension() -> void:
	if client != null:
		client.close()
		client = null
	if probe != null:
		probe.stop()
		probe = null
	telemetry_file = null


func _release_mouse() -> void:
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	if session != null:
		session.handle_trigger(false, Time.get_ticks_usec(), Engine.get_process_frames())


func _physics_process(_delta: float) -> void:
	if not _active():
		return
	if bot != null:
		var p := bot.plan(session)
		for counts: Vector2 in p.moves:
			var mm := InputEventMouseMotion.new()
			mm.device = BOT_DEVICE_ID
			mm.relative = counts
			mm.screen_relative = counts
			Input.parse_input_event(mm)
		if p.trigger != session.trigger_down():
			var mb := InputEventMouseButton.new()
			mb.device = BOT_DEVICE_ID
			mb.button_index = MOUSE_BUTTON_LEFT
			mb.pressed = p.trigger
			Input.parse_input_event(mb)
	session.step_tick(Time.get_ticks_usec(), Engine.get_process_frames())
	if telemetry_file != null and session.tick % Engine.physics_ticks_per_second == 0:
		telemetry_file.flush()
	if session.finished and _waiting_since_ms < 0 and verdict_line.is_empty():
		_finish()


func _finish() -> void:
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	print("aim_range: %s seed %d finished, %d/%d hits, recording %s" % [
		config.scenario, config.seed, session.hits, session.shots,
		recording_path if telemetry_file != null else "(none)"])
	if client != null:
		# Everything recorded so far is queued; end the server session and wait (up to
		# verdict_timeout_s) for its verdict before completing.
		client.finish()
		_waiting_since_ms = Time.get_ticks_msec()
	else:
		_complete()


func _complete() -> void:
	_waiting_since_ms = -1
	if client != null:
		verdict_line = "aim_range: server session %d: %s" % [client.session_id(), client.status()]
		# Only a final verdict counts here; the overlay's live ones (status "open") do not.
		var v: Dictionary = client.verdict() if client.has_method("verdict") else {}
		if not v.is_empty() and v.status != "open":
			verdict_line += ", verdict score %.2f, flagged %s, %d records, %d windows" % [
				v.score, v.flagged, v.records, v.windows]
		elif str(client.status()) != "finished":
			verdict_line += ", no final verdict"
	else:
		verdict_line = "aim_range: no server session (%s)" % run_info.get("server_status", "not used")
	print(verdict_line)
	_write_frame_stats()
	_release_extension()
	run_finished.emit(recording_path)
	if config.quit:
		get_tree().quit(0)


## Per-frame wall time over the active run (--frame-stats), for measuring the cost of
## the probe, the recorder and the uplink.
func _write_frame_stats() -> void:
	if str(config.frame_stats).is_empty() or _frame_times.is_empty():
		return
	var sorted := _frame_times.duplicate()
	sorted.sort()
	var pick := func(q: float) -> int: return sorted[mini(int(q * (sorted.size() - 1) + 0.5), sorted.size() - 1)]
	var total := 0
	for t in sorted:
		total += t
	var stats := {
		"frames": sorted.size(), "p50_us": pick.call(0.5), "p99_us": pick.call(0.99),
		"mean_us": float(total) / sorted.size(), "max_us": sorted[-1],
		"probe": run_info.get("probe_enabled", false), "record": config.record,
		"server": run_info.get("server_status", "not used"), "scenario": config.scenario,
	}
	if client != null and client.has_method("stats"):
		var u: Dictionary = client.stats()
		stats.bytes_sent = u.bytes_sent
		stats.records_queued = u.records_queued
		stats.chunks_sent = u.chunks_sent
	var path := ProjectSettings.globalize_path(config.frame_stats)
	DirAccess.make_dir_recursive_absolute(path.get_base_dir())
	var f := FileAccess.open(path, FileAccess.WRITE)
	if f != null:
		f.store_string(JSON.stringify(stats) + "\n")
	print("aim_range: frame time p50 %d us, p99 %d us over %d frames" % [stats.p50_us, stats.p99_us, stats.frames])


func _count_event() -> void:
	_motion_events += 1
	var frame := Engine.get_process_frames()
	if frame != _frame_seen:
		_frame_seen = frame
		_events_this_frame = 0
	_events_this_frame += 1
	_max_events_per_frame = maxi(_max_events_per_frame, _events_this_frame)


func _process(_delta: float) -> void:
	if session == null:
		return
	var now := Time.get_ticks_usec()
	if _active():
		if _last_frame_us > 0:
			_frame_times.append(now - _last_frame_us)
		_last_frame_us = now
	if _waiting_since_ms >= 0 and (client == null or client.is_done()
			or Time.get_ticks_msec() - _waiting_since_ms > int(config.verdict_timeout_s * 1000.0)):
		_complete()
	var view := session.aim.view()
	_camera.rotation = Vector3(deg_to_rad(view[1]), deg_to_rad(view[0]), 0.0)
	var t := session.current_target()
	var dir := RangeSession.direction(t)
	_target.position = _camera.position \
			+ Vector3(dir[0], dir[1], dir[2]) * Scenario.TARGET_DISTANCE_M
	if not str(config.hud_title).is_empty():
		# Study HUD: nothing that could hint at the condition.
		_hud.text = "%s\n%.0f s left  hits %d / %d\n%s" % [
			config.hud_title, maxf(config.duration - session.time_s(), 0.0), session.hits, session.shots,
			_status_line() if not session.finished else ""]
		return
	_hud.text = "%s  seed %d  %.1f / %.0f s  hits %d / %d\n%s  motion events %d  max per frame %d\n%s" % [
		config.scenario, config.seed, session.time_s(), config.duration, session.hits, session.shots,
		DisplayServer.get_name(), _motion_events, _max_events_per_frame, _status_line()]


func _status_line() -> String:
	if session.finished:
		return "Finished. %s" % verdict_line if not verdict_line.is_empty() else "Finished; waiting for the verdict"
	if bot != null:
		return "Scripted bot"
	if Input.mouse_mode != Input.MOUSE_MODE_CAPTURED:
		return "Click to start (Esc releases the mouse and pauses)"
	return ""


func _build_world() -> void:
	var env := WorldEnvironment.new()
	env.environment = Environment.new()
	env.environment.background_mode = Environment.BG_COLOR
	env.environment.background_color = Color(0.55, 0.62, 0.7)
	env.environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	env.environment.ambient_light_color = Color(0.6, 0.6, 0.6)
	add_child(env)

	var sun := DirectionalLight3D.new()
	sun.rotation_degrees = Vector3(-50.0, 30.0, 0.0)
	add_child(sun)

	var floor_mesh := MeshInstance3D.new()
	var plane := PlaneMesh.new()
	plane.size = Vector2(80.0, 80.0)
	floor_mesh.mesh = plane
	var floor_mat := StandardMaterial3D.new()
	floor_mat.albedo_color = Color(0.35, 0.37, 0.33)
	floor_mesh.material_override = floor_mat
	add_child(floor_mesh)

	_camera = Camera3D.new()
	_camera.position = Vector3(0.0, EYE_HEIGHT_M, 0.0)
	_camera.fov = 90.0
	add_child(_camera)
	_camera.make_current()

	_target = MeshInstance3D.new()
	var sphere := SphereMesh.new()
	sphere.radius = Scenario.TARGET_RADIUS_M
	sphere.height = Scenario.TARGET_RADIUS_M * 2.0
	_target.mesh = sphere
	var target_mat := StandardMaterial3D.new()
	target_mat.albedo_color = Color(0.9, 0.15, 0.1)
	target_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_target.material_override = target_mat
	add_child(_target)

	var ui := CanvasLayer.new()
	add_child(ui)
	_hud = Label.new()
	_hud.position = Vector2(12.0, 8.0)
	ui.add_child(_hud)
	var crosshair := Label.new()
	crosshair.text = "+"
	crosshair.set_anchors_and_offsets_preset(Control.PRESET_CENTER)
	crosshair.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	crosshair.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	ui.add_child(crosshair)
