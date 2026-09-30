## Aim range scene: builds the world, captures raw mouse input, drives a
## RangeSession and draws it. The input path is documented in demo/README.md.
##
## Command-line options come after `--`, for example:
##   godot --path demo -- --scenario spray --seed 7
##   godot --headless --path demo --fixed-fps 120 -- --bot --scenario flick --seed 7 --duration 10
extends Node3D

const RangeSession := preload("res://scripts/range_session.gd")
const Scenario := preload("res://scripts/scenario.gd")
const Recorder := preload("res://scripts/recorder.gd")
const AimModel := preload("res://scripts/aim_model.gd")
const ScriptedBot := preload("res://scripts/scripted_bot.gd")
const ProbeHooks := preload("res://scripts/probe_hooks.gd")

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

var _camera: Camera3D
var _target: MeshInstance3D
var _hud: Label
var _frame_seen := -1
var _events_this_frame := 0
var _max_events_per_frame := 0
var _motion_events := 0


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
		"telemetry": "",
	}


## Parses `--scenario NAME --seed N --duration S --sens DEG --bot --bot-seed N
## --out PATH --quit --no-probe --probe-seed-file PATH --probe-amplitude-ppm N
## --telemetry PATH`. Unknown options are reported and ignored.
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
			"--telemetry": c.telemetry = value; i += 1
			"--no-probe": c.probe = false
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
	session = RangeSession.new(scenario, Engine.physics_ticks_per_second, Recorder.new(recording_path))
	session.aim.deg_per_count = config.sens
	_build_world()
	if config.bot:
		bot = ScriptedBot.new(config.bot_seed)
		_start()


func _start() -> void:
	var ts := Time.get_ticks_usec()
	var frame := Engine.get_process_frames()
	var environment := {
		"source": "bot" if bot != null else "human",
		"bot_seed": config.bot_seed if bot != null else null,
		"godot_version": Engine.get_version_info().string,
		"os": OS.get_name(),
		"display_driver": DisplayServer.get_name(),
		"accumulated_input": Input.is_using_accumulated_input(),
		"delta_source": "InputEventMouseMotion.screen_relative",
		"clock": "Time.get_ticks_usec",
	}
	environment.merge(_start_probe(ts))
	_open_telemetry(ts, frame)
	session.start(environment, ts, frame)


## Starts the input probe (Rust extension) with probe tick 0 at `ts`, and installs it
## on the two aim hooks. Returns the header fields describing it; the seed itself and
## where it is kept are never recorded.
func _start_probe(ts: int) -> Dictionary:
	var info := {"probe_enabled": false, "probe_amplitude_ppm": 0, "probe_start_us": ts, "probe_seed": "none"}
	if not config.probe:
		return info
	if not ProbeHooks.available():
		push_warning("aim_range: Rearguard extension not loaded, running without the input probe"
				+ " (build it with: cargo build -p rearguard-godot)")
		return info
	probe = ClassDB.instantiate("RearguardProbe")
	var err: int
	if str(config.probe_seed_file).is_empty():
		err = probe.start_session_random(config.probe_amplitude_ppm, ts)
		info.probe_seed = "random"
	else:
		var path := ProjectSettings.globalize_path(config.probe_seed_file)
		DirAccess.make_dir_recursive_absolute(path.get_base_dir())
		err = probe.start_session_from_file(path, true, config.probe_amplitude_ppm, ts)
		info.probe_seed = "file"
	if err != OK:
		push_error("aim_range: input probe not started: %s" % error_string(err))
		probe = null
		info.probe_seed = "none"
		return info
	ProbeHooks.install(session.aim, probe)
	info.probe_enabled = true
	info.probe_amplitude_ppm = config.probe_amplitude_ppm
	return info


## Opens the rearguard.telemetry output (Rust extension), if one was asked for.
func _open_telemetry(ts: int, frame: int) -> void:
	if str(config.telemetry).is_empty():
		return
	if not ProbeHooks.available():
		push_warning("aim_range: Rearguard extension not loaded, no telemetry written")
		return
	var path := ProjectSettings.globalize_path(config.telemetry)
	DirAccess.make_dir_recursive_absolute(path.get_base_dir())
	var recorder: RefCounted = ClassDB.instantiate("RearguardRecorder")
	var err: int = recorder.open(path, {
		"ts_us": ts, "frame": frame, "tick": 0,
		"match_id": "aimrange-local", "player_id": 0, "source": "client",
		"probe_start_us": ts,
		"deg_per_count": float(config.sens),
		"physics_hz": Engine.physics_ticks_per_second,
		"scenario": config.scenario,
		"scenario_seed": config.seed,
		"duration_s": float(config.duration),
		"recoil_pattern": Scenario.RECOIL_PATTERN_ID,
		"target_distance_m": Scenario.TARGET_DISTANCE_M,
		"target_radius_m": Scenario.TARGET_RADIUS_M,
	})
	if err != OK:
		push_error("aim_range: telemetry not written: %s" % error_string(err))
		return
	session.telemetry = recorder


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
	if session == null or session.recorder == null:
		return
	session.abort(Time.get_ticks_usec(), Engine.get_process_frames())
	session.recorder.close()


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
	if session.finished:
		_finish()


func _finish() -> void:
	_close_recording()
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	print("aim_range: %s seed %d finished, %d/%d hits, recording %s" % [
		config.scenario, config.seed, session.hits, session.shots, recording_path])
	run_finished.emit(recording_path)
	if config.quit:
		get_tree().quit(0)


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
	var view := session.aim.view()
	_camera.rotation = Vector3(deg_to_rad(view[1]), deg_to_rad(view[0]), 0.0)
	var t := session.current_target()
	var dir := RangeSession.direction(t)
	_target.position = _camera.position \
			+ Vector3(dir[0], dir[1], dir[2]) * Scenario.TARGET_DISTANCE_M
	_hud.text = "%s  seed %d  %.1f / %.0f s  hits %d / %d\n%s  motion events %d  max per frame %d\n%s" % [
		config.scenario, config.seed, session.time_s(), config.duration, session.hits, session.shots,
		DisplayServer.get_name(), _motion_events, _max_events_per_frame, _status_line()]


func _status_line() -> String:
	if session.finished:
		return "Finished. Recording: %s" % recording_path
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
