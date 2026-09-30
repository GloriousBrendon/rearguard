# SPDX-License-Identifier: MIT OR Apache-2.0

## Checks an export written by an exported study build (task 1.9a), with the same
## no-personal-data checks as the repository tests (tests/study_export_check.gd).
## scripts/smoke-study-build.sh runs it with the Godot editor binary, on the machine that
## ran the build, after `RearguardStudy --self-test`:
##
##   godot --headless --path demo --script res://tools/check_study_export.gd -- \
##       --zip EXPORT.zip --key KEY.hex --release LABEL [--forbid TEXT]... [--human]
##
## Besides the personal-data checks, it checks the manifest (release label, a release
## build, marked automated) and replays every session with its probe re-derived from
## KEY.hex. That proves the drift really ran, and that KEY.hex (the facilitator's copy) is
## the key packed into the build. --forbid adds strings that must not appear anywhere,
## for example the directory the build ran from.
##
## With --human, it checks an export a participant sent back instead (see
## docs/study/facilitator-instructions.md): the export must not be marked automated. The
## machine-specific strings are then the facilitator's own, so only the allowlists, the
## key and the replays matter.
##
## Exit code 0 when every check passes, 1 otherwise.
extends SceneTree

const Check := preload("res://tests/study_export_check.gd")

var failures := PackedStringArray()


func _initialize() -> void:
	var zip := ""
	var key := ""
	var release := ""
	var extra := {}
	var human := false
	var a := OS.get_cmdline_user_args()
	var i := 0
	while i < a.size():
		if a[i] == "--human":
			human = true
			i += 1
			continue
		if i + 1 >= a.size():
			break
		match a[i]:
			"--zip": zip = a[i + 1]
			"--key": key = a[i + 1]
			"--release": release = a[i + 1]
			"--forbid": extra["--forbid %d" % extra.size()] = a[i + 1]
		i += 2
	if zip.is_empty() or key.is_empty() or release.is_empty():
		printerr("check_study_export: needs --zip, --key and --release")
		quit(2)
		return
	if not ClassDB.class_exists("RearguardProbe"):
		printerr("check_study_export: the Rust extension is not loaded")
		quit(2)
		return

	var files := Check.unzip(zip)
	_check(not files.is_empty(), "export opens: %s" % zip.get_file())
	if files.is_empty():
		_finish()
		return
	var key_hex := FileAccess.get_file_as_string(key).strip_edges()
	var personal := Check.personal_data_failures(files, key_hex, extra)
	for p in personal:
		_check(false, "no personal data: %s" % p)
	_check(personal.is_empty(), "the same no-personal-data checks as task 1.9")

	var manifest: Dictionary = JSON.parse_string((files["manifest.json"] as PackedByteArray).get_string_from_utf8())
	_check(manifest.release == release, "release label %s" % manifest.release)
	if human:
		_check(manifest.automated == false, "a participant's run (not automated)")
	else:
		_check(manifest.automated == true, "marked as an automated run")
	_check(manifest.input.debug_build == false, "written by a release build")
	_check(manifest.consent_version == "v1", "consent version")
	var sessions: Array = manifest.sessions
	_check(not sessions.is_empty(), "%d sessions" % sessions.size())
	for s: Dictionary in sessions:
		var records := Check.records(files, s.file)
		if records.is_empty():
			_check(false, "%s has telemetry" % s.label)
			continue
		var amplitude := int(s.amplitude_ppm)
		var r := Check.replay(records, key, amplitude)
		_check(s.complete and s.probe_applied and r.error == OK and r.worst < 1e-9,
				"%s replays with the facilitator's key (worst %s)" % [s.label, r.worst])
		if amplitude == 0:
			_check(r.drift < 1e-12, "%s has no drift" % s.label)
		else:
			_check(r.drift > 1e-6, "%s drifted" % s.label)
	_finish()


func _check(ok: bool, what: String) -> void:
	print(("ok    " if ok else "FAIL  ") + what)
	if not ok:
		failures.append(what)


func _finish() -> void:
	print("\ncheck_study_export: %s" % ("ok" if failures.is_empty() else "%d failed" % failures.size()))
	quit(0 if failures.is_empty() else 1)
