# SPDX-License-Identifier: MIT OR Apache-2.0

## In-memory log of a session's telemetry records, and a reader for telemetry files.
##
## Records are dictionaries in the rearguard.telemetry v1 schema (rearguard_core::telemetry),
## the same schema the server ingests. Telemetry *files* are written by the Rust extension
## (RearguardRecorder, via telemetry_out.gd), which writes floats exactly; this log keeps
## the exact in-memory records for tests.
extends RefCounted

const FORMAT := "rearguard.telemetry"
const VERSION := 1

var records: Array[Dictionary] = []


func write(record: Dictionary) -> void:
	records.append(record.duplicate())


## Parses telemetry lines back into dictionaries. Numbers come back as floats, as
## JSON has one number type. Godot's parser can be off by one ulp on some doubles;
## the file itself is exact (see README.md).
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
