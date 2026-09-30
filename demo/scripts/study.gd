## Human-study build of the aim range (task 1.9). The scene is scenes/study.tscn.
##
## Flow:
## 1. **Consent.** Nothing is recorded before the participant agrees; declining quits.
## 2. **Participant ID and plan.** A random pseudonymous ID is drawn, along with a random
##    randomisation seed. The plan (rearguard_core::study, via the extension) orders
##    the baseline sessions and the blind trials.
## 3. **Baseline sessions:** plain play, drift on or off. The participant is not told
##    which.
## 4. **Blind trials:** two intervals, A then B, one of them drifted (or neither, for a
##    catch trial), then a forced choice between A and B.
## 5. **Export:** one zip file with a manifest and every session's telemetry
##    (rearguard.telemetry v1). The temporary session files are then deleted.
##
## Each session's drift seed is derived from the facilitator's study key file inside the
## extension. The export carries only labels, never seed material. Screens never show a
## session's condition or amplitude, and the developer overlay is off.
##
## Options after `--`:
##   --study-key PATH (default user://study/study-key.hex)
##   --protocol PATH (default res://study/protocol.json)
##   --export-dir DIR (default user://exports)
extends Node

const AimRange := preload("res://scripts/aim_range.gd")
const SCENE := preload("res://scenes/aim_range.tscn")
const CONSENT_PATH := "res://study/consent_v1.txt"
const CONSENT_VERSION := "v1"
const EXPORT_FORMAT := "rearguard.study.export"
const EXPORT_VERSION := 1
## 32 symbols without 0/O and 1/I, easy to read out; 32 divides 256, so each byte
## maps to a symbol without bias.
const ID_ALPHABET := "23456789ABCDEFGHJKLMNPQRSTUVWXYZ"

## Emitted when the study ends: the export's path, or "" if the participant declined.
signal finished(export_path: String)
signal _chosen(value: String)

## Run options. Set before the node enters the tree to skip command-line parsing.
var config: Dictionary = {}
var participant_id := ""
var seed_text := ""
var plan: Dictionary = {}
var export_path := ""

var _protocol_text := ""
## The plan exactly as the extension produced it (GDScript's JSON parser turns integers
## into floats, so the export embeds this text, not a re-serialised copy).
var _plan_text := ""
var _work_dir := ""
var _sessions: Array[Dictionary] = []
var _trials: Array[Dictionary] = []
var _panel: PanelContainer
var _text: Label
var _buttons: HBoxContainer
var _answer_rng := RandomNumberGenerator.new()


static func default_config() -> Dictionary:
	return {
		"study_key": "user://study/study-key.hex",
		"protocol": "res://study/protocol.json",
		"export_dir": "user://exports",
		# Tests only: accept consent, let the scripted bot play, answer from a seeded RNG,
		# and fix the participant ID and randomisation seed.
		"auto": false,
		"auto_answer_seed": 1,
		"participant_id": "",
		"seed": "",
	}


static func parse_args(args: PackedStringArray) -> Dictionary:
	var c := default_config()
	var i := 0
	while i < args.size():
		var value := args[i + 1] if i + 1 < args.size() else ""
		match args[i]:
			"--study-key": c.study_key = value; i += 1
			"--protocol": c.protocol = value; i += 1
			"--export-dir": c.export_dir = value; i += 1
			_: push_warning("study: unknown option %s" % args[i])
		i += 1
	return c


func _ready() -> void:
	if config.is_empty():
		config = parse_args(OS.get_cmdline_user_args())
	_build_ui()
	if not ClassDB.class_exists("RearguardStudy"):
		_show("The study cannot start: the Rearguard extension is not built.\n\nFacilitator: run `cargo build -p rearguard-godot` from the repository root.", [["Quit", _quit]])
		return
	if not FileAccess.file_exists(ProjectSettings.globalize_path(config.study_key)):
		_show("The study cannot start: the study key file is missing.\n\nFacilitator: place it at\n%s\n(see the facilitator instructions)." % ProjectSettings.globalize_path(config.study_key), [["Quit", _quit]])
		return
	_protocol_text = FileAccess.get_file_as_string(config.protocol)
	if _protocol_text.is_empty():
		_show("The study cannot start: the protocol file %s is missing." % config.protocol, [["Quit", _quit]])
		return
	if config.auto:
		accept_consent.call_deferred()
	else:
		_show(FileAccess.get_file_as_string(CONSENT_PATH), [["I agree", accept_consent], ["I do not agree", decline_consent]])


## The participant declined: nothing has been recorded, nothing is kept.
func decline_consent() -> void:
	_show("Thank you. Nothing was recorded. You can close this window.", [["Quit", _quit]])
	finished.emit("")


