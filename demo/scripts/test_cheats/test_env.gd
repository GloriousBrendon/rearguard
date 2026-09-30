# SPDX-License-Identifier: MIT OR Apache-2.0
## Environment guard for the test cheats (task 1.8). A test cheat runs only inside
## Rearguard's own test environment:
## - REARGUARD_TEST_ENV=1 is set, and
## - it plays against a Rearguard server given as an IP literal on this machine
##   (127.0.0.0/8 or ::1), or one named in REARGUARD_TEST_SERVER_ALLOWLIST
##   (comma-separated `HOST` or `HOST:PORT` entries).
## Anything else is refused before the cheat is created.
extends RefCounted

const ENV_FLAG := "REARGUARD_TEST_ENV"
const ENV_ALLOWLIST := "REARGUARD_TEST_SERVER_ALLOWLIST"


## The two variables from the process environment.
static func process_environment() -> Dictionary:
	return {ENV_FLAG: OS.get_environment(ENV_FLAG), ENV_ALLOWLIST: OS.get_environment(ENV_ALLOWLIST)}


## Returns "" when a test cheat may play against `server` ("HOST:PORT", "[V6]:PORT"),
## else the reason it may not. `env` holds ENV_FLAG and ENV_ALLOWLIST.
static func refusal(server: String, env: Dictionary) -> String:
	if str(env.get(ENV_FLAG, "")) != "1":
		return "%s=1 is not set: test cheats run only in Rearguard's own test environment" % ENV_FLAG
	if server.is_empty():
		return "no server: test cheats play only against a Rearguard test server (--server HOST:PORT)"
	var host := host_of(server)
	if is_loopback(host):
		return ""
	for entry in str(env.get(ENV_ALLOWLIST, "")).split(",", false):
		var e := entry.strip_edges()
		if not e.is_empty() and (e == server or e == host):
			return ""
	return "server %s is neither loopback nor in %s" % [server, ENV_ALLOWLIST]


## The host part of "HOST:PORT" or "[V6]:PORT" (brackets removed).
static func host_of(server: String) -> String:
	if server.begins_with("["):
		var close := server.find("]")
		return server.substr(1, close - 1) if close > 0 else ""
	var colon := server.rfind(":")
	return server.substr(0, colon) if colon >= 0 else server


## True for an IPv4 literal in 127.0.0.0/8 or the IPv6 literal ::1. Names (even
## "localhost") are not trusted: they could resolve anywhere.
static func is_loopback(host: String) -> bool:
	if host == "::1" or host == "0:0:0:0:0:0:0:1":
		return true
	var parts := host.split(".")
	if parts.size() != 4 or parts[0] != "127":
		return false
	for p in parts:
		if not p.is_valid_int() or p.to_int() < 0 or p.to_int() > 255:
			return false
	return true
