# SPDX-License-Identifier: MIT OR Apache-2.0
## The test cheats (task 1.8): in-tree bots that inject input into this aim range only,
## through its own input path, to exercise Rearguard's detector. See README.md here.
##
## Each is created only after test_env.gd's guard passes. Its ground-truth label goes
## into the server session's metadata for the evaluation harness; the telemetry never
## carries it and the detector never reads it.
extends RefCounted

const TestEnv := preload("res://scripts/test_cheats/test_env.gd")

## Name -> [script, scenario it plays].
const CHEATS := {
	"recoil-macro": ["res://scripts/test_cheats/recoil_macro.gd", "spray"],
	"flick-aimbot": ["res://scripts/test_cheats/flick_aimbot.gd", "flick"],
	"humanised-aimbot": ["res://scripts/test_cheats/humanised_aimbot.gd", "flick"],
	"adaptive-aimbot": ["res://scripts/test_cheats/adaptive_aimbot.gd", "flick"],
}


static func names() -> PackedStringArray:
	return PackedStringArray(CHEATS.keys())


## The ground-truth label for the session's metadata.
static func label_for(cheat: String) -> String:
	return "cheat:" + cheat


## Returns "" when `config` (the aim range's options) may run its test cheat, else why
## not. `env` holds the guard's environment variables (TestEnv.process_environment()).
static func refusal(config: Dictionary, env: Dictionary) -> String:
	var why := TestEnv.refusal(str(config.get("server", "")), env)
	if not why.is_empty():
		return why
	var cheat := str(config.get("cheat", ""))
	if not CHEATS.has(cheat):
		return "unknown test cheat '%s' (%s)" % [cheat, ", ".join(names())]
	if str(config.get("scenario", "")) != CHEATS[cheat][1]:
		return "%s plays the %s scenario, not %s" % [cheat, CHEATS[cheat][1], config.get("scenario", "")]
	return ""


## A new instance of `cheat`, seeded with `bot_seed`. Call only after refusal() is "".
static func create(cheat: String, bot_seed: int) -> RefCounted:
	return load(CHEATS[cheat][0]).new(bot_seed)
