extends "res://tests/test_case.gd"

const AimRange := preload("res://scripts/aim_range.gd")
const Recorder := preload("res://scripts/recorder.gd")
const SCENE := preload("res://scenes/aim_range.tscn")


## Runs the real scene with the scripted bot. Bot events go through
## Input.parse_input_event and the scene's _input, like hardware events.
func _run_scene(scenario: String, seed_value: int, out: String) -> Array[Dictionary]:
	var node: AimRange = SCENE.instantiate()
	var c := AimRange.default_config()
	c.merge({"scenario": scenario, "seed": seed_value, "duration": 2.0, "bot": true, "out": out}, true)
	node.config = c
	tree.root.add_child(node)
	var path: String = await node.run_finished
	node.queue_free()
	await tree.process_frame
	return Recorder.read_file(path)


func _strip_timing(records: Array[Dictionary]) -> Array[Dictionary]:
	var out: Array[Dictionary] = []
	for r in records:
		var d := r.duplicate()
		d.erase("ts_us")
		d.erase("frame")
		out.append(d)
	return out


func test_parse_args() -> void:
	var c := AimRange.parse_args(PackedStringArray(
			["--scenario", "spray", "--seed", "9", "--duration", "5", "--bot", "--out", "x.jsonl"]))
	check_eq(c.scenario, "spray")
	check_eq(c.seed, 9)
	check_eq(c.duration, 5.0)
	check_eq(c.bot, true)
	check_eq(c.quit, true, "bot quits when done")
	check_eq(c.out, "x.jsonl")


func test_headless_bot_run_records_through_input_path() -> void:
	var records := await _run_scene("tracking", 4, "user://test/scene_a.jsonl")
	check(records.size() > 10, "records: %d" % records.size())
	if records.is_empty():
		return
	var header := records[0]
	check_eq(header.source, "bot")
	check_eq(header.accumulated_input, false, "accumulation disabled")
	check_eq(header.delta_source, "InputEventMouseMotion.screen_relative")
	check_eq(records[-1].type, "end")
	var moves := records.filter(func(r: Dictionary) -> bool: return r.type == "move")
	check(moves.size() > 50, "moves reached the session via _input: %d" % moves.size())
	var last := -1
	for r in records:
		check(int(r.ts_us) >= last, "real timestamps monotonic")
		last = int(r.ts_us)


func test_headless_scene_replays_identically() -> void:
	var a := await _run_scene("spray", 11, "user://test/scene_b.jsonl")
	var b := await _run_scene("spray", 11, "user://test/scene_c.jsonl")
	check(a.size() > 10, "records: %d" % a.size())
	check_eq(_strip_timing(a), _strip_timing(b), "same seeds, same run apart from timing")
