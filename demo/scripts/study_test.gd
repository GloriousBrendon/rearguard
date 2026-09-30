extends "res://tests/test_case.gd"

## Task 1.9: the human-study build (scripts/study.gd), run end to end with automated
## consent, the scripted bot and seeded answers.

const Study := preload("res://scripts/study.gd")
const Check := preload("res://tests/study_export_check.gd")
const STUDY_SCENE := preload("res://scenes/study.tscn")

## Test study key (public test data, not a secret).
const KEY_HEX := "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90"
const PARTICIPANT := "P-TEST234567"
const SEED := "20260930"
const PROTOCOL := {
	"protocol_version": 1,
	"study_id": "unit-test",
	"baseline": [
		{"scenario": "flick", "amplitude_ppm": 0, "duration_s": 2, "repeats": 1},
		{"scenario": "spray", "amplitude_ppm": 20000, "duration_s": 2, "repeats": 1},
	],
	"blind": {"scenario": "flick", "interval_duration_s": 1.5, "amplitudes_ppm": [0, 20000], "repeats": 1},
}
func _dir(name: String) -> String:
	return ProjectSettings.globalize_path("user://test/study-%s" % name)


func _write(path: String, text: String) -> void:
	DirAccess.make_dir_recursive_absolute(path.get_base_dir())
	var f := FileAccess.open(path, FileAccess.WRITE)
	f.store_string(text)
	f.close()


func _clear(dir: String) -> void:
	for f in DirAccess.get_files_at(dir):
		DirAccess.remove_absolute(dir.path_join(f))


## Runs the automated study; returns {path, node_info} once it has finished.
func _run_study(name: String, extra: Dictionary = {}) -> Dictionary:
	var dir := _dir(name)
	_write(dir.path_join("study-key.hex"), KEY_HEX + "\n")
	_write(dir.path_join("protocol.json"), JSON.stringify(PROTOCOL, "  "))
	DirAccess.make_dir_recursive_absolute(dir.path_join("exports"))
	_clear(dir.path_join("exports"))
	var node: Node = STUDY_SCENE.instantiate()
	var c := Study.default_config()
	c.merge({"study_key": dir.path_join("study-key.hex"), "protocol": dir.path_join("protocol.json"),
			"export_dir": dir.path_join("exports"), "auto": true, "participant_id": PARTICIPANT,
			"seed": SEED, "auto_answer_seed": 3}, true)
	c.merge(extra, true)
	node.config = c
	tree.root.add_child(node)
	var path: String = await node.finished
	var info := {"path": path, "participant_id": node.participant_id, "seed": node.seed_text,
			"plan": node.plan.duplicate(true)}
	node.queue_free()
	await tree.process_frame
	return info


## Reads every entry of the export zip into {name: bytes}.
func _unzip(path: String) -> Dictionary:
	var files := Check.unzip(path)
	if files.is_empty():
		failures.append("%s: cannot open %s" % [_current, path])
	return files


