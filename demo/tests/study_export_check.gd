# SPDX-License-Identifier: MIT OR Apache-2.0

## Checks on a study export zip (scripts/study.gd), shared by the repository tests
## (scripts/study_test.gd) and the packaged-build check (tools/check_study_export.gd),
## so an exported build passes exactly the same no-personal-data checks as task 1.9.
##
## Every function returns failure messages; an empty list means the check passed.
extends RefCounted

const Recorder := preload("res://scripts/recorder.gd")

## The only keys the export manifest may contain, per level.
const MANIFEST_KEYS := ["format", "version", "study_id", "protocol_version", "protocol",
		"consent_version", "release", "automated", "participant_id", "randomisation_seed", "plan", "input",
		"sessions", "trials"]
const INPUT_KEYS := ["godot_version", "os_name", "display_driver", "accumulated_input",
		"delta_source", "debug_build"]
const SESSION_KEYS := ["label", "block", "condition", "amplitude_ppm", "scenario", "scenario_seed",
		"duration_s", "file", "complete", "probe_applied", "shots", "hits", "trial", "interval"]
const TRIAL_KEYS := ["index", "amplitude_ppm", "drifted", "order", "answer", "correct", "response_ms"]
## Telemetry header fields the export may carry (rearguard.telemetry v1).
const HEADER_KEYS := ["type", "ts_us", "frame", "tick", "format", "version", "match_id", "player_id",
		"source", "probe_start_us", "deg_per_count", "physics_hz", "scenario", "scenario_seed",
		"duration_s", "recoil_pattern", "target_distance_m", "target_radius_m"]


## Reads every entry of the export zip into {name: bytes}; {} if it cannot be opened.
static func unzip(path: String) -> Dictionary:
	var zip := ZIPReader.new()
	if zip.open(path) != OK:
		return {}
	var out := {}
	for f in zip.get_files():
		if not f.ends_with("/"): # Directory entries, if the reader lists any.
			out[f] = zip.read_file(f)
	zip.close()
	return out


## The telemetry records of one exported session file.
static func records(files: Dictionary, name: String) -> Array[Dictionary]:
	var bytes: PackedByteArray = files.get(name, PackedByteArray())
	return Recorder.parse_lines(bytes.get_string_from_utf8().split("\n"))


## The export holds no personal data: only allowlisted keys, and none of this machine's
## identifying strings, today's date or the study key (`key_hex`) anywhere in it.
## `extra` adds more forbidden strings, {what: value}.
static func personal_data_failures(files: Dictionary, key_hex: String, extra: Dictionary = {}) -> PackedStringArray:
	var failures := PackedStringArray()
	if not files.has("manifest.json"):
		failures.append("export has no manifest.json")
		return failures
	var manifest: Variant = JSON.parse_string((files["manifest.json"] as PackedByteArray).get_string_from_utf8())
	if not manifest is Dictionary:
		failures.append("manifest.json is not a JSON object")
		return failures
	_only_keys(failures, "manifest", manifest, MANIFEST_KEYS)
	_only_keys(failures, "input", manifest.get("input", {}), INPUT_KEYS)
	for s: Dictionary in manifest.get("sessions", []):
		_only_keys(failures, "session", s, SESSION_KEYS)
		var header := records(files, s.get("file", ""))
		if header.is_empty():
			failures.append("session %s has no telemetry" % s.get("label", "?"))
		else:
			_only_keys(failures, "telemetry header", header[0], HEADER_KEYS)
	for t: Dictionary in manifest.get("trials", []):
		_only_keys(failures, "trial", t, TRIAL_KEYS)

	var forbidden := {"study key": key_hex.strip_edges()}
	for v in ["USER", "USERNAME", "LOGNAME", "HOSTNAME", "COMPUTERNAME", "HOME", "USERPROFILE"]:
		var value := OS.get_environment(v)
		if value.length() >= 3:
			forbidden[v] = value
	forbidden["user data dir"] = OS.get_user_data_dir()
	forbidden["executable dir"] = OS.get_executable_path().get_base_dir()
	forbidden["project dir"] = ProjectSettings.globalize_path("res://").trim_suffix("/")
	forbidden["unique id"] = OS.get_unique_id()
	forbidden["model"] = OS.get_model_name()
	var today := Time.get_date_dict_from_system()
	forbidden["date"] = "%04d-%02d-%02d" % [today.year, today.month, today.day]
	forbidden["date (compact)"] = "%04d%02d%02d" % [today.year, today.month, today.day]
	forbidden.merge(extra)
	for what in forbidden:
		var value: String = forbidden[what]
		if value.length() < 3 or value == "GenericDevice":
			continue
		for name in files:
			if (files[name] as PackedByteArray).get_string_from_utf8().contains(value):
				failures.append("%s must not contain the %s" % [name, what])
	# Entry names are stored uncompressed in the zip; they are only fixed names and labels.
	for name: String in files:
		if not (name == "manifest.json" or name.begins_with("sessions/")):
			failures.append("unexpected entry %s" % name)
	return failures


## Replays one session's view angles with its probe re-derived from the study key file,
## as the analysis does. Returns {worst: largest yaw mismatch, drift: |drifted - nominal|
## yaw at the end, error: the derivation's Error}.
static func replay(session_records: Array[Dictionary], key_path: String, amplitude_ppm: int) -> Dictionary:
	var header := session_records[0]
	var probe: RefCounted = ClassDB.instantiate("RearguardProbe")
	var err: int = probe.start_session_derived(key_path, header.match_id, 0, amplitude_ppm,
			int(header.probe_start_us))
	var dpc := float(header.deg_per_count)
	var yaw := 0.0
	var nominal := 0.0
	var worst := 0.0
	for r in session_records:
		if r.type == "move":
			var d := -float(r.dx) * dpc
			yaw += d * probe.sensitivity_multiplier(int(r.ts_us))
			nominal += d
		elif r.type == "recoil":
			yaw += float(r.kick_yaw) * probe.recoil_scale(int(r.ts_us))
			nominal += float(r.kick_yaw)
		else:
			continue
		worst = maxf(worst, absf(yaw - float(r.yaw)))
	probe.stop()
	return {"worst": worst, "drift": absf(yaw - nominal), "error": err}


static func _only_keys(failures: PackedStringArray, what: String, d: Variant, allowed: Array) -> void:
	if not d is Dictionary:
		failures.append("%s is not a JSON object" % what)
		return
	for k in (d as Dictionary).keys():
		if not k in allowed:
			failures.append("%s key %s is not allowlisted" % [what, k])
