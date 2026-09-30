## Writes a recording as JSON Lines: one JSON object per line, keys in insertion
## order, floats at full (round-trip) precision. The format is documented in
## demo/README.md under "Recording format".
##
## Writes to a file, or to memory when no path is given (used by the tests).
extends RefCounted

const FORMAT := "rearguard.aimrange.recording"
## Version 2 (task 1.5) adds the probe_* header fields; see demo/README.md.
const VERSION := 2

## When recording to memory: the lines written so far, and the records themselves.
## Keep `records` for exact comparisons: Godot's own JSON/float parser is not
## correctly rounded (see README.md), so parsing `lines` can be off in the last bit.
var lines := PackedStringArray()
var records: Array[Dictionary] = []
var path := ""
var _file: FileAccess


## Opens `p_path` for writing, creating its directory. Empty path: record to memory.
func _init(p_path: String = "") -> void:
	path = p_path
	if path.is_empty():
		return
	DirAccess.make_dir_recursive_absolute(path.get_base_dir())
	_file = FileAccess.open(path, FileAccess.WRITE)
	if _file == null:
		push_error("recorder: cannot open %s: %s" % [path, error_string(FileAccess.get_open_error())])


func write(record: Dictionary) -> void:
	var line := JSON.stringify(record, "", false, true)
	if _file != null:
		_file.store_line(line)
	elif path.is_empty():
		lines.append(line)
		records.append(record.duplicate())


## Pushes buffered lines to disk, so a killed process loses at most the last second.
func flush() -> void:
	if _file != null:
		_file.flush()


func close() -> void:
	if _file != null:
		_file.close()
		_file = null


## Parses recording lines back into dictionaries. Numbers come back as floats, as
## JSON has one number type. Godot's parser can be off by one ulp on some doubles;
## the file itself is exact (checked against a correctly rounded parser).
static func parse_lines(text_lines: PackedStringArray) -> Array[Dictionary]:
	var out: Array[Dictionary] = []
	for line in text_lines:
		if line.strip_edges().is_empty():
			continue
		var value: Variant = JSON.parse_string(line)
		if value is Dictionary:
			out.append(value)
		else:
			push_error("recorder: not a JSON object: %s" % line)
	return out


static func read_file(p_path: String) -> Array[Dictionary]:
	var text := FileAccess.get_file_as_string(p_path)
	return parse_lines(text.split("\n"))