# Acceptance 1, 2 and 3: the whole flow, labelled baseline sessions with drift on and
# off (the drift really applied, from the study key), per-trial answers, amplitude and
# order, and a plan that is reproducible from the logged seed.
func test_automated_study_exports_labelled_sessions_and_trials() -> void:
	if not extension_loaded():
		return
	var info := await _run_study("flow")
	var path: String = info.path
	check(path.ends_with("rearguard-study-unit-test-%s.zip" % PARTICIPANT), "export name: %s" % path)
	var files := _unzip(path)
	if files.is_empty():
		return
	var manifest: Dictionary = JSON.parse_string(files["manifest.json"].get_string_from_utf8())
	check_eq(manifest.format, "rearguard.study.export")
	check_eq(int(manifest.version), 1)
	check_eq(manifest.participant_id, PARTICIPANT)
	check_eq(manifest.randomisation_seed, SEED)
	check_eq(manifest.consent_version, "v1")
	check_eq(manifest.release, "dev", "release label of a repository run")
	check_eq(manifest.automated, true, "an automated run is marked as such")

	# Reproducible: the same protocol and logged seed give the same plan, and the
	# manifest's plan is exactly that.
	var protocol_text := FileAccess.get_file_as_string(_dir("flow").path_join("protocol.json"))
	check_eq(manifest.protocol, JSON.parse_string(protocol_text), "protocol embedded")
	var again: String = ClassDB.class_call_static("RearguardStudy", "plan", protocol_text,
			manifest.randomisation_seed)
	check_eq(JSON.parse_string(again), manifest.plan, "plan re-derived from the logged seed")
	check_eq(manifest.plan, info.plan, "manifest plan is the plan that was run")

	var sessions: Array = manifest.sessions
	check_eq(sessions.size(), 2 + 4, "2 baseline sessions and 2 trials of 2 intervals")
	var expected_files := ["manifest.json"]
	var conditions := {}
	for s: Dictionary in sessions:
		expected_files.append(s.file)
		check(s.complete, "%s complete" % s.label)
		check(s.probe_applied, "%s ran the probe from the study key" % s.label)
		var amplitude := int(s.amplitude_ppm)
		check_eq(s.condition, "drift-off" if amplitude == 0 else "drift-%dppm" % amplitude, "%s condition" % s.label)
		if s.block == "baseline":
			conditions[s.condition] = true
		var records := Check.records(files, s.file)
		check(records.size() > 10, "%s records: %d" % [s.label, records.size()])
		if records.is_empty():
			continue
		var header := records[0]
		check_eq(header.match_id, "study:unit-test:%s:%s" % [PARTICIPANT, s.label], "%s match id" % s.label)
		check_eq(int(header.scenario_seed), int(s.scenario_seed), "%s scenario seed" % s.label)
		check(records[-1].type == "end" and records[-1].complete == true, "%s end record" % s.label)
		# Re-derive this session's probe from the study key and replay the moves.
		var replayed := Check.replay(records, _dir("flow").path_join("study-key.hex"), amplitude)
		check_eq(replayed.error, OK, "%s derive" % s.label)
		check(replayed.worst < 1e-9, "%s replays with the derived probe (worst %s)" % [s.label, replayed.worst])
		if amplitude == 0:
			check(replayed.drift < 1e-12, "%s has no drift" % s.label)
		else:
			check(replayed.drift > 1e-6, "%s drifted (%s deg)" % [s.label, replayed.drift])
	check(conditions.has("drift-off") and conditions.has("drift-20000ppm"), "baseline has drift on and off: %s" % str(conditions.keys()))
	expected_files.sort()
	var names := files.keys()
	names.sort()
	check_eq(names, expected_files, "export entries")

	var trials: Array = manifest.trials
	check_eq(trials.size(), 2)
	var amplitudes := []
	for t: Dictionary in trials:
		amplitudes.append(int(t.amplitude_ppm))
		check(t.answer in ["A", "B"], "forced choice: %s" % t.answer)
		check_eq(t.correct, t.answer == t.drifted, "correct flag")
		check(int(t.response_ms) >= 0, "response time")
		var n := int(t.index) + 1
		check_eq(t.order, ["trial-%02d-A" % n, "trial-%02d-B" % n], "order")
		for s: Dictionary in sessions:
			if s.block == "blind" and int(s.trial) == int(t.index):
				var drifted: bool = s.interval == t.drifted
				check_eq(int(s.amplitude_ppm), int(t.amplitude_ppm) if drifted else 0, "%s amplitude" % s.label)
	amplitudes.sort()
	check_eq(amplitudes, [0, 20000], "each amplitude once")
	# The temporary session files are gone once exported.
	check(not DirAccess.dir_exists_absolute(ProjectSettings.globalize_path("user://study-work").path_join(PARTICIPANT)),
			"work directory removed")


# Acceptance 4: the export holds no personal data. Only whitelisted keys, and none of
# the machine's identifying strings, today's date or the study key anywhere in it.
func test_export_contains_no_personal_data() -> void:
	if not extension_loaded():
		return
	var info := await _run_study("privacy", {"participant_id": "", "seed": ""})
	var id: String = info.participant_id
	check(id.length() == 12 and id.begins_with("P-"), "random participant id: %s" % id)
	check(str(info.seed).is_valid_int(), "random seed: %s" % info.seed)
	var files := _unzip(info.path)
	if files.is_empty():
		return
	for f in Check.personal_data_failures(files, KEY_HEX):
		failures.append("%s: %s" % [_current, f])