## The participant agreed: draw the ID and the plan, then start.
func accept_consent() -> void:
	participant_id = config.participant_id if not str(config.participant_id).is_empty() else _new_participant_id()
	seed_text = config.seed if not str(config.seed).is_empty() else _new_seed()
	_plan_text = ClassDB.class_call_static("RearguardStudy", "plan", _protocol_text, seed_text)
	var planned: Variant = JSON.parse_string(_plan_text)
	if not planned is Dictionary or (planned as Dictionary).has("error"):
		_show("The study cannot start: %s" % (planned.get("error", "invalid plan") if planned is Dictionary else "invalid plan"), [["Quit", _quit]])
		return
	plan = planned
	_answer_rng.seed = int(config.auto_answer_seed)
	_work_dir = ProjectSettings.globalize_path("user://study-work").path_join(participant_id)
	DirAccess.make_dir_recursive_absolute(_work_dir)
	_run.call_deferred()


func _run() -> void:
	var baseline: Array = plan.baseline
	var trials: Array = plan.trials
	await _pause("Your participant ID is %s. Please note it down: it is the only way to find your data later.\n\nFirst you will play %d rounds of about a minute each. Click Start, then click in the game to begin each round. Press Esc to pause." % [participant_id, baseline.size()], "Start")
	for i in baseline.size():
		var s: Dictionary = baseline[i]
		if i > 0:
			await _pause("Round %d of %d." % [i + 1, baseline.size()], "Start")
		var result := await _play(s, "Round %d of %d" % [i + 1, baseline.size()])
		_sessions.append(_session_entry(s, "baseline", result, null, ""))
	await _pause("Now %d comparisons. Each is two short rounds, A then B. In one of them (or sometimes neither) the aim may behave slightly differently. After both, choose the round that felt different. If you cannot tell, make your best guess." % trials.size(), "Start")
	for t: Dictionary in trials:
		var number := int(t.index) + 1
		var intervals: Array = t.intervals
		await _pause("Comparison %d of %d: round A." % [number, trials.size()], "Start A")
		var a := await _play(intervals[0], "Comparison %d of %d: A" % [number, trials.size()])
		await _pause("Comparison %d of %d: round B." % [number, trials.size()], "Start B")
		var b := await _play(intervals[1], "Comparison %d of %d: B" % [number, trials.size()])
		var shown := Time.get_ticks_msec()
		var answer := await _choose("Which round felt different?", ["A", "B"])
		var response_ms := Time.get_ticks_msec() - shown
		_sessions.append(_session_entry(intervals[0], "blind", a, int(t.index), "A"))
		_sessions.append(_session_entry(intervals[1], "blind", b, int(t.index), "B"))
		_trials.append({
			"index": int(t.index),
			"amplitude_ppm": int(t.amplitude_ppm),
			"drifted": t.drifted,
			"order": [intervals[0].label, intervals[1].label],
			"answer": answer,
			"correct": answer == t.drifted,
			"response_ms": response_ms,
		})
	export_path = _export()
	_show("Thank you, you are done.\n\nYour participant ID: %s\n\nYour data is in one file:\n%s\n\nPlease send this file to the study facilitator. It contains only the data described at the start." % [
			participant_id, export_path], [["Show the file", func() -> void: OS.shell_open(export_path.get_base_dir())], ["Quit", _quit]])
	finished.emit(export_path)


## Plays one session and returns what the manifest needs.
func _play(s: Dictionary, title: String) -> Dictionary:
	_panel.visible = false
	var node: AimRange = SCENE.instantiate()
	var c := AimRange.default_config()
	var label: String = s.label
	c.merge({
		"scenario": s.scenario,
		"seed": int(s.scenario_seed),
		"duration": float(s.duration_s),
		"out": _work_dir.path_join(label + ".jsonl"),
		"probe": true,
		"probe_study_key": config.study_key,
		"probe_match_id": ClassDB.class_call_static("RearguardStudy", "match_id", plan.study_id, participant_id, label),
		"probe_amplitude_ppm": int(s.amplitude_ppm),
		"overlay": false,
		"environment_file": false,
		"hud_title": title,
		"quit": false,
		"bot": config.auto,
		"bot_seed": int(s.scenario_seed) % 1000,
	}, true)
	node.config = c
	add_child(node)
	await node.run_finished
	var result := {
		"complete": node.session.finished,
		"shots": node.session.shots,
		"hits": node.session.hits,
		"probe": node.run_info.get("probe_seed", "none") == "study key",
	}
	node.queue_free()
	_panel.visible = true
	return result


func _session_entry(s: Dictionary, block: String, result: Dictionary, trial: Variant, interval: String) -> Dictionary:
	return {
		"label": s.label,
		"block": block,
		"condition": s.condition,
		"amplitude_ppm": int(s.amplitude_ppm),
		"scenario": s.scenario,
		"scenario_seed": int(s.scenario_seed),
		"duration_s": s.duration_s,
		"file": "sessions/%s.jsonl" % s.label,
		"complete": result.complete,
		"probe_applied": result.probe,
		"shots": result.shots,
		"hits": result.hits,
		"trial": trial,
		"interval": interval,
	}


## The export manifest as JSON text. Contains only what the consent text lists.
func manifest_text() -> String:
	var m := {
		"format": EXPORT_FORMAT,
		"version": EXPORT_VERSION,
		"study_id": plan.study_id,
		"protocol_version": int(plan.protocol_version),
		"protocol": "@protocol@",
		"consent_version": CONSENT_VERSION,
		"participant_id": participant_id,
		"randomisation_seed": seed_text,
		"plan": "@plan@",
		"input": {
			"godot_version": Engine.get_version_info().string,
			"os_name": OS.get_name(),
			"display_driver": DisplayServer.get_name(),
			"accumulated_input": Input.is_using_accumulated_input(),
			"delta_source": "InputEventMouseMotion.screen_relative",
			"debug_build": OS.is_debug_build(),
		},
		"sessions": _sessions,
		"trials": _trials,
	}
	# The protocol was validated by the plan (unknown fields are refused), so its text is
	# embedded as the facilitator wrote it.
	return JSON.stringify(m, "  ").replace('"@protocol@"', _protocol_text.strip_edges()) \
			.replace('"@plan@"', _plan_text) + "\n"


## Writes the single export file and deletes the temporary session files.
func _export() -> String:
	var dir := ProjectSettings.globalize_path(config.export_dir)
	DirAccess.make_dir_recursive_absolute(dir)
	var path := dir.path_join("rearguard-study-%s-%s.zip" % [plan.study_id, participant_id])
	var zip := ZIPPacker.new()
	if zip.open(path) != OK:
		push_error("study: cannot write %s" % path)
		return ""
	zip.start_file("manifest.json")
	zip.write_file(manifest_text().to_utf8_buffer())
	zip.close_file()
	for s in _sessions:
		var source := _work_dir.path_join(str(s.label) + ".jsonl")
		zip.start_file(s.file)
		zip.write_file(FileAccess.get_file_as_bytes(source))
		zip.close_file()
	zip.close()
	for s in _sessions:
		DirAccess.remove_absolute(_work_dir.path_join(str(s.label) + ".jsonl"))
	DirAccess.remove_absolute(_work_dir)
	return path


func _new_participant_id() -> String:
	var bytes := Crypto.new().generate_random_bytes(10)
	var id := "P-"
	for b in bytes:
		id += ID_ALPHABET[b % ID_ALPHABET.length()]
	return id


## A random 62-bit seed as a decimal string (it stays within GDScript's signed int).
func _new_seed() -> String:
	var bytes := Crypto.new().generate_random_bytes(8)
	return str(bytes.decode_u64(0) & 0x3FFF_FFFF_FFFF_FFFF)


func _build_ui() -> void:
	var layer := CanvasLayer.new()
	layer.layer = 5
	add_child(layer)
	_panel = PanelContainer.new()
	_panel.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	layer.add_child(_panel)
	var margin := MarginContainer.new()
	for side in ["left", "right", "top", "bottom"]:
		margin.add_theme_constant_override("margin_" + side, 48)
	_panel.add_child(margin)
	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 24)
	margin.add_child(column)
	_text = Label.new()
	_text.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_text.size_flags_vertical = Control.SIZE_EXPAND_FILL
	column.add_child(_text)
	_buttons = HBoxContainer.new()
	_buttons.add_theme_constant_override("separation", 24)
	column.add_child(_buttons)


func _show(text: String, buttons: Array) -> void:
	_panel.visible = true
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	_text.text = text
	for child in _buttons.get_children():
		child.queue_free()
	for spec in buttons:
		var b := Button.new()
		b.text = spec[0]
		b.custom_minimum_size = Vector2(160, 48)
		b.pressed.connect(spec[1])
		_buttons.add_child(b)


## Shows a message with one button and waits for it.
func _pause(text: String, button: String) -> void:
	if config.auto:
		return
	_show(text, [[button, func() -> void: _chosen.emit(button)]])
	await _chosen


## A forced choice: only the given options, no way to skip.
func _choose(question: String, options: Array) -> String:
	if config.auto:
		await get_tree().process_frame
		return options[_answer_rng.randi_range(0, options.size() - 1)]
	var buttons := []
	for o: String in options:
		buttons.append([o, func() -> void: _chosen.emit(o)])
	_show(question, buttons)
	return await _chosen


func _quit() -> void:
	get_tree().quit(0)