# Declining consent writes nothing at all.
func test_declining_consent_writes_nothing() -> void:
	if not extension_loaded():
		return
	var dir := _dir("decline")
	_write(dir.path_join("study-key.hex"), KEY_HEX + "\n")
	_write(dir.path_join("protocol.json"), JSON.stringify(PROTOCOL))
	var exports := dir.path_join("exports")
	DirAccess.make_dir_recursive_absolute(exports)
	_clear(exports)
	var node: Node = STUDY_SCENE.instantiate()
	var c := Study.default_config()
	c.merge({"study_key": dir.path_join("study-key.hex"), "protocol": dir.path_join("protocol.json"),
			"export_dir": exports}, true)
	node.config = c
	tree.root.add_child(node)
	await tree.process_frame
	node.decline_consent()
	check_eq(node.participant_id, "", "no participant id drawn")
	check_eq(DirAccess.get_files_at(exports).size(), 0, "no export")
	node.queue_free()
	await tree.process_frame


func test_parse_args() -> void:
	var c := Study.parse_args(PackedStringArray(["--study-key", "k.hex", "--protocol", "p.json", "--export-dir", "out"]))
	check_eq(c.study_key, "k.hex")
	check_eq(c.protocol, "p.json")
	check_eq(c.export_dir, "out")
	check_eq(c.auto, false, "no automation without --self-test")
	check_eq(c.smoke_decline, false)
	check_eq(Study.parse_args(PackedStringArray(["--smoke-decline"])).smoke_decline, true)
	check_eq(Study.parse_args(PackedStringArray(["--smoke-decline"])).auto, false, "the smoke check can only decline")
	check_eq(Study.parse_args(PackedStringArray(["--self-test"])).auto, true, "--self-test: the bot plays")
	check_eq(Study.parse_args(PackedStringArray(["--self-test"])).quit_when_done, true)
	check_eq(c.quit_when_done, false)


# A repository run has no packed key: the default is the user-folder key file, and the
# release label is "dev". (Exported builds are checked by scripts/smoke-study-build.sh.)
func test_repository_run_uses_the_user_folder_key() -> void:
	check(not FileAccess.file_exists(Study.PACKED_KEY_PATH), "no study key in the repository")
	check(not FileAccess.file_exists(Study.RELEASE_PATH), "no release label in the repository")
	check_eq(Study.default_config().study_key, "user://study/study-key.hex")


# The extension reads a res:// key file itself (a key packed into an exported build).
func test_extension_reads_res_key_paths() -> void:
	if not extension_loaded():
		return
	var probe: RefCounted = ClassDB.instantiate("RearguardProbe")
	check_eq(probe.start_session_derived("res://study/no-such-key.hex", "m", 0, 5000, 0), ERR_FILE_NOT_FOUND)
	# Any res:// file that is not 64 hex digits is refused.
	check_eq(probe.start_session_derived("res://study/protocol.json", "m", 0, 5000, 0), ERR_INVALID_DATA)
	check(not probe.is_active(), "no session from an invalid key")
	# The same key read as res:// and as a file-system path gives the same drift.
	var packed := "res://tests/public-test-key.hex"
	check_eq(FileAccess.get_file_as_string(packed).strip_edges(), KEY_HEX, "test key file is KEY_HEX")
	check_eq(probe.start_session_derived(packed, "m", 0, 20000, 0), OK, "derive from res://")
	var from_res := [probe.multiplier_at_tick(0, 1234), probe.multiplier_at_tick(1, 99)]
	check_eq(probe.start_session_derived(ProjectSettings.globalize_path(packed), "m", 0, 20000, 0), OK)
	check_eq([probe.multiplier_at_tick(0, 1234), probe.multiplier_at_tick(1, 99)], from_res, "same drift")
	check(from_res[0] != 1.0, "drift applied")
	probe.stop()


# The shipped protocol plans: 6 baseline rounds and 20 trials over the D6 amplitude grid.
func test_shipped_protocol_is_valid() -> void:
	if not extension_loaded():
		return
	var p: Dictionary = JSON.parse_string(ClassDB.class_call_static("RearguardStudy", "plan",
			FileAccess.get_file_as_string("res://study/protocol.json"), "1"))
	check(not p.has("error"), "plan error: %s" % p.get("error", ""))
	if p.has("error"):
		return
	check_eq(p.baseline.size(), 6)
	check_eq(p.trials.size(), 20)
	var amplitudes := {}
	for t: Dictionary in p.trials:
		amplitudes[int(t.amplitude_ppm)] = true
	var keys := amplitudes.keys()
	keys.sort()
	check_eq(keys, [0, 2500, 5000, 10000, 20000], "D6 grid")
